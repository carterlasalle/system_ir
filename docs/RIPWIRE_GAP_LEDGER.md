# SCC × Ripwire gap ledger (code-level)

<!-- trace:v1 id=doc.ripwire-gap-ledger type=document work=WORK-ripwire-lessons-phase1 -->

Docs are hints. This ledger is from reading SCC (`crates/scc-*`) and Ripwire (`src/*.h`, `src/*.cpp`, `bench/`, `queries/`) implementations. Where README and code disagree, **code wins** and the discrepancy is recorded.

SCC identity that must not be collapsed: SOURCE/CONFIG/RUNTIME/DECLARED INTENT → Reality Graph → TrustedGraphView → semantic compilation → Atlas (L0) → Surface (L1) → Structural Source (L2) → Exact Source (L3). No fifth level. No replacement of SCC's ontology with Ripwire's symbol/call graph.

Ripwire clone inspected: `/tmp/vendor/ripwire` (redhat-et/ripwire).

## Document vs implementation discrepancies

| Claim | Reality |
|---|---|
| SCC README: extractors for "TypeScript and Python" | Code also has Go, Java, Rust extractors (`scc-indexer/{go,java,rust}.rs`). README updated to match; `scc languages` is now the generated SoT. |
| Ripwire README-style `external=` as "no def in tree" | `serialize.h` Phase-5 `externalCalls` is the **external-name veto** count (`graph.h g.externalCalls`), not "missing name". Pure missing names are often silent drops (`unresolvedOut`). |
| Ripwire RecvKind docs sometimes imply chain resolution | `model.h`: chained field receivers are classified; **chains are unresolved by design**. |
| Ripwire "LSP overlay" | SCIP overlay exists in `src/`. No LSP overlay under `src/`. |
| SCC unified ranking is "the" retrieval story | Production ranker fuses task PPR + global PPR + lexical + semantic + confidence + criticality + change_risk + novelty. Ripwire **deliberately does not fuse** BM25 with PageRank. SCC must not copy fusion without ablation (Phase 2). |

## Ledger

| FEATURE / CAPABILITY | SCC CURRENT STATE | RIPWIRE IMPLEMENTATION | WHAT RIPWIRE DOES BETTER | WHAT SCC DOES BETTER | SHOULD SCC ABSORB IT? | HOW SHOULD SCC ABSORB IT? | RISKS | TEST / BENCHMARK REQUIRED | PRIORITY |
|---|---|---|---|---|---|---|---|---|---|
| Parser / extractor architecture | 5 procedural tree-sitter walkers (Py, TS/JS, Go, Java, Rust). No `.scm` query files. | `queries/<lang>/*.scm` + ingest pipeline into RawDef/RawRef/RawBind. Language table `kLangTable` in `ingest.cpp`. | Declarative queries; faster language expansion; one ingest IR. | Semantic facts (routes, stores, contracts, CFG attrs) in the same pass. | Partial, later | Keep procedural walkers for semantic facts; add `.scm` for mechanical defs/refs where they reduce duplication (Phase 4). | Query-only extractors lose SCC semantic signals. | Per-language fixtures; matrix generated from registry. | P4 / Phase 4 |
| Language adapters | Scan classifies py/ts/js/go/rs/java + config/infra/docs + shell/sql **and** C/C++/ObjC/C#/Ruby/PHP/Lua/Swift/Kotlin as IndexSearch (no extractor). | Broad tree-sitter surface including C/C++/ObjC/C#/Ruby/PHP/Lua/Swift, etc. | Breadth + `.scm` extractors. | Honest tiers; config/infra extractors Ripwire does not treat as system entities. | Yes (honest matrix now; semantic extractors later) | `LANGUAGE_REGISTRY` + `scc languages`. Do not claim support from parse-only. Add languages only with fixtures. | Fake "supported" claims. | Registry uniqueness; scan↔registry; extracted ⇒ tier A. | P10 / Phase 1 SoT, Phase 4 classification |
| Symbol model | File-qualified `symbol_id(repo, path, name)`. Methods `Class.method`. Overload index. | Dense IDs, path-qualified names, fields as a **side table** (not PageRank symbols). | Compact adjacency; fields don't pollute PageRank. | Heterogeneous entities (component, route, contract, state, schema). | Investigate | Optional field table if ranking ablations show field-name collision harm. Do not reduce components to symbols. | Polluting PPR with fields. | Ranking ablation with/without fields. | P1 later / Phase 2 |
| Reference model | `ReferenceKind` includes Call/Read/Write/Import/… plus Macro/FieldAccess/Construct. Writer still emits CALLS edges only for resolved calls. | `RefRole`: Call/Read/Write/Import/Extends/Macro/Type. Only Call+Macro enter CSR. | Roles are first-class at graph build. | Broader semantic predicates (HANDLES, PUBLISHES, READS, WRITES, …). | Yes | Capture role at extract; skip non-execution roles in CALL ranking. Keep semantic predicates. | Treating imports as calls. | Resolver tests: macros not CALLS. | P1 / Phase 1 |
| Receiver-aware resolution | **Now:** RecvKind + conservative resolve + **one-hop type narrowing** (Python, TypeScript, **Go, Rust**, Java locals/params) + **one-hop field-type narrowing** (Python, TypeScript, Java, Go, **Rust**) + **Java unprefixed field-as-receiver** + **Rule 2c class-name receiver** + **Rule 2c CHA/base walk** + **Rule 1 self/this/super base walk** + **Rust `impl Trait for T` CHA**. Unique `x = Order()` / `x: Order` / `new Foo()` / typed params **and Go `x := &Order{}` / Rust `let x = Order {}`** pin `x.m()` to `{Type}.m`. Unique class/struct field types pin `self.x.m()` / `this.x.m()` / Go `s.x.m()` when the root is the enclosing type. Java also pins bare `field.m()` (Ripwire Rule 2b analogue; `.java`-gated). **Go `v.(*Order)` / `Order(v)`, Rust `v as Order`, TS `v as Order` / `<Order>v`, and Java `(Order)v` also pin.** **`Cls.m()` (StaticType) pins the unique in-repo Class/Interface/Type that uniquely defines `m`; a typed local/param named `Cls` wins; an untyped param vetoes; two defining same-named classes stay unresolved. If `Cls` does not define `m`, a BFS of simple-ident bases (cap 16) pins the unique hitting base at the shallowest level (`IERS_B.open()` → `IERS.open`); two hitting bases at one level stay unresolved; two same-named class-likes refuse heritage merge. Heritage is persisted on CLASS entity attributes, never FILE. Modules are not class-like.** **`self.m()` / `this.m()` that is not a sibling of the enclosing class pins via unique class-like plus CHA (`IERS_B.run` `self.open` → `IERS.open`); own-class sibling still wins. `super.m()` and Python `super().m()` never pin the enclosing class (bases only). Two hitting bases at one level stay unresolved (no Ripwire `unionOnMulti` / first-declared MRO). Field chains are not Rule 1. No C++ bare-name Rule 1.** **Rust `impl Open for IERS_B` records Open as a base of IERS_B; trait methods are `Trait.m`; typed `x.open()` pins the unique hitting trait method when T does not define it; inherent impl wins; two traits defining `m` stay unresolved.** ≥2 types tombstone. **Go `s.owned = &Invoice{}` and Rust `self.owned = Invoice {}` are extract-time tombstone fuel** against a declared Order field. Named var still never sprays. Longer field chains unresolved. Import chains (`db.users.Find`) stay unresolved. | `resolve.h` Rule 1 this/self, Rule 2 var type, Rule 2b field type (C++), Rule 2c class-name, Rule 3 include-file. | Include-file (3) still deeper for C++ unprefixed fields; C++ bare-name Rule 1 and `unionOnMulti` super MRO still deeper. | Conservative incomplete edges + provenance; Python/TS/Java/Go/Rust field hop and local type; Java unprefixed field; Go/Rust assignment tombstone; assertion/cast fuel including Java `(Order)v`; class-name receiver with local-shadow veto and unique-base CHA; Rule 1 self/this/super CHA without spraying multi-base ties; Rust trait-impl CHA; binds never FILE attrs. | Yes, incrementally | Phase 7–24. Include-file and C++ extractors later. | Wrong-but-confident edges. | type-narrow + field-type + unprefixed + assignment-tombstone + cast + class-name + CHA + Rule 1 + Rust trait-impl tests. | P1 / Phase 7–24 |
| Unresolved vs external | **Now:** relative import miss → `ImportTarget::Unresolved` (no EXTERNAL_API edge). Bare miss → Unknown, not External. Confirmed external import → EXTERNAL_API. | `externalCalls` is veto, not "name missing". Unresolved often dropped silently. | Operational gauges in headers. | Distinct enum (`ResolutionClass`) + provenance. | Yes | Persist gauges; never label unresolved as external. | Agents treating all miss as stdlib. | quality counts; relative import test. | P2 / Phase 1 |
| Reality Graph node types | First-class System/Service/Component/Module/External/Store/Queue/Topic/Route/Contract/Schema/Deployment/TrustBoundary/State/… | Primarily symbols, files, fields, binds. | N/A (different product). | SCC's ontology is the moat. | No replace | Keep. Use Ripwire-grade refs as *evidence under* entities. | Collapsing Atlas into call graph. | Ontology registry test. | P0 |
| Reality Graph edge types | Semantic predicates + FlowEdgeKind including CALL/BRANCH/ERROR/RETRY/… and Read/Write/Transform/Validate/Authorize/Cache/Invalidate. | CSR call graph + role-filtered refs. | Dense call graph. | Causal FlowGraph + heterogeneous predicates. | Enhance, don't replace | Feed nonlocal R/W into FlowGraph/StateAuthority later (P14). | Mixing roles into CALL ranking. | FlowEdgeKind exhaustive match. | P0 / Phase 6 for depth |
| Provenance / confidence | EXTRACTED/RESOLVED/OBSERVED/DECLARED/INFERRED/STALE. Native resolve is EXTRACTED (never silent RESOLVED). LSP/SCIP can upgrade. | Provenance on edges; SCIP overlay upgrades. Native ≠ compiler-proven. | Battle-tested overlay path. | Full taxonomy + TrustedGraphView filters STALE. | Strengthen overlays | Precision overlays upgrade existing edges (`resolver: SCIP`, confidence 1.0). Optional Ripwire import experiment (P31). | Dual universes of edges. | Native stays EXTRACTED (existing test). | P1 / P31 |
| Stale evidence | Content-hash invalidation; TrustedGraphView drops STALE; incremental ≡ cold (`golden.rs`). Analyzer gauges live in store meta so unchanged FILE entities stay byte-identical across incremental vs cold. | Warm index + watcher optimization; stat/hash authority. | Hardened warm MCP index. | Hash is already authority; watcher (`notify` in `httpd.rs`) is debounce only. | Harden | Keep hash authority. Watcher never correctness. Equivalence tests stay. Do not stamp derived gauges onto exported FILE attributes. | Watcher-as-truth; gauges on FILE entities break incremental≡cold. | Existing incremental≡cold. Add more failure injection later. | P12 / already strong |
| TrustedGraphView | STALE out; inferred floor; only trusted facts in packs. | N/A as named. | — | Unique. | Keep | — | — | Existing pack tests. | P0 |
| System Atlas | Architecture, components, flows, ownership, contracts, deployment, externals, trust. | Arch/quality commands exist (`arch.h`) but are code-quality oriented. | Some hotspot/complexity metrics. | Atlas is agent-facing system model. | Absorb metrics only if they improve ranking/risk | Optional complexity/churn as ranking features (P35), not a linter product. | Becoming a static-analysis suite. | Ablations. | P0 keep / P35 later |
| FlowGraph | Canonical causal graph from CFG attrs + calls + events. | Call graph, not causal workflow. | Precise calls. | "What happens" not only "who calls whom". | Feed better calls/R/W into it | Resolver honesty improves FlowGraph inputs. | — | Existing flow tests. | P0 |
| State authority | Owner/reader/writer/persistence/cache/derived (graph compiler). | `nonlocalstate.h` per-function nonlocal R/W, fields, globals. | Function-level nonlocal facts. | System-level authority model. | Yes, as inputs | Phase 6: per-function R/W/publish/consume → StateAuthority. | Replacing authority with local analysis. | Fixtures for writes/reads. | P14 / Phase 6 |
| Surface ranking | Heterogeneous PPR + fused score. One `build_surface`. | PageRank on call graph; lexical BM25 separate. | Measured: fusion sometimes **hurts** retrieval. | Heterogeneous nodes (routes, components). | Do not blindly fuse | Phase 2: ablation arms A–I. Keep concepts separately inspectable. | Tuning on eval tasks. | Retrieval benchmark + ablations. | P3 / P21 / P22 |
| Task ranking | Same fused formula. | Query → BM25/name → pack; graph is importance not relevance. | Relevance-first retrieval. | Task PPR + Atlas delta. | Experimental arm | Query-shape router + lexical-first arm. Production change only with evidence. | Regressing architecture questions. | Recall@k, MRR, gold file/symbol. | P3 / Phase 2 |
| Structural Source | Level 2 skeleton from CFG evidence. | Signatures + exact bodies under pack budget. | Exact-source dominance; bodies last in packer. | Semantic skeleton. | Yes dominance; keep Level 2 | If exact cost ≤ structural cost, serve exact and disclose. Benchmark Level 2 vs exact-only (P30). | Silent substitution. | exact-dominance unit + tempfile test. | P7 / Phase 1 |
| Exact-source retrieval | `scc://` handles on structural_source **and Task Context FETCH**. Stale hash refuses; explore pack-consumer will not guess a path. | `sym#idHash@contentHash`; `fetch` verbs; stale hash refuses. | Handles + lazy bodies. | Heterogeneous entities + Atlas in the same pack. | Yes | FETCH section in Task Context; refuse stale. No new MCP tool. | Tool explosion. | Handle roundtrip + stale refuse + explore refuse. | P8 / Phase 1 + 13 |
| Indexing / incremental | blake3 content hash; purge path; rebuild derived layer. incremental≡cold. | Warm parse-once; dense IDs; persisted lexical stats. | Speed (lexical persist, contiguous storage). | Correctness-first incremental. | Absorb speed later | Profile first (P27). Equivalence gates for every fast path (P25). | Unsafe incremental cleverness. | incremental_after_edit == fresh_rebuild. | P13 / P25 |
| Filesystem watching | `notify` debounce → refresh_paths. Hash still authority. **Watcher fail → `refresh_stale_by_hash` sweep.** | kqueue/inotify/ReadDirectoryChangesW; fallback sweep. | Platform watchers. | Already "watcher is not authority". | Keep contract | If watcher fails, stat/hash sweep. Determinism: watcher must not change result bytes. | OS-dependent output. | Hash-sweep refreshes edited file. | P12 |
| MCP tools | **Exactly 10** intent-level tools. | **31 verbs** (`mcpverbs.h`). | Fine-grained latency/token control. | Agents don't orchestrate 8 tools for one impact question. | No 31-tool clone | Enrich internals of existing tools. Lower-level only for fetch/edit/diagnostics. | Skill/tool overhead (Ripwire pilot). | `tools().len() == 10`. | P18 |
| Task packs | Adaptive sections; critical never cut; dropped_sections disclosed. | Fixed quotas (rank 40 / bodies 30 / callers 15 / notes 5 / tests 10) + rollover; bodies last. | Simple robust allocation; tests_to_run. | Atlas + contracts + provenance in the pack. | Benchmark both | Budget allocator abstraction (Phase 3). Do not assume adaptive wins. | Silent cap. | Truncation disclosure tests. | P5 / P6 / Phase 3 |
| Change-risk / git | Co-change CLI exists (`cmd_cochange`); git revision on snapshot. | gitmine, cochange, whereis, stray_content, merge_scout, dirty-tree situ. | Mature change intelligence. | Semantic impact (contracts/state/flows) possible. | Selective | Co-change as historical evidence, never override current semantics. Forgotten partners in impact_context. Skip generic git utilities. | History overriding truth. | Co-change fixtures. | P4 / P15 / Phase 2–3 |
| tests_to_run | Tests extracted; tested_by relink on change. Pack lists tests with reasons (`direct` / `import` / `contract` / `state`). | `testmap.h` + pack `tests_to_run`. | Actionable verify list with reasons. | Can reason via contracts/state/flows. | Yes | Phase 3: tests_to_run with reasons (direct, contract, state, import). Filename-only is not a reason. | Filename-only lists. | Behavioral fixtures. | P16 / Phase 3 |
| Tests / evals | 21-task corpus; atlas recall; `scc bench retrieval`; locator `scc bench loop`; **JSONL explore** `scc bench loop --explore` (deterministic pack-consumer; `SCC_EXPLORE_AGENT_CMD` LLM plug-in). **Measured explore vs live Ripwire** (`benchmarks/results/ripwire-lessons-explore-loop.json`). Locator/explore score **tests_to_run vs gold tests** (`tests_localization`; empty gold omitted; kebab gold matches `it()` titles). LLM/SWE-bench repair still missing (no agent runtime in this env). | locbench, agentloop (baseline / ripwire_cli / ripwire_skills), contamination, clustered stats. | Real LLM agent-loop. | Semantic corpus + Atlas GT + locator/explore vs live Ripwire + tests_to_run scoring. | Mandatory add | Explore protocol + measured pack-consumer vs Ripwire landed; tests_to_run scored (SCC clustered_tests 0.833 vs 0.000); LLM repair still needed before product claims. | Contamination; model substitution. | Multi-seed, clustered stats, resolved primary. | P20 / Phase 5–7 |
| BM25 / lexical | **Now:** experimental BM25 lens + persisted `bm25_corpus` meta (n/avgdl/df/dl). Production fused ranker **unchanged**. | Custom BM25: camel/snake split, field weights, persisted stats, top-K prune. | Measured retrieval; persisted stats. | Semantic + graph + Atlas. | Yes as separate lens | Implemented tokenizer+BM25+router+persist; do not fuse into PPR without ablation evidence. | Embeddings cargo-cult; silent fusion. | Acronym pins; BM25 determinism; production arm emits nothing from BM25 lens. | P3 / Phase 2 |
| Anchors / mentions | Goal terms / explicit files/symbols. No doc→symbol mention index. | Query anchors; doc mentions. | Name-first when the query names a symbol. | Heterogeneous anchors (component, route, contract, …). | Yes | Phase 2: resolve anchor first; mentions as DECLARED evidence, degradable. | Docs as truth. | Mention vs code contradiction tests. | P4 / Phase 2 / P33 |
| Symbol-addressed edits | None. | replace_symbol_body / insert_before/after; stale hash refuses; file lock. | Agent can edit by symbol. | Receipt could include contracts/state/tests. | Evaluate first | Do **not** add in Phase 1. If later: refuse stale/malformed; byte-identical on refuse; reindex; semantic receipt. | Races, silent wrong target. | Byte-identity on refuse; stale handle. | P9 hold |
| Agent reflex / skills | Startup context injection; Claude/Codex/OMP plugins. | skills/hooks/wrap; pilot showed overhead can **increase** tokens. | Aggressive default tool use. | Automatic startup Task Context. | Measure | Prefer SCC before grep; measure substitution rate. Don't assume more instructions win. | Token bloat. | SCC_calls / (SCC + native reads). | P19 / Phase 5 |
| Determinism | Content-hash ids; BTree maps; golden tests. | Obsessive stable serialize; tie-break. | Byte-stable CLI. | Hash-stable ids. | Tighten | Explicit tie-breaks; watcher/FS order must not change output. | Hidden HashMap iteration. | Repeated-run byte tests where appropriate. | P23 |
| Single source of truth | Some duplicated lists (README langs vs extractors). `predicates::ALL` had a real gap (fixed). | `kLangTable` SoT; they learned from drift. | Aggressive SoT. | Schema + kinds modules. | Yes | Registry-driven langs, roles, tools. Tests fail on drift. | Hand-maintained docs. | Mutation: drop mapping → test fails. | P24 / Phase 1 langs |
| Failure injection | Stale handle refuse (unit + **CLI `HANDLE REFUSED`**). Same-name defs do not spray. Watcher fail → hash sweep. **Truncated/garbage `scc.db` refuses (`StoreError::Corrupt`) instead of migrating into an empty index.** | Stronger adversarial coverage in places. | — | Incremental equivalence. | Expand | Remaining: more cache-corruption shapes, same-name defs at CLI. | Silent recovery that fabricates certainty. | Each failure degrades or refuses. | P26 |
| Runtime evidence | OTel ingest; static vs observed reconcile. | Essentially static. | — | Unique moat. | Strengthen | Observed upgrades EXTRACTED edges; do not overwrite history (P32). | Mixing static/runtime. | Existing runtime tests + more traces. | P32 / Phase 6 |
| Cross-language | Limited (TS↔JS share extractor). | Some C↔C++/ObjC. | Native ABI shapes. | Reality Graph can represent JNI/RPC/proto semantically. | Yes, SCC-native | Bridges as semantic edges with provenance, not forced CALLS. | Fake CALLS across langs. | Fixtures per bridge. | P1 later / Phase 6 |
| Precision overlays | SCIP import + LSP (pyright, tsserver) exist. | SCIP overlay. | Hardened SCIP path. | Overlays should **upgrade** edges, not fork a graph. | Continue | Provenance `RESOLVED` + resolver name. Optional Ripwire backend experiment. | Dual graphs. | Differential resolution bench (exists). | P31 |
| Output format | Structured packs; warnings; dropped_sections. | Compact headers with stats. | Analyzer-health in the first screen. | Semantic richness. | Compact gauges | `analysis_quality` compact line + JSON field. Caps disclosed. | Noisy prose. | Pack JSON schema-ish assertions. | P28 / Phase 1 |
| Representation selection | Caller chooses Atlas vs Surface vs Structural. | Packer chooses signatures vs bodies by budget. | Automatic cheapest sufficient form. | Four levels. | Policy later | Task-type + cost (P29). Phase 1: exact vs structural cost only. | Agent still spelunks. | P30 benchmark. | P29 / Phase 3 |
| Non-local mutable state | Store refs / READS / WRITES predicates from extractors (not SSA). | Per-function nonlocal analysis. | Function-level completeness. | System-level authority. | Integrate | Phase 6. | — | Per-function fixtures. | P14 |
| Stack/error locus | Query router extracts frames; Task Context seeds affected files + innermost enclosing symbol; unmapped frames disclosed. | FILE:LINE / locpin / `--from-trace`. | Fast localization + more frame formats. | Climbs to component/flow/state via SCC ontology. | Yes | Phase 3 ingest traces → symbols → Atlas explanation. | Stopping at the frame. | Trace fixtures. | P17 / Phase 3 |
| Quality/linter suite | Not a linter. | complexity, purity, clones, LCOM, etc. | Extra ranking features. | Stay a context compiler. | Only if it helps ranking/risk | P35 filter. | Product dilution. | Don't add without retrieval/agent evidence. | P35 |

## Phase 1 absorbed in this change (implementation, not docs)

- Receiver classification + conservative resolver (no field-chain pin, no same-name spray, unresolved ≠ external).
- `analysis_quality` persisted in store meta (`analysis_quality` + per-file `analysis_quality_files`); **not** on FILE entity attributes (those are exported in System IR and would break incremental≡cold). Task pack compact section; `scc index`/`scc status` print it.
- Field-chain `db.users.findMany` through `import { db }` seeds the imported `db` object without claiming `findMany` resolved.
- `scc://` handles; `structural_source` files arg accepts handles and **refuses** stale hashes.
- Exact-source dominance when the on-disk span is cheaper; reason disclosed.
- `LANGUAGE_REGISTRY` + `scc languages`; scan classification must appear in the registry; extractors are tier A.
- `predicates::ALL` / `kinds::ALL` mutation test; FlowEdgeKind Read/Write/… kept first-class.
- MCP remains **10 tools**.

## Explicitly not absorbed (and why)

- 31 MCP tools — tool count is not a feature; Ripwire's own pilot showed skill overhead can increase tokens.
- Production ranker fusion change — Ripwire measured fusion hurting retrieval; SCC needs ablation arms first.
- Embeddings expansion — benchmark first.
- Symbol-addressed edits — reliability/races not evaluated (P9).
- C/C++/ObjC/… extractors — classification≠support; add with fixtures only.
- Tree-sitter `.scm` rewrite of working semantic walkers — Phase 4 evaluation.
- SWE-bench / LLM agent-loop — locator vs Ripwire is measured; LLM repair success is still required before product claims.
- Copying Ripwire quality/linter commands.

<!-- trace:exempt reason=document-structure -->
## Phase 2 started (retrieval lenses; production ranker unchanged)

- Subtoken tokenizer (ACRONYMWord, min length 2) + BM25 field weights as named constants.
- Query-shape router (conservative: one `path:line` is not a stack trace).
- RankingArm A–I; `collect_relevance_candidates` returns empty for ProductionBlended.
- Exact-name anchors are a boolean, not mixed into the BM25 number.
- Query-text mentions (path / dotted / backtick) mark corpus matches as anchors; plain prose never qualifies.
- Markdown backticks become `DECLARED_AS` (provenance DECLARED), never CALLS, never PPR.
- Co-change partners are inspectable relevance candidates (`cochange` / `cochange-surprise`); `NoCochange` skips them.
- `scc bench retrieval` reports Recall@1/5/10 and MRR over `benchmarks/tasks.json` gold. Production fused ranker is unchanged.
- BM25 corpus stats persist in store meta (`bm25_corpus`); experimental arms score with corpus IDF when meta is present. Production `build_surface` does not read this meta.
- Measured 21-task ablation (`benchmarks/results/ripwire-lessons-retrieval-arms.json`): lexical-then-graph MRR 0.831 / R@10 0.561; production-blended MRR 0.645 / R@10 0.594. Mixed — **do not switch production**.

<!-- trace:exempt reason=document-structure -->
## Phase 3 started (task context)

- TESTS section is tests_to_run with reasons (`direct` / `import` / `contract` / `state`).
- Fixed-percent rollover allocator (20/25/30/10/10/5) is implemented and tested; production packer stays adaptive-priority until ablation prefers rollover. `finish_with_rollover` discloses `quota:<bucket>` truncation.
- Stack/error FILE:LINE loci seed Task Context (mapped files + innermost enclosing symbol); unmapped frames are disclosed, not fabricated.
- Task packs append an **EXACT SOURCE** section last (priority 1, first dropped). Truncation is disclosed (`shown=`/`total=`/`capped=`). Not a fifth context level.

<!-- trace:exempt reason=document-structure -->
## Phase 4 started (honest language matrix)

- C/C++/ObjC/C#/Ruby/PHP/Lua/Swift/Kotlin are **scan-classified** as IndexSearch. Classification is not an extractor. No `.scm` rewrite of semantic walkers.

<!-- trace:exempt reason=document-structure -->
## Phase 5 started (agent loop)

- `scc bench loop` compares baseline (lexical file overlap) vs SCC `task_context` vs black-box Ripwire `--pack-task`. Metrics are clustered by fixture repo. Missing Ripwire is `skipped`, not a win. CLI also searches `/tmp/vendor/ripwire/build*` (not in unit tests).
- Measured 21-task locator run with a real Ripwire binary (`benchmarks/results/ripwire-lessons-locator-loop.json`): clustered localization baseline 0.948, SCC 1.000, Ripwire 0.722 (status `ran`). tests_to_run clustered_tests baseline 0.000, SCC 0.833, Ripwire 0.000 (7 gold-test tasks). This is pack file-name localization plus tests_to_run scoring, **not** an LLM/SWE-bench agent-loop.
- `scc bench loop --explore` is a JSONL pack-consumer protocol (baseline grep+read, SCC `task_context`+read, Ripwire `--pack-task`+read) scored with bench-agent metrics. `SCC_EXPLORE_AGENT_CMD` is the LLM plug-in. Still not SWE-bench repair.
- Measured 21-task explore run with a real Ripwire binary (`benchmarks/results/ripwire-lessons-explore-loop.json`): clustered localization baseline 0.948, SCC 1.000, Ripwire 0.722 (all `ran`). SCC `mean_search` 0.0 and substitution 1.0 (task_context then FETCH/read, no grep). Ripwire first-correct on 20/21 tasks; SCC 21/21. tests_to_run clustered_tests baseline 0.000, SCC 0.833, Ripwire 0.000 (7 gold-test tasks; SCC hits 5/7). This is still a deterministic pack-consumer, **not** an LLM/SWE-bench repair claim.

<!-- trace:exempt reason=document-structure -->
## Phase 6 started (system moat)

- Per-function store R/W/query/publish lines feed State Authority; component `owns` stays write-derived; readers are not owners.
- Runtime matching CALLS gain `OBSERVED_AS`; EXTRACTED/RESOLVED CALLS are not rewritten; observed-only traffic is not invented as CALLS.
- Impact discloses forgotten co-change partners without merging them into EXTRACTED impact sets.
- `.proto` is DataConfig; rpc CONTRACTs link via INVOKES/IMPLEMENTS. Cross-language CALLS are not emitted.

<!-- trace:exempt reason=document-structure -->
## Phase 7 started (type narrowing, JSONL explore, hash sweep)

- Unique local constructor/annotation/param type binds (Python + TypeScript) narrow NamedVariable calls to `{Type}.method` when that method exists locally or on the imported type. Two types tombstone. No same-name spray. EXTRACTED provenance. Binds are extract-time only.
- Incremental refresh re-extracts files that IMPORT or CALL into a changed/removed path (hash-unchanged dependents). Type-narrowed CALLS live on the caller; purging only the callee is not enough for incremental≡cold.
- `replace_components` drops derived component *entities* that vanished after a clustering topology change (merged `root+services` must not survive a later split). The `components` table was already replaced; the entities table was INSERT-OR-REPLACE only.
- Task Context treats every component that CONTAINS a file as affected (merged cluster and member regions). Context-benchmark recall counts a merged `root+services` (id `root-services`) as covering gold `root` and `services`.
- `scc bench loop --explore` emits JSONL grep/read/`task_context` events and scores them with bench-agent metrics. Default loop stays locator. Missing Ripwire is skipped. `SCC_EXPLORE_AGENT_CMD` is the LLM plug-in. Not a SWE-bench repair claim.
- Watcher start/watch failure falls back to `refresh_stale_by_hash`. Hash remains authority.

<!-- trace:exempt reason=document-structure -->
## Phase 8 started (field-type one-hop)

- Unique class field types (Python `self.x = Order()`, class `x: Order`, param assigned to `self.x`; TypeScript class fields and `this.x = new Foo()`) pin `self.x.m()` / `this.x.m()` when that method exists. Two types tombstone. Longer chains stay unresolved. EXTRACTED provenance. Binds extract-time only.
- Writable-matrix agent label is inferred from argv[0] basename (`infer_agent_name`); native and aider/repomix cells share `run_writable_variant`. Paired CIs and `micro_task_success` are named in the write-protocol contract.

<!-- trace:exempt reason=document-structure -->
## Phase 9 started (Java field-type one-hop)

- Unique Java class field types (`private Order repo`, `this.x = new Foo()`, param assigned to `this.x`) pin `this.x.m()` when that method exists. Two types tombstone. Longer chains stay unresolved. EXTRACTED provenance. Binds extract-time only. No C++ unprefixed field lookup.

<!-- trace:exempt reason=document-structure -->
## Phase 10 started (Go receiver-field one-hop)

- Unique Go struct field types (`owned *Order`) plus named method receivers (`func (s *Svc) Run`) pin `s.owned.Process()` when `s` is uniquely typed as the enclosing struct. Two types tombstone. Longer chains stay unresolved. Import chains (`db.users.Find`) stay unresolved. EXTRACTED provenance. Binds extract-time only.

<!-- trace:exempt reason=document-structure -->
## Phase 11 started (Rust self-field one-hop)

- Unique Rust struct field types (`owned: Order`, `Box<Order>`, `&Order`) pin `self.owned.process()` when that method exists. Two types tombstone. Longer chains stay unresolved. EXTRACTED provenance. Binds extract-time only.

<!-- trace:exempt reason=document-structure -->
## Phase 12 started (Java unprefixed field-as-receiver + P26 cache refuse)

- Java `owned.process()` (NamedVariable, two path segments, no `this.`) pins to the unique field type of the enclosing class when that method exists. A parameter, local, enhanced-for, or catch of the same name shadows the field (even untyped) and vetoes the narrow. Two field types tombstone. Longer chains stay unresolved. Python/TS/Go/Rust are not gated through this rule. EXTRACTED provenance. Binds extract-time only.
- `Store::open` refuses truncated or non-SQLite `scc.db` files (`StoreError::Corrupt`) instead of applying migrations onto garbage and serving an empty index. Empty files remain a fresh index.
- `scc context structural --files <stale scc:// handle>` prints `# HANDLE REFUSED` and does not guess a target.

<!-- trace:exempt reason=document-structure -->
## Phase 13 started (Go assignment tombstone + FETCH handles in Task Context)

- Go `s.owned = &Invoice{}` (one-hop, root uniquely typed as the enclosing struct) records a class-scoped field type bind. Declared `owned *Order` plus assignment `Invoice` tombstones; `s.owned.Process()` stays unresolved. Same-type assignment still pins. EXTRACTED provenance. Binds extract-time only.
- Task Context stamps a compact **FETCH** section (`path handle=scc://...`) so agents can lazy-fetch Level 3 without a fifth context level. EXACT SOURCE headers also carry the handle. Explore pack-consumer prefers FETCH handles and **does not guess** a path when every handle is stale.

<!-- trace:exempt reason=document-structure -->
## Phase 14 started (Rust self-field assignment tombstone)

- Rust `self.owned = Invoice {}` / `Invoice::new()` (one-hop `self` field) records a class-scoped field type bind. Declared `owned: Order` plus assignment `Invoice` tombstones; `self.owned.process()` stays unresolved. Same-type assignment still pins. EXTRACTED provenance. Binds extract-time only.

<!-- trace:exempt reason=document-structure -->
## Phase 15 started (Go/Rust local and param type narrowing)

- Unique Go `x := &Order{}` / `var x *Order` / `func handle(x *Order)` and unique Rust `let x = Order {}` / `fn handle(x: Order)` pin `x.Process()` / `x.process()` when that method exists. Two types tombstone. A later conflicting assignment (`z = &Invoice{}` / `z = Invoice {}`) is extract-time tombstone fuel. Same-type assignment still pins. Longer chains stay unresolved. Opaque factory calls (`MakeOrder()`, `make_order()`) do not mint a bind. EXTRACTED provenance. Binds extract-time only.

<!-- trace:exempt reason=document-structure -->
## Phase 16 started (type assertion / conversion / cast)

- Unique Go `x := v.(*Order)` / `y := Order(v)` (simple type only; generic `fs[i](3)` is not a bind), unique Rust `let y = v as Order`, and unique TypeScript `const y = v as Order` / `const z = <Order>v` pin `x.Process()` / `x.process()` when that method exists. Two types tombstone. Longer chains stay unresolved. Opaque factory calls still do not mint a bind. EXTRACTED provenance. Binds extract-time only.

<!-- trace:exempt reason=document-structure -->
## Phase 17 started (Python identifier RHS copy)

- Unique Python `y = x` when `x` is a uniquely typed local or parameter pins `y.m()` to that type when the method exists. Two types tombstone. A later conflicting constructor assignment is tombstone fuel. Longer chains stay unresolved. EXTRACTED provenance. Binds extract-time only.

<!-- trace:exempt reason=document-structure -->
## Phase 18 started (tests_to_run scored in the agent loop)

- Locator and explore arms parse SCC TESTS rows (`- name (file) — reason`) and Ripwire `<test p="...">` pack rows and score `tests_localization` against `tasks.json` `ground_truth.tests`. Baseline has no tests_to_run list and scores 0 when gold tests exist. Empty gold tests are omitted from the clustered mean, not scored as 1.0. FETCH `- path handle=` rows are not tests. Ripwire file-only rows do not match gold function names. Production ranking, MCP tool count, context levels, and packer quotas are unchanged. This is still a deterministic pack-consumer, not an LLM/SWE-bench repair claim.
- Measured 21-task locator and explore runs with live Ripwire (`benchmarks/results/ripwire-lessons-{locator,explore}-loop.json`): tests_tasks=7; clustered_tests baseline 0.000, SCC 0.833, Ripwire 0.000. SCC hits 5/7 gold-test tasks (Python fixtures plus large-ts, monorepo, queue-worker-ts). File localization unchanged (SCC 1.000, baseline 0.948, Ripwire 0.722).

<!-- trace:exempt reason=document-structure -->
## Phase 19 started (Java cast as Rule 2 fuel)

- Unique Java `(Order)v` pins `x.process()` for `var x = (Order)v` and later `x = (Order)v` assignment when that method exists. `this.owned = (Order)v` is class-scoped field fuel. Two types tombstone. Array and generic casts do not mint a bind. Opaque factories still do not. Longer chains stay unresolved. EXTRACTED provenance. Binds extract-time only.

<!-- trace:exempt reason=document-structure -->
## Phase 20 started (tests_to_run recall for TypeScript it() titles)

- Gold kebab-case names match SCC TESTS titles by ident-token sequence (`expands-department-street-names` ≡ `expands department street names`). Ripwire file-only `<test p>` rows still do not match function/title gold. `queue-worker-ts` test imports resolve (`./text`, `../geo/resolver`) so import-reason tests_to_run can fire. Production ranking, MCP tool count, context levels, and packer quotas unchanged.
- Measured after this change: clustered_tests SCC 0.833 vs baseline/Ripwire 0.000 (7 gold-test tasks). ts-api-web.contract-field and creation-test still miss **until ident-token matching is ASCII-case-insensitive** (`API` vs `api`). Packs already list `joins user names from the API response`; the matcher was the gap.

<!-- trace:exempt reason=document-structure -->
## Phase 21 started (Rule 2c class-name receiver)

- `Cls.m()` classified as StaticType pins the unique in-repo Class/Interface/Type named `Cls` that uniquely defines `m` (same-file or cross-file; Go structs are `Type`, Rust structs are `Class`). Two same-named class-like defs that both define `m` stay unresolved — no same-name spray.
- A unique typed local/param named `Cls` is Rule 2 and wins (`def handle(Order: Invoice): Order.process()` → `Invoice.process`).
- Any bind for that name, including Python/TS untyped-parameter empty-type shadows, vetoes the class-name pin.
- Modules are not class-like; `TypeQualified` `Foo::bar` is unchanged. No CHA/base walk in this phase (`IERS_B.open()` still unresolved unless `IERS_B` itself defines `open`). EXTRACTED provenance. Binds extract-time only. Production ranking, MCP tool count, and context levels unchanged.

<!-- trace:exempt reason=document-structure -->
## Phase 22 started (Rule 2c CHA / methodOnTypeOrBases)

- When `Cls.m()` or a unique typed receiver of type `Cls` does not define `m`, walk simple-ident bases breadth-first (cap 16 visited names). Pin only if the unique in-repo class-like named `Cls` has heritage and the shallowest level has exactly one hitting base that uniquely defines `m` (`IERS_B.open()` → `IERS.open`).
- Two hitting bases at one level stay unresolved. Two same-named class-likes refuse heritage merge (no Ripwire name-keyed CHA spray). Python/Java/TS extract heritage. Persist `class_bases` on CLASS entity attributes, never FILE, so incremental caller edits still pin. Modules are not class-like. Rule 1 `self`/`this`/`super` walk is Phase 23. EXTRACTED provenance. Production ranking, MCP tool count, and context levels unchanged.

<!-- trace:exempt reason=document-structure -->
## Phase 23 started (Rule 1 self/this/super base walk)

- When `self.m()` / `this.m()` is not a same-file sibling of the enclosing class, pin via unique class-like plus CHA (`IERS_B.run` `self.open` → `IERS.open`). Own-class sibling still wins. `super.m()` and Python `super().m()` walk bases only (`skipSelf`) and never pin the enclosing class's own `m`. Two hitting bases at one level stay unresolved (no `unionOnMulti`, no first-declared MRO). Classify `super()` as `RecvKind::Super`. Field chains are not Rule 1. No C++ bare-name Rule 1. EXTRACTED provenance. Production ranking, MCP tool count, and context levels unchanged.

<!-- trace:exempt reason=document-structure -->
## Phase 24 started (Rust `impl Trait for T` CHA heritage)

- Ripwire `rustImplVisitNode` emits inherit Extends (derived T, base Trait). SCC extracts simple-ident trait bases onto T (`impl Open for IERS_B` → IERS_B bases `[Open]`; `impl std::fmt::Display` → `Display`). Multiple impls merge. Trait methods are `Trait.m` so unique-class-like CHA can pin a typed `x.open()` / `IERS_B.open()` to the unique hitting trait method when T does not define it. Inherent `impl T { fn m }` still wins. Two traits defining the same method at one level stay unresolved. Persist on CLASS entity attributes like other heritage. No Go embedding (Ripwire `captureBases` does not include Go). No C++ extractors. EXTRACTED provenance. Production ranking, MCP tool count, and context levels unchanged.

<!-- trace:exempt reason=document-structure -->
## Working order

1. Truth foundation (this PR)
2. Retrieval (BM25, routing, anchors, mentions, co-change, ablations)
3. Task context (packs, tests_to_run, budget rollover, stack locus)
4. Language breadth
5. Agent loop vs baseline + Ripwire
6. System moat (state, runtime, cross-language, contracts, flows)
7. Type narrowing + JSONL explore protocol + watcher hash sweep
8. Field-type one-hop (`self.x.m()` / `this.x.m()`)
9. Java field-type one-hop (`this.x.m()`)
10. Go receiver-field one-hop (`s.x.m()` when `s` is the receiver)
11. Rust self-field one-hop (`self.x.m()`)
12. Java unprefixed field-as-receiver (`repo.save()`) + corrupt-cache refuse
13. Go assignment field-type tombstone + Task Context FETCH handles
14. Rust self-field assignment tombstone
15. Go/Rust local and param type narrowing
16. Type assertion / conversion / cast as Rule 2 fuel (Go, Rust, TypeScript)
17. Python identifier RHS copy bind (`x = y` when `y` is uniquely typed)
18. Score tests_to_run vs gold tests in locator/explore agent loop
19. Java `(Order)v` extract-time cast as Rule 2 fuel
20. tests_to_run recall: kebab gold vs `it()` titles; fix queue-worker-ts test imports; ASCII case-fold so `API` matches `api`
21. Rule 2c class-name receiver (`Cls.m()`) + local/untyped-param shadow veto
22. Rule 2c CHA/base walk (`IERS_B.open()` → `IERS.open`) + CLASS-entity heritage persist
23. Rule 1 self/this/super base walk (`IERS_B.run` `self.open` / `super().open` → `IERS.open`)
24. Rust `impl Trait for T` CHA heritage (`x.open()` on IERS_B → Open.open)
