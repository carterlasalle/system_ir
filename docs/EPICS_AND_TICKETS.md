# Epics and Implementation Tickets

Priority: P0 core, P1 next, P2 later. Effort = relative points.

## EPIC-001 Benchmark Harness
- SCC-001 [P0,5] Benchmark task schema
- SCC-002 [P0,8] Agent-run recorder
- SCC-003 [P0,8] Curate 10 benchmark repos
- SCC-004 [P0,5] Plain-agent baseline
- SCC-005 [P0,5] Serena/graph-tool baselines

## EPIC-010 Schema and Store
- SCC-010 [P0,8] System IR Rust types
- SCC-011 [P0,5] JSON Schema validation
- SCC-012 [P0,8] SQLite schema
- SCC-013 [P0,5] Snapshot/version model
- SCC-014 [P0,5] Evidence/provenance
- SCC-015 [P0,5] Migrations
- SCC-016 [P0,5] JSON/JSONL import-export

## EPIC-020 Indexer
- SCC-020 [P0,5] Repo/Git snapshot resolver
- SCC-021 [P0,5] File hash pipeline
- SCC-022 [P0,8] Extractor framework
- SCC-023 [P0,13] TypeScript extractor
- SCC-024 [P0,13] Python extractor
- SCC-025 [P0,8] Import normalization
- SCC-026 [P0,13] Candidate calls
- SCC-027 [P0,8] Route extractor
- SCC-028 [P0,5] Test classifier
- SCC-029 [P0,8] Incremental invalidation
- SCC-030 [P0,5] File watcher
- SCC-031 [P0,8] Full-vs-incremental equivalence

## EPIC-040 Component Compiler
- SCC-040 [P0,8] Package/directory signals
- SCC-041 [P0,8] Call/import cohesion
- SCC-042 [P0,5] Route ownership
- SCC-043 [P0,5] Explicit overrides
- SCC-044 [P0,8] Stable identity across moves
- SCC-045 [P0,8] Component evidence/confidence
- SCC-046 [P1,8] Git co-change
- SCC-047 [P1,8] Deployment boundary

## EPIC-050 System Atlas
- SCC-050 [P0,8] Architecture compiler
- SCC-051 [P0,13] Sequence compiler
- SCC-052 [P0,13] Data Flow compiler
- SCC-053 [P0,8] Path abstraction
- SCC-054 [P0,5] Step-to-symbol drilldown
- SCC-055 [P1,13] Workflow compiler
- SCC-056 [P1,13] Lifecycle compiler
- SCC-057 [P1,8] Retry/fallback
- SCC-058 [P1,8] Failure branches

## EPIC-060 Context Compiler
- SCC-060 [P0,8] Candidate retrieval
- SCC-061 [P0,8] Lexical ranking
- SCC-062 [P0,8] Graph ranking
- SCC-063 [P0,8] Flow/ownership/contract ranking
- SCC-064 [P0,13] Task pack
- SCC-065 [P0,8] Startup capsule
- SCC-066 [P0,8] Hard token budget
- SCC-067 [P0,8] Dedup/abstraction
- SCC-068 [P0,5] Evidence map
- SCC-069 [P0,8] Cache/invalidation
- SCC-070 [P0,13] Precision/recall evaluation
- SCC-071 [P1,8] Embedding ranker
- SCC-072 [P1,8] Constrained LLM compression

## EPIC-080 Agent Interfaces
- SCC-080 [P0,8] MCP server
- SCC-081 [P0,5] system_overview
- SCC-082 [P0,8] task_context
- SCC-083 [P0,5] component_context
- SCC-084 [P0,5] flow_context
- SCC-085 [P0,8] impact_context
- SCC-086 [P0,5] verify_context
- SCC-087 [P0,8] CLI
- SCC-088 [P1,8] HTTP API
- SCC-089 [P1,8] TS SDK
- SCC-090 [P1,8] Python SDK

## EPIC-100 Claude Code
- SCC-100 [P0,5] Installer
- SCC-101 [P0,8] SessionStart
- SCC-102 [P0,8] UserPromptSubmit injection
- SCC-103 [P0,8] Changed-file refresh
- SCC-104 [P0,5] PreCompact checkpoint
- SCC-105 [P0,5] Rehydration
- SCC-106 [P0,8] Integration harness
- SCC-107 [P1,5] Subagent context policy

## EPIC-120 Semantic Resolution
- SCC-120 [P1,8] LSP adapter contract
- SCC-121 [P1,13] TS LSP resolver
- SCC-122 [P1,13] Python LSP resolver
- SCC-123 [P1,8] SCIP importer
- SCC-124 [P1,8] Serena adapter
- SCC-125 [P1,8] Resolution conflict model
- SCC-126 [P1,8] Differential benchmark

## EPIC-140 Data/Events/Infra
- SCC-140 [P1,13] SQL/ORM ownership
- SCC-141 [P1,8] Redis/cache
- SCC-142 [P1,13] Queue producer/consumer
- SCC-143 [P1,8] Docker Compose
- SCC-144 [P1,8] Kubernetes
- SCC-145 [P1,8] Terraform
- SCC-146 [P1,8] GitHub Actions
- SCC-147 [P1,8] Config refs
- SCC-148 [P1,8] Trust boundaries

## EPIC-160 Runtime
- SCC-160 [P1,8] OTel ingest
- SCC-161 [P1,8] Runtime edge aggregation
- SCC-162 [P1,8] Latency/error aggregate
- SCC-163 [P1,13] Static-vs-observed
- SCC-164 [P1,8] Replay tests

## EPIC-180 Intent and Drift
- SCC-180 [P1,8] intent.yaml schema
- SCC-181 [P1,8] Declared components/ownership
- SCC-182 [P1,13] Intent-reality diff
- SCC-183 [P1,8] Ownership drift
- SCC-184 [P1,8] Contract drift
- SCC-185 [P1,8] Flow drift
- SCC-186 [P1,8] CI policies

## EPIC-200 External Adapters
- SCC-200 [P1,8] GitNexus
- SCC-201 [P1,8] Narsil CCG
- SCC-202 [P2,8] CBM
- SCC-203 [P1,5] Beads
- SCC-204 [P2,8] Hindsight
- SCC-205 [P2,5] Context7
- SCC-206 [P2,5] RTK policy hints

## EPIC-220 Security
- SCC-220 [P0,8] Secret redaction
- SCC-221 [P0,8] Path/symlink sandbox
- SCC-222 [P0,8] Untrusted-text labeling
- SCC-223 [P0,5] Local-only default
- SCC-224 [P1,8] Adapter capability manifest
- SCC-225 [P1,8] Adapter sandbox
- SCC-226 [P1,5] SBOM/release checks

## EPIC-240 Performance
- SCC-240 [P0,5] Index telemetry
- SCC-241 [P0,8] 50k/250k benchmarks
- SCC-242 [P0,8] Incremental optimization
- SCC-243 [P1,8] Context cache
- SCC-244 [P1,8] Memory profiling
