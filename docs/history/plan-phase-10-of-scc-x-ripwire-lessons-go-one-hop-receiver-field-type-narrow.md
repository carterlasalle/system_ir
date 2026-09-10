# Go one-hop receiver-field type narrowing (Ripwire Rule 2b analogue)

<!-- trace:v1 id=PLAN-phase-10-of-scc-x-ripwire-lessons-go-one-hop-receiver-field-type-narrow type=plan work=WORK-phase-10-of-scc-x-ripwire-lessons-go-one-hop-receiver-field-type-narrow implements=REQ-implement-phase-10-of-scc-x-ripwire-lessons-go-one-hop-receiver-field -->

<!-- trace:exempt reason=document-structure -->
## Steps

1. Go extractor: record struct-scoped type binds from field declarations (`owned *Order`) and method-scope binds from named receivers (`func (s *Svc) Run`). Persist `type_binds` on `ExtractedFile` (today `into_extracted` uses struct-update default and drops them).
2. Resolver: `FieldOfVariable` with exactly one field hop pins to `{Type}.method` only when the unique type of the root variable equals the enclosing type, the field type is unique, and the method exists. Tombstone and longer chains stay unresolved. Import chains (`db.users.Find`) stay unresolved.
3. Tests: extract binds, extract-then-resolve pin, tombstone, longer-chain honesty, index CALLS.
4. Do not change production ranking, MCP tool count, context levels, or REQ-resolution-honesty-gauges. Do not add C++ unprefixed field lookup.
