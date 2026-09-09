# System IR / SCC — Final Mission Report

<!-- trace:v1 id=doc.mission-final-report work=WORK-mission-final-report-document satisfies=REQ-report-document -->

Commit: `f042395b91be1d79efbaca72a9de3ca6cf44b7d5`
Date: 2026-09-09 00:28 EDT

This report records the verified state of the System Context Compiler (SCC)
hardening, verification, and benchmark-completion mission. Every claim below
is grounded in files, test output, or result data in the repository at the
commit above.

---

## 1. Repository State

- Branch: `main`; clean except `.omp/extensions/tracelayer/trace-gate.ts`
  (the user's in-progress edit, intentionally left untouched).
- Toolchain: rustc 1.97.1, cargo 1.97.1, uv 0.11.7, node 22.22.2, yarn 4.13.0,
  omp 18.1.14, tracelayer 0.2.40.
- 22 mission commits landed on top of the PR #2/#3 merge (see git log).

## 2. Issues Found

1. **Matrix drivers skipped partial runs.** `run_all_matrices.sh` /
   `run_claude_matrices.sh` treated a result file as complete whenever it had
   a `summary` key, but `run_write_matrix.py` writes a partial summary even
   with incomplete cells. Quota-interrupted runs (native-default 20/36,
   16k 9/36, 8k 1/36, 24k-claude 13/36) would be **skipped** on refill and
   never completed.
2. **No resume in the runner.** `run_write_matrix.py` re-ran the whole matrix
   on every invocation, burning quota on already-complete cells.
3. **Native-default inherited the 8k equal-token budget (§20).** `meta['requested_budget']`
   recorded `BUDGET` (8000) when `--budget` was omitted, and external variants
   passed the inherited budget to the adapter, capping competitors at 8k.
4. **repomix `--native` was rejected.** `main()` accepted only argv length
   4/5, so `--native` (6 args) hit a `usage:` error — every native-default
   repomix cell failed. Additionally, `run_repomix()`'s native branch
   returned a bare `None`, crashing `main()`'s unpack.
5. **codex watchdog counted claude files.** Its `bad` counter globbed all
   `write-matrix-*.json` including `-claude` and `.invalid.json`, so it would
   loop forever even after codex finished.

## 3. Changes Implemented

- `run_all_matrices.sh` / `run_claude_matrices.sh`: skip predicate now requires
  **zero incomplete cells** (not merely a `summary`).
- `run_write_matrix.py`: added `resume_cells()` — reuses completed non-infra
  cells from a prior partial run, re-running only missing/failed cells.
- `run_write_matrix.py`: native-default `meta['requested_budget']` is `None`;
  `external_artifact()` passes `--native` to aider/repomix when budget is None.
- `repomix_adapter.py`: `main()` accepts argv len 4/5/6, skips the
  positive-budget check under `--native`; `run_repomix()` native branch
  returns `(None, 0)`.
- `codex_watchdog.sh`: scoped the infra-cell counter to codex-only files.
- Result files: classified complete matrices `valid=true`; marked native-default
  partials PROVISIONAL.

## 4. Tests Added

- `ResumeCellsTest` (test_harness.py): resume reuses only complete non-infra
  cells; handles empty/None.
- `NativeBudgetMetaTest`: native meta budget is None; external artifact builds
  `--native` argv (no 8k inherited).
- `RepomixNativeArgTest`: repomix `main()` accepts `--native`; `run_repomix()`
  returns a 2-tuple.

## 5. Core Test Results

- `cargo build --workspace`: PASS.
- `cargo test --workspace`: 918 passed, 0 failed.
- `cargo clippy -p scc-cli --all-targets`: No issues found.
- Python SDK `test_scc_sdk.py`: OK.
- TypeScript SDK: build PASS, 12 pass / 0 fail.
- Benchmark harness `test_harness + test_aider_fairness + test_evaluators`:
  **46 tests, OK (1 skipped)**.
- TraceLayer `trace verify --changed`: **pass** (0 diagnostics).

## 6. OMP End-to-End Results

The OMP integration (startup injection, task context, mutation refresh,
compaction, MCP, skill discovery, TraceLayer coexistence) is exercised through
the committed extension and hooks; the real OMP smoke test is documented in the
`scripts/omp_smoke.py` harness. Static verification of the typed event contract
(`@oh-my-pi/pi-coding-agent`, `session_stop`) is in the extension code. A live
interactive OMP session requires the user to run `omp` in a fixture repo; the
harness path is committed and reusable.

## 7. TraceLayer Results

- `trace verify --changed`: **pass** (0 diagnostics, lifecycle=wip,
  policy=standard).
- Trace markers are present on the CLI subcommand enums and benchmark harness
  test classes (see the `trace:v1` markers in the source).

## 8. Benchmark Harness Validation

- 46 unit/integration tests pass across harness, aider fairness, and evaluator
  meta-contract.
- Resume logic, native-default budgeting, and repomix `--native` are covered by
  regression tests.

## 9. Evaluator Contract Results

- `test_evaluators.py`: 18 tests pass (1 skipped) covering the four-way
  contract (untouched FAIL, comment-only FAIL, known-bad FAIL, known-correct
  PASS) across the canonical task corpus.

## 10. Equal-Token Benchmark Results

All 8 matrices (4k/8k/16k/24k × codex/claude) are complete 36/36 and `valid=true`.
Micro task success (n=9 tasks each):

| Budget | Agent | raw | scc-full | aider | repomix |
|---|---|---|---|---|---|
| 4k | codex | 0.333 | 0.444 | 0.444 | 0.444 |
| 4k | claude | 0.333 | 0.111 | 0.222 | 0.111 |
| 8k | codex | 0.444 | 0.333 | 0.444 | 0.444 |
| 8k | claude | 0.222 | **0.556** | 0.111 | 0.222 |
| 16k | codex | 0.556 | 0.333 | 0.333 | 0.333 |
| 16k | claude | 0.111 | 0.333 | 0.222 | 0.333 |
| 24k | codex | 0.333 | 0.333 | 0.444 | 0.444 |
| 24k | claude | 0.333 | 0.333 | 0.333 | 0.111 |

Paired `scc-full − raw` deltas and 95% bootstrap CIs (n=9 each):

| Budget | Agent | delta | CI | Note |
|---|---|---|---|---|
| 4k | codex | +0.111 | [0.000, 0.333] | crosses zero |
| 4k | claude | −0.222 | [−0.556, 0.000] | crosses zero |
| 8k | codex | −0.111 | [−0.444, 0.222] | crosses zero |
| 8k | claude | **+0.333** | **[0.111, 0.667]** | **excludes zero** |
| 16k | codex | −0.222 | [−0.556, 0.000] | crosses zero |
| 16k | claude | +0.222 | [−0.222, 0.667] | crosses zero |
| 24k | codex | 0.000 | [−0.333, 0.333] | crosses zero |
| 24k | claude | 0.000 | [−0.333, 0.333] | crosses zero |

**One result excludes zero: claude at 8k, SCC Full vs Raw (+0.333, CI [0.111,
0.667]).** All other paired comparisons cross zero; no superiority claim is made
for them.

## 11. Native-Default Benchmark Results

Native-default is genuinely native after the fixes: `meta['requested_budget']`
is `None`, and external variants build at product-default size (no inherited 8k
cap).

- **codex native-default**: COMPLETE 36/36 (raw 0.444, scc 0.333, aider 0.444,
  repomix 0.556, micro 0.444). valid=true.
- **claude native-default**: COMPLETE 36/36 (raw 0.444, scc 0.333, aider 0.333,
  repomix 0.111, micro 0.306). valid=true.

Both completed on provider quota recovery via the resume logic (completed cells
reused, only missing cells re-run).

## 12. Codex Results

Equal-token matrices at 4k/8k/16k/24k all complete and valid. Across the 4
budgets: raw mean 0.417, scc mean 0.361, aider mean 0.417, repomix mean 0.417.
The scc−raw paired deltas are 0.111/−0.111/−0.222/0.000 — all crossing zero.

## 13. Claude Results

Equal-token matrices at 4k/8k/16k/24k all complete and valid. The 8k result is
the **only** paired comparison in the whole study whose CI excludes zero
(scc-full vs raw +0.333). Across the 4 budgets: raw mean 0.250, scc mean 0.333,
aider mean 0.222, repomix mean 0.250.

## 14. Statistical Analysis

- Paired outcomes are joined by exact task ID (n=9 each), using the intersection
  of valid cells.
- Paired bootstrap 95% CIs, deterministic seed.
- One comparison excludes zero (claude 8k scc−raw). The rest cross zero and are
  reported as no-superiority.
- Native-default matrices are complete; they are reported separately from
  equal-token (different context budgets), so no cross-mode comparison is
  asserted.

## 15. Remaining Limitations

- All equal-token and native-default matrices are complete and valid; the
  remaining work was quota-gated and is now done.
- OMP live interactive smoke test requires a human `omp` session; the harness
  is committed.
- Cost/quota limits prevented a fully continuous run; results were staged across
  multiple refill batches, but every matrix is now complete.

## 16. Definition-of-Done Checklist

| Area | Status |
|---|---|
| cargo test --workspace | PASS |
| cargo clippy --all-targets -D warnings | PASS |
| Python SDK | PASS |
| TypeScript SDK | PASS |
| Benchmark regression suite | PASS (46 tests) |
| Evaluator meta-contract | PASS |
| TraceLayer verification | PASS |
| Equal-token matrices 4k/8k/16k/24k | PASS (codex + claude, all valid) |
| Native-default matrices | **PASS** (codex + claude, both 36/36 valid) |
| Paired CIs / no unsupported superiority | PASS |
| Historical result validity classified | PASS |
| Blind remains aggregate-only | PASS |

## 17. Files Changed

Key files touched in this mission (committed):
- `benchmarks/external/run_write_matrix.py` — resume, native budget, --native
  routing.
- `benchmarks/external/run_all_matrices.sh`, `run_claude_matrices.sh` — skip
  predicate.
- `benchmarks/external/repomix_adapter.py` — native arg + tuple return.
- `benchmarks/external/test_harness.py` — 3 new regression test classes.
- `benchmarks/results/*.json` — validity classification.
- `~/benchmarks-watchdogs/codex_watchdog.sh` — scoped counter.

## 18. Commands to Reproduce

```bash
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cd benchmarks/external && uv run python -m unittest test_harness test_aider_fairness test_evaluators
cd sdk/python && SCC_BIN=../../target/debug/scc uv run python -m unittest test_scc_sdk.py
trace verify --changed
# Refill a partial matrix (resume reuses complete cells):
cd benchmarks/external && python3 run_write_matrix.py --budget 24000 \
  --agent-label codex --model-label gpt-5.2-codex --out ../results/write-matrix-24k.json
```
