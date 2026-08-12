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
    assert!(stdout.contains("components"), "{stdout}");
    assert!(stdout.contains("entrypoints"), "{stdout}");
    assert!(stdout.contains("overall"), "{stdout}");
    assert!(stdout.contains("http-service-python"), "{stdout}");
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
    // the deliberately nonexistent ground-truth items are reported as missed
    assert!(
        stdout.contains("ownership:zzz_missing_store"),
        "missed items must be listed: {stdout}"
    );
}
