# Rank-edge ontology: CONTAINS/PARTICIPATES_IN/HANDLES/DEFINES enter the PPR adjacency

<!-- trace:v1 id=doc.rank-edge-ontology-c-o-n-t-a-i-n-s-p-a-r-t-i-c-i-p-a-t-e-s-i-n-h-a-n-d-l-e-s-d-e-f-i-n-e-s-enter-the-p-p-r-adjacency -->

<!-- trace:exempt reason=document-structure -->
## Goal

Heterogeneous rankable nodes exist but membership/participation/invocation/definition predicates never enter the PageRank adjacency, leaving components/flows/routes/schemas disconnected from the rank graph. Add an explicit RankEdgeKind ontology (distinct from reference_kind) so CONTAINS (container->member 0.4, member->container ranking transition 0.8), PARTICIPATES_IN (symbol->flow 0.6, flow->participant 1.2), HANDLES (symbol->route 1.5), and DEFINES (symbol->schema 1.2) feed PPR with deliberate direction and weight.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-rank-edges-heterogeneous-predicates-enter-page-rank — Rank edges: heterogeneous predicates enter PageRank

<!-- trace:v1 id=REQ-rank-edges-heterogeneous-predicates-enter-page-rank type=requirement work=WORK-rank-edge-ontology-c-o-n-t-a-i-n-s-p-a-r-t-i-c-i-p-a-t-e-s-i-n-h-a-n-d-l-e-s-d-e-f-i-n-e-s-enter-the-p-p-r-adjacency -->

The SystemRanker adjacency must include the rank-edge predicates CONTAINS, PARTICIPATES_IN, HANDLES and DEFINES with the documented direction and weight ontology (RankEdgeKind, never reference_kind doubling): CONTAINS Component/Subsystem/Service -> File/Symbol at 0.4 plus the reverse member -> container ranking transition at 0.8 (explicitly a ranking transition, never a Reality Graph relationship); PARTICIPATES_IN Symbol -> Flow at 0.6 plus Flow -> participant at 1.2; HANDLES Symbol -> Route at 1.5; DEFINES Symbol -> Schema at 1.2. A flow with participants must receive a non-zero vector (previously disconnected); a component with members must spread importance both ways; HANDLES/DEFINES edges must feed PPR.
