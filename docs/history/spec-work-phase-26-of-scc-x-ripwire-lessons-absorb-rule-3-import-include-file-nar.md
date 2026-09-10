# Phase 26 of SCC x Ripwire lessons: absorb Rule 3 import/include-file nar…

<!-- trace:v1 id=doc.phase-26-of-scc-x-ripwire-lessons-absorb-rule-3-import-include-file-nar -->

<!-- trace:exempt reason=document-structure -->
## Goal

Phase 26 of SCC x Ripwire lessons: absorb Rule 3 import/include-file narrowing. Ripwire rule3IncludeFile is language-agnostic on the path-precise file-to-file import/include graph: it fires only when the caller has at least one resolved import, no same-name candidate lives in the caller file (same-file is the local ladder), and exactly one imported file holds a same-name callable. Zero or two-plus imported defining files stay unresolved. Never invent a target. Never basename-guess. SCC analog uses existing IMPORTS and resolve_import Internal files (not C++ extractors, not Go go.mod, not Java namespaces, not transitive C include closure). Bare helper() with two defs: unique imported defining file pins; both imported stay unresolved; neither imported stays unresolved; same-file def wins and Rule 3 must not override. Also pin when the unique imported file is the only in-repo def (SCC has no unique-across-repo bare-name ladder). Function-alias unique_function_id uses the same Rule 3 after local and named-import, before unique-across-repo. Path-precise: from pkg.a import foo must pin pkg/a.py not other/a.py. No C++ extractors. No fifth context level. Keep four context levels, 10 MCP tools, and the production ranker.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-implement-phase-26-of-scc-x-ripwire-lessons-absorb-rule-3-import-incl — Implement Phase 26 of SCC x Ripwire lessons: absorb Rule 3 import/incl…

<!-- trace:v1 id=REQ-implement-phase-26-of-scc-x-ripwire-lessons-absorb-rule-3-import-incl type=requirement work=WORK-phase-26-of-scc-x-ripwire-lessons-absorb-rule-3-import-include-file-nar -->

Phase 26 of SCC x Ripwire lessons: absorb Rule 3 import/include-file narrowing. Ripwire rule3IncludeFile is language-agnostic on the path-precise file-to-file import/include graph: it fires only when the caller has at least one resolved import, no same-name candidate lives in the caller file (same-file is the local ladder), and exactly one imported file holds a same-name callable. Zero or two-plus imported defining files stay unresolved. Never invent a target. Never basename-guess. SCC analog uses existing IMPORTS and resolve_import Internal files (not C++ extractors, not Go go.mod, not Java namespaces, not transitive C include closure). Bare helper() with two defs: unique imported defining file pins; both imported stay unresolved; neither imported stays unresolved; same-file def wins and Rule 3 must not override. Also pin when the unique imported file is the only in-repo def (SCC has no unique-across-repo bare-name ladder). Function-alias unique_function_id uses the same Rule 3 after local and named-import, before unique-across-repo. Path-precise: from pkg.a import foo must pin pkg/a.py not other/a.py. No C++ extractors. No fifth context level. Keep four context levels, 10 MCP tools, and the production ranker.
