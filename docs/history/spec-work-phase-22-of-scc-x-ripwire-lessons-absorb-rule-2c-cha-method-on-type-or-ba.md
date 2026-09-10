# Phase 22 of SCC x Ripwire lessons: absorb Rule 2c CHA / methodOnTypeOrBa…

<!-- trace:v1 id=doc.phase-22-of-scc-x-ripwire-lessons-absorb-rule-2c-cha-method-on-type-or-ba -->

<!-- trace:exempt reason=document-structure -->
## Goal

Phase 22 of SCC x Ripwire lessons: absorb Rule 2c CHA / methodOnTypeOrBases. When Cls.m() (StaticType) or a unique typed receiver of type Cls does not define m, walk direct bases breadth-first (cap 16 visited names). Pin only when the unique in-repo class-like named Cls has heritage and the shallowest level has exactly one hitting base that uniquely defines m (IERS_B.open() → IERS.open). Two hitting bases at one level stay unresolved. Two same-named class-likes refuse heritage merge. Persist simple-ident class_bases on CLASS entity attributes (not FILE) so incremental≡cold. Python/Java/TS extract heritage. Do not pin modules. No Rule 1 self/super walk yet. Keep extract-time-only type binds, four context levels, 10 MCP tools, and the production ranker.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-implement-phase-22-of-scc-x-ripwire-lessons-absorb-rule-2c-cha-meth — Implement Phase 22 of SCC x Ripwire lessons: absorb Rule 2c CHA / meth…

<!-- trace:v1 id=REQ-implement-phase-22-of-scc-x-ripwire-lessons-absorb-rule-2c-cha-meth type=requirement work=WORK-phase-22-of-scc-x-ripwire-lessons-absorb-rule-2c-cha-method-on-type-or-ba -->

Phase 22 of SCC x Ripwire lessons: absorb Rule 2c CHA / methodOnTypeOrBases. When Cls.m() (StaticType) or a unique typed receiver of type Cls does not define m, walk direct bases breadth-first (cap 16 visited names). Pin only when the unique in-repo class-like named Cls has heritage and the shallowest level has exactly one hitting base that uniquely defines m (IERS_B.open() → IERS.open). Two hitting bases at one level stay unresolved. Two same-named class-likes refuse heritage merge. Persist simple-ident class_bases on CLASS entity attributes (not FILE) so incremental≡cold. Python/Java/TS extract heritage. Do not pin modules. No Rule 1 self/super walk yet. Keep extract-time-only type binds, four context levels, 10 MCP tools, and the production ranker.
