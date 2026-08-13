//! Wave 9 python extractor semantic-facts integration: indexing the
//! python-facts-service fixture must surface export/annotation/field/
//! registration/configuration/callback entities and their typed
//! relationships in the System IR export.

mod golden;

use golden::{copy_fixture, run_ok, workdir};

#[test]
fn python_facts_appear_in_system_ir() {
    let repo = copy_fixture("python-facts-service");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);

    let out = run_ok(&dir, &["export", "system-ir.json"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let entities = v["entities"].as_array().expect("entities array");
    let rels = v["relationships"].as_array().expect("relationships array");

    let count_kind = |k: &str| entities.iter().filter(|e| e["kind"] == k).count();
    assert!(count_kind("export") >= 3, "export entities: {}", count_kind("export"));
    assert!(count_kind("annotation") >= 3, "annotation entities: {}", count_kind("annotation"));
    assert!(count_kind("field") >= 2, "field entities: {}", count_kind("field"));
    assert!(count_kind("configuration") >= 1, "configuration entities: {}", count_kind("configuration"));

    let count_pred = |p: &str| rels.iter().filter(|r| r["predicate"] == p).count();
    assert!(count_pred("exports") >= 3, "exports rels: {}", count_pred("exports"));
    assert!(count_pred("annotates") >= 3, "annotates rels: {}", count_pred("annotates"));
    assert!(count_pred("registers") >= 2, "registers rels: {}", count_pred("registers"));
    assert!(count_pred("handles_callback") >= 2, "handles_callback rels: {}", count_pred("handles_callback"));
    assert!(count_pred("configured_by") >= 1, "configured_by rels: {}", count_pred("configured_by"));
}

#[test]
fn python_facts_include_expected_symbols() {
    let repo = copy_fixture("python-facts-service");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);

    let out = run_ok(&dir, &["export", "system-ir.json"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let entities = v["entities"].as_array().unwrap();

    let has = |kind: &str, name: &str| {
        entities
            .iter()
            .any(|e| e["kind"] == kind && e["name"].as_str() == Some(name))
    };
    // public exports: create_app + the __all__-only entry
    assert!(has("export", "create_app"), "export entity for create_app");
    assert!(has("export", "ping"), "export entity for ping");
    assert!(has("export", "Item"), "export entity for Item");
    // annotations
    assert!(has("annotation", "dataclass"), "annotation entity for @dataclass");
    assert!(has("annotation", "router.get"), "annotation entity for @router.get");
    assert!(has("annotation", "celery.task"), "annotation entity for @celery.task");
    // fields are owned namespaced entities
    assert!(has("field", "Cart.items"), "field entity Cart.items");
    // configuration keys
    assert!(has("configuration", "PORT"), "configuration entity PORT");
    assert!(has("configuration", "DATABASE_URL"), "configuration entity DATABASE_URL");

    // registrations resolve to owner symbols: create_app performs them
    let owners: Vec<&str> = rels(v["relationships"].as_array().unwrap())
        .filter(|r| r["predicate"] == "registers")
        .filter_map(|r| {
            entities
                .iter()
                .find(|e| e["id"] == r["subject"])
                .map(|e| e["name"].as_str().unwrap_or(""))
        })
        .collect();
    assert!(owners.contains(&"create_app"), "create_app performs registrations, got: {owners:?}");
    assert!(owners.contains(&"make_web"), "make_web performs registrations, got: {owners:?}");
    assert!(owners.contains(&"send_email"), "send_email registers as a celery task, got: {owners:?}");
}

fn rels(rs: &[serde_json::Value]) -> impl Iterator<Item = &serde_json::Value> {
    rs.iter()
}
