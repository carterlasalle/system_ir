# PLAN-ripwire-lessons-phase1

<!-- trace:v1 id=PLAN-ripwire-lessons-phase1 type=plan work=WORK-ripwire-lessons-phase1 implements=REQ-receiver-aware-resolution,REQ-resolution-honesty-gauges,REQ-stable-content-handles,REQ-language-support-matrix,REQ-ontology-single-source,REQ-exact-source-dominance -->

<!-- trace:exempt reason=document-structure -->
## Steps

1. Code-level gap ledger vs Ripwire (`docs/RIPWIRE_GAP_LEDGER.md`) — docs are hints; implementation is truth.
2. Receiver classification + conservative resolver (no same-name spray, field-chain honesty, unresolved≠external).
3. Persist and expose `analysis_quality` gauges; task packs carry a compact section.
4. Stable `scc://` handles with stale refusal on existing structural_source tools.
5. Exact-source dominance when the on-disk span is cheaper.
6. Generated language-support matrix (`scc languages`) bound to scan classification.
7. Keep Atlas / Reality Graph / TrustedGraphView / provenance / 10 MCP tools. Do not port Ripwire's 31-tool surface or fuse PageRank with BM25 without ablation evidence (Phase 2+).
