// TraceLayer × Oh My Pi — native pre-mutation gate and stop gate.
// Copy to .omp/extensions/trace-gate.ts (project) or ~/.omp/extensions/ (global).
// Blocking enforcement: pre-edit context guard + fail-closed completion gate.
//
// Type-only import (erased at runtime): the canonical package name is
// @earendil-works/pi-coding-agent; older runtimes used @oh-my-pi/... The
// extension runtime never type-checks this file.
import { spawnSync } from "node:child_process";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

// The session_stop event is a compat surface of the OMP runtime (it fires
// in current versions too) even though the published ExtensionAPI type
// union does not list it. Extend the type rather than fighting it.
// trace:exempt reason=internal-helper
type SessionStopCompat = ExtensionAPI & {
  on(
    event: "session_stop",
    handler: (event: unknown, ctx: unknown) => Promise<unknown> | unknown
  ): void;
};

// trace:v1 id=impl.omp.trace-gate work=WORK-TL-001
export default function hook(pi: ExtensionAPI): void {
  // node:child_process spawnSync (NOT Bun.spawnSync): Bun's spawnSync
  // ignores the `input` option, which silently dropped every hook payload —
  // the gate then ran with an empty body and never blocked. Node's
  // spawnSync writes input to stdin reliably and runs under the Bun host.
  // trace:exempt reason=internal-helper
  const run = (args: string[], input: string): { code: number; out: string } => {
    const res = spawnSync("uv", ["run", "trace", ...args], {
      input,
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
    });
    const code = res.status ?? -1;
    return { code, out: String(res.stdout ?? "") };
  };

  // Runtime-narrowed session id: old ctx shape carried `.session.id`;
  // current ExtensionContext exposes `sessionManager.getSessionId()`.
  // trace:exempt reason=internal-helper
  const sessionId = (ctx: unknown): string => {
    if (ctx && typeof ctx === "object") {
      const c = ctx as { session?: { id?: unknown }; sessionManager?: { getSessionId?: () => unknown } };
      if (c.session && typeof c.session === "object" && typeof c.session.id === "string") {
        return c.session.id;
      }
      try {
        const sid = c.sessionManager?.getSessionId?.();
        if (typeof sid === "string" && sid) return sid;
      } catch {
        // fall through
      }
    }
    return "default";
  };

  // Resolve the target file + mutation text from a tool input via runtime
  // narrowing. Current OMP: Write = {path, content}, Edit = {path, edits:
  // [{oldText, newText}]}; older runtimes used Claude-style {file_path,
  // old_string, new_string}. The pre gate simulates ONE replacement, so a
  // multi-edit input passes the first edit; the post hook + obligations
  // re-validate the real file.
  // trace:exempt reason=internal-helper
  const fileInfo = (input: unknown): { path: string; line: number | null; payload: Record<string, unknown> } => {
    if (!input || typeof input !== "object") return { path: "", line: null, payload: {} };
    const rec = input as Record<string, unknown>;
    let path = "";
    if (typeof rec.file_path === "string") {
      path = rec.file_path;
    } else if (typeof rec.path === "string") {
      path = rec.path;
    }
    const line = typeof rec.line === "number" ? rec.line : null;
    const payload: Record<string, unknown> = {};
    if (rec.content !== undefined) payload.content = rec.content;
    if (typeof rec.old_string === "string" && typeof rec.new_string === "string") {
      payload.old_string = rec.old_string;
      payload.new_string = rec.new_string;
    } else if (Array.isArray(rec.edits) && rec.edits.length > 0) {
      const first = rec.edits[0] as { oldText?: unknown; newText?: unknown };
      if (typeof first?.oldText === "string" && typeof first.newText === "string") {
        payload.old_string = first.oldText;
        payload.new_string = first.newText;
      }
    }
    return { path, line, payload };
  };

  // Pre-authoring gate: pass the FULL proposed mutation so TraceLayer can
  // simulate the edit (new boundaries, modified untraced behavior).
  pi.on("tool_call", async (event, ctx) => {
    if (event.toolName !== "edit" && event.toolName !== "write") return;
    const { path, line, payload } = fileInfo(event.input);
    if (!path) return;
    const body = JSON.stringify({
      path,
      line,
      ...payload,
      session_id: sessionId(ctx),
    });
    const res = run(["hook", "pre-mutation", "--format", "json"], body);
    if (res.code !== 0) {
      let reason = "trace policy blocks this edit";
      try {
        const d = JSON.parse(res.out) as { output?: string };
        if (typeof d.output === "string" && d.output) reason = d.output;
      } catch {
        // keep default reason
      }
      return { block: true, reason };
    }
  });

  // Post-edit coaching: run post-mutation so obligations resolve, and
  // append the trace guidance directly to the tool result the model sees.
  // Write/Edit carry a path; Bash and other opaque filesystem-mutating
  // tools run the working-tree scan (no path -> _scan_changed_files), so
  // cat >/patch/generator mutations get the same immediate coaching.
  pi.on("tool_result", async (event, ctx) => {
    const opaque = event.toolName === "bash" || event.toolName === "patch";
    if (event.toolName !== "edit" && event.toolName !== "write" && !opaque) return;
    let body;
    if (opaque) {
      body = JSON.stringify({ session_id: sessionId(ctx) });
    } else {
      const { path } = fileInfo(event.input);
      if (!path) return;
      body = JSON.stringify({ path, session_id: sessionId(ctx) });
    }
    const res = run(["hook", "post-mutation", "--format", "json"], body);
    if (res.code !== 0) return;
    try {
      const d = JSON.parse(res.out) as { output?: string };
      if (typeof d.output !== "string" || !d.output) return;
      const content = Array.isArray(event.content) ? [...event.content] : [];
      content.push({
        type: "text",
        text: `\n\n<TraceLayer>\n${d.output}\n</TraceLayer>`,
      });
      return { content };
    } catch {
      return;
    }
  });

  // Fail-closed completion gate: block while trace obligations or verify
  // fail. OMP's SessionStopEventResult carries decision/reason (or
  // continuation fields) — not a `block` property. The engine's stop hook
  // ALSO runs the merge-grade auto-finalizer internally.
  (pi as SessionStopCompat).on("session_stop", async (_event, ctx) => {
    const body = JSON.stringify({
      lifecycle: "wip",
      session_id: sessionId(ctx),
    });
    const res = run(["hook", "stop", "--format", "json"], body);
    if (res.code !== 0) {
      let reason = "trace verification has blocking failures";
      try {
        const d = JSON.parse(res.out) as { output?: string };
        if (typeof d.output === "string" && d.output) reason = d.output;
      } catch {
        // keep default reason
      }
      // The extension API has no `log` method — stderr lands in the OMP
      // log, and the block reason is returned to the session result.
      console.error(`trace gate: ${reason}`);
      return { decision: "block", reason };
    }
  });
}
