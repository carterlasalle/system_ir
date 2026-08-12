//! Rust extractor end-to-end (Wave 7, plan §46): the rust-service fixture
//! must produce a main flow, a service component in the atlas, exported
//! symbols, and a task pack that surfaces the retry decoration.

mod golden;

use golden::{copy_fixture, run_ok, workdir};

#[test]
fn rust_service_indexes_and_atlas() {
    let repo = copy_fixture("rust-service");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);

    // flows: the fn main entrypoint compiles a sequence flow
    let flows = run_ok(&dir, &["flows"]);
    assert!(flows.contains("main"), "flows must mention main: {flows}");

    // atlas: the src component (service code) + the main entrypoint
    let atlas = run_ok(&dir, &["atlas"]);
    assert!(atlas.contains("main [entrypoint]"), "entrypoint in atlas: {atlas}");
    assert!(atlas.contains("SRC"), "src service component in atlas: {atlas}");

    // export: symbols extracted from src/main.rs
    let ir = run_ok(&dir, &["export", "system-ir.json"]);
    let v: serde_json::Value = serde_json::from_str(&ir).unwrap();
    let syms: Vec<&str> = v["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "symbol" && e["attributes"]["file"] == "src/main.rs")
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert!(syms.contains(&"main"), "main symbol: {syms:?}");
    assert!(syms.contains(&"publish_with_retry"), "retry symbol: {syms:?}");

    // task pack: the retry symbol and its policy are surfaced
    let task = run_ok(&dir, &["context", "task", "describe the retry behavior"]);
    assert!(task.contains("publish_with_retry"), "task pack: {task}");
    assert!(task.contains("retry(attempts = 3"), "retry policy in task pack: {task}");
}
