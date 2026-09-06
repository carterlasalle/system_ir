# Ripwire lessons Phase 2 — retrieval lenses

<!-- trace:v1 id=doc.spec-work-ripwire-lessons-phase2 type=document work=WORK-ripwire-lessons-phase2 -->

<!-- trace:exempt reason=document-structure -->
## Goal

Give SCC a relevance-first retrieval path learned from Ripwire's *implementation* (not its README): identifier subtokens, BM25 with field weights, query-shape routing, and exact-name anchors. Do not change the production fused ranker until ablation evidence exists. Do not add embeddings. Do not collapse Atlas/PPR into BM25.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-subtoken-bm25 — Shared subtoken tokenizer and BM25 lens

<!-- trace:v1 id=REQ-subtoken-bm25 type=requirement work=WORK-ripwire-lessons-phase2 -->

Identifier tokenization must use one state machine for query and documents: camelCase and snake_case splits, ACRONYMWord (`HTTPServer` → `http` + `server`, `MCP` stays `mcp`), tokens shorter than 2 dropped. BM25 uses k1=1.5, b=0.75, Lucene-style IDF `ln((N-df+0.5)/(df+0.5)+1)`, field weights name=3, path=2, doc=2, body=1. Scoring is deterministic (doc order for stats, rank tie-break by id). Query-side stemming stays off until a benchmark justifies it. This lens must not be fused into PageRank by default.

### REQ-query-shape-router — Query shape routing without changing production ranking

<!-- trace:v1 id=REQ-query-shape-router type=requirement work=WORK-ripwire-lessons-phase2 -->

Classify a query from text alone into at least: identifier, symbol-like, path-like, stack-trace, error-message, conceptual, architecture, flow, state, impact. Conservatism: a single generic `path:line` is not a stack trace; URLs with ports must not fire. Identifier-shaped queries prefer exact/subtoken name before body vocabulary. Conceptual queries prefer BM25. Stack/error queries extract FILE:LINE loci. Architecture/flow/state/impact set routing flags for later packers. Production `build_surface` remains the fused ranker until an ablation arm wins.

### REQ-ranking-arms-inspectable — Ranking arms A–I stay separately inspectable

<!-- trace:v1 id=REQ-ranking-arms-inspectable type=requirement work=WORK-ripwire-lessons-phase2 -->

RankingArm enumerates production blended (A), lexical-then-graph (B), query-routed (C), no-graph (D), no-lexical (E), no-semantic (F), no-cochange (G), no-task-PPR (H), no-global-PPR (I). Default is A. Relevance (BM25/anchors) and importance (PPR) are separate scores on a hit; they must not be collapsed into one number unless an experiment arm explicitly blends them. No production ranking change without a retrieval eval.

### REQ-exact-anchors — Query-named entities are anchors, not just BM25 hits

<!-- trace:v1 id=REQ-exact-anchors type=requirement work=WORK-ripwire-lessons-phase2 -->

If the query text names a known entity (symbol, component, route, contract, state, …) after identifier normalization, that entity is an exact anchor. Anchors are recorded as a boolean on the hit, not mixed into the BM25 score. Whole-query exact-name matching lives here; embedded path/dotted/`backtick` mentions are REQ-query-mentions.

### REQ-query-mentions — Query text mentions are anchors, not BM25 terms

<!-- trace:v1 id=REQ-query-mentions type=requirement work=WORK-ripwire-lessons-phase2 -->

Extract explicit mentions from the query/task text the way Ripwire `extractMentions` does in code: `/`-joined paths, dotted identifier chains, and `backticked` identifiers. Plain prose words never qualify (precision over recall). Match mentions against the indexed corpus (entity name, path suffix, basename, basename-sans-extension), never the live filesystem. A matched entity is an exact-anchor flag; mention matching must not be added into the BM25 number and must not silently change production `build_surface`.

### REQ-declared-mentions — Markdown backticks are DECLARED evidence

<!-- trace:v1 id=REQ-declared-mentions type=requirement work=WORK-ripwire-lessons-phase2 -->

Markdown (and MDX) backtick spans outside fenced code, of identifier length ≥ 3, that match known heterogeneous entities become `DECLARED_AS` facts from the doc FILE to the matched entity, provenance DECLARED. They must never become CALLS edges and must never enter PageRank adjacency. Unmatched backticks are not facts (the claim degrades); they may increment `unmatched_doc_mentions` in store-meta `analysis_quality` only — never FILE entity attributes. EXTRACTED/RESOLVED code is authority when a doc names something the index does not contain.

### REQ-cochange-retrieval-evidence — Co-change is inspectable historical evidence

<!-- trace:v1 id=REQ-cochange-retrieval-evidence type=requirement work=WORK-ripwire-lessons-phase2 -->

Git co-change pairs may appear on the experimental relevance lens as extra candidates (`reason=cochange` or `reason=cochange-surprise` when the pair has no static IMPORTS/CALLS coupling). The NoCochange ranking arm must omit them. Co-change must not be fused into BM25 or production PPR and must never override current EXTRACTED/RESOLVED semantic truth.

### REQ-retrieval-eval — Recall@k and MRR over gold, before any production switch

<!-- trace:v1 id=REQ-retrieval-eval type=requirement work=WORK-ripwire-lessons-phase2 -->

A retrieval harness must compute Recall@1/5/10 and MRR against `benchmarks/tasks.json` gold (files, symbols, components, routes) for inspectable ranking arms including ProductionBlended (today's surface), LexicalThenGraph, and QueryRouted. `scc bench retrieval` prints the table. Production ranking must not change because an arm looks better on this corpus alone.

### REQ-bm25-persist — Corpus BM25 stats live in store meta

<!-- trace:v1 id=REQ-bm25-persist type=requirement work=WORK-ripwire-lessons-phase2 -->

Index persists BM25 corpus statistics (`n`, `avgdl`, per-term `df`, per-doc `dl`) in store meta key `bm25_corpus`, never on FILE entity attributes. Scoring a document set with those stats must match cold `bm25_scores` when the set *is* the corpus. Using corpus IDF on a candidate slice is allowed on the experimental lens only. Production `build_surface` must not read this meta.
