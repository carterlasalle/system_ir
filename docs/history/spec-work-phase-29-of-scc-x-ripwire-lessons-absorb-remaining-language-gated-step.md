# Phase 29 of SCC x Ripwire lessons: absorb remaining language-gated Step-…

<!-- trace:v1 id=doc.phase-29-of-scc-x-ripwire-lessons-absorb-remaining-language-gated-step -->

<!-- trace:exempt reason=document-structure -->
## Goal

Phase 29 of SCC x Ripwire lessons: absorb remaining language-gated Step-A honesty for Go and Java without cargo-culting go.mod or Java namespaces. Gate on caller extension so Go/Java never steal Python/TS files via the generic first-match resolver (import fmt must stay External even if fmt.py exists). Java type imports (import com.foo.Bar) unique-or-degrade to exactly one .java file by dots-to-slashes plus .java, including unique SCC Java source-root fallback src/main/java and src/test/java; star imports (com.foo.*) contribute no file (namespace, not a type); zero hits stay External; two-plus in-repo hits stay Unresolved. Go import paths unique-or-degrade to a package: exactly one of {path}.go or the non-test .go files directly in {path}/, plus unique source-root fallback; {path}.go and {path}/ both present degrade; two package dirs degrade; a unique package directory with several .go files expands to one Internal import per file so Rule 3 sees the whole package (not a guessed representative file); *_test.go is not a production package file; no go.mod parsing, no module-path suffix guessing, no C++ extractors. Keep four context levels, 10 MCP tools, and the production ranker.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-implement-phase-29-of-scc-x-ripwire-lessons-absorb-remaining-language — Implement Phase 29 of SCC x Ripwire lessons: absorb remaining language…

<!-- trace:v1 id=REQ-implement-phase-29-of-scc-x-ripwire-lessons-absorb-remaining-language type=requirement work=WORK-phase-29-of-scc-x-ripwire-lessons-absorb-remaining-language-gated-step -->

Phase 29 of SCC x Ripwire lessons: absorb remaining language-gated Step-A honesty for Go and Java without cargo-culting go.mod or Java namespaces. Gate on caller extension so Go/Java never steal Python/TS files via the generic first-match resolver (import fmt must stay External even if fmt.py exists). Java type imports (import com.foo.Bar) unique-or-degrade to exactly one .java file by dots-to-slashes plus .java, including unique SCC Java source-root fallback src/main/java and src/test/java; star imports (com.foo.*) contribute no file (namespace, not a type); zero hits stay External; two-plus in-repo hits stay Unresolved. Go import paths unique-or-degrade to a package: exactly one of {path}.go or the non-test .go files directly in {path}/, plus unique source-root fallback; {path}.go and {path}/ both present degrade; two package dirs degrade; a unique package directory with several .go files expands to one Internal import per file so Rule 3 sees the whole package (not a guessed representative file); *_test.go is not a production package file; no go.mod parsing, no module-path suffix guessing, no C++ extractors. Keep four context levels, 10 MCP tools, and the production ranker.
