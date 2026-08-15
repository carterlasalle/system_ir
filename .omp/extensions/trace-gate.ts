// TraceLayer × Oh My Pi — native pre-mutation gate and stop gate.
// Copy to .omp/extensions/trace-gate.ts (project) or ~/.omp/extensions/ (global).
// Blocking enforcement: pre-edit context guard + fail-closed completion gate.
import type { HookAPI } from "@oh-my-pi/pi-coding-agent/extensibility/hooks";

// trace:v1 id=impl.omp.trace-gate work=WORK-TL-001
export default function hook(pi: HookAPI): void {
  const run = (args: string[], input: string): { code: number; out: string } => {
    const res = Bun.spawnSync(["uv", "run", "trace", ...args], {
      stdin: "pipe",
      input,
    });
    return { code: res.exitCode ?? -1, out: res.stdout.toString() };
  };

  const sessionId = (ctx: unknown): string => {
    if (ctx && typeof ctx === "object" && "session" in ctx) {
      const session = ctx.session;
      if (session && typeof session === "object" && "id" in session && typeof session.id === "string") {
        return session.id;
      }
    }
    return "default";
  };

  // Resolve the target file from a tool input via runtime narrowing.
  const fileInfo = (input: unknown): { path: string; line: number | null } => {
    if (!input || typeof input !== "object") return { path: "", line: null };
    let path = "";
    if ("file_path" in input && typeof input.file_path === "string") {
      path = input.file_path;
    } else if ("path" in input && typeof input.path === "string") {
      path = input.path;
    }
    const line = "line" in input && typeof input.line === "number" ? input.line : null;
    return { path, line };
  };

  // Pre-authoring gate: pass the FULL proposed mutation so TraceLayer can
  // simulate the edit (new boundaries, modified untraced behavior).
  pi.on("tool_call", async (event, ctx) => {
    if (event.toolName !== "edit" && event.toolName !== "write") return;
    const { path, line } = fileInfo(event.input);
    if (!path) return;
    const input = (event.input ?? {}) as Record<string, unknown>;
    const payload = JSON.stringify({
      path,
      line,
      content: input.content,
      old_string: input.old_string,
      new_string: input.new_string,
      session_id: sessionId(ctx),
    });
    const res = run(["hook", "pre-mutation", "--format", "json"], payload);
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
  pi.on("tool_result", async (event, ctx) => {
    if (event.toolName !== "edit" && event.toolName !== "write") return;
    const { path } = fileInfo(event.input);
    if (!path) return;
    const payload = JSON.stringify({ path, session_id: sessionId(ctx) });
    const res = run(["hook", "post-mutation", "--format", "json"], payload);
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

  // Fail-closed completion gate: block while trace obligations or verify fail.
  pi.on("session_stop", async (event, ctx) => {
    const payload = JSON.stringify({
      lifecycle: "wip",
      session_id: sessionId(ctx),
    });
    const res = run(["hook", "stop", "--format", "json"], payload);
    if (res.code !== 0) {
      let reason = "trace verification has blocking failures";
      try {
        const d = JSON.parse(res.out) as { output?: string };
        if (typeof d.output === "string" && d.output) reason = d.output;
      } catch {
        // keep default reason
      }
      pi.log(`trace gate: ${reason}`);
      return { block: true, reason };
    }
  });
}
