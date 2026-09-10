# TraceLayer adapter for system_ir

<!-- trace:v1 id=doc.trace-layer-adapter-for-system-ir -->

<!-- trace:exempt reason=document-structure -->
## Goal

Implement TraceLayer as an evidence adapter for system_ir, parsing trace:v1 markers from source files and emitting structured facts in System IR (requirements, work items, implementation links, test verification links, decision/ADR links).

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-trace-layer-marker-parser — TraceLayer marker parser

<!-- trace:v1 id=REQ-trace-layer-marker-parser type=requirement work=WORK-trace-layer-adapter-for-system-ir -->

Parse trace:v1 markers from source files using the canonical grep-friendly format: trace:v1 id=<trace-id> key=value key=value ...

### REQ-system-i-r-entity-mapping — System IR entity mapping

<!-- trace:v1 id=REQ-system-i-r-entity-mapping type=requirement work=WORK-trace-layer-adapter-for-system-ir -->

Map parsed markers to correct System IR entities: type=requirement -> Requirement, work=WORK-XXX -> Work, satisfies=REQ-XXX -> implemented_by fact, verifies=REQ-XXX -> tested_by fact, type=decision addresses=REQ-XXX -> Decision

### REQ-adapter-registration — Adapter registration

<!-- trace:v1 id=REQ-adapter-registration type=requirement work=WORK-trace-layer-adapter-for-system-ir -->

Register TraceLayer adapter in the adapter manifest and integrate with the existing adapter system
