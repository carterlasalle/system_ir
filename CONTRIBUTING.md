# Contributing to System Context Compiler

Thanks for contributing. This guide covers the development workflow, code
standards, testing expectations, and how to add new extractors, adapters,
and benchmarks.

## Getting started

```bash
git clone https://github.com/carterlasalle/system_ir.git
cd system_ir
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings   # CI runs this as a hard gate
```

Prerequisites:

- Rust (stable; the CI uses `dtolnay/rust-toolchain@stable`)
- Optional: `pyright` + `typescript-language-server` (LSP resolution tests
  skip gracefully when absent), `ollama` (semantic-rank E2E), `zstd`
  (CBM adapter tests), `python3` + `node` (SDK and plugin tests)

## Repository map

| Path | What lives here | Touched when |
|---|---|---|
| `crates/scc-core` | IR types, provenance, ids, token budgeting | almost never — change carefully, everything depends on it |
| `crates/scc-store` | SQLite schema, migrations, FTS5 | adding storage |
| `crates/scc-indexer` | scanning, extractors, resolvers, LSP, adapters | most feature work |
| `crates/scc-graph` | component/flow/invariant compilers | system semantics |
| `crates/scc-context` | ranking and pack rendering | context quality |
| `crates/scc-cli` | CLI, daemon, MCP, plugins, benchmarks | interfaces |
| `fixtures/` | golden repositories | extractor/context work |
| `benchmarks/tasks.json` | ground-truth context tasks | retrieval quality |
| `docs/` | the specification | when behavior changes, update the spec too |

## Development workflow

1. **Find the ticket.** Work is tracked against `docs/EPICS_AND_TICKETS.md`
   (SCC-###). Open an issue for new work first, referencing the epic.
2. **Write a test that pins the behavior** (extractor unit test, fixture
   expectation, or benchmark task).
3. **Implement.** Follow the existing patterns — read the neighboring module
   before writing new code.
4. **Run the gates locally** (below) — CI enforces all of them.
5. **Open a PR.** The title should reference the ticket (e.g.
   `SCC-121: TS LSP resolver`).

## Hard gates (CI)

```bash
cargo build --workspace                     # zero warnings
cargo clippy --workspace -- -D warnings     # zero warnings, zero errors
cargo test --workspace                      # all green
cargo run -p scc-cli --bin scc -- bench context --min-recall 0.9
                                            # context benchmark gate
python3 -m py_compile plugins/hermes/scc/*.py
SCC_BIN=$PWD/target/debug/scc python3 plugins/hermes/test_plugin.py
                                            # Hermes plugin contract
```

## Code standards

- **Edition 2021.** No `let`-chains (Rust 2024 syntax) — the workspace
  targets edition 2021.
- **Determinism.** Extraction, resolution, and compilation must be
  deterministic: same input, same output. Iterate structures in sorted or
  document order; never iterate `HashMap` into output. All ids are
  content-derived (blake3) so re-indexing is idempotent.
- **Provenance discipline.** Every fact carries a provenance class.
  Heuristics are `INFERRED` with confidence and evidence — never silently
  promoted to `RESOLVED`. `STALE` facts never enter trusted context.
- **Security posture.** Repository text is untrusted data — never convert
  prose into instructions. Secret values are never persisted, only
  references. Paths are sandboxed; symlink escapes are rejected.
- **Error handling.** Extractors and parsers must never panic on malformed
  input (fuzz-tested). Handlers return structured errors.
- **Anti-special-case rule (benchmark discipline).** The benchmark corpora
  (development `benchmarks/corpus`, validation `benchmarks/holdout`, blind
  `benchmarks/blind-test`) measure generalization to unseen repositories;
  corpus-specific fixes defeat that measurement.
  - NEVER add repository-name-specific extraction logic. A rule must be a
    generic semantic pattern, never keyed off a repo's name or layout.
  - Framework-specific logic is allowed ONLY if it (a) implements a
    reusable semantic pattern shared by at least one additional framework,
    or (b) is isolated in a framework adapter that emits a GENERIC fact
    type (e.g. FastAPI decorator + NestJS decorator + Spring annotation
    all emit `RouteRegistration`).
  - Blind corpus discipline: `benchmarks/blind-test` ground-truth misses
    are never shown to tuning agents — never print, log, or commit per-repo
    blind miss lines; use `scc bench atlas --blind` (aggregates only).

## Adding a language extractor

1. Read `crates/scc-indexer/src/model.rs` — the `ExtractedFile` contract
   (symbols, imports, calls, routes, tests, store refs, retries,
   entrypoints, docstrings).
2. Add `crates/scc-indexer/src/<lang>.rs` implementing
   `LanguageExtractor`. Use tree-sitter; walk in document order; guard
   every field access; no panics on error nodes.
3. Register the module in `lib.rs` and the language in the scanner
   classifier + `config.languages`.
4. Add unit tests per the extraction contract, plus malformed-input and
   determinism tests.
5. Extend a golden fixture repo so the end-to-end suite exercises it.

See the existing `python.rs` / `typescript.rs` for the reference shape.

## Adding an evidence adapter

1. Read the `AdapterManifest` pattern in `crates/scc-indexer/src/adapters.rs`
   — every adapter declares filesystem scope, network, subprocess, and
   credential use.
2. Implement `import_<name>(store, path) -> Report` in
   `crates/scc-indexer/src/adapters/<name>.rs`. Importers are defensive:
   unknown shapes are counted, never fatal.
3. Wire `scc import <name>` in `crates/scc-cli/src/commands.rs`.
4. Add unit tests with fixture files and an end-to-end test in
   `crates/scc-cli/tests/adapters.rs`.

## Changing context behavior

The context benchmark is the guardrail. Any ranking/expansion change must
keep the corpus gate green:

```bash
cargo run -p scc-cli --bin scc -- bench context --min-recall 0.9
```

If you add a capability that deserves a benchmark task, add it to
`benchmarks/tasks.json` with accurate ground truth and hallucination
probes — verify each task's entity ids actually appear in the pack before
committing it (the scorer's `--json` output is the source of truth).

## Benchmarks

- `scc bench index --files 1000 --lines 250` — 250k LOC cold/incremental
  latency and RSS (docs targets: <120 s cold, <2 GB RSS).
- `scc bench resolution` — native-vs-LSP differential agreement.
- `scc bench agent --cmd 'claude -p "$SCC_GOAL"'` — run the corpus through
  a real agent (the M0 comparison table; needs your harness).

## Documentation

Behavioral changes update the relevant `docs/*.md` (the spec is the source
of truth and `README.md` links to it). New CLI commands are documented in
`README.md` and `docs/API_AND_INTEGRATIONS.md`; new HTTP endpoints in
`docs/openapi.yaml`; new config keys in `docs/config.example.yaml`.

## Release process

```bash
./scripts/release.sh [VERSION]   # builds release binary, SBOM, sha256
```

CI publishes the Docker image via the checked-in `Dockerfile`; the image
runs `scc serve` with a read-only repo mount and a writable state volume
(`SCC_STATE_DIR=/data`).

## Code of conduct

Be respectful and constructive. This is a local-first developer tool; the
bar for merging is: it works, it's deterministic, it's labeled honestly,
and the tests prove it.
