//! Performance targets (docs/TEST_PLAN.md §16): 50k LOC cold index.
//! Runs against the release-style path via the test binary; the bound is
//! deliberately generous for debug builds (docs target: <30s).

mod golden;
use golden::*;
use std::io::Write;

fn generate(dir: &std::path::Path, files: usize, lines: usize) -> usize {
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

#[test]
fn cold_index_50k_loc_under_30s() {
    let repo = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(workdir(repo.path())).unwrap();
    let loc = generate(&workdir(repo.path()), 200, 250);
    assert!(loc >= 50_000, "generated {loc} LOC");
    let start = std::time::Instant::now();
    run_ok(&workdir(repo.path()), &["index", "--quiet"]);
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_secs() < 30,
        "cold index of {loc} LOC took {elapsed:?} (docs target <30s)"
    );
    eprintln!("50k LOC cold index: {elapsed:?}");
    let status = run_ok(&workdir(repo.path()), &["status"]);
    assert!(status.contains("relationships:"), "{status}");
}

/// 250k LOC cold index (SCC-241): 1000 files x 250 lines, 120s bound.
/// Manual run:
///   cargo test -p scc-cli --test perf cold_index_250k -- --ignored --nocapture
#[test]
#[ignore]
fn cold_index_250k_loc() {
    let repo = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(workdir(repo.path())).unwrap();
    let loc = generate(&workdir(repo.path()), 1000, 250);
    assert!(loc >= 250_000, "generated {loc} LOC");
    let start = std::time::Instant::now();
    run_ok(&workdir(repo.path()), &["index", "--quiet"]);
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_secs() < 120,
        "cold index of {loc} LOC took {elapsed:?} (120s bound)"
    );
    eprintln!("250k LOC cold index: {elapsed:?}");
}
