# Wave 15.1: one authoritative Surface service (build_surface)

<!-- trace:v1 id=doc.wave-15-1-one-authoritative-surface-service-build-surface -->

<!-- trace:exempt reason=document-structure -->
## Goal

Unify every surface consumer (production, CLI, MCP, plugin, benchmark ablations, startup) on ONE authoritative surface pipeline: build_surface(Global|Task) with token-aware quotas, soft budget + hard max with structural compression, and build_surface_staged ablation toggles; route startup.rs through build_surface and delete its duplicate task/PPR selection implementations.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-unified-surface-service — Unified surface service

<!-- trace:v1 id=REQ-unified-surface-service type=requirement work=WORK-wave-15-1-one-authoritative-surface-service-build-surface -->

REQ-SCC-IR: The context compiler MUST expose one authoritative surface pipeline (build_surface) that all consumers route through; the pipeline MUST compile candidates, rank (global or task PPR), apply required coverage, MMR diversity, token-aware quotas, and a soft/hard token budget, then render deterministically; task mode MUST suppress already-visible unchanged entries via the context ledger; a staged variant MUST toggle pipeline stages for ablation benchmarks.
