# Relationship Guide

How to link traced artifacts with the right edge. The generated normative
table lives in `marker-protocol.md` (produced by `trace docs generate` from
the machine registry); this guide explains *when* to use each edge.

## The default path

```text
WORK -> addresses -> REQUIREMENT
IMPLEMENTATION -> work -> WORK
IMPLEMENTATION -> satisfies -> REQUIREMENT
TEST -> verifies -> REQUIREMENT
TEST -> exercises -> IMPLEMENTATION
EVIDENCE -> proves/observes -> TEST + IMPLEMENTATION + REVISION
```

Policy checks traversability along these paths. You declare the links that
only you can know; the engine derives structure and observation.

## Semantic edges

| Edge | Meaning | Typical source -> target | Example |
|---|---|---|---|
| `derived_from` | source was derived from target | requirement -> PRD; plan -> decision | `REQ-AUTH-017 derived_from=PRD-AUTH-002` |
| `addresses` | source is intended to address target | decision/work -> requirement | `WORK-AUTH-237 addresses=REQ-AUTH-017` |
| `satisfies` | source behavior fulfills target contract | implementation -> requirement | `impl.auth.refresh satisfies=REQ-AUTH-017` |
| `implements` | source realizes target plan/decision | implementation/config -> plan/decision | `impl.auth.refresh implements=ADR-0042,PLAN-AUTH-237/P3` |
| `verifies` | source test/evidence verifies target contract | test -> requirement | `test.auth.refresh-reuse verifies=REQ-AUTH-017` |
| `exercises` | source test intends to execute target implementation | test -> implementation | `test.auth.refresh-reuse exercises=impl.auth.refresh` |
| `documents` | source documents target | doc/runbook -> implementation/requirement | `doc.auth.rotation documents=REQ-AUTH-017` |
| `deploys` | source ops/config deploys target | operation -> implementation | `ops.auth.redis deploys=impl.auth.refresh` |
| `depends_on` | semantic dependency beyond trivial inferred calls | implementation/requirement -> artifact | `impl.billing.export depends_on=impl.billing.tax` |
| `supersedes` | source replaces target | decision/requirement -> same type | `ADR-0042 supersedes=ADR-0021` |
| `produces` | source activity produces target | plan/CI -> implementation/evidence | `PLAN-AUTH-237/P3 produces=impl.auth.refresh` |
| `consumes` | source relies on target artifact/data | implementation -> data/config | `impl.auth.refresh consumes=data.auth.session` |
| `blocks` | source must resolve before target progresses | work/requirement -> work/release | `WORK-AUTH-237 blocks=WORK-AUTH-238` |

## Derived edges — do not declare

Structural edges come from code analysis and must never be declared by hand:

```text
contains, calls, imports, inherits, references_symbol, reads, writes,
changed_by, owned_by
```

A function call does not automatically imply `depends_on`. If you need a
semantic dependency between implementations, use `depends_on` explicitly.

Observed edges come from evidence ingestion:

```text
executed, passed, failed, built_in, deployed_in, attested_by
```

Never write `executed` or `passed` into a marker — that is fabricated
evidence.

## Decision guide

Ask three questions before adding an edge:

1. **Is it intent?** The link describes what *should* hold, not what the code
   does → semantic edge (`satisfies`, `verifies`, `addresses`, ...).
2. **Is it fact about the code?** The link is derivable from the source →
   do nothing; the engine derives it.
3. **Is it fact about a run?** The link comes from test/CI evidence →
   ingest evidence; never hand-declare.

When the path already exists, prefer the shortest path that policy checks.
A requirement with a `verifies` test and a `satisfies` implementation needs
no `WORK` node; a changed behavior without requirement ancestry does.
