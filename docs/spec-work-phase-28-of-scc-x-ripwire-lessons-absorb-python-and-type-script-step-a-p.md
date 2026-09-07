# Phase 28 of SCC x Ripwire lessons: absorb Python and TypeScript Step-A p…

<!-- trace:v1 id=doc.phase-28-of-scc-x-ripwire-lessons-absorb-python-and-type-script-step-a-p -->

<!-- trace:exempt reason=document-structure -->
## Goal

Phase 28 of SCC x Ripwire lessons: absorb Python and TypeScript Step-A path-precise import resolution (resolvePythonImport / resolveTsImport). Unique-or-degrade: Python import a / import pkg.mod / from pkg.mod import Z / from .rel import Z resolve to exactly one .py or /__init__.py or contribute nothing. Never basename-guess. Never pick when both pkg.py and pkg/__init__.py exist. Relative leading-dot imports probe includer-dir only. Absolute imports probe file-relative, repo-root, and SCC unique source-root fallback (src/svc/lib/app/services/service/packages) as one unique-or-degrade set: only src/util.py still pins, util.py and src/util.py degrade. Zero absolute hits stay External; two-plus in-repo hits stay Unresolved (not External, not a guess). TypeScript relative ./x and ../a/b probe exact then a fixed extension list then index files relative-to-includer; unique hit pins, 0 or two-plus degrade to Unresolved. Bare TS specifiers (react, lodash) stay External and must not match in-repo files by basename. Language-gate on caller extension (.py vs .ts/.tsx/.js/.jsx/.mjs/.cjs) so Python does not steal TS files and TS does not steal Python files. No tsconfig multi-root aliases, no go.mod, no C++ extractors, no Java namespaces, no transitive include closure. Keep four context levels, 10 MCP tools, and the production ranker.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-implement-phase-28-of-scc-x-ripwire-lessons-absorb-python-and-type-scr — Implement Phase 28 of SCC x Ripwire lessons: absorb Python and TypeScr…

<!-- trace:v1 id=REQ-implement-phase-28-of-scc-x-ripwire-lessons-absorb-python-and-type-scr type=requirement work=WORK-phase-28-of-scc-x-ripwire-lessons-absorb-python-and-type-script-step-a-p -->

Phase 28 of SCC x Ripwire lessons: absorb Python and TypeScript Step-A path-precise import resolution (resolvePythonImport / resolveTsImport). Unique-or-degrade: Python import a / import pkg.mod / from pkg.mod import Z / from .rel import Z resolve to exactly one .py or /__init__.py or contribute nothing. Never basename-guess. Never pick when both pkg.py and pkg/__init__.py exist. Relative leading-dot imports probe includer-dir only. Absolute imports probe file-relative, repo-root, and SCC unique source-root fallback (src/svc/lib/app/services/service/packages) as one unique-or-degrade set: only src/util.py still pins, util.py and src/util.py degrade. Zero absolute hits stay External; two-plus in-repo hits stay Unresolved (not External, not a guess). TypeScript relative ./x and ../a/b probe exact then a fixed extension list then index files relative-to-includer; unique hit pins, 0 or two-plus degrade to Unresolved. Bare TS specifiers (react, lodash) stay External and must not match in-repo files by basename. Language-gate on caller extension (.py vs .ts/.tsx/.js/.jsx/.mjs/.cjs) so Python does not steal TS files and TS does not steal Python files. No tsconfig multi-root aliases, no go.mod, no C++ extractors, no Java namespaces, no transitive include closure. Keep four context levels, 10 MCP tools, and the production ranker.
