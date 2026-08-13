//! Wave 9 TypeScript SemanticFacts end-to-end (fixtures/ts-facts-service):
//! indexing the fixture must produce public-export / annotation / field /
//! registration / configuration / callback entities and their relationships
//! in the exported System IR, deterministically.

mod golden;
use golden::*;

fn indexed_ir() -> (tempfile::TempDir, serde_json::Value) {
    let repo = copy_fixture("ts-facts-service");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);
    let ir: serde_json::Value =
        serde_json::from_str(&run_ok(&dir, &["export", "system-ir.json"])).unwrap();
    (repo, ir)
}

fn entities_by_kind<'a>(ir: &'a serde_json::Value, kind: &str) -> Vec<&'a serde_json::Value> {
    ir["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == kind)
        .collect()
}

fn has_entity_name(ir: &serde_json::Value, kind: &str, name: &str) -> bool {
    entities_by_kind(ir, kind).iter().any(|e| e["name"] == name)
}

fn has_relationship(ir: &serde_json::Value, predicate: &str) -> bool {
    ir["relationships"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["predicate"] == predicate)
}

#[test]
fn ts_facts_export_entities_and_relationships() {
    let (_repo, ir) = indexed_ir();

    // Public exports: nest classes, express handler, react component + hooks,
    // plain interface/const/type.
    for name in [
        "UsersController",
        "UsersService",
        "AppModule",
        "health",
        "App",
        "handleTheme",
        "handleClick",
        "Page",
        "VERSION",
        "PageList",
    ] {
        assert!(
            has_entity_name(&ir, "export", name),
            "missing export entity {name}: {ir}"
        );
    }

    // Annotations (nest decorators, import-verified).
    for (name, target) in [
        ("Controller", "UsersController"),
        ("Get", "UsersController.list"),
        ("Get", "UsersController.get"),
        ("Module", "AppModule"),
        ("Injectable", "UsersService"),
    ] {
        let anns = entities_by_kind(&ir, "annotation");
        assert!(
            anns.iter().any(|e| e["name"] == name),
            "missing annotation entity {name}: {ir}"
        );
        assert!(
            ir["relationships"].as_array().unwrap().iter().any(|r| {
                r["predicate"] == "annotates"
                    && r["object"].as_str().unwrap().contains(target)
            }),
            "missing ANNOTATES ->{target}: {ir}"
        );
    }

    // Fields with mutability.
    assert!(has_entity_name(&ir, "field", "UsersService.table"), "{ir}");
    assert!(has_entity_name(&ir, "field", "UsersService.retries"), "{ir}");
    let fields = entities_by_kind(&ir, "field");
    let table = fields.iter().find(|e| e["name"] == "UsersService.table").unwrap();
    assert_eq!(table["attributes"]["mutable"], serde_json::json!(false), "{ir}");
    let retries = fields.iter().find(|e| e["name"] == "UsersService.retries").unwrap();
    assert_eq!(retries["attributes"]["mutable"], serde_json::json!(true), "{ir}");

    // Registrations: nest module arrays + express route (contract).
    assert!(
        ir["relationships"].as_array().unwrap().iter().any(|r| {
            r["predicate"] == "registers"
                && r["subject"].as_str().unwrap().contains("AppModule")
                && r["object"].as_str().unwrap().contains("UsersController")
        }),
        "missing REGISTERS AppModule->UsersController: {ir}"
    );
    // Cross-file target (UsersService lives in users.service.ts) becomes a
    // contract entity keyed by the slugged target.
    assert!(
        ir["relationships"].as_array().unwrap().iter().any(|r| {
            r["predicate"] == "registers"
                && r["subject"].as_str().unwrap().contains("AppModule")
                && r["object"].as_str().unwrap().contains("usersservice")
        }),
        "missing REGISTERS AppModule->UsersService: {ir}"
    );
    assert!(
        ir["relationships"].as_array().unwrap().iter().any(|r| {
            r["predicate"] == "registers"
                && r["subject"].as_str().unwrap().contains("app")
                && r["object"].as_str().unwrap().contains("get-/health")
        }),
        "missing express route registration GET /health: {ir}"
    );
    assert!(
        has_entity_name(&ir, "route", "GET /health"),
        "route contract entity: {ir}"
    );
    // Registration targets that are not same-file symbols become contract
    // entities (writer inserts them alongside the REGISTERS relationship).
    assert!(
        has_entity_name(&ir, "contract", "GET /health"),
        "registered route contract: {ir}"
    );
    assert!(
        has_entity_name(&ir, "contract", "next"),
        "next config contract: {ir}"
    );

    // Configuration: process.env reads.
    let cfgs = entities_by_kind(&ir, "configuration");
    assert!(
        cfgs.iter().any(|e| e["name"] == "PORT"),
        "missing configuration PORT: {ir}"
    );
    assert!(
        cfgs.iter().any(|e| e["name"] == "API_URL"),
        "missing configuration API_URL: {ir}"
    );

    // Callbacks: react useEffect + DOM addEventListener (framework-verified).
    assert!(
        ir["relationships"].as_array().unwrap().iter().any(|r| {
            r["predicate"] == "handles_callback"
                && r["subject"].as_str().unwrap().contains("App")
                && r["object"].as_str().unwrap().contains("handleTheme")
        }),
        "missing HANDLES_CALLBACK App->handleTheme: {ir}"
    );
    assert!(
        ir["relationships"].as_array().unwrap().iter().any(|r| {
            r["predicate"] == "handles_callback"
                && r["subject"].as_str().unwrap().contains("App")
                && r["object"].as_str().unwrap().contains("handleClick")
        }),
        "missing HANDLES_CALLBACK App->handleClick: {ir}"
    );

    // next.config registration.
    assert!(
        ir["relationships"].as_array().unwrap().iter().any(|r| {
            r["predicate"] == "registers"
                && r["subject"].as_str().unwrap().contains("nextConfig")
        }),
        "missing next.config registration: {ir}"
    );

    // Relationship families present.
    for pred in ["exports", "annotates", "registers", "configured_by", "handles_callback"] {
        assert!(has_relationship(&ir, pred), "missing {pred} relationships: {ir}");
    }

    // Express route is also a first-class route (existing extraction).
    let routes = run_ok(&workdir(_repo.path()), &["flows"]);
    assert!(routes.contains("GET /health"), "{routes}");

    // Deterministic export: identical bytes on re-export.
    let again = run_ok(&workdir(_repo.path()), &["export", "system-ir.json"]);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&again).unwrap(),
        ir,
        "re-export must be byte-identical"
    );

    // verify stays clean with the new fact layer.
    let verify = run_ok(&workdir(_repo.path()), &["verify"]);
    assert!(verify.contains("VERIFIED"), "{verify}");
}
