//! Part C/N task-artifact parity + ledger-novelty tests: JSON and text are
//! two views of ONE artifact; the delta is derived through the SAME
//! builder; the context-ledger novelty contract holds across calls.

use scc_context::ContextPack;

// trace:v1 id=test.scc-cli-task-parity work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching verifies=REQ-complete-task-context-identical-across-transports exercises=impl.crates-scc-cli-src-commands.build-task-context
fn fixture_repo() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(root.join("services")).unwrap();
    std::fs::write(
        root.join("README.md"),
        "Transcript radio service. The API layer routes requests; services own transcripts.",
    )
    .unwrap();
    std::fs::write(
        root.join("main.py"),
        "from services.transcripts import TranscriptService\n\ndef handle(path):\n    svc = TranscriptService()\n    return svc.get(path)\n",
    )
    .unwrap();
    std::fs::write(
        root.join("services/transcripts.py"),
        "class TranscriptService:\n    def get(self, path):\n        return open(path).read()\n",
    )
    .unwrap();
    scc_cli::commands::cmd_index(&root, true).unwrap();
    (dir, root)
}

/// JSON and text derive from ONE builder: the serialized artifact carries
/// the same pack content and the delta block; deserialization round-trips.
#[test]
// trace:v1 id=test.scc-cli-task-parity.json-text-one-artifact work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching verifies=REQ-complete-task-context-identical-across-transports
fn json_and_text_are_views_of_one_artifact() {
    let (_dir, root) = fixture_repo();
    let goal = "rename the transcript field in the api response";
    let artifact = scc_cli::commands::build_task_context(&root, goal, &[], &[], None, false)
        .expect("artifact");

    // pack shape
    assert_eq!(artifact.pack.kind, "task");
    assert!(artifact.pack.content.starts_with("# TASK"), "{:80}", artifact.pack.content);

    // delta present on a fresh ledger (something is always new at start-up)
    assert!(artifact.delta.starts_with("# SCC TASK DELTA"), "{}", artifact.delta);

    // serialization is the ONLY difference between transports
    let json = serde_json::to_string(&artifact).unwrap();
    let back: scc_cli::commands::TaskContextArtifact = serde_json::from_str(&json).unwrap();
    assert_eq!(back.pack.content, artifact.pack.content);
    assert_eq!(back.delta, artifact.delta);
    assert_eq!(back.delta_ids, artifact.delta_ids);
}

/// Ledger novelty (Wave 14E): ids rendered by call 1 are suppressed in
/// call 2's delta within the same model epoch (unchanged sources).
#[test]
// trace:v1 id=test.scc-cli-task-parity.ledger-novelty-suppression work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching verifies=REQ-complete-task-context-identical-across-transports
fn second_delta_suppresses_first_rendered_ids() {
    let (_dir, root) = fixture_repo();
    let goal = "rename the transcript field in the api response";
    let first = scc_cli::commands::build_task_context(&root, goal, &[], &[], None, false).unwrap();
    assert!(!first.delta_ids.is_empty(), "first delta must render something on a fresh ledger");
    let second = scc_cli::commands::build_task_context(&root, goal, &[], &[], None, false).unwrap();
    let overlap: Vec<&String> = second
        .delta_ids
        .iter()
        .filter(|id| first.delta_ids.contains(id))
        .collect();
    assert!(
        overlap.is_empty(),
        "second delta re-injected already-visible ids: {overlap:?}"
    );
}

/// The pack-only compatibility shim returns the SAME pack half.
#[test]
// trace:v1 id=test.scc-cli-task-parity.pack-shim-equals-artifact-pack work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
fn pack_shim_matches_artifact_pack() {
    let (_dir, root) = fixture_repo();
    let goal = "rename the transcript field";
    let shim: ContextPack =
        scc_cli::commands::build_task_pack(&root, goal, &[], &[], None).unwrap();
    let full = scc_cli::commands::build_task_context(&root, goal, &[], &[], None, false).unwrap();
    assert_eq!(shim.content, full.pack.content);
}
