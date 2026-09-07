# Ripwire lessons Phase 5 — agent-loop vs baseline and Ripwire

<!-- trace:v1 id=doc.spec-work-ripwire-lessons-phase5 type=document work=WORK-ripwire-lessons-phase5 -->

<!-- trace:exempt reason=document-structure -->
## Goal

Measure whether SCC Task Context reduces search versus a lexical baseline **and** versus Ripwire's `--pack-task` bundle on the same gold tasks. This is a locator loop (pack then open named files), not a claim that an LLM was run. MCP stays at 10 tools. Missing Ripwire is reported as skipped, not as a win.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-agent-loop-three-way — Baseline vs SCC vs Ripwire with clustered stats

<!-- trace:v1 id=REQ-agent-loop-three-way type=requirement work=WORK-ripwire-lessons-phase5 -->

`scc bench loop` runs three arms over `benchmarks/tasks.json`:

- `baseline` — subtoken overlap over file path+content; no SCC pack; `substitution_rate = 0`.
- `scc` — `task_context` pack; open files the pack names; `substitution_rate = scc_calls / (scc_calls + extra_search)`.
- `ripwire` — black-box `ripwire <root> --pack-task="<goal>"` when a binary is configured; parse `p=` paths from the bundle. If the binary is absent, the arm is `skipped` and must not be scored as SCC beating Ripwire.

Metrics per task: localization (gold files among the first K opened), first-correct rank, search calls, substitution rate. Aggregate with **clustered** means (mean of per-repo means), not a pooled mean that lets one fixture dominate. Contamination: the harness must not inject gold file lists into the goal or environment; record `contaminated` if `SCC_GOLD` is set. Do not change production ranking because this loop looks better.
