# Benchmark infrastructure regression suite: ledger visibility + incremental freshness

<!-- trace:v1 id=doc.benchmark-infrastructure-regression-suite-ledger-visibility-incremental-freshness -->

<!-- trace:exempt reason=document-structure -->
## Goal

Add CI regression tests proving the Context Ledger records only model-visible entries (SCC mission 53) and that incremental path refresh serves fresh context after edits/deletions (54)

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-ledger-actual-visibility-regression — Ledger actual-visibility regression

<!-- trace:v1 id=REQ-ledger-actual-visibility-regression type=requirement work=WORK-benchmark-infrastructure-regression-suite-ledger-visibility-incremental-freshness -->

A test asserts the ledger records only ids whose entries the rendered artifact actually showed; budget-dropped entries are never recorded as visible.

### REQ-incremental-refresh-freshness-regression — Incremental refresh freshness regression

<!-- trace:v1 id=REQ-incremental-refresh-freshness-regression type=requirement work=WORK-benchmark-infrastructure-regression-suite-ledger-visibility-incremental-freshness -->

A test asserts context generated after an incremental single-path reindex reflects the edit, and that deleted symbols do not appear as current truth.
