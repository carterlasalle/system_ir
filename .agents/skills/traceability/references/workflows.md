# Agent Workflows
<!-- trace:v1 id=doc.tracelayer.workflows -->

TraceLayer as a working tool — exploring, debugging, and developing — not
just marker duty. Every recipe names the CLI form; MCP twins are in
[mcp.md](mcp.md).

<!-- trace:v1 id=doc.tracelayer.workflows.exploring work=WORK-p0-remediation-passive-activation-remind-first-enforcement-branch-safe-identity-safe-bootstrap -->
### Exploring

- **Orient in a new repo:** `trace status`, then `trace search <topic>`,
  then `trace context <id>` on the hits. Context loads count toward the
  pre-edit gate, so orienting first means fewer interruptions later.
- **Blast radius before editing shared behavior:** `trace impact <id>`,
  then `trace graph <id> --format json` for the machine-readable
  neighborhood. Read callers before changing signatures.
- **Bug → requirement:** `trace why <impl-id>` climbs the causal path to
  the root; `trace context <req-id>` loads the requirement's staleness and
  provenance. Fix the requirement misunderstanding, not just the symptom.
- **Knowledge-first edit:** `trace knowledge --for <artifact>` plus
  `trace facts` before tricky edits — the repo's hard-won lessons and
  canonical values, not just text search.
- **Design review export:** `trace graph <id> --format mermaid` (or `dot`,
  `jsonl`) for review artifacts; `--depth 2` tree for terminal skims.

<!-- trace:v1 id=doc.tracelayer.workflows.debugging work=WORK-p0-remediation-passive-activation-remind-first-enforcement-branch-safe-identity-safe-bootstrap -->
### Debugging

- **Triage first:** `trace doctor` lists what is actually wrong;
  `trace review <id>` acknowledges reviewed staleness; re-run evidence,
  then `trace verify --changed`.
- **Facts drift:** `trace facts <id>` shows one canonical value, its
  source, and every dependent; fix the source, not the copies.
- **Obligations:** `trace task context` lists what is pending;
  `trace task resolve-obligation <path> --symbol <sym>` clears entries
  whose file is gone (moot) or confirms deliberate ones. Deleted paths
  re-arm automatically if the file comes back — no evasion.
- **Exclusions:** generated/vendor trees belong in policy (`trace ignore
  --extension <ext>`, `--file`, `--directory`, `--glob`), then
  `trace index --changed` and re-verify. Never mark generated code.
- **Identity collisions:** `trace ids collisions` detects duplicate IDs;
  `trace doctor --names` plus `trace tidy names` / `trace refactor ids
  --plan` / `--apply` repair them. Never copy an existing ID onto
  unrelated behavior.
- **Stale session state:** entries for deleted files resolve as moot via
  `resolve-obligation`; no restore-the-file dances.

<!-- trace:v1 id=doc.tracelayer.workflows.developing work=WORK-p0-remediation-passive-activation-remind-first-enforcement-branch-safe-identity-safe-bootstrap -->
### Developing

- **Requirement-first build:** `trace plan suggest "<intent>"` scopes the
  work (tiny/small/medium/large), `trace task bootstrap --json` (or
  `--file`/`--stdin`) authors WORK + requirements + spec + plan + tasks
  atomically, `trace task begin <WORK-ID>` binds the session. `--prompt`
  only writes DRAFT scaffolding — implementation stays blocked until a
  real bundle lands.
- **Session lifecycle:** `begin` binds WORK, `activate` switches,
  `task context` shows where you are, `task state` moves TASKs,
  `finish`/`end` clears. One active WORK per session.
- **Questions that block:** `trace question add "<text>" --work <id>`
  persists material unknowns; answer them before the work they block.
- **Syncing external TODOs:** `trace work sync-todos --harness <name>`
  persists harness TODOs as TASKs (the post-mutation hook only syncs
  TodoWrite-bearing payloads automatically).
- **Evidence:** `trace evidence ingest` proves linked tests pass (TL021); the verifies= edge itself comes from a `verifies=REQ-...` test marker (markers work in test files even though tests are excluded from auto-obligations);
  `trace verify --changed` is the gate before completion, every time.
