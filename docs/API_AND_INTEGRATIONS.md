# API and Integrations

## 1. Default agent API

Expose only:
1. `system_overview`
2. `task_context`
3. `component_context`
4. `flow_context`
5. `impact_context`
6. `verify_context`

Advanced graph/evidence tools are opt-in.

## 2. MCP semantics

### `system_overview`
Purpose, components, boundaries, stores, externals, flows, invariants, freshness.

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
scc context task "..."
scc context component <id>
scc context flow <id>
scc impact --diff origin/main...HEAD
scc verify
scc drift
scc export system-ir.json
scc setup claude
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
