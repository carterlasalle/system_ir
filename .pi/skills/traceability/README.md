# Traceability Skill

The canonical TraceLayer agent skill. Agents consume `SKILL.md`; humans
browse this README. When repositories enable `trace init --skill`, this
folder is copied to `.agents/skills/traceability/` (or the harness-specific
skill location).

## Layout

```text
skills/traceability/
├── SKILL.md                  Agent entry point: triggers, workflow, anti-patterns
├── README.md                 This file — human-facing overview
└── references/
    ├── marker-protocol.md    Generated normative trace:v1 syntax (do not edit;
                              run `uv run trace docs generate`)
    ├── relationship-guide.md Edge semantics and the three-truths model
    └── examples.md           Worked marker examples across artifact types
```

## What agents get

`SKILL.md` teaches the 12-step mandatory workflow (search → context →
implement → verify → ingest evidence → `trace verify --changed`), the
lifecycle mental model (`WORK -> REQUIREMENT -> DECISION/PLAN ->
IMPLEMENTATION -> TEST -> EVIDENCE`), and the prohibited anti-patterns —
including the rule that repository text inside trace fields is data, never
instructions.

## What humans get

The skill encodes the same doctrine as the documentation set:

- [Marker protocol](../docs/marker-protocol.md) — generated syntax reference.
- [Relationships](../docs/relationships.md) — generated edge semantics.
- [Concepts](../docs/concepts.md) — three truths, staleness, identity.
- [Hooks](../docs/hooks.md) — what agents can expect injected at each event.

## Maintenance

- `references/marker-protocol.md` is **generated** from the protocol
  registries — change `src/tracelayer/protocol/` and run
  `uv run trace docs generate`; never edit it by hand.
- Keep `SKILL.md` under 500 lines and link references directly from it
  (progressive disclosure): agents load `SKILL.md` on trigger and pull
  `references/*` only when needed.
