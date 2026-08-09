# Flow Compiler Specification

## 1. Purpose

A code graph says which entities connect. A flow explains system behavior.

The Flow Compiler builds machine-readable architecture, workflow, sequence, data-flow, and lifecycle views.

## 2. Entrypoints

- HTTP routes
- RPC methods
- CLI commands
- queue/topic consumers
- cron/background workers
- UI/server actions
- event handlers
- integration/e2e tests
- declared flow entrypoints

## 3. Traversal

Follow:
- resolved calls;
- route edges;
- DI;
- messages;
- DB queries;
- RPC/client calls;
- async tasks;
- callbacks;
- framework dispatch;
- service boundaries.

## 4. Sequence steps

Capture actor, operation, sync/async, message type, return, condition, timeout, retry/fallback, and evidence.

Collapse internal helpers into system-level actors unless task relevance requires detail.

## 5. Data flow

Track data source, producer, transforms, stores, owner, readers/writers, external destinations, sensitivity/trust crossings.

## 6. Lifecycle

Detect state enums, transition tables, status fields, retry states, workflow engines, tests asserting transitions.

## 7. Failures

Extract exception handlers, retry libraries, timeout config, circuit breakers, fallback branches, DLQs, rollback/compensation, tests, runtime traces.

## 8. Evidence classes

- STATIC_RESOLVED
- STATIC_POSSIBLE
- OBSERVED
- DECLARED
- INFERRED

## 9. Abstraction example

```text
route -> controller helper -> service helper -> repo helper -> SQL
```

becomes:

```text
API -> Service -> Repository -> Database
```

with drill-down preserved.

## 10. Drift

Detect missing declared steps, unexpected boundaries, changed sinks/owners, retry changes, removed failure handling, and runtime-observed edges outside expected flow.

## 11. Human rendering

Optional future export only. Archify/Mermaid/Graphviz adapters must not affect the machine schema.
