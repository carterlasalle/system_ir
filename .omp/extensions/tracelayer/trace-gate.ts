// TraceLayer × Oh My Pi — native pre-mutation gate and stop gate.
//
// This file is the extension entry declared by package.json in this
// directory. `trace install --agent omp` installs the whole package at:
//   .omp/extensions/tracelayer/          project scope
//   ~/.omp/agent/extensions/tracelayer/  global scope
// and `omp install ./adapters/oh-my-pi` links the same directory as a
// plugin. Do NOT copy this file loose into extensions/ — the legacy raw
// layout double-registers the factory and was removed deliberately.
//
// Type-only import (erased at runtime): the canonical package name is
// @oh-my-pi/pi-coding-agent (omp 18.x); legacy runtimes used
// @earendil-works/pi-coding-agent. The extension runtime never type-checks
// this file.
import { spawn } from "node:child_process";
import { appendFileSync, mkdirSync, readdirSync, statSync, unlinkSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { basename } from "node:path";
import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";

// trace:v1 id=impl.omp.trace-gate work=WORK-TL-001
export default function hook(pi: ExtensionAPI): void {
  // Single-registration guard: the manifest declares both `pi.extensions`
  // and `omp.extensions` (old vs new runtimes each honor one key, but some
  // runtimes honor BOTH, invoking this factory twice in one process and
  // double-registering every handler — 2x gate latency, duplicate banners,
  // duplicate receipts). Same-file double load shares the module instance,
  // so one flag suffices. Cross-scope installs (global + project) are
  // separate files; `trace install` warns about those instead.
  // trace:exempt reason=internal-helper
  const g = globalThis as unknown as Record<string, unknown>;
  if (g.__tracelayer_gate_installed === true) return;
  g.__tracelayer_gate_installed = true;
  // Single-gate election: OMP may load TWO copies of this file at once
  // (npm plugin + project/global extension are separate files, possibly
  // separate JS realms, so the flag above cannot dedupe them). Both
  // registering handlers doubles every hook subprocess — proven by CPU
  // profile 2026-09-10: two trace-gate.ts instances, ~6.9s of spawnSync
  // on the event loop in one tool-result. Exactly one copy per OMP
  // *process* wins via an atomic exclusive-create lock; losers register
  // nothing. Stale locks from dead PIDs are swept (signal 0); a lock for
  // a recycled PID belonging to an unrelated live process is harmless
  // (it names a pid nobody claims against).
  let claimedHere = false;
  // trace:v1 id=impl.omp.gate-election work=WORK-p0-remediation-passive-activation-remind-first-enforcement-branch-safe-identity-safe-bootstrap
  const claimPrimary = (): boolean => {
    try {
      const dir = `${homedir()}/.trace/var/gate`;
      mkdirSync(dir, { recursive: true });
      try {
        for (const f of readdirSync(dir)) {
          const m = /^(\d+)\.lock$/.exec(f);
          if (!m || Number(m[1]) === process.pid) continue;
          try {
            process.kill(Number(m[1]), 0);
            // Alive pid, but a lock older than a day cannot belong to a
            // live OMP session holding this HOME: the pid was recycled by
            // an unrelated process after the claimant died. Reclaim it;
            // worst case the loser re-claims next firing (cheap, rare).
            try {
              const ageMs = Date.now() - statSync(`${dir}/${f}`).mtimeMs;
              if (ageMs > 24 * 3600 * 1000) unlinkSync(`${dir}/${f}`);
            } catch {
              // sweep is hygiene, not correctness
            }
          } catch (e) {
            const gone =
              e !== null && typeof e === "object" && "code" in e && e.code === "ESRCH";
            if (gone) {
              try {
                unlinkSync(`${dir}/${f}`);
              } catch {
                // sweep is hygiene, not correctness
              }
            }
          }
        }
      } catch {
        // sweep is hygiene, not correctness
      }
      if (claimedHere) return false; // sibling copy in this process won
      try {
        writeFileSync(`${dir}/${process.pid}.lock`, "tracelayer-gate", { flag: "wx" });
        claimedHere = true;
        return true;
      } catch {
        // EEXIST on our own pid file without a sibling claim means a stale
        // lock from a dead previous incarnation (sweep skips own pid):
        // reclaim once, then honor a second failure as a live sibling.
        try {
          unlinkSync(`${dir}/${process.pid}.lock`);
          writeFileSync(`${dir}/${process.pid}.lock`, "tracelayer-gate", { flag: "wx" });
          claimedHere = true;
          return true;
        } catch {
          return false;
        }
      }
    } catch {
      return true; // FS unavailable: proceed rather than disable the gate
    }
  };
  if (!claimPrimary()) return;
  // Bounded gate transport: one subprocess call per firing. The timeout is
  // a tripwire, not a budget — healthy hooks finish in ~2s (receipt
  // 2026-09-09), so 60s means "hung", never "slow". Without it a hung hook
  // hangs the tool call until the user aborts the request.
  // trace:exempt reason=internal-helper
  // Request-path tripwire (pre/post mutation): 4s. Fresh-log receipt
  // 2026-09-10: p95 pre/post ≈ 2s, observed request abort ≈ 5.4s. A hook
  // that cannot answer in 4s is down; fail open loudly (obligations persist
  // to Stop/CI) rather than hang the request into an abort. Completion
  // (stop) keeps a 60s budget: ending a session may wait, edits may not.
  const GATE_TIMEOUT_MS = 4_000;
  const STOP_TIMEOUT_MS = 60_000;

  // Telemetry: one JSONL record per firing (success, deny, crash, timeout)
  // into ~/.trace/var/timing.log — the same file `trace timing` reads.
  // Best-effort: telemetry never breaks the gate.
  // trace:v1 id=impl.omp.gate-telemetry work=WORK-TL-005 satisfies=REQ-mutation-enforcement-is-reminder-first-by-default
  const logGate = (event: string, durationMs: number, extra: Record<string, unknown>): void => {
    try {
      const dir = `${homedir()}/.trace/var`;
      mkdirSync(dir, { recursive: true });
      let repo: string | undefined;
      try {
        repo = basename(process.cwd());
      } catch {
        repo = undefined;
      }
      appendFileSync(
        `${dir}/timing.log`,
        JSON.stringify({
          ts: new Date().toISOString(),
          source: "omp-gate",
          event,
          repo,
          duration_ms: Math.round(durationMs * 10) / 10,
          ...extra,
        }) + "\n",
      );
    } catch {
      // telemetry must never break the gate
    }
  };

  // Async transport: the subprocess is awaited, never join-blocked. The
  // old spawnSync parked OMP's entire JS event loop for the whole hook
  // (profile 2026-09-10: ~6.9s frozen per tool-result across two loaded
  // copies). Handlers are async and OMP awaits them, so awaiting here
  // yields the loop while the hook runs. stdin is written explicitly
  // (Bun's sync spawn ignores `input`; node async spawn takes it).
  // Transport binary: every other harness invokes the `trace` console
  // script directly; `uv run` adds ~200-260ms resolver overhead per firing
  // and SIGKILL then reaps the uv parent while the real trace grandchild
  // survives. TRACE_GATE_BIN overrides (dev skew: checkout vs installed).
  // trace:exempt reason=internal-detail
  const TRACE_BIN: string[] = (() => {
    const override = process.env.TRACE_GATE_BIN;
    if (typeof override === "string" && override) return [override];
    return ["trace"];
  })();
  // trace:v1 id=impl.omp.gate-transport work=WORK-TL-005 satisfies=REQ-mutation-enforcement-is-reminder-first-by-default
  const run = (
    args: string[],
    input: string,
  ): Promise<{ code: number; out: string; err: string; transportOk: boolean; timedOut: boolean }> =>
    runWith(args, input, GATE_TIMEOUT_MS);
  // trace:exempt reason=internal-detail
  const runWith = (
    args: string[],
    input: string,
    budgetMs: number,
  ): Promise<{ code: number; out: string; err: string; transportOk: boolean; timedOut: boolean }> =>
    new Promise((resolve) => {
      let out = "";
      let err = "";
      let done = false;
      // trace:exempt reason=internal-detail
      const finish = (r: {
        code: number;
        out: string;
        err: string;
        transportOk: boolean;
        timedOut: boolean;
      }): void => {
        if (done) return;
        done = true;
        clearTimeout(timer);
        resolve(r);
      };
      // Direct `trace` spawn: `uv run` adds ~200-260ms resolver overhead
      // per firing, and SIGKILL then reaps the uv parent while the real
      // trace grandchild survives. A missing binary surfaces as an async
      // ENOENT error below and fails open (logged), same as any transport
      // failure — no second spawn path to maintain.
      let child: ReturnType<typeof spawn>;
      try {
        child = spawn(TRACE_BIN[0], [...TRACE_BIN.slice(1), ...args]);
      } catch {
        finish({ code: -1, out: "", err: "spawn threw", transportOk: false, timedOut: false });
        return;
      }
      const timer = setTimeout(() => {
        try {
          child?.kill("SIGKILL");
        } catch {
          // already gone
        }
        finish({
          code: -1,
          out,
          err: `${err}\ngate timeout after ${budgetMs / 1000}s`.slice(-500),
          transportOk: false,
          timedOut: true,
        });
      }, budgetMs);
      const proc = child;
      proc.stdout?.on("data", (d: unknown) => {
        out += String(d);
        if (out.length > 65536) out = out.slice(-65536);
      });
      proc.stderr?.on("data", (d: unknown) => {
        err += String(d);
      });
      proc.on("error", (e: unknown) => {
        finish({ code: -1, out, err: `${err}\n${String(e)}`.slice(-500), transportOk: false, timedOut: false });
      });
      proc.on("close", (code: number | null) => {
        finish({ code: code ?? -1, out, err: err.slice(-500), transportOk: true, timedOut: false });
      });
      try {
        proc.stdin?.write(input);
        proc.stdin?.end();
      } catch {
        finish({ code: -1, out, err: "stdin write failed", transportOk: false, timedOut: false });
      }
    });

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
  // Only exit 2 (policy deny) blocks. A hook that crashes, times out, or
  // exits anything else is a transport failure, not a verdict: fail open
  // (post-mutation coaching plus Stop/CI still enforce) and leave the
  // receipt in the timing log. Never block on a crash with a generic
  // message again.
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
    const start = performance.now();
    const res = await run(["hook", "pre-mutation", "--format", "json"], body);
    const ms = performance.now() - start;
    if (res.timedOut || !res.transportOk || (res.code !== 0 && res.code !== 2)) {
      logGate("pre-mutation", ms, {
        code: res.code,
        timedOut: res.timedOut,
        transportOk: res.transportOk,
        error: res.err || undefined,
        outcome: "allow-on-error",
        path: path || undefined,
      });
      return;
    }
    logGate("pre-mutation", ms, {
      code: res.code,
      outcome: res.code === 2 ? "deny" : "allow",
      path: path || undefined,
    });
    if (res.code !== 2) return;
    let reason = "trace policy blocks this edit";
    try {
      const d = JSON.parse(res.out) as { output?: string };
      if (typeof d.output === "string" && d.output) reason = d.output;
    } catch {
      // fall through to stderr
    }
    if (reason === "trace policy blocks this edit" && res.err) reason += `\n\nHook stderr: ${res.err}`;
    return { block: true, reason };
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
    let loggedPath: string | undefined;
    if (opaque) {
      // Pass the shell command (truncated, matched server-side, never
      // echoed) so the hook can coach test-run evidence ingest.
      const rawInput = (event.input ?? {}) as Record<string, unknown>;
      const cmd = typeof rawInput.command === "string" ? rawInput.command.slice(0, 500) : undefined;
      body = JSON.stringify({ session_id: sessionId(ctx), ...(cmd ? { command: cmd } : {}) });
    } else {
      const { path } = fileInfo(event.input);
      if (!path) return;
      loggedPath = path;
      body = JSON.stringify({ path, session_id: sessionId(ctx) });
    }
    const start = performance.now();
    const res = await run(["hook", "post-mutation", "--format", "json"], body);
    const ms = performance.now() - start;
    if (res.code !== 0) {
      logGate("post-mutation", ms, {
        code: res.code,
        timedOut: res.timedOut,
        error: res.err || undefined,
        outcome: "hook-error",
        path: loggedPath,
      });
      // Surface the failure to the model (append-only, never replacing the
      // real result): silent coaching drops leave the agent unaware that
      // obligations may be pending. The write already happened; Stop/CI
      // still enforce.
      if (Array.isArray(event.content)) {
        return {
          content: [
            ...event.content,
            {
              type: "text",
              text: `\n\n<TraceLayer>\nPost-mutation check failed to run (exit ${res.code}${res.timedOut ? ", timed out" : ""}); obligations may be pending — run \`trace verify\` before completing.\n</TraceLayer>`,
            },
          ],
        };
      }
      return;
    }
    logGate("post-mutation", ms, { code: 0, outcome: "coached", path: loggedPath });
    try {
      const d = JSON.parse(res.out) as { output?: string };
      if (typeof d.output !== "string" || !d.output) return;
      // Never replace a result we cannot safely append to: a non-array
      // content shape must pass through untouched, or the model loses the
      // real tool output and its next edit goes wrong.
      if (!Array.isArray(event.content)) return;
      return {
        content: [
          ...event.content,
          {
            type: "text",
            text: `\n\n<TraceLayer>\n${d.output}\n</TraceLayer>`,
          },
        ],
      };
    } catch {
      return;
    }
  });

  // Fail-closed completion gate: block while trace obligations or verify
  // fail. SessionStopEventResult carries decision/reason (or continuation
  // fields) — not a `block` property. The engine's stop hook ALSO runs the
  // merge-grade auto-finalizer internally.
  pi.on("session_stop", async (event, _ctx) => {
    const body = JSON.stringify({ lifecycle: "wip", session_id: event.session_id });
    const start = performance.now();
    const res = await runWith(["hook", "stop", "--format", "json"], body, STOP_TIMEOUT_MS);
    const ms = performance.now() - start;
    if (res.code === 0) {
      logGate("stop", ms, { code: 0, outcome: "allow" });
      return;
    }
    if (res.code === 2) {
      logGate("stop", ms, { code: 2, outcome: "deny" });
      let reason = "trace verification has blocking failures";
      try {
        const d = JSON.parse(res.out) as { output?: string };
        if (typeof d.output === "string" && d.output) reason = d.output;
      } catch {
        // keep default reason
      }
      // Diagnostics go to stderr (the OMP log); the block reason is returned
      // to the session result.
      console.error(`trace gate: ${reason}`);
      return { decision: "block", reason };
    }
    // Transport failure at completion stays fail-closed (obligations are
    // unverified), but says so honestly instead of claiming violations.
    logGate("stop", ms, {
      code: res.code,
      timedOut: res.timedOut,
      error: res.err || undefined,
      outcome: "block-on-error",
    });
    const why = res.timedOut ? `timed out after ${STOP_TIMEOUT_MS / 1000}s` : `failed to run (exit ${res.code})`;
    const reason =
      `trace stop hook ${why}: completion is blocked because obligations could not be verified — ` +
      `not because a violation was found. Run \`trace index --all\` and retry; ` +
      `if it persists, the hook itself is down and obligations stay unverified.${res.err ? `\n\nHook stderr: ${res.err}` : ""}`;
    console.error(`trace gate: ${reason}`);
    return { decision: "block", reason };
  });
}
