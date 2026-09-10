<!-- trace:v1 id=doc.adr-0001 type=document work=WORK-SI-MMMJA4G6 -->
# ADR 0001: Production-scope default System Atlas

<!-- trace:exempt reason=document-structure -->
## Context

Repository-role labels (`[fixture]`, `[benchmark]`) on components did not stop
fixture routes, benchmark DB writers, fixture topics, and benchmark imports
from compiling into global entrypoints, contracts, state authority, flows,
and trust boundaries. Labels are presentation; compilation scope is semantics.

<!-- trace:exempt reason=document-structure -->
## Decision

`AtlasScope::Production` (default): architecture sections admit only
production-placed evidence (file-attribute and participant based;
unplaceable facts are kept, never dropped). Components and files still list
every role, labeled. `AtlasScope::Full` (`scc atlas --full`, MCP
`scope: full`) restores unfiltered compilation for archaeology. The scope
receipt (`coverage["scope"]`) counts scoped-out facts per section.

<!-- trace:exempt reason=document-structure -->
## Alternatives considered

- Labels only (status quo): rejected — proven insufficient by live
  self-index (fixture `GET /users` was a global entrypoint).
- Per-kind views (TestView/BenchmarkView): deferred — one production/full
  bit covers the demonstrated need; more views on evidence of need.
- Dropping non-production facts: rejected — structure must stay visible.

<!-- trace:exempt reason=document-structure -->
## Consequences

- `scc atlas` output on repos with fixtures/benchmarks shrinks honestly;
  benchmark tasks must pass `--full` / `scope: full` explicitly.
- Entity file attribution (`attributes.file`, handler files, pub/sub
  participants) is now load-bearing for scope: extractors must keep
  setting it (enforced by the atlas scope test).
- Classifier false positives are architecture bugs (see `com.example`
  namespace incident): the examples family matches only leading segments.
