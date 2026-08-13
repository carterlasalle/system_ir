# Product Requirements Document
<!-- trace:v1 id=PRD-SCC-001 type=prd work=WORK-SCC-001 title="System Context Compiler product requirements" -->

## 1. Problem

Coding agents currently choose between two bad extremes:

- **Under-context:** they begin with a vague view of the repository and burn turns searching, grepping, following imports, and reconstructing architecture.
- **Over-context:** they receive large repository dumps, generated docs, or giant instruction files that consume context and reduce relevance.

Code-intelligence tools improve navigation but usually answer narrow questions such as “where is this symbol?” or “what calls this?” They do not reliably give the agent a ready-made model of **how the system works**.

Human-facing architecture tools solve a different problem: they make diagrams or prose for people. SCC must instead make an architecture model optimized for another machine.

## 2. Product vision

When an agent receives a task, SCC should immediately tell it:

- what subsystem it is touching;
- what that subsystem is responsible for;
- which flows include it;
- what is upstream and downstream;
- what data it owns and consumes;
- what contracts and invariants apply;
- what failure, retry, fallback, and lifecycle behavior exists;
- where the implementation and tests live;
- which claims are proven, observed, declared, inferred, or stale.

The agent can then use Serena/LSP or source tools for exact editing.

## 3. Primary users

### Coding-agent user
A developer running Claude Code, Codex, Hermes, OpenCode, or another agent against a non-trivial repository.

### Repository maintainer
A team that wants consistent architecture understanding across humans and multiple agents.

### Platform/AI infrastructure engineer
A user integrating SCC into an agent harness, CI system, or multi-agent environment.

## 4. Jobs to be done

1. Onboard an agent to an unfamiliar repository quickly.
2. Give a task-specific architecture slice before implementation starts.
3. Prevent missed downstream dependencies in cross-layer changes.
4. Surface system behavior, not merely file/symbol relationships.
5. Preserve architectural constraints across compaction and agent handoff.
6. Detect when declared architecture differs from implementation reality.
7. Provide a stable machine interface regardless of the underlying analyzer.

## 5. Goals

### G1 — System understanding
Represent systems, services, components, responsibilities, ownership, contracts, boundaries, flows, failure behavior, and invariants.

### G2 — Context efficiency
Default startup pack <= 8k tokens; default task pack <= 12k tokens.

### G3 — Evidence
Every trusted fact is linked to evidence and a repository revision.

### G4 — Freshness
Changed files invalidate dependent facts and affected packs incrementally.

### G5 — Progressive detail
Agents receive compact system context first and source/CFG/DFG/PDG detail only when necessary.

### G6 — Agent-native API
Expose a small intent-level API instead of dozens of graph operations.

### G7 — Local-first
No source-code upload by default. Remote inference/embeddings are opt-in.

### G8 — Analyzer independence
Allow GitNexus, Narsil, CBM, SCIP/LSP, CodeQL, Joern, or native SCC extractors to contribute evidence without changing the agent API.

## 6. Non-goals

SCC is not initially:
- an IDE;
- a diagram editor;
- a replacement for GitHub Issues;
- a general memory system;
- an agent orchestrator;
- a full observability platform;
- a generic code search engine;
- a replacement for an LSP/compiler;
- a human documentation portal.

## 7. Functional requirements

### P0
- Index a repository and revision.
- Extract files, symbols, imports, candidate calls, routes, tests.
- Build a Reality Graph.
- Build component and service candidates.
- Build Architecture, Sequence, and Data Flow machine views.
- Attach provenance and confidence.
- Generate `system_overview`.
- Generate bounded `task_context`.
- Detect stale evidence after edits.
- Incrementally re-index changed files.
- Run as local daemon + CLI + MCP.
- Integrate with Claude Code.

### P1
- Lifecycle and Workflow views.
- DB ownership and writers/readers.
- Queue/event producers and consumers.
- Retry/fallback detection.
- LSP/SCIP semantic-resolution adapters.
- GitNexus/Narsil import adapters.
- Serena exact-navigation integration.
- Beads task metadata integration.
- CI drift and architecture checks.
- OpenTelemetry trace ingestion.

### P2
- Cross-repository system model.
- Deployment/trust-boundary graph.
- Hindsight recall integration.
- External docs via Context7.
- Team server.
- Enterprise ACLs.
- Multi-repo contract drift.
- Runtime/static reconciliation analytics.

## 8. User experience requirements

### First run
```bash
scc init
scc index
scc setup claude
```

After that, normal coding should not require a slash command.

### Agent startup
The integration should inject:
- repository identity and revision;
- system capsule;
- stale-model warning if applicable.

### Agent task
For repository-changing requests the integration should obtain a task pack and inject it before planning or editing.

### Agent drill-down
The agent asks semantic questions instead of SPARQL/Cypher or analyzer-specific commands.

## 9. Success metrics

### Agent outcome metrics
- higher task completion rate;
- lower missed-dependency rate;
- fewer exploratory tool calls;
- fewer irrelevant file reads;
- lower context-token usage;
- lower incorrect-assumption rate;
- fewer post-hoc discoveries after tests fail.

### System metrics
- task-context precision >= 0.85;
- task-context recall >= 0.95;
- EXTRACTED fact precision >= 99.5%;
- RESOLVED fact precision >= 98%;
- warm task-context P95 < 500 ms;
- incremental update P95 < 3 s for ordinary edits.

## 10. Acceptance scenario

Given a repo with frontend, API, worker, queue, DB, and external model service, and the request:

> Rename API response field `transcript` to `normalizedTranscript`.

Before the first edit, SCC should include:
- route/handler;
- response contract;
- frontend consumer;
- worker/background consumer;
- relevant schema;
- affected flow;
- tests;
- persistence mapping if relevant;
- any invariant around raw vs normalized transcript.

A failure is discovering one of these only because a later test broke.
