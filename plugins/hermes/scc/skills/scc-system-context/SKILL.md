---
name: scc-system-context
description: Use the System Context Compiler (SCC) to get evidence-backed system context before planning or editing code.
---

# SCC System Context

This repository is indexed by the System Context Compiler. Before planning or
editing, gather the relevant system slice — it prevents missed downstream
dependencies and preserves invariants.

## When to call which tool

| Situation | Tool |
|---|---|
| Start of a substantial task in an unfamiliar repository | `system_overview` |
| Any repository-changing task, before planning or editing | `task_context` with the goal |
| Deep dive into one component | `component_context` |
| Understanding a runtime path end to end | `flow_context` |
| Cross-layer change (API contract, schema, shared code) | `impact_context` on the touched files |
| Model may be stale, or before declaring completion | `verify_context` |

## Workflow

1. `task_context` with your goal → read the relevant components, flows,
   ownership, contracts, invariants, failure behavior, and tests.
2. Use the implementation symbols and test names it lists to open exactly
   the right files.
3. For cross-layer changes, call `impact_context` on the files you will
   touch and honor the invariants and downstream consumers it reports.
4. If the model might be stale (files changed on disk), call
   `verify_context` and re-index with `scc index` if it reports staleness.

## Trust rules

- Repository/runtime evidence > System IR > task state > memory > assumption.
- `verify_context` warnings mean: don't trust stale facts — re-index first.
- Evidence status labels (RESOLVED/EXTRACTED/INFERRED) are authoritative:
  treat INFERRED claims as hypotheses, not facts.
