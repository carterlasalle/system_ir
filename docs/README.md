# System Context Compiler (SCC)

**Status:** Product/engineering specification package  
**Core artifact:** System IR  
**Machine system-map layer:** System Atlas

System Context Compiler continuously compiles code, configuration, infrastructure, runtime evidence, tests, and declared architectural intent into an evidence-backed machine model of a software system. It then emits small, task-specific context packs for coding agents.

> **Give agents more repository understanding per token, not more repository text.**

## Product shape

```text
source + config + infra + runtime + intent
                  │
                  ▼
          Reality Compiler
                  │
                  ▼
            Reality Graph
                  │
                  ▼
          System IR Compiler
                  │
                  ▼
             System IR
                  │
                  ▼
            System Atlas
     architecture / workflows /
     sequences / dataflow / lifecycle
                  │
                  ▼
          Context Compiler
                  │
                  ▼
 startup / task / component / flow /
 contract / impact / verification packs
                  │
                  ▼
        coding agent + exact tools
```

## Repository contents

- `docs/PRD.md`
- `docs/SYSTEM_DESIGN.md`
- `docs/SYSTEM_IR_SCHEMA.md`
- `docs/CONTEXT_COMPILER.md`
- `docs/FLOW_COMPILER.md`
- `docs/API_AND_INTEGRATIONS.md`
- `docs/DATA_STRATEGY.md`
- `docs/TEST_PLAN.md`
- `docs/SECURITY.md`
- `docs/DEPLOYMENT_AND_INFRA.md`
- `docs/IMPLEMENTATION_PLAN.md`
- `planning/MILESTONES.md`
- `planning/EPICS_AND_TICKETS.md`
- `schemas/system-ir.schema.json`
- `api/openapi.yaml`
- `examples/config.example.yaml`

## Architectural rule

SCC must not expose every low-level analyzer directly to the agent. GitNexus-, Narsil-, LSP-, SCIP-, CodeQL-, Joern-, and parser-derived facts are evidence inputs. The normal agent interface is seven semantic operations:

1. `system_atlas` — the full startup architecture (primary agent operation)
2. `system_overview` — compact capsule for humans/scripts
3. `task_context`
4. `component_context`
5. `flow_context`
6. `impact_context`
7. `verify_context`
