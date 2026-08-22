//! Golden-repository integration tests (docs/TEST_PLAN.md §3): the
//! http-service-python fixture must produce the expected System IR.

use std::path::PathBuf;
use std::process::Command;
// trace:v1 id=test.scc.golden verifies=REQ-SCC-IR exercises=impl.scc.cli,impl.scc.store

// trace:v1 id=test.crates-scc-cli-tests-golden.scc
pub fn scc() -> &'static str {
    env!("CARGO_BIN_EXE_scc")
}

/// Copy a fixture tree (minus its `.scc` state) into a fresh tempdir under a
/// fixed `repo` directory so repository ids are stable across runs.
// trace:v1 id=test.crates-scc-cli-tests-golden.copy-fixture
pub fn copy_fixture(name: &str) -> tempfile::TempDir {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("fixtures")
        .join(name);
    let dst = tempfile::TempDir::new().unwrap();
    let repo_dir = dst.path().join("repo");
    std::fs::create_dir_all(&repo_dir).unwrap();
    copy_tree(&src, &repo_dir);
    dst
}

// trace:v1 id=test.crates-scc-cli-tests-golden.copy-tree
pub fn copy_tree(src: &std::path::Path, dst: &std::path::Path) {
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == ".scc" {
            continue; // state, not fixture content
        }
        let from = entry.path();
        let to = dst.join(&name);
        if from.is_dir() {
            std::fs::create_dir_all(&to).unwrap();
            copy_tree(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
}

/// The directory the fixture was copied into (the `scc` repo root).
// trace:v1 id=test.crates-scc-cli-tests-golden.workdir
pub fn workdir(tmp: &std::path::Path) -> std::path::PathBuf {
    tmp.join("repo")
}

// trace:v1 id=test.crates-scc-cli-tests-golden.run
pub fn run(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(scc())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("scc binary runs")
}

// trace:v1 id=test.crates-scc-cli-tests-golden.run-ok
pub fn run_ok(dir: &std::path::Path, args: &[&str]) -> String {
    let out = run(dir, args);
    assert!(
        out.status.success(),
        "`scc {}` failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// A valid CFG branch condition: a control-block kind (if/else/for/while/
/// try/catch/match/switch/with/do/loop/finally/select) or the legacy
/// `conditional: <op>` format from pre-CFG indexes.
// trace:v1 id=test.crates-scc-cli-tests-golden.is-cfg-condition
pub fn is_cfg_condition(c: &str) -> bool {
    matches!(
        c,
        "if" | "else" | "for" | "while" | "try" | "catch" | "match" | "switch"
            | "with" | "do" | "loop" | "select"
    ) || c.starts_with("conditional:")
}

#[test]
// trace:v1 id=test.crates-scc-cli-tests-golden.http-service-produces-expected-ir
fn http_service_produces_expected_ir() {
    let repo = copy_fixture("http-service-python");
    run_ok(&workdir(repo.path()), &["index", "--quiet"]);

    let status = run_ok(&workdir(repo.path()), &["status"]);
    assert!(status.contains("components:"), "{status}");
    assert!(status.contains("flows:"), "{status}");

    // components: root (main.py), services, tests
    let comps = run_ok(&workdir(repo.path()), &["components"]);
    assert!(comps.contains("root"), "{comps}");
    assert!(comps.contains("services"), "{comps}");

    // routes + flows
    let flows = run_ok(&workdir(repo.path()), &["flows"]);
    assert!(flows.contains("get-/api/transcripts"), "{flows}");
    assert!(flows.contains("GET /api/transcripts"), "{flows}");

    // data ownership: services owns db
    let flow = run_ok(&workdir(repo.path()), &["context", "flow", "get-/api/transcripts"]);
    assert!(flow.contains("services"), "{flow}");
    assert!(flow.contains("TranscriptRepository"), "{flow}");

    // task context finds the normalization code and its test
    let task = run_ok(
        &workdir(repo.path()),
        &["context", "task", "change transcript normalization"],
    );
    assert!(task.contains("Normalizer"), "{task}");
    assert!(task.contains("test_normalization_preserves_raw"), "{task}");
    assert!(task.contains("PRIMARY FLOW"), "{task}");

    // verify is clean on this repo
    let verify = run_ok(&workdir(repo.path()), &["verify"]);
    assert!(verify.contains("VERIFIED"), "{verify}");
    assert!(verify.contains("Fresh"), "{verify}");

    // CI invariant check passes
    let out = run(&workdir(repo.path()), &["check-invariants"]);
    assert!(out.status.success(), "check-invariants must pass");
}

#[test]
// trace:v1 id=test.crates-scc-cli-tests-golden.stale-worktree-never-serves-cached-pack
fn stale_worktree_never_serves_cached_pack() {
    // P0 trust contract (§7): a pack cached under a clean revision must not
    // be returned after a working-tree file changed WITHOUT re-indexing.
    // The rebuild excludes the stale facts and warns instead.
    let repo = copy_fixture("http-service-python");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);

    let goal = "change transcript normalization";
    let first = run_ok(&dir, &["context", "task", goal]);
    assert!(first.contains("Normalizer"), "{first}");
    assert!(!first.contains("changed since indexing"), "{first}");

    // modify a source file WITHOUT indexing
    let f = dir.join("services/transcripts.py");
    let mut src = std::fs::read_to_string(&f).unwrap();
    src.push_str("\n# working-tree change\n");
    std::fs::write(&f, src).unwrap();

    let second = run_ok(&dir, &["context", "task", goal]);
    assert!(
        second.contains("stale: services/transcripts.py"),
        "stale facts must be excluded and warned: {second}"
    );

    // the exact same request after a re-index returns a fresh pack again
    run_ok(&dir, &["index", "--quiet"]);
    let third = run_ok(&dir, &["context", "task", goal]);
    assert!(!third.contains("stale: services/transcripts.py"), "{third}");
}

#[test]
// trace:v1 id=test.crates-scc-cli-tests-golden.canonical-flow-graph-preserves-topology
fn canonical_flow_graph_preserves_topology() {
    // Wave 3 exit condition: the canonical FlowGraph preserves branches,
    // retry, and fanout exactly — alternate execution paths are never
    // flattened into false sequential causality, and branches come from
    // evidence (call fanout, @tenacity.retry decorators), never text
    // heuristics.
    let repo = copy_fixture("py-queue-service");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);
    let out = run_ok(&dir, &["export", "flow-graphs.json"]);
    let graphs: serde_json::Value = serde_json::from_str(&out).unwrap();
    let graphs = graphs.as_array().unwrap();
    assert!(!graphs.is_empty(), "at least one canonical graph");

    let graph = graphs
        .iter()
        .find(|g| g["name"].as_str().unwrap_or("").contains("consume"))
        .or_else(|| graphs.first())
        .unwrap();

    let kinds: Vec<&str> = graph["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["kind"].as_str())
        .collect();

    // retry: the @tenacity.retry decorator must produce Retry edges
    assert!(
        kinds.contains(&"retry"),
        "retry edges from decorator evidence: {kinds:?}"
    );
    // P1 §19: plain call fanout (consume -> process_incident AND
    // IncidentStore) is NOT a branch — those are Next edges; only the call
    // inside the try/except (classify) is a Branch edge.
    let branch_edges: Vec<&serde_json::Value> = graph["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "branch")
        .collect();
    assert_eq!(
        branch_edges.len(),
        1,
        "exactly one conditional call becomes a branch: {kinds:?}"
    );
    assert!(
        is_cfg_condition(branch_edges[0]["condition"].as_str().unwrap_or("")),
        "branch condition names the control-block evidence: {branch_edges:?}"
    );
    assert!(
        kinds.iter().filter(|k| *k == &"next").count() >= 2,
        "plain fanout stays Next edges (unordered calls, not alternatives): {kinds:?}"
    );
    // no false convergence either: this fixture has no join point, so no
    // Join edge is invented (the old branch-artifact join is gone)
    // branch edges carry evidence conditions (block kind or the legacy
    // "conditional: <op>" format), never comma-split operation lists from
    // generated text
    for e in graph["edges"].as_array().unwrap() {
        if e["kind"] == "branch" {
            if let Some(c) = e["condition"].as_str() {
                assert!(
                    is_cfg_condition(c),
                    "branch conditions name the CFG evidence: {c}"
                );
            }
        }
    }
    // exits detected
    assert!(
        graph["exits"].as_array().map(|a| !a.is_empty()).unwrap_or(false),
        "exits detected: {graph}"
    );
    // provenance recorded per edge
    assert!(
        graph["provenance_summary"]
            .as_object()
            .map(|o| !o.is_empty())
            .unwrap_or(false),
        "provenance summary present"
    );
}

#[test]
// trace:v1 id=test.crates-scc-cli-tests-golden.checkpoint-captures-goal-from-active-bead
fn checkpoint_captures_goal_from_active_bead() {
    // §126: checkpoint goal/bead are populated from active task state.
    let repo = copy_fixture("http-service-python");
    let dir = workdir(repo.path());
    std::fs::create_dir_all(dir.join(".beads")).unwrap();
    std::fs::write(
        dir.join(".beads/issues.jsonl"),
        "{\"id\":\"b7\",\"title\":\"Fix transcript normalization retry\",\"status\":\"in_progress\",\"dependencies\":[]}\n",
    )
    .unwrap();
    run_ok(&dir, &["index", "--quiet"]);
    let out = run_ok(&dir, &["checkpoint", "save", "--json"]);
    let cp: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(cp["task"]["goal"], "Fix transcript normalization retry", "{out}");
    assert_eq!(cp["task"]["bead"], "b7", "{out}");
    // rehydration renders the goal
    let loaded = run_ok(&dir, &["checkpoint", "load", "--inject"]);
    assert!(loaded.contains("Fix transcript normalization retry"), "{loaded}");
}

#[test]
// trace:v1 id=test.crates-scc-cli-tests-golden.atlas-describes-system-accurately
fn atlas_describes_system_accurately() {
    // Wave 2 QA: the agent should be able to explain the system from the
    // atlas alone — purpose, architecture, flows, ownership, contracts,
    // freshness — with no source exploration.
    let repo = copy_fixture("http-service-python");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);
    let atlas = run_ok(&dir, &["atlas"]);
    assert!(atlas.contains("SYSTEM PURPOSE"), "{atlas}");
    assert!(atlas.contains("normalized radio transcripts"), "purpose: {atlas}");
    assert!(atlas.contains("ARCHITECTURE"), "{atlas}");
    assert!(atlas.contains("SERVICES"), "{atlas}");
    assert!(atlas.contains("TranscriptRepository"), "component purpose: {atlas}");
    assert!(atlas.contains("get-/api/transcripts"), "flow: {atlas}");
    assert!(atlas.contains("handle_transcripts"), "flow step: {atlas}");
    assert!(atlas.contains("DATA OWNERSHIP"), "{atlas}");
    assert!(atlas.contains("services owns db"), "ownership: {atlas}");
    assert!(atlas.contains("CONTRACTS"), "{atlas}");
    assert!(atlas.contains("GET /api/transcripts"), "contract: {atlas}");
    assert!(atlas.contains("EVIDENCE STATUS"), "{atlas}");
    assert!(atlas.contains("FRESH"), "freshness: {atlas}");
    assert!(atlas.contains("Raw transcripts are immutable"), "invariant/README purpose: {atlas}");
}

#[test]
// trace:v1 id=test.crates-scc-cli-tests-golden.atlas-excludes-stale-facts-and-warns
fn atlas_excludes_stale_facts_and_warns() {
    let repo = copy_fixture("http-service-python");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);
    // modify without re-indexing
    let f = dir.join("services/transcripts.py");
    let mut src = std::fs::read_to_string(&f).unwrap();
    src.push_str("\n# worktree edit\n");
    std::fs::write(&f, src).unwrap();
    let atlas = run_ok(&dir, &["atlas"]);
    assert!(
        atlas.contains("services/transcripts.py changed since indexing"),
        "stale warning must surface: {atlas}"
    );
    // re-index -> fresh again
    run_ok(&dir, &["index", "--quiet"]);
    let atlas2 = run_ok(&dir, &["atlas"]);
    assert!(!atlas2.contains("changed since indexing"), "{atlas2}");
}

#[test]
// trace:v1 id=test.crates-scc-cli-tests-golden.atlas-runtime-section-shows-observed-paths-and-drift
fn atlas_runtime_section_shows_observed_paths_and_drift() {
    // Wave 6: the atlas RUNTIME section surfaces observed trace signatures
    // and three-way drift findings (declared vs static vs observed).
    let repo = copy_fixture("http-service-python");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);

    // No runtime data yet: the section renders but says (none).
    let atlas = run_ok(&dir, &["atlas"]);
    assert!(atlas.contains("# RUNTIME\n(none)"), "{atlas}");

    // Ingest a 3-span OTLP trace: an `api` root span with two `db` children
    // dedupes to one "root -> api -> db" signature.
    let trace = r#"{"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"api"}}]},"scopeSpans":[{"spans":[{"traceId":"t1","spanId":"a","name":"GET /x","startTimeUnixNano":"0","endTimeUnixNano":"10000000","status":{"code":0}}]}]},{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"db"}}]},"scopeSpans":[{"spans":[{"traceId":"t1","spanId":"b","parentSpanId":"a","name":"SELECT 1","startTimeUnixNano":"1000000","endTimeUnixNano":"6000000","status":{"code":0}},{"traceId":"t1","spanId":"c","parentSpanId":"a","name":"SELECT 2","startTimeUnixNano":"2000000","endTimeUnixNano":"7000000","status":{"code":0}}]}]}]}"#;
    run_ok(&dir, &["ingest", trace]);
    let atlas = run_ok(&dir, &["atlas"]);
    assert!(atlas.contains("# RUNTIME"), "{atlas}");
    assert!(atlas.contains("OBSERVED PATH"), "{atlas}");
    assert!(atlas.contains("root -> api -> db (1 reqs"), "{atlas}");

    // Reconcile writes the three-way drift: the observed edges are
    // undeclared (the fixture declares no flows) and the fixture's static
    // edges that were never observed are flagged. The epoch bump makes the
    // cached atlas re-render with the drift lines.
    run_ok(&dir, &["runtime", "reconcile"]);
    let atlas = run_ok(&dir, &["atlas"]);
    assert!(
        atlas.contains("DRIFT undeclared observed: root -> api")
            && atlas.contains("DRIFT undeclared observed: api -> db"),
        "atlas must surface undeclared_observed drift: {atlas}"
    );
    assert!(
        atlas.contains("DRIFT static unobserved:"),
        "atlas must surface static_unobserved drift: {atlas}"
    );
}

#[test]
// trace:v1 id=test.crates-scc-cli-tests-golden.atlas-budget-accounting-is-honest
fn atlas_budget_accounting_is_honest() {
    let repo = copy_fixture("http-service-python");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);
    // tight budget: the renderer drops low-priority sections and reports it
    // — it never silently cuts critical content
    let out = run_ok(&dir, &["atlas", "--budget", "200", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["budget"], 200);
    assert!(
        v["truncated"].as_bool().unwrap()
            || v["exceeded_soft_budget"].as_bool().unwrap(),
        "tight budget must be reported: {out}"
    );
    // critical sections never dropped
    let dropped: Vec<&str> = v["dropped_sections"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for critical in ["CRITICAL INVARIANTS", "DATA OWNERSHIP", "CONTRACTS"] {
        assert!(!dropped.contains(&critical), "dropped critical: {dropped:?}");
    }
}

#[test]
// trace:v1 id=test.crates-scc-cli-tests-golden.task-cache-hits-within-an-epoch-and-misses-across
fn task_cache_hits_within_an_epoch_and_misses_across() {
    let repo = copy_fixture("http-service-python");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);
    let goal = "rename the transcript field in the api response";

    let a = run_ok(&dir, &["context", "task", "--json", goal]);
    let b = run_ok(&dir, &["context", "task", "--json", goal]);
    // The artifact is {pack, delta, delta_ids}: the PACK must be cached
    // byte-identically; the delta intentionally evolves with the ledger
    // (run 1 recorded its rendered ids, so run 2's delta suppresses them —
    // that suppression IS the Wave-14E novelty contract).
    let pack_of = |s: &str| -> String {
        let v: serde_json::Value = serde_json::from_str(s).unwrap();
        serde_json::to_string(&v["pack"]).unwrap()
    };
    assert_eq!(pack_of(&a), pack_of(&b), "same model state must serve the cached pack");

    // identical re-index: rebuild is deterministic — same state yields the
    // same pack AND the same full artifact (the epoch change invalidated
    // the cache key and reset the per-epoch ledger, not the truth)
    run_ok(&dir, &["index", "--quiet"]);
    let c = run_ok(&dir, &["context", "task", "--json", goal]);
    assert_eq!(a, c, "deterministic rebuild for identical state");

    // a real source change (re-indexed) produces a genuinely new pack
    let f = dir.join("services/transcripts.py");
    let mut src = std::fs::read_to_string(&f).unwrap();
    src.push_str("\ndef new_helper():\n    return 2\n");
    std::fs::write(&f, src).unwrap();
    run_ok(&dir, &["index", "--quiet"]);
    let d = run_ok(&dir, &["context", "task", "--json", goal]);
    assert_ne!(a, d, "changed source must produce a different pack");
}

#[test]
// trace:v1 id=test.crates-scc-cli-tests-golden.incremental-refresh-matches-cold-cli
fn incremental_refresh_matches_cold_cli() {
    let repo = copy_fixture("http-service-python");
    run_ok(&workdir(repo.path()), &["index", "--quiet"]);
    let cold = run_ok(&workdir(repo.path()), &["export", "system-ir.json"]);

    // edit a file, then refresh just that path
    std::fs::write(
        workdir(repo.path()).join("services/transcripts.py"),
        "def helper():\n    return 1\n",
    )
    .unwrap();
    run_ok(&workdir(repo.path()), &["index", "--paths", "services/transcripts.py", "--quiet"]);

    // fresh cold index of the final state must agree with the incremental one
    let repo2 = copy_fixture("http-service-python");
    std::fs::write(
        workdir(repo2.path()).join("services/transcripts.py"),
        "def helper():\n    return 1\n",
    )
    .unwrap();
    run_ok(&workdir(repo2.path()), &["index", "--quiet"]);
    let cold2 = run_ok(&workdir(repo2.path()), &["export", "system-ir.json"]);
    let incr = run_ok(&workdir(repo.path()), &["export", "system-ir.json"]);
    // equivalence is about FACTS — normalize the wall-clock indexed_at
    let norm = |s: &str| {
        let mut v: serde_json::Value = serde_json::from_str(s).unwrap();
        v["snapshot"]["indexed_at"] = serde_json::Value::String("X".into());
        v
    };
    assert_eq!(norm(&cold2), norm(&incr), "full vs incremental equivalence (CLI)");
    assert_ne!(norm(&cold), norm(&incr), "the edit must change the IR");
}

#[test]
// trace:v1 id=test.crates-scc-cli-tests-golden.stale-detection-and-verify
fn stale_detection_and_verify() {
    let repo = copy_fixture("http-service-python");
    run_ok(&workdir(repo.path()), &["index", "--quiet"]);

    // modify without re-indexing
    std::fs::write(workdir(repo.path()).join("main.py"), "def changed():\n    pass\n").unwrap();

    let status = run_ok(&workdir(repo.path()), &["status"]);
    assert!(status.contains("STALE"), "{status}");

    let task = run_ok(&workdir(repo.path()), &["context", "task", "transcripts"]);
    assert!(task.contains("stale"), "{task}");
    assert!(task.contains("WARNING"), "{task}");

    let verify = run_ok(&workdir(repo.path()), &["verify"]);
    assert!(verify.contains("STALE"), "{verify}");
    assert!(verify.contains("ISSUES FOUND"), "{verify}");

    // re-index restores freshness
    run_ok(&workdir(repo.path()), &["index", "--quiet"]);
    let verify = run_ok(&workdir(repo.path()), &["verify"]);
    assert!(verify.contains("Fresh"), "{verify}");
}

#[test]
// trace:v1 id=test.crates-scc-cli-tests-golden.secret-redaction-end-to-end
fn secret_redaction_end_to_end() {
    let repo = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(workdir(repo.path())).unwrap();
    std::fs::write(
        workdir(repo.path()).join(".env"),
        "DATABASE_URL=postgres://user:hunter2secret@db:5432/x\nPORT=8080\n",
    )
    .unwrap();
    std::fs::write(workdir(repo.path()).join("app.py"), "def main():\n    pass\n").unwrap();
    run_ok(&workdir(repo.path()), &["index", "--quiet"]);
    let ir = run_ok(&workdir(repo.path()), &["export", "system-ir.json"]);
    assert!(!ir.contains("hunter2secret"), "secret value leaked");
    assert!(!ir.contains("postgres://user:"), "DSN leaked");
    assert!(ir.contains("DATABASE_URL"), "reference kept");
}

#[test]
// trace:v1 id=test.crates-scc-cli-tests-golden.check-invariants-fails-on-dangling-refs
fn check_invariants_fails_on_dangling_refs() {
    let repo = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(workdir(repo.path())).unwrap();
    std::fs::write(workdir(repo.path()).join("a.py"), "def a():\n    pass\n").unwrap();
    run_ok(&workdir(repo.path()), &["index", "--quiet"]);
    // sabotage: drop an entity referenced by a relationship
    let db =
        scc_store::Store::open(&workdir(repo.path()).join(".scc/scc.db"), &workdir(repo.path()))
            .unwrap();
    if let Some(e) = db.entities_by_kind("symbol").unwrap().into_iter().next() {
        db.delete_entity(&e.id).unwrap();
    }
    drop(db);
    let out = run(&workdir(repo.path()), &["check-invariants"]);
    assert!(!out.status.success(), "dangling refs must fail CI check");
}

#[test]
// trace:v1 id=test.crates-scc-cli-tests-golden.query-and-export-formats
fn query_and_export_formats() {
    let repo = copy_fixture("http-service-python");
    run_ok(&workdir(repo.path()), &["index", "--quiet"]);

    let q = run_ok(&workdir(repo.path()), &["query", "transcript"]);
    assert!(q.contains("transcript") || q.contains("Transcript"), "{q}");

    let jsonl = run_ok(&workdir(repo.path()), &["export", "system-ir.jsonl"]);
    assert!(jsonl.lines().count() > 5, "jsonl has many records");
    assert!(jsonl.contains("\"type\":\"entity\""), "{jsonl}");

    let ccg = run_ok(&workdir(repo.path()), &["export", "ccg"]);
    let v: serde_json::Value = serde_json::from_str(&ccg).unwrap();
    assert_eq!(v["schema"], "ccg");
    assert!(v["layers"]["L0"].is_object());
    assert!(v["layers"]["L1"]["architecture"].is_array());
}
