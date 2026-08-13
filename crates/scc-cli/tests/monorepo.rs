//! Monorepo acceptance-scenario tests (docs/PRD.md §10) and intent/drift
//! behavior (EPIC-180).

mod golden;
use golden::*;

const INTENT: &str = r#"
components:
  api:
    responsibility:
      - serve transcript records over HTTP
    owns:
      - transcript
invariants:
  raw-transcript-immutable:
    statement: Raw ASR output must never be overwritten by transcript normalization.
    severity: critical
    scope: [transcript]
    enforced_by: [normalization_preserves_raw]
flows:
  live-radio:
    entrypoint: consume
    kind: sequence
    trigger: radio-audio event
"#;

fn monorepo_with_intent() -> tempfile::TempDir {
    let repo = copy_fixture("monorepo-acceptance");
    std::fs::create_dir_all(workdir(repo.path()).join(".scc")).unwrap();
    std::fs::write(workdir(repo.path()).join(".scc/intent.yaml"), INTENT).unwrap();
    run_ok(&workdir(repo.path()), &["index", "--quiet"]);
    repo
}

#[test]
fn acceptance_scenario_entities_identified_before_any_edit() {
    let repo = monorepo_with_intent();
    let out = run(
        &workdir(repo.path()),
        &[
            "context",
            "task",
            "rename transcript response field in the API response",
            "--json",
        ],
    );
    assert!(out.status.success());
    let pack: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let ids: Vec<&str> = pack["entity_ids"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();

    // docs §94 ground truth: API handler, frontend consumer, worker consumer,
    // schema, response contract, tests, affected flow, persistence mapping.
    let must_contain = [
        // semantic clustering: api + shared (db hub) + worker merge into
        // one backend component; frontend stays separate
        "component/api-shared-worker",
        "component/frontend",
        "route/get-/api/transcripts",
        "route/post-/api/transcripts",
        "symbol/frontend/view.ts/renderTranscript",
        "symbol/worker/indexer.ts/indexTranscripts",
        "data/prisma.transcript",
        "file/api/routes.ts",
    ];
    for needle in must_contain {
        assert!(
            ids.iter().any(|id| id.contains(needle)),
            "task pack missing {needle}; ids: {ids:?}"
        );
    }

    // invariant + flow must appear in the pack content
    let content = pack["content"].as_str().unwrap();
    assert!(
        content.contains("Raw ASR output must never be overwritten"),
        "critical invariant missing from task pack"
    );
    assert!(content.contains("PRIMARY FLOW"), "flow missing from task pack");

    // token budget respected
    let tokens = pack["tokens"].as_u64().unwrap();
    let budget = pack["budget"].as_u64().unwrap();
    assert!(tokens <= budget, "{tokens} > {budget}");

    // trusted sections carry no STALE facts
    assert!(!content.contains("STALE"), "stale facts in trusted pack");
}

#[test]
fn intent_invariants_and_drift() {
    let repo = monorepo_with_intent();
    let status = run_ok(&workdir(repo.path()), &["status"]);
    assert!(status.contains("invariants:"), "{status}");

    let drift = run_ok(&workdir(repo.path()), &["drift"]);
    assert!(
        drift.contains("invariant_test_missing"),
        "declared enforcing test does not exist here: {drift}"
    );

    // the critical invariant is declared in the component pack scope check
    let overview = run_ok(&workdir(repo.path()), &["overview"]);
    assert!(overview.contains("Raw ASR output"), "{overview}");
}

#[test]
fn impact_identifies_downstream_consumers() {
    let repo = monorepo_with_intent();
    let impact = run_ok(&workdir(repo.path()), &["impact", "api/routes.ts"]);
    assert!(impact.contains("AFFECTED COMPONENTS"), "{impact}");
    // the backend component is the merged api+shared+worker cluster
    assert!(impact.contains("api+shared+worker"), "{impact}");
    assert!(impact.contains("DOWNSTREAM"), "{impact}");
    assert!(impact.contains("shared"), "{impact}");
    assert!(impact.contains("CONTRACTS"), "{impact}");
    assert!(impact.contains("GET /api/transcripts/:id"), "{impact}");
    assert!(impact.contains("Raw ASR output must never be overwritten"), "{impact}");
}

#[test]
fn precision_and_recall_on_acceptance_task() {
    let repo = monorepo_with_intent();
    let out = run(
        &workdir(repo.path()),
        &[
            "context",
            "task",
            "rename transcript response field in the API response",
            "--json",
        ],
    );
    let pack: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let ids: Vec<String> = pack["entity_ids"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();

    // ground truth (relevant entities for this change)
    let ground_truth = [
        "component/api-shared-worker",
        "component/frontend",
        "file/shared/db.ts",
        "route/get-/api/transcripts",
        "route/post-/api/transcripts",
        "symbol/frontend/view.ts/renderTranscript",
        "symbol/worker/indexer.ts/indexTranscripts",
        "file/api/routes.ts",
        "file/frontend/view.ts",
        "file/worker/indexer.ts",
        "data/prisma.transcript",
    ];
    let relevant: Vec<String> = ground_truth.iter().map(|s| s.to_string()).collect();
    let included = ids.len() as f64;
    let hits = relevant
        .iter()
        .filter(|gt| ids.iter().any(|id| id.contains(gt.as_str())))
        .count();
    let recall = hits as f64 / relevant.len() as f64;
    let precision = hits as f64 / included;
    assert!(
        recall >= 0.9,
        "task context recall too low: {hits}/{} = {recall:.2}",
        relevant.len()
    );
    assert!(
        precision >= 0.4,
        "task context precision too low: {hits}/{included} = {precision:.2}"
    );
    eprintln!("recall={recall:.2} precision={precision:.2}");
}
