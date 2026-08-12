//! Java extractor end-to-end tests (Wave 7 §46): a small Maven service must
//! produce entrypoints, flows, an atlas naming the service component and
//! main entrypoint, an export with symbols in Main.java, and a "store" task
//! pack that surfaces the JDBC store-write method.

mod golden;
use golden::*;

const CONFIG: &str = "languages:\n  java: true\n";

const INTENT: &str = r#"
components:
  service:
    responsibility:
      - persist and process orders
    paths:
      - src/main/java
flows:
  order-processing:
    entrypoint: Service.process
    kind: sequence
    trigger: order submitted
"#;

/// Index the java-service fixture with java enabled and the service
/// component declared.
fn java_service() -> tempfile::TempDir {
    let repo = copy_fixture("java-service");
    let dir = workdir(repo.path());
    std::fs::create_dir_all(dir.join(".scc")).unwrap();
    std::fs::write(dir.join(".scc/config.yaml"), CONFIG).unwrap();
    std::fs::write(dir.join(".scc/intent.yaml"), INTENT).unwrap();
    run_ok(&dir, &["index", "--quiet"]);
    repo
}

#[test]
fn flows_mention_main_entrypoint() {
    let repo = java_service();
    let flows = run_ok(&workdir(repo.path()), &["flows"]);
    assert!(flows.contains("main"), "flows: {flows}");
    assert!(flows.contains("Main.main"), "flows: {flows}");
    // the declared intent flow walks the same-class call fanout
    assert!(flows.contains("order-processing"), "flows: {flows}");
}

#[test]
fn atlas_contains_service_component_and_main_entrypoint() {
    let repo = java_service();
    let atlas = run_ok(&workdir(repo.path()), &["atlas"]);
    assert!(atlas.contains("ENTRYPOINTS"), "atlas: {atlas}");
    assert!(atlas.contains("Main.main"), "main entrypoint: {atlas}");
    // the declared `service` component owns the java sources
    assert!(atlas.contains("SERVICE"), "service component: {atlas}");
    assert!(atlas.contains("storeOrder"), "atlas: {atlas}");
}

#[test]
fn export_has_symbols_in_main_java() {
    let repo = java_service();
    let out = run_ok(&workdir(repo.path()), &["export", "system-ir.json"]);
    let ir: serde_json::Value = serde_json::from_str(&out).unwrap();
    let entities = ir["entities"].as_array().expect("entities array");

    let main_syms: Vec<&serde_json::Value> = entities
        .iter()
        .filter(|e| e["kind"] == "symbol")
        .filter(|e| {
            e["attributes"]["file"]
                .as_str()
                .map(|f| f.contains("Main.java"))
                .unwrap_or(false)
        })
        .collect();
    assert!(!main_syms.is_empty(), "expected symbols in Main.java: {out}");
    let names: Vec<&str> = main_syms
        .iter()
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert!(names.contains(&"Main.main"), "names: {names:?}");

    // retry policy from @Retryable lands on the annotated method
    let process = entities
        .iter()
        .find(|e| e["kind"] == "symbol" && e["name"] == "Service.process")
        .expect("Service.process symbol");
    let policy = process["attributes"]["retry_policy"]
        .as_str()
        .unwrap_or("");
    assert!(policy.contains("Retryable"), "retry policy: {policy}");
}

#[test]
fn task_pack_for_store_mentions_store_write_method() {
    let repo = java_service();
    let task = run_ok(&workdir(repo.path()), &["context", "task", "store"]);
    assert!(
        task.contains("storeOrder"),
        "store write method must surface in the task pack: {task}"
    );
}
