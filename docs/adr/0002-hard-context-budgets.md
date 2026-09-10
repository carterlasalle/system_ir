<!-- trace:v1 id=doc.adr-0002 type=document work=WORK-SI-MMMJA4G6 -->
# ADR 0002: Hard token budgets on all agent-facing packs

<!-- trace:exempt reason=document-structure -->
## Context

Startup and detail packs could exceed their budgets (`BUDGET OVERFLOW`
escape hatch; `usize::MAX` for component/flow/impact/verify), contradicting
the documented "hard token budgets" contract.

<!-- trace:exempt reason=document-structure -->
## Decision

`rendered_tokens <= hard_max` by measurement on every agent-facing pack:
soft-drop (priority order), then halve bodies, drop whole sections
(recorded), line-truncate the last body. Startup degrades structurally
(atlas essentials + skeleton + receipt) instead of escaping. New
`detail_tokens` (6000) budgets the detail packs. No human `--full`
unlimited mode yet (humans share bounded packs; deferred, not rejected).

<!-- trace:exempt reason=document-structure -->
## Alternatives considered

- Soft budget + `exceeded` flag (status quo): rejected — contradicts the
  product contract; an over-budget startup breaks agent context windows.
- Mid-record byte truncation: rejected — corrupts structured records;
  line granularity + explicit markers instead.
- Priority-9+ never dropped (old test contract): rejected — at tiny
  budgets something must give; recording preserves honesty.

<!-- trace:exempt reason=document-structure -->
## Consequences

- Two golden tests redefined "tight budget" as fit + recorded cuts.
- Estimator (`chars/4`) is load-bearing: caps are measured, never promised.
