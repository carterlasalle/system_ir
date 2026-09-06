# Ripwire lessons Phase 3 — actionable Task Context

<!-- trace:v1 id=doc.spec-work-ripwire-lessons-phase3 type=document work=WORK-ripwire-lessons-phase3 -->

<!-- trace:exempt reason=document-structure -->
## Goal

Make Task Context actionable: every listed test has a reason, and pack budgets can use fixed percents with unused tokens rolling forward. Do not drop Atlas / Surface / Structural / Exact levels. Default packing stays adaptive-priority until a retrieval or agent-loop ablation prefers rollover.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-tests-to-run-reasons — tests_to_run names why each test is listed

<!-- trace:v1 id=REQ-tests-to-run-reasons type=requirement work=WORK-ripwire-lessons-phase3 -->

The task pack TESTS section is a `tests_to_run` list. Each row names the test, its file when known, and at least one reason from: `direct` (TESTED_BY an affected symbol), `import` (test file imports an affected file), `contract` (test covers a contract an affected symbol handles/implements), `state` (test reads/writes state owned by an affected component). Filename-only matching is not a reason. Rows are sorted by test id.

### REQ-budget-rollover — Fixed percents with unused rolling forward

<!-- trace:v1 id=REQ-budget-rollover type=requirement work=WORK-ripwire-lessons-phase3 -->

A budget allocator splits a token budget into buckets whose percents sum to 100 (Atlas 20, Surface 25, Structural-exact 30, Call-flow 10, Verify 10, Uncertainty 5). Unused quota in a bucket rolls forward to the next. Truncation of a bucket that exceeded its (rolled) quota is disclosed. Production task packs keep adaptive priority dropping until an ablation prefers this allocator; both strategies must remain callable.

### REQ-stack-locus-ingest — Stack/error FILE:LINE seeds Task Context

<!-- trace:v1 id=REQ-stack-locus-ingest type=requirement work=WORK-ripwire-lessons-phase3 -->

When the task goal is a stack trace or error message with extracted FILE:LINE loci, Task Context must treat matching indexed files as affected (suffix path match, never `extra.py` for `a.py`) and, when line ranges exist, the innermost enclosing symbol. A LOCUS section discloses the frames and whether they mapped. Unmapped frames are skipped, not fabricated. Production ranking is unchanged; this is pack seed selection.
