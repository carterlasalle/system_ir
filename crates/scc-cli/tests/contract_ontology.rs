//! Contract ontology integration tests: indexing the
//! contract-ontology-service fixture must derive the right contract
//! SUBCLASSES per language from general semantic evidence (public fn
//! signatures → public-api, builder/factory → config/public-api, event
//! producer+consumer pairs → event, serializer/deserializer pairs →
//! serialization, interface + implementations → extension, and the classic
//! route/flag/topic/config facts → http/cli/event/config) and render the
//! atlas CONTRACTS section as per-subclass groups.

mod golden;

use golden::{copy_fixture, run_ok, workdir};
// trace:v1 id=test.scc.contract-ontology verifies=REQ-SCC-CTX exercises=impl.scc.atlas,impl.scc.extract.python,impl.scc.extract.typescript,impl.scc.extract.rust,impl.scc.extract.go,impl.scc.extract.java

fn fixture() -> tempfile::TempDir {
    let repo = copy_fixture("contract-ontology-service");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);
    repo
}

#[test]
// trace:v1 id=test.scc.contracts verifies=REQ-SCC-CTX exercises=impl.scc.atlas
fn atlas_renders_per_subclass_contract_groups() {
    let repo = fixture();
    let dir = workdir(repo.path());
    let atlas = run_ok(&dir, &["atlas"]);

    assert!(atlas.contains("# CONTRACTS"), "{atlas}");

    // http from ROUTE entities (flask @app.get, framework-gated)
    assert!(atlas.contains("http: GET /users"), "http contract: {atlas}");

    // cli from cli_flags attrs (argparse in the fixture)
    assert!(atlas.contains("cli: --port"), "cli contract: {atlas}");
    assert!(atlas.contains("cli: --verbose"), "cli contract: {atlas}");

    // event from the kafka producer+consumer pair around a topic
    assert!(atlas.contains("event: user.created"), "event contract: {atlas}");

    // config from CONFIGURATION entities (os.getenv / settings reads)
    assert!(atlas.contains("config: PORT"), "config contract: {atlas}");
    assert!(atlas.contains("config: DEBUG"), "config contract: {atlas}");

    // public-api from exported function signatures
    assert!(atlas.contains("public-api: NewUser"), "public-api contract (go): {atlas}");
    assert!(atlas.contains("public-api: toJson"), "public-api contract (ts): {atlas}");
    assert!(atlas.contains("public-api: list_users"), "public-api contract (python): {atlas}");

    // extension from interface + implementations (cross-file surfaces)
    assert!(atlas.contains("extension: Plugin"), "extension contract (ts): {atlas}");
    assert!(atlas.contains("extension: Greeter"), "extension contract (java): {atlas}");

    // serialization from serializer/deserializer pairs around a type,
    // per language
    assert!(
        atlas.contains("serialization: to_dict/from_dict"),
        "serialization contract (python): {atlas}"
    );
    assert!(
        atlas.contains("serialization: toJson/fromJson"),
        "serialization contract (ts/java): {atlas}"
    );
    assert!(
        atlas.contains("serialization: Serialize/Deserialize"),
        "serialization contract (rust): {atlas}"
    );
    assert!(
        atlas.contains("serialization: MarshalJSON/UnmarshalJSON"),
        "serialization contract (go): {atlas}"
    );

    // determinism: identical model state renders the same atlas
    let atlas2 = run_ok(&dir, &["atlas"]);
    assert_eq!(atlas, atlas2, "atlas must be deterministic");
}

#[test]
fn contracts_render_as_grouped_subclass_lines_only() {
    let repo = fixture();
    let dir = workdir(repo.path());
    let atlas = run_ok(&dir, &["atlas"]);
    // the CONTRACTS body: everything between the header and the next
    // section header
    let body = atlas
        .split("# CONTRACTS")
        .nth(1)
        .expect("CONTRACTS header")
        .split("\n# ")
        .next()
        .unwrap_or("");
    let lines: Vec<&str> = body
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    assert!(!lines.is_empty(), "CONTRACTS body must render lines: {atlas}");

    const SUBCLASS_PREFIXES: [&str; 12] = [
        "call", "public-api", "http", "rpc", "cli", "event", "message", "schema", "config",
        "plugin", "extension", "serialization",
    ];
    // every line is a `{subclass}: {operation}` group line
    for l in &lines {
        let prefix = l.split(':').next().unwrap_or("");
        assert!(
            SUBCLASS_PREFIXES.contains(&prefix),
            "line has no known subclass prefix: {l:?} (full: {atlas})"
        );
    }
    // annotations and framework-specific registrations are NOT first-class
    // contracts (they stay framework semantics)
    assert!(
        !lines.iter().any(|l| l.starts_with("annotation:")),
        "annotations must not be contracts: {lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.starts_with("call: Debug") || l.starts_with("call: app.get")),
        "framework noise must not be contracts: {lines:?}"
    );
    // per-subclass groups: same-prefix lines are contiguous and prefixes
    // appear in sorted order (the CONTRACTS section renders as groups)
    let prefixes: Vec<&str> = lines.iter().map(|l| l.split(':').next().unwrap()).collect();
    let mut sorted = prefixes.clone();
    sorted.sort_unstable();
    assert_eq!(prefixes, sorted, "per-subclass groups must be contiguous: {lines:?}");

    // determinism: identical model state renders the same atlas
    let atlas2 = run_ok(&dir, &["atlas"]);
    assert_eq!(atlas, atlas2, "atlas must be deterministic");
}
