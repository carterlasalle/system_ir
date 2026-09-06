# PLAN-ripwire-lessons-phase2

<!-- trace:v1 id=PLAN-ripwire-lessons-phase2 type=plan work=WORK-ripwire-lessons-phase2 implements=REQ-subtoken-bm25,REQ-query-shape-router,REQ-ranking-arms-inspectable,REQ-exact-anchors,REQ-query-mentions,REQ-declared-mentions,REQ-cochange-retrieval-evidence,REQ-retrieval-eval -->

<!-- trace:exempt reason=document-structure -->
## Steps

1. Port Ripwire's *observed* subtoken state machine (ACRONYMWord, min length 2) into `scc-core` as the single tokenizer.
2. Implement deterministic BM25 over named fields; keep weights/k1/b as named constants.
3. Classify query shape conservatively; extract FILE:LINE loci for stack/error queries.
4. Expose RankingArm A–I; default A is identity with today's fused surface.
5. Score heterogeneous entities with BM25 + exact-anchor flags without wiring that blend into `build_surface`.
6. Unit tests pin acronym splits, BM25 determinism, shape conservatism, and "production default is not BM25-fused".
7. Extract query mentions (path / dotted / backtick) and mark corpus matches as anchors without touching BM25.
8. Index markdown backticks as DECLARED_AS to known entities; unmatched names are gauges, not CALLS.
9. Attach co-change partners as inspectable relevance candidates; NoCochange skips them.
10. Add Recall@k / MRR plus `scc bench retrieval` over fixture gold; do not switch production ranking.
