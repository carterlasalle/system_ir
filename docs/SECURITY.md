# Security Specification
<!-- trace:v1 id=REQ-SCC-SEC type=requirement derived_from=PRD-SCC-001 title="Security model: local-first, sandbox, provenance" -->

## 1. Threat model

Repository content is untrusted input that ultimately influences high-privilege coding agents.

Threats:
- prompt injection in docs/comments;
- secrets;
- hostile adapters;
- symlink/path traversal;
- dependency compromise;
- tool poisoning;
- malicious intent files;
- sensitive runtime traces.

## 2. Trust model

```text
repository text = untrusted data
deterministic extractors = trusted code
third-party adapters = constrained trust
LLM inference = untrusted inference
declared intent = intent only
runtime observation = observation only
agent = privileged consumer
```

## 3. Prompt injection

Never convert repository prose into authoritative instructions. Tag raw material as source excerpt, documentation, comment, fixture, or untrusted text.

## 4. Secrets

Scan/redact before persistence. Preserve variable/reference names, not values.

## 5. Filesystem

Canonicalize paths; reject repo escapes; safe symlink handling; read-only repository mount in server/container mode.

## 6. Adapters

Capability manifest:
- FS scope;
- network;
- subprocess;
- credentials.

Unknown adapters should be sandboxed.

## 7. Network

Loopback only in local mode. Remote mode requires TLS/auth/project scoping.

## 8. Remote models

Explicit opt-in, visible egress policy, redaction, never send secrets.

## 9. Runtime traces

Redact auth headers/tokens, support allowlists, aggregate, limit retention.

## 10. MCP

Default SCC MCP is repository read-only. Any mutation tool is a separate permission class.

## 11. Supply chain

Lock dependencies, generate SBOM, signed checksums/releases where possible, optional Agent Scan for plugins.

## 12. Auditability

Record revision, adapter versions, inference model/version, context-pack inputs, evidence IDs.
