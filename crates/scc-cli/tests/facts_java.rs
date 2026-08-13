//! Java SemanticFacts end-to-end (Wave 9): indexing the java-facts-service
//! fixture must export export/annotation/field entities plus the
//! EXPORTS/ANNOTATES/REGISTERS/CONTAINS/HANDLES_CALLBACK relationships that
//! the extractor's facts produce.

mod golden;
use golden::*;

const CONFIG: &str = "languages:\n  java: true\n";

/// Index the java-facts-service fixture with java enabled.
fn indexed() -> tempfile::TempDir {
    let repo = copy_fixture("java-facts-service");
    let dir = workdir(repo.path());
    std::fs::create_dir_all(dir.join(".scc")).unwrap();
    std::fs::write(dir.join(".scc/config.yaml"), CONFIG).unwrap();
    run_ok(&dir, &["index", "--quiet"]);
    repo
}

#[test]
fn export_shows_fact_entities_and_relationships() {
    let repo = indexed();
    let ir = run_ok(&workdir(repo.path()), &["export", "system-ir.json"]);
    let v: serde_json::Value = serde_json::from_str(&ir).unwrap();

    let entities = v["entities"].as_array().unwrap();
    let names: Vec<&str> = entities
        .iter()
        .map(|e| e["name"].as_str().unwrap_or(""))
        .collect();

    // export entities: public class, public methods, interface method
    assert!(
        names.contains(&"GreetingController.greet"),
        "exports missing: {names:?}"
    );
    assert!(names.contains(&"GreetingProvider.provide"), "{names:?}");
    let class_export = entities
        .iter()
        .find(|e| e["kind"] == "export" && e["name"] == "GreetingController")
        .unwrap_or_else(|| panic!("class export missing: {names:?}"));
    assert_eq!(class_export["attributes"]["kind"], "class");

    // annotation entities (framework-verified: imports present in fixture)
    assert!(names.contains(&"RestController"), "{names:?}");
    assert!(names.contains(&"GetMapping"), "{names:?}");
    assert!(names.contains(&"Test"), "{names:?}");
    assert!(names.contains(&"BeforeClass"), "{names:?}");

    // field entities carry the owner + mutability
    let count_field = entities
        .iter()
        .find(|e| e["kind"] == "field" && e["name"] == "GreetingService.count")
        .unwrap_or_else(|| panic!("count field missing: {names:?}"));
    assert_eq!(count_field["attributes"]["mutable"], true);
    let prefix_field = entities
        .iter()
        .find(|e| e["kind"] == "field" && e["name"] == "GreetingService.prefix")
        .unwrap();
    assert_eq!(prefix_field["attributes"]["mutable"], false);

    // relationships: typed fact edges
    let rels = v["relationships"].as_array().unwrap();
    let preds: Vec<&str> = rels
        .iter()
        .map(|r| r["predicate"].as_str().unwrap_or(""))
        .collect();
    assert!(preds.contains(&"exports"), "{preds:?}");
    assert!(preds.contains(&"annotates"), "{preds:?}");
    assert!(preds.contains(&"registers"), "{preds:?}");
    assert!(preds.contains(&"contains"), "{preds:?}");
    assert!(preds.contains(&"handles_callback"), "{preds:?}");

    // spring routes register their handler methods (kind lives on the
    // originating fact, not the relationship row)
    let route = rels
        .iter()
        .find(|r| {
            r["predicate"] == "registers" && r["object"].as_str().unwrap_or("").contains("greet")
        })
        .unwrap_or_else(|| panic!("route registration missing: {preds:?}"));
    assert!(route["object"].as_str().unwrap_or("").contains("greet"), "{route:?}");

    // junit lifecycle callbacks point at the annotated methods
    let cb = rels
        .iter()
        .find(|r| r["predicate"] == "handles_callback")
        .unwrap_or_else(|| panic!("callback missing: {preds:?}"));
    assert!(
        cb["object"].as_str().unwrap_or("").contains("setupAll"),
        "{cb:?}"
    );
}
