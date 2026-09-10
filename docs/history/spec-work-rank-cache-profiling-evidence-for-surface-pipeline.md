# Rank-cache profiling evidence for surface pipeline

<!-- trace:v1 id=doc.rank-cache-profiling-evidence-for-surface-pipeline -->

<!-- trace:exempt reason=document-structure -->
## Goal

Measure global-PPR rebuild cost vs the per-ModelEpoch GlobalRankCache seam so the cache-wiring decision for plain `scc surface` is empirical (spec Part F: profile before optimizing; if negligible, document and leave).

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-cache-wiring-decision-backed-by-measurement — Cache-wiring decision backed by measurement

<!-- trace:v1 id=REQ-cache-wiring-decision-backed-by-measurement type=requirement work=WORK-rank-cache-profiling-evidence-for-surface-pipeline -->

The decision to keep or extend the GlobalRankCache seam beyond startup MUST be justified by a recorded timing comparison of cold rebuild vs cached render on a real fixture, with identical rendered token counts across the seam.
