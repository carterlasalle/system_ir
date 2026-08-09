<div align="center">

# System Context Compiler

**Compile repositories into evidence-backed system context for coding agents.**

[![CI](https://github.com/carterlasalle/system_ir/actions/workflows/ci.yml/badge.svg)](https://github.com/carterlasalle/system_ir/actions/workflows/ci.yml)
![Rust](https://img.shields.io/badge/Rust-stable-DEA584?logo=rust&logoColor=white)
![SQLite](https://img.shields.io/badge/SQLite-WAL%20%2B%20FTS5-003B57?logo=sqlite&logoColor=white)
![MCP](https://img.shields.io/badge/MCP-6%20tools-000000?logo=modelcontextprotocol&logoColor=white)

[Getting started](docs/IMPLEMENTATION_PLAN.md) · [Context packs](docs/CONTEXT_COMPILER.md) · [System IR schema](docs/SYSTEM_IR_SCHEMA.md) · [Adapters](docs/API_AND_INTEGRATIONS.md) · [Benchmarks](docs/TEST_PLAN.md) · [Contributing](CONTRIBUTING.md)

</div>

SCC continuously compiles code, configuration, infrastructure, runtime evidence, tests, and declared architectural intent into a **verified machine model of the software system** — then emits small, task-specific context packs so AI coding agents start every task with correct system understanding instead of rediscovering it through search. Every fact carries a provenance class and points at the exact evidence that supports it; stale facts never enter trusted context.

> Give agents more repository understanding per token, not more repository text.

## How it works

```mermaid
flowchart LR
    A[Source code] --> B[Reality Graph]
    C[Config and infra] --> B
    D[Runtime traces] --> B
    E[Intent] --> B
    F[External analyzers] --> B
    B --> G[System IR]
    G --> H[System Atlas]
    H --> I[Context Compiler]
    I --> J[Context packs]
    J --> K[Coding agents]
    K --> L[Exact tools]
    L --> A
```

The authority chain is strict:

```text
runtime observation + compiler/LSP facts > deterministic extraction
> declared architecture > high-confidence inference > memory > model assumption
```

Inferred claims are labeled with confidence and evidence and never silently promoted. `STALE` facts are excluded from trusted packs and surfaced as warnings. Conflicts between evidence sources are recorded, not merged.

## Capabilities

| Area | What SCC provides |
|---|---|
| Extraction | Tree-sitter extractors for TypeScript and Python (symbols, imports, calls, routes, tests, store access), infrastructure parsing (Docker, Compose, Kubernetes, Terraform, GitHub Actions), config-reference and failure-pattern post-passes (retry, fallback, DLQ, circuit breakers) |
| Resolution | Cross-file call resolution with conventional source-root fallbacks (`src/`/`svc`/`lib`/`app`), LSP definition resolution (pyright + typescript-language-server), SCIP import, and a resolution-conflict model — disagreements are surfaced, never merged |
| System IR | Component compiler (workspaces, deployment units, directories, intent), responsibilities, ownership, contracts, invariants, trust boundaries, Git co-change |
| Atlas | Machine-readable architecture, sequence, dataflow, lifecycle, and workflow views — no colors, no coordinates, semantics only |
| Context | Six agent-facing operations with hard token budgets that never cut invariants, ownership, or failure behavior; optional semantic ranking via any OpenAI-compatible embedding endpoint plus a separate `/rerank` model |
| Freshness | Content-hash invalidation, incremental indexing with full↔incremental equivalence guarantees, staleness detection, intent↔reality drift, CI gates |
| Runtime | OpenTelemetry trace ingestion, static-vs-observed edge reconciliation, replay-verified aggregates |
| Integrations | Claude Code hooks, Codex AGENTS.md, OpenCode MCP config, Hermes plugin, MCP server, HTTP API, TypeScript and Python SDKs, Beads/CBM/Hindsight/Context7 adapters |
| Security | Local-first, secret redaction, path sandboxing, untrusted-text labeling, adapter capability manifests, no telemetry |

## Quick start

### Prerequisites

- Rust stable
- Optional: `pyright` + `typescript-language-server` (LSP resolution), `ollama` (semantic ranking), `zstd` (CBM adapter), `python3` + `node` (SDK and plugin tests)

```bash
cargo build --release -p scc-cli     # → target/release/scc
cargo test --workspace               # 290 tests
cargo clippy --workspace -- -D warnings
```

### Index your repository

```bash
cd /path/to/your/repo
scc init                                  # .scc/config.yaml + database
scc index                                 # cold index; incremental afterwards
scc overview                              # startup capsule
scc context task "change transcript normalization"
scc setup claude                          # automatic Claude Code hooks
```

The full command surface is in the [CLI reference](docs/API_AND_INTEGRATIONS.md#cli) and `scc --help`.

## Local access

### CLI

```bash
scc overview | context task <goal> | context component <id> | context flow <id>
scc impact [--diff HEAD~1] [files...]
scc verify | drift | ci check
scc bench context | resolution | agent --cmd <agent-cmd> | index
scc import scip|ccg|gitnexus|beads|cbm|hindsight <file>
scc resolve --lsp | scc embed
scc runtime status | reconcile
scc export system-ir.json | jsonl | ccg | capsule.md
```

### Daemon and MCP

The local daemon implements [`docs/openapi.yaml`](docs/openapi.yaml) on loopback only (`security.listen`, default `127.0.0.1:7777`):

| Surface | How to reach it |
|---|---|
| HTTP API | `scc serve` → `http://127.0.0.1:7777` (`/v1/system`, `/v1/context/task`, `/v1/components/{id}`, `/v1/flows/{id}`, `/v1/impact`, `/v1/verify`, `/v1/index`, `/v1/index/status`, `/v1/runtime/traces`) |
| MCP server | `scc mcp` on stdio — exactly the six semantic tools, repository read-only |
| Health | `scc serve` → `http://127.0.0.1:7777/healthz` |

### Agent integrations

| Harness | Setup | What you get |
|---|---|---|
| Claude Code | `scc setup claude` | SessionStart capsule, task-pack injection, post-edit refresh, PreCompact checkpoint + rehydration |
| Codex | `scc setup codex` | AGENTS.md with capsule, usage rules, authority ordering |
| OpenCode | `scc setup opencode` | AGENTS.md + `.opencode/opencode.json` wiring the SCC MCP server |
| Hermes | `scc setup hermes` | Native plugin: six tools + bundled `scc-system-context` skill |

SDKs: TypeScript (`sdk/typescript`, `@scc/sdk`) and Python (`sdk/python`, `scc-sdk`) wrapping the CLI.

## Application workflow

1. Install SCC and run `scc init` in the repository.
2. Run `scc index` — cold on first run, incremental afterwards.
3. Ask for a task pack before planning or editing: `scc context task "<goal>"`.
4. For cross-layer changes, run `scc impact <files>` and honor the reported contracts, invariants, and downstream consumers.
5. Before trusting the model after external changes, run `scc verify`; re-index if it reports staleness.
6. In CI, gate with `scc ci check` and the context benchmark: `scc bench context --min-recall 0.9`.

Start with the [implementation plan](docs/IMPLEMENTATION_PLAN.md) (phases 0–12 and the MVP cut) and the [context compiler spec](docs/CONTEXT_COMPILER.md).

## Architecture

SCC is a Rust workspace with deliberately narrow crate boundaries:

```text
crates/
  scc-core/        System IR types, provenance, identifiers, token budgeting
  scc-store/       SQLite persistence (WAL + FTS5), migrations, snapshots
  scc-indexer/     Scanning, extractors, resolvers, LSP, evidence adapters
  scc-graph/       Components, flows, invariants, boundaries, co-change
  scc-context/     Ranking, expansion, budgets, pack rendering
  scc-cli/         CLI, daemon, MCP, plugins, benchmarks
adapters/          External evidence importers (beads, cbm, hindsight, context7)
sdk/               TypeScript and Python SDKs
plugins/           Hermes plugin package
fixtures/          8 golden repositories with ground-truth expectations
benchmarks/        tasks.json — 21 ground-truth tasks + hallucination probes
docs/              The full specification this implements
```

Extractors and adapters speak one evidence contract; analyzer-specific types never leak into the System IR. The [adapter manifest](docs/SECURITY.md) declares filesystem/network/subprocess/credential use for every evidence source.

## Safety model

SCC intentionally keeps repository analysis local and honest:

- **Local-first by default.** Source never leaves the machine; the daemon binds loopback only; remote inference and embeddings are opt-in.
- **Secrets are never persisted.** Config values are reduced to references; values are redacted before storage.
- **Repository text is untrusted data.** README/docs/comments are labeled (`DOCUMENTATION`, `UNTRUSTED TEXT`) in context packs and never presented as system facts.
- **Path sandboxing.** Symlink escapes and `..` traversal are rejected; the Docker deployment mounts the repository read-only.
- **Provenance honesty.** Inferred and memory-sourced claims are ranked below deterministic evidence; stale facts are excluded from trusted context; conflicts are surfaced.
- **No telemetry.** No repository content is exported; benchmarks emit only aggregate metrics.

The normative requirements are in [docs/SECURITY.md](docs/SECURITY.md).

## Deployment status

SCC ships as a single static binary and supports three modes:

- **Mode A — developer local (default):** `sccd` (or `scc serve`), SQLite, local parsers, loopback HTTP + MCP. No network dependencies.
- **Mode B — CI:** `scc verify`, `scc drift`, `scc impact --diff origin/main...HEAD`, `scc ci check`. CI can fail on stale generated IR, critical drift, ownership violations, broken invariants, or unapproved new boundaries.
- **Mode C — team server:** post-MVP; not built.

Docker runs the daemon against a read-only repository mount with a writable state volume:

```yaml
services:
  scc:
    image: system-context-compiler   # built from the checked-in Dockerfile
    volumes:
      - .:/repo:ro
      - scc-data:/data               # SCC_STATE_DIR=/data (set in the image)
    ports: ["7777:7777"]
```

## Benchmarks

| Benchmark | Result | Target |
|---|---|---|
| Task-context recall (21 tasks / 8 repos) | **1.000** | ≥ 0.95 |
| Task-context localization | **1.000** | — |
| Hallucination violations | **0** | 0 |
| Cold index 250k LOC | 96.5 s | < 120 s |
| Peak RSS 250k LOC | 217 MiB | < 2 GB |
| Incremental P95 250k LOC | 92.4 s | — |
| Task pack generation | ~70 ms | < 500 ms |
| Differential resolution conflicts | 0 | 0 |

Reproduce them: `scc bench context --min-recall 0.9`, `scc bench resolution`, `scc bench index --files 1000 --lines 250`, or `cargo test -p scc-cli --test perf cold_index_250k -- --ignored`.

## Documentation

| Document | Purpose |
|---|---|
| [Implementation plan](docs/IMPLEMENTATION_PLAN.md) | Phases 0–12, MVP cut, per-phase deliverables |
| [Product requirements](docs/PRD.md) | Problem, goals, functional requirements, success metrics |
| [System design](docs/SYSTEM_DESIGN.md) | Architecture, modules, authority model |
| [System IR schema](docs/SYSTEM_IR_SCHEMA.md) | Entities, relationships, provenance, invariants |
| [System IR JSON Schema](docs/system-ir.schema.json) | Machine-validated export contract |
| [Context compiler](docs/CONTEXT_COMPILER.md) | Context levels, ranking, budgets, pack format |
| [Flow compiler](docs/FLOW_COMPILER.md) | Sequence/dataflow/lifecycle compilation |
| [API and integrations](docs/API_AND_INTEGRATIONS.md) | MCP/HTTP/CLI/SDK contracts, harness integrations |
| [Data strategy](docs/DATA_STRATEGY.md) | Storage layers, freshness, incremental indexing |
| [Test plan](docs/TEST_PLAN.md) | Test strategy, benchmarks, quality gates |
| [Security](docs/SECURITY.md) | Threat model, trust boundaries, sandboxing |
| [Deployment and infra](docs/DEPLOYMENT_AND_INFRA.md) | Deployment modes, Docker, observability |
| [Milestones and tickets](docs/MILESTONES.md) · [Epics](docs/EPICS_AND_TICKETS.md) | M0–M10, per-ticket status |
| [OpenAPI](docs/openapi.yaml) | HTTP API contract |

## Contributing

This repository uses protected, squash-only pull requests with required checks. Read [CONTRIBUTING.md](CONTRIBUTING.md) before making changes — it covers the extractor contract, adapter writing, benchmark methodology, and the CI gates (`cargo clippy --workspace -- -D warnings`, the full test suite, and `scc bench context --min-recall 0.9`).
