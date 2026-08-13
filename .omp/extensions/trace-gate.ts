// TraceLayer × Oh My Pi — native pre-mutation gate and stop gate.
// Copy to .omp/extensions/trace-gate.ts (project) or ~/.omp/extensions/ (global).
// Blocking enforcement: pre-edit context guard + fail-closed completion gate.
import type { HookAPI } from "@oh-my-pi/pi-coding-agent/extensibility/hooks";

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

  // Block the first edit of protected traced behavior until context is loaded.
  pi.on("tool_call", async (event, ctx) => {
    if (event.toolName !== "edit" && event.toolName !== "write") return;
    const { path, line } = fileInfo(event.input);
    if (!path) return;
    const payload = JSON.stringify({
      path,
      line,
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

  // Fail-closed completion gate: block shutdown while trace verify fails.
  pi.on("session_shutdown", async (event, ctx) => {
    const payload = JSON.stringify({
      lifecycle: "wip",
      session_id: sessionId(ctx),
    });
    const res = run(["hook", "stop", "--format", "json"], payload);
    if (res.code !== 0) {
      pi.log(`trace verify: ${res.out}`);
      return { block: true, reason: "trace verification has blocking failures" };
    }
  });
}
