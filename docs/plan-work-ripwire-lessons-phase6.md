# PLAN-ripwire-lessons-phase6

<!-- trace:v1 id=PLAN-ripwire-lessons-phase6 type=plan work=WORK-ripwire-lessons-phase6 implements=REQ-state-function-access,REQ-observed-call-upgrade,REQ-forgotten-impact-partners,REQ-cross-lang-semantic-bridges -->

<!-- trace:exempt reason=document-structure -->
## Steps

1. Attribute per-function store access into State Authority sections; keep component `owns` write-derived; readers are not owners.
2. On runtime ingest/reconcile, attach OBSERVED_AS to matching CALLS; never mutate CALLS provenance; never invent CALLS for observed-only edges.
3. Disclose forgotten co-change partners on impact packs without merging them into semantic impact sets.
4. Classify `.proto` as DataConfig, extract rpc CONTRACTs, link INVOKES/IMPLEMENTS by path role + name, fixture-test no cross-language CALLS.
5. Keep production ranking, 10 MCP tools, four context levels, and incremental≡cold.
