//! Performance targets (docs/TEST_PLAN.md §16): 50k LOC cold index.
//!
//! Do **not** `mod golden` here. Cargo compiles that file as a submodule of
//! this binary, so every `#[test]` in golden.rs would run again in parallel
//! with the wall-clock gate and steal CPU on shared GHA runners.
//!
//! The bound is the documented 30s target. GHA VMs are not a calibrated
//! clock: a same-SHA run was 13s on the PR job and 39s on the push job.
//! Retry once before failing so a noisy neighbor is not a product failure.

use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

// trace:exempt reason=internal-helper
fn scc() -> &'static str {
    env!("CARGO_BIN_EXE_scc")
}

// trace:exempt reason=internal-helper
fn workdir(tmp: &Path) -> std::path::PathBuf {
    tmp.join("repo")
}

// trace:exempt reason=internal-helper
fn run_ok(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(scc())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("scc binary runs");
    assert!(
        out.status.success(),
        "`scc {}` failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

// trace:exempt reason=internal-helper
fn generate(dir: &Path, files: usize, lines: usize) -> usize {
    std::fs::create_dir_all(dir).unwrap();
    let mut loc = 0;
    for i in 0..files {
        let name = format!("mod_{i:04}");
        let mut body = format!("# module {name}\n");
        if i > 0 {
            body.push_str(&format!("from mod_{:04} import helper\n", i - 1));
        }
        let mut line = 3;
        for s in 0..(lines / 10).max(2) {
            body.push_str(&format!(
                "def func_{s:03}(a, b):\n    r = a + {s}\n    if r > 0:\n        return helper(r)\n    return r\n"
            ));
            line += 4;
        }
        while line < lines {
            body.push_str("# pad\n");
            line += 1;
        }
        loc += line;
        let mut f = std::fs::File::create(dir.join(format!("{name}.py"))).unwrap();
        f.write_all(body.as_bytes()).unwrap();
    }
    loc
}

// trace:exempt reason=internal-helper
fn cold_index_once() -> (Duration, usize, String) {
    let repo = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(workdir(repo.path())).unwrap();
    let loc = generate(&workdir(repo.path()), 200, 250);
    let start = Instant::now();
    run_ok(&workdir(repo.path()), &["index", "--quiet"]);
    let elapsed = start.elapsed();
    let status = run_ok(&workdir(repo.path()), &["status"]);
    (elapsed, loc, status)
}

#[test]
// trace:v1 id=test.scc-cli.perf.cold-index-50k verifies=REQ-SCC-TEST
fn cold_index_50k_loc_under_30s() {
    let bound = Duration::from_secs(30);
    let (first, loc, status) = cold_index_once();
    assert!(loc >= 50_000, "generated {loc} LOC");
    let (elapsed, status) = if first < bound {
        (first, status)
    } else {
        let (retry, _, status) = cold_index_once();
        assert!(
            retry < bound,
            "cold index of {loc} LOC took {first:?} then {retry:?} (docs target <30s)"
        );
        (retry, status)
    };
    eprintln!("50k LOC cold index: {elapsed:?}");
    assert!(status.contains("relationships:"), "{status}");
}

/// 250k LOC cold index (SCC-241): 1000 files x 250 lines, 120s bound.
/// Manual run:
///   cargo test -p scc-cli --test perf cold_index_250k -- --ignored --nocapture
#[test]
#[ignore]
// trace:v1 id=test.scc-cli.perf.cold-index-250k verifies=REQ-SCC-TEST
fn cold_index_250k_loc() {
    let repo = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(workdir(repo.path())).unwrap();
    let loc = generate(&workdir(repo.path()), 1000, 250);
    assert!(loc >= 250_000, "generated {loc} LOC");
    let start = Instant::now();
    run_ok(&workdir(repo.path()), &["index", "--quiet"]);
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_secs() < 120,
        "cold index of {loc} LOC took {elapsed:?} (120s bound)"
    );
    eprintln!("250k LOC cold index: {elapsed:?}");
}
