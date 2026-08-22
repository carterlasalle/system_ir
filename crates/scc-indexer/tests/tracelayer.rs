//! Integration tests for the TraceLayer adapter.

use scc_indexer::adapters::tracelayer::import_tracelayer;
use scc_store::Store;
use std::fs;
use tempfile::TempDir;

#[test]
// trace:v1 id=test.scc.tracelayer.basic work=WORK-trace-layer-adapter-for-system-ir satisfies=REQ-SCC-IR
fn test_import_tracelayer_basic() {
    let tmp_dir = TempDir::new().unwrap();
    let db_path = tmp_dir.path().join("test.db");
    let root_path = tmp_dir.path();
    let store = Store::open(&db_path, root_path).unwrap();
    let test_file = tmp_dir.path().join("test.rs");
    fs::write(&test_file, "// trace:v1 id=REQ-TEST-001 type=requirement\npub fn f() {}\n").unwrap();
    let report = import_tracelayer(&store, &test_file).unwrap();
    assert_eq!(report.requirements, 1);
}
