//! Go extractor end-to-end tests (Wave 7 §46): indexing a small Go service
//! must surface the main entrypoint in flows/atlas, symbols in the System IR
//! export, and the store write symbol in a task pack for a "store" goal.

mod golden;
use golden::*;

fn go_service() -> tempfile::TempDir {
    let repo = copy_fixture("go-service");
    run_ok(&workdir(repo.path()), &["index", "--quiet"]);
    repo
}

#[test]
fn go_index_flows_mention_main() {
    let repo = go_service();
    let flows = run_ok(&workdir(repo.path()), &["flows"]);
    assert!(flows.contains("main"), "flows must mention main: {flows}");
}

#[test]
fn go_atlas_has_service_component_and_main_entrypoint() {
    let repo = go_service();
    let atlas = run_ok(&workdir(repo.path()), &["atlas"]);
    assert!(atlas.contains("ENTRYPOINTS"), "entrypoints section: {atlas}");
    assert!(atlas.contains("main"), "main entrypoint missing: {atlas}");
    // the service lives under internal/, which compiles to the "internal"
    // component (implementation listed as its path)
    assert!(atlas.contains("INTERNAL"), "service component missing: {atlas}");
    assert!(
        atlas.contains("Implementation: internal"),
        "service component implementation missing: {atlas}"
    );
}

#[test]
fn go_export_has_symbols_from_main_go() {
    let repo = go_service();
    let out = run_ok(&workdir(repo.path()), &["export", "system-ir.json"]);
    let ir: serde_json::Value = serde_json::from_str(&out).unwrap();
    let entities = ir["entities"].as_array().unwrap();
    let main_go_symbols: Vec<&str> = entities
        .iter()
        .filter(|e| e["kind"].as_str() == Some("symbol"))
        .filter(|e| e["attributes"]["file"].as_str() == Some("main.go"))
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert!(
        main_go_symbols.contains(&"main"),
        "main.go symbols missing main: {main_go_symbols:?}"
    );
    // the service package's symbols are exported too (file=internal/service.go)
    let all_symbols: Vec<&str> = entities
        .iter()
        .filter(|e| e["kind"].as_str() == Some("symbol"))
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert!(
        all_symbols.contains(&"NewStore") && all_symbols.contains(&"Store.Save"),
        "service symbols missing: {all_symbols:?}"
    );
}

#[test]
fn go_task_pack_store_mentions_write_symbol() {
    let repo = go_service();
    let out = run_ok(
        &workdir(repo.path()),
        &["context", "task", "store", "--json"],
    );
    let pack: serde_json::Value = serde_json::from_str(&out).unwrap();
    let content = pack["content"].as_str().unwrap();
    assert!(
        content.contains("Store.Save"),
        "store write symbol missing from pack: {content}"
    );
    let ids: Vec<&str> = pack["entity_ids"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        ids.iter().any(|id| id.contains("Store.Save")),
        "store write symbol missing from entity ids: {ids:?}"
    );
}
