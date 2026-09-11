# Authoring Requirements, Specs, ADRs, and Plans
<!-- trace:v1 id=doc.tracelayer.authoring -->

Traceability starts before code: name the requirement, the spec, and the
decisions first, then implement against them. Markers point at these
artifacts (`satisfies=REQ-…`, `implements=PLAN-…`); without them markers
are labels without meaning.

<!-- trace:v1 id=doc.tracelayer.authoring.requirements work=WORK-p0-remediation-passive-activation-remind-first-enforcement-branch-safe-identity-safe-bootstrap -->
### Requirements

- Author with intent: `trace task bootstrap --json` (or `--file` /
  `--stdin`) creates WORK + requirements + spec + plan + tasks in one
  atomic transaction. `--prompt` only writes DRAFT scaffolding.
- Mint single IDs any time: `trace new requirement --name "Frobnicator
  retries"`.
- Markers reference requirements via `satisfies=REQ-…`. A `satisfies`
  target that does not exist yet becomes a stub node — flesh it out, or
  TL002 blocks the merge.
- Changing a requirement flags downstream artifacts stale (prior evidence
  is historical). Re-verify before completion.

<!-- trace:v1 id=doc.tracelayer.authoring.specs-plans work=WORK-p0-remediation-passive-activation-remind-first-enforcement-branch-safe-identity-safe-bootstrap -->
### Specs and plans

- `trace plan suggest "<intent>"` scopes tiny/small/medium/large so the
  ceremony fits the change. Plans declare what they will produce
  (`expects=`); TL014 enforces each exists and links back.
- Requirement/ADR/plan edits mark dependents stale — review, don't ignore.
- `trace work mirror` / `reconcile` keeps native TASK state coherent.

<!-- trace:v1 id=doc.tracelayer.authoring.adrs-decisions work=WORK-p0-remediation-passive-activation-remind-first-enforcement-branch-safe-identity-safe-bootstrap -->
### ADRs and decisions

- Record durable decisions in `docs/adr/` (context, decision,
  alternatives, consequences). Link code via `addresses=` and superseded
  decisions via `supersedes=`.
- Open unknowns that block work: `trace question add "<text>" --work
  <WORK-ID>` (with `--blocks` for the TASK/REQ it gates). Answer before
  the blocked work lands; decisions close questions.
- No ceremony artifacts: one-line fake specs and empty ADRs fail review.
  If the change is too small for an ADR, say so in the marker's work
  item instead of inventing process.

<!-- trace:v1 id=doc.tracelayer.authoring.knowledge-facts work=WORK-p0-remediation-passive-activation-remind-first-enforcement-branch-safe-identity-safe-bootstrap -->
### Knowledge and facts

- Durable lessons: `trace knowledge-capture` into `docs/knowledge.md`;
- Retire wrong lessons: `trace knowledge-retire <id> --to SUPERSEDED|INVALIDATED|ARCHIVED` (supersede needs `--successor`); retired nodes stop governing.
  govern artifacts with knowledge edges so future edits surface them.
- Canonical values: `trace facts` lists them, `trace facts <id>` shows
  source + dependents, `trace facts --verify` fails on drift. Fix the
  source value, never the copies.
