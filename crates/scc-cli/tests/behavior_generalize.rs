//! Behavior flows from NATIVE call chains (Wave 9 holdout generalization):
//! a fixture with same-file calls — indexed WITHOUT any language server —
//! must yield sequence flows with EXTRACTED steps, and the atlas FLOWS
//! section must show the chain. LSP resolution is optional; behavior must
//! not depend on it.

use std::path::Path;

mod golden;
// trace:v1 id=test.scc.behavior verifies=REQ-SCC-FLOW exercises=impl.scc.flows,impl.scc.flowgraph

/// Parse the `scc export flow-graphs.json` output for the given fixture
/// (indexed natively — no `resolve` pass ever runs in these tests).
fn flow_graphs(dir: &Path) -> serde_json::Value {
    let out = golden::run_ok(dir, &["export", "flow-graphs.json"]);
    let v: serde_json::Value = serde_json::from_str(&out).expect("flow-graphs.json parses");
    assert!(v.is_array() && !v.as_array().unwrap().is_empty(), "fixture yields flow graphs");
    v
}

#[test]
fn native_same_file_calls_yield_sequence_flow_with_extracted_steps() {
    // The behavior-native fixture is pure intra-file Python: run ->
    // handle -> normalize -> {validate, parse}. The native extractor
    // resolves every callee to a local symbol, so the canonical flow graph
    // exists with EXTRACTED edges — no LSP, no resolve, no pyright.
    let repo = golden::copy_fixture("behavior-native");
    let dir = golden::workdir(repo.path());
    golden::run_ok(&dir, &["index", "--quiet"]);

    let graphs = flow_graphs(&dir);
    let graphs = graphs.as_array().unwrap();

    // the main-guard entrypoint `run` produces the pipeline flow
    let graph = graphs
        .iter()
        .find(|g| g["name"].as_str() == Some("run"))
        .unwrap_or_else(|| panic!("flow graph for `run` exists: {}", serde_json::to_string(&graphs).unwrap()));

    let nodes = graph["nodes"].as_array().unwrap();
    let ops: Vec<&str> = nodes
        .iter()
        .filter_map(|n| n["operation"].as_str())
        .collect();
    for want in ["run", "handle", "normalize", "validate", "parse"] {
        assert!(
            ops.iter().any(|o| o.ends_with(want)),
            "flow graph reaches {want}: {ops:?}"
        );
    }

    // the run -> handle edge is native evidence (EXTRACTED), not LSP proof
    let edges = graph["edges"].as_array().unwrap();
    assert!(
        edges.iter().any(|e| e["provenance"] == "EXTRACTED"),
        "native call edges carry EXTRACTED provenance: {}",
        serde_json::to_string(&edges).unwrap()
    );
    let run_idx = nodes
        .iter()
        .position(|n| n["operation"].as_str().map(|o| o.ends_with("run")).unwrap_or(false))
        .unwrap();
    let handle_idx = nodes
        .iter()
        .position(|n| n["operation"].as_str().map(|o| o.ends_with("handle")).unwrap_or(false))
        .unwrap();
    let run_handle = edges
        .iter()
        .find(|e| e["from"] == run_idx && e["to"] == handle_idx)
        .unwrap_or_else(|| panic!("run -> handle edge exists: {}", serde_json::to_string(&edges).unwrap()));
    assert_eq!(
        run_handle["provenance"], "EXTRACTED",
        "same-file native call is EXTRACTED evidence"
    );

    // no LSP ran: RESOLVED edges only ever appear for compiler-invented
    // joins, never for call edges
    let call_edges_resolved: Vec<&serde_json::Value> = edges
        .iter()
        .filter(|e| e["kind"] == "next" && e["provenance"] == "RESOLVED")
        .collect();
    assert!(
        call_edges_resolved.is_empty(),
        "no resolved call edges without a language server: {:?}",
        call_edges_resolved
    );
}

#[test]
fn atlas_flows_section_shows_native_chain() {
    // The rendered atlas FLOWS section must show the native chain: the
    // `run` sequence flow with handle/normalize/validate/parse steps.
    let repo = golden::copy_fixture("behavior-native");
    let dir = golden::workdir(repo.path());
    golden::run_ok(&dir, &["index", "--quiet"]);

    let atlas = golden::run_ok(&dir, &["atlas"]);
    let flows = atlas
        .split("FLOWS")
        .nth(1)
        .unwrap_or_default()
        .split("STATE & DATA AUTHORITY")
        .next()
        .unwrap_or_default();
    assert!(
        flows.contains("run [sequence]"),
        "FLOWS section lists the native run flow: {flows}"
    );
    for step in ["handle", "normalize", "validate", "parse"] {
        assert!(
            flows.contains(step),
            "FLOWS section shows the {step} step: {flows}"
        );
    }
    // the chain is a same-file pipeline: the entry actor and callee actors
    // are the same component (root — the fixture is a single top-level
    // directory), so the steps read as one actor's call chain
    assert!(
        flows.contains("root: run") && flows.contains("-> root: handle"),
        "run -> handle chain rendered from native evidence: {flows}"
    );
}
