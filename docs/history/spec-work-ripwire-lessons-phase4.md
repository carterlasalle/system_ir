# Ripwire lessons Phase 4 — honest language matrix

<!-- trace:v1 id=doc.spec-work-ripwire-lessons-phase4 type=document work=WORK-ripwire-lessons-phase4 -->

<!-- trace:exempt reason=document-structure -->
## Goal

Make the language-support matrix honest at scan time: every IndexSearch registry row is classified when its extension is seen. Classification is not support. Do not add tree-sitter extractors or claim C/C++/Ruby/… as SemanticDeep without fixtures.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-scan-classifies-registry-languages — Registry IndexSearch languages are classified

<!-- trace:v1 id=REQ-scan-classifies-registry-languages type=requirement work=WORK-ripwire-lessons-phase4 -->

Scan classification must cover every `LANGUAGE_REGISTRY` row whose tier is IndexSearch, using that row's extensions. Those languages stay `extractor: false` and must not enter the Python/TS/Go/Java/Rust extract path. `.scm` query-driven extractors remain a later evaluation; do not rewrite working semantic walkers in this phase.
