//! `scc resolve --lsp` end-to-end test (Phase 7, EPIC-120): a package living
//! in `lib/` is invisible to the native module resolver, so its call edges are
//! stored as EXTRACTED `external_api` edges; pyright (with `extraPaths` from
//! pyrightconfig.json) resolves the call through the `__init__.py` re-export
//! to the real implementation. After `scc resolve --lsp` the exported IR must
//! carry a RESOLVED `calls` edge to the true symbol with `lsp-pyright`
//! evidence.
//!
//! The test skips gracefully when pyright is not installed.

use scc_cli::{export_ir, index_and_recompile, load_config, open_store, scc_dir};
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

#[test]
fn resolve_lsp_upgrades_reexported_call_edges() {
    if !pyright_available() {
        eprintln!("pyright not installed — skipping resolve --lsp test");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();

    write(&root, "pyrightconfig.json", r#"{"extraPaths": ["lib"]}"#);
    write(&root, "lib/helper_pkg/__init__.py", "from .impl import helper\n");
    write(&root, "lib/helper_pkg/impl.py", "def helper():\n    return 1\n");
    write(
        &root,
        "b.py",
        "from helper_pkg import helper\n\n\ndef main():\n    helper()\n",
    );

    // 1. index the fixture; the native resolver must store an EXTRACTED edge
    //    to external_api for the call.
    let config = load_config(&root).unwrap();
    index_and_recompile(&root, &config).unwrap();
    let store = open_store(&root).unwrap();
    let repo = store.repository().id;
    let external = scc_core::entity_id(&repo, scc_core::kinds::EXTERNAL_API, "helper-pkg");
    let pre: Vec<_> = store
        .all_relationships()
        .unwrap()
        .into_iter()
        .filter(|r| r.predicate == scc_core::predicates::CALLS)
        .collect();
    assert_eq!(pre.len(), 1, "fixture must yield one EXTRACTED call edge");
    assert_eq!(pre[0].object, external);
    assert_eq!(pre[0].provenance, scc_core::Provenance::Extracted);

    // 2. resolve with the LSP adapter.
    scc_cli::resolve::cmd_resolve_lsp(&root).unwrap();

    // 3. the exported IR carries the upgraded edge with lsp-pyright evidence.
    let store = open_store(&root).unwrap();
    let ir = export_ir(&store).unwrap();
    let target = scc_core::symbol_id(&repo, "lib/helper_pkg/impl.py", "helper");
    let subject = scc_core::symbol_id(&repo, "b.py", "main");

    let upgraded: Vec<_> = ir
        .relationships
        .iter()
        .filter(|r| r.predicate == scc_core::predicates::CALLS)
        .collect();
    assert_eq!(upgraded.len(), 1, "old edge replaced, new edge present");
    let rel = &upgraded[0];
    assert_eq!(rel.subject, subject);
    assert_eq!(rel.object, target, "must point at the real implementation");
    assert_eq!(rel.provenance, scc_core::Provenance::Resolved);
    assert_eq!(rel.confidence, 0.99);
    assert!(!rel.evidence.is_empty(), "RESOLVED edges must carry evidence");

    let ev = store
        .get_evidence(&rel.evidence[0])
        .unwrap()
        .expect("evidence row exists");
    assert_eq!(ev.extractor.as_deref(), Some(scc_indexer::lsp::LSP_EXTRACTOR));
    assert_eq!(ev.symbol.as_deref(), Some("helper"));
    assert_eq!(ev.start_line, Some(5));
    assert!(
        ev.extractor_version.as_deref().is_some_and(|v| !v.is_empty()),
        "evidence must carry the pyright version"
    );

    // no EXTRACTED external_api edge remains
    assert!(!ir
        .relationships
        .iter()
        .any(|r| r.provenance == scc_core::Provenance::Extracted
            && r.predicate == scc_core::predicates::CALLS));
    let _ = scc_dir(&root);
}

#[test]
fn resolve_lsp_without_index_reports_clearly() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    let err = scc_cli::resolve::cmd_resolve_lsp(&root).unwrap_err();
    assert!(
        err.to_string().contains("no index found"),
        "unexpected error: {err}"
    );
}
