# Implementation Plan
<!-- trace:v1 id=PLAN-SCC-001 type=plan work=WORK-SCC-001 implements=SD-SCC-001 title="10/10 implementation plan, waves 1-9" -->

## Phase 0 — Benchmark harness
10 representative repos, 5–10 ground-truth tasks each, baseline agent/Serena/graph-tool metrics.

## Phase 1 — Schema + storage
System IR schema, Rust types, SQLite, evidence/provenance, migrations, import/export.

## Phase 2 — Reality Graph MVP
TypeScript + Python; files, symbols, imports, calls, routes, tests; incremental indexing.

## Phase 3 — Component compiler
Package/directory/call/import/route/data signals + `.scc/intent.yaml` overrides.

## Phase 4 — System Atlas v1
Architecture, Sequence, Data Flow; route/worker entrypoints; evidence drill-down.

## Phase 5 — Context Compiler
Startup/task/component/flow/impact packs, hard budgets, lexical+graph ranking, benchmark.

## Phase 6 — Claude Code
Automatic SessionStart, task injection, refresh, PreCompact checkpoint, rehydration.

## Phase 7 — Semantic precision
LSP, SCIP, Serena; upgrade candidate facts to RESOLVED.

## Phase 8 — System semantics
DB ownership, queues/events, config, deployment units, retry/fallback, Workflow/Lifecycle.

## Phase 9 — Runtime
OpenTelemetry ingest and static-vs-observed reconciliation.

## Phase 10 — Intent/drift/CI
Architecture intent, ownership/contract/flow drift, PR/CI policies.

## Phase 11 — External adapters
GitNexus, Narsil, Beads, Context7, Hindsight.

## Phase 12 — More harnesses/multi-repo
Codex, Hermes, OpenCode, cross-repo contracts, optional team server.

## MVP cut

Ship after Phase 6 if:
- TS/Python reliable;
- Architecture/Sequence/DataFlow useful;
- context benchmark materially improves exploration;
- Claude Code automatic;
- freshness correct.

Do not block on UI, diagrams, cloud, many languages, embeddings, CodeQL/Joern, runtime, Hindsight.
