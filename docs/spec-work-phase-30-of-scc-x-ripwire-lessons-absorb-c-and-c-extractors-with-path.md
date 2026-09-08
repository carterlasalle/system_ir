# Phase 30 of SCC x Ripwire lessons: absorb C and C++ extractors with path…

<!-- trace:v1 id=doc.phase-30-of-scc-x-ripwire-lessons-absorb-c-and-c-extractors-with-path -->

<!-- trace:exempt reason=document-structure -->
## Goal

Phase 30 of SCC x Ripwire lessons: absorb C and C++ extractors with path-precise quote-include Step-A. Promote C and C++ from IndexSearch to SemanticDeep with fixtures: tree-sitter extract functions, C++ class methods, calls, and #include. Quote includes (#include "foo.h") resolve by lexical join with the includer directory only — exact file hit pins, miss is Unresolved, never basename-guess other/foo.h. Angle includes (#include <stdio.h>) stay External even if a same-named header exists in-repo. Direct includes only — not Ripwire transitive include closure, not compile_commands -I, not C++ CHA, not fn-pointer address-of/escape, not ObjC, not .scm rewrite. Gate on C/C++ extensions so they never steal .py/.ts. Keep four context levels, 10 MCP tools, and the production ranker.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-implement-phase-30-of-scc-x-ripwire-lessons-absorb-c-and-c-extracto — Implement Phase 30 of SCC x Ripwire lessons: absorb C and C++ extracto…

<!-- trace:v1 id=REQ-implement-phase-30-of-scc-x-ripwire-lessons-absorb-c-and-c-extracto type=requirement work=WORK-phase-30-of-scc-x-ripwire-lessons-absorb-c-and-c-extractors-with-path -->

Phase 30 of SCC x Ripwire lessons: absorb C and C++ extractors with path-precise quote-include Step-A. Promote C and C++ from IndexSearch to SemanticDeep with fixtures: tree-sitter extract functions, C++ class methods, calls, and #include. Quote includes (#include "foo.h") resolve by lexical join with the includer directory only — exact file hit pins, miss is Unresolved, never basename-guess other/foo.h. Angle includes (#include <stdio.h>) stay External even if a same-named header exists in-repo. Direct includes only — not Ripwire transitive include closure, not compile_commands -I, not C++ CHA, not fn-pointer address-of/escape, not ObjC, not .scm rewrite. Gate on C/C++ extensions so they never steal .py/.ts. Keep four context levels, 10 MCP tools, and the production ranker.
