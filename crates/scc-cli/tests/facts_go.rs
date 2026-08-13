//! Go SemanticFacts end-to-end (Wave 9): indexing the go-facts-service
//! fixture must surface export/field/registration/callback facts and their
//! EXPORTS/CONTAINS/REGISTERS/HANDLES_CALLBACK relationships in the System
//! IR export, with gin GET/POST routes and gorilla mux routes registered as
//! contract endpoints (closing the go-routes gap).

mod golden;
use golden::*;

fn go_facts_service() -> tempfile::TempDir {
    let repo = copy_fixture("go-facts-service");
    run_ok(&workdir(repo.path()), &["index", "--quiet"]);
    repo
}

#[test]
fn go_facts_export_has_fact_entities_and_relationships() {
    let repo = go_facts_service();
    let out = run_ok(&workdir(repo.path()), &["export", "system-ir.json"]);
    let ir: serde_json::Value = serde_json::from_str(&out).unwrap();
    let entities = ir["entities"].as_array().unwrap();
    let kinds: Vec<&str> = entities
        .iter()
        .filter_map(|e| e["kind"].as_str())
        .collect();
    assert!(
        kinds.contains(&"export"),
        "export entities missing: {kinds:?}"
    );
    assert!(kinds.contains(&"field"), "field entities missing: {kinds:?}");

    let rels = ir["relationships"].as_array().unwrap();
    let preds: Vec<&str> = rels
        .iter()
        .filter_map(|r| r["predicate"].as_str())
        .collect();
    assert!(preds.contains(&"exports"), "EXPORTS missing: {preds:?}");
    assert!(
        preds.contains(&"contains"),
        "CONTAINS (field owner) missing: {preds:?}"
    );
    assert!(
        preds.contains(&"registers"),
        "REGISTERS (routes) missing: {preds:?}"
    );
    assert!(
        preds.contains(&"handles_callback"),
        "HANDLES_CALLBACK missing: {preds:?}"
    );
}

#[test]
fn go_facts_gin_routes_are_contracts() {
    let repo = go_facts_service();
    let out = run_ok(&workdir(repo.path()), &["export", "system-ir.json"]);
    let ir: serde_json::Value = serde_json::from_str(&out).unwrap();
    // registered route targets are contract entities in the export
    let names: Vec<&str> = ir["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"].as_str() == Some("contract"))
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert!(
        names.contains(&"GET /ping"),
        "GET /ping contract entity missing: {names:?}"
    );
    assert!(
        names.contains(&"GET /users/:id"),
        "GET /users/:id contract entity missing: {names:?}"
    );
    assert!(
        names.contains(&"POST /users"),
        "POST /users contract entity missing: {names:?}"
    );
    // /api group prefix resolved into the contract target
    assert!(
        names.contains(&"GET /api/health"),
        "grouped /api/health contract entity missing: {names:?}"
    );
    // gorilla mux routes are contracts too
    assert!(
        names.contains(&"/items") && names.contains(&"/items/{id}"),
        "gorilla route contract entities missing: {names:?}"
    );
    // http.HandleFunc is a callback, never a route contract
    assert!(
        !names.iter().any(|n| n.contains("legacy")),
        "http.HandleFunc must not become a route: {names:?}"
    );

    let rels = ir["relationships"].as_array().unwrap();
    let objects: Vec<&str> = rels
        .iter()
        .filter(|r| r["predicate"].as_str() == Some("registers"))
        .filter_map(|r| r["object"].as_str())
        .collect();
    assert!(
        objects.iter().any(|o| o.contains("contract/get-/ping")),
        "main must register the GET /ping contract: {objects:?}"
    );

    // no dangling contract objects: invariants must hold
    let verify = run_ok(&workdir(repo.path()), &["check-invariants"]);
    assert!(
        verify.contains("ok") || verify.is_empty(),
        "check-invariants must pass with route contracts: {verify}"
    );
}

#[test]
fn go_facts_exports_fields_and_callbacks() {
    let repo = go_facts_service();
    let out = run_ok(&workdir(repo.path()), &["export", "system-ir.json"]);
    let ir: serde_json::Value = serde_json::from_str(&out).unwrap();
    let entities = ir["entities"].as_array().unwrap();

    let exports: Vec<&str> = entities
        .iter()
        .filter(|e| e["kind"].as_str() == Some("export"))
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert!(
        exports.contains(&"User") && exports.contains(&"PingHandler"),
        "export entities missing: {exports:?}"
    );

    let fields: Vec<&str> = entities
        .iter()
        .filter(|e| e["kind"].as_str() == Some("field"))
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert!(
        fields.contains(&"User.ID") && fields.contains(&"User.Email"),
        "field entities missing: {fields:?}"
    );

    let rels = ir["relationships"].as_array().unwrap();
    // callbacks: gin.Use middleware chain + http.HandleFunc
    let callbacks: Vec<&str> = rels
        .iter()
        .filter(|r| r["predicate"].as_str() == Some("handles_callback"))
        .filter_map(|r| r["object"].as_str())
        .collect();
    assert!(
        callbacks.iter().any(|o| o.contains("LoggerMiddleware")),
        "gin.Use middleware callback missing: {callbacks:?}"
    );
    assert!(
        callbacks.iter().any(|o| o.contains("LegacyHandler")),
        "http.HandleFunc callback missing: {callbacks:?}"
    );

    // routes register from the owning symbol (main) to the contract endpoint
    let registers: Vec<(&str, &str)> = rels
        .iter()
        .filter(|r| r["predicate"].as_str() == Some("registers"))
        .filter_map(|r| Some((r["subject"].as_str()?, r["object"].as_str()?)))
        .collect();
    assert!(
        registers.iter().any(|(s, o)| s.contains("main.go/main") && o.contains("get-/ping")),
        "main must register GET /ping: {registers:?}"
    );
}
