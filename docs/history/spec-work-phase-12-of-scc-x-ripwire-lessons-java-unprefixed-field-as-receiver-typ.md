# Phase 12 of SCC x Ripwire lessons: Java unprefixed field-as-receiver typ…

<!-- trace:v1 id=doc.phase-12-of-scc-x-ripwire-lessons-java-unprefixed-field-as-receiver-typ -->

<!-- trace:exempt reason=document-structure -->
## Goal

Phase 12 of SCC x Ripwire lessons: Java unprefixed field-as-receiver type narrowing (Ripwire Rule 2b analogue). Inside a Java method, repo.save() (NamedVariable, exactly two path segments, no this. prefix) pins to the unique field type of the enclosing class when that method exists. A parameter or local of the same name shadows the field and vetoes the narrow even if tombstoned. Two field types tombstone. Longer chains stay unresolved. EXTRACTED provenance. Binds extract-time only. Also P26: Store::open refuses truncated or corrupt scc.db instead of fabricating an empty index. CLI context structural --files with a stale scc:// handle prints HANDLE REFUSED and does not guess a target. Do not change production ranking, MCP tool count, context levels, or REQ-resolution-honesty-gauges. Do not add C++ extractors.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-implement-phase-12-of-scc-x-ripwire-lessons-java-unprefixed-field-as — Implement Phase 12 of SCC x Ripwire lessons: Java unprefixed field-as-…

<!-- trace:v1 id=REQ-implement-phase-12-of-scc-x-ripwire-lessons-java-unprefixed-field-as type=requirement work=WORK-phase-12-of-scc-x-ripwire-lessons-java-unprefixed-field-as-receiver-typ -->

Phase 12 of SCC x Ripwire lessons: Java unprefixed field-as-receiver type narrowing (Ripwire Rule 2b analogue). Inside a Java method, repo.save() (NamedVariable, exactly two path segments, no this. prefix) pins to the unique field type of the enclosing class when that method exists. A parameter or local of the same name shadows the field and vetoes the narrow even if tombstoned. Two field types tombstone. Longer chains stay unresolved. EXTRACTED provenance. Binds extract-time only. Also P26: Store::open refuses truncated or corrupt scc.db instead of fabricating an empty index. CLI context structural --files with a stale scc:// handle prints HANDLE REFUSED and does not guess a target. Do not change production ranking, MCP tool count, context levels, or REQ-resolution-honesty-gauges. Do not add C++ extractors.
