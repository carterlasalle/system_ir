//! CFG-backed causal FlowGraph tests (Wave 3, docs/SYSTEM_DESIGN.md §9):
//! control-block evidence from the extractors — lexical order, block kind,
//! awaited — drives branch edges, Next-edge ordering, and Async edges.
//! Branches come from CFG evidence, never text heuristics.

mod golden;

use golden::{copy_fixture, run_ok, workdir};

#[test]
fn cfg_evidence_drives_branch_async_and_lexical_order() {
    let repo = copy_fixture("cfg-service");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);

    let out = run_ok(&dir, &["export", "flow-graphs.json"]);
    let graphs: serde_json::Value = serde_json::from_str(&out).unwrap();
    let graphs = graphs.as_array().expect("flow graphs array");
    assert!(!graphs.is_empty(), "at least one flow graph");

    let graph = graphs
        .iter()
        .find(|g| g["name"] == "main")
        .unwrap_or_else(|| panic!("main flow graph, got: {:?}", graphs));
    let nodes = graph["nodes"].as_array().unwrap();
    let edges = graph["edges"].as_array().unwrap();

    let node_of = |op: &str| -> u32 {
        nodes
            .iter()
            .find(|n| n["operation"].as_str() == Some(op))
            .map(|n| n["id"].as_u64().unwrap() as u32)
            .unwrap_or_else(|| panic!("node `{op}` missing: {nodes:?}"))
    };
    let edge_to = |from: u32, to: u32| -> &serde_json::Value {
        edges
            .iter()
            .find(|e| e["from"].as_u64() == Some(from as u64) && e["to"].as_u64() == Some(to as u64))
            .unwrap_or_else(|| panic!("no edge {from} -> {to}: {edges:?}"))
    };
    let index_of = |from: u32, to: u32| -> usize {
        edges
            .iter()
            .position(|e| {
                e["from"].as_u64() == Some(from as u64) && e["to"].as_u64() == Some(to as u64)
            })
            .expect("edge present")
    };

    let proc = node_of("process");

    // Branch for save/reject: control-block evidence (if/else) is the
    // condition — never a fanout heuristic.
    let save = edge_to(proc, node_of("save"));
    assert_eq!(save["kind"], "branch", "save is a Branch edge: {save:?}");
    assert_eq!(save["condition"], "if", "condition names the block kind: {save:?}");
    let reject = edge_to(proc, node_of("reject"));
    assert_eq!(reject["kind"], "branch", "reject is a Branch edge: {reject:?}");
    assert_eq!(reject["condition"], "else", "condition names the block kind: {reject:?}");

    // validate is called inside the try body — CFG block evidence "try".
    let validate = edge_to(proc, node_of("validate"));
    assert_eq!(validate["kind"], "branch", "validate inside try: {validate:?}");
    assert_eq!(validate["condition"], "try", "{validate:?}");

    // cleanup runs in finally: guaranteed, so it is a plain Next edge that
    // follows the branches by lexical order (validate 0, save 1, reject 2,
    // cleanup 3 — source order, not alphabetical).
    let cleanup = edge_to(proc, node_of("cleanup"));
    assert_eq!(cleanup["kind"], "next", "finally cleanup is sequential: {cleanup:?}");
    let i_validate = index_of(proc, node_of("validate"));
    let i_save = index_of(proc, node_of("save"));
    let i_reject = index_of(proc, node_of("reject"));
    let i_cleanup = index_of(proc, node_of("cleanup"));
    assert!(
        i_validate < i_save && i_save < i_reject && i_reject < i_cleanup,
        "Next edges ordered by lexical_order: validate<save<reject<cleanup, got \
         {i_validate} < {i_save} < {i_reject} < {i_cleanup}"
    );

    // The awaited call (`await tick()`) is an Async edge.
    let persist = node_of("persist");
    let tick_id = node_of("tick");
    let tick = edges
        .iter()
        .find(|e| {
            e["from"].as_u64() == Some(persist as u64)
                && e["to"].as_u64() == Some(tick_id as u64)
                && e["kind"] == "async"
        })
        .unwrap_or_else(|| panic!("no async edge persist -> tick: {edges:?}"));
    assert_eq!(tick["condition"], "awaited: tick", "{tick:?}");

    // Plain fanout is Next-only with zero Branch edges.
    let fanout = node_of("fanout");
    let fanout_edges: Vec<&serde_json::Value> = edges
        .iter()
        .filter(|e| e["from"].as_u64() == Some(fanout as u64))
        .collect();
    assert_eq!(fanout_edges.len(), 2, "fanout -> first/second: {fanout_edges:?}");
    assert!(
        fanout_edges.iter().all(|e| e["kind"] == "next"),
        "plain fanout stays Next (no branch invented): {fanout_edges:?}"
    );
    assert!(
        fanout_edges.iter().all(|e| e["kind"] != "branch"),
        "zero Branch edges for straight-line fanout: {fanout_edges:?}"
    );
}

#[test]
fn flow_graphs_are_deterministic() {
    let repo = copy_fixture("cfg-service");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);

    let first = run_ok(&dir, &["export", "flow-graphs.json"]);
    let second = run_ok(&dir, &["export", "flow-graphs.json"]);
    assert_eq!(first, second, "flow-graphs.json must be byte-identical across runs");
}
