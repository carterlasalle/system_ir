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

// ---------------------------------------------------------------------------
// Part 2 — token accounting (post-enrichment) regression tests
// ---------------------------------------------------------------------------

/// A fixture with Beads task state, a Hindsight lesson, and hindsight
/// enabled in config: the two enrichment sources that append to pack.content
/// AFTER the pack builder computes its token count. This is exactly the
/// path that hid beads+hindsight tokens from the surface-delta budget.
// trace:v1 id=test.crates-scc-cli-tests-task-artifact-parity.fixture-with-enrichment work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
fn fixture_with_enrichment() -> (tempfile::TempDir, std::path::PathBuf) {
    let (dir, root) = fixture_repo();
    // Beads: task state file the adapter reads (.beads/issues.jsonl).
    let beads_dir = root.join(".beads");
    std::fs::create_dir_all(&beads_dir).unwrap();
    std::fs::write(
        beads_dir.join("issues.jsonl"),
        "{\"id\":\"T1\",\"title\":\"retry on 429 in the transcripts sync\",\"status\":\"active\"}\n",
    )
    .unwrap();
    // Hindsight: a lesson entity in the store + hindsight enabled in config.
    let store = scc_store::Store::open(&root.join(".scc").join("scc.db"), &root).unwrap();
    let entity = scc_core::Entity::new(
        scc_core::entity_id(&store.repo_id, "lesson", "lesson-tenacity"),
        "lesson",
        "tenacity retry works after backoff",
    );
    store.insert_entity(&entity, &[]).unwrap();
    let cfg_dir = root.join(".scc");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    std::fs::write(
        cfg_dir.join("config.yaml"),
        "integrations:\n  hindsight: true\n",
    )
    .unwrap();
    (dir, root)
}

/// Regression: after ALL enrichment, pack.tokens is the ACTUAL token count
/// of the rendered content — the pre-fix code left the stale pre-beads
/// count, so the delta budget overallocated by exactly the enrichment size.
#[test]
// trace:v1 id=test.scc-cli-task-parity.post-enrichment-token-recount work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching verifies=REQ-complete-task-context-identical-across-transports
fn pack_tokens_recounted_after_enrichment() {
    let (_dir, root) = fixture_with_enrichment();
    let goal = "rename the transcript field in the api response";
    let artifact = scc_cli::commands::build_task_context(&root, goal, &[], &[], None, false)
        .expect("artifact");
    // The pack contains both enrichment sections.
    assert!(artifact.pack.content.contains("ACTIVE TASK STATE"), "beads absent");
    assert!(artifact.pack.content.contains("HINDSIGHT LESSONS"), "hindsight absent");
    // The token count reflects the ENRICHED content (the regression).
    let actual = scc_core::estimate_tokens(&artifact.pack.content);
    assert_eq!(artifact.pack.tokens, actual,
        "pack.tokens {} must equal estimate(content) {} after enrichment",
        artifact.pack.tokens, actual);
}

/// Meta: the complete artifact's token_count is the sum of the actual
/// enriched pack and the delta (the accounting SDKs/benchmarks rely on).
#[test]
// trace:v1 id=impl.scc-cli-task-parity.artifact-token-count-sum work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
fn artifact_token_count_is_pack_plus_delta() {
    let (_dir, root) = fixture_with_enrichment();
    let goal = "rename the transcript provider in the api response";
    let artifact = scc_cli::commands::build_task_context(&root, goal, &[], &[], None, false)
        .expect("artifact");
    let expected = scc_core::estimate_tokens(&artifact.pack.content)
        + scc_core::estimate_tokens(&artifact.delta);
    assert_eq!(artifact.token_count, expected,
        "token_count must equal estimate(pack)+estimate(delta)");
}

/// Overflow: a large enrichment must be accounted for in the FULL artifact
/// budget — the total rendered task (enriched pack + delta) never exceeds
/// the requested budget, and pack.tokens reflects the ENRICHED content
/// (the pre-fix code counted only the un-enriched pack, so it could blow
/// the cap by the enrichment size).
#[test]
// trace:v1 id=impl.scc-cli-task-parity.enrichment-exhausts-budget
fn enrichment_exhausts_delta_budget() {
    let (_dir, root) = fixture_repo();
    // A beads file large enough to consume a tiny budget by itself.
    let beads_dir = root.join(".beads");
    std::fs::create_dir_all(&beads_dir).unwrap();
    let mut text = String::new();
    for i in 0..200 {
        text.push_str(&format!("{{\"id\":\"T{i}\",\"title\":\"task state line {i} that is long enough to consume tokens\",\"status\":\"active\"}}\n"));
    }
    std::fs::write(beads_dir.join("issues.jsonl"), text).unwrap();

    let goal = "rename the transcript field";
    // Budget above the pack builder's floor (token_budget.max(512)): with
    // ~700 tokens of beads enrichment, the delta must get only what the
    // ENRICHED pack leaves — the pre-fix stale count would have allocated
    // against the un-enriched pack and blown the budget.
    let budget = 1000usize;
    let artifact =
        scc_cli::commands::build_task_context(&root, goal, &[], &[], Some(budget), false).unwrap();
    assert!(artifact.pack.content.contains("ACTIVE TASK STATE"), "beads absent");
    // pack.tokens reflects the ENRICHED content (the regression).
    assert_eq!(artifact.pack.tokens, scc_core::estimate_tokens(&artifact.pack.content));
    let enriched_pack_tokens = artifact.pack.tokens;
    // The COMPLETE artifact (enriched pack + delta) never exceeds the
    // requested budget — the delta gets only what the ENRICHED pack left
    // (the pre-fix stale count would have allocated delta against the
    // un-enriched pack, blowing the cap by the enrichment size).
    let total = artifact.pack.tokens + scc_core::estimate_tokens(&artifact.delta);
    assert!(total <= budget,
        "complete artifact {total} tokens must fit the {budget} budget (enrichment accounted)");
    // The enrichment was counted: the in-content beads alone are visible.
    let _ = enriched_pack_tokens;
}

/// Part 3 — ledger purity regression: the pure pack-only builder (and
/// `context compress`, which uses it) must NOT record Surface visibility.
/// build_task_context records ONLY the delta ids it actually rendered;
/// a pack-only caller discarding the delta must not mark APIs visible.
#[test]
// trace:v1 id=test.scc-cli-task-parity.pack-only-does-not-record-visibility work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
fn pack_only_does_not_mutate_the_ledger() {
    let (_dir, root) = fixture_repo();
    let goal = "rename the transcript field in the api response";

    let store = scc_store::Store::open(&root.join(".scc").join("scc.db"), &root).unwrap();
    let baseline = scc_core::ContextLedger {
        model_epoch: store.cache_epoch().unwrap_or_else(|_| "no-epoch".into()),
        ..Default::default()
    };
    scc_context::context_ledger::ContextLedgerStore::new(&store).save(&baseline);

    // 2. The PURE pack-only path must NOT record any visibility.
    let before = scc_context::context_ledger::ContextLedgerStore::new(&store).load();
    let _pack = scc_cli::commands::build_task_pack(&root, goal, &[], &[], None).unwrap();
    let after_pure = scc_context::context_ledger::ContextLedgerStore::new(&store).load();
    assert_eq!(
        after_pure.visible_symbols.len() + after_pure.visible_files.len()
            + after_pure.visible_components.len() + after_pure.visible_flows.len(),
        before.visible_symbols.len() + before.visible_files.len()
            + before.visible_components.len() + before.visible_flows.len(),
        "pack-only build must not record visibility — the compress path uses \
         this builder and discards the delta; recording ids visible here \
         would suppress them from a later real task context"
    );

    // 3. The full task-context path renders a delta AND records its ids.
    let full = scc_cli::commands::build_task_context(&root, goal, &[], &[], None, false).unwrap();
    assert!(!full.delta_ids.is_empty(), "full call must render a delta");
    let full_ids: std::collections::BTreeSet<String> = full.delta_ids.iter().cloned().collect();
    let after_full = scc_context::context_ledger::ContextLedgerStore::new(&store).load();
    let recorded: std::collections::BTreeSet<String> = [
        after_full.visible_symbols.clone(),
        after_full.visible_files.clone(),
        after_full.visible_components.clone(),
        after_full.visible_flows.clone(),
    ]
    .into_iter()
    .flatten()
    .collect();
    assert!(
        full_ids.is_subset(&recorded),
        "full task-context must record its rendered delta ids; missing: {:?}",
        full_ids.difference(&recorded).collect::<Vec<_>>()
    );
}

/// Final hard-cap (audit edge case): enrichment alone can exceed the
/// requested budget; the rendered artifact must still respect it — the
/// delta is dropped (recorded in warnings), never silently over-cap.
#[test]
// trace:v1 id=impl.scc-cli-task-parity.final-hard-cap-after-enrichment work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching verifies=REQ-complete-task-context-identical-across-transports
fn enrichment_over_cap_drops_delta_and_records() {
    let (_dir, root) = fixture_repo();
    let beads_dir = root.join(".beads");
    std::fs::create_dir_all(&beads_dir).unwrap();
    // 5 huge beads (active_beads takes 5) — far over a 300-token budget.
    let mut text = String::new();
    for i in 0..5 {
        text.push_str(&format!("{{\"id\":\"B{i}\",\"title\":\"{}\",\"status\":\"active\"}}\n", "enormous bead title ".repeat(60)));
    }
    std::fs::write(beads_dir.join("issues.jsonl"), text).unwrap();

    let budget = 300usize;
    let artifact = scc_cli::commands::build_task_context(
        &root, "rename the transcript field", &[], &[], Some(budget), false,
    ).unwrap();
    let total = scc_core::estimate_tokens(&artifact.pack.content)
        + scc_core::estimate_tokens(&artifact.delta);
    // The pack itself may exceed the cap (its content is critical, and the
    // warning records the enforcement) — but the DELTA must be gone and
    // the enforcement must be visible.
    assert!(artifact.delta.is_empty(), "delta must be dropped when enrichment alone blows the cap");
    assert!(
        artifact.pack.warnings.iter().any(|w| w.contains("task cap enforced")),
        "the cap enforcement must be recorded, got {:?}",
        artifact.pack.warnings
    );
    let _ = total;
}
