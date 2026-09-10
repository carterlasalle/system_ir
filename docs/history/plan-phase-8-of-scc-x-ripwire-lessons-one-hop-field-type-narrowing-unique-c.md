# One-hop field-type narrowing (Ripwire Rule 2b analogue)

<!-- trace:v1 id=PLAN-phase-8-of-scc-x-ripwire-lessons-one-hop-field-type-narrowing-unique-c type=plan work=WORK-phase-8-of-scc-x-ripwire-lessons-one-hop-field-type-narrowing-unique-c implements=REQ-implement-phase-8-of-scc-x-ripwire-lessons-one-hop-field-type-narrowi -->

<!-- trace:exempt reason=document-structure -->
## Steps

1. Python extractor: record class-scoped type binds from `self.x = Order()`, class `x: Order`, and unique param/local assigned to `self.x`.
2. TypeScript extractor: record class-scoped type binds from class fields and `this.x = new Foo()` / typed assignment.
3. Resolver: for `FieldOfSelf`/`FieldOfThis` with exactly one field hop, pin to `{Type}.method` when unique and the method exists. Tombstone and longer chains stay unresolved. No same-name spray.
4. Tests: extract binds, pin, tombstone, longer-chain honesty, import type, index CALLS.
5. Do not change production ranking, MCP tool count, context levels, or REQ-resolution-honesty-gauges.
