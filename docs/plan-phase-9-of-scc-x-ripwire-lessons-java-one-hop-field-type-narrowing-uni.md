# Java one-hop field-type narrowing (Ripwire Rule 2b analogue)

<!-- trace:v1 id=PLAN-phase-9-of-scc-x-ripwire-lessons-java-one-hop-field-type-narrowing-uni type=plan work=WORK-phase-9-of-scc-x-ripwire-lessons-java-one-hop-field-type-narrowing-uni implements=REQ-implement-phase-9-of-scc-x-ripwire-lessons-java-one-hop-field-type-na -->

<!-- trace:exempt reason=document-structure -->
## Steps

1. Java extractor: record class-scoped type binds from field declarations (`private Order repo`), `this.x = new Foo()`, and unique constructor/method params assigned to `this.x`. Persist `type_binds` on `ExtractedFile` (today the Java `into_extracted` path drops them).
2. Reuse the existing resolver: `FieldOfThis` with exactly one field hop pins to `{Type}.method` when unique and the method exists. Tombstone and longer chains stay unresolved. No same-name spray. No C++ unprefixed field lookup.
3. Tests: extract binds, extract-then-resolve pin, tombstone, longer-chain honesty, index CALLS.
4. Do not change production ranking, MCP tool count, context levels, or REQ-resolution-honesty-gauges.
