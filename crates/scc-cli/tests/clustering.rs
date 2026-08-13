//! Semantic hierarchical clustering integration tests (generalization
//! wave): files belong to architecture because of BEHAVIOR, not
//! directories. End-to-end through `scc index` + `scc atlas` on synthetic
//! repos:
//! - two modules in the SAME top-level directory SPLIT into separate
//!   components when intra-dir cohesion is low;
//! - two modules in DIFFERENT directories MERGE into one component when
//!   call+state weight is high;
//! - a library repo's architecture comes from its EXPORTS (public surface
//!   graph, LibrarySdk archetype doubling);
//! - clustering is deterministic across identical recompiles.

mod golden;

use golden::run_ok;
use std::path::Path;


/// Two modules inside the same top-level `src/` directory with no
/// behavioral evidence between them: the clusterer SPLITS the directory
/// into two components — the old longest-prefix assignment would have
/// fused them into one `src` blob.
#[test]
// trace:v1 id=test.scc.clustering verifies=REQ-SCC-IR exercises=impl.scc.clustering
fn same_dir_modules_split_on_low_cohesion() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path().join("repo");
    write_split_fixture(&dir);

    run_ok(&dir, &["index", "--quiet"]);
    let atlas = run_ok(&dir, &["atlas"]);
    // both modules surface as their own components...
    assert!(atlas.contains("SRC/CHECKOUT"), "checkout module component: {atlas}");
    assert!(atlas.contains("SRC/PRICING"), "pricing module component: {atlas}");
    // ...and no fused `src` blob exists
    assert!(
        !atlas.contains("SRC\nImplementation: src"),
        "no directory-blob component: {atlas}"
    );
}

/// Two modules in different top-level directories with a semantic call
/// (+2) and shared store writes (+4): one component spanning both dirs —
/// behavior beats directory.
#[test]
fn cross_dir_modules_merge_on_call_and_state_weight() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path().join("repo");
    write_merge_fixture(&dir);

    run_ok(&dir, &["index", "--quiet"]);
    let atlas = run_ok(&dir, &["atlas"]);
    assert!(
        atlas.contains("AUTH+USERS"),
        "cross-dir merge into one component: {atlas}"
    );
    // the merged component keeps both dirs as implementation paths
    let idx = atlas.find("AUTH+USERS").unwrap();
    let window = &atlas[idx..idx + 600];
    assert!(window.contains("auth"), "auth dir kept as prior: {window}");
    assert!(window.contains("users"), "users dir kept as prior: {window}");
}

/// A library whose modules share no calls: the public surface graph
/// (EXPORT entities + IMPLEMENTS hierarchy + LibrarySdk doubling) is the
/// architecture — three exported classes implementing one exported
/// interface across three modules merge into one component.
#[test]
fn library_architecture_comes_from_exports() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path().join("repo");
    write_library_fixture(&dir);

    run_ok(&dir, &["index", "--quiet"]);
    let atlas = run_ok(&dir, &["atlas"]);
    assert!(atlas.contains("ARCHETYPE: library_sdk"), "library archetype: {atlas}");
    // the two implementation modules (exports consumed by the same
    // interface — the `iface` extension point) merge into one component
    assert!(
        atlas.contains("LIB\nPurpose:") || atlas.contains("LIB\nImplementation:"),
        "export-driven component: {atlas}"
    );
    // no per-impl shells survive the merge
    assert!(!atlas.contains("LIB/IMPL_A"), "no impl_a shell: {atlas}");
    assert!(!atlas.contains("LIB/IMPL_B"), "no impl_b shell: {atlas}");
    // the interface module itself stays a separate component (it exports
    // the facade — the cohesion rule binds the CONSUMERS of a facade)
    assert!(atlas.contains("LIB/CONTRACTS"), "interface module: {atlas}");
}

/// Two identical index+atlas runs produce byte-identical component
/// structure (names + layers), and re-indexing is idempotent.
#[test]
fn clustering_is_deterministic_across_recompiles() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path().join("repo");
    write_split_fixture(&dir);

    run_ok(&dir, &["index", "--quiet"]);
    let atlas1 = run_ok(&dir, &["atlas"]);
    run_ok(&dir, &["index", "--quiet"]);
    let atlas2 = run_ok(&dir, &["atlas"]);
    // the architecture is deterministic; the only difference between two
    // recompiles is the monotonic model-epoch generation counter
    let norm = |a: &str| {
        a.lines()
            .filter(|l| !l.starts_with("model_epoch_generations:"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(norm(&atlas1), norm(&atlas2), "atlas output must be deterministic");
    assert!(atlas1.contains("SRC/CHECKOUT"), "first run: {atlas1}");
    assert!(atlas2.contains("SRC/CHECKOUT"), "second run: {atlas2}");
}

/// Two low-cohesion modules in one dir plus two high-cohesion modules
/// across dirs in the same repo: the clusterer splits AND merges in one
/// pass (the split is not an artifact of the fixture shape).
#[test]
fn split_and_merge_coexist_in_one_repo() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path().join("repo");
    write_split_fixture(&dir);
    // add a merged pair under a different top dir
    std::fs::create_dir_all(dir.join("checkout")).unwrap();
    std::fs::create_dir_all(dir.join("cart")).unwrap();
    std::fs::write(
        dir.join("checkout/service.py"),
        "import sqlite3\n\ndef start_checkout(user):\n    conn = sqlite3.connect('shop.db')\n    conn.execute('INSERT INTO checkouts (user) VALUES (?)', (user,))\n    conn.commit()\n    conn.close()\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("cart/service.py"),
        "import sqlite3\nfrom checkout.service import start_checkout\n\ndef add_to_cart(item):\n    conn = sqlite3.connect('shop.db')\n    conn.execute('INSERT INTO cart (item) VALUES (?)', (item,))\n    conn.commit()\n    conn.close()\n    start_checkout('guest')\n",
    )
    .unwrap();
    run_ok(&dir, &["index", "--quiet"]);
    let atlas = run_ok(&dir, &["atlas"]);
    assert!(atlas.contains("SRC/CHECKOUT"), "split side: {atlas}");
    assert!(atlas.contains("SRC/PRICING"), "split side: {atlas}");
    assert!(atlas.contains("CART+CHECKOUT"), "merge side: {atlas}");
}

// ---------------------------------------------------------------------------
// fixture writers
// ---------------------------------------------------------------------------

fn write_split_fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("src/checkout")).unwrap();
    std::fs::create_dir_all(dir.join("src/pricing")).unwrap();
    std::fs::write(
        dir.join("src/checkout/cart.py"),
        "def add_item(item: str) -> None:\n    pass\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/pricing/tax.py"),
        "def compute_tax(amount: float) -> float:\n    return amount * 0.2\n",
    )
    .unwrap();
    std::fs::write(dir.join("README.md"), "# split fixture\n").unwrap();
}

fn write_merge_fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("auth")).unwrap();
    std::fs::create_dir_all(dir.join("users")).unwrap();
    std::fs::write(
        dir.join("auth/session.py"),
        "import sqlite3\nfrom users.api import get_user\n\ndef create_session(user_id: int) -> str:\n    conn = sqlite3.connect('app.db')\n    conn.execute('INSERT INTO sessions (user_id) VALUES (?)', (user_id,))\n    conn.commit()\n    conn.close()\n    get_user(user_id)\n    return 'tok'\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("users/api.py"),
        "import sqlite3\n\ndef get_user(user_id: int) -> dict:\n    conn = sqlite3.connect('app.db')\n    conn.execute('INSERT INTO access_log (user_id) VALUES (?)', (user_id,))\n    conn.commit()\n    conn.close()\n    return {'id': user_id}\n",
    )
    .unwrap();
    std::fs::write(dir.join("README.md"), "# merge fixture\n").unwrap();
}

fn write_library_fixture(dir: &Path) {
    // TypeScript: the extractor emits EXPORT entities for exported
    // classes/interfaces and REGISTERS "extension" facts when a class
    // implements a CROSS-FILE interface — the public-surface graph the
    // clusterer consumes (python does not emit hierarchy/registration
    // facts).
    std::fs::create_dir_all(dir.join("lib/contracts")).unwrap();
    std::fs::create_dir_all(dir.join("lib/impl_a")).unwrap();
    std::fs::create_dir_all(dir.join("lib/impl_b")).unwrap();
    std::fs::write(
        dir.join("lib/contracts/base.ts"),
        "export interface iface {\n  run(): void;\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("lib/impl_a/a.ts"),
        "import { iface } from '../contracts/base';\n\nexport class ImplA implements iface {\n  run(): void {}\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("lib/impl_b/b.ts"),
        "import { iface } from '../contracts/base';\n\nexport class ImplB implements iface {\n  run(): void {}\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("README.md"), "# library fixture\n").unwrap();
}
