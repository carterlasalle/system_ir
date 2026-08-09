# Test Suite and Quality Assurance

## 1. Philosophy

Test **context correctness**, not just software correctness.

## 2. Unit tests

Per extractor: definitions, imports, calls, inheritance, routes, DB access, config refs, events, retry/fallback.

## 3. Golden repositories

Fixtures for:
- HTTP -> service -> DB
- async queue
- retry/fallback
- lifecycle
- DI
- microservice
- feature flag
- event data flow
- frontend/API contract
- monorepo deps

Commit expected System IR.

## 4. Semantic differential tests

Compare against compiler/LSP, SCIP, GitNexus, Narsil, and manual source.

## 5. Graph invariants

Release blocking:
- no dangling nodes;
- RESOLVED facts have evidence;
- no trusted STALE fact in context;
- valid flow entities;
- conflicting owners flagged;
- unenforced critical invariants flagged.

## 6. Property/fuzz tests

Rename/move/cycles/duplicate names/partial edits; fuzz parsers, YAML, JSON, Compose, Terraform, IR import, MCP inputs.

## 7. Incremental equivalence

```text
full_index(final_repo) == incremental_index(edit_sequence)
```

## 8. Flow benchmark

Measure step precision/recall, ordering, branch recall, sink, ownership.

## 9. Agent benchmark

Compare:
1. plain agent;
2. + Serena;
3. + graph tool;
4. + SCC.

Measure tool calls, files, tokens, localization, missed deps, incorrect assumptions, task success.

## 10. Context metrics

Targets:
- precision >= 0.85
- recall >= 0.95

Weight contracts/invariants/ownership/downstream consumers heavily.

## 11. Fact precision

- EXTRACTED >= 99.5%
- RESOLVED >= 98%

## 12. Hallucination

Nonexistent relationships must return unknown/no evidence.

## 13. Staleness

Mutate route/call/topic/schema/config/deployment; ensure affected packs invalidate.

## 14. Security

Prompt injection, secrets, symlink/path traversal, hostile adapter, malformed MCP, oversized payloads.

## 15. Agent integration

Claude Code, Codex, Hermes, OpenCode: startup injection, task injection, compaction recovery, subagents, budgets.

## 16. Performance

- 50k LOC cold < 30s baseline
- 250k LOC < 2m
- incremental P95 < 3s
- warm task pack P95 < 500ms
