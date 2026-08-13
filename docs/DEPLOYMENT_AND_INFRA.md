# Deployment and Infrastructure
<!-- trace:v1 id=REQ-SCC-DEPLOY type=requirement derived_from=PRD-SCC-001 title="Deployment modes, Docker, observability" -->

## 1. Modes

### Local developer
`sccd` + SQLite + local extractors + loopback HTTP/MCP.

### CI
```bash
scc index --ci
scc verify
scc drift
scc impact --diff origin/main...HEAD
```

### Team server
Post-MVP: API, repo workers, queue, Postgres, object storage, auth, optional graph/vector services.

## 2. Local daemon

Rust `sccd`, Unix socket or loopback port, per-repo DB, watcher, bounded worker pool.

## 3. Docker

Read-only repo mount + writable SCC data volume.

## 4. Resource targets

For 250k LOC baseline:
- < 2 GB memory
- index storage < 10% source size excluding optional embeddings/traces

## 5. Observability

OpenTelemetry for indexing, extractors, fact counts, staleness, context latency, tokens, cache, errors. Never export source content by default.

## 6. CI cache

Key by repository tree + SCC schema/extractor versions.

## 7. Releases

Static binaries where possible, container image, thin JS integration packages, signed checksums.

## 8. Migrations

Transactional DB migrations with fixture tests and version metadata.

## 9. Scaling

Do not prematurely build distributed team-server architecture for MVP.
