# Phase 27 of SCC x Ripwire lessons: absorb Rust Step-A path-precise impor…

<!-- trace:v1 id=doc.phase-27-of-scc-x-ripwire-lessons-absorb-rust-step-a-path-precise-impor -->

<!-- trace:exempt reason=document-structure -->
## Goal

Phase 27 of SCC x Ripwire lessons: absorb Rust Step-A path-precise import resolution (resolveRustImport). Unique-or-degrade: use crate::a::b / super:: / self:: and body-less mod x; resolve to exactly one .rs or /mod.rs file or contribute nothing. Never basename-guess. Never pick when both a.rs and a/mod.rs exist. std:: and bare external crates stay External. crate:: without a locatable crate root (src/lib.rs or src/main.rs, first in sorted file list) degrades to Unresolved. Brace groups degrade. Gate on .rs callers so Python/TS/Go do not steal .rs files. No go.mod, no C++ extractors, no Java namespaces, no transitive include closure. Keep four context levels, 10 MCP tools, and the production ranker.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-implement-phase-27-of-scc-x-ripwire-lessons-absorb-rust-step-a-path-p — Implement Phase 27 of SCC x Ripwire lessons: absorb Rust Step-A path-p…

<!-- trace:v1 id=REQ-implement-phase-27-of-scc-x-ripwire-lessons-absorb-rust-step-a-path-p type=requirement work=WORK-phase-27-of-scc-x-ripwire-lessons-absorb-rust-step-a-path-precise-impor -->

Phase 27 of SCC x Ripwire lessons: absorb Rust Step-A path-precise import resolution (resolveRustImport). Unique-or-degrade: use crate::a::b / super:: / self:: and body-less mod x; resolve to exactly one .rs or /mod.rs file or contribute nothing. Never basename-guess. Never pick when both a.rs and a/mod.rs exist. std:: and bare external crates stay External. crate:: without a locatable crate root (src/lib.rs or src/main.rs, first in sorted file list) degrades to Unresolved. Brace groups degrade. Gate on .rs callers so Python/TS/Go do not steal .rs files. No go.mod, no C++ extractors, no Java namespaces, no transitive include closure. Keep four context levels, 10 MCP tools, and the production ranker.
