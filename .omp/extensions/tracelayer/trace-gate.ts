// TraceLayer × Oh My Pi — native pre-mutation gate, prompt context,
// session-start, and fail-closed stop gate.
//
// This file is the extension entry declared by package.json in this
// directory. `trace install --agent omp` installs the whole package at:
//   .omp/extensions/tracelayer/          project scope
//   ~/.omp/agent/extensions/tracelayer/  global scope
// and `omp install ./adapters/oh-my-pi` links the same directory as a
// plugin. Do NOT copy this file loose into extensions/ — the legacy raw
// layout double-registers the factory and was removed deliberately.
//
// Type-only import (erased at runtime): the canonical public package is
// `@oh-my-pi/pi-coding-agent` (verified in the compiled OMP 18.0.11
// extension loader; the legacy `@earendil-works/pi-coding-agent` alias is
// not exposed). The extension runtime never type-checks this file.
import { spawnSync } from "node:child_process";
import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";

// trace:v1 id=impl.omp.trace-gate work=WORK-TL-001
export default function hook(pi: ExtensionAPI): void {
  // node:child_process spawnSync (NOT Bun.spawnSync): Bun's spawnSync
  // ignores the `input` option, which silently dropped every hook payload —
  // the gate then ran with an empty body and never blocked. Node's
  // spawnSync writes input to stdin reliably and runs under the Bun host.
  // TRACE_BIN / `trace` on PATH, then `uv run trace` (the installed layout).
  // trace:exempt reason=internal-helper
  const run = (args: string[], input: string): { code: number; out: string } => {
    const bin = process.env.TRACE_BIN;
    const res = bin
      ? spawnSync(bin, args, { input, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 })
      : spawnSync("uv", ["run", "trace", ...args], {
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

  // Parse a hook JSON body for a human-readable reason / block decision.
  // trace:exempt reason=internal-helper
  const parseHook = (out: string): { reason: string; block: boolean; extra: string } => {
    try {
      const d = JSON.parse(out) as {
        output?: string;
        reason?: string;
        decision?: string;
        block?: boolean;
      };
      const extra = typeof d.output === "string" ? d.output : "";
      const reason = (typeof d.reason === "string" && d.reason) || extra || "trace policy blocks this action";
      const block = d.block === true || d.decision === "block";
      return { reason, block, extra };
    } catch {
      return { reason: "trace policy blocks this action", block: false, extra: "" };
    }
  };

  // session.created → session-start (migrated from the inert YAML hooks).
  pi.on("session_start", async (_event, ctx) => {
    run(
      ["hook", "session-start", "--format", "json"],
      JSON.stringify({ session_id: sessionId(ctx) }),
    );
  });

  // user.prompt.submit → prompt-context. before_agent_start is the native
  // OMP seam that can still inject model-visible context.
  pi.on("before_agent_start", async (event, ctx) => {
    const prompt = typeof (event as { prompt?: unknown }).prompt === "string"
      ? (event as { prompt: string }).prompt
      : "";
    const res = run(
      ["hook", "prompt-context", "--format", "json"],
      JSON.stringify({ prompt, session_id: sessionId(ctx) }),
    );
    if (res.code !== 0) return;
    const parsed = parseHook(res.out);
    if (!parsed.extra) return;
    return {
      message: {
        customType: "tracelayer-prompt-context",
        content: parsed.extra,
        display: false,
        details: { source: "tracelayer" },
      },
    };
  });

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
      const parsed = parseHook(res.out);
      return { block: true, reason: parsed.reason || "trace policy blocks this edit" };
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
  // fail. The trace CLI event is `stop` (verified against tracelayer
  // 0.2.40: `session-stop` is rejected as an unknown hook event).
  // SessionStopEventResult carries decision/reason (or continuation
  // fields) — not a `block` property.
  pi.on("session_stop", async (event, ctx) => {
  // session_stop: fail-closed completion gate. OMP documents session_stop
  // and can return a continuation or `{ decision: "block", reason }`.
  // package.json already advertises this gate.
  pi.on("session_stop", async (event, ctx) => {
    const ev = event as {
      session_id?: unknown;
      session_file?: unknown;
      turn_id?: unknown;
      last_assistant_message?: unknown;
    };
    const body = JSON.stringify({
      session_id: (typeof ev.session_id === "string" && ev.session_id) || sessionId(ctx),
      session_file: ev.session_file ?? null,
      turn_id: ev.turn_id ?? null,
      last_assistant_message: ev.last_assistant_message ?? null,
    });
    const res = run(["hook", "stop", "--format", "json"], body);
    if (res.code !== 0) {
      const parsed = parseHook(res.out);
      console.error(`trace gate: ${parsed.reason}`);
      return { decision: "block", reason: parsed.reason };
    }
    const parsed = parseHook(res.out);
    if (parsed.block) {
      return { decision: "block", reason: parsed.reason };
    }
    if (parsed.extra) {
      return { continue: true, additionalContext: parsed.extra };
    }
  });
}
