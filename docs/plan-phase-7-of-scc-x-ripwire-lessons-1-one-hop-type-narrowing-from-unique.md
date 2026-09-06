# Type narrowing, JSONL explore protocol, and watcher hash sweep

<!-- trace:v1 id=PLAN-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing-from-unique type=plan work=WORK-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing-from-unique implements=REQ-implement-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing -->

<!-- trace:exempt reason=document-structure -->
## Steps

1. Capture unique per-scope local/parameter type binds from Python and TypeScript extractors (`x = Order()`, `x: Order`, `new Foo()`, typed params). Tombstone a name bound to two types. Do not stamp binds onto FILE attributes.
2. Resolver: for `NamedVariable` receivers, pin `x.m()` to `{Type}.m` when the bind is unique and that method exists locally or on the imported type. Never spray same-name methods; never invent a candidate.
3. `scc bench loop --explore`: deterministic pack-consumer emits JSONL tool events (baseline grep+read, SCC `task_context`+read, Ripwire `--pack-task`+read). Score with bench-agent metrics. `SCC_EXPLORE_AGENT_CMD` is an LLM plug-in. Locator mode stays the default. Missing Ripwire is skipped.
4. If the filesystem watcher cannot start or cannot watch, fall back to content-hash sweep (`stale_paths` → `cmd_index_paths`). Hash remains authority.
5. Do not change production ranking, MCP tool count, context levels, or REQ-resolution-honesty-gauges.
