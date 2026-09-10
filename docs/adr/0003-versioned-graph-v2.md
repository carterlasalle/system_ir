<!-- trace:v1 id=doc.adr-0003 type=document work=WORK-SI-MMMJA4G6 -->
# ADR 0003: Content-hashed temporal graph, record-after-recompile

<!-- trace:exempt reason=document-structure -->
## Context

Revision identity (source inventory only) missed extractor/config/
confidence changes, and recording ran before recompile, leaving history
one recompile behind (exposed: identical re-indexes added component
facts the recorded revision lacked). Set-membership diffs were blind to
same-id content change.

<!-- trace:exempt reason=document-structure -->
## Decision

Four-axis identity (source hash, extractor version, semantic config hash,
graph content hash); dedup requires all four. Recording moved out of the
indexer to after recompile in every index path. Diffs report
modified entities/relationships + per-kind counts from recorded row JSON.
Snapshots pin fingerprints (entities, touching rels, contracts, state,
artifact hash) and OMP compaction rehydrates through them
(`checkpoint save`/`load`).

<!-- trace:exempt reason=document-structure -->
## Alternatives considered

- Event sourcing: rejected — revision row-sets + content hashes suffice;
  no new infrastructure for the demonstrated need.
- Order-sensitive hashing (status quo ante): rejected — HashMap-ordered
  evidence vecs made identical re-indexes diverge; evidence vecs are now
  sort+deduped at assembly and the e2e dedup test guards it.

<!-- trace:exempt reason=document-structure -->
## Consequences

- Schema v8 (additive columns); existing DBs migrate on open.
- Enabling a language backend without touching files now advances history
  (correct: the model changed).
