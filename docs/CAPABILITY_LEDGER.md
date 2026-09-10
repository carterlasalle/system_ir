# SCC Current Capability Ledger

<!-- trace:v1 id=doc.scc-capability-ledger type=document work=WORK-SI-MMMJA4G6 -->

Repository-backed assessment of what System Context Compiler actually does
today. Evidence: code, tests, and observed behavior at HEAD `aa6d79a`
(main) **plus the uncommitted hardening change described herein** (startup
hard caps, atlas production scope, freshness single-scan, benchmark-science
validity fix, temporal/snapshot V2, skeleton depth, stitch directionality).
Documentation and reports are **not** evidence and were not used.
`docs/MISSION_FINAL_REPORT.md` is stamped HISTORICAL and contradicts this
ledger wherever they differ — this ledger wins.

Status meanings: `COMPLETE` (works, tested, no known gaps) ·
`PARTIAL` (works with known gaps) · `BROKEN` (claims more than it does) ·
`MISSING` (absent) · `DEFERRED` (consciously postponed) ·
`NOT WORTH IMPLEMENTING` (rejected).

<!-- trace:v1 id=doc.indexing-extraction-layer work=WORK-SI-MMMJA4G6 documents=REQ-SI-503JSBGP -->
## Indexing / extraction layer (strongest part of the system)

| Capability | Status | Locations | Tests | Observed behavior | Failure modes / action |
|---|---|---|---|---|---|
| indexing (scan→extract→resolve→store) | COMPLETE | `crates/scc-indexer/src/lib.rs:112-230` (index), `:464-547` (refresh_paths), `crates/scc-store/src/lib.rs` | `crates/scc-cli/tests/golden.rs`, `scc-indexer/tests/invalidation.rs` | Cold + incremental indexing works; in-file incremental≡cold tested; extracted source carried forward (no duplicate reads); removal reconciliation purges deleted/newly-ignored files. | Single read per phase; see freshness/performance. |
| scanner | COMPLETE | `crates/scc-indexer/src/scan.rs` (`scan_repo:311-372`, blake3 `hash_bytes:303-306`, `is_ignored:378-399`) | scan unit tests | Authoritative walk+classify+hash; `FileKind` Source/Test/Config/Infra/Docs/Other (`:125-132`, `is_test_path:185`) | Oversized/unreadable/unsupported/ignored/symlink-escape counted in `ScanStats` (every met file lands in exactly one bucket); root canonicalized once per walk. | Keep. |
| language registry | PARTIAL | `scan.rs` (`Language` enum incl. Python/TS/JS/Go/Rust/Java/Json/Yaml/Toml/Csharp/Ruby/Php/Lua/Swift), extractor registry `lib.rs:97-102` | Ruby-counts-as-unparsed test | Registry is broad but extraction is deep only for Python, TypeScript, Go, Java, Rust, C-family; rest count as unsupported | Honest tiers exist implicitly. Action: document tiers, don't chase counts (Part XXXII). |
| language extraction | PARTIAL | `python.rs`, `typescript.rs`, `go`, `java`, `rust`, `cfamily` extractors emitting `facts: Vec<SemanticFact>` | extractor unit tests | Tree-sitter extractors never fail; symbols/imports/calls/routes emitted | Long tail (C++ templates, macros, FFI) missing. Action: defer to P3. |
| receiver resolution | COMPLETE | `crates/scc-indexer/src/resolve.rs` (`resolve_calls`), Ripwire-absorbed rules (self/this/super, field narrowing, unprefixed-field-as-receiver) | extensive resolve unit tests | Precise receiver-aware resolution is the proven core; do not regress | Keep. Precision gauge wiring needed (see honesty), not resolver changes. |
| type narrowing | PARTIAL | one-hop narrowing from unique constructors/assignments (Ripwire phases) | narrowing tests | Single-hop only | Documented limit; deeper inference deferred. |
| inheritance | PARTIAL | base-class/super handling in resolve.rs | super-call tests | `super().x` resolves; full MRO not modeled | Acceptable; record as known limit. |
| cross-file resolution | COMPLETE | import binding + index (`SymbolIndex`), path-style rules (dots→slashes, `mod.py`/`__init__.py` probes) | cross-file tests | Reliable within repo | Same-name veto rules exist (Rule 3). Keep. |
| LSP resolution | PARTIAL | `lsp.rs`, `lsp_ts.rs` overlays writing `Relationship.confidence via with_confidence` | overlay tests | Works as confidence overlay; does NOT feed the precise gauge (`quality_from_calls` is native-only, `resolve.rs:1437-1442`) | Wire LSP-exact into precise gauge or document. See §13. |
| confidence | COMPLETE | literals scattered across `resolve.rs` (30 emit sites), overlays (`0.99/0.8`), `Provenance::default_confidence` (`scc-core/src/lib.rs:77-85`, bypassed by native resolver) | — | Named confidence classes, explicitly ordinal (not calibrated probabilities); single precision gauge fed by `is_precise()`. | Keep. |

<!-- trace:v1 id=doc.graph-semantic-layer work=WORK-SI-MMMJA4G6 documents=REQ-SI-503JSBGP -->
## Graph / semantic layer

| Capability | Status | Locations | Tests | Observed behavior | Failure modes / action |
|---|---|---|---|---|---|
| Reality Graph | PARTIAL | `crates/scc-graph` (`RealityGraph`, `state.rs`, `invariants.rs:77-302`) | invariant tests | Compiles entities/relationships/evidence; drift findings (unenforced invariants, missing components, conflicting writers) | Dangling refs from postpass IDs possible (see §21). |
| TrustedGraphView | PARTIAL | `trust.rs` (`allows:52`, claim filtering `:311-350` strips low-confidence INFERRED below 0.85 floor from `scc-context/src/lib.rs:170-180`) | trust tests | Derived claims gated; policy-consistent | Floor is config (`include_low_confidence_inference`). Keep. |
| provenance | PARTIAL | `Provenance` enum (Extracted/Resolved/Observed/Declared/Inferred/Stale) in `scc-core` | provenance tests | Internal evidence carries provenance+confidence correctly | Lost at render boundary for some facts (see §8). |
| System Atlas | PARTIAL | `crates/scc-context/src/atlas.rs` (assembly, entrypoints deduped by `(name,kind):251`, `project_flow_graph:1894-1975`) | atlas tests, `scc bench atlas` (dev 0.079 / holdout 0.033) | 7-layer recall benchmark exists with precision + density (v3); gate 0.5 on 5 startup layers | COMPILER is dominant gap per `--diagnose`. Atlas budget can silently overflow (see budgets). |
| components | PARTIAL | `components.rs` (5 evidence classes, boundary ranking), `clustering.rs` (`build_regions:165`, `cluster_components:346`, `compile_services:1174`) | clustering tests | Repository roles (`component_role`: production/test/fixture/benchmark/example/docs/config/generated/vendor/build/tooling/sdk/mixed) label every component; empty fallback responsibilities removed (silence over boilerplate); entrypoint routes deduped by method+path+handler. | Cohesion/manifest-aware boundaries still heuristic; production crates can still fragment. |
| entrypoints | COMPLETE | `atlas.rs` (exact method+path+handler dedup; production scope: `scoped_entities`) | atlas scope test (fixture `GET /admin` excluded, production listed) | Global entrypoints are production-evidence only. | Keep. |
| contracts | PARTIAL | typed contracts (http/cli/event/config/public-api) + stitch directionality (`system.rs`: routes `MatchingContract`, topics Exact only on cross-member pub↔sub, per-end roles) | contract tests + stitch unit/e2e tests | Fixture routes/topics no longer pollute; HTTP client-call extraction deferred (no `HTTP_CLIENT_CALL` entities yet — documented, not faked). | Client-call evidence is the remaining gap. |
| state authority | COMPLETE | `state.rs` + atlas production scope (non-production components' owns stripped; DATA STORES rebuilt from production writers; state lines filtered by leading component) | state tests + atlas scope test | Benchmark/fixture writers cannot own production state. | Keep. |
| FlowGraph | COMPLETE | `flowgraph.rs`; unsupported kinds (Fallback/Timeout/Compensation/…) explicitly `DEFERRED` in code, never emitted | flow tests | `Read`/`Write` edges from state access; flows actor-scoped in atlas (fixture-only choreography excluded). | Keep. |
| deployment | PARTIAL | docker-compose evidence class in components; deployment manifests | — | Basic | Cross-repo deploy stitching missing (P2). |
| trust boundaries | COMPLETE | `boundaries.rs` (`boundary_crossings` diagnostic + `production_crossings` subject-side filtered for atlas scope) | boundary tests | Benchmark/fixture crossings no longer pollute architecture; verify keeps the unfiltered diagnostic. | Keep. |
| runtime evidence | PARTIAL | runtime traces (`/v1/runtime/traces`), `Observed` provenance kept separate | runtime tests | Static vs observed preserved separately | OTel etc. deferred; fine. |
| Surface Map | COMPLETE | `surface.rs` (`build_surface`, staged pipeline, token-aware quotas, hard max + structural compression) | `token_hard_max.rs` (hard max enforced incl. huge-signature fixture) | The bounded pack that works: final render ≤ hard_max tested | Keep as reference pattern for startup/atlas. |
| task retrieval / ranking | PARTIAL | `rank.rs` (heterogeneous PPR + lexical, `rank_startup_atlas` precision-via-ordering), `pagerank.rs` (test/generated/vendored penalties `:963-1003` — ranking-only) | rank tests | Relevance-first vs PPR tradeoff known; rank penalties exist but don't fix component roles | Cheap-benchmark ranking study only (Part XIII). |
| Structural Source / exact source | COMPLETE | `structural_source.rs` (`choose_representation` exact/structural/signatures) | structural tests | Exact-source dominance preserved; representation policy exists | Keep. No L4. |
| source handles | PARTIAL | `ContentHandle`, `fnv1a64_hex`, `entity_id:1074`/`symbol_id:1139`/`relationship_id:1157` constructors in `scc-core` | round-trip tests? | Canonical constructors exist BUT parallel schemes in `scc-indexer/src/write.rs` (`rel_id` blake3, content-hash `evidence_id` name-collide with `scc_core::evidence_id`) | Centralize; audit small-hash collision claims (§22, §24). |
| context budgets | COMPLETE | `ContextBudget`, `allocate_startup_budget` (startup.rs — single allocator, transport parity), per-pack budgets | `token_hard_max.rs` | Every agent-facing pack fits its budget by measurement: `render()` hard phase (halve bodies → drop recorded → line-truncate last) + `detail_tokens` (6000) for component/flow/impact/verify + startup emergency floor (atlas essentials + skeleton + receipt, no overflow hatch). One estimator (`scc_core::estimate_tokens`, chars/4). | `render_hard_cap_always_fits`, `--budget 200` fit test, atlas tight-budget test. |
| ContextLedger | COMPLETE | `context_ledger.rs` (visible-ids from same render, novelty suppression) | `ledger_freshness.rs` (§54 post-mutation freshness: edits+deletions) | Answers what was rendered; semantics must not weaken | Keep. Snapshot is separate (P2). |
| startup capsule | COMPLETE | `startup.rs` (`build_startup` atlas+surface fusion, deterministic per epoch, rank cache) | hard-max test | Deterministic, cached, genuinely hard-capped (emergency floor; ledger coupled via `atlas_budget_used`); skeleton (role-labeled, own budget, priority depth-3) in every transport; durable Codex/OpenCode capsule carries overview + skeleton. | Keep. |
| MCP | COMPLETE | `mcp.rs` (10 tools: system_overview/atlas/task/component/flow/impact/verify/system_context/surface_map/structural_source) | MCP tests | Small surface; hard budgets on all packs; `system_atlas` takes `scope: production|full`; read-only/idempotent annotations asserted in schema test. | Keep. |
| OMP / agent setup | COMPLETE | `plugin_omp.rs`, `plugin.rs` (SessionStart/UserPromptSubmit/PostToolUse/PreCompact), `compress.rs` (codex/opencode/hermes setup) | setup idempotence tests | Deepest lifecycle integration is OMP; setup idempotent; capsule carries skeleton; `checkpoint save` pins a ContextSnapshot and `load` reports still-valid/invalidated/modified (semantic rehydration in PreCompact path). | Cursor/VSCode/Gemini deferred (P3). |
| incremental refresh | COMPLETE | `refresh_paths`, daemon `refresh_stale_by_hash` (`httpd.rs:300-323`, scan-diff, added-file aware), watcher notify `:327-387` | `golden.rs:349-382` (incremental≡cold), `invalidation.rs` (modified + added callee cascade) | Single-read phases; scan-diff freshness (modified/deleted/added); removal reconciliation purges. | Keep. |
| multi-repo | COMPLETE (hardened) | `scc-core/src/identity.rs` (explicit → sanitized remote → basename; credential stripping), `scc-store/src/system.rs` (`System::open`, `stitch_routes/topics/package_exports`, Exact/Declared/Inferred/Ambiguous), `Store::open` persistent-id + `adopt_stable_identity`, `Config.repository.id`, `scc system` CLI | store unit tests (5 stitch/match tests) + identity tests (5) + `golden.rs:multi_repo_system_stitches_contracts_topics_and_packages` (real fixtures: GET /health MatchingContract/Server ends, orders.created Exact Publisher→Subscriber, Declared package, /health ambiguous, move survival, incremental≡cold) | Checkout moves keep ids; standalone-vs-system ids byte-identical; stitches read-only | Remote-derived id only adopts on fresh DBs (later `git remote add` does not rewrite load-bearing ids — documented). Directional verdicts: server+server routes are `MatchingContract` (never Exact without client-call evidence); topics Exact only on cross-member pub↔sub with per-end roles; HTTP client-call extraction deferred (documented, not faked). |
| temporal graph | COMPLETE (hardened) | `scc-store/src/history.rs` (`GraphRevision`, `SemanticDelta`, `record_revision`/`record_current_revision` (transactional, content-deduped), `revision_members`, `semantic_diff`), migration v7, auto-recorded at index, `scc history` / `scc diff` CLI | store history test (intro/removal/diff/idempotence) + `golden.rs:graph_history_records_and_snapshots_diff` | Full row-sets per revision; V2: `semantic_config_hash` + `graph_content_hash` (extractor/config/confidence/provenance changes advance history without file moves; 4-axis dedup); `semantic_diff` reports modified entities/relationships + per-kind counts (same-id content change visible). Snapshots suffice — no event sourcing. | Storage-heavy on huge repos (noted). |
| ContextSnapshot | COMPLETE (hardened) | `scc-store/src/snapshot.rs` (`ContextSnapshot`, `save_snapshot` (idempotent), `load_snapshot`, `diff_snapshot` still-valid/invalidated), `scc snapshot save/show/diff` CLI (save renders the real task pack) | store snapshot test (persist/diff/survival/unknown-id) + golden e2e (save/show/diff/invalidation) | Ledger untouched; newly-relevant knowledge needs re-render (diff does not invent it). V2: entity/rel/contract/state fingerprints + artifact hash — diff reports still-valid/invalidated/modified + changed rels/contracts/state/flows + artifact drift. OMP: `checkpoint save` pins a snapshot; `load` renders the validity verdict (semantic rehydration). |

<!-- trace:v1 id=doc.freshness-honesty-p0 work=WORK-SI-MMMJA4G6 documents=REQ-SI-503JSBGP -->
## Freshness / honesty (P0 — confirmed broken)

| Capability | Status | Locations | Tests | Observed behavior | Failure modes / action |
|---|---|---|---|---|---|
| freshness (CLI) | COMPLETE | `stale_paths` is scan-diff (unchanged/modified/deleted/added); freshness classes CURRENT/STALE in verify output | add/modify/delete verify tests + `deleted_symbol_never_haunts_task_packs` (exact-source exclusion) | Added files make the model stale. Single-scan diff (one read+hash per file per check, canonicalized root hoisted); newly-ignored files converge via removal reconciliation; warm `scc verify` ~0.7s on the self repo (617 files). Watcher dirty-set + metadata fast path deferred (no daemon owns it). Known residual: flow/surface entries still name stale symbols; exact-source exclusion is the enforced invariant. |
| `precise` gauge | COMPLETE | `resolve.rs:1437-1442` (`quality_from_calls` calls `record_call(c.class, false)` — literal `false`); zero `record_call(*, true)` repo-wide | unit test only (`resolution.rs:272-283` passes booleans directly) | `ResolvedCall::is_precise()` (exact import binding, LSP definition, namespace-qualified, exact receiver+type) feeds `record_call`; `verify` prints honest precise/heuristic rates. | Keep. |
| dead resolution categories | COMPLETE | `ResolutionClass::AmbiguousInternal` (`resolution.rs:79-87`): zero production emits. `files.partial`, `stale_facts_dropped`: declared+merged+printed, never set. | — | Dead `AmbiguousInternal` removed rather than left permanently zero; no permanently-zero metric exposed. | Keep. |
| resolution coverage honesty | COMPLETE | `AnalysisQuality` struct (`:104`), `CallQuality` (`:119`), `FileQuality` (`:130`), `compact_line` (`:182-195`); surfaces: `scc index` line, `scc status`, task-pack section, `ContextPack` field | — | Counts + rates published (`verify`, index line, task packs); no permanently-zero buckets. | Keep. |
| skipped-file accounting | COMPLETE | `scan.rs:357-359` silently `continue`; no counter anywhere | — | Discovered/indexed/ignored/unsupported/oversized/unreadable/symlink-escape counted in `ScanStats`, surfaced in `scc status`/`verify`/analysis quality. | Keep. |
| one token estimator | COMPLETE | canonical `scc_core::estimate_tokens` = `chars/4 ceil` (`scc-core/src/lib.rs:1305-1308`); DIVERGENT `main.rs:1242-1247` (`bytes/4`), `external_bench.rs` test + `run_write_matrix.py:164` + `run_context_bench.py:390` (`len//4` bytes) | estimator tests? | One canonical `scc_core::estimate_tokens` (chars/4, conservative); harness + adapters + Rust share the rule; hard caps measured with it, never promised past it. | Keep. |

<!-- trace:v1 id=doc.benchmarking-security-traceability work=WORK-SI-MMMJA4G6 documents=REQ-SI-503JSBGP -->
## Benchmarking / evaluation / security / traceability / performance

| Capability | Status | Locations | Tests | Observed behavior | Failure modes / action |
|---|---|---|---|---|---|
| benchmarking (deterministic) | COMPLETE | `benchatlas.rs` (v2 structured recall + v3 precision/density/F2, gate 0.5), `run_context_bench.py`, `benchmarks/tasks.json`, `benchmarks/ground-truth/` + holdout ground truth | `test_harness.py`, `test_evaluators.py`, evaluator four-way contract tests | Local benchmarks run with no model quota. `bench_science.py` honors its own validity model (`.invalid.json` twins AND `validity.valid == false` excluded from every claim); floor/ceiling pooled per corpus hash, never across; lineage section lists valid files per corpus. Deterministic: regen is byte-identical. | Recall-only gating documented (no precision/MRR gate exists). |
| paid benchmarking + interlock | COMPLETE | `run_write_matrix.py` (`check_paid_opt_in`, `--dry-run`, `launch_agent` flag), `run_context_bench.py` (env gate), `benchagent.rs` (`paid_opt_in_allowed`/`require_paid_opt_in` + gates in `run_agent_benchmark`/`run_variant_benchmark`), `benchloop.rs` (gate when `agent_cmd.is_some()`), `main.rs` (gate on non-artifact `bench external` path) | `PaidBenchmarkGateTest` (refuse/dry-run/opt-in/summary-exclusion), patched `test_harness.py` native-mode tests, 3 new + 1 benchloop Rust gate tests, 4 mock tests wrapped in `with_paid_opt_in` | Normal invocation exits 2 before any external launch; `--dry-run` builds artifacts only (`DRY-RUN` cells excluded from rates); `SCC_ALLOW_PAID_BENCHMARKS=1` opts in; preflight verifies current `tasks.json` hash against valid manifest corpora and dry-builds the PROPOSED caps asking will-THIS-cap-bind (historical binding never refuses a future experiment). | Keep. Zero quota spent. |
| blind evaluation | PARTIAL | `benchmarks/blind-test/` (20 repos), `blind-lock.json`, `holdout/` ground truth, `holdout-v*.txt` + `blind-v*.txt` result lineage in `benchmarks/results/` | blind protocol tests? | Frozen corpus + lock exist; aggregate result files present | Lineage (which commit/config produced which report) unverified; blind protocol vs quality must be distinguished (§42). No individual blind misses inspected (per mission). |
| security: secret handling | COMPLETE (current mission) | placebo `SecurityConfig.redact_secrets` REMOVED (field + default arm + example + test fixture); structural protection (references-only) documented in `config.example.yaml` + SECURITY.md | config round-trip + parses-example tests green; old configs still parse (serde ignores the key) | Remote listen: `httpd.rs` refuses non-loopback binds without `SCC_ALLOW_REMOTE_LISTEN=1` (fail-closed; no auth exists — documented, TLS/auth deferred). |
| security: remote / loopback | PARTIAL | `httpd.rs:1-7` (loopback default `config.security.listen`), `embed.rs:24-35` (`is_remote` treats loopback/`0.0.0.0` as local), `embed_cli.rs:111-115,192-204` (remote inference fails closed unless `allow_remote_models`) | `remote_policy_fails_closed`, `is_remote` unit tests | Loopback default good; remote models fail closed; HTTP daemon documents read-only-except-index/traces | Non-loopback `listen` has no auth/token contract; Docker docs must match behavior (§30). |
| traceability | PARTIAL (scoped by design) | `.trace/` (1124 nodes, 1645 declared edges, 0 broken refs), policy `standard/lifecycle=wip`, exclusions cover `benchmarks/external/**`, `fixtures/**`, large indexer files | `trace verify --changed` gate | NARROWED GUARANTEE (option A, decided): TraceLayer tracks SELECTED requirement boundaries (core context/graph/store paths) — not all behavioral implementation. Indexer internals, benchmark harness, fixtures, most CLI integration tests, and harness skill syncs are excluded by policy (`.trace/policy.toml [exclusions]`); the core resolver/indexer paths are intentionally NOT fully traced. Do not read `trace verify` as full-coverage proof. | Policy-owned; this ledger states the guarantee, the policy enforces it. |
| performance | PARTIAL | release-profile builds, rank cache per epoch (`startup.rs`), cochange cache by git HEAD + 500-file cap, sweep_orphan set-based (never O(n²) LIKE) | — | Fixed this pass: quality map persisted once per phase (was O(n²) per file); freshness single-scan (was scan + N re-reads); root canonicalized once per walk. Warm `scc verify` ~0.7s on the self repo (617 files, debug build). | Release-profile hot-path numbers still unpublished; no daemon exists (watcher fast path deferred). |
| graph integrity (cold≡verify) | PARTIAL | `verify()` (`packs.rs:1585-1795`: dangling check `:1637-1667`, unenforced `:1677-1683`, drift `:1696-1702`), `check-invariants` | golden verify tests | Verification exists and is strict | Unproven on fresh cold index for all fixtures; malformed postpass IDs suspected. Action: add `cold_index → verify_graph → zero invalid dangling` test (§21). |
| causal completeness | COMPLETE | `FlowEdgeKind` (19 variants), state authority, retry annotations (`maybe_retry`, `@Retryable`/`@Retry`, celery tasks, `Scheduled`/`RabbitListener`/`KafkaListener` facts in java extractor) | retry/fact tests | Failure/retry evidence extracted where annotations exist | Unsupported `FlowEdgeKind` variants explicitly `DEFERRED` in code (documented, never emitted — SCC's philosophy over fake detection); state↔flow joined via Read/Write edges. | Keep. |
| determinism | PARTIAL | epoch-keyed caches, deterministic ordering in extractors (`into_extracted`), sorted rels in atlas | determinism tests? | Mostly deterministic; rank cache per epoch | Audit HashMap iteration, filesystem ordering, parallel processing, tie-breaks (§35). |
| failure/adversarial coverage | PARTIAL | invalidation tests, ledger freshness §54, pin-mismatch/infra cell typing in harness | — | Good harness-level error typing; fixture-level cascade tests | Mission list (§37) mostly uncovered: non-UTF8, unreadable, same-size mods, moved repos, corrupt snapshots. Action: add representative cases. |

<!-- trace:v1 id=doc.scc-capability-ledger.pre-existing work=WORK-SI-MMMJA4G6 documents=REQ-SI-503JSBGP -->
## Pre-existing capabilities (acknowledged, not claimed)

The following are prior-mission achievements preserved by this mission, not
new work: receiver-aware resolver + Ripwire lesson absorption (type
narrowing, field-as-receiver, import-target discipline); provenance and
trust modeling; `RealityGraph` + `TrustedGraphView`; System Atlas
compilation with 7-layer ground truth; state authority; contracts;
causal-flow machinery; task/global ranking with PPR ablations; Structural
Source + exact-source fetching; ContextLedger; runtime evidence separation;
incremental indexing; OMP/MCP integrations; `build_task_pack` transport
parity; evaluator four-way contract + infra-cell error typing; blind corpus
with lock file.

<!-- trace:v1 id=doc.scc-capability-ledger.bugs-confirmed work=WORK-SI-MMMJA4G6 documents=REQ-SI-503JSBGP -->
## Bugs confirmed during this mission (review-claim verdicts)

| Claim | Verdict | Evidence |
|---|---|---|
| Freshness misses added files | CONFIRMED | `stale_paths` iterates `store.all_files()` only (`scc-cli/src/lib.rs:115-130`) |
| `precise` never fires | CONFIRMED | literal `false` in `quality_from_calls` (`resolve.rs:1440`); zero `record_call(*,true)` repo-wide |
| Dead `AmbiguousInternal` / `files.partial` / `stale_facts_dropped` | CONFIRMED | grep: no production emit/set sites |
| Skipped files uncounted | CONFIRMED | bare `continue` in `scan.rs:357-359` |
| Divergent token estimators | CONFIRMED | `chars/4` vs `bytes/4` vs `len//4` across 5 sites |
| Fixture pollution of architecture | CONFIRMED | no role concept; fallback string at `components.rs:914`; rank penalties only |
| `redact_secrets` placebo | CONFIRMED | zero readers repo-wide; real protection is structural |
| Startup massively exceeds budget | FIXED (hardening) | corrective loop replaced the overflow escape hatch with an emergency floor (atlas essentials + skeleton + receipt); every pack render guarantees fit by measurement; `--budget 200` asserts fit |
| Cold index fails verify | NOT REPRODUCED yet | needs the §21 test to determine |
| bench_science pooled invalid matrices | FIXED (hardening) | `load_matrices` excluded only `.invalid.json` filenames, not `validity.valid == false` — analyzer violated its own manifest (e.g. 48 vs 40 attempts). Now `valid_matrices()` gates every claim; floor/ceiling grouped per corpus hash |
| Route/topic stitches overclaimed Exact | FIXED (hardening) | server+server routes called Exact cross-repo contracts; bare topics called Exact. Now `MatchingContract` without evidenced direction; topics Exact only on cross-member pub↔sub; per-end roles |
| Scope was labels, not semantics | FIXED (hardening) | `[fixture]` labels on components while routes/contracts/state/flows compiled from the global graph. Now `AtlasScope::Production` (default) filters architecture sections; `Full` opt-in (`scc atlas --full`, MCP `scope`) |
| Agent packs unbounded | FIXED (hardening) | `component/flow/impact/verify` rendered with `usize::MAX`. Now `detail_tokens` (6000) + hard render; startup has no overflow hatch |

<!-- trace:v1 id=doc.scc-capability-ledger.deferred work=WORK-SI-MMMJA4G6 documents=REQ-SI-503JSBGP -->
## Deferred / not worth implementing (this mission)

| Item | Disposition | Reason |
|---|---|---|
| C/C++ long tail (function pointers, `compile_commands.json`, templates, macros, FFI) | DEFERRED (P3) | Prevalence/precision tradeoff; no testability story yet |
| Cursor/VS Code/Gemini setup | DEFERRED (P3) | OMP is deepest integration; others on demand |
| OTel/runtime source expansion | DEFERRED | Static/runtime separation exists; no interface justifies more |
| L4 retrieval layer | NOT WORTH IMPLEMENTING | Mission non-goal; internal units suffice |
| Watcher dirty-set + metadata fast path + daemon | DEFERRED | No daemon owns freshness; single-scan reconciliation is correct and fast enough (0.7s warm self-repo) |
| HTTP client-call extraction (`HTTP_CLIENT_CALL`) | DEFERRED | No extractor emits client evidence; route stitches stay `MatchingContract` until it exists |
| Event-sourced graph history | NOT WORTH IMPLEMENTING | Revision row-sets + V2 content hashes suffice; no ArangoDB-style infrastructure |
| Human `--full` unlimited output mode | DEFERRED | Agent/MCP paths are hard-capped; humans share the bounded packs for now |
| ArangoDB / Docker infra / 30 MCP tools / mandatory embeddings | NOT WORTH IMPLEMENTING | Explicit non-goals; absorb principles, not baggage |
| Shallow languages for count | NOT WORTH IMPLEMENTING | Honest tiers over raw counts |
