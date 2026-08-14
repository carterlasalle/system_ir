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
    assert!(stdout.contains("=== DEVELOPMENT corpus ==="), "{stdout}");
    assert!(stdout.contains("=== VALIDATION corpus ==="), "{stdout}");
    assert!(stdout.contains("=== gap (validation - development) ==="), "{stdout}");
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
    assert!(text.contains("validation repo overall recall"), "{text}");
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

#[test]
fn bench_atlas_blind_prints_aggregates_only_and_writes_results_file() {
    // Blind protocol: validation corpus at <root>/benchmarks/holdout and
    // blind corpus at <root>/benchmarks/blind-test (both with ground
    // truth). Output must be aggregates ONLY: no per-repo rows, no
    // filenames, no missed keys; benchmarks/results/blind-v1.txt written.
    let ws = workspace();
    let tmp = tempfile::TempDir::new().unwrap();

    let validation = tmp.path().join("benchmarks").join("holdout");
    let validation_gt = tmp.path().join("benchmarks").join("holdout-ground-truth");
    let blind = tmp.path().join("benchmarks").join("blind-test");
    let blind_gt = tmp.path().join("benchmarks").join("blind-test-ground-truth");
    std::fs::create_dir_all(&validation).unwrap();
    std::fs::create_dir_all(&validation_gt).unwrap();
    std::fs::create_dir_all(&blind).unwrap();
    std::fs::create_dir_all(&blind_gt).unwrap();
    copy_tree(
        &ws.join("fixtures/http-service-python"),
        &validation.join("http-service-python"),
    );
    copy_tree(
        &ws.join("fixtures/http-service-python"),
        &blind.join("http-service-python"),
    );
    std::fs::write(
        validation_gt.join("http-service-python.md"),
        "## components\n- root\n- services\n## entrypoints\n- handle_transcripts\n## flows\n- TranscriptRepository\n## ownership\n- zzz_missing_store\n## contracts\n- GET /api/transcripts\n- GET /api/zzz_missing\n## tests\n- test_transcripts\n",
    )
    .unwrap();
    // slightly different ground truth so the gap is not trivially zero
    std::fs::write(
        blind_gt.join("http-service-python.md"),
        "## components\n- root\n- services\n## entrypoints\n- handle_transcripts\n## flows\n- TranscriptRepository\n## ownership\n- zzz_missing_store\n## contracts\n- GET /api/transcripts\n## tests\n- test_transcripts\n",
    )
    .unwrap();

    let out = std::process::Command::new(golden::scc())
        .args(["bench", "atlas", "--blind"])
        .current_dir(tmp.path())
        .output()
        .expect("scc bench atlas --blind runs");
    assert!(
        out.status.success(),
        "`scc bench atlas --blind` failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("aggregates only"), "{stdout}");
    assert!(
        stdout.contains("generalization gap (blind - validation)"),
        "{stdout}"
    );
    // Wave 11: the blind manifest hash and transfer ratios are printed
    assert!(
        stdout.contains("blind manifest sha256:"),
        "manifest hash in blind output: {stdout}"
    );
    assert!(
        stdout.contains("blind transfer ratio"),
        "transfer ratio in blind output: {stdout}"
    );
    // aggregates-only: the repo filename must never appear, and no per-repo
    // miss lines may leak
    assert!(
        !stdout.contains("http-service-python"),
        "no repo names in blind output: {stdout}"
    );
    assert!(
        !stdout.contains("missed:"),
        "no missed keys in blind output: {stdout}"
    );
    let results = tmp.path().join("benchmarks/results/blind-v1.txt");
    let text = std::fs::read_to_string(&results)
        .unwrap_or_else(|e| panic!("blind-v1.txt missing: {e}"));
    assert!(text.contains("aggregates only"), "{text}");
    assert!(
        text.contains("blind-test failures are never shown to tuning agents"),
        "{text}"
    );
    assert!(text.contains("overall (gate)"), "{text}");
    assert!(
        text.contains("blind manifest sha256:"),
        "manifest hash in results header: {text}"
    );
    assert!(
        text.contains("blind transfer ratio"),
        "transfer ratio in results file: {text}"
    );
    assert!(!text.contains("http-service-python"), "no repo rows: {text}");
    assert!(!text.contains("missed:"), "no missed keys: {text}");
}

#[test]
fn bench_atlas_blind_refuses_diagnose() {
    // The blind corpus is not diagnosable: diagnosis prints per-repo miss
    // lines, which would leak the blind misses.
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("benchmarks/holdout")).unwrap();
    std::fs::create_dir_all(tmp.path().join("benchmarks/holdout-ground-truth")).unwrap();
    std::fs::create_dir_all(tmp.path().join("benchmarks/blind-test")).unwrap();
    std::fs::create_dir_all(tmp.path().join("benchmarks/blind-test-ground-truth")).unwrap();

    let out = std::process::Command::new(golden::scc())
        .args(["bench", "atlas", "--blind", "--diagnose"])
        .current_dir(tmp.path())
        .output()
        .expect("scc bench atlas --blind --diagnose runs");
    assert!(!out.status.success(), "expected failure, got success");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("blind corpus is not diagnosable"),
        "clear error expected: {stderr}"
    );
}

#[test]
fn bench_atlas_blind_errors_when_manifest_changes() {
    // Wave 11: `--blind` verifies the sha256 manifest of the frozen blind
    // set (ground-truth keys + clone list) against the previous run before
    // scoring. A bogus previous hash (mismatch) must refuse to score with a
    // clear error — before any indexing.
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join("benchmarks/holdout")).unwrap();
    std::fs::create_dir_all(root.join("benchmarks/holdout-ground-truth")).unwrap();
    std::fs::create_dir_all(root.join("benchmarks/blind-test")).unwrap();
    std::fs::create_dir_all(root.join("benchmarks/blind-test-ground-truth")).unwrap();
    std::fs::write(
        root.join("benchmarks/blind-test-ground-truth/repo.md"),
        "## architecture\n- root\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("benchmarks/results")).unwrap();
    std::fs::write(
        root.join("benchmarks/results/blind-v1.txt"),
        "# Blind v1 — validation vs blind (aggregates only)\n\
         blind manifest sha256: 0000000000000000000000000000000000000000000000000000000000000000\n",
    )
    .unwrap();

    let out = std::process::Command::new(golden::scc())
        .args(["bench", "atlas", "--blind"])
        .current_dir(root)
        .output()
        .expect("scc bench atlas --blind runs");
    assert!(
        !out.status.success(),
        "expected failure on manifest mismatch, got success"
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("blind-test set changed"),
        "clear error expected: {stderr}"
    );
}

fn holdout_json(
    dev_overall: f64,
    dev_contracts: f64,
    val_overall: f64,
    val_contracts: f64,
) -> String {
    // Minimal but complete HoldoutComparison JSON (the serde shape the
    // `--holdout --json` path writes); sections default to the overall.
    format!(
        r#"{{
  "dev": {{"mode": "corpus: old", "repos": [], "mean_architecture": {d}, "mean_entrypoints": {d},
           "mean_behavior": {d}, "mean_state_authority": {d}, "mean_contracts": {dc},
           "mean_landmarks": 0.0, "mean_tests": 0.0, "mean_overall": {d},
           "mean_precision": 0.0, "mean_f2": 0.0, "mean_density": 0.0,
           "mean_atlas_tokens": 0.0, "scored": 1, "skipped": 0, "gate_passed": true}},
  "holdout": {{"mode": "validation: x", "repos": [], "mean_architecture": {v}, "mean_entrypoints": {v},
              "mean_behavior": {v}, "mean_state_authority": {v}, "mean_contracts": {vc},
              "mean_landmarks": 0.0, "mean_tests": 0.0, "mean_overall": {v},
              "mean_precision": 0.0, "mean_f2": 0.0, "mean_density": 0.0,
              "mean_atlas_tokens": 0.0, "scored": 1, "skipped": 0, "gate_passed": true}},
  "gap_architecture": 0.0, "gap_entrypoints": 0.0, "gap_behavior": 0.0,
  "gap_state_authority": 0.0, "gap_contracts": 0.0, "gap_overall": 0.0,
  "verdict": "NO_OVERFIT", "results_file": "old"
}}"#,
        d = dev_overall,
        dc = dev_contracts,
        v = val_overall,
        vc = val_contracts
    )
}

#[test]
fn bench_atlas_compare_applies_wave11_gates() {
    // Wave 11: `--compare OLD NEW` loads two saved holdout JSON results and
    // applies the GE gate + per-section regression guard. Generalizing run
    // (dev and validation both improve) passes; a validation section
    // regression beyond the guard fails with exit code 1.
    let tmp = tempfile::TempDir::new().unwrap();
    let old = tmp.path().join("old.json");
    let new_pass = tmp.path().join("new-pass.json");
    let new_fail = tmp.path().join("new-fail.json");
    std::fs::write(&old, holdout_json(0.50, 0.50, 0.50, 0.50)).unwrap();
    // dev 0.50 -> 0.56, validation 0.50 -> 0.53: GE = 0.5, no regressions
    std::fs::write(&new_pass, holdout_json(0.56, 0.56, 0.53, 0.53)).unwrap();
    // validation contracts regress 0.50 -> 0.30 (beyond the 0.05 guard)
    std::fs::write(&new_fail, holdout_json(0.56, 0.56, 0.51, 0.30)).unwrap();

    let scc = golden::scc();
    let pass = std::process::Command::new(scc)
        .args([
            "bench",
            "atlas",
            "--compare",
            old.to_str().unwrap(),
            new_pass.to_str().unwrap(),
        ])
        .current_dir(tmp.path())
        .output()
        .expect("scc bench atlas --compare passes");
    assert!(
        pass.status.success(),
        "generalizing run must pass: {}",
        String::from_utf8_lossy(&pass.stderr)
    );
    let stdout = String::from_utf8_lossy(&pass.stdout).to_string();
    assert!(stdout.contains("generalization efficiency"), "{stdout}");
    assert!(stdout.contains("0.500"), "GE 0.5 rendered: {stdout}");
    assert!(stdout.contains("verdict: PASS"), "{stdout}");

    let fail = std::process::Command::new(scc)
        .args([
            "bench",
            "atlas",
            "--compare",
            old.to_str().unwrap(),
            new_fail.to_str().unwrap(),
        ])
        .current_dir(tmp.path())
        .output()
        .expect("scc bench atlas --compare fails");
    assert!(
        !fail.status.success(),
        "regressing run must fail the guard"
    );
    let stderr = String::from_utf8_lossy(&fail.stderr).to_string();
    assert!(
        stderr.contains("generalization gates FAILED"),
        "gate failure surfaced: {stderr}"
    );

    // a missing/unparseable result file is a clear error
    let missing = std::process::Command::new(scc)
        .args([
            "bench",
            "atlas",
            "--compare",
            old.to_str().unwrap(),
            tmp.path().join("nope.json").to_str().unwrap(),
        ])
        .current_dir(tmp.path())
        .output()
        .expect("scc bench atlas --compare missing file");
    assert!(!missing.status.success());
    let stderr = String::from_utf8_lossy(&missing.stderr).to_string();
    assert!(stderr.contains("cannot read"), "{stderr}");
}
