# Examples
<!-- trace:v1 id=doc.tracelayer.skill-examples -->

Worked examples of correct markers and the full agent workflow. Syntax rules
are in [marker-protocol.md](marker-protocol.md).

## Marker basics

```python
# trace:v1 id=impl.auth.refresh work=AUTH-237 satisfies=REQ-AUTH-017 plan=PLAN-AUTH-237/P3
```

```python
# trace:v1 id=test.auth.refresh-reuse verifies=REQ-AUTH-017 exercises=impl.auth.refresh
```

```markdown
<!-- trace:v1 id=ADR-0042 addresses=REQ-AUTH-017 supersedes=ADR-0021 -->
```

```yaml
# trace:v1 id=ops.auth.redis implements=PLAN-AUTH-237/P4 deploys=impl.auth.refresh
```

Values containing whitespace are double-quoted; backslash escapes `\`, `"`,
`\n`, `\t` inside quoted values. Repeated relations are comma-separated with
no semantic ordering. `title` is optional descriptive metadata, not an edge:

```text
trace:v1 id=doc.auth.rotation documents=REQ-AUTH-017 title="Refresh token rotation operations"
```

## Marker placement

Markers MUST be adjacent to the behavior they define.

Good:

```python
# trace:v1 id=impl.billing.export satisfies=REQ-BILL-031 work=BILL-208
def export_invoices(...):
```

Bad (200 lines between marker and symbol):

```python
# trace:v1 id=impl.billing.export satisfies=REQ-BILL-031

# 200 lines later...
def export_invoices(...):
```

Module/file-level behavior: place the marker at the top of the module. For
Markdown headings, place the marker immediately below the heading unless the
repository style says otherwise.

## Full feature: authentication (spec 50)

### Requirement

```markdown
## REQ-AUTH-017 - Refresh token rotation

<!-- trace:v1 id=REQ-AUTH-017 type=requirement derived_from=PRD-AUTH-002 -->

Whenever a refresh token is exchanged, the previous token must become unusable.
```

### Decision

```markdown
# ADR-0042 - One-time refresh-token families

<!-- trace:v1 id=ADR-0042 type=decision addresses=REQ-AUTH-017 supersedes=ADR-0021 -->

Tokens are organized into families. Rotation revokes the previous token and records the successor.
```

### Work item metadata

`.trace/work.toml`:

```toml
[work."WORK-AUTH-237"]
title = "Implement refresh token rotation"

[work."WORK-AUTH-237".mirrors]
github_issue = "812"
jira = "AUTH-237"
```

### Plan

```markdown
## Phase 3 - Rotation persistence

<!-- trace:v1 id=PLAN-AUTH-237/P3 type=plan work=WORK-AUTH-237 implements=ADR-0042 -->

Persist token-family rotation and rejection of reuse.
```

### Implementation

```python
# trace:v1 id=impl.auth.refresh work=WORK-AUTH-237 satisfies=REQ-AUTH-017 implements=ADR-0042,PLAN-AUTH-237/P3
def rotate_refresh_token(token: str) -> TokenPair: ...
```

The engine derives `path=src/auth/tokens.py`,
`symbol=auth.tokens.rotate_refresh_token`, `lines=83-121`, and
`last_modified=<git sha>` — none of that is written in the marker.

### Test

```python
# trace:v1 id=test.auth.refresh-reuse verifies=REQ-AUTH-017 exercises=impl.auth.refresh
def test_reused_refresh_token_is_rejected():
    first = issue_refresh_token()
    second = rotate_refresh_token(first)
    assert_rejected(first)
    assert_accepted(second.refresh_token)
```

### The agent edit loop

1. User: "Change refresh token reuse behavior to improve replay protection."
2. Prompt hook surfaces `REQ-AUTH-017`, `ADR-0042`, `impl.auth.refresh`.
3. Agent edits `rotate_refresh_token` without loading context → pre-edit hook
   blocks once: `TRACE CONTEXT REQUIRED — Run: trace context impl.auth.refresh`.
4. Agent runs `trace context impl.auth.refresh`, reviews, retries — allowed.
5. Post-edit hook: `impl.auth.refresh semantic hash changed.
   test.auth.refresh-reuse verification is now dirty.`
6. Agent runs linked tests with coverage and ingests evidence.
7. `trace verify --changed --lifecycle merge` → PASS.

### Evidence

```text
test.auth.refresh-reuse -> PASSED at revision a81d41
test.auth.refresh-reuse -> executed -> impl.auth.refresh [proof L2]
```

## After a requirement changes (spec 51)

Initial state:

```text
REQ-GEO-011@fingerprint:A
  <- satisfies - impl.geo.resolver@fingerprint:X
  <- verifies  - test.geo.ridge@fingerprint:T
  evidence run R1 current
```

The spec author changes the requirement semantics → `REQ-GEO-011@fingerprint:B`.

Policy propagation:

```text
impl.geo.resolver      STALE_REVIEW_REQUIRED
test.geo.ridge         STALE_REVIEW_REQUIRED
R1 evidence            HISTORICAL_NOT_CURRENT
```

The code might still be correct. TraceLayer does not claim it is wrong; it
claims the old proof no longer establishes conformance to the new
requirement. After review, `trace review <id>` records the acknowledgement
and requires fresh verification before `CURRENT` under strict policy.
