# Wave 15.2 — heterogeneous hierarchy edges, semantic scoring, explain, rank caching

<!-- trace:v1 id=doc.wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching -->

<!-- trace:exempt reason=document-structure -->
## Goal

Complete the reviewer's 7 remaining items: full containment hierarchy in the rank graph (System-Subsystem-Service-Component-File-Symbol), HandledBy/DefinedBy ranking transitions, live semantic 10% in final importance, real --explain score decomposition, per-ModelEpoch rank caching with single-pass startup, hard-max invariant on rendered text, adaptive startup budgets.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-full-containment-hierarchy-in-rank-graph — Full containment hierarchy in rank graph

<!-- trace:v1 id=REQ-full-containment-hierarchy-in-rank-graph type=requirement work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching -->

CONTAINS ranking must traverse System-Subsystem-Service-Component-File-Symbol with directional weights, not only Component->File and Component->Symbol.

### REQ-handled-by-defined-by-ranking-transitions — HandledBy/DefinedBy ranking transitions

<!-- trace:v1 id=REQ-handled-by-defined-by-ranking-transitions type=requirement work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching -->

Route->handler and Schema->definer ranking-only reverse edges must exist and project_to_symbols must include HANDLES/DEFINES.

### REQ-semantic-10-live-in-final-importance — Semantic 10% live in final importance

<!-- trace:v1 id=REQ-semantic-10-live-in-final-importance type=requirement work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching -->

The semantic weight in final_importance must be populated by a real SemanticScorer when available, or explicitly redistributed; no phantom weight.

### REQ-explain-renders-score-decomposition — Explain renders score decomposition

<!-- trace:v1 id=REQ-explain-renders-score-decomposition type=requirement work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching -->

--explain must output task_ppr/global_ppr/lexical/semantic/confidence/criticality/change_risk/novelty/reasons per entry, not just importance.

### REQ-global-rank-cached-per-model-epoch — Global rank cached per ModelEpoch

<!-- trace:v1 id=REQ-global-rank-cached-per-model-epoch type=requirement work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching -->

Rank graph, global PPR vector, and surface candidates must be cached per (epoch, policy); startup must not compute the global Surface twice.

### REQ-hard-max-invariant-on-rendered-text — Hard-max invariant on rendered text

<!-- trace:v1 id=REQ-hard-max-invariant-on-rendered-text type=requirement work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching -->

Rendered token count must not exceed hard_max; progressive compression down to symbol identity when required.

### REQ-adaptive-startup-budgets — Adaptive startup budgets

<!-- trace:v1 id=REQ-adaptive-startup-budgets type=requirement work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching -->

Startup budget allocation must scale Atlas/Surface by repo complexity, not a fixed 13:7 split.
