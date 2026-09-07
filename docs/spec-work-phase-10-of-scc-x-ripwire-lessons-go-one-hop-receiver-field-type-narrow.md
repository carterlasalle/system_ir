# Phase 10 of SCC x Ripwire lessons: Go one-hop receiver-field type narrow…

<!-- trace:v1 id=doc.phase-10-of-scc-x-ripwire-lessons-go-one-hop-receiver-field-type-narrow -->

<!-- trace:exempt reason=document-structure -->
## Goal

Phase 10 of SCC x Ripwire lessons: Go one-hop receiver-field type narrowing. Unique Go struct field types pin s.field.method() when s is a method receiver whose type is the enclosing struct. Extract TypeBinds from struct field declarations (owned *Order) and method receivers (func (s *Svc) Run). Resolver: FieldOfVariable with exactly three path segments pins to Type.m only when the unique type of the root variable equals the enclosing type and the field type is unique and that method exists. Two types tombstone. Longer chains stay unresolved. Do not pin import chains like db.users.Find. Native provenance stays EXTRACTED. Binds extract-time only. Do not change production ranking, MCP tool count, context levels, or REQ-resolution-honesty-gauges. Do not add C++ unprefixed field lookup.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-implement-phase-10-of-scc-x-ripwire-lessons-go-one-hop-receiver-field — Implement Phase 10 of SCC x Ripwire lessons: Go one-hop receiver-field…

<!-- trace:v1 id=REQ-implement-phase-10-of-scc-x-ripwire-lessons-go-one-hop-receiver-field type=requirement work=WORK-phase-10-of-scc-x-ripwire-lessons-go-one-hop-receiver-field-type-narrow -->

Phase 10 of SCC x Ripwire lessons: Go one-hop receiver-field type narrowing. Unique Go struct field types pin s.field.method() when s is a method receiver whose type is the enclosing struct. Extract TypeBinds from struct field declarations (owned *Order) and method receivers (func (s *Svc) Run). Resolver: FieldOfVariable with exactly three path segments pins to Type.m only when the unique type of the root variable equals the enclosing type and the field type is unique and that method exists. Two types tombstone. Longer chains stay unresolved. Do not pin import chains like db.users.Find. Native provenance stays EXTRACTED. Binds extract-time only. Do not change production ranking, MCP tool count, context levels, or REQ-resolution-honesty-gauges. Do not add C++ unprefixed field lookup.
