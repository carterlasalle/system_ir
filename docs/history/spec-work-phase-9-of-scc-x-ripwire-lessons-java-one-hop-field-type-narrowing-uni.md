# Phase 9 of SCC x Ripwire lessons: Java one-hop field-type narrowing. Uni…

<!-- trace:v1 id=doc.phase-9-of-scc-x-ripwire-lessons-java-one-hop-field-type-narrowing-uni -->

<!-- trace:exempt reason=document-structure -->
## Goal

Phase 9 of SCC x Ripwire lessons: Java one-hop field-type narrowing. Unique Java class field types pin this.field.method() the same way Python and TypeScript already pin self/this field methods. The Java extractor must record class-scoped TypeBinds from field declarations (private Order repo), this.x = new Foo(), and this.x = param when the param has a unique type in that constructor or method. Two distinct types for the same field tombstone and must not narrow. Longer chains this.x.y.m() stay unresolved. Native provenance stays EXTRACTED. Binds are extract-time only, never FILE attributes. Do not change production ranking, MCP tool count, context levels, or REQ-resolution-honesty-gauges. Do not add C++ unprefixed field lookup.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-implement-phase-9-of-scc-x-ripwire-lessons-java-one-hop-field-type-na — Implement Phase 9 of SCC x Ripwire lessons: Java one-hop field-type na…

<!-- trace:v1 id=REQ-implement-phase-9-of-scc-x-ripwire-lessons-java-one-hop-field-type-na type=requirement work=WORK-phase-9-of-scc-x-ripwire-lessons-java-one-hop-field-type-narrowing-uni -->

Phase 9 of SCC x Ripwire lessons: Java one-hop field-type narrowing. Unique Java class field types pin this.field.method() the same way Python and TypeScript already pin self/this field methods. The Java extractor must record class-scoped TypeBinds from field declarations (private Order repo), this.x = new Foo(), and this.x = param when the param has a unique type in that constructor or method. Two distinct types for the same field tombstone and must not narrow. Longer chains this.x.y.m() stay unresolved. Native provenance stays EXTRACTED. Binds are extract-time only, never FILE attributes. Do not change production ranking, MCP tool count, context levels, or REQ-resolution-honesty-gauges. Do not add C++ unprefixed field lookup.
