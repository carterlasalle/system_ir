// SCC × Oh My Pi — native lifecycle integration.
//
// This file is the extension entry declared by package.json in this
// directory. `scc setup omp` installs the whole package at
// `.omp/extensions/scc/` (project scope). It registers ONE module that
// owns ALL ordering-dependent SCC lifecycle behavior via native OMP
// events — there is no cross-module or filename-ordering dependence.
//
// Lifecycle events wired here (verified against the current OMP API):
//   session_start          -> precompute/state only (NOT model-visible)
//   before_agent_start     -> the ONLY model-visible injection point:
//                             startup (once/session) + task context/prompt
//   tool_result            -> post-edit `scc index --path <path> --quiet`
//   session_before_compact -> `scc checkpoint save` (persist state)
//   session_compact        -> re-inject fresh startup after compaction
//
// The canonical public type is `ExtensionAPI`. The runtime resolves the
// `@oh-my-pi/pi-coding-agent` package (verified in the bundled binary's
// virtual-module map for the installed OMP v18.0.4 — the `@oh-my-pi` scope
// is canonical; the legacy `@earendil-works/...` alias is NOT exposed by
// the compiled binary's extension loader).
import type {
  BeforeAgentStartEvent,
  ExtensionAPI,
  ExtensionContext,
  SessionCompactEvent,
  SessionStartEvent,
  ToolResultEvent,
} from "@oh-my-pi/pi-coding-agent";

// Sessions that already received the startup capsule this process. The
// startup capsule is injected ONCE per session (duplicate-on-resume is
// acceptable, but never on every prompt). Dynamic membership (runtime
// insert/delete keyed by session id) — a Set, not a static lookup table.
const startupInjected = new Set<string>();

// trace:exempt reason=internal-helper
const scc = async (
  pi: ExtensionAPI,
  args: string[],
  cwd: string,
): Promise<{ code: number; out: string }> => {
  try {
    const res = await pi.exec("scc", args, { cwd });
    return { code: res.code, out: String(res.stdout ?? "") };
  } catch {
    return { code: -1, out: "" };
  }
};

// trace:exempt reason=internal-helper
const isConversational = (prompt: string): boolean => {
  const p = prompt.trim().toLowerCase();
  if (p === "") return true;
  return /^(hi|hey|hello|yo|thanks|thank you|ok|okay|yes|no|bye|cool|nice|good|great|sure)[.!?\s]*$/i.test(p);
};

// trace:exempt reason=internal-helper
const isFileMutation = (toolName: string): boolean =>
  toolName === "edit" || toolName === "write";

// trace:exempt reason=internal-helper
const editedPath = (input: Record<string, unknown>): string =>
  typeof input.path === "string" ? input.path : "";

// trace:v1 id=impl.omp.scc work=WORK-SCC-001 satisfies=REQ-SCC-API
export default function hook(pi: ExtensionAPI): void {
  // session_start: precompute/state only. This is a notification — it
  // CANNOT return model context. We only reset the once-per-session
  // startup marker (a fresh session re-injects startup on its first real
  // prompt) and do a lightweight state-path presence check. Never kick a
  // huge cold index from this 30-second event.
  pi.on("session_start", async (_event: SessionStartEvent, ctx: ExtensionContext) => {
    startupInjected.delete(ctx.sessionManager.getSessionId());
    await scc(pi, ["state-path"], ctx.cwd);
  });

  // before_agent_start: THE model-visible injection point. Returns a
  // CustomMessage that the model sees in TUI/print/RPC/headless.
  pi.on("before_agent_start", async (event: BeforeAgentStartEvent, ctx: ExtensionContext) => {
    const prompt = event.prompt;
    // A conversational greeting does not need a task pack (never blanket
    // length filter — only obvious acknowledgements).
    if (isConversational(prompt)) return;

    let content = "";
    // 1. Startup capsule: once per session (keyed by session id).
    const sid = ctx.sessionManager.getSessionId();
    if (!startupInjected.has(sid)) {
      const startup = await scc(pi, ["context", "startup"], ctx.cwd);
      if (startup.code === 0 && startup.out.trim()) {
        content += startup.out.trim() + "\n\n";
        startupInjected.add(sid);
      }
    }
    // 2. Task context for this prompt (--hook keeps the 1500-token cap).
    if (prompt) {
      const task = await scc(pi, ["context", "task", prompt, "--hook"], ctx.cwd);
      if (task.code === 0 && task.out.trim()) {
        content += task.out.trim();
      }
    }
    if (!content.trim()) return;
    return {
      message: {
        customType: "scc-context",
        content,
        display: false,
        details: { source: "scc", injectedAt: Date.now() },
      },
    };
  });

  // tool_result: post-edit incremental refresh — index the touched file
  // so the next task context sees the new state.
  pi.on("tool_result", async (event: ToolResultEvent, ctx: ExtensionContext) => {
    if (!isFileMutation(event.toolName)) return;
    const path = editedPath(event.input);
    if (!path) return;
    // Incremental index of the single touched path; identical copy, quiet.
    await scc(pi, ["index", "--path", path, "--quiet"], ctx.cwd);
  });

  // session_before_compact: persist a checkpoint so the architecture
  // survives compaction.
  pi.on("session_before_compact", async (_event, ctx: ExtensionContext) => {
    await scc(pi, ["checkpoint", "save"], ctx.cwd);
  });

  // session_compact: after compaction the startup capsule is gone from
  // context; clear the once-per-session marker so the next real prompt
  // re-injects startup.
  pi.on("session_compact", (_event: SessionCompactEvent, ctx: ExtensionContext) => {
    startupInjected.delete(ctx.sessionManager.getSessionId());
  });
}