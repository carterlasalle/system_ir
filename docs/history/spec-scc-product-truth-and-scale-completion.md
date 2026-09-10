# SCC product truth and scale completion

<!-- trace:v1 id=SPEC-SI-B7YTHVY7 type=spec work=WORK-SI-MMMJA4G6 -->

## Problem

The extraction and resolution layer outperforms the architecture-compilation, packaging, freshness, scaling, and evaluation layers above it: startup context can exceed budget, fixtures pollute architecture, freshness misses added files, the precise gauge never fires, token estimators diverge, and paid benchmark entrypoints run without opt-in.

## Goals

- Paid benchmarks refuse by default with explicit opt-in

- Startup context truthful, bounded, physically grounded

- Freshness detects added, modified, and deleted files

- Honesty gauges report reality

- Benchmark science corrected from existing data only

## Non-goals

- Running paid model benchmarks

- Rewriting SCC into Ripwire or Ix

- Adding database infrastructure or dozens of MCP tools

- Tuning against individual blind misses

## Test strategy

Zero-cost verification only: cargo test and clippy, Python harness unit tests, deterministic fixture tests, aggregate-only analysis of checked-in result JSON. No external model is ever invoked.

## Requirements

### Paid benchmark safety interlock

<!-- trace:v1 id=REQ-SI-AVWY67TJ type=requirement work=WORK-SI-MMMJA4G6 derived_from=SPEC-SI-B7YTHVY7 -->

Paid benchmark entrypoints refuse by default unless SCC_ALLOW_PAID_BENCHMARKS=1 or a local dry-run or mock path is used.

Acceptance:

- Normal invocation exits 2 before any external launch

- Dry-run and mock paths proceed with no model contact

### Current capability ledger

<!-- trace:v1 id=REQ-SI-503JSBGP type=requirement work=WORK-SI-MMMJA4G6 derived_from=SPEC-SI-B7YTHVY7 -->

A repository-backed ledger classifies every major capability with locations, tests, observed behavior, failure modes, and recommended actions.

Acceptance:

- Ledger covers the mission capability list

- Each BROKEN claim cites verified file and line evidence

### Agent-facing correctness and truth

<!-- trace:v1 id=REQ-SI-NX53P4B7 type=requirement work=WORK-SI-MMMJA4G6 derived_from=SPEC-SI-B7YTHVY7 -->

Startup context is bounded and grounded, architecture is role-scoped, provenance survives rendering, freshness detects added files, the precise gauge fires, and skipped files are counted.

Acceptance:

- Rendered tokens never exceed the hard limit

- An added file makes verify report stale

### Benchmark science from existing data

<!-- trace:v1 id=REQ-SI-JFVRJN6E type=requirement work=WORK-SI-MMMJA4G6 derived_from=SPEC-SI-B7YTHVY7 -->

Result manifest, cap-utilization analysis, repo-clustered statistics, and floor and ceiling diagnostics are computed from checked-in result files.

Acceptance:

- Analyses reproduce from benchmarks/results files

- Zero paid model calls during the mission
