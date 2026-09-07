# Phase 11 of SCC x Ripwire lessons: Rust one-hop self-field type narrowin…

<!-- trace:v1 id=doc.phase-11-of-scc-x-ripwire-lessons-rust-one-hop-self-field-type-narrowin -->

<!-- trace:exempt reason=document-structure -->
## Goal

Phase 11 of SCC x Ripwire lessons: Rust one-hop self-field type narrowing. Unique Rust struct field types pin self.field.method() the same way Python already pins self.x.m(). Extract TypeBinds from struct field declarations (owned: Order, owned: Box<Order>, owned: &Order). Resolver already pins FieldOfSelf with exactly three path segments; the extractor must emit class-scoped binds. Two types tombstone. Longer chains stay unresolved. Native provenance stays EXTRACTED. Binds extract-time only, never FILE attributes. Do not change production ranking, MCP tool count, context levels, or REQ-resolution-honesty-gauges.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-implement-phase-11-of-scc-x-ripwire-lessons-rust-one-hop-self-field-t — Implement Phase 11 of SCC x Ripwire lessons: Rust one-hop self-field t…

<!-- trace:v1 id=REQ-implement-phase-11-of-scc-x-ripwire-lessons-rust-one-hop-self-field-t type=requirement work=WORK-phase-11-of-scc-x-ripwire-lessons-rust-one-hop-self-field-type-narrowin -->

Phase 11 of SCC x Ripwire lessons: Rust one-hop self-field type narrowing. Unique Rust struct field types pin self.field.method() the same way Python already pins self.x.m(). Extract TypeBinds from struct field declarations (owned: Order, owned: Box<Order>, owned: &Order). Resolver already pins FieldOfSelf with exactly three path segments; the extractor must emit class-scoped binds. Two types tombstone. Longer chains stay unresolved. Native provenance stays EXTRACTED. Binds extract-time only, never FILE attributes. Do not change production ranking, MCP tool count, context levels, or REQ-resolution-honesty-gauges.
