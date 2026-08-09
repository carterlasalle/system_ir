# Security Policy

SCC is a local-first repository-analysis tool. Its threat model treats
**repository content as untrusted data** that influences high-privilege
coding agents — the full threat model and mitigations are specified in
[docs/SECURITY.md](docs/SECURITY.md).

## Reporting a vulnerability

Please report security issues privately to the maintainer (open a
[private advisory](https://github.com/carterlasalle/system_ir/security/advisories/new)
or email the maintainer directly). Do not open a public issue for
unpatched vulnerabilities.

We aim to acknowledge reports within 48 hours and triage within a week.

## What SCC guarantees

- **Local-first by default.** Source code never leaves the machine. The
  daemon binds loopback only (`security.listen`, default `127.0.0.1:7777`).
  Remote inference and embeddings are opt-in (`inference.enabled`) with
  visible egress.
- **Secrets are never persisted.** Config/env values are reduced to
  references (variable names); values are redacted before storage.
- **Repository text is data, not instructions.** README/docs/comments are
  labeled (`DOCUMENTATION`, `UNTRUSTED TEXT`) in context packs and never
  presented as system facts.
- **Path sandboxing.** Symlink escapes and `..` traversal are rejected.
  The Docker deployment mounts the repository read-only.
- **Adapter capability manifests.** Every evidence adapter declares
  filesystem/network/subprocess/credential use (`scc adapters`); the
  default profile allows no network or credentials and subprocess only
  for declared server adapters (LSP).
- **Provenance honesty.** Inferred and memory-sourced claims are labeled
  and ranked below deterministic evidence; stale facts are excluded from
  trusted context.
- **No telemetry.** No repository content is exported. The benchmark and
  daemon emit only aggregate metrics.

## Scope

The following are outside the security boundary: optional user-configured
remote providers (OpenAI-compatible endpoints, Context7 servers — you are
responsible for the credentials you configure), third-party adapters you
install, and any model/embedding provider you enable.
