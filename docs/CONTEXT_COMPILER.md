# Context Compiler Specification

## 1. Objective

Convert a large System IR into the smallest high-recall context pack that gives an agent correct system understanding for a task.

## 2. Context levels

### L0 Identity — 500–1,500 tokens
Repo identity, revision, languages, entrypoints, freshness.

### L1 Startup capsule — 3,000–8,000
System purpose, major components/boundaries, critical stores/externals, primary flows, ownership, critical invariants.

### L2 Task pack — 4,000–12,000
Targets, flows, upstream/downstream, data/ownership, contracts, failures/retries, invariants, implementation, tests, recent changes.

### L3 Deep evidence — on demand
Exact symbols, source excerpts, call/CFG/DFG/PDG slices, traces, history.

## 3. Inputs

- user goal;
- explicit files/symbols;
- active diff;
- active Bead/task;
- checkpoint;
- recent actions;
- selected component/flow.

## 4. Candidate generation

Use:
- BM25/FTS;
- embeddings;
- file/symbol matches;
- graph neighbors;
- flow membership;
- ownership;
- contract relationships;
- invariant scope;
- impact;
- tests;
- runtime paths.

## 5. Ranking

```text
semantic + lexical + graph + flow + ownership + contract +
change + invariant + runtime + provenance + freshness
- redundancy - stale_penalty
```

Weights must be benchmark tuned.

## 6. Expansion

1. task target;
2. containing component;
3. participating flows;
4. contracts/data;
5. direct upstream/downstream;
6. invariants/tests;
7. second-order dependencies only when justified.

## 7. Compression

Before LLM summarization:
1. deduplicate;
2. collapse symbols into components;
3. collapse repeated edges;
4. preserve critical branches/failures;
5. preserve ownership/contracts/invariants;
6. drop low relevance;
7. compress evidence refs;
8. optional constrained prose summary.

## 8. Pack format

```text
TASK
SYSTEM ROLE
RELEVANT COMPONENTS
PRIMARY FLOW
SECONDARY FLOWS
UPSTREAM
DOWNSTREAM
DATA OWNERSHIP
CONTRACTS
INVARIANTS
FAILURE / RETRY / FALLBACK
IMPLEMENTATION
TESTS
RECENT CHANGES
EVIDENCE STATUS
```

## 9. Token budgeting

Never cut:
- critical invariants;
- ownership;
- directly affected contracts;
- known failure behavior;
- stale/conflict warnings.

## 10. Benchmark

Measure:
- precision/recall;
- files opened;
- tool calls;
- tokens;
- localization latency;
- missed dependencies;
- task success.

Optimization counts only if end-to-end agent outcomes improve.
