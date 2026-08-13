//! CLI command components (Wave 9): directories holding CLI registrations
//! (clap/cobra/argparse/click) must compile into their own component with
//! `boundary_kind=cli` — never a generic code-region dir component — and
//! the CLI component surfaces in `scc components` and the exported IR.

use std::collections::BTreeMap;

mod golden;
// trace:v1 id=test.scc.cli-components verifies=REQ-SCC-IR exercises=impl.scc.components

#[test]
fn cli_service_fixture_yields_cli_boundary_component() {
    // cli-service: cli.py (argparse), cli.rs (clap), main.go (cobra) at the
    // repo root — the whole repo is the CLI package, so the root component
    // must carry boundary_kind=cli (evidence-backed), not the bare
    // code-region/root fallback.
    let repo = golden::copy_fixture("cli-service");
    let dir = golden::workdir(repo.path());
    golden::run_ok(&dir, &["index", "--quiet"]);

    let ir = golden::run_ok(&dir, &["export", "system-ir.json"]);
    let v: serde_json::Value =
        serde_json::from_str(&ir).expect("system-ir.json parses");
    let entities = v["entities"].as_array().expect("entities array");
    let mut by_name: BTreeMap<&str, &serde_json::Value> = BTreeMap::new();
    for e in entities {
        if e["kind"].as_str() == Some("component") {
            by_name.insert(e["name"].as_str().unwrap_or(""), e);
        }
    }
    assert!(
        by_name.contains_key("root"),
        "cli fixture must still produce the root component: {by_name:?}"
    );
    let root = by_name["root"];
    assert_eq!(
        root["attributes"]["boundary_kind"],
        serde_json::json!("cli"),
        "root component of a CLI-only repo must be a cli boundary"
    );
    assert_eq!(
        root["attributes"]["layer"],
        serde_json::json!("component"),
        "cli components are authoritative components, not code regions"
    );
    // the CLI component is evidence-rich: it owns the registration files
    let paths = root["attributes"]["implementation"]["paths"]
        .as_array()
        .expect("implementation.paths");
    assert_eq!(paths, &vec![serde_json::json!("root")], "cli component paths: {paths:?}");

    // `scc components` lists the CLI command component
    let listed = golden::run_ok(&dir, &["components"]);
    assert!(listed.contains("root"), "components must list root: {listed}");
}

#[test]
fn non_cli_repo_keeps_code_region_boundaries() {
    // http-service-python has no cli-subcommand/cli_flags evidence: its dir
    // components must stay generic code-region boundaries (no spurious cli
    // components invented).
    let repo = golden::copy_fixture("http-service-python");
    let dir = golden::workdir(repo.path());
    golden::run_ok(&dir, &["index", "--quiet"]);

    let ir = golden::run_ok(&dir, &["export", "system-ir.json"]);
    let v: serde_json::Value =
        serde_json::from_str(&ir).expect("system-ir.json parses");
    let entities = v["entities"].as_array().expect("entities array");
    let mut by_name: BTreeMap<&str, &serde_json::Value> = BTreeMap::new();
    for e in entities {
        if e["kind"].as_str() == Some("component") {
            by_name.insert(e["name"].as_str().unwrap_or(""), e);
        }
    }
    for (name, e) in &by_name {
        let kind = e["attributes"]["boundary_kind"].as_str().unwrap_or("");
        assert_ne!(
            kind, "cli",
            "component {name} must not be cli without cli evidence"
        );
    }
}
