# Phase 8 of SCC x Ripwire lessons: one-hop field-type narrowing

<!-- trace:v1 id=doc.phase-8-of-scc-x-ripwire-lessons-one-hop-field-type-narrowing-unique-c type=document work=WORK-phase-8-of-scc-x-ripwire-lessons-one-hop-field-type-narrowing-unique-c -->

<!-- trace:exempt reason=document-structure -->
## Goal

Absorb Ripwire Rule 2b as an SCC-native field-type pin for languages that write `self.field.method()` / `this.field.method()`. Do not cargo-cult C++ unprefixed field lookup, do not resolve longer chains, and do not replace SCC's ontology.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-implement-phase-8-of-scc-x-ripwire-lessons-one-hop-field-type-narrowi — Implement Phase 8 of SCC x Ripwire lessons: one-hop field-type narrowi…

<!-- trace:v1 id=REQ-implement-phase-8-of-scc-x-ripwire-lessons-one-hop-field-type-narrowi type=requirement work=WORK-phase-8-of-scc-x-ripwire-lessons-one-hop-field-type-narrowing-unique-c -->

Unique class field types pin exactly one hop. Extractors record field type binds from Python `self.x = Order()`, class-level `x: Order`, and `self.x = repo` when `repo` has a unique param/local type in that method; TypeScript class fields (`x: Order`) and `this.x = new Foo()`. Scope is the enclosing class name. A field bound to two distinct types is tombstoned and must not narrow.

Resolver: `self.x.m()` (`FieldOfSelf`) and `this.x.m()` (`FieldOfThis`) with exactly three path segments pin to `{Type}.m` when that method is a real definition on the local class or the imported type. Same-name methods on other classes must not receive the edge. Zero or many remaining candidates stay unresolved. Longer chains (`self.x.y.m()`) stay unresolved even when `x` has a unique type. Native provenance stays EXTRACTED. Binds are extract-time only (not FILE attributes).

Do not change production ranking, do not add a fifth context level, do not add MCP tools, do not rewrite REQ-resolution-honesty-gauges.
