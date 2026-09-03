// SCC × Oh My Pi — native lifecycle integration.
//
// This file is the extension entry declared by package.json in this
// directory. `scc setup omp` installs the whole package at
// `.omp/extensions/scc/` (project scope). It registers ONE module that
// owns ALL ordering-dependent SCC lifecycle behavior via native OMP
// events — there is no cross-module or filename-ordering dependence.
//
// Lifecycle events wired here (verified against the current OMP API):
//   session_start / session_switch / session_branch / session_tree
//                          -> reset the startup-injection marker
//   before_agent_start     -> the ONLY model-visible injection point for
//                             normal turns: startup (once per branch)
//                             + task context/prompt
//   tool_call              -> snapshot dirty files before opaque tools
//   tool_result            -> post-edit `scc index --paths` (edit/write
//                             AND opaque bash/patch/generator mutations);
//                             index failure is retried then reported
//   session_before_compact  -> `scc checkpoint save`
//   session.compacting      -> inject startup + checkpoint into the
//                             compaction result (`{ context: [...] }`)
//   session_compact        -> fallback reset if compacting did not run
//
// The canonical public type is `ExtensionAPI`. The runtime resolves the
// `@oh-my-pi/pi-coding-agent` package (verified in the bundled binary's
// virtual-module map — the `@oh-my-pi` scope is canonical).
import type {
  BeforeAgentStartEvent,
  ExtensionAPI,
  ExtensionContext,
  SessionCompactEvent,
  SessionStartEvent,
  ToolResultEvent,
} from "@oh-my-pi/pi-coding-agent";

// Honor SCC_BIN (claimed by `scc setup omp`) — never hard-code "scc" as
// the only lookup. A static path may also be written into .omp/mcp.json
// at setup time when SCC_BIN is set in the installer environment.
// trace:exempt reason=const-data
const SCC_BIN = process.env.SCC_BIN || "scc";

// Sessions/branches that already received the startup capsule this process.
// Keyed by session id PLUS the active leaf/file when available: a Set of
// session IDs is not enough when the session id stays the same but the
// active branch no longer contains the injected message (switch/branch/tree
// /resume/compaction).
const startupInjected = new Set<string>();

// toolCallId -> fingerprint map captured on tool_call for opaque tools.
const dirtySnapshots = new Map<string, Map<string, string>>();

// True when session.compacting already rehydrated the compacted context
// so session_compact (post-notification) must not wipe the marker.
let compactingRehydrated = false;

// trace:exempt reason=internal-helper
const onEvent = (
  pi: ExtensionAPI,
  event: string,
  handler: (event: unknown, ctx: ExtensionContext) => unknown,
): void => {
  (pi.on as unknown as (type: string, handler: (event: unknown, ctx: ExtensionContext) => unknown) => void)(
    event,
    handler,
  );
};

// trace:exempt reason=internal-helper
const scc = async (
  pi: ExtensionAPI,
  args: string[],
  cwd: string,
): Promise<{ code: number; out: string; err: string }> => {
  try {
    const res = await pi.exec(SCC_BIN, args, { cwd });
    return {
      code: res.code,
      out: String(res.stdout ?? ""),
      err: String(res.stderr ?? ""),
    };
  } catch (e) {
    return { code: -1, out: "", err: e instanceof Error ? e.message : String(e) };
  }
};

// trace:exempt reason=internal-helper
const execBin = async (
  pi: ExtensionAPI,
  bin: string,
  args: string[],
  cwd: string,
): Promise<{ code: number; out: string }> => {
  try {
    const res = await pi.exec(bin, args, { cwd });
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

// Read-only tools never mutate the working tree. Everything else is treated
// as potentially opaque (bash, patch, generators, formatters, scripts).
// trace:exempt reason=internal-helper
const READ_ONLY_TOOLS = new Set([
  "read",
  "grep",
  "glob",
  "find",
  "ls",
  "search",
  "semsearch",
  "websearch",
  "webfetch",
]);

// trace:exempt reason=internal-helper
const isFileMutation = (toolName: string): boolean =>
  toolName === "edit" || toolName === "write";

// trace:exempt reason=internal-helper
const isOpaqueMutation = (toolName: string): boolean =>
  !READ_ONLY_TOOLS.has(toolName) && !isFileMutation(toolName);

// trace:exempt reason=internal-helper
const editedPath = (input: Record<string, unknown>): string => {
  if (typeof input.path === "string") return input.path;
  // Current OMP edit tool: the model passes a structured edit-command
  // DSL string (`{"input": "[path#tag]\nPUT ...", "i": "..."}`). Without
  // this branch the post-edit reindex NEVER fires (verified against a
  // real 18.0.11 session log: STALE after every edit).
  const dsl = input.input;
  if (typeof dsl === "string") {
    const m = /^\[([^\]#]+)(#[^\]]+)?\]/.exec(dsl);
    if (m) return m[1];
  }
  if (typeof input.file_path === "string") return input.file_path;
  return "";
};

// Does the RESUMED conversation already carry an SCC startup capsule?
// Scans session entries for a prior scc-context custom message with
// details.hasStartup. Persisted entries carry customType/details at the
// TOP level (verified against 18.0.11 session files), not nested.
// trace:exempt reason=internal-helper
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
      if (e?.type !== "custom_message" || e.customType !== "scc-context") continue;
      if (e.details?.hasStartup === true) return true;
    }
    return false;
  } catch {
    return false;
  }
};

// Composite injection key: session id stays stable across branch/tree
// navigation, so the leaf/file identity is required to know whether the
// injected startup message is still on the active branch.
// trace:exempt reason=internal-helper
const injectionKey = (ctx: ExtensionContext): string => {
  const sid = ctx.sessionManager.getSessionId();
  const sm = ctx.sessionManager as unknown as {
    getLeafId?: () => unknown;
    getSessionFile?: () => unknown;
    sessionFile?: unknown;
  };
  let leaf = "";
  try {
    const id = sm.getLeafId?.();
    if (typeof id === "string" && id) leaf = id;
  } catch {
    // fall through
  }
  if (!leaf) {
    try {
      const file = sm.getSessionFile?.() ?? sm.sessionFile;
      if (typeof file === "string" && file) leaf = file;
    } catch {
      // fall through
    }
  }
  return leaf ? `${sid}::${leaf}` : sid;
};

// trace:exempt reason=internal-helper
const resetInjection = (ctx: ExtensionContext): void => {
  const sid = ctx.sessionManager.getSessionId();
  for (const k of [...startupInjected]) {
    if (k === sid || k.startsWith(`${sid}::`)) startupInjected.delete(k);
  }
};

// Porcelain + untracked + vs-HEAD names, fingerprinted with git hash-object
// so an already-dirty file that bash mutates further is still detected.
// trace:exempt reason=internal-helper
const parsePorcelainPaths = (out: string): string[] => {
  const paths: string[] = [];
  for (const line of out.split("\n")) {
    if (line.length < 4) continue;
    let rest = line.slice(3);
    const arrow = rest.indexOf(" -> ");
    if (arrow >= 0) rest = rest.slice(arrow + 4);
    const p = rest.trim().replace(/^"|"$/g, "");
    if (p) paths.push(p);
  }
  return paths;
};

// trace:exempt reason=internal-helper
const snapshotDirty = async (pi: ExtensionAPI, cwd: string): Promise<Map<string, string>> => {
  const map = new Map<string, string>();
  const status = await execBin(pi, "git", ["status", "--porcelain=v1", "-uall"], cwd);
  const names = new Set<string>(parsePorcelainPaths(status.out));
  const diff = await execBin(pi, "git", ["diff", "--name-only", "HEAD"], cwd);
  if (diff.code === 0) {
    for (const p of diff.out.split("\n")) {
      if (p.trim()) names.add(p.trim());
    }
  }
  for (const p of names) {
    const h = await execBin(pi, "git", ["hash-object", "--", p], cwd);
    map.set(p, h.code === 0 && h.out.trim() ? h.out.trim() : "missing");
  }
  return map;
};

// trace:exempt reason=internal-helper
const dirtySince = (
  before: Map<string, string>,
  after: Map<string, string>,
): string[] => {
  const out: string[] = [];
  for (const [p, hash] of after) {
    if (before.get(p) !== hash) out.push(p);
  }
  for (const p of before.keys()) {
    if (!after.has(p)) out.push(p);
  }
  return out;
};

// Index failure is NEVER treated as success: retry once, then report.
// trace:exempt reason=internal-helper
const indexPaths = async (
  pi: ExtensionAPI,
  cwd: string,
  paths: string[],
): Promise<{ ok: boolean; err: string }> => {
  const unique = [...new Set(paths.filter(Boolean))];
  if (!unique.length) return { ok: true, err: "" };
  const args = ["index", "--paths", ...unique, "--quiet"];
  let r = await scc(pi, args, cwd);
  if (r.code !== 0) {
    r = await scc(pi, args, cwd);
  }
  if (r.code !== 0) {
    const err = `scc index --paths failed (exit ${r.code}): ${r.err || r.out || "no output"}`;
    try {
      pi.logger?.error?.(err);
    } catch {
      // logger is optional
    }
    return { ok: false, err };
  }
  return { ok: true, err: "" };
};

// trace:exempt reason=internal-helper
const loadStartupAndCheckpoint = async (
  pi: ExtensionAPI,
  cwd: string,
): Promise<string[]> => {
  const lines: string[] = [];
  const startup = await scc(pi, ["context", "startup"], cwd);
  if (startup.code === 0 && startup.out.trim()) lines.push(startup.out.trim());
  const checkpoint = await scc(pi, ["checkpoint", "load", "--inject"], cwd);
  if (checkpoint.code === 0 && checkpoint.out.trim()) lines.push(checkpoint.out.trim());
  return lines;
};

// trace:v1 id=impl.omp.scc work=WORK-SCC-001 satisfies=REQ-SCC-API
export default function hook(pi: ExtensionAPI): void {
  const resetHandler = async (_event: unknown, ctx: ExtensionContext) => {
    resetInjection(ctx);
    await scc(pi, ["state-path"], ctx.cwd);
  };

  // session_start: precompute/state only. This is a notification — it
  // CANNOT return model context. Reset the injection marker (a fresh
  // session, resume, or newly loaded session re-injects startup on its
  // first real prompt) and do a lightweight state-path presence check.
  pi.on("session_start", async (_event: SessionStartEvent, ctx: ExtensionContext) => {
    await resetHandler(_event, ctx);
  });
  onEvent(pi, "session_switch", resetHandler);
  onEvent(pi, "session_branch", resetHandler);
  onEvent(pi, "session_tree", resetHandler);

  // before_agent_start: THE model-visible injection point for normal turns.
  pi.on("before_agent_start", async (event: BeforeAgentStartEvent, ctx: ExtensionContext) => {
    const prompt = event.prompt;
    if (isConversational(prompt)) return;

    let content = "";
    let injectedStartup = false;
    // Startup capsule: once per branch (branch-aware key) AND skipped on
    // resumed conversations that already carry an SCC startup capsule
    // (a `-c`/`-r` resume in a fresh process has an empty Set but the
    // architecture is already in context — verified via the session
    // entry scan; re-injecting ~7k tokens would pure-duplicate it).
    const key = injectionKey(ctx);
    if (!startupInjected.has(key) && !sessionHasStartup(ctx)) {
      const startup = await scc(pi, ["context", "startup"], ctx.cwd);
      if (startup.code === 0 && startup.out.trim()) {
        content += startup.out.trim() + "\n\n";
        startupInjected.add(key);
        injectedStartup = true;
      }
    }
    if (prompt) {
      // No `--hook`: that flag is the passive Claude-hook opt-in gate
      // (silent no-op unless context.inject_task_focus=true, default
      // false) — with it, task context NEVER reaches the model (verified:
      // `--hook` prints nothing, the direct call prints the pack). This
      // extension IS the injection decision-maker; the 1500-token focus
      // budget matches hook mode's cap.
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
        details: { source: "scc", injectedAt: Date.now(), hasStartup: injectedStartup },
      },
    };
  });

  // tool_call: snapshot dirty files before opaque mutations so the post
  // hook can index whatever bash/patch/generators actually changed.
  pi.on("tool_call", async (event, ctx) => {
    if (!isOpaqueMutation(event.toolName)) return;
    const id = event.toolCallId || `${event.toolName}:${Date.now()}`;
    dirtySnapshots.set(id, await snapshotDirty(pi, ctx.cwd));
  });

  // tool_result: post-mutation incremental refresh. edit/write index the
  // touched path; opaque tools index the dirty-fingerprint diff. A failed
  // index is retried then surfaced to the model — never silently ignored.
  pi.on("tool_result", async (event: ToolResultEvent, ctx: ExtensionContext) => {
    const paths: string[] = [];
    if (isFileMutation(event.toolName)) {
      const path = editedPath(event.input as Record<string, unknown>);
      if (path) paths.push(path);
    }
    if (isOpaqueMutation(event.toolName) || isFileMutation(event.toolName)) {
      const id = event.toolCallId;
      const before = id ? dirtySnapshots.get(id) : undefined;
      if (id) dirtySnapshots.delete(id);
      const after = await snapshotDirty(pi, ctx.cwd);
      if (before) {
        paths.push(...dirtySince(before, after));
      } else if (isOpaqueMutation(event.toolName)) {
        // No pre-snapshot (tool_call missed): index everything currently dirty.
        paths.push(...after.keys());
      }
    }
    if (!paths.length) return;
    const result = await indexPaths(pi, ctx.cwd, paths);
    if (result.ok) return;
    const content = Array.isArray(event.content) ? [...event.content] : [];
    content.push({
      type: "text",
      text: `\n\n<SCC>\n${result.err}\nPost-edit index did not succeed; subsequent task context may be stale. Re-run \`scc index --paths\`.\n</SCC>`,
    });
    return { content };
  });

  // session_before_compact: persist a checkpoint so architecture + task
  // state can be restored into the compaction result. Registered via
  // onEvent because older published typings omit this event name.
  onEvent(pi, "session_before_compact", async (_event: unknown, ctx: ExtensionContext) => {
    await scc(pi, ["checkpoint", "save"], ctx.cwd);
  });

  // session.compacting: the compaction-result seam. Inject startup +
  // checkpoint NOW so architecture/task state survive immediately — do
  // not wait for the next user prompt. Returns { context: string[] }.
  onEvent(pi, "session.compacting", async (_event: unknown, ctx: ExtensionContext) => {
    const lines = await loadStartupAndCheckpoint(pi, ctx.cwd);
    compactingRehydrated = lines.length > 0;
    if (lines.length) {
      startupInjected.add(injectionKey(ctx));
      return { context: lines };
    }
    return {};
  });

  // session_compact: post-compaction notification. If session.compacting
  // already put SCC context into the summary, keep the injection marker.
  // Otherwise clear it so the next real prompt re-injects (older OMP).
  pi.on("session_compact", (_event: SessionCompactEvent, ctx: ExtensionContext) => {
    if (!compactingRehydrated) {
      resetInjection(ctx);
    }
    compactingRehydrated = false;
  });
}
