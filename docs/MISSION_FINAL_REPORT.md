<!-- trace:exempt reason=mission-report-document (reporting artifact, not product behavior) -->
# SCC Final Hardening, Verification, and Benchmark Report

Mission: take system_ir / SCC from "very close" to genuinely complete,
reproducible, scientifically defensible, and production-ready; then run the
real coding-agent experiment and report the truth.

---

## 1. Repository State

- Start: `ddbb073` (main, clean, synced with origin)
- End: `713c766` + result-classification commits (see section 17)
- Toolchain: rustc/cargo 1.97.1, uv 0.11.7, node 22.23.2, yarn 4.13.0,
  omp 18.0.11, codex-cli 0.146.0, Claude Code 2.1.239, tracelayer 0.2.40
  (released during this mission from 0.2.39).
- Baseline validation at start: `cargo test --workspace` 405 passed
  (one flake, root-caused and fixed - see section 2), clippy clean.

## 2. Issues Found

Every issue was reproduced, fixed, regression-covered, and re-verified:

1. **OMP task context NEVER injected (P0)**: the extension called
   `scc context task <prompt> --hook`; `--hook` is the Claude-hook opt-in
   gate (silent no-op unless `context.inject_task_focus=true`, default
   false). Reproduced: `--hook` printed nothing while the plain call
   printed the 8.9KB pack. Fix: the extension calls the command directly
   with `--budget 1500` (the hook mode's focus cap).
2. **OMP post-edit reindex never fired (P0)**: the edit tool's input in
   OMP 18.x is the structured DSL `{"input": "[path#tag]\nPUT ...", "i":
   ...}`; `editedPath` read `input.path` -> undefined -> no reindex.
   Reproduced via a session log (post-edit `scc verify` showed the file
   STALE). Fix: parse the `[path#tag]` prefix; verified E2E (freshness
   green after a real OMP edit).
3. **OMP `.omp/AGENTS.md` shadowed the user's root AGENTS.md (P0)**:
   verified in the OMP 18.0.11 binary that context discovery keys files
   by project depth and the native provider (priority 100) shadows the
   agents-md provider (10) at the same depth - root instructions would
   be silently dropped. Fix: `scc setup omp` emits `@../AGENTS.md` (OMP's
   `@`-import, verified in the loader; a bare `@AGENTS.md` resolves to
   itself and OMP skips it as cyclic - caught via the runtime debug log).
4. **Stale CLI syntax**: the extension used `scc index --path`
   (nonexistent; the real flag is `--paths`). Fixed everywhere.
5. **OMP MCP server failed to spawn**: `.omp/mcp.json` wrote
   `"command": "scc"`; OMP's MCP spawn does not resolve a bare name from
   the project PATH (real session: `Executable not found in $PATH:
   "scc"`). Fix: absolute path (SCC_BIN or current exe). Verified:
   `Connected: scc`.
6. **Installed extension carried dangling cross-repo trace markers**:
   `scc setup omp` wrote the source's authoring marker (WORK-SCC-001 /
   REQ-SCC-API nodes) into user repos -> TL002 dangling edges. Fix: the
   installed copy rewrites the marker to an exempt comment.
7. **TraceLayer crashed on binary changed files**: `_file_level_exempt`
   read with `encoding="utf-8"` catching only OSError; a
   `UnicodeDecodeError` (ValueError) from SCC's SQLite state DB
   (byte 0xfb at offset 106) crashed `trace verify` - blocking every
   TraceLayer-gated completion in an SCC-indexed repo. Fixed in
   tracelayer 0.2.40 (released to PyPI + brew; regression test added).
   Also added `.scc/**` to TraceLayer's default exclusions (SCC state
   is machine-generated, like `.git`).
8. **`scc context startup --budget` below the atlas floor PANICKED**
   (S56 violation): "startup artifact 1899 tokens exceeds hard_max 700
   (15 corrective-loop rounds)". Fix: honest `## BUDGET OVERFLOW` marker
   in OMISSIONS instead of a crash.
9. **Test flake - global git config leaked into tests**: the user's
   `commit.gpgsign=true` made parallel test commits fail in gpg-agent
   ("Cannot allocate memory"). Root-caused via a stress loop; fix:
   repo-local `commit.gpgsign=false` in the test fixture factories.
10. **Evaluator contract violations (P1)**: the writable matrix's
    validators were word-greps ("retry", "page") that a comment-only
    patch could satisfy; two fixtures shipped already-satisfied tasks.
    Replaced with the evaluator registry (section 9).
11. **Claude results file carried codex metadata**: historical
    `write-matrix-claude.json` had `"agent": "codex"` hardcoded - the
    misleading-file issue from the mission brief. All legacy files are
    now machine-classified `valid: false` (S34).
12. **Accidentally committed user file**: my `git add -A` swept in the
    pre-existing untracked `.omp/config.yml` (machine-local model
    roles). Removed from git, gitignored, kept on disk.

## 3. Changes Implemented

- **plugins/omp/scc/index.ts** (rewritten): full lifecycle -
  `session_start` reset; `session_switch/_branch/_tree` resets;
  `before_agent_start` startup-once (with resume dedup via the session
  entry scan) + task context; `tool_result` edit/write reindex
  (`scc index --paths <p> --quiet`) with surfaced failures; bash/patch
  opaque-mutation git-porcelain snapshot diff (precise paths; full-index
  fallback only when git is unavailable); `session_before_compact`
  checkpoint save; `session.compacting` REAL rehydration (fresh startup
  + checkpoint injected as `<additional-context>` into the compaction
  summarizer); `session_compact` marker reset.
- **plugin_omp.rs**: fused-startup durable language (`scc context
  startup`; `scc atlas` demoted to its Level-0 component), root-AGENTS
  preservation import, marker-stripped installable extension, absolute
  MCP path, idempotency tests incl. root-preservation cases.
- **bench external**: `evaluators.py` (behavioral registry; TS fixtures
  explicitly classified structural per S26), `test_evaluators.py`
  (four-way meta-contract + alignment + wiring + Full-SCC-Structural
  regression, 10 tests), `run_write_matrix.py` rewritten (canonical
  corpus by task id, registry evaluators, `scc bench external
  --artifact-only` authoritative builder, equal-token postcondition,
  error typing, agent/model labels, task-ID-intersection paired CIs),
  `report_stats.py`, resumable drivers.
- **scc bench external --artifact-only**: the CLI's authoritative
  artifact builder (startup N/2 + task N/4 + structural remainder, hard
  shrink loop) exposed to external harnesses - the benchmark no longer
  reconstructs SCC semantics in Python (S18/S40).
- **startup.rs**: below-floor budget -> honest BUDGET OVERFLOW marker
  (no panic).
- **ledger_freshness.rs** (new): S53 ledger-visibility and S54
  incremental-refresh/deletion regression tests.
- **CI**: pinned `uv tool install tracelayer==0.2.39` (no pip, no
  continue-on-error), evaluator meta-tests + benchmark suites in CI,
  clippy `--all-features`, uv-managed python steps.
- **tracelayer 0.2.40** (released): UnicodeDecodeError crash fix,
  `.scc/**` default exclusions, regression tests, brew bump, dogfood
  refresh.

## 4. Tests Added

- plugin_omp: root-AGENTS preservation, no-import-when-absent (2 tests)
- ledger_freshness.rs: 3 tests (task-ledger visibility, startup budget
  coupling, edit/delete freshness)
- test_evaluators.py: 10 tests (four-way contract over 9 evaluators,
  registry/canonical derivation, structural classification,
  prompt-evaluator alignment + leakage bans, matrix wiring/labels,
  Full-SCC-contains-Structural-Source)
- tracelayer: binary-file crash regression + default-exclusion list
- CI now runs: harness (13) + aider fairness (9) + evaluator (10)

## 5. Core Test Results

- `cargo test --workspace`: **1092 passed, 0 failed** (serial-threads=4
  after the gpg fix; 25 test binaries)
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  **clean**
- Python SDK: **PASS** (uv run)
- TypeScript SDK: **12/12 PASS** (npm - the repo's existing setup; no
  yarn lockfile)
- Benchmark suites: **32 passed** (test_harness 13, test_aider_fairness
  9, test_evaluators 10)
- TraceLayer gate: `trace verify --changed` **PASS** (0 blocking);
  tracelayer 0.2.40 upstream: 838 tests, ruff, format, vulture, bandit,
  merge-gate PASS
- Evaluator meta-contract: **PASS for every registered task** (untouched
  FAIL, comment-only FAIL, plausible-wrong FAIL, known-good PASS)

## 6. OMP End-to-End Results

Real OMP 18.0.11 sessions on a disposable fixture (`/tmp/scc_omp_e2e/repo`):

- `scc init && scc index && scc setup omp` x3 - **idempotent** (no
  duplicate blocks/imports/JSON; MCP merge preserves existing servers)
- Codex->OMP->Codex->OMP install orders - **coexist** (user content
  first, one SCC section per file, one `@../AGENTS.md` import)
- Fresh session prompt 1 ("which module handles GET /api/transcripts, do
  NOT open files") - **answered correctly from injected context only**
  (`scc-context` custom message, 7664 chars = Atlas + Surface + TASK
  pack; session-file verified)
- Resume (`-c`) prompt 2 - **startup not duplicated** (task-pack-only
  1363-char injection, `hasStartup: false`)
- Edit mutation ("change attempts 3->5") - **post-edit freshness green**
  (`scc verify`: all indexed files match the working tree)
- Write mutation (new `app/format.py`) - **new file immediately
  indexed** (`scc query normalize_ws` finds it)
- Bash mutation - snapshot-diff path verified (fake-pi harness + real
  run)
- Compaction (`/compact`, soft-LLM path) - **rehydration proven with an
  SCC-ONLY fact**: after compaction the agent answered "web_framework,
  MERLIN-99, app/format.py" - the ARCHETYPE exists only in SCC startup,
  never in conversation; codeword + new-file location survived.
  Checkpoint `scc checkpoint load` returns content.
- MCP: `Connected: scc` in-session; the model CALLED `scc/surface_map`
  and returned real surface entries. 10-tool intent-level surface.
- TraceLayer coexistence: both extensions loaded and operated in the
  same sessions; the session_stop fail-closed gate blocked quota-era
  runs that carried trace failures and allowed clean ones.

## 7. TraceLayer Results

- The 18.0.11 binary's real event contracts were extracted and matched:
  `tool_call` `{block, reason}`, `tool_result` `{content}`,
  `session_stop` `{decision: "block", reason}`, `before_agent_start`
  `{message: {customType...}}`, `session.compacting` `{context: [...]}` ->
  `<additional-context>` in the summarizer.
- `@oh-my-pi/pi-coding-agent` is the canonical import (verified against
  the binary's loader; the `@earendil-works` alias is not exposed).
- hooks.yaml: verified INERT in 18.x (no YAML hook loader exists; the
  ambient scan accepts only `.ts`/`.js`) - INERT header restored.
- Released tracelayer **0.2.40** (crash fix + `.scc/**` exclusions):
  PyPI, brew tap, local, dogfood - all refreshed and verified.
- session_stop gate: blocked unsafe completions in real OMP runs (the
  fixture's trace failures produced `trace gate: trace verification has
  blocking failures` and the stop was blocked) and allowed clean ones.

## 8. Benchmark Harness Validation

- One canonical corpus: matrix tasks derive from `benchmarks/tasks.json`
  by task id (9 evaluator-backed tasks; unknown ids are hard errors,
  never silent passes).
- One writable runner for all variants: isolated copy -> artifact ->
  same agent -> same evaluator -> same metrics. Raw receives NO artifact
  (asserted in code and tests).
- Aider/repomix: pinned installs (`external-lock.json`; the bench venv
  holds the pinned aider), adapters verified producing artifacts
  (178-token map / 516-token pack on the probe fixture), equal-token
  caps enforced (test_equal_token_map_never_exceeds_budget).
- SCC full artifact built by the AUTHORITATIVE CLI builder (`scc bench
  external --artifact-only`): 533 tokens at 4000 budget on the probe,
  STRUCTURAL SOURCE + ATLAS + SURFACE sections verified present;
  `actual <= budget` postcondition asserted.
- Error typing: infra (artifact/adapter/evaluator-crash/quota) vs coding
  failures recorded separately; infra cells excluded from numerators
  and from paired sets, and reported.
- Metadata: schema 2 - agent_label/agent_cmd/model_label, commits,
  corpus + evaluator hashes, mode, budget, per-cell context tokens, wall
  time, patch/modified-files, error type.
- Historical files: all 4 legacy matrices + the quota-poisoned reruns
  carry machine-readable `validity` blocks (S34).

## 9. Evaluator Contract Results

9 evaluator-backed tasks; the four-way contract holds for every one
(CI-enforced):

| task | untouched | cosmetic | wrong-patch | known-good |
|---|---|---|---|---|
| http-service.rename-transcript-field | FAIL | FAIL | FAIL | PASS |
| http-service.transcript-normalization | FAIL | FAIL | FAIL | PASS |
| http-service.health-check | FAIL | FAIL | FAIL | PASS |
| py-queue.empty-messages | FAIL | FAIL | FAIL | PASS |
| py-queue.classification-fallback | FAIL | FAIL | FAIL | PASS |
| py-queue.store-changes | FAIL | FAIL | FAIL | PASS |
| queue-worker.asr-retry* | FAIL | FAIL | FAIL | PASS |
| queue-worker.street-vocabulary* | FAIL | FAIL | FAIL | PASS |
| ts-api-web.pagination* | FAIL | FAIL | FAIL | PASS |

(*) classified structural (S26): TS fixtures carry no node_modules by
design; the checks verify the change's semantic shape on the real call
path (a loop wrapping the transcribe call; a vocabulary map APPLIED;
pagination params threaded into the data call) - not word greps. The
Python evaluators are fully behavioral (real sqlite state transitions;
the orthogonal tenacity decorator is stubbed).

Prompt-evaluator alignment (S27): goals state the observable (e.g.
"rename ... to transcriptText") and a test bans target/solution leakage.

## 10. Equal-Token Benchmark Results

VALID matrices (36/36 cells, 0 infra errors unless noted):

| matrix | raw | scc-full | aider | repomix | scc-raw (95% CI) |
|---|---|---|---|---|---|
| codex 4k | 33.3% | 44.4% | 44.4% | 44.4% | +11.1pp [0, +33.3] crosses zero |
| codex 8k+ | 44.4% | 33.3% | 44.4% | 44.4% | -11.1pp [-44.4, +22.2] crosses zero |
| claude 4k | 33.3% | 11.1% | 22.2% | 11.1% | -22.2pp [-55.6, 0] crosses zero |
| claude 8k | 22.2% | 55.6% | 11.1% | 22.2% | **+33.3pp [+11.1, +66.7] EXCLUDES ZERO** |

+ 35/36 completed (1 disclosed infra cell, excluded from numerators/pairs).

16k/24k: every attempt so far was quota-poisoned (codex died again right
after completing 8k; Claude's spend limit persisted) - preserved as
machine-classified INVALID files. Both agents' quotas are the binding
external constraint; the resumable drivers regenerate only missing
budgets, so the remaining matrices need only agent quota, no human step.

## 11. Native-Default Benchmark Results

BLOCKED (agent quotas). First attempt invalidated with a machine-readable
reason. The runner's native-default mode is verified (mode label
`writable-native-default`, no inherited budget) - what is missing is
quota, not infrastructure.

## 12. Codex Results

4k complete (36/36): micro raw 33.3% / scc-full 44.4% / aider 44.4% /
repomix 44.4%; macro(repo) 41.7% / 50.0% / 54.2% / 54.2%. No variant
separates from raw at 4k. 8k (35/36): raw 44.4% / scc-full 33.3% - scc
below raw, CI crosses zero. 16k/24k/native: BLOCKED on daily quota;
first rerun attempts preserved as INVALID.

## 13. Claude Results

4k complete: raw 33.3% / scc-full 11.1% - scc HURT at 4k (CI crosses
zero). 8k complete: raw 22.2% / scc-full 55.6% - scc HELPED, CI excludes
zero. Per-cell at 4k, Claude lost rename + normalization when carrying
artifacts; at 8k scc-full won several tasks raw failed. 16k/24k/native:
BLOCKED on the spend limit (stated reset window did not restore
capacity).

## 14. Statistical Analysis

- 9 paired tasks per matrix; bootstrap CIs pair by exact task id; infra
  cells never enter pairs.
- Exactly one significant result exists across the valid matrices:
  **claude 8k: scc-full - raw = +33.3pp, CI [+11.1, +66.7]** - SCC full
  context improved Claude's task success at the 8k equal-token budget on
  this 9-task corpus.
- Every other comparison crosses zero; two show scc numerically BELOW
  raw (codex 8k, claude 4k) - reported as such.
- Honest summary: **no consistent direction across agents/budgets; one
  significant positive; n=9 makes CIs wide.** The claude-8k effect needs
  replication at 16k/24k and on more tasks before any product claim.

## 15. Remaining Limitations

1. **Agent quotas are the binding external constraint** for 16k/24k and
   native-default on both agents. The infrastructure is complete and
   resumable: reruns require only quota (Codex resets daily ~5:43 AM;
   Claude's spend limit per its stated window).
2. **Corpus scale**: 9 evaluator-backed tasks across 5 fixture repos
   (Python fixtures runnable; TS fixtures classified structural). CIs at
   n=9 are wide.
3. Medium/large repo validity (S51): fixtures are realistic but small;
   monorepo/polyglot/large-ts corpora are indexed for read-only
   benchmarks but have no behavioral evaluators yet.
4. OMP resume-dedup is conservative (any prior startup in the resumed
   conversation suppresses re-injection; post-mutation staleness is
   surfaced in the fresh task pack rather than forcing a new startup).

## 16. Definition-of-Done Checklist

CORE PRODUCT - all PASS: cargo test (1092), clippy all-features, Python
SDK, TypeScript SDK (12/12), benchmark suite (32), evaluator
meta-contract (9/9), TraceLayer verification (+ 0.2.40 release), surface
/startup/task hard caps (startup below-floor now degrades honestly),
ledger actual-visibility, Full-SCC-contains-Structural-Source,
CLI/MCP/plugin/SDK parity (one authoritative builder).

OMP - all PASS: setup exists/idempotent/preserves instructions/install-
order safe/no shadow loss/no SYSTEM.md/current ExtensionAPI/startup
reaches model once/task reaches model/edit+write+bash+patch+new-file+
non-Git refresh/reindex failures surfaced/compaction checkpoint +
fresh-startup injection/MCP merge + connect + tool call/SCC skill
discovered/TraceLayer coexistence + session_stop policy/legacy YAML
inert/real OMP smoke test.

BENCHMARK - all PASS: canonical corpus, four-way contract per task,
prompt-evaluator alignment, one writable runner, raw receives nothing,
aider/repomix/SCC artifacts reach the agent, Structural Source in Full
SCC, equal-token postcondition, native-default mode genuinely uncapped,
completion != success, infra != coding, micro/macro separation, task-id
pairing, reproducibility metadata, historical classification, blind
corpus untouched.

REAL EXPERIMENT:
- 4k equal-token (codex + claude): **PASS**
- 8k equal-token (codex 35/36 + claude): **PASS**
- 16k equal-token: **BLOCKED** (agent quota; infra ready, resumable)
- 24k equal-token: **BLOCKED** (agent quota)
- native-default: **BLOCKED** (agent quota)
- Codex matrix: **PARTIAL** (4k + 8k complete; rest quota-blocked)
- Claude matrix: **PARTIAL** (4k + 8k complete; rest quota-blocked)
- isolation/tokens/success/paired stats/no unsupported claims: **PASS**

## 17. Files Changed

- `plugins/omp/scc/index.ts` (rewritten), `SKILL.md`
- `crates/scc-cli/src/plugin_omp.rs`, `main.rs` (--artifact-only),
  `compress.rs`
- `crates/scc-context/src/startup.rs` (overflow marker)
- `crates/scc-graph/src/{lib,cochange}.rs` (hermetic test git)
- `crates/scc-cli/tests/{ledger_freshness.rs, benchatlas.rs}`
- `benchmarks/external/{evaluators.py, test_evaluators.py,
  run_write_matrix.py, report_stats.py, run_all_matrices.sh,
  run_claude_matrices.sh}`
- `benchmarks/tasks.json` (aligned goals), results/* (validity blocks,
  valid + invalidated matrices)
- `.github/workflows/ci.yml`, `.gitignore`, `README.md`
- tracelayer repo (0.2.40): `config.py`, `policy/rules.py`, tests, brew

## 18. Commands to Reproduce

```bash
cargo test --workspace -- --test-threads=4
cargo clippy --workspace --all-targets --all-features -- -D warnings
cd sdk/python && SCC_BIN=$PWD/../../target/debug/scc uv run python -m unittest test_scc_sdk.py
cd sdk/typescript && npm install && npm run build && SCC_BIN=$PWD/../../target/debug/scc npm test
cd benchmarks/external && uv run python -m unittest test_harness.py test_aider_fairness.py test_evaluators.py
uv tool install tracelayer==0.2.40 && trace verify
# experiments (each cell: isolated copy -> artifact -> agent -> evaluator)
python3 run_write_matrix.py --budget 4000 --agent-label codex --model-label gpt-5.2-codex
python3 run_write_matrix.py --budget 8000 --agent-cmd "claude -p --model claude-sonnet-4-5 --dangerously-skip-permissions" --agent-label claude --model-label claude-sonnet-4-5
python3 report_stats.py          # validity-honoring paired report
# OMP E2E
cd /tmp/fixture && scc init && scc index && SCC_BIN=... scc setup omp && omp
```
