//! Incremental invalidation cascade: hash-unchanged importers must be
//! re-extracted when a callee changes so type-narrowed CALLS match a cold
//! index (docs/DATA_STRATEGY.md §6).

use scc_indexer::{Config, Indexer};
use scc_store::Store;
use std::path::Path;
use tempfile::TempDir;

fn indexer_for(root: &Path) -> (Indexer, TempDir) {
    let tmp = TempDir::new().unwrap();
    let store = Store::open(&tmp.path().join("scc.db"), root).unwrap();
    (Indexer::new(store, Config::default()), tmp)
}

fn call_objects(idx: &Indexer, caller_file: &str, caller: &str) -> Vec<String> {
    let sid = scc_core::symbol_id(&idx.store.repo_id, caller_file, caller);
    let mut objs: Vec<String> = idx
        .store
        .all_relationships()
        .unwrap()
        .into_iter()
        .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == sid)
        .map(|r| r.object)
        .collect();
    objs.sort();
    objs
}

#[test]
// trace:v1 id=test.scc.index.invalidation-cascade verifies=REQ-implement-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing exercises=impl.scc.index.invalidation-cascade
fn incremental_refresh_reresolves_importers_to_match_cold() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("order.py"),
        "class Order:\n    def process(self):\n        return 1\n",
    )
    .unwrap();
    std::fs::write(
        root.join("app.py"),
        "from order import Order\n\ndef handle():\n    x = Order()\n    return x.process()\n",
    )
    .unwrap();

    let (idx, _db) = indexer_for(root);
    idx.index().unwrap();
    let order_m = scc_core::symbol_id(&idx.store.repo_id, "order.py", "Order.process");
    assert!(
        call_objects(&idx, "app.py", "handle").contains(&order_m),
        "cold type-narrow must pin handle → Order.process"
    );

    // Removing the class is visible only if app.py is re-resolved.
    std::fs::write(root.join("order.py"), "def helper():\n    return 1\n").unwrap();
    idx.refresh_paths(&["order.py".into()]).unwrap();
    assert!(
        !call_objects(&idx, "app.py", "handle").contains(&order_m),
        "stale CALL to removed Order.process must not survive incremental refresh"
    );
    let helper = scc_core::symbol_id(&idx.store.repo_id, "order.py", "helper");
    let mut names: Vec<String> = idx
        .store
        .symbols_in_file("order.py")
        .unwrap()
        .into_iter()
        .map(|(_, n, _, _, _, _, _, _)| n)
        .collect();
    names.sort();
    assert_eq!(names, vec!["helper".to_string()]);
    assert!(idx.store.get_entity(&helper).unwrap().is_some());
    assert!(idx.store.get_entity(&order_m).unwrap().is_none());
}

#[test]
// trace:v1 id=test.scc.index.invalidation-cascade-add verifies=REQ-implement-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing exercises=impl.scc.index.invalidation-cascade
fn incremental_refresh_creates_type_narrow_calls_when_callee_gains_method() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    std::fs::write(root.join("order.py"), "class Order:\n    pass\n").unwrap();
    std::fs::write(
        root.join("app.py"),
        "from order import Order\n\ndef handle():\n    x = Order()\n    return x.process()\n",
    )
    .unwrap();

    let (idx, _db) = indexer_for(root);
    idx.index().unwrap();
    let order_m = scc_core::symbol_id(&idx.store.repo_id, "order.py", "Order.process");
    assert!(
        !call_objects(&idx, "app.py", "handle").contains(&order_m),
        "no method yet: must not invent a CALL"
    );

    std::fs::write(
        root.join("order.py"),
        "class Order:\n    def process(self):\n        return 1\n",
    )
    .unwrap();
    idx.refresh_paths(&["order.py".into()]).unwrap();
    assert!(
        call_objects(&idx, "app.py", "handle").contains(&order_m),
        "refreshing the callee must re-resolve the importer's type-narrowed CALL"
    );
}
