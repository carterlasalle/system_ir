# Ripwire lessons Phase 6 — system moat (state, runtime, impact, bridges)

<!-- trace:v1 id=doc.spec-work-ripwire-lessons-phase6 type=document work=WORK-ripwire-lessons-phase6 -->

<!-- trace:exempt reason=document-structure -->
## Goal

Deepen SCC's unique semantic system — State Authority, runtime evidence, impact, and cross-language contracts — using Ripwire-grade per-function R/W honesty and forgotten-partner disclosure. Do not replace State Authority with a nonlocal-state linter, do not fabricate CALLS across languages, do not overwrite EXTRACTED history with OBSERVED, and do not add a fifth context level or a new MCP tool.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-state-function-access — Per-function R/W feed State Authority without replacing ownership

<!-- trace:v1 id=REQ-state-function-access type=requirement work=WORK-ripwire-lessons-phase6 -->

Store READS/WRITES/QUERIES (and publish/subscribe/consume) already extracted per caller remain the inputs. State Authority keeps component-level ownership (`owns` from writers). It must also emit function-level access lines `{component}::{symbol} {reads|writes|queries|…} {target} ({PROV})` so readers are not owners. A symbol that only reads a persistent store must not appear as `owns`. Cache vs persistent sectioning is unchanged. Provenance is copied, never promoted.

### REQ-observed-call-upgrade — Observed runtime upgrades existing CALLS without rewriting history

<!-- trace:v1 id=REQ-observed-call-upgrade type=requirement work=WORK-ripwire-lessons-phase6 -->

When an ingested runtime edge matches a static CALLS edge at component granularity, SCC must attach `OBSERVED_AS` (provenance OBSERVED) to that subject/object pair. The original CALLS row stays EXTRACTED or RESOLVED — ingest must not delete it, change its provenance, or fabricate a new CALLS edge for observed-only traffic. Synthetic `root` heads and intra-component self-edges are not upgrades. Missing static matches remain drift findings, not invented calls.

### REQ-forgotten-impact-partners — Co-change partners are disclosed, never become semantic impact

<!-- trace:v1 id=REQ-forgotten-impact-partners type=requirement work=WORK-ripwire-lessons-phase6 -->

`impact_context` lists historical co-change partners of the changed files that are not in the current file set, with reason `cochange` and commit count. Those partners must not be inserted into affected components, flows, contracts, or data. The pack discloses them in a lower-priority FORGOTTEN PARTNERS section as historical evidence. EXTRACTED/RESOLVED impact stays authoritative. No co-change yields an omitted section, not a fabricated partner.

### REQ-cross-lang-semantic-bridges — Proto RPC bridges are INVOKES/IMPLEMENTS, never cross-language CALLS

<!-- trace:v1 id=REQ-cross-lang-semantic-bridges type=requirement work=WORK-ripwire-lessons-phase6 -->

`.proto` files are DataConfig (not a semantic language extractor). `service`/`rpc` syntax becomes CONTRACT entities (`kind=rpc`). Name-matched symbols in `server`/`svc` paths IMPLEMENT the contract; name-matched symbols in `client`/`web` paths INVOKES it. Matching is identifier-normalized (camel/snake) on the rpc name. The linker must not emit CALLS from a client-language symbol to a server-language symbol. Classification of protobuf is not a claim of C++/gRPC AST support.
