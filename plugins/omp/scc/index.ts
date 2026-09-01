// SCC × Oh My Pi — native lifecycle integration.
//
// This file is the extension entry declared by package.json in this
// directory. `scc setup omp` installs the whole package at
// `.omp/extensions/scc/` (project scope). It registers ONE module that
// owns ALL ordering-dependent SCC lifecycle behavior via native OMP
// events — there is no cross-module or filename-ordering dependence.
//
// Lifecycle events wired here (verified against the installed OMP 18.0.11
// binary's actual emit/result contracts):
//   session_start          -> reset per-session state (NOT model-visible)
//   session_switch/_branch/_tree -> same reset: new conversation context
//   before_agent_start     -> the ONLY per-turn model-visible injection:
//                             startup (once/session) + task context
//   session.compacting     -> compaction REHYDRATION: fresh startup +
//                             checkpoint injected as extraContext so the
//                             summarizer carries SCC forward
//   session_before_compact -> `scc checkpoint save` (persist state)
//   session_compact        -> clear startup marker (post-compaction
//                             context no longer holds it)
//   tool_result (edit/write) -> `scc index --paths <path> --quiet`
//   tool_result (bash/patch) -> git snapshot diff -> `scc index --paths
//                             <changed...> --quiet`, full-index fallback
//                             when change detection is unavailable.
//
// The canonical public type is `ExtensionAPI` from
// `@oh-my-pi/pi-coding-agent` — the @oh-my-pi scope is the one exposed by
// the compiled OMP binary's extension loader.
import type {
  BeforeAgentStartEvent,
  ExtensionAPI,
  ExtensionContext,
  SessionCompactingEvent,
  SessionCompactEvent,
  SessionStartEvent,
  ToolResultEvent,
} from "@oh-my-pi/pi-coding-agent";

// Sessions that already received the startup capsule this process. The
// startup capsule is injected ONCE per session (duplicate-on-resume is
// acceptable, but never on every prompt). Dynamic membership (runtime
// insert/delete keyed by session id) — a Set, not a static lookup table.
const startupInjected = new Set<string>();
// Last SCC model epoch injected per session (runtime insert/update — Map).
const lastEpoch = new Map<string, string>();

// Does the RESUMED conversation already carry an SCC startup capsule?
const sessionHasStartup = (ctx: ExtensionContext): boolean => {
  try {
    const entries = ctx.sessionManager.getEntries();
    if (!Array.isArray(entries)) return false;
    for (let i = entries.length - 1; i >= 0; i--) {
      const e = entries[i] as {
        type?: unknown;
        customType?: unknown;
        details?: { hasStartup?: unknown };
      };
      // Persisted custom entries carry customType/details at the TOP
      // level (verified against 18.0.11 session files), not nested.
      if (e?.type !== "custom_message" || e.customType !== "scc-context") continue;
      if (e.details?.hasStartup === true) return true;
    }
    return false;
  } catch {
    return false;
  }
};

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
const editedPath = (input: Record<string, unknown>): string => {
  if (typeof input.path === "string") return input.path;
  // Current OMP edit tool: the model passes a structured edit-command
  // DSL string (`{"input": "[path#tag]\nPUT ...", "i": "..."}`). The
  // target path is embedded in the first `[path#tag]` bracket prefix.
  // Runtime narrowing per the no-any rule.
  const dsl = input.input;
  if (typeof dsl === "string") {
    const m = /^\[([^\]#]+)(#[^\]]+)?\]/.exec(dsl);
    if (m) return m[1];
  }
  const legacy = input.file_path;
  return typeof legacy === "string" ? legacy : "";
};

// trace:exempt reason=internal-helper
const isFileMutation = (toolName: string): boolean =>
  toolName === "edit" || toolName === "write";

// Opaque-mutation change detection: `git status --porcelain -z` snapshot
// lines. Two successive snapshots diff into the precise changed paths
// (new/deleted/renamed all appear as porcelain entries).
// trace:exempt reason=internal-helper
const gitSnapshot = async (
  pi: ExtensionAPI,
  cwd: string,
): Promise<string[]> => {
  try {
    const res = await pi.exec("git", ["status", "--porcelain", "-z", "--untracked-files=all"], { cwd });
    if (res.code !== 0) return [];
    return String(res.stdout ?? "").split("\0").filter((l) => l.length > 0);
  } catch {
    return [];
  }
};

// Parse `git status --porcelain -z` entries into repo-relative paths.
// Each entry is `XY <path>`; with -z, rename pairs arrive as two
// consecutive entries (orig then new) — both count as changed paths.
// trace:exempt reason=internal-helper
const porcelainPaths = (entries: string[]): string[] => {
  const out: string[] = [];
  for (const e of entries) {
    if (e.length < 4) continue;
    out.push(e.slice(3));
  }
  return out;
};
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
    let epoch = "";
    let injectedStartup = false;
    // 1. Startup capsule: once per session (keyed by session id). A
    // resumed conversation that already carries an SCC startup capsule
    // (same process via the Set, or a prior session via the entry scan)
    // does not duplicate it; anything else re-injects — favoring
    // correctness over dedup means a missing architecture is the only
    // unacceptable outcome.
    const sid = ctx.sessionManager.getSessionId();
    if (!startupInjected.has(sid) && !sessionHasStartup(ctx)) {
      const startup = await scc(pi, ["context", "startup"], ctx.cwd);
      if (startup.code === 0 && startup.out.trim()) {
        content += startup.out.trim() + "\n\n";
        // The artifact header renders `epoch:epoch:<hex>` (label + value
        // that itself carries an `epoch:` prefix) — capture the hex.
        const m = /epoch:epoch:([0-9a-f]+)/.exec(startup.out) ?? /epoch:([0-9a-f]+)/.exec(startup.out);
        if (m) epoch = m[1];
        startupInjected.add(sid);
        injectedStartup = true;
        lastEpoch.set(sid, epoch);
      }
    }
    // 2. Task context for this prompt. No `--hook`: that flag is the
    // passive Claude-hook opt-in gate (silent unless
    // context.inject_task_focus=true in .scc/config.yaml). This
    // extension IS the injection decision-maker, so it calls the
    // command directly with the same 1500-token focus budget the hook
    // mode would use.
    if (prompt) {
      const task = await scc(pi, ["context", "task", prompt, "--budget", "1500"], ctx.cwd);
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
        details: {
          source: "scc",
          injectedAt: Date.now(),
          hasStartup: injectedStartup,
          epoch: epoch || lastEpoch.get(sid) || "",
        },
      },
    };
  });

  // tool_result: post-mutation incremental refresh.
  //  - edit/write: known path -> `scc index --paths <path> --quiet`
  //  - bash/patch: opaque mutation -> diff git porcelain snapshots
  //    taken on successive opaque mutations; precise paths when the
  //    delta is known, full quiet index only when git change detection
  //    is unavailable. A failed reindex after a successful source
  //    mutation is surfaced on stderr (never silently ignored).
  pi.on("tool_result", async (event: ToolResultEvent, ctx: ExtensionContext) => {
    if (isFileMutation(event.toolName)) {
      const path = editedPath(event.input);
      if (!path) return;
      const res = await scc(pi, ["index", "--paths", path, "--quiet"], ctx.cwd);
      if (res.code !== 0) {
        console.error(`scc: post-edit reindex failed for ${path} (exit ${res.code})`);
      }
      return;
    }
    if (event.toolName !== "bash" && event.toolName !== "patch") return;
    // Opaque mutation: snapshot git status now and diff against the last
    // snapshot (kept per cwd — runtime insert/update Map). The first
    // opaque mutation in a process only establishes the baseline.
    const now = await gitSnapshot(pi, ctx.cwd);
    const prev = opaqueSnapshot.get(ctx.cwd);
    opaqueSnapshot.set(ctx.cwd, now);
    if (prev === undefined) return;
    const before = new Set(porcelainPaths(prev));
    const changed = porcelainPaths(now).filter((p) => !before.has(p));
    if (changed.length > 0) {
      const res = await scc(pi, ["index", "--paths", ...changed, "--quiet"], ctx.cwd);
      if (res.code !== 0) {
        console.error(`scc: post-bash reindex failed for ${changed.length} path(s) (exit ${res.code})`);
      }
      return;
    }
    // No git-visible delta and empty snapshots on both sides: git change
    // detection is unavailable (non-Git SCC project) — safe full-index
    // fallback. Two NON-empty snapshots with no delta means the command
    // did not touch tracked/untracked source; nothing to do.
    if (now.length === 0 && prev.length === 0) {
      const res = await scc(pi, ["index", "--quiet"], ctx.cwd);
      if (res.code !== 0) {
        console.error(`scc: fallback full reindex failed (exit ${res.code})`);
      }
    }
  });

  // Per-cwd git snapshots for opaque-mutation change detection.
  const opaqueSnapshot = new Map<string, string[]>();

  // session_switch / session_branch / session_tree: the conversation
  // context changed wholesale — reset the marker so the next real prompt
  // re-injects. (A duplicate startup is cheap; missing architecture is
  // not.)
  const resetSession = (_e: unknown, ctx: ExtensionContext) => {
    startupInjected.delete(ctx.sessionManager.getSessionId());
  };
  pi.on("session_switch", resetSession);
  pi.on("session_branch", resetSession);
  pi.on("session_tree", resetSession);

  // session_before_compact: persist a checkpoint so the architecture
  // survives compaction.
  pi.on("session_before_compact", async (_event, ctx: ExtensionContext) => {
    await scc(pi, ["checkpoint", "save"], ctx.cwd);
  });

  // session.compacting: REAL rehydration. OMP injects the returned
  // `context` entries as <additional-context> into the compaction
  // summarizer prompt, so the compacted summary carries the SCC
  // architecture and task state forward instead of dropping them.
  pi.on("session.compacting", async (_event: SessionCompactingEvent, ctx: ExtensionContext) => {
    const context: string[] = [];
    const startup = await scc(pi, ["context", "startup"], ctx.cwd);
    if (startup.code === 0 && startup.out.trim()) {
      context.push(`<scc-startup>\n${startup.out.trim()}\n</scc-startup>`);
    }
    const checkpoint = await scc(pi, ["checkpoint", "load"], ctx.cwd);
    if (checkpoint.code === 0 && checkpoint.out.trim()) {
      context.push(`<scc-checkpoint>\n${checkpoint.out.trim()}\n</scc-checkpoint>`);
    }
    if (context.length === 0) return;
    return { context };
  });

  // session_compact: after compaction the startup capsule is gone from
  // context; clear the once-per-session marker so the next real prompt
  // re-injects startup. The compacted summary carries the architecture
  // (via session.compacting), so this re-injection is the fused artifact
  // refresh, not a duplicate.
  pi.on("session_compact", (_event: SessionCompactEvent, ctx: ExtensionContext) => {
    startupInjected.delete(ctx.sessionManager.getSessionId());
  });
}