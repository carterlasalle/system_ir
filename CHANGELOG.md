# Changelog

## [0.1.0] — 2026-08-31

First public release of the System Context Compiler (SCC).

### What it does

SCC compiles a repository into a structured System Atlas — a layered knowledge
graph of components, flows, contracts, and entrypoints — so coding agents start
with real architectural context instead of guessing.

### Highlights

- **Full extraction pipeline.** Tree-sitter parsing for Rust, Go, Java,
  TypeScript, Python, and Kotlin; Rust and Go AST extractors produce entities,
  relationships, imports, and symbols in a single pass.

- **Canonical causal flow graphs.** `FlowGraph` preserves branches, retries,
  joins, and fan-out — the behavioral truth from which surface projections are
  derived.

- **Four-level context stack.** Startup context (architectural overview),
  task context (relevant slices), surface context (API signatures and
  contracts), and runtime context (latency/error aggregates) — each with
  its own budget and ranker.

- **Semantic resolution.** Heuristics nominate candidate symbols, then LSP-backed
  engines resolve cross-file references, conflicts, and ambiguous callsites.

- **External benchmark suite.** 20-repo corpus with independently authored ground
  truth, pinned aider/repomix adapters, A–H harness variants, and automated
  recall/precision measurement against a 0.9 recall gate.

- **Context benchmark gate.** `scc bench context --min-recall 0.9` runs in CI —
  context packs must surface ≥ 90% of ground-truth keys to merge.

- **Model epoch invalidation.** Any change to source, semantic, evidence, intent,
  or runtime generations recomputes the cache epoch, so stale context packs are
  never served.

### SDKs

- **Python SDK** (`scc_sdk.py`). Token-optimized CLI proxy that strips up to 90%
  of bash noise. Supports `pip install` and direct script inclusion.

- **TypeScript SDK** (`scc-sdk`). Pack builder for `scc context --format json`
  output, with typed interfaces and unit tests.

### Tooling

- **CLI.** `scc` binary with subcommands: `context`, `atlas`, `bench` (atlas +
  context + agent), `index`, `info`, `doctor`.

- **MCP server.** Model Context Protocol adapter exposing SCC context as a
  tool for Claude, Codex, and other agents.

- **Hermes plugin.** Native tool registration for the Hermes agent framework,
  with auto-installer and contract test.

### Infrastructure

- **TraceLayer integration.** Mandatory `trace:v1` markers on all behavioral
  boundaries; `trace verify --changed` gates every merge.

- **CI.** GitHub Actions workflow with Build, Clippy (deny warnings), Python SDK
  tests, TypeScript build, benchmark harness, cargo tests, trace policy gate,
  context benchmark gate, Hermes contract test, and SBOM generation.

- **Docker support.** `SCC_STATE_DIR` for writable state outside read-only
  repos; SBOM artifact uploaded per run.

- **OMP integration.** Native extension registers SCC lifecycle with Oh My Pi
  harness; idle YAML hook marked inert.

### Fixes in this release

- `chunks_exact` → `as_chunks` for Rust 1.98 clippy compliance (`lib.rs`,
  `benchatlas.rs`).
- Python SDK: `from __future__ import annotations` for py3.9-safe PEP 604
  unions; typing modernization (`Dict` → `dict`, `Optional[X]` → `X | None`).
- Fixture repairs: `large-ts` test stub (private `db` prop), `go-facts-service`
  `go.sum` regeneration, `java-service` local `@Retryable`/`@Backoff`
  annotations.
- Benchmark scripts: shellcheck warnings resolved (`verify_gt.sh` SC2034/SC2155,
  `run_agent_bench.sh` scoped SC2016 disable).
- Trace obligations: all 102 `lib.rs` symbols and 106 `benchatlas.rs` symbols
  accounted; `get_embedding` exempt marker added.
- CI: `tracelayer` pip install step added to workflow (was missing from runner).
