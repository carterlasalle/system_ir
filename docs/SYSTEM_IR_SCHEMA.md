# System IR Schema Specification

## 1. Purpose

System IR is the durable machine model consumed by the Context Compiler. It contains semantics, not presentation.

## 2. Stable identifiers

```text
repo://{repo-id}/{kind}/{stable-key}
```

Examples:
```text
repo://phoenix/service/asr
repo://phoenix/component/transcript-normalizer
repo://phoenix/data/transcript
repo://phoenix/flow/live-radio
```

## 3. Core entities

Repository, Workspace, Package, Module, File, Symbol, System, Subsystem, Service, Component, DeploymentUnit, Route, Endpoint, RPCMethod, Event, Topic, Queue, DataEntity, DataStore, Table, Collection, Cache, ExternalSystem, ExternalAPI, Configuration, FeatureFlag, SecretReference, Contract, Invariant, Test, TestSuite, Flow, FlowStep, Branch, State, Transition, TrustBoundary, SecurityControl, RuntimeObservation, Evidence.

## 4. Relationship ontology

```text
contains implements inherits imports calls reads writes queries owns
publishes consumes subscribes produces transforms validates routes_to
handles invokes depends_on deployed_with deployed_in configured_by
protected_by crosses_boundary enforces tested_by participates_in
precedes follows branches_to retries falls_back_to observed_as
declared_as implemented_by
```

## 5. Provenance

- `EXTRACTED`: direct syntax/config evidence
- `RESOLVED`: compiler/LSP/type/binding-resolved
- `OBSERVED`: runtime evidence
- `DECLARED`: architectural intent
- `INFERRED`: heuristic/LLM claim
- `STALE`: evidence invalid for active revision

## 6. Fact envelope

```json
{
  "id": "fact:123",
  "subject": "repo://r/component/a",
  "predicate": "writes",
  "object": "repo://r/data/x",
  "provenance": "RESOLVED",
  "confidence": 0.99,
  "evidence": ["evidence:17"],
  "extractor": {"name": "python-lsp-resolver", "version": "0.1.0"},
  "revision": "git-sha",
  "verified_at": "2026-08-07T00:00:00Z"
}
```

## 7. System Atlas types

### Architecture
Components, services, stores, queues, externals, deployment/trust boundaries.

### Workflow
Steps, actors, responsibility, branches, approvals, failures, rollback.

### Sequence
Caller, callee, message, return, async boundary, timeout, retry.

### Data Flow
Source, data, transform, owner, store, consumer, sensitivity/retention.

### Lifecycle
State, event, transition, guard, retry, recovery, terminal outcome.

No coordinates/colors/layout.

## 8. Invariant

```json
{
  "id": "repo://r/invariant/raw-transcript-immutable",
  "statement": "Raw ASR output must never be overwritten.",
  "severity": "critical",
  "scope": ["repo://r/data/transcript"],
  "enforced_by": ["repo://r/test/test_normalization_preserves_raw"]
}
```

## 9. Confidence rules

- EXTRACTED default 1.0
- RESOLVED normally >= 0.95
- OBSERVED describes an observation, not exhaustive possibility
- DECLARED means intent, not proven reality
- INFERRED must be < 1.0 and evidence-backed
- STALE cannot enter trusted packs except as warning

## 10. Validation invariants

- No dangling relation endpoints.
- Every trusted fact has evidence.
- Every RESOLVED fact identifies resolver/revision.
- Every flow step references an entity or external actor.
- Critical invariants lacking enforcement are flagged.
- Conflicting authoritative ownership is flagged.
- STALE facts are excluded from trusted context.
