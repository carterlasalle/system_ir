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

/// Wave 13: a FLAT library package (a workspace member whose direct files
/// are its modules) is no longer an indivisible atom. The region hierarchy
/// starts one level below the package dir, and package membership is only
/// a +5 cohesion signal — below MERGE_THRESHOLD — so modules with NO cross
/// evidence split into separate components instead of one fused blob.
#[test]
// trace:exempt reason=unit-test
fn flat_library_package_splits_into_unrelated_modules() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path().join("repo");
    write_flat_package_fixture(&dir);

    run_ok(&dir, &["index", "--quiet"]);
    let comps = run_ok(&dir, &["components"]);
    let lines: Vec<&str> = comps.lines().filter(|l| !l.is_empty()).collect();
    assert!(
        lines.len() >= 2,
        "flat package must split into 2+ components: {comps}"
    );
    for n in ["src/core.py", "src/parser.py", "src/termui.py"] {
        assert!(lines.contains(&n), "module component {n}: {comps}");
    }
    assert!(
        !lines.contains(&"src"),
        "no fused package blob: {comps}"
    );
}

/// Wave 13: single-link chaining is blocked. Four clusters A-B-C-D with
/// ONE strong edge (call+state = 6) between consecutive clusters only:
/// pure max-linkage would collapse the whole chain into one component.
/// The cohesion-aware acceptance (avg >= 0.4 * max over ALL cross pairs)
/// stops the chain at the third link — {A,B,C} vs {D} has avg 6/3 = 2 <
/// 2.4 — so the repo keeps >= 2 components.
#[test]
// trace:exempt reason=unit-test
fn single_link_chaining_is_blocked() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path().join("repo");
    write_chain_fixture(&dir);

    run_ok(&dir, &["index", "--quiet"]);
    let comps = run_ok(&dir, &["components"]);
    let lines: Vec<&str> = comps.lines().filter(|l| !l.is_empty()).collect();
    assert!(
        lines.len() >= 2,
        "chained clusters must not collapse into one component: {comps}"
    );
    assert!(
        lines.contains(&"a+b+c"),
        "the first three links cohere (avg 3 >= 2.4): {comps}"
    );
    assert!(
        lines.contains(&"d"),
        "the tail link must NOT chain on a single edge: {comps}"
    );
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

/// A flat library package: a workspace member (`src`) whose direct files
/// are its modules, with NO cross imports/calls between them. Package
/// membership (+5) is the only evidence, below MERGE_THRESHOLD — the
/// clusterer must split the package into per-module components.
// trace:exempt reason=unit-test
fn write_flat_package_fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("package.json"),
        "{\n  \"name\": \"flatlib\",\n  \"workspaces\": [\"src\"]\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/core.py"),
        "def core_fn() -> str:\n    return 'core'\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/parser.py"),
        "def parse_arg(raw: str) -> str:\n    return raw.strip()\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/termui.py"),
        "def render_line(text: str) -> str:\n    return f'[{text}]'\n",
    )
    .unwrap();
    std::fs::write(dir.join("README.md"), "# flat package fixture\n").unwrap();
}

/// Four chained regions a -> b -> c -> d, ONE strong edge (call + shared
/// store = 6) between consecutive regions only. Max-linkage alone would
/// collapse a+b+c+d; the cohesion-aware acceptance stops the chain at
/// {a,b,c} vs {d} (avg 6/3 = 2 < 0.4 * 6 = 2.4). NOTE: the extractors are
/// busy — module-level defs are exported (public_api entrypoints that seed
/// flows, a full flow clique), and store writes key on the connection
/// RECEIVER name — so the fixture uses private `_`-prefixed functions and
/// distinct receivers (cursor/engine/pool) per shared store to keep the
/// graph to exactly the intended 6-weight chain edges.
// trace:exempt reason=unit-test
fn write_chain_fixture(dir: &Path) {
    for d in ["a", "b", "c", "d"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    std::fs::write(
        dir.join("a/x.py"),
        "import sqlite3\nfrom b.x import _b_run\n\ndef _a_run(user: str) -> str:\n    cursor = sqlite3.connect('db1.db')\n    cursor.execute('INSERT INTO a (user) VALUES (?)', (user,))\n    cursor.commit()\n    cursor.close()\n    _b_run(user)\n    return 'a'\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("b/x.py"),
        "import sqlite3\nfrom c.x import _c_run\n\ndef _b_run(user: str) -> str:\n    cursor = sqlite3.connect('db1.db')\n    cursor.execute('INSERT INTO b (user) VALUES (?)', (user,))\n    cursor.commit()\n    cursor.close()\n    engine = sqlite3.connect('db2.db')\n    engine.execute('INSERT INTO b (user) VALUES (?)', (user,))\n    engine.commit()\n    engine.close()\n    _c_run(user)\n    return 'b'\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("c/x.py"),
        "import sqlite3\nfrom d.x import _d_run\n\ndef _c_run(user: str) -> str:\n    engine = sqlite3.connect('db2.db')\n    engine.execute('INSERT INTO c (user) VALUES (?)', (user,))\n    engine.commit()\n    engine.close()\n    pool = sqlite3.connect('db3.db')\n    pool.execute('INSERT INTO c (user) VALUES (?)', (user,))\n    pool.commit()\n    pool.close()\n    _d_run(user)\n    return 'c'\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("d/x.py"),
        "import sqlite3\n\ndef _d_run(user: str) -> str:\n    pool = sqlite3.connect('db3.db')\n    pool.execute('INSERT INTO d (user) VALUES (?)', (user,))\n    pool.commit()\n    pool.close()\n    return 'd'\n",
    )
    .unwrap();
    std::fs::write(dir.join("README.md"), "# chain fixture\n").unwrap();
}
