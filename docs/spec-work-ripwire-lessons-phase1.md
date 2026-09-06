# Ripwire lessons Phase 1 — extraction truth foundation

<!-- trace:v1 id=doc.spec-work-ripwire-lessons-phase1 type=document work=WORK-ripwire-lessons-phase1 -->

<!-- trace:exempt reason=document-structure -->
## Goal

Harden SCC's evidence/extraction/retrieval layers using Ripwire as an implementation reference, while keeping SCC's semantic compiler (Atlas, Reality Graph, TrustedGraphView, provenance, FlowGraph, state authority) intact. Phase 1 is the truth foundation: receiver-aware resolution, honesty gauges, stable handles, exact-source dominance, and a generated language-support matrix.

Code is the source of truth. Ripwire docs overstate several gauges; this spec requires SCC to implement the *observed* Ripwire behavior (or better), not README claims.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-receiver-aware-resolution — Receiver-aware reference resolution

<!-- trace:v1 id=REQ-receiver-aware-resolution type=requirement work=WORK-ripwire-lessons-phase1 -->

Native call resolution must classify the receiver (`NONE`, `THIS`, `SELF`, `NAMED_VARIABLE`, `TYPE_QUALIFIED`, `FIELD_OF_THIS`, `FIELD_OF_SELF`, `FIELD_OF_VARIABLE`, `SUPER`, `STATIC_TYPE`, `UNKNOWN_RECEIVER`) from extractor facts or callee text. `this.method()` / `self.method()` bind only sibling methods of the enclosing type. Field-chain receivers (`self.client.process`) must not pin the intermediate name as the callee. Named variables without a type must not spray onto every same-name method. Super calls must not spray onto the caller's class. Prefer a correct incomplete edge over a confidently wrong edge. Non-CALL roles (import, type, macro) must not become CALLS edges.

### REQ-resolution-honesty-gauges — Unresolved is not external; gauges are machine-readable

<!-- trace:v1 id=REQ-resolution-honesty-gauges type=requirement work=WORK-ripwire-lessons-phase1 -->

Failing to resolve a name is not evidence that the target is outside the repository. Relative import misses are `Unresolved`, not `External`. Bare unresolved names are `Unknown` or `UnresolvedLikelyInternal`, never `ConfirmedExternal` unless an import specifier was confirmed external. Each index persists a compact `analysis_quality` snapshot (resolved/precise/heuristic/ambiguous/likely_internal_unresolved/external call counts; parsed/partial/unsupported files; stale_facts_dropped) on FILE entities and as store meta. Task context exposes the compact line by default and the structured object on the pack JSON.

### REQ-stable-content-handles — Stable, stale-refusing content handles

<!-- trace:v1 id=REQ-stable-content-handles type=requirement work=WORK-ripwire-lessons-phase1 -->

Content handles identify heterogeneous SCC objects (`symbol`, `component`, `flow`, `contract`, `state`, `route`, `file`, `span`) as `scc://{repo}/{epoch}/{kind}/{encoded_key}@{content_hash}`. Identity is path-aware (never basename-only). A stale content hash must refuse; refusal must not guess another target. Existing high-level tools (`structural_source` / `scc context structural`) accept handles in the `files` argument. No extra MCP tool.

### REQ-language-support-matrix — Honest generated language matrix

<!-- trace:v1 id=REQ-language-support-matrix type=requirement work=WORK-ripwire-lessons-phase1 -->

`LANGUAGE_REGISTRY` is the single source of truth for language ids, tiers (A semantic/deep, B structural, C index/search, D data/config), extractor presence, and capability flags. `scc languages` prints a matrix generated from that registry. Scan-classified languages (except `other`) must appear in the registry. Claiming an extractor requires tier A plus definitions and calls. Parsing syntax without an extractor is not "supported".

### REQ-ontology-single-source — Predicate and kind registries stay complete

<!-- trace:v1 id=REQ-ontology-single-source type=requirement work=WORK-ripwire-lessons-phase1 -->

`predicates::ALL` and `kinds::ALL` must include every predicate/kind the compiler, ranker, and exporters rely on (including DEFINES, COMPOSES, EXPORTS, ANNOTATES, REGISTERS, INJECTS, HANDLES_CALLBACK, DECORATES, OCCURS, and kinds FIELD, SCHEMA, TRUST_BOUNDARY, OCCURRENCE). Tests must fail if a required entry is dropped from `ALL` while the constant still compiles.

### REQ-exact-source-dominance — Exact source when it is not more expensive

<!-- trace:v1 id=REQ-exact-source-dominance type=requirement work=WORK-ripwire-lessons-phase1 -->

When an on-disk exact span costs no more tokens than the generated Structural/Signatures skeleton, serve exact source and disclose `representation: EXACT` with `reason: exact_body_cheaper_than_structural`. When structural saves tokens, disclose `reason: structural_saved_N_percent`. When no on-disk file exists (synthetic fixtures), keep the generated representation. Never silently substitute one for the other.
