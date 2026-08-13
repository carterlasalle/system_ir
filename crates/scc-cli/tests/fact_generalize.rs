//! Wave 9 fact-generalization (holdout families): indexing the
//! builder-factory-service fixture must surface builder/factory contract
//! registrations (requests Session/PreparedRequest fluent + factories,
//! zerolog New/Context, cobra AddCommand, guava ImmutableList.of/builder,
//! zod z.object/string, vue createApp, axios createInstance/axios.create)
//! and symbol→state authority (module-level globals + class statics as
//! mutable FIELD entities owned by their module/class symbol).

mod golden;

use golden::{copy_fixture, run_ok, workdir};
// trace:v1 id=test.scc.fact-generalize verifies=REQ-SCC-IR exercises=impl.scc.facts,impl.scc.extract.python

fn builder_factory_service() -> tempfile::TempDir {
    let repo = copy_fixture("builder-factory-service");
    run_ok(&workdir(repo.path()), &["index", "--quiet"]);
    repo
}

fn exported_ir(repo: &tempfile::TempDir) -> serde_json::Value {
    let out = run_ok(&workdir(repo.path()), &["export", "system-ir.json"]);
    serde_json::from_str(&out).unwrap()
}

#[test]
fn builder_and_factory_registrations_surface() {
    let repo = builder_factory_service();
    let ir = exported_ir(&repo);
    let entities = ir["entities"].as_array().unwrap();

    // Registration targets that are not symbols become CONTRACT entities
    // carrying the registration kind. zod-style object-literal schema
    // factories (`z.object`, `z.string`, ...) and the axios namespace
    // (`axios.create`) produce factory contracts.
    let contracts: Vec<&serde_json::Value> = entities
        .iter()
        .filter(|e| e["kind"] == "contract")
        .collect();
    assert!(
        contracts
            .iter()
            .any(|e| e["name"] == "object" && e["attributes"]["kind"] == "factory"),
        "z.object factory contract missing: {contracts:?}"
    );
    assert!(
        contracts
            .iter()
            .any(|e| e["name"] == "string" && e["attributes"]["kind"] == "factory"),
        "z.string factory contract missing"
    );
    assert!(
        contracts
            .iter()
            .any(|e| e["name"] == "create" && e["attributes"]["kind"] == "factory"),
        "axios.create factory contract missing"
    );
    assert!(
        contracts.iter().all(|e| e["attributes"]["kind"].is_string()),
        "contracts must carry a registration kind"
    );

    // Builder/factory registrations whose target is a declared symbol
    // register the symbol: the REGISTERS edge subject is the owning
    // factory/builder (Session, Builder, Command, ImmutableList,
    // create_session, New, createApp, createClient, z...).
    let registers: Vec<(&str, &str)> = ir["relationships"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["predicate"] == "registers")
        .filter_map(|r| Some((r["subject"].as_str()?, r["object"].as_str()?)))
        .collect();
    let register_names: Vec<String> = registers
        .iter()
        .map(|(s, o)| format!("{s} -> {o}"))
        .collect();
    for owner in [
        "Session",      // python fluent builder
        "create_session", // python module factory
        "Builder",      // java fluent builder (guava)
        "ImmutableList", // java static factories (guava of/builder)
        "Command",      // go compositional builder (cobra AddCommand)
        "New",          // go package factory (zerolog New)
        "createApp",    // vue module factory
        "createClient", // axios-style module factory
        "z",            // zod schema-builder namespace
    ] {
        assert!(
            register_names.iter().any(|r| {
                r.split(" -> ")
                    .next()
                    .map(|s| s.contains(owner))
                    .unwrap_or(false)
            }),
            "{owner} registration missing: {register_names:?}"
        );
    }
    // every registration target is a known entity — invariants hold
    let verify = run_ok(&workdir(repo.path()), &["check-invariants"]);
    assert!(
        verify.contains("ok") || verify.is_empty(),
        "check-invariants must pass with builder/factory contracts: {verify}"
    );
}

#[test]
fn module_globals_and_statics_are_mutable_state() {
    let repo = builder_factory_service();
    let ir = exported_ir(&repo);

    let fields: Vec<&serde_json::Value> = ir["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "field")
        .collect();
    let mutable_field_names: Vec<String> = fields
        .iter()
        .filter(|e| e["attributes"]["mutable"].as_bool() == Some(true))
        .filter_map(|e| e["name"].as_str().map(str::to_string))
        .collect();
    // python module global owned by the module symbol (file stem)
    assert!(
        mutable_field_names
            .iter()
            .any(|n| n == "requests_style.DEFAULT_TIMEOUT"),
        "python module global missing: {mutable_field_names:?}"
    );
    // go package var owned by the module symbol
    assert!(
        mutable_field_names
            .iter()
            .any(|n| n == "zerolog_style.DefaultLogger"),
        "go package var missing: {mutable_field_names:?}"
    );
    // java static field owned by its class (final is immutable → excluded)
    assert!(
        mutable_field_names
            .iter()
            .any(|n| n == "ImmutableList.DEFAULT_CAPACITY"),
        "java static field missing: {mutable_field_names:?}"
    );
    assert!(
        !mutable_field_names
            .iter()
            .any(|n| n == "ImmutableList.PACKAGE"),
        "final static must not be mutable: {mutable_field_names:?}"
    );
    // ts module `let` global
    assert!(
        mutable_field_names
            .iter()
            .any(|n| n == "axios_style.defaultTimeout"),
        "ts module let missing: {mutable_field_names:?}"
    );

    // the module symbols exist and own the globals (module kind)
    let module_syms: Vec<&str> = ir["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| {
            e["kind"] == "symbol" && e["attributes"]["kind"].as_str() == Some("module")
        })
        .filter_map(|e| e["name"].as_str())
        .collect();
    for want in ["requests_style", "zerolog_style", "axios_style"] {
        assert!(
            module_syms.contains(&want),
            "module symbol {want} missing: {module_syms:?}"
        );
    }
}
