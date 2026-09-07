# Phase 25 of SCC x Ripwire lessons: absorb extract-time function-alias bi…

<!-- trace:v1 id=doc.phase-25-of-scc-x-ripwire-lessons-absorb-extract-time-function-alias-bi -->

<!-- trace:exempt reason=document-structure -->
## Goal

Phase 25 of SCC x Ripwire lessons: absorb extract-time function-alias binds for bare f(). Ripwire fnPtrBindingTarget is C/C++/ObjC only and fires before Rule 1: unique var=function ident pins f() to that function, never falls back to a same-named global, two targets/lambda/clobber tombstone. SCC analog for Python/TypeScript/Go/Rust: unique extract-time f = helper / const f = helper / f := helper / let f = helper pins RecvKind::None f() to the unique in-repo Function (local, then import, then unique-across-repo). Lambda/arrow/closure is an empty-target bind that blocks the name ladder. A later non-function assignment clobbers only when a fn bind already exists. Type-copy ident RHS is not a fn bind but clobbers a prior fn bind. File-scope empty-scope binds apply when the caller has no local bind; local vs file disagreement refuses. Do not pin Class constructor aliases. No C++ extractors. No Go embedding. Keep four context levels, 10 MCP tools, and the production ranker.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-implement-phase-25-of-scc-x-ripwire-lessons-absorb-extract-time-funct — Implement Phase 25 of SCC x Ripwire lessons: absorb extract-time funct…

<!-- trace:v1 id=REQ-implement-phase-25-of-scc-x-ripwire-lessons-absorb-extract-time-funct type=requirement work=WORK-phase-25-of-scc-x-ripwire-lessons-absorb-extract-time-function-alias-bi -->

Phase 25 of SCC x Ripwire lessons: absorb extract-time function-alias binds for bare f(). Ripwire fnPtrBindingTarget is C/C++/ObjC only and fires before Rule 1: unique var=function ident pins f() to that function, never falls back to a same-named global, two targets/lambda/clobber tombstone. SCC analog for Python/TypeScript/Go/Rust: unique extract-time f = helper / const f = helper / f := helper / let f = helper pins RecvKind::None f() to the unique in-repo Function (local, then import, then unique-across-repo). Lambda/arrow/closure is an empty-target bind that blocks the name ladder. A later non-function assignment clobbers only when a fn bind already exists. Type-copy ident RHS is not a fn bind but clobbers a prior fn bind. File-scope empty-scope binds apply when the caller has no local bind; local vs file disagreement refuses. Do not pin Class constructor aliases. No C++ extractors. No Go embedding. Keep four context levels, 10 MCP tools, and the production ranker.
