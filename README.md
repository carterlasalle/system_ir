# System Context Compiler (SCC)

**Status:** MVP (Phases 1–6) + P1 + Phase 7 semantic precision + M0/M4
benchmarks: TS+Python LSP resolution (pyright + typescript-language-server),
resolution conflict model, differential benchmark, lifecycle/workflow views,
Kubernetes/Terraform/GitHub Actions extraction, SCIP/Narsil CCG/GitNexus
import, OpenTelemetry reconciliation, config refs, trust boundaries, CI
drift policies, Git co-change, failure-pattern detection (DLQ/circuit
breaker/except), TS+Python SDKs, subagent policy, RTK hints, constrained
claims compression, adapter capability manifests, capsule export, Codex
plugin, property/fuzz tests, and a 21-task/8-repo context benchmark (mean
recall 1.000, localization 1.000).

> **Give agents more repository understanding per token, not more repository text.**

SCC continuously compiles code, configuration, infrastructure, runtime
evidence, and declared architectural intent into an evidence-backed machine
model of a software system (`System IR`), then emits small, task-specific
context packs for coding agents. The specification lives in `docs/`; this
repository is the implementation.

## Pipeline

```text
source + config + infra + intent
        │
        ▼
scc-indexer        Reality Graph (SQLite): files, symbols, imports, calls,
                   routes, tests, stores, topics — every fact evidence-backed
        │
        ▼
scc-graph          System IR: components, responsibilities, ownership,
                   flows (sequence/dataflow/architecture), invariants, drift
        │
        ▼
scc-context        Context Compiler: ranked, deduplicated, token-budgeted
                   packs (overview / task / component / flow / impact / verify)
        │
        ▼
scc-cli            CLI + loopback HTTP daemon + MCP (stdio) + Claude Code hooks
```

## Quickstart

```bash
cargo build --release -p scc-cli        # produces target/release/scc

cd /path/to/your/repo
scc init          # creates .scc/config.yaml + database
scc index         # cold index (incremental afterwards; ~50ms for small repos)
scc overview      # startup capsule (L0 identity + L1 system)
scc context task "change transcript normalization"   # bounded task pack
scc setup claude  # installs automatic Claude Code hooks (no slash command)
```

### The six agent-facing operations (MCP tools, CLI, HTTP)

| Operation | Purpose |
|---|---|
| `system_overview` | system purpose, components, boundaries, stores, externals, flows, invariants, freshness |
| `task_context` | bounded task pack: targets, flows, up/downstream, ownership, contracts, invariants, failures, implementation, tests, evidence status |
| `component_context` | responsibility, implementation, deps, ownership, flows, contracts, tests |
| `flow_context` | trigger, steps, branches, data, failures, retries, evidence |
| `impact_context` | affected components/flows/consumers/contracts/data/invariants/tests + risk |
| `verify_context` | freshness, stale facts, graph invariants, drift, low-confidence deps |

```bash
# MCP (newline-delimited JSON-RPC on stdio)
scc mcp

# HTTP daemon (docs/openapi.yaml; binds 127.0.0.1:7777 by default)
scc serve
curl -s -X POST http://127.0.0.1:7777/v1/context/task \
     -d '{"goal":"rename transcript response field"}'
```

## CLI reference

```bash
scc init                     # initialize workspace
scc index [--paths ...]      # cold or incremental index
scc status                   # snapshot, stats, freshness
scc watch                    # filesystem watcher + incremental re-index
scc overview [--json]
scc context task <goal> [--files ...] [--symbols ...] [--budget N] [--json]
scc context component <id>
scc context flow <id>
scc impact [--diff HEAD~1] [files...]
scc verify [--warnings]
scc drift [--json]
scc ci check [--max-severity medium]   # CI gate: invariants + drift policy
scc check-invariants         # graph invariants only; exits nonzero on violation
scc export system-ir.json | system-ir.jsonl | ccg
scc import scip <file>       # import a SCIP index (RESOLVED facts)
scc import ccg <file>        # import a Narsil CCG export
scc import gitnexus <file>   # import a GitNexus-style evidence export
scc import beads <file>      # import Beads task state (.beads/issues.jsonl)
scc import cbm <file>        # import a codebase-memory graph.db.zst snapshot
scc import hindsight <file>  # import a Hindsight memory-bank export
scc context docs <owner/name>  # external library docs via Context7 (labeled)
scc resolve --lsp            # upgrade EXTRACTED calls via pyright (Phase 7)
scc query <terms>            # lexical search over entities + symbols
scc checkpoint save|load     # PreCompact checkpoint (compaction recovery)
scc setup claude             # install Claude Code hooks
scc serve                    # local daemon (HTTP + watcher)
scc mcp                      # MCP stdio server
scc ingest '<json>'          # runtime ingest: OTLP/JSON traces or {source,target,count}
scc runtime status           # observed edge aggregates
scc runtime reconcile        # static-vs-observed reconciliation
scc bench index [--files N] [--lines N]   # latency benchmark on a synthetic repo
scc bench context [--min-recall 0.9]  # ground-truth recall/precision benchmark
scc bench resolution         # native-vs-LSP differential benchmark
scc bench agent --cmd <cmd>  # run the corpus through an external agent command
scc context subagent <goal>  # bounded task pack with explicit scope boundaries
scc context compress <goal> [--cmd <summarizer>] [--claims]  # compression (+typed claims)
scc export capsule.md        # portable startup capsule (any harness)
scc setup codex              # write AGENTS.md with the capsule
scc setup hermes             # install the Hermes plugin (6 tools + skill)
scc embed                    # compute entity embeddings (opt-in ranker)
scc adapters                 # adapter capability manifests (security audit)
scc cochange [--min-commits N]  # git co-change pairs
```

## Evidence and provenance

Every relationship carries a provenance class and confidence
(`docs/SYSTEM_IR_SCHEMA.md` §5):

- **EXTRACTED** — direct syntax/config evidence (1.0)
- **RESOLVED** — cross-file call/import resolution (≥0.95)
- **DECLARED** — `.scc/intent.yaml` architectural intent
- **INFERRED** — heuristic claims, always labeled, never silently trusted
- **STALE** — evidence no longer matches the working tree; excluded from
  trusted context and surfaced as warnings

All ids are content-derived and deterministic: re-indexing identical content
produces identical System IR, which makes full-vs-incremental equivalence
exact (verified by tests, `docs/TEST_PLAN.md` §7).

## Intent and drift

`.scc/intent.yaml` declares components (responsibility/owns), invariants
(severity/scope/enforced-by), and flows (entrypoint/trigger). Intent never
overwrites reality — `scc drift` reports declared components that don't
exist, declared ownership with no supporting write edge, critical invariants
without enforcing tests, and conflicting store writers.

## P1 system semantics

- **Lifecycle views** (`scc context flow <component>-lifecycle`): state machines
  detected from Enum/State/Status/Stage symbols and transition verbs, compiled
  into state/transition/terminal-outcome step flows with provenance.
- **Workflow views**: intent-declared workflows, branching sequences
  (collapsed multi-path steps), and retry/fallback component workflows with
  failure outcomes.
- **Infrastructure extraction**: Kubernetes manifests (deployments, services,
  namespaces, env refs — values never stored), Terraform (stores, deployment
  units, variables, modules), GitHub Actions workflows/jobs, Dockerfiles.
- **Evidence import**: `scc import scip` upgrades symbol/call facts to
  RESOLVED with scip provenance; `scc import ccg` consumes Narsil CCG exports.
- **Runtime reconciliation**: OTLP/JSON trace ingestion (span trees → service
  edges with count/latency/error aggregates), `scc runtime reconcile` reports
  matched / observed-not-static / static-not-observed edges, and `scc verify`
  surfaces the runtime section.
- **CI drift policies**: `scc ci check` fails on graph violations and drift
  findings above a configurable severity (declared flow entrypoints missing,
  flow sinks unreachable, ownership targets missing, unenforced invariants).
- **Semantic precision (LSP)**: `scc resolve --lsp` drives pyright over LSP
  and upgrades EXTRACTED candidate calls to RESOLVED with `lsp-pyright`
  evidence — verified on package re-exports the native resolver cannot link.
- **Config references**: `os.environ["X"]` / `os.getenv("X")` /
  `process.env.X` reads become `configured_by` edges to name-only
  configuration entities.
- **Trust boundaries**: component/unit mapping from deployment units plus
  external-API calls compile `crosses_boundary` edges, surfaced in verify.
- **Agent integrations**: subagent-scoped task packs (SCC-107) and RTK
  output-compression policy hints on every task pack (SCC-206).
- **Context benchmark (M0/M4)**: `benchmarks/tasks.json` — 15 ground-truth
  tasks across 5 fixture repos with hallucination probes; `scc bench context`
  scores recall / precision / localization / budget compliance and fails the
  gate on hallucination violations. Current: mean recall 1.000, localization
  1.000, 0 hallucination violations. Running the benchmark also surfaced and
  fixed real retrieval bugs (docstring-aware substring search, multi-segment
  `src/`-layout module resolution, upstream/downstream graph expansion).
- **Constrained compression**: `scc context compress` applies the structural
  ladder by default; `--cmd` pipes through an external summarizer whose
  output is explicitly labeled INFERRED and size-bounded.
- **Cross-harness onboarding**: `scc export capsule.md` renders the startup
  capsule with a machine-readable header; `scc setup codex` writes AGENTS.md
  (capsule + usage rules + authority ordering) idempotently.
- **TS LSP resolution (SCC-121)**: typescript-language-server integration
  with cold-start retry and import/export binding-hop for barrel re-exports.
- **Resolution conflicts (SCC-125)**: upgrades that change the native target
  become `resolution_conflict` drift findings (4 genuine conflicts found on
  the polyglot fixture — pyright resolved module roots the native index
  missed).
- **Differential benchmark (SCC-126)**: `scc bench resolution` compares
  native vs LSP edge sets per repo (totals: 49 resolved / 20 external /
  0 upgrades / 0 conflicts — the source-root fallback now resolves natively
  what LSP previously caught); the strict ratio gate is exercised by the
  benchres fixture test; the conflicts gate is the default.
- **Full-index invalidation fix**: `scc index` now purges changed files
  before re-extraction (found by the rename property test) — no stale edges
  after import-target renames.
- **Source-root fallback**: module resolution tries src/svc/lib/app/
  services/packages roots — fixes polyglot repos natively.
- **SDKs (SCC-089/090)**: `sdk/typescript` (@scc/sdk) and `sdk/python`
  (scc-sdk) with the six context operations; both suites pass against the
  CLI.
- **Failure patterns (SCC-058)**: except/catch fallback, circuit-breaker,
  and DLQ detection feed symbol `failures` attributes + dlq topic entities.
- **Git co-change (SCC-046)**: `scc cochange` reports files changed together
  across commits; components carry `cochange` enrichment attributes.
- **Property/fuzz tests (TEST_PLAN §6)**: proptest determinism, binary-safe
  extractors, parser no-panic, rename stability (no dangling refs), cycle
  termination — 6 properties, 32 cases each.
- **250k LOC benchmark (SCC-241/244)**: cold index 96.5s (<120s bound), peak
  RSS 217 MiB (<2GB target), incremental P95 92.4s (SCC-242).
- **Adapter manifests (SCC-224/225)**: `scc adapters` lists filesystem/
  network/subprocess/credentials per adapter; default profile enforced by
  tests.
- **Optional semantic ranker (SCC-071)**: embeddings via any OpenAI-compatible
  endpoint (Ollama, OpenAI, self-hosted gateways) — `inference.enabled` +
  `scc embed` stores vectors, the task pack fuses cosine similarity, and an
  optional separate `/rerank` model reorders top candidates. Verified live
  against Ollama (all-minilm): embeddings surfaced 12 relevant symbols vs 6
  lexically for a no-overlap goal. Provider failures degrade to lexical.
- **Hermes plugin (M10)**: `scc setup hermes` installs a native plugin
  (`plugin.yaml` + `register(ctx)`) exposing the six semantic tools plus a
  bundled skill, verified with a mock-ctx contract test in CI.
- **External adapters (SCC-202/203/204/205)**: Beads task-state import +
  active-task enrichment, codebase-memory-mcp graph.db.zst import (zstd +
  SQLite introspection), Hindsight lessons import with below-the-line
  labeling, and Context7 external docs via its MCP server (`scc context
  docs`), all opt-in via config and labeled so external/memory/task content
  never masquerades as repository facts.

## Security

- **Local-first**: loopback-only daemon, no source leaves the machine.
- **Secrets**: config values are never persisted; only variable references.
- **Path sandbox**: symlink escapes and `..` traversal are rejected.
- **Untrusted text**: README-derived content is labeled `DOCUMENTATION` in
  packs, never presented as fact.
- **Repository read-only** MCP surface; the six semantic tools only.

## Repository layout

```text
crates/
  scc-core/      System IR types, provenance, identifiers, token budgeting
  scc-store/     SQLite persistence (WAL + FTS5), migrations, snapshots
  scc-indexer/   scanner, hashing, redaction, tree-sitter extractors
                 (Python, TypeScript), call resolution, incremental refresh
  scc-graph/     component compiler, flow compiler, invariants, impact, drift
  scc-context/   ranking, expansion, dedup, token budgets, pack rendering
  scc-cli/       CLI, HTTP daemon, MCP server, Claude Code plugin, checkpoint
fixtures/        golden repositories (http-service-python, queue-worker-ts,
                 monorepo-acceptance) with expected IR assertions
docs/            the specification this implements
```

## Testing

```bash
cargo test --workspace        # 275 tests: extractor units, golden repos,
                              # incremental==cold equivalence, staleness,
                              # secrets, schema validation, MCP e2e, HTTP e2e,
                              # precision/recall, graph invariants, infra
                              # extraction, lifecycle/workflow views, runtime
                              # reconciliation, CI policy, SCIP/CCG import,
                              # 50k-LOC perf bound, LSP resolution,
                              # trace replay aggregates, context benchmark
```

The acceptance scenario from `docs/PRD.md` §10 is a test: for "rename
`transcript` response field" the task pack identifies the API handlers,
frontend consumer, worker consumer, schema, contracts, tests, and the
raw-transcript immutability invariant **before any edit** — with measured
recall ≥ 0.9 and precision ≥ 0.4 on the fixture ground truth.

## Scope notes (relative to the docs)

Implemented: Phases 1–6 (schema/store, TS+Python reality graph, component
compiler, sequence/dataflow/architecture atlas, context compiler, Claude Code
integration), intent/drift, security P0, CLI/daemon/MCP, export formats.

Not yet implemented (documented as P1/P2 in the docs): LSP/Serena/GitNexus
live adapters, Beads/Hindsight/Context7 integrations, embeddings ranker,
constrained LLM compression, team server, multi-repo contracts.
