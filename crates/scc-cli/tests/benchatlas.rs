//! Atlas recall benchmark integration tests (Wave 8 §57): `scc bench atlas`
//! on the fixtures fallback path (no corpus dir) must complete and print
//! the recall table with per-section recall, overall, and the gate verdict.

use std::path::{Path, PathBuf};

mod golden;

fn workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
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
// trace:v1 id=test.scc.bench.atlas verifies=REQ-SCC-TEST exercises=impl.scc.bench.atlas

#[test]
fn bench_atlas_fixtures_fallback_prints_table() {
    // Hermetic workspace: fixtures/ + benchmarks/tasks.json but NO
    // benchmarks/corpus, so the fixtures fallback path triggers regardless
    // of the state of the real benchmarks/ directory.
    let ws = workspace();
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("ws");
    std::fs::create_dir_all(root.join("benchmarks")).unwrap();
    for entry in std::fs::read_dir(ws.join("fixtures")).unwrap() {
        let e = entry.unwrap();
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &root.join("fixtures").join(e.file_name()));
        }
    }
    std::fs::copy(
        ws.join("benchmarks/tasks.json"),
        root.join("benchmarks/tasks.json"),
    )
    .unwrap();

    let out = std::process::Command::new(golden::scc())
        .args(["bench", "atlas"])
        .current_dir(&root)
        .output()
        .expect("scc bench atlas runs");
    assert!(
        out.status.success(),
        "`scc bench atlas` failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("scc bench atlas"), "{stdout}");
    assert!(stdout.contains("fixtures fallback"), "{stdout}");
    // v2 report: startup-required layer columns + precision/density/tokens
    assert!(stdout.contains("arch"), "{stdout}");
    assert!(stdout.contains("entry"), "{stdout}");
    assert!(stdout.contains("prec"), "{stdout}");
    assert!(stdout.contains("f/1k"), "{stdout}");
    assert!(stdout.contains("mean"), "{stdout}");
    assert!(stdout.contains("gate:"), "{stdout}");
    assert!(stdout.contains("PASS") || stdout.contains("FAIL"), "{stdout}");
}

#[test]
fn bench_atlas_corpus_mode_with_explicit_dirs() {
    // Corpus mode: `--corpus`/`--ground-truth` point at a hermetic temp
    // workspace holding one real fixture repo + its ground-truth doc. The
    // table must print per-section recall, overall, and the gate verdict,
    // with the missing ground-truth item flagged.
    let ws = workspace();
    let tmp = tempfile::TempDir::new().unwrap();
    let corpus = tmp.path().join("corpus");
    let gt = tmp.path().join("ground-truth");
    std::fs::create_dir_all(&corpus).unwrap();
    std::fs::create_dir_all(&gt).unwrap();
    copy_tree(
        &ws.join("fixtures/http-service-python"),
        &corpus.join("http-service-python"),
    );
    std::fs::write(
        gt.join("http-service-python.md"),
        "## components\n- root\n- services\n## entrypoints\n- handle_transcripts\n## flows\n- TranscriptRepository\n## ownership\n- zzz_missing_store\n## contracts\n- GET /api/transcripts\n- GET /api/zzz_missing\n## tests\n- test_transcripts\n",
    )
    .unwrap();

    let out = std::process::Command::new(golden::scc())
        .args([
            "bench",
            "atlas",
            "--corpus",
            corpus.to_str().unwrap(),
            "--ground-truth",
            gt.to_str().unwrap(),
        ])
        .current_dir(tmp.path())
        .output()
        .expect("scc bench atlas runs");
    assert!(
        out.status.success(),
        "`scc bench atlas` failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("scc bench atlas"), "{stdout}");
    assert!(stdout.contains("corpus"), "mode: {stdout}");
    assert!(stdout.contains("http-service-python"), "{stdout}");
    assert!(stdout.contains("overall"), "{stdout}");
    assert!(stdout.contains("mean"), "{stdout}");
    assert!(stdout.contains("gate:"), "{stdout}");
    assert!(stdout.contains("PASS") || stdout.contains("FAIL"), "{stdout}");
    assert!(
        stdout.contains("state_authority:zzz_missing_store"),
        "missed items must be listed: {stdout}"
    );
}

#[test]
fn bench_atlas_holdout_compares_and_writes_results_file() {
    // Holdout protocol: dev corpus + ground truth via --corpus/--ground-truth
    // (temp dirs), holdout corpus at <root>/benchmarks/holdout with ground
    // truth at <root>/benchmarks/holdout-ground-truth. The run must print
    // both reports, the gap summary, and write
    // <root>/benchmarks/results/holdout-v3.txt.
    let ws = workspace();
    let tmp = tempfile::TempDir::new().unwrap();
    let corpus = tmp.path().join("corpus");
    let gt = tmp.path().join("ground-truth");
    std::fs::create_dir_all(&corpus).unwrap();
    std::fs::create_dir_all(&gt).unwrap();
    copy_tree(
        &ws.join("fixtures/http-service-python"),
        &corpus.join("http-service-python"),
    );
    std::fs::write(
        gt.join("http-service-python.md"),
        "## components\n- root\n- services\n## entrypoints\n- handle_transcripts\n## flows\n- TranscriptRepository\n## ownership\n- zzz_missing_store\n## contracts\n- GET /api/transcripts\n- GET /api/zzz_missing\n## tests\n- test_transcripts\n",
    )
    .unwrap();

    // holdout dirs under the workspace root, as the protocol requires
    let holdout = tmp.path().join("benchmarks").join("holdout");
    let holdout_gt = tmp.path().join("benchmarks").join("holdout-ground-truth");
    std::fs::create_dir_all(&holdout).unwrap();
    std::fs::create_dir_all(&holdout_gt).unwrap();
    copy_tree(
        &ws.join("fixtures/http-service-python"),
        &holdout.join("http-service-python"),
    );
    // slightly different ground truth so the gap is not trivially zero
    std::fs::write(
        holdout_gt.join("http-service-python.md"),
        "## components\n- root\n- services\n## entrypoints\n- handle_transcripts\n## flows\n- TranscriptRepository\n## ownership\n- zzz_missing_store\n## contracts\n- GET /api/transcripts\n## tests\n- test_transcripts\n",
    )
    .unwrap();

    let out = std::process::Command::new(golden::scc())
        .args([
            "bench",
            "atlas",
            "--holdout",
            "--corpus",
            corpus.to_str().unwrap(),
            "--ground-truth",
            gt.to_str().unwrap(),
        ])
        .current_dir(tmp.path())
        .output()
        .expect("scc bench atlas --holdout runs");
    assert!(
        out.status.success(),
        "`scc bench atlas --holdout` failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("=== DEV corpus ==="), "{stdout}");
    assert!(stdout.contains("=== HOLDOUT corpus ==="), "{stdout}");
    assert!(stdout.contains("=== gap (holdout - dev) ==="), "{stdout}");
    assert!(stdout.contains("verdict:"), "{stdout}");
    assert!(
        stdout.contains("NO OVERFIT") || stdout.contains("BORDERLINE") || stdout.contains("OVERFIT"),
        "verdict string missing: {stdout}"
    );
    let results = tmp.path().join("benchmarks/results/holdout-v3.txt");
    let text = std::fs::read_to_string(&results)
        .unwrap_or_else(|e| panic!("results file missing: {e}"));
    assert!(text.contains("overall (gate)"), "{text}");
    assert!(text.contains("## verdict:"), "{text}");
    assert!(text.contains("holdout repo overall recall"), "{text}");
}

#[test]
fn bench_atlas_holdout_errors_when_holdout_corpus_missing() {
    // The holdout corpus is a required protocol input: a missing dir must
    // fail with a clear error, not silently run an empty holdout.
    let ws = workspace();
    let tmp = tempfile::TempDir::new().unwrap();
    let corpus = tmp.path().join("corpus");
    let gt = tmp.path().join("ground-truth");
    std::fs::create_dir_all(&corpus).unwrap();
    std::fs::create_dir_all(&gt).unwrap();
    copy_tree(
        &ws.join("fixtures/http-service-python"),
        &corpus.join("http-service-python"),
    );
    std::fs::write(
        gt.join("http-service-python.md"),
        "## components\n- root\n## entrypoints\n- handle_transcripts\n## flows\n- TranscriptRepository\n## ownership\n- s\n## contracts\n- GET /api/transcripts\n",
    )
    .unwrap();

    let out = std::process::Command::new(golden::scc())
        .args([
            "bench",
            "atlas",
            "--holdout",
            "--corpus",
            corpus.to_str().unwrap(),
            "--ground-truth",
            gt.to_str().unwrap(),
        ])
        .current_dir(tmp.path())
        .output()
        .expect("scc bench atlas --holdout runs");
    assert!(!out.status.success(), "expected failure, got success");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("holdout corpus dir not found"),
        "clear error expected: {stderr}"
    );
}
