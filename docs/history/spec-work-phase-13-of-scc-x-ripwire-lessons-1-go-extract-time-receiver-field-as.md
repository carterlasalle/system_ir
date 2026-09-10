# Phase 13 of SCC x Ripwire lessons: (1) Go extract-time receiver-field as…

<!-- trace:v1 id=doc.phase-13-of-scc-x-ripwire-lessons-1-go-extract-time-receiver-field-as -->

<!-- trace:exempt reason=document-structure -->
## Goal

Phase 13 of SCC x Ripwire lessons: (1) Go extract-time receiver-field assignment tombstone: s.owned = &Invoice{} adds a class-scoped field type bind so a conflicting declared type Order plus assignment Invoice tombstones and s.owned.Process() stays unresolved; same-type assignment still pins. (2) Task Context stamps scc:// content handles on a compact FETCH section so agents can lazy-fetch exact source; stale handles must not be guessed. (3) scc bench loop --explore pack-consumer prefers FETCH handles and skips refused stale handles instead of reading the path. Do not change production ranking, MCP tool count, context levels, or REQ-resolution-honesty-gauges. Do not add C++ extractors or symbol-addressed edits.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-implement-phase-13-of-scc-x-ripwire-lessons-1-go-extract-time-recei — Implement Phase 13 of SCC x Ripwire lessons: (1) Go extract-time recei…

<!-- trace:v1 id=REQ-implement-phase-13-of-scc-x-ripwire-lessons-1-go-extract-time-recei type=requirement work=WORK-phase-13-of-scc-x-ripwire-lessons-1-go-extract-time-receiver-field-as -->

Phase 13 of SCC x Ripwire lessons: (1) Go extract-time receiver-field assignment tombstone: s.owned = &Invoice{} adds a class-scoped field type bind so a conflicting declared type Order plus assignment Invoice tombstones and s.owned.Process() stays unresolved; same-type assignment still pins. (2) Task Context stamps scc:// content handles on a compact FETCH section so agents can lazy-fetch exact source; stale handles must not be guessed. (3) scc bench loop --explore pack-consumer prefers FETCH handles and skips refused stale handles instead of reading the path. Do not change production ranking, MCP tool count, context levels, or REQ-resolution-honesty-gauges. Do not add C++ extractors or symbol-addressed edits.
