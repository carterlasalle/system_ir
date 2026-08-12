//! Golden-repository integration tests (docs/TEST_PLAN.md §3): the
//! http-service-python fixture must produce the expected System IR.

use std::path::PathBuf;
use std::process::Command;

pub fn scc() -> &'static str {
    env!("CARGO_BIN_EXE_scc")
}

/// Copy a fixture tree (minus its `.scc` state) into a fresh tempdir under a
/// fixed `repo` directory so repository ids are stable across runs.
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
pub fn workdir(tmp: &std::path::Path) -> std::path::PathBuf {
    tmp.join("repo")
}

pub fn run(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(scc())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("scc binary runs")
}

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

#[test]
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
fn task_cache_hits_within_an_epoch_and_misses_across() {
    let repo = copy_fixture("http-service-python");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);
    let goal = "rename the transcript field in the api response";

    let a = run_ok(&dir, &["context", "task", "--json", goal]);
    let b = run_ok(&dir, &["context", "task", "--json", goal]);
    assert_eq!(a, b, "same model state must serve the cached pack");

    // identical re-index: rebuild is deterministic — same state yields the
    // same pack (the epoch change invalidated the cache key, not the truth)
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
