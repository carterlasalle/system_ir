# Data Strategy
<!-- trace:v1 id=REQ-SCC-DATA type=requirement derived_from=PRD-SCC-001 title="Data strategy: storage, freshness, epochs" -->

## 1. Principles

Local-first, evidence-first, incremental, provenance-aware, progressively retrieved.

## 2. Layers

### L0 Snapshot
Repo ID, branch, commit, file paths/hashes, language, timestamp.

### L1 Evidence
Source locations, symbols, extractor metadata, trace/config sources.

### L2 Reality Graph
Typed entities/relations with provenance/confidence/freshness.

### L3 System IR
Components, responsibilities, ownership, contracts, Atlas, invariants.

### L4 Context indexes
Lexical, semantic, adjacency, component/flow/contract/test membership.

### L5 Runtime/history
Aggregated traces, Git co-change, prior snapshots, drift history.

## 3. MVP storage

SQLite + WAL + FTS5. In-memory adjacency for hot graph traversal. No required Neo4j/server DB in MVP.

## 4. Suggested tables

```text
repositories snapshots files symbols entities relationships evidence
components flows flow_steps invariants tests ownership
context_cache runtime_edges intent_claims drift_findings
```

## 5. Retrieval

Fuse lexical, optional semantic, graph, flow, ownership, contract, and test signals. Vector similarity is never truth.

## 6. Freshness

Every source-backed fact stores source hash, extractor version, snapshot, and dependency keys.

Change:
1. invalidate direct evidence;
2. invalidate derived relations;
3. invalidate affected component/flow;
4. invalidate pack cache;
5. regenerate affected subgraph.

## 7. History

Retain latest + configurable committed snapshots for drift and regression debugging.

## 8. Runtime

Store aggregates by default; configurable sampling/redaction/retention.

## 9. Secrets

Persist references, not resolved secret values.

## 10. LLM inference

Separate namespace/table; provenance INFERRED; evidence/model/version required; no silent promotion.

## 11. Exports

JSON/JSONL native. Optional JSON-LD, N-Quads, CCG-compatible, GraphML, Cypher.
