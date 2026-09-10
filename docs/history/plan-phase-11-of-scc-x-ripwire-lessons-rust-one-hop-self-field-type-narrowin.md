# Rust one-hop self-field type narrowing (Ripwire Rule 2b analogue)

<!-- trace:v1 id=PLAN-phase-11-of-scc-x-ripwire-lessons-rust-one-hop-self-field-type-narrowin type=plan work=WORK-phase-11-of-scc-x-ripwire-lessons-rust-one-hop-self-field-type-narrowin implements=REQ-implement-phase-11-of-scc-x-ripwire-lessons-rust-one-hop-self-field-t -->

<!-- trace:exempt reason=document-structure -->
## Steps

1. Rust extractor: record struct-scoped type binds from field declarations (`owned: Order`, `owned: Box<Order>`, `owned: &Order`). Persist `type_binds` on `ExtractedFile` (today `into_extracted` drops them).
2. Reuse the existing resolver: `FieldOfSelf` with exactly one field hop pins to `{Type}.method` when unique and the method exists. Tombstone and longer chains stay unresolved.
3. Tests: extract binds, extract-then-resolve pin, longer-chain honesty, index CALLS.
4. Do not change production ranking, MCP tool count, context levels, or REQ-resolution-honesty-gauges.
