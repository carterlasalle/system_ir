//! Context benchmark integration tests (docs/TEST_PLAN.md §84–87).

mod golden;

#[test]
fn benchmark_corpus_runs_and_meets_recall_gate() {
    let summary = scc_cli::benchctx::run_context_benchmark(0.9).expect("benchmark gate");
    assert_eq!(summary.tasks, 21);
    assert!(summary.mean_recall >= 0.95, "recall {:.3}", summary.mean_recall);
    assert!(summary.mean_localization >= 0.9, "localization {:.3}", summary.mean_localization);
    assert_eq!(summary.hallucination_violations, 0);
    assert_eq!(summary.budget_ok, summary.tasks);
    // every task must be at least individually decent
    for r in &summary.results {
        assert!(r.recall >= 0.7, "task {} recall {:.3}", r.id, r.recall);
    }
}

#[test]
fn hallucination_gate_fails_when_nonexistent_entity_surfaces() {
    // craft a task whose pack legitimately contains a name we mark as a
    // hallucination — the gate must flag it
    let summary = scc_cli::benchctx::run_context_benchmark(0.9).expect("benchmark gate");
    // the corpus is clean, so simulate a violation by checking the
    // hallucination scan logic directly against a known-real entity
    let fixtures = scc_cli::benchctx::locate_fixtures_dir().unwrap();
    let repo_dir = fixtures.join("http-service-python");
    let task = scc_cli::benchctx::BenchTask {
        id: "hallucination-probe".into(),
        repo: "http-service-python".into(),
        goal: "rename the transcript field in the api response".into(),
        ground_truth: scc_cli::benchctx::GroundTruth {
            files: vec!["main.py".into()],
            ..Default::default()
        },
        hallucinations: vec![scc_cli::benchctx::Hallucination {
            kind: "symbol".into(),
            name: "handle_transcripts".into(), // real symbol: would surface
        }],
    };
    let result = scc_cli::benchctx::score_task_public(&repo_dir, &task).unwrap();
    assert_eq!(
        result.hallucinations_hit.len(),
        1,
        "a name in the pack must be flagged when marked as hallucination"
    );
    let _ = summary;
}
