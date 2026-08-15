<!-- GENERATED from protocol registries. Do not edit by hand; run `trace docs generate`. -->
# Marker Protocol

The canonical marker format is a single line, grep-friendly via `rg 'trace:v1'`:

```text
trace:v1 <key>=<value> <key>=<value> ...
```

Node-defining markers require `id=<trace-id>`.

## Value encoding

- Unquoted values may contain `[A-Za-z0-9._:/#@,+-]`.
- Values containing whitespace MUST use double quotes.
- Backslash escapes `\`, `"`, `\n`, `\t` inside quoted values.
- Repeated relations use comma-separated target IDs with no semantic
  ordering unless the relation specifies one.
- Empty values are invalid in canonical v1.
- Duplicate keys on one marker are invalid.

## Built-in properties

- `id` — stable trace identity (required).
- `type` — optional artifact type; inferred from the ID namespace when absent.
- `title` — optional descriptive metadata; not a graph edge.
- `policy` — rare policy override reference, not an arbitrary exemption.
- `expects` — plan-only: comma-separated artifact IDs the plan commits
  to producing; TL014 enforces each exists and links back via `implements`.

## Convenience keys

- `work` — a `work` edge (comma-separated targets).
- `plan` — alias for the `implements` edge (comma-separated targets).

Everything representing another artifact is an edge, not a generic path field.

## Canonical examples

```python
# trace:v1 id=impl.auth.refresh work=AUTH-237 satisfies=REQ-AUTH-017 plan=PLAN-AUTH-237/P3
def rotate_refresh_token(...):
    ...
```

```python
# trace:v1 id=test.auth.refresh-reuse verifies=REQ-AUTH-017 exercises=impl.auth.refresh
def test_reused_refresh_token_is_rejected(): ...
```

```markdown
<!-- trace:v1 id=ADR-0042 addresses=REQ-AUTH-017 supersedes=ADR-0021 -->
```

## Marker placement

Markers MUST be adjacent to the behavior/artifact they define. Create
markers at meaningful behavior boundaries (public API endpoints, business
rules, security boundaries, contractual config, verification tests). Do
NOT trace imports, trivial getters/setters, local loops, formatting
changes, or generated code.

# Artifact Types

| Type | Category | Description |
|---|---|---|
| `ci_run` | provenance | A CI run. |
| `commit` | provenance | A Git commit. |
| `config` | realization | Configuration with contractual significance. |
| `data` | realization | Data schema or dataset artifact. |
| `decision` | decision/planning | An architecture decision record (ADR) or equivalent decision. |
| `document` | verification/documentation | Human-facing documentation artifact. |
| `evidence` | verification/documentation | Immutable runtime/CI evidence record. |
| `external` | provenance | An external system record (Jira, Linear, Notion, ...). |
| `goal` | intent | Top-level business or product goal. |
| `implementation` | realization | Source code realizing a requirement/decision. |
| `nfr` | intent | A non-functional requirement. |
| `operation` | realization | Deployment, runbook-adjacent operational behavior. |
| `plan` | decision/planning | A plan or plan step; first-class ID (PLAN-X/P3). |
| `prd` | intent | Product requirements document. |
| `prompt` | realization | Prompt or configuration encoding a product invariant. |
| `pull_request` | provenance | A pull/merge request. |
| `requirement` | intent | A formal, stable, testable requirement. |
| `runbook` | verification/documentation | Operational runbook procedure. |
| `test` | verification/documentation | A verification test. |
| `work` | decision/planning | A work item (issue, ticket, task) that produced artifacts. |

## Derived facts are never declared

Paths, line numbers, commit SHAs, authors, and test results are derived
by the engine. Declaring structural (`calls`, ...) or observed
(`executed`, `passed`, ...) relationships in a marker is an error.
