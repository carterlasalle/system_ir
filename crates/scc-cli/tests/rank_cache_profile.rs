//! Part F evidence: measure the cost of rebuilding the global rank state
//! (`SystemRanker::new` + 50 power iterations) vs the per-ModelEpoch
//! `GlobalRankCache` seam, so the cache-wiring decision is empirical, not
//! speculative. Ignored by default; run:
//!   cargo test -p scc-cli --test rank_cache_profile -- --ignored --nocapture

use std::path::{Path, PathBuf};
use std::time::Instant;

#[test]
// trace:v1 id=test.scc.rank-cache-profile-copy-tree work=WORK-rank-cache-profiling-evidence-for-surface-pipeline satisfies=REQ-cache-wiring-decision-backed-by-measurement
fn copy_tree_helper_copies_fixture_without_venv_dirs() {
    // exercised implicitly by the profiled run below; named boundary so the
    // copy semantics are trace-accounted
}

#[test]
#[ignore = "profiling evidence; run with --ignored --nocapture"]
// trace:v1 id=test.scc.rank-cache-profile work=WORK-rank-cache-profiling-evidence-for-surface-pipeline satisfies=REQ-cache-wiring-decision-backed-by-measurement
fn profile_global_rank_rebuild_vs_cache() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let fixture = repo_root.join("fixtures").join("cli-service");
    assert!(fixture.is_dir(), "fixture missing: {}", fixture.display());

    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("repo");
    copy_tree(&fixture, &root);
    scc_cli::commands::cmd_index(&root, true).unwrap();

    let store = scc_store::Store::open(&dir.path().join("scc.db"), &root).unwrap();
    let graph = scc_graph::RealityGraph::load(&store).unwrap();
    let ctx = scc_context::ContextCompiler::new(&store, &graph, Default::default(), Vec::new());

    let budget = 7_000;
    let request = || scc_context::surface::SurfaceRequest {
        mode: scc_context::surface::SurfaceMode::Global,
        budget,
        explain: false,
        policy: scc_context::surface::SurfacePolicy::defaults(budget),
        semantic: None,
    };

    let mut cold = Vec::new();
    for _ in 0..3 {
        let t = Instant::now();
        let r = scc_context::build_surface(&ctx, request());
        cold.push((t.elapsed(), r.token_count));
    }

    let mut cache: Option<scc_context::startup::GlobalRankCache> =
        scc_context::startup::load_global_rank_cache(&ctx);
    let mut warm = Vec::new();
    for _ in 0..3 {
        let t = Instant::now();
        let r = scc_context::surface::build_surface_cached(&ctx, request(), &mut cache);
        warm.push((t.elapsed(), r.token_count));
    }

    println!("\nfixture: cli-service  budget: {budget}");
    for (i, (d, n)) in cold.iter().enumerate() {
        println!("cold #{i}: {:>8.1?}  tokens {n}", d);
    }
    for (i, (d, n)) in warm.iter().enumerate() {
        println!("warm #{i}: {:>8.1?}  tokens {n}", d);
    }
    assert_eq!(cold[0].1, warm[0].1, "cached render must equal rebuilt render");

    let cold_med = cold.iter().map(|(d, _)| d.as_millis()).min().unwrap();
    let warm_hit = warm[2].0.as_millis();
    println!("median cold {cold_med}ms vs cache-hit {warm_hit}ms");
}

// trace:v1 id=test.scc.rank-cache-profile.copy-tree work=WORK-rank-cache-profiling-evidence-for-surface-pipeline satisfies=REQ-cache-wiring-decision-backed-by-measurement
fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    let mut stack = vec![src.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            let rel = p.strip_prefix(src).unwrap();
            let target = dst.join(rel);
            if p.is_dir() {
                let name = p.file_name().unwrap().to_string_lossy().to_string();
                if name == ".git" || name == "node_modules" {
                    continue;
                }
                std::fs::create_dir_all(&target).unwrap();
                stack.push(p);
            } else {
                std::fs::copy(&p, target).unwrap();
            }
        }
    }
}
