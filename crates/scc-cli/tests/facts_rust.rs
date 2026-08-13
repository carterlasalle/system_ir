//! Wave 9 semantic facts (rust family): indexing the cli-service fixture
//! must surface public exports, derive annotations, struct fields, axum
//! router registrations, and env configuration reads as first-class
//! entities and relationships in the exported System IR.

use std::collections::BTreeMap;

mod golden;
// trace:v1 id=test.scc.facts.rust verifies=REQ-SCC-IR exercises=impl.scc.facts,impl.scc.extract.rust

#[test]
fn rust_facts_surface_in_system_ir() {
    let repo = golden::copy_fixture("cli-service");
    let dir = golden::workdir(repo.path());
    golden::run_ok(&dir, &["index", "--quiet"]);

    let ir = golden::run_ok(&dir, &["export", "system-ir.json"]);
    let v: serde_json::Value = serde_json::from_str(&ir).expect("system-ir.json parses");

    let entities = v["entities"].as_array().expect("entities array");
    let mut by_kind_name: BTreeMap<(String, String), serde_json::Value> = BTreeMap::new();
    let mut id_to_name: BTreeMap<String, String> = BTreeMap::new();
    for e in entities {
        let kind = e["kind"].as_str().unwrap_or("").to_string();
        let name = e["name"].as_str().unwrap_or("").to_string();
        let id = e["id"].as_str().unwrap_or("").to_string();
        by_kind_name.insert((kind, name.clone()), e["attributes"].clone());
        id_to_name.insert(id, name);
    }
    let has = |kind: &str, name: &str| {
        by_kind_name.contains_key(&(kind.to_string(), name.to_string()))
    };

    // Public exports: pub fn / pub struct. (The writer materializes export
    // entities only for names that are also declared symbols in the file,
    // so the `pub use std::time::Duration` re-export fact — covered at the
    // extractor unit level — does not surface here as an entity.)
    assert!(has("export", "greet"), "greet export");
    assert!(has("export", "ServerState"), "ServerState export");
    assert!(has("export", "build_router"), "build_router export");

    // Derive annotations (Parser/Subcommand from the CLI, Clone on the
    // new struct).
    assert!(has("annotation", "Parser"), "Parser annotation");
    assert!(has("annotation", "Subcommand"), "Subcommand annotation");
    assert!(has("annotation", "Clone"), "Clone annotation");

    // Struct fields: owner/name attributes and the mutability flag.
    assert!(has("field", "ServerState.port"), "port field");
    assert!(has("field", "ServerState.cache"), "cache field");
    assert_eq!(
        by_kind_name[&("field".to_string(), "ServerState.port".to_string())]["mutable"],
        false,
        "plain fields are immutable"
    );
    assert_eq!(
        by_kind_name[&("field".to_string(), "ServerState.cache".to_string())]["mutable"],
        true,
        "RwLock fields are mutable state"
    );

    // Configuration reads: std::env::var("PORT") in service_port.
    assert!(has("configuration", "PORT"), "PORT configuration");
    assert!(
        by_kind_name[&("configuration".to_string(), "PORT".to_string())].is_null(),
        "configuration entity carries no extra attributes"
    );

    // Relationships: map ids back to names (fall back to the raw id for
    // contract ids, which the writer references but does not materialize).
    let rels: Vec<(String, String, String)> = v["relationships"]
        .as_array()
        .expect("relationships array")
        .iter()
        .map(|r| {
            let subj = r["subject"].as_str().unwrap_or("").to_string();
            let obj = r["object"].as_str().unwrap_or("").to_string();
            (
                r["predicate"].as_str().unwrap_or("").to_string(),
                id_to_name.get(&subj).cloned().unwrap_or(subj),
                id_to_name.get(&obj).cloned().unwrap_or(obj),
            )
        })
        .collect();

    // EXPORTS: symbol -> export entity.
    assert!(
        rels.contains(&("exports".into(), "greet".into(), "greet".into())),
        "greet exports rel: {rels:?}"
    );
    // ANNOTATES: annotation -> target symbol.
    assert!(
        rels.contains(&("annotates".into(), "Clone".into(), "ServerState".into())),
        "Clone annotates ServerState: {rels:?}"
    );
    // CONTAINS: struct -> field.
    assert!(
        rels.contains(&("contains".into(), "ServerState".into(), "ServerState.port".into())),
        "contains rel: {rels:?}"
    );
    // REGISTERS: owner -> registered contract (route path / middleware).
    let regs: Vec<&(String, String, String)> = rels
        .iter()
        .filter(|(p, s, _)| p == "registers" && s == "build_router")
        .collect();
    assert!(
        regs.iter().any(|(_, _, o)| o.contains("/health")),
        "registers /health: {regs:?}"
    );
    assert!(
        regs.iter().any(|(_, _, o)| o.contains("/users")),
        "registers /users: {regs:?}"
    );
    assert!(
        regs
            .iter()
            .any(|(_, _, o)| o.to_ascii_lowercase().contains("tracelayer")),
        "registers the TraceLayer middleware: {regs:?}"
    );
    assert_eq!(
        regs.len(),
        3,
        "two routes + one middleware registered: {regs:?}"
    );
    // CONFIGURED_BY: config -> owning symbol.
    assert!(
        rels.contains(&("configured_by".into(), "PORT".into(), "service_port".into())),
        "configured_by rel: {rels:?}"
    );
}
