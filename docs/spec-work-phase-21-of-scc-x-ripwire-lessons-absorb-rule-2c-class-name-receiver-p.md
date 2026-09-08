# Phase 21 of SCC x Ripwire lessons: absorb Rule 2c class-name receiver. P…

<!-- trace:v1 id=doc.phase-21-of-scc-x-ripwire-lessons-absorb-rule-2c-class-name-receiver-p -->

<!-- trace:exempt reason=document-structure -->
## Goal

Phase 21 of SCC x Ripwire lessons: absorb Rule 2c class-name receiver. Pin Cls.m() to the unique in-repo Class/Struct/Interface/Type named Cls that uniquely defines m, after typed local/param binds (Rule 2). Any local or untyped param of that name vetoes the class-name pin. Two same-named class-like defs that both define m stay unresolved (no spray). Do not pin modules. StaticType consults binds; TypeQualified :: stays unchanged. Python and TypeScript untyped parameters are extract-time shadow binds with empty type_name so the veto is visible. Keep extract-time-only binds, four context levels, 10 MCP tools, and the production ranker.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-implement-phase-21-of-scc-x-ripwire-lessons-absorb-rule-2c-class-name — Implement Phase 21 of SCC x Ripwire lessons: absorb Rule 2c class-name…

<!-- trace:v1 id=REQ-implement-phase-21-of-scc-x-ripwire-lessons-absorb-rule-2c-class-name type=requirement work=WORK-phase-21-of-scc-x-ripwire-lessons-absorb-rule-2c-class-name-receiver-p -->

Phase 21 of SCC x Ripwire lessons: absorb Rule 2c class-name receiver. Pin Cls.m() to the unique in-repo Class/Struct/Interface/Type named Cls that uniquely defines m, after typed local/param binds (Rule 2). Any local or untyped param of that name vetoes the class-name pin. Two same-named class-like defs that both define m stay unresolved (no spray). Do not pin modules. StaticType consults binds; TypeQualified :: stays unchanged. Python and TypeScript untyped parameters are extract-time shadow binds with empty type_name so the veto is visible. Keep extract-time-only binds, four context levels, 10 MCP tools, and the production ranker.
