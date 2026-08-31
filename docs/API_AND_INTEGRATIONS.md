# API and Integrations
<!-- trace:v1 id=REQ-SCC-API type=requirement derived_from=PRD-SCC-001 title="Agent API: MCP, HTTP, CLI, SDKs" -->

## 1. Default agent API

Expose only:
1. `system_atlas` — the full startup architecture
2. `system_overview`
3. `task_context`
4. `component_context`
5. `flow_context`
6. `impact_context`
7. `verify_context`
8. `system_context` — fused session-startup artifact (Atlas + Surface Map + coverage + omissions; **adaptive** budget, not a fixed 13:7 split)
9. `surface_map` — the callable API layer, global or task-personalized
10. `structural_source` — declaration headers plus per-symbol evidence

Advanced graph/evidence tools are opt-in.

## 2. MCP semantics

### `system_atlas`
The complete System Atlas: system purpose, every component (purpose,
implementation paths, consumes/produces, upstream/downstream, ownership with
provenance), entrypoints, primary flows, data ownership, contracts, critical
invariants, failure/retry behavior, deployment units, trust and async
boundaries, external systems, implementation map, evidence summary, and
freshness warnings.

Input:
```json
{"token_budget": 15000}
```

### `system_overview`
Compact capsule: purpose, components, boundaries, stores, externals, flows, invariants, freshness.

### `task_context`
Input:
```json
{"goal":"modify transcript normalization","files":[],"symbols":[],"token_budget":8000}
```

### `component_context`
Responsibility, implementation, deps, ownership, flows, contracts, tests.

### `flow_context`
Trigger, steps, branches, data, failures, retries, evidence.

### `impact_context`
Input files/symbols/diff/contract; returns affected components, flows, consumers, contracts, data, invariants, tests.

### `verify_context`
Freshness, stale facts, conflicts, low-confidence deps, drift, missing evidence.

<!-- trace:exempt reason=document-structure -->
### `system_context`
Session-startup artifact: the System Atlas fused with the System Surface Map
(the actual callable API layer), model coverage, and honest omissions in one
deterministic pack. The primary agent startup tool. The optional `token_budget`
is the **total**; the atlas:surface split is chosen by the production adaptive
allocator from repository complexity (not a fixed 13:7 formula). Default
total is the configured startup budget.

Input:
```json
{"token_budget": 20000}
```

<!-- trace:exempt reason=document-structure -->
### `surface_map`
The System Surface Map: the repository's actual callable API layer, ranked by
global importance — or, with a `goal`, personalized to that task (task PPR
re-ranking).

Input:
```json
{"goal":"modify transcript normalization","token_budget":7000}
```

<!-- trace:exempt reason=document-structure -->
### `structural_source`
Structural Source representation of files: exact declaration headers plus
per-symbol call/write evidence (deep) or signatures and imports (fallback).
Pass `files` or a `goal` (a goal selects the task-matched files via the
PPR→Surface pipeline).

Input:
```json
{"goal":"modify transcript normalization","token_budget":6000}
```

Protocol: stdio JSON-RPC 2.0. The server negotiates MCP `2025-06-18` and
`2025-11-25` (the client-requested revision is echoed when supported).

## 3. Advanced API

- `query_graph`
- `search_system`
- `search_symbols`
- `get_fact`
- `get_evidence`
- `export_ir`

## 4. CLI

```bash
scc init
scc index
scc watch
scc status
scc overview
scc context startup
scc context task "..."
scc context component <id>
scc context flow <id>
scc surface [--task "..."]
scc context structural --task "..."
scc impact --diff origin/main...HEAD
scc verify
scc drift
scc export system-ir.json
scc setup claude
scc setup omp
scc setup codex
scc mcp
```

## 5. Claude Code

### SessionStart
Verify freshness, inject startup capsule, restore checkpoint, optional active Bead.

### UserPromptSubmit
For repository-changing prompts, generate/inject task pack before planning.

### Post-edit
Refresh changed-file evidence and affected IR.

### PreCompact
Persist goal, task, affected system entities, files, tests, decisions, next action.

Normal usage requires no slash command.

<!-- trace:exempt reason=document-structure -->
## 5a. Oh My Pi (OMP)

`scc setup omp` installs a native project-scoped integration:

- `.omp/extensions/scc/` — lifecycle extension (`session_start`,
  `before_agent_start`, post-edit `scc index --paths`, opaque-mutation
  dirty-file refresh, `session_before_compact` + `session.compacting`
  rehydration via `scc context startup` and `scc checkpoint load --inject`)
- `.omp/mcp.json` — SCC MCP server (`command` is `SCC_BIN` when set at
  setup, else `scc`)
- `.omp/skills/scc-system-context/SKILL.md` — on-demand workflow for
  `system_context`, `surface_map`, `structural_source`
- AGENTS.md — durable SCC rules. Existing `.omp/AGENTS.md` is patched;
  otherwise an existing root `AGENTS.md` is patched (no shadowing
  `.omp/AGENTS.md` is created). Marker-aware rewrite keeps user text on
  both sides of `<!-- SCC-SECTION -->`.

After install: **restart OMP**, then run `/extensions` to **verify** the
extension loaded (`/extensions` is an inspector, not a reload). The
extension honors `process.env.SCC_BIN`.

## 6. Serena

SCC answers **what matters**; Serena answers **where exactly it is**.

## 7. GitNexus / Narsil / CBM

Treat as evidence backends. Do not expose all overlapping tool surfaces to the same agent by default.

## 8. Context7

External dependency/API truth only; source labels remain separate.

## 9. Beads

Operational task state. Import active goal/dependencies and optionally attach affected components/flows.

## 10. Hindsight

Durable lessons only; never repository source-of-truth.

## 11. RTK

Optional output middleware; SCC may advise when raw diagnostics should be preserved.

## 12. Ponytail / Superpowers

SCC = system knowledge.  
Ponytail = implementation restraint.  
Superpowers = engineering process.

## 13. Agent Deck / MCP Gateway

SCC belongs in always-on `repo-core`; cloud/browser/security tools remain profile-scoped.
