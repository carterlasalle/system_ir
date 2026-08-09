# System Context Compiler
## Product Requirements, Technical Specification, Data Strategy, API, QA, Infrastructure, and Implementation Plan

**Working name:** System Context Compiler  
**Core artifact:** System IR  
**Machine system-map layer:** System Atlas  
**Version:** 0.1  
**Status:** Proposed

---

# 1. Executive Summary

Coding agents have a repository-context problem.

Existing approaches generally make one of four tradeoffs:

1. **Give the model lots of source code.**
   - Accurate.
   - Expensive.
   - Context-flooding.
   - Mostly irrelevant to any particular task.

2. **Give the model search and navigation tools.**
   - Efficient.
   - Precise.
   - But the agent must know what to search for.
   - The agent spends many turns reconstructing the system.

3. **Give the model a code graph.**
   - Better understanding of calls, imports, types, dependencies, and impact.
   - Still usually too low-level.
   - Does not necessarily tell the model how the *system* works.

4. **Generate architecture documentation or diagrams.**
   - Good for humans.
   - Often authored or interpreted by an LLM.
   - Frequently stale.
   - Not optimized as a machine reasoning substrate.

System Context Compiler solves the missing layer.

It compiles repositories, configuration, infrastructure, runtime observations, tests, and explicit architectural intent into a **verified machine representation of the software system**.

The product then compiles that representation again into exactly the context an AI coding agent needs for the current task.

The fundamental pipeline is:

```text
Source code
Configuration
Infrastructure
Tests
Runtime traces
Architectural intent
Git history
       │
       ▼
┌─────────────────────────────┐
│      Reality Compiler       │
└─────────────────────────────┘
       │
       ▼
Reality Graph
       │
       ▼
┌─────────────────────────────┐
│    System IR Compiler       │
└─────────────────────────────┘
       │
       ▼
System IR
├── components
├── responsibilities
├── ownership
├── contracts
├── system boundaries
├── architecture
├── workflows
├── sequences
├── data flows
├── lifecycles
├── failure paths
├── retries
├── invariants
└── evidence
       │
       ▼
┌─────────────────────────────┐
│      Context Compiler       │
└─────────────────────────────┘
       │
       ├── startup capsule
       ├── task context
       ├── component context
       ├── flow context
       ├── impact context
       └── verification context
       │
       ▼
Claude Code / Codex / Hermes / OpenCode / other agents
```

The product is therefore **not another LSP, code search engine, RAG system, architecture diagram generator, or MCP collection**.

It is the layer above them.

---

# 2. Product Thesis

The core thesis is:

> Coding agents perform better when they are given a compact, verified model of the system before they begin searching through implementation details.

An agent should not have to rediscover:

- Why a service exists.
- Which component owns a data entity.
- How a user request travels through the application.
- Which asynchronous boundary follows an API handler.
- What happens when a dependency fails.
- Which retry mechanism applies.
- Which database is authoritative.
- Which service is allowed to mutate a record.
- Which invariants a refactor must preserve.
- Which tests prove those invariants.
- Which deployment unit contains a component.
- Which external dependencies participate in the flow.

Code-level tools remain essential for precision.

They simply should not be responsible for constructing the model's entire mental representation of the system on every task.

---

# 3. Product Vision

The desired experience is:

```text
Developer:
"Change street-name normalization so radio transcripts preserve
department-specific street names."

             │
             ▼

SCC identifies:
- Transcript Normalizer
- Street Name Resolver
- ASR ingestion flow
- replay flow
- Incident extraction consumer
- transcript database ownership
- raw-text immutability invariant
- config containing department vocabulary
- relevant unit/integration tests

             │
             ▼

Agent receives:

SYSTEM
The application processes live emergency-radio audio...

TASK SCOPE
TranscriptNormalizer.normalize
StreetNameResolver.resolve

RELEVANT FLOW
Radio → Buffer → ASR → Normalize → Incident Extraction → Store

DATA CONTRACT
Transcript.raw_text is immutable.
Transcript.normalized_text is derived.

UPSTREAM
ASR Service

DOWNSTREAM
IncidentExtractor
SearchIndexer

FAILURE BEHAVIOR
Resolver failure falls back to normalized ASR text.

IMPLEMENTATION
...
TESTS
...

             │
             ▼

Agent begins implementation with correct system understanding.
```

The agent can still drill into code.

It simply begins from the right abstraction.

---

# 4. Product Goals

## G1. Give agents system-level understanding

The product must explain:

- What the system does.
- What the major subsystems are.
- What each subsystem is responsible for.
- How those subsystems communicate.
- Where implementation lives.
- Which runtime paths matter.

---

## G2. Prevent context flooding

The complete System IR may contain millions of facts.

The agent must never receive all of them.

Instead:

```text
Always loaded
    ↓
small system capsule

Task arrives
    ↓
task-specific system slice

Agent investigates
    ↓
progressively deeper evidence
```

---

## G3. Ground system claims in evidence

Every meaningful claim must have provenance.

The system must distinguish:

```text
EXTRACTED
RESOLVED
OBSERVED
DECLARED
INFERRED
STALE
```

An inferred architecture claim must never appear equivalent to a compiler-resolved call edge.

---

## G4. Model behavior, not just structure

The product must model:

- Runtime workflows
- Sequences
- Data movement
- State transitions
- Branches
- Failures
- Retries
- Fallbacks

A dependency graph alone is insufficient.

---

## G5. Keep the model continuously current

The model must be tied to:

- Git commit
- File content hashes
- Extraction version
- Configuration version
- Runtime evidence timestamps

Changed files should invalidate dependent facts.

---

## G6. Be agent-native

The main consumer is an AI agent.

The primary output therefore must be:

- Typed
- Queryable
- Compact
- Deterministic where possible
- Provenance-aware
- Token-budgeted

Human visualization is optional.

---

## G7. Work across agent harnesses

Initial targets:

- Claude Code
- Codex
- Hermes
- OpenCode

Additional integrations can follow.

---

## G8. Be local-first

Source code must remain local by default.

Cloud models, embeddings, or hosted services must be opt-in.

---

# 5. Non-Goals

SCC is not initially intended to be:

- An IDE.
- A GitHub replacement.
- A task manager.
- A general vector database.
- A code-review product.
- An architecture drawing application.
- A documentation generator.
- A generic agent framework.
- A replacement for LSPs.
- A replacement for compilers.
- A replacement for runtime observability.
- A replacement for GitNexus, Serena, Narsil, CodeQL, or similar analyzers.

Those systems can become evidence providers or integrations.

---

# 6. Design Influences From Existing Tools

SCC deliberately combines ideas that currently exist separately.

## GitNexus influence

GitNexus demonstrates the usefulness of precomputed code relationships, execution processes, route maps, API impact, shape checking, cross-repository contracts, and graph-based change analysis. Its current agent surface exposes 17 MCP-oriented tools/resources.

SCC should consume this kind of evidence but expose a smaller semantic surface.

---

## Narsil / Code Context Graph influence

Narsil demonstrates a machine-first progressive representation:

```text
L0 manifest
L1 architecture
L2 symbols/calls
L3 full detail
```

Its CCG implementation is specifically intended as an AI-consumable representation and supports JSON-LD/RDF and progressive querying.

SCC should preserve this principle while expanding the architecture vocabulary from:

```text
module
API
dependency
symbol
```

to:

```text
system
service
component
responsibility
contract
ownership
workflow
sequence
data flow
lifecycle
failure
retry
invariant
```

---

## Serena influence

Serena demonstrates why live LSP-backed semantic navigation remains valuable even after indexing. It provides IDE-like symbol retrieval and editing and can be connected directly to coding agents.

SCC should use Serena or native language servers for exact live workspace truth rather than rebuilding every IDE operation.

---

## Augment influence

Augment's Context Engine emphasizes:

- Semantic repository understanding.
- Cross-repository relationships.
- Code plus external knowledge.
- Task-specific curation.
- Context compression.

That curation philosophy is central to SCC's Context Compiler.

---

## Archify influence

Archify provides useful conceptual categories:

- Architecture
- Workflow
- Sequence
- Data flow
- Lifecycle

Its product is primarily oriented toward polished, validated system diagrams.

SCC will reuse the **semantic categories**, not the presentation layer.

The result will be **Archify for machines**.

---

# 7. Core Product Concepts

There are four core data products.

## 7.1 Reality Graph

The Reality Graph stores low-level facts directly derived from evidence.

Examples:

```text
file contains symbol
symbol calls symbol
class implements interface
module imports module
route handled_by symbol
service deployed_in container
function queries table
consumer subscribes_to topic
test exercises symbol
config references service
```

This layer should be as deterministic as practical.

---

## 7.2 System IR

System IR contains the higher-level system model.

It answers:

- What components exist?
- What are they responsible for?
- Which data do they own?
- What contracts connect them?
- Which runtime flows pass through them?
- What invariants must remain true?
- What evidence supports those facts?

---

## 7.3 System Atlas

System Atlas is the machine-oriented equivalent of the human-facing architecture maps discussed earlier.

It contains five semantic view types.

### Architecture

Static system structure:

```text
components
services
stores
queues
external systems
deployment boundaries
trust boundaries
relationships
```

### Workflow

Operational processes:

```text
step
actor
responsibility
branch
approval
failure
rollback
```

### Sequence

Ordered interactions:

```text
caller
callee
message
return
async boundary
timeout
retry
```

### Data Flow

Movement and transformation:

```text
source
payload
transform
classification
store
consumer
ownership
retention
```

### Lifecycle

State behavior:

```text
state
event
transition
retry
wait
failure
terminal outcome
```

No coordinates.

No colors.

No SVG.

No layout.

Only machine semantics.

---

## 7.4 Context Packs

A Context Pack is a generated, bounded representation sent to an agent.

Types:

```text
startup
task
component
flow
contract
impact
verification
```

---

# 8. System IR Data Model

Every entity receives a stable URI-like identifier.

Example:

```text
repo://phoenix-fireworks/component/transcript-normalizer
repo://phoenix-fireworks/service/asr
repo://phoenix-fireworks/data/transcript
repo://phoenix-fireworks/flow/live-radio
```

---

# 9. Entity Types

Initial schema:

```text
Repository
Workspace
Package
Module
File
Symbol

System
Subsystem
Service
Component
DeploymentUnit

Route
Endpoint
RPCMethod
Tool
Event
Topic
Queue

DataEntity
DataStore
Table
Collection
Cache

ExternalSystem
ExternalAPI

Configuration
FeatureFlag
SecretReference

Contract
Invariant

Test
TestSuite

Flow
FlowStep
Branch
Transition
State

TrustBoundary
SecurityControl

RuntimeObservation
Evidence
```

---

# 10. Relationship Types

Initial relationship ontology:

```text
contains
implements
inherits
imports
calls
reads
writes
queries
owns
publishes
consumes
subscribes
produces
transforms
validates
routes_to
handles
invokes
depends_on
deployed_with
deployed_in
configured_by
protected_by
crosses_boundary
enforces
tested_by
participates_in
precedes
follows
branches_to
retries
falls_back_to
observed_as
declared_as
implemented_by
```

---

# 11. Provenance Model

Every fact must contain:

```json
{
  "fact_id": "fact:123",
  "subject": "component:incident-engine",
  "predicate": "writes",
  "object": "store:postgres-incidents",

  "provenance": "RESOLVED",
  "confidence": 0.99,

  "evidence": [
    {
      "type": "source",
      "path": "services/incidents/repository.py",
      "symbol": "IncidentRepository.save",
      "lines": [44, 78],
      "commit": "abc123"
    }
  ],

  "extractor": {
    "name": "python-call-resolver",
    "version": "0.6.2"
  },

  "verified_at": "..."
}
```

---

# 12. Provenance Classes

## EXTRACTED

Direct syntax/configuration evidence.

Examples:

```text
import statement
route decorator
Docker service
Terraform resource
environment reference
```

Target confidence:

```text
1.0
```

---

## RESOLVED

Relationship resolved through semantic tooling.

Examples:

```text
actual function call target
interface implementation
overload resolution
dependency injection target
```

---

## OBSERVED

Runtime evidence.

Examples:

```text
OpenTelemetry span
production trace
test trace
message delivery
SQL query
```

---

## DECLARED

Explicit human or repository intent.

Examples:

```text
ADR
architecture manifest
ownership declaration
System IR override
```

---

## INFERRED

Heuristic or model-derived conclusion.

Examples:

```text
probable component boundary
probable responsibility
possible failure relationship
```

Inferred facts must:

- Include confidence.
- Include evidence.
- Be separately labeled.
- Never silently become authoritative.

---

## STALE

Fact evidence no longer matches the active repository snapshot.

---

# 13. Example Component IR

```json
{
  "id": "component:incident-engine",
  "kind": "component",
  "name": "Incident Engine",

  "responsibilities": [
    {
      "text": "Extract incidents from normalized radio transcripts",
      "provenance": "RESOLVED",
      "confidence": 0.94
    },
    {
      "text": "Merge related transmissions into active incidents",
      "provenance": "RESOLVED",
      "confidence": 0.96
    }
  ],

  "implementation": {
    "paths": [
      "services/incidents/**"
    ],
    "symbols": [
      "IncidentExtractor",
      "IncidentMerger",
      "IncidentRepository"
    ]
  },

  "owns": [
    "entity:Incident"
  ],

  "participates_in": [
    "flow:live-radio",
    "flow:historical-replay"
  ],

  "depends_on": [
    "component:transcript-normalizer"
  ],

  "evidence": []
}
```

---

# 14. Example Sequence IR

```json
{
  "id": "sequence:live-transmission",

  "trigger": {
    "type": "event",
    "name": "Radio packet received"
  },

  "steps": [
    {
      "order": 1,
      "actor": "component:radio-receiver",
      "operation": "receive"
    },
    {
      "order": 2,
      "actor": "component:transmission-buffer",
      "operation": "append"
    },
    {
      "order": 3,
      "actor": "component:segmenter",
      "operation": "finalize",
      "condition": "silence threshold OR timeout"
    },
    {
      "order": 4,
      "actor": "service:asr",
      "operation": "transcribe"
    },
    {
      "order": 5,
      "actor": "component:transcript-normalizer",
      "operation": "normalize"
    },
    {
      "order": 6,
      "actor": "component:incident-engine",
      "operation": "extractAndMerge"
    }
  ],

  "branches": [
    {
      "after": 4,
      "condition": "ASR timeout",
      "behavior": {
        "type": "retry",
        "policy": "bounded-backoff"
      }
    }
  ]
}
```

---

# 15. Example Data Flow IR

```json
{
  "id": "dataflow:audio-to-incident",

  "data": [
    {
      "name": "RawAudio",
      "source": "component:radio-receiver"
    },
    {
      "name": "RawTranscript",
      "producer": "service:asr",
      "owner": "component:transcript-store"
    },
    {
      "name": "NormalizedTranscript",
      "producer": "component:transcript-normalizer"
    },
    {
      "name": "Incident",
      "producer": "component:incident-engine",
      "owner": "component:incident-engine"
    }
  ],

  "transforms": [
    {
      "from": "RawAudio",
      "to": "RawTranscript",
      "operation": "speech-to-text"
    },
    {
      "from": "RawTranscript",
      "to": "NormalizedTranscript",
      "operation": "normalization"
    }
  ]
}
```

---

# 16. Example Invariant

```json
{
  "id": "invariant:raw-transcript-immutable",

  "statement": "Raw ASR output must never be overwritten by transcript normalization.",

  "scope": [
    "entity:Transcript",
    "component:transcript-normalizer"
  ],

  "enforced_by": [
    "schema:Transcript.raw_text",
    "schema:Transcript.normalized_text",
    "test:test_normalization_preserves_raw"
  ],

  "severity": "critical"
}
```

---

# 17. Context Compiler

The Context Compiler is arguably the most important product component.

A perfect graph with a bad context compiler still fails.

---

# 18. Context Levels

## Level 0 — Identity

Approximately:

```text
500–1,500 tokens
```

Contains:

- Repository purpose
- Languages
- Major entrypoints
- Major systems
- Index status

---

## Level 1 — System Capsule

Target:

```text
3,000–8,000 tokens
```

Contains:

- System purpose
- Major components
- Major boundaries
- Data ownership
- Critical stores
- Major external systems
- Primary workflows
- Architectural invariants

Normally loaded at session start.

---

## Level 2 — Task Context

Target:

```text
4,000–12,000 tokens
```

Generated for each user task.

Contains:

```text
target components
affected flows
upstream dependencies
downstream dependencies
contracts
data ownership
states
failure behavior
relevant invariants
implementation symbols
tests
recent changes
```

---

## Level 3 — Deep Evidence

Loaded only when required:

```text
symbol definitions
call graphs
source excerpts
CFG
DFG
PDG
taint
runtime traces
Git history
```

---

# 19. Context Selection Algorithm

Input:

```text
user goal
active files
active symbols
git diff
current Bead/task
recent agent actions
```

Candidate entities are gathered from:

```text
semantic search
lexical search
graph expansion
flow participation
contract dependencies
ownership
runtime relationships
change impact
test coverage
```

Ranking should combine:

```text
task semantic relevance
graph distance
flow participation
change proximity
ownership relevance
contract relevance
invariant severity
runtime frequency
source confidence
freshness
```

Conceptually:

```text
score =
    semantic_relevance
  + graph_relevance
  + flow_relevance
  + ownership_relevance
  + change_relevance
  + invariant_relevance
  + confidence
  - staleness
  - redundancy
```

Context generation then performs:

1. Deduplication.
2. Abstraction.
3. Evidence compression.
4. Token budgeting.
5. Consistency validation.

---

# 20. Context Pack Structure

Agent-facing packs should be structured text rather than raw graph data.

Example:

```text
# TASK CONTEXT

Goal
----
Modify street-name correction for fire-radio transcripts.

System role
-----------
Transcript Normalizer converts raw ASR output into a normalized transcript
used by Incident Extraction and Search Indexing.

Relevant components
-------------------
TranscriptNormalizer
StreetNameResolver
IncidentExtractor
TranscriptRepository

Primary flow
------------
Radio
→ Segmenter
→ ASR
→ TranscriptNormalizer
→ IncidentExtractor
→ IncidentRepository

Secondary flow
--------------
Historical Replay
→ TranscriptNormalizer
→ IncidentExtractor

Data ownership
--------------
RawTranscript:
  owner: TranscriptRepository
  immutable: true

NormalizedTranscript:
  producer: TranscriptNormalizer

Invariant
---------
Raw ASR output must never be overwritten.

Upstream
--------
ASRClient.transcribe

Downstream
----------
IncidentExtractor.extract
SearchIndexer.index

Failures
--------
Resolver failure:
  fallback: normalized ASR without vocabulary correction

Relevant implementation
-----------------------
...

Tests
-----
...

Evidence status
---------------
11 RESOLVED
6 EXTRACTED
2 OBSERVED
0 STALE
```

---

# 21. Reality Compilation Pipeline

The indexer should use multiple evidence sources.

```text
Repository
   │
   ├── syntax extraction
   ├── semantic resolution
   ├── framework extraction
   ├── infrastructure parsing
   ├── test analysis
   ├── history
   ├── runtime telemetry
   └── explicit intent
```

---

# 22. Language Extraction

Architecture should support pluggable frontends.

Priority order:

## Tier 1

- TypeScript / JavaScript
- Python
- Go
- Rust

## Tier 2

- Java
- C#
- C / C++

## Tier 3

- Kotlin
- Swift
- PHP
- Ruby
- Dart

---

# 23. Parser Sources

The extraction engine may combine:

### Tree-sitter

For:

```text
syntax
definitions
imports
decorators
basic call candidates
```

### SCIP

Where high-quality indexers exist.

### LSP

For:

```text
definitions
references
implementations
type resolution
diagnostics
```

### Compiler APIs

Where maximum precision is needed.

### Existing analyzers

Adapters can consume results from:

- GitNexus
- Narsil
- codebase-memory-mcp
- CodeQL
- Joern

This dramatically accelerates early development.

---

# 24. Framework Extractors

Initial framework adapters:

## Web

- Next.js
- React
- Express
- FastAPI
- Flask
- Django
- Spring
- ASP.NET
- Go HTTP frameworks

Extract:

```text
routes
handlers
middleware
server actions
RPCs
API consumers
```

---

# 25. Data Layer Extractors

Support:

- PostgreSQL
- MySQL
- SQLite
- MongoDB
- Redis
- Supabase

Analyze:

```text
ORM entities
SQL queries
schema migrations
read/write ownership
transactions
indexes
```

---

# 26. Messaging Extractors

Support:

- Kafka
- RabbitMQ
- Redis streams
- NATS
- SQS
- Pub/Sub

Derive:

```text
producer
topic
consumer
consumer group
retry
dead-letter flow
```

---

# 27. Infrastructure Extractors

Parse:

```text
Dockerfile
docker-compose.yml
Kubernetes manifests
Helm
Terraform
GitHub Actions
Vercel config
Cloudflare config
Supabase config
systemd
environment schema
```

Derive:

```text
deployment unit
service exposure
network boundary
secret dependency
port
region
storage
health check
startup dependency
```

---

# 28. Runtime Evidence

Static analysis alone cannot prove runtime architecture.

SCC should optionally ingest:

- OpenTelemetry traces.
- Test traces.
- Application logs.
- Sentry traces.
- HTTP access logs.
- SQL traces.
- Queue telemetry.

Runtime observations add:

```text
OBSERVED call edge
OBSERVED service flow
frequency
latency
failure rate
last observation
```

---

# 29. Flow Compiler

This is a major differentiator.

The Flow Compiler derives candidate runtime flows from entrypoints.

Example:

```text
HTTP route
→ handler
→ service
→ repository
→ database
```

Then it follows:

```text
calls
events
messages
queues
RPC
storage
external APIs
```

It attempts to identify:

```text
happy path
branches
async handoffs
retry loops
fallbacks
terminal outcomes
```

---

# 30. Flow Confidence

Each step receives evidence:

```text
STATIC_RESOLVED
STATIC_POSSIBLE
RUNTIME_OBSERVED
DECLARED
INFERRED
```

A sequence can therefore say:

```text
step A → B
  resolved: true

step B → C
  observed: 394 times

step C → D
  inferred: 0.61
```

---

# 31. Component Compiler

Low-level symbols must be aggregated into system components.

Signals:

```text
package boundaries
directory boundaries
deployment boundaries
namespace
call cohesion
dependency direction
shared data ownership
route ownership
event ownership
Git co-change
framework modules
explicit declarations
```

Community detection can propose components but cannot establish architectural truth by itself.

---

# 32. Responsibility Compiler

Responsibilities are inherently more semantic than calls.

The compiler should derive candidate responsibilities from:

```text
public APIs
routes
owned data
events
entrypoints
tests
docstrings
names
ADRs
configuration
```

LLMs may assist here.

However:

> The LLM may label evidence. It may not invent topology.

Generated responsibility claims must be marked:

```text
INFERRED
```

until promoted by:

- Explicit declaration.
- Human approval.
- Strong deterministic evidence.

---

# 33. Architectural Intent

Allow:

```text
.scc/intent.yaml
```

Example:

```yaml
components:
  incident-engine:
    responsibility:
      - derive structured incidents from radio transcripts
    owns:
      - Incident

invariants:
  raw-transcript-immutable:
    statement: raw ASR output cannot be modified
    severity: critical

flows:
  live-radio:
    entrypoint: RadioReceiver.handle
```

Intent never overwrites reality.

Instead:

```text
DECLARED intent
versus
RESOLVED reality
```

produces drift.

---

# 34. Intent–Reality Drift

SCC should detect:

```text
DECLARED component missing
DECLARED ownership violated
unexpected dependency
unexpected database writer
flow no longer reaches declared sink
component moved
API contract changed
new trust-boundary crossing
invariant lacks enforcing test
```

This becomes a CI feature.

---

# 35. API Design

The central principle:

> Agents receive intent-level tools, not analyzer-level tools.

Do not expose 50–100 graph functions.

Expose approximately six primary calls.

---

# 36. MCP API

## `system_overview`

Returns:

```text
system purpose
components
boundaries
primary flows
stores
external systems
invariants
freshness
```

---

## `task_context`

Input:

```json
{
  "goal": "Modify transcript normalization",
  "files": [],
  "symbols": [],
  "token_budget": 8000
}
```

Returns the context pack.

This should be the primary agent operation.

---

## `component_context`

Input:

```text
component ID/name
```

Returns:

```text
responsibility
implementation
dependencies
ownership
flows
contracts
tests
evidence
```

---

## `flow_context`

Input:

```text
flow ID/name
```

Returns:

```text
trigger
steps
branches
data
failures
retries
evidence
```

---

## `impact_context`

Input:

```text
files
symbols
git diff
contract
```

Returns:

```text
affected components
flows
consumers
contracts
data
invariants
tests
risk
```

---

## `verify_context`

Checks:

```text
freshness
stale facts
unverified claims
missing evidence
intent drift
```

---

# 37. Advanced API

Power users may access:

```text
query_graph
get_fact
get_evidence
search_symbols
search_system
export_ir
```

These should not appear in the default agent profile.

---

# 38. HTTP API

Suggested endpoints:

```text
GET  /v1/system
POST /v1/context/task
GET  /v1/components/{id}
GET  /v1/flows/{id}
POST /v1/impact
POST /v1/verify
GET  /v1/facts/{id}
GET  /v1/evidence/{id}

POST /v1/index
GET  /v1/index/status
POST /v1/index/refresh

POST /v1/runtime/traces
```

---

# 39. CLI

```bash
scc init
scc index
scc status
scc watch

scc overview

scc context task "change transcript normalization"
scc context component incident-engine
scc context flow live-radio

scc impact
scc impact --diff HEAD~1

scc verify
scc drift

scc export system-ir.json
scc export ccg
```

---

# 40. SDK

First-party SDKs:

- Rust
- TypeScript
- Python

Example:

```typescript
const ctx = await scc.taskContext({
  goal: "Add retry handling to ASR calls",
  tokenBudget: 8000
});
```

---

# 41. Claude Code Integration

Provide an SCC Claude Code plugin.

## SessionStart

```text
scc verify freshness
scc system_overview
restore task checkpoint
load active Bead if available
```

---

## UserPromptSubmit

For repository-changing requests:

```text
derive task context
inject task pack
```

Do not inject on:

```text
simple conversational requests
git-only operations
non-code questions
```

---

## PreToolUse / PreEdit

Before source modification:

```text
resolve relevant component
resolve invariants
optional Serena symbol verification
```

---

## PostToolUse / PostEdit

After edits:

```text
incremental reindex
detect affected flows
update impact state
```

---

## PreCompact

Store:

```text
active goal
active task
modified files
affected components
affected flows
decisions
tests
next action
```

---

## SessionStart after compaction

Restore:

```text
checkpoint
active task
system capsule
task slice
```

---

# 42. Serena Integration

SCC answers:

> What matters?

Serena answers:

> Where exactly is it right now?

Workflow:

```text
SCC task_context
        ↓
SCC identifies IncidentEngine
        ↓
Serena find_symbol
        ↓
Serena references
        ↓
agent edits
        ↓
SCC impact_context
```

Serena remains ideal for live semantic navigation rather than being replaced.

---

# 43. GitNexus Integration

Optional adapter:

```text
GitNexus
    ↓
symbols
processes
impact
routes
contracts
PDG
    ↓
SCC Reality Graph
```

SCC should hide GitNexus's tool surface from normal agents.

GitNexus becomes an evidence backend.

---

# 44. Narsil Integration

Optional adapter:

```text
Narsil CCG
    ↓
L0–L3 repository evidence
    ↓
SCC Reality Graph
    ↓
System IR
```

Narsil's progressive CCG format makes it especially useful as an import/export source.

---

# 45. codebase-memory-mcp Integration

CBM is an alternative broad/polyglot evidence source.

The user should generally select:

```text
GitNexus
OR
CBM
OR
Narsil
```

rather than present all of them to the coding model.

SCC's adapter architecture allows these backends without changing the agent interface.

---

# 46. Context7 Integration

SCC owns:

```text
internal system truth
```

Context7 owns:

```text
external library/API documentation
```

When task context identifies a dependency whose API may have changed:

```text
SCC
→ request relevant dependency
→ Context7
→ append external API evidence
```

External docs must never be mixed with repository facts without source labels.

---

# 47. Beads Integration

Beads should remain operational task state, not architecture state.

Beads currently describes itself as a persistent dependency-aware issue graph for coding agents.

SCC integration:

```text
active Bead
    ↓
goal
dependencies
related files
previous discoveries
    ↓
task_context
```

SCC may attach:

```text
affected_components
affected_flows
affected_contracts
```

to Bead metadata.

---

# 48. Hindsight Integration

Hindsight should store durable learned experience:

```text
root causes
failed approaches
architecture decisions
operational lessons
project conventions
```

Hindsight provides retain, recall, and reflect over memory banks and combines semantic, keyword, graph, and temporal retrieval.

Authority remains:

```text
repository/runtime
>
System IR
>
task state
>
Hindsight
>
model assumption
```

SCC should never import Hindsight memories as deterministic code facts.

---

# 49. RTK Integration

RTK should remain optional output middleware.

RTK compresses shell output before agent context.

SCC can provide RTK with context-aware compression policies.

Example:

```text
task touches tests
→ preserve failures
→ compress passing output

task is performance investigation
→ disable log compression
```

Do not treat output-byte reduction as proof of total agent savings; real-world reports have shown that end-to-end savings can vary substantially by workload.

---

# 50. Ponytail Integration

Ponytail operates after SCC understands the task.

```text
SCC:
what is the system and what must change?

Ponytail:
what is the smallest correct implementation?
```

Ponytail's normal Full mode applies a reuse/minimalism ladder after understanding the touched code and preserves security, validation, accessibility, and data-loss protections.

Recommended agent flow:

```text
SCC task context
→ planning
→ Serena exact source
→ Ponytail implementation discipline
→ verification
```

---

# 51. Superpowers Integration

Superpowers remains workflow/process policy.

Example:

```text
SCC = knowledge
Superpowers = procedure
Ponytail = implementation restraint
```

No semantic overlap is required.

---

# 52. Agent Deck Integration

Agent Deck can become an operator-layer integration for multi-session workflows.

It already handles session visibility, worktrees, MCP selection, skills, and process pooling.

Potential SCC integration:

```text
session card:
  active component
  active flow
  active Bead
  System IR revision
  stale/not stale
```

---

# 53. Docker MCP Gateway Integration

Use profiles rather than exposing every external system.

Docker MCP Gateway supports MCP profiles and tool enable/disable controls.

Suggested profiles:

```text
repo-core
web-debug
production
security
infrastructure
```

SCC belongs in:

```text
repo-core
```

and remains always available.

---

# 54. External Integration Profiles

## Web

- Playwright
- Chrome DevTools
- Next.js tooling

## Production

- Sentry
- GitHub
- Vercel

## Infrastructure

- Cloudflare
- Supabase
- Terraform

## Security

- Semgrep
- CodeQL
- Joern

These provide evidence or verification.

They are not part of the repository-modeling core.

---

# 55. Security Strategy

Security is critical because SCC reads the entire repository and injects information into agents.

---

# 56. Local-First

Default mode:

```text
source never leaves machine
```

Remote services require explicit enablement.

---

# 57. Repository Text Is Untrusted

README files, comments, documentation, test fixtures, and vendored code can contain prompt injection.

SCC must parse repository text as **data**, not executable agent instructions.

Context packs should clearly delimit:

```text
SYSTEM FACT
SOURCE EXCERPT
DECLARED DOCUMENTATION
UNTRUSTED TEXT
```

---

# 58. Secret Handling

Before indexing:

```text
detect likely secrets
redact secret values
store only references where possible
```

Example:

```text
DATABASE_URL referenced
```

not:

```text
postgres://username:password...
```

---

# 59. Plugin Security

Adapters must declare:

```text
filesystem access
network access
commands executed
credentials used
```

Third-party adapters should run sandboxed.

Snyk Agent Scan can optionally be used during plugin onboarding; it currently scans agent configs, MCP servers, and skills for prompt injection, tool poisoning, credential handling, and related risks.

---

# 60. Data Strategy

The storage architecture should use six logical layers.

```text
L0 Repository Snapshot
L1 Evidence Store
L2 Reality Graph
L3 System IR
L4 Context Indexes
L5 Runtime/History
```

---

# 61. L0 — Repository Snapshot

Contains metadata only:

```text
repository ID
commit SHA
branch
file paths
content hashes
languages
index timestamp
```

Avoid duplicating entire source files unless explicitly configured.

---

# 62. L1 — Evidence Store

Stores evidence pointers:

```text
file
line range
symbol
extractor
hash
runtime trace
configuration source
```

---

# 63. L2 — Reality Graph

Typed facts:

```text
node
edge
source
confidence
freshness
```

---

# 64. L3 — System IR

Higher-level system objects.

Small enough to be inspected and exported.

---

# 65. L4 — Context Indexes

Indexes for retrieval:

```text
BM25
semantic embedding
graph adjacency
flow membership
component membership
contract membership
test membership
```

---

# 66. L5 — Runtime and Historical Evidence

Optional:

```text
traces
aggregated runtime counts
Git co-change
blame
previous System IR snapshots
drift history
```

---

# 67. Storage Technology

For MVP, prefer simplicity.

## Core metadata

**SQLite**

Use for:

```text
facts
entities
edges
evidence
snapshots
configuration
schema versions
```

Benefits:

- Embedded.
- Transactional.
- Easy backup.
- FTS5.
- Cross-platform.
- Excellent local development ergonomics.

---

## Graph execution

Use application-side adjacency indexes in Rust.

Do not introduce a dedicated graph server in MVP.

For larger deployments, optional backends may later support:

- LadybugDB/Kuzu-style graph storage.
- Neo4j.
- RDF/Oxigraph.

---

## Search

### Lexical

SQLite FTS5 or Tantivy.

### Semantic

Optional HNSW vector index.

Embeddings must be optional.

---

# 68. Export Formats

Native:

```text
system-ir.json
system-ir.jsonl
```

Optional:

```text
JSON-LD
N-Quads
CCG-compatible exports
GraphML
Cypher
```

Human diagram export is not a core requirement.

---

# 69. Freshness Strategy

Each fact references:

```text
source content hash
extractor version
System IR schema version
```

When a source changes:

```text
invalidate direct facts
→ invalidate derived relationships
→ invalidate affected components
→ invalidate affected flows
→ regenerate context summaries
```

---

# 70. Incremental Indexing

Watch:

```text
filesystem
Git status
branch switch
commit
```

Changed files trigger only relevant analyzers.

Target:

```text
ordinary edit → usable updated model within 1–3 seconds
```

for typical local repositories.

---

# 71. Model-Assisted Extraction

LLMs are allowed in tightly constrained areas:

```text
responsibility labeling
component naming
flow summarization
ambiguous grouping
ADR interpretation
```

But the LLM receives structured evidence and must produce typed claims.

Example output:

```json
{
  "claim": "Component owns incident state",
  "evidence": ["fact:1", "fact:8"],
  "confidence": 0.82,
  "provenance": "INFERRED"
}
```

LLMs may never create silent deterministic edges.

---

# 72. Technical Architecture

Recommended implementation language:

**Rust**

Reasons:

- Fast parsing.
- Low memory.
- Easy static CLI/daemon.
- Strong concurrency.
- Tree-sitter ecosystem.
- Good SQLite support.
- Strong data modeling.

Agent plugins can use TypeScript where native harness SDKs require it.

---

# 73. Repository Layout

```text
system-context-compiler/
├── crates/
│   ├── scc-core/
│   ├── scc-schema/
│   ├── scc-store/
│   ├── scc-indexer/
│   ├── scc-graph/
│   ├── scc-system-ir/
│   ├── scc-flow/
│   ├── scc-context/
│   ├── scc-runtime/
│   ├── scc-api/
│   ├── scc-mcp/
│   └── scc-cli/
│
├── adapters/
│   ├── gitnexus/
│   ├── narsil/
│   ├── serena/
│   ├── beads/
│   ├── hindsight/
│   └── opentelemetry/
│
├── extractors/
│   ├── typescript/
│   ├── python/
│   ├── go/
│   ├── rust/
│   ├── docker/
│   ├── kubernetes/
│   ├── terraform/
│   └── github-actions/
│
├── plugins/
│   ├── claude-code/
│   ├── codex/
│   ├── hermes/
│   └── opencode/
│
├── schemas/
│   └── system-ir/
│
├── fixtures/
│
└── benchmarks/
```

---

# 74. Process Architecture

Local daemon:

```text
                 sccd
                  │
       ┌──────────┼──────────┐
       │          │          │
    Indexer    Context     API/MCP
       │       Compiler
       │          │
       └──────┬───┘
              │
          SQLite DB
```

Workers perform:

```text
parsing
semantic resolution
flow extraction
embedding
runtime ingestion
```

---

# 75. Concurrency

Use job queues internally.

Priorities:

```text
P0 task-critical changed file
P1 active repository incremental
P2 background enrichment
P3 full semantic refresh
```

An active coding task should never wait behind full-repository reanalysis.

---

# 76. Test Suite and Quality Assurance

The QA strategy must validate both traditional software correctness and **context correctness**.

---

# 77. Unit Tests

Every extractor receives fixtures for:

```text
symbols
calls
imports
routes
database relationships
config references
events
retry constructs
```

---

# 78. Golden Repository Fixtures

Create intentionally small repositories demonstrating:

```text
HTTP request flow
queue-based workflow
database ownership
retry/fallback
state machine
microservice boundary
dependency injection
dynamic dispatch
feature flags
```

Expected System IR is committed.

Indexing must produce semantically equivalent results.

---

# 79. Differential Semantic Tests

For supported languages:

Compare SCC conclusions with:

```text
compiler
LSP
GitNexus
Narsil
SCIP
```

Disagreements become test cases.

---

# 80. Graph Invariant Tests

Examples:

```text
every edge references valid nodes
every flow step references a component/symbol
every RESOLVED fact has evidence
STALE facts cannot enter trusted task context
owned entity has at most one authoritative writer unless explicitly shared
```

---

# 81. Property-Based Tests

Use generated source trees to test:

```text
rename stability
incremental indexing
graph reachability
file movement
package movement
duplicate names
cyclic dependencies
```

---

# 82. Parser Fuzzing

Fuzz:

```text
source parser adapters
YAML
JSON
Docker Compose
Terraform
environment files
System IR imports
```

Malformed repository data must never crash the daemon.

---

# 83. Incremental Index Correctness

For every fixture:

```text
cold index result
==
incremental result after equivalent edit sequence
```

This is a release-blocking invariant.

---

# 84. Context Relevance Benchmark

This is the defining benchmark.

Create real repository tasks with ground-truth affected entities.

Compare:

```text
Baseline Claude Code
Baseline + Serena
Baseline + GitNexus
Baseline + SCC
```

Measure:

```text
files opened
search commands
tool calls
input tokens
output tokens
time to correct localization
affected dependencies missed
incorrect assumptions
final test success
```

---

# 85. Context Precision

For each generated Task Pack:

```text
precision =
relevant facts included
/
all facts included
```

---

# 86. Context Recall

```text
recall =
relevant ground-truth facts included
/
all relevant ground-truth facts
```

Targets for mature release:

```text
Task context precision ≥ 0.85
Task context recall ≥ 0.95
```

---

# 87. Fact Precision

Targets:

```text
EXTRACTED ≥ 99.5%
RESOLVED ≥ 98%
```

INFERRED facts are evaluated separately.

---

# 88. Flow Benchmark

Create manually verified flows.

Measure:

```text
step precision
step recall
branch precision
branch recall
ordering accuracy
ownership accuracy
```

---

# 89. Hallucination Test

Ask:

```text
"Does Service A call Service B?"
```

when no relationship exists.

System must:

```text
return unknown / no verified evidence
```

not synthesize a plausible path.

---

# 90. Staleness Tests

Change:

```text
function
route
queue
config
deployment
schema
```

Ensure affected System IR becomes stale or updates before it is served as trusted context.

---

# 91. Runtime Replay Tests

Feed stored traces.

Verify:

```text
OBSERVED edges
counts
latency aggregates
failure paths
```

---

# 92. Security Tests

Include:

```text
prompt injection in README
prompt injection in code comments
secrets in .env
malicious package documentation
symlink traversal
path traversal
malformed MCP response
hostile adapter
```

---

# 93. Agent Integration Tests

Run headless tasks through:

- Claude Code
- Codex
- Hermes
- OpenCode

Verify:

```text
context injection
compaction recovery
subagent behavior
tool availability
token budgeting
```

---

# 94. End-to-End Acceptance Scenario

Repository contains:

```text
frontend
API
worker
queue
database
external model service
```

Task:

```text
"Change API response field from transcript to normalizedTranscript."
```

SCC should identify before implementation:

```text
API handler
frontend consumer
worker consumer
schema
response contract
tests
affected flow
database mapping if applicable
```

The task is a failure if downstream dependencies are only discovered after tests break.

---

# 95. Product Quality Metrics

Primary:

```text
task success rate
dependency recall
context token count
tool-call reduction
incorrect assumption rate
```

Secondary:

```text
cold-index time
incremental-index latency
memory consumption
context generation latency
storage size
```

---

# 96. Performance Targets

Initial targets:

## Cold indexing

50k LOC:

```text
< 30 sec
```

250k LOC:

```text
< 2 min
```

excluding expensive optional compiler/security analysis.

---

## Incremental

Typical edit:

```text
P95 < 3 sec
```

---

## Task Context

Generation:

```text
P95 < 500 ms
```

after index is warm.

---

## Startup capsule

```text
≤ 8k tokens default
```

---

## Task pack

```text
≤ 12k tokens default
```

Hard configurable limit.

---

# 97. Deployment Strategy

SCC should support three modes.

---

# 98. Mode A — Developer Local

Default.

```text
sccd
SQLite
local parsers
local MCP
```

No network dependencies.

---

# 99. Mode B — CI

Run:

```bash
scc verify
scc drift
scc impact --diff origin/main...HEAD
```

CI can fail for:

```text
stale generated System IR
critical architectural drift
ownership violation
broken invariant
unapproved new boundary
```

---

# 100. Mode C — Team Server

For multi-repository organizations.

Components:

```text
API service
index workers
repository workers
PostgreSQL metadata
object storage
graph/query layer
optional vector service
authentication
```

This is post-MVP.

---

# 101. Docker

Publish:

```text
ghcr.io/.../scc
```

Example:

```yaml
services:
  scc:
    image: system-context-compiler
    volumes:
      - .:/repo:ro
      - scc-data:/data
```

---

# 102. Observability

SCC should expose OpenTelemetry itself.

Track:

```text
index duration
extractor duration
fact count
stale fact count
context generation
token size
cache hit
flow generation
errors
```

No repository content in telemetry by default.

---

# 103. Configuration

```text
.scc/config.yaml
```

Example:

```yaml
schema: 1

index:
  ignore:
    - vendor/**
    - generated/**

languages:
  typescript: true
  python: true

runtime:
  opentelemetry: false

context:
  startup_tokens: 6000
  task_tokens: 10000

inference:
  enabled: true
  provider: local

integrations:
  serena: true
  beads: true
```

---

# 104. Product Requirements

## P0 requirements

The MVP is not complete without:

- Repository indexing.
- Symbol/import/call graph.
- System component model.
- Architecture Atlas.
- Sequence Atlas.
- Data Flow Atlas.
- Evidence/provenance.
- Task Context generation.
- Incremental refresh.
- Claude Code integration.
- MCP integration.
- CLI.
- Staleness verification.

---

# 105. P1 Requirements

- Lifecycle Atlas.
- Workflow Atlas.
- Database ownership.
- Queue producer/consumer analysis.
- Runtime trace ingestion.
- Invariant system.
- GitNexus adapter.
- Narsil adapter.
- Serena integration.
- Beads integration.
- CI drift checks.

---

# 106. P2 Requirements

- Multi-repository architecture.
- Deployment graph.
- Trust boundaries.
- Cross-repo contracts.
- Hindsight integration.
- External documentation integration.
- Team server.
- Enterprise ACLs.
- Runtime-to-static reconciliation.

---

# 107. MVP Implementation Plan

## Phase 0 — Research Harness

**Duration:** milestone-based, not calendar-driven.

Build benchmark first.

Select 10 repositories:

```text
small Python
small TypeScript
large TypeScript
Python service
Go service
Rust service
Next.js full stack
queue worker system
Docker multi-service
polyglot monorepo
```

Create 5–10 ground-truth tasks per repository.

Measure baseline agent behavior.

This prevents optimizing based on intuition.

---

# 108. Phase 1 — Core Schema

Implement:

```text
Entity
Relationship
Evidence
Snapshot
Flow
Invariant
Component
ContextPack
```

Deliver:

```text
system-ir.schema.json
Rust types
TypeScript types
Python types
migration framework
```

No indexing complexity yet.

---

# 109. Phase 2 — Reality Graph MVP

Support:

- TypeScript
- Python

Extract:

```text
files
symbols
imports
calls
classes
interfaces
routes
tests
```

Store in SQLite.

Build:

```text
scc index
scc query
```

---

# 110. Phase 3 — Component Compiler

Generate components using:

```text
package
directory
imports
call cohesion
routes
data ownership
```

Provide:

```text
scc overview
```

Test against manually labeled repositories.

---

# 111. Phase 4 — System Atlas

Implement machine-oriented:

```text
Architecture
Sequence
Data Flow
```

Do not implement rendering.

Create flow extraction from:

```text
route
→ calls
→ external/storage/message boundaries
```

---

# 112. Phase 5 — Context Compiler

Implement:

```text
startup_context
task_context
component_context
flow_context
impact_context
```

Create strict token budgeting.

Benchmark context relevance.

This is the first point where the product becomes meaningfully differentiated.

---

# 113. Phase 6 — Claude Code Plugin

Implement hooks:

```text
SessionStart
UserPromptSubmit
PostToolUse
PreCompact
```

Install command:

```text
scc setup claude
```

Normal use should require no slash command.

---

# 114. Phase 7 — Semantic Precision

Add:

```text
LSP adapter
Serena adapter
SCIP support
```

Upgrade facts from:

```text
candidate
```

to:

```text
RESOLVED
```

---

# 115. Phase 8 — System Semantics

Add:

```text
database ownership
queues
events
deployment units
configuration
failure paths
retry/fallback
lifecycle
```

This is where SCC stops looking like another code graph.

---

# 116. Phase 9 — Runtime Reconciliation

Add OpenTelemetry ingestion.

Compare:

```text
static possible flow
vs
runtime observed flow
```

Expose:

```text
observed
never observed
new runtime edge
```

---

# 117. Phase 10 — Drift and CI

Implement:

```text
intent.yaml
scc drift
PR annotations
impact reports
architecture contracts
```

---

# 118. Phase 11 — Multi-Agent Integrations

Add:

```text
Beads
Agent Deck
Hindsight
Codex
Hermes
OpenCode
```

Keep SCC's data authority separate from task or memory systems.

---

# 119. Phase 12 — Multi-Repo

Build:

```text
organization model
cross-repository contracts
service dependencies
shared schema ownership
```

---

# 120. Suggested MVP Scope Reduction

Do **not** initially build:

- A frontend.
- Architecture diagrams.
- Cloud hosting.
- 30 languages.
- A custom vector database.
- An LSP implementation.
- A GitHub issue tracker.
- A memory system.
- An agent orchestrator.
- A security scanner.
- A general documentation crawler.

Reuse existing tools where they are already strong.

This follows the same minimalism principle Ponytail applies to implementation.

---

# 121. MVP Technical Stack

Recommended:

```text
Core:
Rust

Storage:
SQLite

Search:
FTS5 / Tantivy

Graph:
typed edge tables + in-memory adjacency

Parsing:
Tree-sitter

Semantic:
LSP / SCIP adapters

API:
Axum

MCP:
Rust MCP implementation

Plugin:
TypeScript thin adapters where required

Serialization:
Serde

Schema:
JSON Schema

Tracing:
OpenTelemetry

Testing:
cargo test
proptest
insta snapshots
criterion
```

---

# 122. Context Compression Philosophy

Do not use an LLM blindly to summarize everything.

Compression order:

1. Remove duplicate facts.
2. Collapse low-level nodes into validated components.
3. Preserve critical edges.
4. Preserve branches and failure behavior.
5. Preserve ownership and invariants.
6. Remove unrelated detail.
7. Only then use semantic summarization.

The context compiler should preserve structure before prose.

---

# 123. Context Integrity

Every generated statement should optionally carry hidden evidence IDs.

Example internal representation:

```text
Transcript Normalizer writes NormalizedTranscript. [F182,F195]
```

Agent output does not need to display those IDs, but a tool can retrieve them.

This makes every context claim auditable.

---

# 124. Trust Model

System facts have an authority ordering:

```text
current runtime observation
+
current compiler/LSP facts
+
current deterministic extraction
>
declared architecture
>
high-confidence inference
>
historical memory
>
model assumption
```

Conflicts must be surfaced.

---

# 125. Memory and Compaction Strategy

SCC does not become another memory store.

Use:

```text
System IR
    current system truth

Beads
    active/persistent work graph

.agent-state/current-task.json
    exact transient work state

Hindsight
    durable learned experience
```

---

# 126. Checkpoint Schema

```json
{
  "task": {
    "goal": "...",
    "bead": "..."
  },

  "system_ir_revision": "...",

  "affected": {
    "components": [],
    "flows": [],
    "contracts": [],
    "invariants": []
  },

  "files": {
    "modified": [],
    "inspected": []
  },

  "tests": {
    "passed": [],
    "failed": [],
    "not_run": []
  },

  "decisions": [],

  "next_actions": []
}
```

---

# 127. Session Rehydration

After compaction:

```text
checkpoint
+
active Bead
+
fresh system capsule
+
task context
+
relevant Hindsight lessons
```

This should make compaction largely transparent.

---

# 128. Verification Pipeline

Before an agent says a task is complete:

```text
format
lint
typecheck
targeted tests
integration tests where relevant
security checks
System IR impact
contract validation
invariant validation
runtime/browser validation where relevant
```

SCC contributes:

```text
What should be verified?
```

not necessarily:

```text
How every verification tool runs.
```

---

# 129. Failure Modes

## FM1 — Wrong component abstraction

Mitigation:

```text
retain evidence
allow human declarations
confidence labels
drift detection
```

---

## FM2 — Dynamic behavior missing

Mitigation:

```text
runtime traces
framework adapters
explicit intent
OBSERVED/STATIC distinction
```

---

## FM3 — Context too large

Mitigation:

```text
hard token budgets
relevance ranking
progressive fetch
```

---

## FM4 — Context misses critical dependency

Mitigation:

```text
flow membership
impact expansion
ownership
contract relationships
high recall bias for critical entities
```

---

## FM5 — Stale model

Mitigation:

```text
content hashes
Git commit
watchers
staleness flags
fail-closed verification
```

---

## FM6 — LLM-generated false architecture

Mitigation:

```text
INFERRED provenance
evidence requirements
never auto-promote inference
```

---

## FM7 — Too many tools

Mitigation:

Expose six semantic tools.

Keep analyzers behind SCC.

---

# 130. Competitive Positioning

SCC versus code search:

```text
Search:
"Where is this implemented?"

SCC:
"What system behavior does this implementation participate in?"
```

SCC versus LSP:

```text
LSP:
"Where is the definition?"

SCC:
"Why does this symbol matter to this task?"
```

SCC versus GitNexus:

```text
GitNexus:
"Trace relationships and analyze impact."

SCC:
"Here is the relevant system model before you begin."
```

SCC versus Narsil CCG:

```text
CCG:
"Here is structured code intelligence."

SCC:
"Here is structured system semantics derived from code intelligence."
```

SCC versus Tecture:

```text
Tecture:
"Maintain agent-authored architecture documentation."

SCC:
"Continuously compile architecture and behavior from evidence."
```

SCC versus Archify:

```text
Archify:
"Represent the system for a human."

SCC System Atlas:
"Represent the system for another machine."
```

SCC versus Augment:

```text
Augment:
"Retrieve excellent relevant context."

SCC:
"Own an inspectable, evidence-linked System IR and compile context from it."
```

---

# 131. Product Differentiator

The moat is not the graph.

The moat is the chain:

```text
repository reality
       ↓
semantic system model
       ↓
machine behavioral atlas
       ↓
evidence-backed context compiler
       ↓
agent task performance
```

Any individual layer can be copied.

The difficult part is making the abstraction reliable enough that an agent can trust it.

---

# 132. Definition of Success

SCC succeeds when a coding agent can enter an unfamiliar repository and behave more like an engineer who already understands the architecture.

Specifically:

- It opens fewer irrelevant files.
- It runs fewer exploratory searches.
- It misses fewer dependencies.
- It understands system boundaries earlier.
- It changes fewer unrelated files.
- It preserves invariants more reliably.
- It uses fewer context tokens.
- It completes cross-layer changes more successfully.
- It survives compaction without rediscovering the system.
- It can explain which evidence supports its assumptions.

The strongest success test is not:

```text
"Does the graph look impressive?"
```

It is:

```text
"Did the agent make the correct change with less exploration
and fewer missed system consequences?"
```

---

# 133. Final Product Architecture

```text
                        ┌───────────────────────┐
                        │     Source Repos      │
                        └───────────┬───────────┘
                                    │
            ┌───────────────────────┼───────────────────────┐
            │                       │                       │
          Code                  Config/Infra             Runtime
            │                       │                       │
   Tree-sitter/LSP/SCIP      Docker/K8s/TF/etc.       OTel/Sentry/etc.
            │                       │                       │
            └───────────────────────┼───────────────────────┘
                                    │
                                    ▼
                        ┌───────────────────────┐
                        │     Reality Graph     │
                        │                       │
                        │ symbols / calls       │
                        │ types / routes        │
                        │ stores / events       │
                        │ deployment            │
                        │ runtime observations  │
                        └───────────┬───────────┘
                                    │
                                    ▼
                        ┌───────────────────────┐
                        │       System IR       │
                        │                       │
                        │ components            │
                        │ responsibilities      │
                        │ ownership             │
                        │ contracts             │
                        │ boundaries            │
                        │ invariants            │
                        └───────────┬───────────┘
                                    │
                                    ▼
                        ┌───────────────────────┐
                        │     System Atlas      │
                        │                       │
                        │ Architecture          │
                        │ Workflow              │
                        │ Sequence              │
                        │ Data Flow             │
                        │ Lifecycle             │
                        └───────────┬───────────┘
                                    │
                                    ▼
                        ┌───────────────────────┐
                        │   Context Compiler    │
                        └───────────┬───────────┘
                                    │
                ┌───────────────────┼───────────────────┐
                │                   │                   │
             Startup              Task               Impact
             capsule              slice              slice
                │                   │                   │
                └───────────────────┼───────────────────┘
                                    │
                                    ▼
                            ┌───────────────┐
                            │ Coding Agent  │
                            └───────┬───────┘
                                    │
                            precise execution
                                    │
                   ┌────────────────┼─────────────────┐
                   │                │                 │
                Serena         verification         Beads
                exact             tools             state
                code
                                    │
                               Hindsight
                              durable lessons
```

---

# 134. One-Sentence Product Definition

**System Context Compiler continuously compiles code, configuration, infrastructure, runtime evidence, and architectural intent into an evidence-backed machine model of a software system, then gives coding agents exactly the architecture, flows, contracts, ownership, invariants, implementation details, and tests relevant to the task they are performing.**

---

# 135. The Core Principle

The entire project should be evaluated against one principle:

> **Do not give the agent more repository information. Give the agent more repository understanding per token.**