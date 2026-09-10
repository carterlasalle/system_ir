# SCC Current Capability Ledger

<!-- trace:v1 id=doc.scc-capability-ledger type=document work=WORK-SI-MMMJA4G6 -->

Repository-backed assessment of what System Context Compiler actually does
today. Evidence: code, tests, and observed behavior at HEAD `cc1b789`
(main). Documentation and reports are **not** evidence and were not used.

Status meanings: `COMPLETE` (works, tested, no known gaps) ·
`PARTIAL` (works with known gaps) · `BROKEN` (claims more than it does) ·
`MISSING` (absent) · `DEFERRED` (consciously postponed) ·
`NOT WORTH IMPLEMENTING` (rejected).

Prior mission work is acknowledged inline as pre-existing; this ledger
claims credit only for fixes made during the current mission (the paid
benchmark interlock).

<!-- trace:v1 id=doc.indexing-extraction-layer work=WORK-SI-MMMJA4G6 documents=REQ-SI-503JSBGP -->
## Indexing / extraction layer (strongest part of the system)

| Capability | Status | Locations | Tests | Observed behavior | Failure modes / action |
|---|---|---|---|---|---|
| indexing (scan→extract→resolve→store) | PARTIAL | `crates/scc-indexer/src/lib.rs:112-230` (index), `:464-547` (refresh_paths), `crates/scc-store/src/lib.rs` | `crates/scc-cli/tests/golden.rs`, `scc-indexer/tests/invalidation.rs` | Cold + incremental indexing works; in-file incremental≡cold tested (`lib.rs:1074-1154`) | Added-file freshness blind (see freshness); duplicate reads (see §18). Action: fix freshness + single-read. |
| scanner | PARTIAL | `crates/scc-indexer/src/scan.rs` (`scan_repo:311-372`, blake3 `hash_bytes:303-306`, `is_ignored:378-399`) | scan unit tests | Authoritative walk+classify+hash; `FileKind` Source/Test/Config/Infra/Docs/Other (`:125-132`, `is_test_path:185`) | Oversized (>5 MiB `:359`) and unreadable (`:357`) files silently `continue` with no counter. Action: skipped-file accounting (§17). |
| language registry | PARTIAL | `scan.rs` (`Language` enum incl. Python/TS/JS/Go/Rust/Java/Json/Yaml/Toml/Csharp/Ruby/Php/Lua/Swift), extractor registry `lib.rs:97-102` | Ruby-counts-as-unparsed test | Registry is broad but extraction is deep only for Python, TypeScript, Go, Java, Rust, C-family; rest count as unsupported | Honest tiers exist implicitly. Action: document tiers, don't chase counts (Part XXXII). |
| language extraction | PARTIAL | `python.rs`, `typescript.rs`, `go`, `java`, `rust`, `cfamily` extractors emitting `facts: Vec<SemanticFact>` | extractor unit tests | Tree-sitter extractors never fail; symbols/imports/calls/routes emitted | Long tail (C++ templates, macros, FFI) missing. Action: defer to P3. |
| receiver resolution | COMPLETE | `crates/scc-indexer/src/resolve.rs` (`resolve_calls`), Ripwire-absorbed rules (self/this/super, field narrowing, unprefixed-field-as-receiver) | extensive resolve unit tests | Precise receiver-aware resolution is the proven core; do not regress | Keep. Precision gauge wiring needed (see honesty), not resolver changes. |
| type narrowing | PARTIAL | one-hop narrowing from unique constructors/assignments (Ripwire phases) | narrowing tests | Single-hop only | Documented limit; deeper inference deferred. |
| inheritance | PARTIAL | base-class/super handling in resolve.rs | super-call tests | `super().x` resolves; full MRO not modeled | Acceptable; record as known limit. |
| cross-file resolution | COMPLETE | import binding + index (`SymbolIndex`), path-style rules (dots→slashes, `mod.py`/`__init__.py` probes) | cross-file tests | Reliable within repo | Same-name veto rules exist (Rule 3). Keep. |
| LSP resolution | PARTIAL | `lsp.rs`, `lsp_ts.rs` overlays writing `Relationship.confidence via with_confidence` | overlay tests | Works as confidence overlay; does NOT feed the precise gauge (`quality_from_calls` is native-only, `resolve.rs:1437-1442`) | Wire LSP-exact into precise gauge or document. See §13. |
| confidence | BROKEN | literals scattered across `resolve.rs` (30 emit sites), overlays (`0.99/0.8`), `Provenance::default_confidence` (`scc-core/src/lib.rs:77-85`, bypassed by native resolver) | — | Numbers look calibrated but are ordinal guesses | Centralize named classes (`LSP_EXACT`, `IMPORT_EXACT`, …) or document as ordinal (§16). |

<!-- trace:v1 id=doc.graph-semantic-layer work=WORK-SI-MMMJA4G6 documents=REQ-SI-503JSBGP -->
## Graph / semantic layer

| Capability | Status | Locations | Tests | Observed behavior | Failure modes / action |
|---|---|---|---|---|---|
| Reality Graph | PARTIAL | `crates/scc-graph` (`RealityGraph`, `state.rs`, `invariants.rs:77-302`) | invariant tests | Compiles entities/relationships/evidence; drift findings (unenforced invariants, missing components, conflicting writers) | Dangling refs from postpass IDs possible (see §21). |
| TrustedGraphView | PARTIAL | `trust.rs` (`allows:52`, claim filtering `:311-350` strips low-confidence INFERRED below 0.85 floor from `scc-context/src/lib.rs:170-180`) | trust tests | Derived claims gated; policy-consistent | Floor is config (`include_low_confidence_inference`). Keep. |
| provenance | PARTIAL | `Provenance` enum (Extracted/Resolved/Observed/Declared/Inferred/Stale) in `scc-core` | provenance tests | Internal evidence carries provenance+confidence correctly | Lost at render boundary for some facts (see §8). |
| System Atlas | PARTIAL | `crates/scc-context/src/atlas.rs` (assembly, entrypoints deduped by `(name,kind):251`, `project_flow_graph:1894-1975`) | atlas tests, `scc bench atlas` (dev 0.079 / holdout 0.033) | 7-layer recall benchmark exists with precision + density (v3); gate 0.5 on 5 startup layers | COMPILER is dominant gap per `--diagnose`. Atlas budget can silently overflow (see budgets). |
| components | BROKEN | `components.rs` (5 evidence classes, boundary ranking), `clustering.rs` (`build_regions:165`, `cluster_components:346`, `compile_services:1174`) | clustering tests | Fallback `"Hosts the {name} code module"` dominates (`components.rs:914`); test/fixture dirs become architecture; no repo-role concept (only file-level `FileKind`) | Fix per Part II: roles, manifest-aware boundaries, drop boilerplate (§5-7). |
| entrypoints | PARTIAL | `atlas.rs:196-268` | atlas tests | Deduped by (name,kind); duplicates across kinds possible | Canonicalize fully (§9). |
| contracts | PARTIAL | contract entities (Registration facts must insert contract entity for non-symbol targets — `write.rs` invariant) | `check-invariants` | HTTP/CLI/API contracts extracted | Fixture routes pollute production (see roles). |
| state authority | PARTIAL | `state.rs`, `StateAuthority` | state tests | Owners + conflicting-writer detection (`invariants.rs:288-296`) | Fixture writers create false conflicts (see roles). |
| FlowGraph | PARTIAL | `flowgraph.rs` (sole producer; emits only Next/Branch/Async/Retry/Error/Publish/Consume/Join) | flow tests | Branch edges from call_blocks evidence, not fanout | `FlowEdgeKind` has 19 variants; most have no producer — audit vs defer explicitly (§23). |
| deployment | PARTIAL | docker-compose evidence class in components; deployment manifests | — | Basic | Cross-repo deploy stitching missing (P2). |
| trust boundaries | PARTIAL | `trust.rs`, stdlib-import edges | — | Exists | Benchmark stdlib imports pollute product boundaries (see roles). |
| runtime evidence | PARTIAL | runtime traces (`/v1/runtime/traces`), `Observed` provenance kept separate | runtime tests | Static vs observed preserved separately | OTel etc. deferred; fine. |
| Surface Map | COMPLETE | `surface.rs` (`build_surface`, staged pipeline, token-aware quotas, hard max + structural compression) | `token_hard_max.rs` (hard max enforced incl. huge-signature fixture) | The bounded pack that works: final render ≤ hard_max tested | Keep as reference pattern for startup/atlas. |
| task retrieval / ranking | PARTIAL | `rank.rs` (heterogeneous PPR + lexical, `rank_startup_atlas` precision-via-ordering), `pagerank.rs` (test/generated/vendored penalties `:963-1003` — ranking-only) | rank tests | Relevance-first vs PPR tradeoff known; rank penalties exist but don't fix component roles | Cheap-benchmark ranking study only (Part XIII). |
| Structural Source / exact source | COMPLETE | `structural_source.rs` (`choose_representation` exact/structural/signatures) | structural tests | Exact-source dominance preserved; representation policy exists | Keep. No L4. |
| source handles | PARTIAL | `ContentHandle`, `fnv1a64_hex`, `entity_id:1074`/`symbol_id:1139`/`relationship_id:1157` constructors in `scc-core` | round-trip tests? | Canonical constructors exist BUT parallel schemes in `scc-indexer/src/write.rs` (`rel_id` blake3, content-hash `evidence_id` name-collide with `scc_core::evidence_id`) | Centralize; audit small-hash collision claims (§22, §24). |
| context budgets | PARTIAL | `ContextBudget`, `allocate_startup_budget` (startup.rs — single allocator, transport parity), per-pack budgets | `token_hard_max.rs` | Surface hard max enforced; startup has corrective loop (`startup_hard_max = total + total/5, min 500`) with test | `packs.rs finish()` still allows `exceeded_soft_budget` overflow for atlas path; divergent estimators (see below). Action: §3 + §26-28. |
| ContextLedger | COMPLETE | `context_ledger.rs` (visible-ids from same render, novelty suppression) | `ledger_freshness.rs` (§54 post-mutation freshness: edits+deletions) | Answers what was rendered; semantics must not weaken | Keep. Snapshot is separate (P2). |
| startup capsule | PARTIAL | `startup.rs` (`build_startup` atlas+surface fusion, deterministic per epoch, rank cache) | hard-max test | Deterministic, cached, hard-capped; Repository Skeleton added with role labels + budget (§1-4, current mission). |
| MCP | PARTIAL | `mcp.rs` (10 tools: system_overview/atlas/task/component/flow/impact/verify/system_context/surface_map/structural_source) | MCP tests | Small surface (good); CLI/MCP/HTTP parity via `build_task_pack` | Annotations added (`tool_annotations`, asserted in schema test); all ten read-only/non-destructive/idempotent, `task_context` open-world (§30, current mission). |
| OMP / agent setup | PARTIAL | `plugin_omp.rs`, `plugin.rs` (SessionStart/UserPromptSubmit/PostToolUse/PreCompact), `compress.rs` (codex/opencode/hermes setup) | setup idempotence tests | Deepest lifecycle integration is OMP; setup tested idempotent | Cursor/VSCode/Gemini deferred (P3). |
| incremental refresh | PARTIAL | `refresh_paths`, daemon `refresh_stale_by_hash` (`httpd.rs:300-323`, scan-diff, added-file aware), watcher notify `:327-387` | `golden.rs:349-382` (incremental≡cold), `invalidation.rs` (modified + added callee cascade) | Daemon path handles added files; CLI path doesn't (see freshness) | Unify on scan-diff (§10-11). |
| multi-repo | COMPLETE (current mission) | `scc-core/src/identity.rs` (explicit → sanitized remote → basename; credential stripping), `scc-store/src/system.rs` (`System::open`, `stitch_routes/topics/package_exports`, Exact/Declared/Inferred/Ambiguous), `Store::open` persistent-id + `adopt_stable_identity`, `Config.repository.id`, `scc system` CLI | store unit tests (5 stitch/match tests) + identity tests (5) + `golden.rs:multi_repo_system_stitches_contracts_topics_and_packages` (real fixtures: GET /health exact, orders.created exact, Declared package, /health ambiguous, move survival, incremental≡cold) | Checkout moves keep ids; standalone-vs-system ids byte-identical; stitches read-only | Remote-derived id only adopts on fresh DBs (later `git remote add` does not rewrite load-bearing ids — documented). |
| temporal graph | COMPLETE (current mission) | `scc-store/src/history.rs` (`GraphRevision`, `SemanticDelta`, `record_revision`/`record_current_revision` (transactional, content-deduped), `revision_members`, `semantic_diff`), migration v7, auto-recorded at index, `scc history` / `scc diff` CLI | store history test (intro/removal/diff/idempotence) + `golden.rs:graph_history_records_and_snapshots_diff` | Full row-sets per revision (simple/correct; storage-heavy on huge repos — noted); component-level diff deferred (entity/rel-level only). ModelEpoch (live view) vs GraphRevision (history position) documented in DATA_STRATEGY L5. |
| ContextSnapshot | COMPLETE (current mission) | `scc-store/src/snapshot.rs` (`ContextSnapshot`, `save_snapshot` (idempotent), `load_snapshot`, `diff_snapshot` still-valid/invalidated), `scc snapshot save/show/diff` CLI (save renders the real task pack) | store snapshot test (persist/diff/survival/unknown-id) + golden e2e (save/show/diff/invalidation) | Ledger untouched; newly-relevant knowledge needs re-render (diff does not invent it — documented). |

<!-- trace:v1 id=doc.freshness-honesty-p0 work=WORK-SI-MMMJA4G6 documents=REQ-SI-503JSBGP -->
## Freshness / honesty (P0 — confirmed broken)

| Capability | Status | Locations | Tests | Observed behavior | Failure modes / action |
|---|---|---|---|---|---|
| freshness (CLI) | COMPLETE (current mission) | `stale_paths` is scan-diff (unchanged/modified/deleted/added); freshness classes CURRENT/STALE in verify output | add/modify/delete verify tests + `deleted_symbol_never_haunts_task_packs` (exact-source exclusion) | Added files make the model stale. Known gap (deferred with evidence): flow/surface entries still name stale symbols; exact-source exclusion is the enforced invariant. |
| `precise` gauge | BROKEN | `resolve.rs:1437-1442` (`quality_from_calls` calls `record_call(c.class, false)` — literal `false`); zero `record_call(*, true)` repo-wide | unit test only (`resolution.rs:272-283` passes booleans directly) | `calls.precise` is always 0; every resolved call lands in `heuristic`; compact line prints `precise=0` forever. No definition of what qualifies as precise. | CONFIRMED. Action: define precise (exact import binding, LSP definition, namespace-qualified, exact receiver+type) and wire per-emit (§13). |
| dead resolution categories | BROKEN | `ResolutionClass::AmbiguousInternal` (`resolution.rs:79-87`): zero production emits. `files.partial`, `stale_facts_dropped`: declared+merged+printed, never set. | — | Metrics that can never change are exposed. | CONFIRMED. Action: implement or remove/rename each (§14). |
| resolution coverage honesty | PARTIAL | `AnalysisQuality` struct (`:104`), `CallQuality` (`:119`), `FileQuality` (`:130`), `compact_line` (`:182-195`); surfaces: `scc index` line, `scc status`, task-pack section, `ContextPack` field | — | Counts exist but precise≡0 and dead buckets mislead; `verify` (`commands.rs:985`) does NOT print gauges | Publish honest counts + rates after §13-14 (§15). |
| skipped-file accounting | MISSING | `scan.rs:357-359` silently `continue`; no counter anywhere | — | `files.discovered/indexed/unsupported/oversized/unreadable/parse_failed` does not exist. | CONFIRMED. Action: count + surface in verify/quality/startup (§17). |
| one token estimator | BROKEN | canonical `scc_core::estimate_tokens` = `chars/4 ceil` (`scc-core/src/lib.rs:1305-1308`); DIVERGENT `main.rs:1242-1247` (`bytes/4`), `external_bench.rs` test + `run_write_matrix.py:164` + `run_context_bench.py:390` (`len//4` bytes) | estimator tests? | Budgets computed in different units per path; byte-based undercounts CJK/emoji vs char-based. | CONFIRMED (prior session). Action: replace all with canonical; Python mirror with same semantics + note (§4). |

<!-- trace:v1 id=doc.benchmarking-security-traceability work=WORK-SI-MMMJA4G6 documents=REQ-SI-503JSBGP -->
## Benchmarking / evaluation / security / traceability / performance

| Capability | Status | Locations | Tests | Observed behavior | Failure modes / action |
|---|---|---|---|---|---|
| benchmarking (deterministic) | PARTIAL | `benchatlas.rs` (v2 structured recall + v3 precision/density/F2, gate 0.5), `run_context_bench.py`, `benchmarks/tasks.json`, `benchmarks/ground-truth/` + holdout ground truth | `test_harness.py`, `test_evaluators.py`, evaluator four-way contract tests | Local retrieval/semantic benchmarks run with no model quota; precision now reported alongside recall | Historical matrices need science corrections (manifest, cap binding, clustered CIs, floor/ceiling) — Part XII, current mission. |
| paid benchmarking + interlock | COMPLETE (current mission) | `run_write_matrix.py` (`check_paid_opt_in`, `--dry-run`, `launch_agent` flag), `run_context_bench.py` (env gate), `benchagent.rs` (`paid_opt_in_allowed`/`require_paid_opt_in` + gates in `run_agent_benchmark`/`run_variant_benchmark`), `benchloop.rs` (gate when `agent_cmd.is_some()`), `main.rs` (gate on non-artifact `bench external` path) | `PaidBenchmarkGateTest` (refuse/dry-run/opt-in/summary-exclusion), patched `test_harness.py` native-mode tests, 3 new + 1 benchloop Rust gate tests, 4 mock tests wrapped in `with_paid_opt_in` | Normal invocation exits 2 before any external launch; `--dry-run` builds artifacts only (`DRY-RUN` cells excluded from rates); `SCC_ALLOW_PAID_BENCHMARKS=1` opts in; `--artifact-only` stays open | Shell drivers (`run_all_matrices.sh`, `run_claude_matrices.sh`) funnel through the gated Python — covered. External watchdogs outside repo noted, not gated. |
| blind evaluation | PARTIAL | `benchmarks/blind-test/` (20 repos), `blind-lock.json`, `holdout/` ground truth, `holdout-v*.txt` + `blind-v*.txt` result lineage in `benchmarks/results/` | blind protocol tests? | Frozen corpus + lock exist; aggregate result files present | Lineage (which commit/config produced which report) unverified; blind protocol vs quality must be distinguished (§42). No individual blind misses inspected (per mission). |
| security: secret handling | COMPLETE (current mission) | placebo `SecurityConfig.redact_secrets` REMOVED (field + default arm + example + test fixture); structural protection (references-only) documented in `config.example.yaml` + SECURITY.md | config round-trip + parses-example tests green; old configs still parse (serde ignores the key) | Remote listen: `httpd.rs` refuses non-loopback binds without `SCC_ALLOW_REMOTE_LISTEN=1` (fail-closed; no auth exists — documented, TLS/auth deferred). |
| security: remote / loopback | PARTIAL | `httpd.rs:1-7` (loopback default `config.security.listen`), `embed.rs:24-35` (`is_remote` treats loopback/`0.0.0.0` as local), `embed_cli.rs:111-115,192-204` (remote inference fails closed unless `allow_remote_models`) | `remote_policy_fails_closed`, `is_remote` unit tests | Loopback default good; remote models fail closed; HTTP daemon documents read-only-except-index/traces | Non-loopback `listen` has no auth/token contract; Docker docs must match behavior (§30). |
| traceability | PARTIAL | `.trace/` (1124 nodes, 1645 declared edges, 0 broken refs), policy `standard/lifecycle=wip`, exclusions cover `benchmarks/external/**`, `fixtures/**`, large indexer files | `trace verify --changed` gate | Strong coverage where marked; 43 blocking stale traces + 215 changed traced artifacts pre-existing | Action: narrow scope / reduce ceremony / remove unjustified exclusions per Part X; do not touch in-progress user TraceLayer work. Final report must state what TraceLayer actually guarantees. |
| performance | PARTIAL | release-profile builds, rank cache per epoch (`startup.rs`), cochange cache by git HEAD + 500-file cap, sweep_orphan set-based (never O(n²) LIKE) | — | No measured hot-path profile published; `scc bench atlas` full corpus ~30 min | Suspected O(n²): per-file analysis-quality map deserialize/insert/serialize per file (§19); freshness full rehash per query (§12); duplicate file reads (§18). Action: measure cold/warm/incremental/daemon on release build, then fix (§20, §38). |
| graph integrity (cold≡verify) | PARTIAL | `verify()` (`packs.rs:1585-1795`: dangling check `:1637-1667`, unenforced `:1677-1683`, drift `:1696-1702`), `check-invariants` | golden verify tests | Verification exists and is strict | Unproven on fresh cold index for all fixtures; malformed postpass IDs suspected. Action: add `cold_index → verify_graph → zero invalid dangling` test (§21). |
| causal completeness | PARTIAL | `FlowEdgeKind` (19 variants), state authority, retry annotations (`maybe_retry`, `@Retryable`/`@Retry`, celery tasks, `Scheduled`/`RabbitListener`/`KafkaListener` facts in java extractor) | retry/fact tests | Failure/retry evidence extracted where annotations exist | Most `FlowEdgeKind` variants unproduced; state↔flow not joined. Action: implement-or-defer each (§23-25). |
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
| Startup massively exceeds budget | PARTIAL | corrective hard-max loop + test exist (`startup.rs`, `token_hard_max.rs`); atlas `exceeded_soft_budget` path unverified |
| Cold index fails verify | NOT REPRODUCED yet | needs the §21 test to determine |

<!-- trace:v1 id=doc.scc-capability-ledger.deferred work=WORK-SI-MMMJA4G6 documents=REQ-SI-503JSBGP -->
## Deferred / not worth implementing (this mission)

| Item | Disposition | Reason |
|---|---|---|
| C/C++ long tail (function pointers, `compile_commands.json`, templates, macros, FFI) | DEFERRED (P3) | Prevalence/precision tradeoff; no testability story yet |
| Cursor/VS Code/Gemini setup | DEFERRED (P3) | OMP is deepest integration; others on demand |
| OTel/runtime source expansion | DEFERRED | Static/runtime separation exists; no interface justifies more |
| L4 retrieval layer | NOT WORTH IMPLEMENTING | Mission non-goal; internal units suffice |
| ArangoDB / Docker infra / 30 MCP tools / mandatory embeddings | NOT WORTH IMPLEMENTING | Explicit non-goals; absorb principles, not baggage |
| Shallow languages for count | NOT WORTH IMPLEMENTING | Honest tiers over raw counts |
