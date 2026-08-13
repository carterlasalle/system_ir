# System Design
<!-- trace:v1 id=SD-SCC-001 type=decision work=WORK-SCC-001 title="System design: architecture, modules, authority model" -->

## 1. High-level architecture

```text
          Repository / Workspace
                  │
        ┌─────────┼──────────┐
        │         │          │
      Code      Config      Runtime
        │         │          │
        ▼         ▼          ▼
  Extractors   Infra       Trace
              parsers      ingest
        └─────────┼──────────┘
                  ▼
           Reality Compiler
                  ▼
            Reality Graph
                  ▼
        System IR Compiler
          ┌───────┼────────┐
          ▼       ▼        ▼
      Components Flows  Invariants
          └───────┼────────┘
                  ▼
             System IR
                  ▼
            System Atlas
                  ▼
          Context Compiler
                  ▼
         MCP / HTTP / CLI
                  ▼
              Agents
```

## 2. Major modules

### `scc-indexer`
Repository scanning, language detection, file hashing, extractor scheduling, incremental invalidation.

### `scc-evidence`
Normalizes facts from native extractors/external analyzers.

### `scc-graph`
Reality Graph storage/traversal.

### `scc-system-ir`
Components, ownership, contracts, boundaries, invariants, Atlas views.

### `scc-flow`
Architecture/workflow/sequence/dataflow/lifecycle compilation.

### `scc-context`
Ranking, compression, deduplication, token budgeting.

### `scc-runtime`
Runtime trace ingest and static/observed reconciliation.

### `scc-api`, `scc-mcp`, `scc-cli`
External interfaces.

## 3. Suggested codebase

```text
crates/
  scc-core/
  scc-schema/
  scc-store/
  scc-indexer/
  scc-evidence/
  scc-graph/
  scc-system-ir/
  scc-flow/
  scc-context/
  scc-runtime/
  scc-api/
  scc-mcp/
  scc-cli/

extractors/
  typescript/
  python/
  go/
  rust/
  docker/
  kubernetes/
  terraform/
  github-actions/

adapters/
  lsp/
  scip/
  serena/
  gitnexus/
  narsil/
  cbm/
  opentelemetry/
  beads/
  hindsight/
  context7/

plugins/
  claude-code/
  codex/
  hermes/
  opencode/
```

## 4. Core execution flow

### Cold index
1. Resolve repo root and Git revision.
2. Hash/index files.
3. Classify language/config/infra/test files.
4. Run syntax extractors.
5. Run semantic resolution where available.
6. Normalize evidence into Reality Graph.
7. Infer candidate components.
8. Compile Atlas views.
9. Generate system capsule.
10. Persist snapshot.

### Incremental update
1. Watch filesystem/Git.
2. Detect changed hashes.
3. Invalidate direct facts.
4. Invalidate derived edges/components/flows.
5. Re-run only relevant extractors.
6. Recompile affected System IR.
7. Invalidate cached context packs.

### Task request
1. Accept goal + optional files/symbols/diff.
2. Locate candidate entities.
3. Expand through flow, ownership, contract, invariant relations.
4. Rank facts.
5. Deduplicate and abstract.
6. Enforce token budget.
7. Return Context Pack with evidence status.

## 5. Analyzer architecture

Adapter contract:

```text
discover(repo) -> capabilities
index(repo, revision) -> EvidenceBatch
refresh(changes) -> EvidenceBatch
health() -> AdapterHealth
```

GitNexus/Narsil/CBM/native parsers all normalize into the same evidence layer.

## 6. Authority model

When sources conflict:

1. current runtime observation + current compiler/LSP facts;
2. deterministic extraction from current revision;
3. declared architecture;
4. high-confidence inference;
5. historical memory;
6. model assumption.

Conflicts are surfaced, not silently merged.

## 7. Component compilation signals

- package/workspace boundaries;
- directories/namespaces;
- deployment units;
- route ownership;
- data ownership;
- call/import cohesion;
- dependency direction;
- event ownership;
- Git co-change;
- explicit intent.

Community detection may propose a component but cannot alone establish one.

## 8. Responsibility compilation

Responsibilities may derive from public APIs, routes/tools, owned entities, events, tests, docstrings, ADRs, and config. LLM assistance is allowed only as typed `INFERRED` claims with evidence IDs.

## 9. Concurrency priorities

- P0 active-task changed file;
- P1 active repo incremental refresh;
- P2 background enrichment;
- P3 full semantic/security refresh.

## 10. Failure behavior

If SCC cannot prove a claim, return `unknown` or separately labeled inference. If the model is stale, refresh targeted evidence or fail closed for critical impact operations.
