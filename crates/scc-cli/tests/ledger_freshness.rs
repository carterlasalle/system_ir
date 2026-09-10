//! Regression tests for the Context Ledger's actual-visibility semantics
//! (mission §53) and post-mutation incremental freshness (§54).
//!
//! §53: the ledger records ONLY information actually shown to the model —
//! budget-dropped entries are never recorded as visible.
//! §54: after an edit + single-path incremental refresh, generated context
//! reflects the edit; deleted symbols do not appear as current truth.

use std::process::Command;

// trace:exempt reason=internal-detail  # binary locator for the regression suite
fn scc_bin() -> std::path::PathBuf {
    std::env::var("SCC_BIN")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/debug/scc")
        })
}

// trace:exempt reason=internal-detail  # CLI driver
fn run(dir: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new(scc_bin())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("spawn scc");
    assert!(
        out.status.success(),
        "`scc {}` failed:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// trace:exempt reason=internal-detail  # fixture factory
fn setup_fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn alpha() -> u32 { 1 }\n\
         pub fn beta() -> u32 { 2 }\n\
         pub fn gamma() -> u32 { 3 }\n\
         pub fn delta() -> u32 { 4 }\n\
         pub fn epsilon() -> u32 { 5 }\n\
         pub fn zeta() -> u32 { 6 }\n",
    )
    .unwrap();
    run(root, &["init"]);
    run(root, &["index", "--quiet"]);
    dir
}

/// The visible id sets recorded in the fixture's ledger, merged into one
/// lowercase blob (kind-scoped: symbols/files/components/flows).
// trace:exempt reason=internal-detail  # ledger reader
fn ledger_visible_blob(dir: &std::path::Path) -> String {
    let store = scc_store::Store::open(&dir.join(".scc").join("scc.db"), dir).unwrap();
    let (syms, files, comps, flows) =
        scc_context::context_ledger::ContextLedgerStore::new(&store).visible_ids();
    let mut all = String::new();
    for id in syms.iter().chain(files.iter()).chain(comps.iter()).chain(flows.iter()) {
        all.push_str(&id.to_lowercase());
        all.push('\n');
    }
    all
}

const SYMS: [&str; 6] = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta"];

#[test]
// trace:v1 id=test.scc-cli.ledger-freshness.task-ledger-visibility work=WORK-benchmark-infrastructure-regression-suite-ledger-visibility-incremental-freshness verifies=REQ-SCC-IR
fn task_ledger_records_only_what_survived_the_hard_cap() {
    let dir = setup_fixture();
    let root = dir.path();

    // Default task budget: the enriched pack renders the goal-matched
    // surface entries; the ledger records what survived all caps.
    let first = run(root, &["context", "task", "use alpha and beta"]);

    // §53 core: a recorded SYMBOL id whose name is absent from the
    // rendered artifact is a violation (budget-dropped ≠ visible).
    let blob = ledger_visible_blob(root);
    for sym in SYMS {
        if blob.contains(sym) {
            assert!(
                first.contains(sym),
                "ledger recorded symbol {sym} but the task artifact dropped it (§53)"
            );
        }
    }
    assert!(
        !blob.trim().is_empty(),
        "ledger must record the ids actually shown (§53: only visible entries)"
    );

    // The NEXT task still renders (ledger-aware delta path exercised).
    let second = run(root, &["context", "task", "use zeta and epsilon"]);
    assert!(second.contains("TASK"), "second pack must render: {second}");
}

#[test]
// trace:v1 id=test.scc-cli.ledger-freshness.startup-budget-omits-unshown work=WORK-benchmark-infrastructure-regression-suite-ledger-visibility-incremental-freshness verifies=REQ-SCC-IR
fn startup_ledger_omits_budget_dropped_entries() {
    let dir = setup_fixture();
    let root = dir.path();

    // A below-floor startup budget must NOT panic (§56) and must NOT
    // escape the cap: the emergency floor (atlas essentials + skeleton +
    // receipt) always fits the hard max (200 + max(200/5, 500) = 700).
    let out = run(root, &["context", "startup", "--budget", "200"]);
    assert!(
        !out.contains("BUDGET OVERFLOW"),
        "no overflow escape hatch: the artifact must fit the hard max"
    );
    let est = out.chars().count().div_ceil(4);
    assert!(
        est <= 700,
        "startup artifact must fit the hard max: ~{est} > 700"
    );
    let blob = ledger_visible_blob(root);

    // Ledger-vs-artifact coupling (§53): any recorded symbol id is in the
    // rendered text.
    for sym in SYMS {
        if blob.contains(sym) {
            assert!(
                out.contains(sym),
                "ledger recorded {sym} but the startup artifact dropped it (§53)"
            );
        }
    }
}

#[test]
// trace:v1 id=test.scc-cli.ledger-freshness.incremental-refresh-freshness work=WORK-benchmark-infrastructure-regression-suite-ledger-visibility-incremental-freshness verifies=REQ-SCC-IR
fn incremental_refresh_reflects_edits_and_deletions() {
    let dir = setup_fixture();
    let root = dir.path();

    let before = run(root, &["context", "task", "all functions"]);
    assert!(before.contains("alpha"), "baseline must include alpha");
    assert!(!before.contains("omega"), "omega must not exist yet");

    // Edit: add a new exported symbol, refresh the single path.
    let lib = root.join("src/lib.rs");
    let text = std::fs::read_to_string(&lib).unwrap();
    std::fs::write(
        &lib,
        text.replace(
            "pub fn zeta() -> u32 { 6 }",
            "pub fn zeta() -> u32 { 6 }\npub fn omega() -> u32 { 7 }",
        ),
    )
    .unwrap();
    run(root, &["index", "--paths", "src/lib.rs", "--quiet"]);

    // §54: the second context reflects the edit, not the stale snapshot.
    let after = run(root, &["context", "task", "all functions"]);
    assert!(
        after.contains("omega"),
        "context after incremental refresh must reflect the edit (§54): {after}"
    );

    // Deletion: symbols removed from the path must not appear as current
    // truth after refresh.
    std::fs::write(
        &lib,
        "pub fn alpha() -> u32 { 1 }\npub fn omega() -> u32 { 7 }\n",
    )
    .unwrap();
    run(root, &["index", "--paths", "src/lib.rs", "--quiet"]);
    let after_del = run(root, &["context", "task", "all functions"]);
    assert!(
        !after_del.contains("zeta()"),
        "deleted symbol must not appear as current truth (§54): {after_del}"
    );
    assert!(
        after_del.contains("omega"),
        "surviving symbol still served: {after_del}"
    );
}
