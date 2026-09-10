# Phase 7 of SCC x Ripwire lessons: type narrowing, explore protocol, hash sweep

<!-- trace:v1 id=doc.phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing-from-unique type=document work=WORK-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing-from-unique -->

<!-- trace:exempt reason=document-structure -->
## Goal

Absorb three verified Ripwire ideas without collapsing SCC's ontology: one-hop type narrowing from unique locals/params; a real JSONL exploration protocol vs baseline and Ripwire; watcher-fail → content-hash sweep. Production ranking, four context levels, and the 10 MCP tools stay unchanged.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-implement-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing — Type narrowing, JSONL explore protocol, hash-sweep fallback

<!-- trace:v1 id=REQ-implement-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing type=requirement work=WORK-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing-from-unique -->

Type narrowing: extractors record per-scope local type binds from constructors (`x = Order()`, `const x = new Foo()`) and parameter/annotation types (`x: Order`). A name bound to two distinct types in one scope is not unique and must not narrow. Native resolution of `x.process()` (RecvKind NamedVariable) must pin to `{Type}.process` when the bind is unique and that method is a real definition on the local class or the imported type. Same-name methods on other classes must not receive the edge. Zero or many remaining candidates stay unresolved. Native provenance stays EXTRACTED. Binds are extract-time only (not FILE attributes). When a path is refreshed or removed, files that IMPORT or CALL into it are re-extracted even if their content hash did not change, so type-narrowed CALLS stay equivalent to a cold index.

Explore protocol: `scc bench loop --explore` runs a three-way pack-consumer that emits JSONL tool events scored like `scc bench agent` (search/read calls, files opened, first-correct ordinal, wrong-first, substitution). Baseline greps then reads. SCC calls `task_context` then reads pack files (substitution 1.0 when it does not grep). Ripwire is a black-box `--pack-task` then reads `p=` files; missing binary is `skipped`, never an SCC win. Default `scc bench loop` stays the locator. `SCC_EXPLORE_AGENT_CMD` may replace the deterministic consumer. This is not an LLM/SWE-bench repair claim.

Watcher: if notify cannot start or cannot watch the root, fall back to content-hash sweep via `stale_paths` then incremental refresh. Watcher never becomes correctness authority.

Task Context and context-benchmark scoring treat merged cluster ids (`root-services`) as covering gold member-region names (`root`, `services`). Packs include every component that CONTAINS a selected file, not last-write-wins. Clustering still does not emit member-region shell entities after a merge.

Do not change production ranking, do not add a fifth context level, do not add MCP tools, do not rewrite REQ-resolution-honesty-gauges.
