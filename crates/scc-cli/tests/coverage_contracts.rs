//! Wave 9 first-class contracts + invocation surfaces + explicit
//! uncertainty/coverage map (integration): indexing python-facts-service
//! must render typed CONTRACTS lines (`http:`/`cli:`/`config:`), surface
//! public_api + framework_callback entrypoints, and a MODEL COVERAGE
//! section that states what the model knows AND what it does not.

mod golden;

use golden::{copy_fixture, run_ok, workdir};
// trace:v1 id=test.scc.coverage verifies=REQ-SCC-CTX exercises=impl.scc.atlas

#[test]
fn atlas_renders_typed_contracts_surfaces_and_coverage() {
    let repo = copy_fixture("python-facts-service");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);
    let atlas = run_ok(&dir, &["atlas"]);

    // ---- CONTRACTS: first-class `kind: operation` lines ----
    assert!(atlas.contains("# CONTRACTS"), "{atlas}");
    // http from ROUTE entities
    assert!(atlas.contains("http: GET /ping"), "http contract: {atlas}");
    assert!(atlas.contains("http: GET /items/{item_id}"), "http contract: {atlas}");
    assert!(atlas.contains("http: GET /admin"), "http contract: {atlas}");
    // cli from cli_flags attrs (argparse in the fixture)
    assert!(atlas.contains("cli: --port"), "cli contract: {atlas}");
    assert!(atlas.contains("cli: --verbose"), "cli contract: {atlas}");
    assert!(atlas.contains("cli: --workers"), "cli contract: {atlas}");
    // config from CONFIGURATION entities
    assert!(atlas.contains("config: DEBUG"), "config contract: {atlas}");
    assert!(atlas.contains("config: PORT"), "config contract: {atlas}");
    assert!(atlas.contains("config: DATABASE_URL"), "config contract: {atlas}");

    // ---- ENTRYPOINTS: invocation-surface seeds are additive ----
    assert!(atlas.contains("create_app [public_api]"), "export surface: {atlas}");
    assert!(atlas.contains("ping [public_api]"), "export surface: {atlas}");
    assert!(
        atlas.contains("[framework_callback]"),
        "callback surface: {atlas}"
    );
    // routes still render as routes
    assert!(atlas.contains("GET /ping [route]"), "route entrypoint: {atlas}");

    // ---- MODEL COVERAGE: the explicit uncertainty map ----
    assert!(atlas.contains("# MODEL COVERAGE"), "{atlas}");
    for key in [
        "parsed_source_files:",
        "exported_api:",
        "call_targets_resolved:",
        "invocation_surfaces:",
        "dynamic_receivers_unresolved:",
        "framework_registrations_unknown:",
        "stale_evidence:",
        "unparsed_files:",
        "model_epoch_generations:",
    ] {
        assert!(atlas.contains(key), "coverage key {key} missing: {atlas}");
    }
    // honest, deterministic numbers on the fixture: 3 python files parsed,
    // no stale files, all calls target external modules (0% resolved to
    // local symbols), 7 external/dynamic receivers
    assert!(atlas.contains("parsed_source_files: 100% (3/3)"), "{atlas}");
    assert!(atlas.contains("call_targets_resolved: 0% (0/7"), "{atlas}");
    assert!(atlas.contains("dynamic_receivers_unresolved: 7"), "{atlas}");
    assert!(atlas.contains("stale_evidence: 0 (model FRESH)"), "{atlas}");

    // determinism: identical model state renders the same atlas
    let atlas2 = run_ok(&dir, &["atlas"]);
    assert_eq!(atlas, atlas2, "atlas must be deterministic");

    // ---- System IR regression guard: routes still surface as route
    // entities in the export, and registration targets as contract
    // entities (include_router/register_blueprint/exception handlers) ----
    let out = run_ok(&dir, &["export", "system-ir.json"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let entities = v["entities"].as_array().unwrap();
    let has_kind_name = |kind: &str, name: &str| {
        entities
            .iter()
            .any(|e| e["kind"] == kind && e["name"].as_str() == Some(name))
    };
    assert!(
        has_kind_name("route", "GET /ping"),
        "route entity for GET /ping missing"
    );
    assert!(
        has_kind_name("contract", "router") && has_kind_name("contract", "bp"),
        "registration-target contract entities missing"
    );
}

#[test]
fn model_coverage_is_droppable_but_contracts_never() {
    // The MODEL COVERAGE section is priority 7 (droppable before critical),
    // while CONTRACTS (priority 9) is never dropped — a tight budget hides
    // the coverage map but never the contracts.
    let repo = copy_fixture("python-facts-service");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);
    let out = run_ok(&dir, &["atlas", "--budget", "250", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let dropped: Vec<&str> = v["dropped_sections"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let content = v["content"].as_str().unwrap_or("");
    if content.contains("# MODEL COVERAGE") {
        assert!(
            dropped.contains(&"MODEL COVERAGE") || !dropped.contains(&"CONTRACTS"),
            "CONTRACTS must never drop; MODEL COVERAGE may: dropped={dropped:?}"
        );
    } else {
        assert!(
            dropped.contains(&"MODEL COVERAGE"),
            "MODEL COVERAGE must be the dropped section: {dropped:?}"
        );
        assert!(
            !dropped.contains(&"CONTRACTS"),
            "CONTRACTS must never drop: {dropped:?}"
        );
        assert!(content.contains("# CONTRACTS"), "contracts survive budget: {content}");
    }
}
