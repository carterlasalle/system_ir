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

If the query text names a known entity (symbol, component, route, contract, state, …) after identifier normalization, that entity is an exact anchor. Anchors are recorded as a boolean on the hit, not mixed into the BM25 score. Mentions and co-change remain later slices; this requirement is exact-name anchors only.
