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
| Language adapters | Scan classifies py/ts/js/go/rs/java + json/yaml/toml/env/docker/compose/tf/md/shell/sql. C/C++/ObjC/C#/Ruby/PHP/Lua/Swift/Kotlin **not classified**. | Broad tree-sitter surface including C/C++/ObjC/C#/Ruby/PHP/Lua/Swift, etc. | Breadth. | Honest tiers once registry exists; config/infra extractors Ripwire does not treat as system entities. | Yes (honest matrix now; languages later) | `LANGUAGE_REGISTRY` + `scc languages`. Do not claim support from parse-only. Add languages only with fixtures. | Fake "supported" claims. | Registry uniqueness; scan↔registry; extracted ⇒ tier A. | P10 / Phase 1 SoT, Phase 4 breadth |
| Symbol model | File-qualified `symbol_id(repo, path, name)`. Methods `Class.method`. Overload index. | Dense IDs, path-qualified names, fields as a **side table** (not PageRank symbols). | Compact adjacency; fields don't pollute PageRank. | Heterogeneous entities (component, route, contract, state, schema). | Investigate | Optional field table if ranking ablations show field-name collision harm. Do not reduce components to symbols. | Polluting PPR with fields. | Ranking ablation with/without fields. | P1 later / Phase 2 |
| Reference model | `ReferenceKind` includes Call/Read/Write/Import/… plus Macro/FieldAccess/Construct. Writer still emits CALLS edges only for resolved calls. | `RefRole`: Call/Read/Write/Import/Extends/Macro/Type. Only Call+Macro enter CSR. | Roles are first-class at graph build. | Broader semantic predicates (HANDLES, PUBLISHES, READS, WRITES, …). | Yes | Capture role at extract; skip non-execution roles in CALL ranking. Keep semantic predicates. | Treating imports as calls. | Resolver tests: macros not CALLS. | P1 / Phase 1 |
| Receiver-aware resolution | **Now:** `RecvKind` + `classify_callee` + conservative `resolve_calls`. this/self → enclosing class sibling only. Field chains do not pin intermediate/terminal method names. Imported field-chain roots (`import { db }` + `db.users.findMany`) seed the imported object without claiming `findMany` resolved. Named var no spray. Super unresolved. | `resolve.h` Narrower with RecvKind ThisObj/NamedVar/FieldOfThis/FieldOfVar/SuperObj. Type narrowing from locals/fields/params. | Deeper type narrowing and C++/ObjC shapes. | Conservative incomplete edges + provenance taxonomy. | Yes, incrementally | Phase 1: classification + no spray + import-root seeding. Later: type facts from extractors/LSP/SCIP overlays. | Wrong-but-confident edges. | `this_process_does_not_bind_other_class`, field-chain, import-root, named-var spray tests. | P1 / Phase 1 |
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
| Exact-source retrieval | File reads via CLI/MCP paths. No stable handle. | `sym#idHash@contentHash`; `fetch` verbs; stale hash refuses. | Handles + lazy bodies. | Can stamp handles on heterogeneous entities. | Yes | `scc://` handles on structural_source; refuse stale. No new MCP tool. | Tool explosion. | Handle roundtrip + stale refuse. | P8 / Phase 1 |
| Indexing / incremental | blake3 content hash; purge path; rebuild derived layer. incremental≡cold. | Warm parse-once; dense IDs; persisted lexical stats. | Speed (lexical persist, contiguous storage). | Correctness-first incremental. | Absorb speed later | Profile first (P27). Equivalence gates for every fast path (P25). | Unsafe incremental cleverness. | incremental_after_edit == fresh_rebuild. | P13 / P25 |
| Filesystem watching | `notify` debounce → refresh_paths. Hash still authority. | kqueue/inotify/ReadDirectoryChangesW; fallback sweep. | Platform watchers. | Already "watcher is not authority". | Keep contract | If watcher fails, stat/hash sweep. Determinism: watcher must not change result bytes. | OS-dependent output. | Watcher path == sweep path. | P12 |
| MCP tools | **Exactly 10** intent-level tools. | **31 verbs** (`mcpverbs.h`). | Fine-grained latency/token control. | Agents don't orchestrate 8 tools for one impact question. | No 31-tool clone | Enrich internals of existing tools. Lower-level only for fetch/edit/diagnostics. | Skill/tool overhead (Ripwire pilot). | `tools().len() == 10`. | P18 |
| Task packs | Adaptive sections; critical never cut; dropped_sections disclosed. | Fixed quotas (rank 40 / bodies 30 / callers 15 / notes 5 / tests 10) + rollover; bodies last. | Simple robust allocation; tests_to_run. | Atlas + contracts + provenance in the pack. | Benchmark both | Budget allocator abstraction (Phase 3). Do not assume adaptive wins. | Silent cap. | Truncation disclosure tests. | P5 / P6 / Phase 3 |
| Change-risk / git | Co-change CLI exists (`cmd_cochange`); git revision on snapshot. | gitmine, cochange, whereis, stray_content, merge_scout, dirty-tree situ. | Mature change intelligence. | Semantic impact (contracts/state/flows) possible. | Selective | Co-change as historical evidence, never override current semantics. Forgotten partners in impact_context. Skip generic git utilities. | History overriding truth. | Co-change fixtures. | P4 / P15 / Phase 2–3 |
| tests_to_run | Tests extracted; tested_by relink on change. Pack lists tests weakly. | `testmap.h` + pack `tests_to_run`. | Actionable verify list with reasons. | Can reason via contracts/state/flows. | Yes | Phase 3: tests_to_run with reasons (direct, contract, state, co-change, e2e). | Filename-only lists. | Behavioral fixtures. | P16 / Phase 3 |
| Tests / evals | Deterministic 21-task behavioral corpus; atlas recall; resolution bench; golden incremental. | locbench, agentloop (baseline / ripwire_cli / ripwire_skills), contamination, clustered stats. | Real agent-loop outcomes. | Stronger semantic corpus + Atlas GT. | Mandatory add | Keep 21-task. Add SWE-bench-style agent-loop vs baseline + Ripwire (Phase 5). | Contamination; model substitution. | Multi-seed, clustered stats, resolved primary. | P20 / Phase 5 |
| BM25 / lexical | **Now:** experimental `scc-core` subtoken BM25 lens (name/path/doc/body weights, k1=1.5, b=0.75) + query-shape router + RankingArm A–I. Production fused ranker **unchanged**. | Custom BM25: camel/snake split, field weights, persisted stats, top-K prune. | Measured retrieval; persisted stats. | Semantic + graph + Atlas. | Yes as separate lens | Implemented tokenizer+BM25+router; do not fuse into PPR without ablation evidence. Persist stats later. | Embeddings cargo-cult; silent fusion. | Acronym pins; BM25 determinism; production arm emits nothing from BM25 lens. | P3 / Phase 2 |
| Anchors / mentions | Goal terms / explicit files/symbols. No doc→symbol mention index. | Query anchors; doc mentions. | Name-first when the query names a symbol. | Heterogeneous anchors (component, route, contract, …). | Yes | Phase 2: resolve anchor first; mentions as DECLARED evidence, degradable. | Docs as truth. | Mention vs code contradiction tests. | P4 / Phase 2 / P33 |
| Symbol-addressed edits | None. | replace_symbol_body / insert_before/after; stale hash refuses; file lock. | Agent can edit by symbol. | Receipt could include contracts/state/tests. | Evaluate first | Do **not** add in Phase 1. If later: refuse stale/malformed; byte-identical on refuse; reindex; semantic receipt. | Races, silent wrong target. | Byte-identity on refuse; stale handle. | P9 hold |
| Agent reflex / skills | Startup context injection; Claude/Codex/OMP plugins. | skills/hooks/wrap; pilot showed overhead can **increase** tokens. | Aggressive default tool use. | Automatic startup Task Context. | Measure | Prefer SCC before grep; measure substitution rate. Don't assume more instructions win. | Token bloat. | SCC_calls / (SCC + native reads). | P19 / Phase 5 |
| Determinism | Content-hash ids; BTree maps; golden tests. | Obsessive stable serialize; tie-break. | Byte-stable CLI. | Hash-stable ids. | Tighten | Explicit tie-breaks; watcher/FS order must not change output. | Hidden HashMap iteration. | Repeated-run byte tests where appropriate. | P23 |
| Single source of truth | Some duplicated lists (README langs vs extractors). `predicates::ALL` had a real gap (fixed). | `kLangTable` SoT; they learned from drift. | Aggressive SoT. | Schema + kinds modules. | Yes | Registry-driven langs, roles, tools. Tests fail on drift. | Hand-maintained docs. | Mutation: drop mapping → test fails. | P24 / Phase 1 langs |
| Failure injection | Some (unreadable file, empty index). Incomplete vs the P26 list. | Stronger adversarial coverage in places. | — | Incremental equivalence. | Expand | Phase 1+: stale handle, same-name defs, watcher failure, cache corruption. | Silent recovery that fabricates certainty. | Each failure degrades or refuses. | P26 |
| Runtime evidence | OTel ingest; static vs observed reconcile. | Essentially static. | — | Unique moat. | Strengthen | Observed upgrades EXTRACTED edges; do not overwrite history (P32). | Mixing static/runtime. | Existing runtime tests + more traces. | P32 / Phase 6 |
| Cross-language | Limited (TS↔JS share extractor). | Some C↔C++/ObjC. | Native ABI shapes. | Reality Graph can represent JNI/RPC/proto semantically. | Yes, SCC-native | Bridges as semantic edges with provenance, not forced CALLS. | Fake CALLS across langs. | Fixtures per bridge. | P1 later / Phase 6 |
| Precision overlays | SCIP import + LSP (pyright, tsserver) exist. | SCIP overlay. | Hardened SCIP path. | Overlays should **upgrade** edges, not fork a graph. | Continue | Provenance `RESOLVED` + resolver name. Optional Ripwire backend experiment. | Dual graphs. | Differential resolution bench (exists). | P31 |
| Output format | Structured packs; warnings; dropped_sections. | Compact headers with stats. | Analyzer-health in the first screen. | Semantic richness. | Compact gauges | `analysis_quality` compact line + JSON field. Caps disclosed. | Noisy prose. | Pack JSON schema-ish assertions. | P28 / Phase 1 |
| Representation selection | Caller chooses Atlas vs Surface vs Structural. | Packer chooses signatures vs bodies by budget. | Automatic cheapest sufficient form. | Four levels. | Policy later | Task-type + cost (P29). Phase 1: exact vs structural cost only. | Agent still spelunks. | P30 benchmark. | P29 / Phase 3 |
| Non-local mutable state | Store refs / READS / WRITES predicates from extractors (not SSA). | Per-function nonlocal analysis. | Function-level completeness. | System-level authority. | Integrate | Phase 6. | — | Per-function fixtures. | P14 |
| Stack/error locus | Not a first-class ingest. | FILE:LINE / locpin. | Fast localization. | Can climb to component/flow/state. | Yes | Phase 3 ingest traces → symbols → Atlas explanation. | Stopping at the frame. | Trace fixtures. | P17 / Phase 3 |
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
- SWE-bench agent-loop — Phase 5 (mandatory before product claims).
- Copying Ripwire quality/linter commands.

<!-- trace:exempt reason=document-structure -->
## Phase 2 started (retrieval lenses; production ranker unchanged)

- Subtoken tokenizer (ACRONYMWord, min length 2) + BM25 field weights as named constants.
- Query-shape router (conservative: one `path:line` is not a stack trace).
- RankingArm A–I; `collect_relevance_candidates` returns empty for ProductionBlended.
- Exact-name anchors are a boolean, not mixed into the BM25 number.
- Not yet: persisted lexical stats, mentions/co-change ranking, production switch, retrieval Recall@k harness over holdout gold.

<!-- trace:exempt reason=document-structure -->
## Working order

1. Truth foundation (this PR)
2. Retrieval (BM25, routing, anchors, mentions, co-change, ablations)
3. Task context (packs, tests_to_run, budget rollover, stack locus)
4. Language breadth
5. Agent loop vs baseline + Ripwire
6. System moat (state, runtime, cross-language, contracts, flows)
