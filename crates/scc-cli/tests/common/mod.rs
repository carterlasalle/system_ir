//! Shared helpers for scc-cli integration tests.
//!
//! This is a **module**, not a test binary: it lives under `tests/common/`
//! so Cargo does not auto-discover it. Other integration tests `mod common`
//! for `copy_fixture` / `run_ok` without compiling `golden.rs` (and its
//! `#[test]`s) into every binary.

#![allow(dead_code)]

use std::path::PathBuf;
use std::process::Command;

// trace:v1 id=impl.scc-cli.tests.common-helpers work=WORK-stop-re-running-the-golden-integration-suite-in-every-scc-cli-test-binar satisfies=REQ-implement-stop-re-running-the-golden-integration-suite-in-every-scc-cl implements=PLAN-stop-re-running-the-golden-integration-suite-in-every-scc-cli-test-binar
pub struct IntegrationHelpers;

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
        "if" | "else"
            | "for"
            | "while"
            | "try"
            | "catch"
            | "match"
            | "switch"
            | "with"
            | "do"
            | "loop"
            | "select"
    ) || c.starts_with("conditional:")
}
