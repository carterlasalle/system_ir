//! Differential resolution benchmark integration tests (SCC-126): the
//! re-export fixture must yield >= 1 LSP upgrade with the gate passing, and
//! the conflict model (SCC-125) must record drift findings for target
//! changes.

use std::path::Path;

fn pyright_available() -> bool {
    std::process::Command::new("pyright-langserver")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, content).unwrap();
}

fn summary_of(repos: Vec<scc_cli::benchres::RepoResolution>) -> scc_cli::benchres::ResolutionSummary {
    let mut s = scc_cli::benchres::ResolutionSummary {
        repos,
        ..Default::default()
    };
    for r in &s.repos {
        s.total_resolved += r.native_resolved;
        s.total_external += r.native_external;
        s.total_upgrades += r.lsp_upgrades;
        s.total_unresolved += r.lsp_unresolved;
        s.total_agreement += r.agreement;
        s.total_conflicts += r.conflicts;
    }
    s
}

#[test]
fn differential_resolution_upgrades_reexport_fixture() {
    if !pyright_available() {
        eprintln!("pyright not installed — skipping differential benchmark test");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();

    // The python re-export fixture: `lib/helper_pkg` is invisible to the
    // native module resolver (pyrightconfig extraPaths), so the call edge is
    // stored EXTRACTED against external_api; pyright resolves it through the
    // `__init__.py` re-export to the real implementation.
    write(&root, "pyrightconfig.json", r#"{"extraPaths": ["lib"]}"#);
    write(&root, "lib/helper_pkg/__init__.py", "from .impl import helper\n");
    write(&root, "lib/helper_pkg/impl.py", "def helper():\n    return 1\n");
    write(
        &root,
        "b.py",
        "from helper_pkg import helper\n\n\ndef main():\n    helper()\n",
    );

    let r = scc_cli::benchres::diff_repo(&root).unwrap();
    assert_eq!(r.native_external, 1, "fixture must store one EXTRACTED edge");
    assert!(
        r.lsp_upgrades >= 1,
        "the re-export fixture must yield >= 1 upgrade: {r:?}"
    );
    assert_eq!(r.lsp_unresolved, 0, "the only candidate must resolve: {r:?}");
    assert_eq!(
        r.agreement, r.native_resolved,
        "native RESOLVED edges must be left untouched: {r:?}"
    );
    assert!(r.conflicts >= 1, "external -> symbol is a target change: {r:?}");

    // SCC-125: the conflict must be persisted as a drift finding
    let store = scc_cli::open_store(&root).unwrap();
    let findings = store.drift_findings(false).unwrap();
    let conflicts: Vec<_> = findings
        .iter()
        .filter(|(_, kind, _, _, _)| kind == "resolution_conflict")
        .collect();
    assert!(!conflicts.is_empty(), "resolution_conflict drift findings expected");
    let msg = &conflicts[0].3;
    assert!(msg.contains("b.py:5 call to helper"), "{msg}");
    assert!(msg.contains("resolved by LSP to"), "{msg}");
    assert!(msg.contains("native index had"), "{msg}");

    // SCC-126: the gate passes for this differential
    let summary = summary_of(vec![r]);
    scc_cli::benchres::check_gate(&summary, 0.3).expect("gate must pass");
}

#[test]
fn gate_passes_when_native_covers_everything() {
    // zero upgrades is healthy when the native resolver already covers the
    // externals an LSP would resolve (third-party libs stay external)
    let repo = scc_cli::benchres::RepoResolution {
        repo: "x".into(),
        native_external: 10,
        lsp_upgrades: 0,
        lsp_unresolved: 9, // 90% < 95% limit
        ..Default::default()
    };
    let summary = summary_of(vec![repo]);
    assert!(scc_cli::benchres::check_gate(&summary, 0.95).is_ok());
}

#[test]
fn gate_rejects_resolution_conflicts() {
    let repo = scc_cli::benchres::RepoResolution {
        repo: "x".into(),
        native_external: 1,
        lsp_upgrades: 1,
        lsp_unresolved: 0,
        conflicts: 1,
        ..Default::default()
    };
    let summary = summary_of(vec![repo]);
    let err = scc_cli::benchres::check_gate(&summary, 0.3).unwrap_err();
    assert!(err.contains("resolution conflict"), "{err}");
}

#[test]
fn gate_rejects_high_unresolved_ratio() {
    // 1 upgrade, but 8 of 9 external candidates stay unresolved (0.89 >= 0.3)
    let repo = scc_cli::benchres::RepoResolution {
        repo: "x".into(),
        native_external: 9,
        lsp_upgrades: 1,
        lsp_unresolved: 8,
        ..Default::default()
    };
    let summary = summary_of(vec![repo]);
    let err = scc_cli::benchres::check_gate(&summary, 0.3).unwrap_err();
    assert!(err.contains("gate failed"), "{err}");
    assert!(err.contains("88.9%"), "{err}");

    // same differential passes with a looser limit
    scc_cli::benchres::check_gate(&summary, 0.9).expect("looser limit accepts");
}

#[test]
fn corpus_runs_to_completion() {
    // The corpus benchmark must run every repo. Whether the gate passes
    // depends on the fixture corpus containing LSP-resolvable externals, so
    // accept both outcomes but require a clear gate diagnosis on failure.
    match scc_cli::benchres::run_resolution_benchmark(0.3) {
        Ok(summary) => {
            assert!(!summary.repos.is_empty());
            assert!(summary.total_upgrades >= 1);
        }
        Err(e) => {
            assert!(e.contains("gate failed"), "unexpected corpus error: {e}");
        }
    }
}
