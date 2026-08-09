//! Runtime replay tests (docs/TEST_PLAN.md §15 / SCC-164): stored traces must
//! reproduce exact OBSERVED aggregates (counts, latency, errors) and feed
//! reconciliation.

mod golden;
use golden::*;

const OTLP_TRACE: &str = r#"{
  "resourceSpans": [
    {
      "resource": {"attributes": [{"key": "service.name", "value": {"stringValue": "api"}}]},
      "scopeSpans": [{"spans": [
        {"traceId": "t1", "spanId": "s1", "name": "GET /x",
         "startTimeUnixNano": "1000000000", "endTimeUnixNano": "3000000000"},
        {"traceId": "t1", "spanId": "s2", "parentSpanId": "s1", "name": "db query",
         "startTimeUnixNano": "1200000000", "endTimeUnixNano": "2400000000"},
        {"traceId": "t1", "spanId": "s3", "parentSpanId": "s1", "name": "queue send",
         "startTimeUnixNano": "1500000000", "endTimeUnixNano": "2000000000",
         "status": {"code": 2}}
      ]}]
    },
    {
      "resource": {"attributes": [{"key": "service.name", "value": {"stringValue": "worker"}}]},
      "scopeSpans": [{"spans": [
        {"traceId": "t2", "spanId": "w1", "name": "process",
         "startTimeUnixNano": "5000000000", "endTimeUnixNano": "7000000000"}
      ]}]
    }
  ]
}"#;

#[test]
fn trace_replay_reproduces_aggregates() {
    let repo = copy_fixture("http-service-python");
    let root = workdir(repo.path());
    run_ok(&root, &["index", "--quiet"]);

    run_ok(&root, &["ingest", OTLP_TRACE]);
    let status = run_ok(&root, &["runtime", "status", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&status).unwrap();
    let edges = v.as_array().unwrap();

    // root -> api (1 span, 2000ms)
    let root_api = edges
        .iter()
        .find(|e| e["source"] == "root" && e["target"] == "api")
        .expect("root->api edge");
    assert_eq!(root_api["count"], 1);
    assert_eq!(root_api["latency_ms"], 2000.0);
    assert_eq!(root_api["errors"], 0);

    // api -> api (2 child spans, avg (1200+500)/2 = 850ms, 1 error)
    let api_api = edges
        .iter()
        .find(|e| e["source"] == "api" && e["target"] == "api")
        .expect("api->api edge");
    assert_eq!(api_api["count"], 2);
    assert_eq!(api_api["errors"], 1);
    let latency: f64 = api_api["latency_ms"].as_f64().unwrap();
    assert!((latency - 850.0).abs() < 0.001, "latency {latency}");

    // worker trace root (1 span, 2000ms)
    let root_worker = edges
        .iter()
        .find(|e| e["source"] == "root" && e["target"] == "worker")
        .expect("root->worker edge");
    assert_eq!(root_worker["count"], 1);
    assert_eq!(root_worker["latency_ms"], 2000.0);

    // replay the same trace: aggregates are additive
    run_ok(&root, &["ingest", OTLP_TRACE]);
    let status = run_ok(&root, &["runtime", "status", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&status).unwrap();
    let edges = v.as_array().unwrap();
    let api_api = edges
        .iter()
        .find(|e| e["source"] == "api" && e["target"] == "api")
        .unwrap();
    assert_eq!(api_api["count"], 4, "additive counts on replay");
    assert_eq!(api_api["errors"], 2, "additive errors on replay");
    let latency: f64 = api_api["latency_ms"].as_f64().unwrap();
    assert!((latency - 850.0).abs() < 0.001, "running average preserved: {latency}");

    // verify pack surfaces the aggregates
    let verify = run_ok(&root, &["verify"]);
    assert!(verify.contains("observed edge(s)"), "{verify}");
    assert!(verify.contains("total observations"), "{verify}");
}

#[test]
fn reconcile_reports_matched_edges() {
    // a tiny repo whose static calls match an ingested trace
    let repo = tempfile::TempDir::new().unwrap();
    let root = workdir(repo.path());
    std::fs::create_dir_all(root.join("svc")).unwrap();
    std::fs::write(
        root.join("svc/a.py"),
        "from svc.b import b\n\ndef a():\n    return b()\n",
    )
    .unwrap();
    std::fs::write(root.join("svc/b.py"), "def b():\n    return 1\n").unwrap();
    run_ok(&root, &["index", "--quiet"]);

    // observed: svc -> svc (component-level)
    run_ok(
        &root,
        &[
            "ingest",
            r#"[{"source":"svc","target":"svc","count":3}]"#,
        ],
    );
    let rec = run_ok(&root, &["runtime", "reconcile"]);
    assert!(rec.contains("matched:"), "{rec}");
    // the static a->b call maps both sides to component "svc", so the
    // observed svc -> svc edge should match
    assert!(rec.contains("[matched] svc -> svc"), "{rec}");
}
