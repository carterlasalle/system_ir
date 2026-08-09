//! P1-wave integration tests: infrastructure extraction, behavioral views
//! (lifecycle/workflow), runtime reconciliation, CI checks, and evidence
//! import/export round trips (docs/EPICS_AND_TICKETS.md EPIC-050/140/160/180).

mod golden;
use golden::*;

const STATE_MACHINE_PY: &str = r#"
from enum import Enum

class OrderStatus(Enum):
    PENDING = 1
    ACTIVE = 2
    DONE = 3

class OrderStateMachine:
    def __init__(self):
        self.status = OrderStatus.PENDING

    def advance(self):
        self.status = OrderStatus.ACTIVE

    def complete(self):
        self.status = OrderStatus.DONE

    def cancel(self):
        self.status = OrderStatus.PENDING
"#;

#[test]
fn infra_extraction_e2e() {
    let repo = tempfile::TempDir::new().unwrap();
    let root = workdir(repo.path());
    std::fs::create_dir_all(root.join("k8s")).unwrap();
    std::fs::create_dir_all(root.join(".github/workflows")).unwrap();
    std::fs::write(
        root.join("k8s/deploy.yaml"),
        "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: api\n  namespace: prod\nspec:\n  replicas: 2\n  template:\n    spec:\n      containers:\n        - name: api\n          image: my/api:1.0\n          env:\n            - name: API_KEY\n              value: super-secret-value\n",
    )
    .unwrap();
    std::fs::write(
        root.join("main.tf"),
        "resource \"aws_db_instance\" \"transcripts\" {\n  engine = \"postgres\"\n}\nresource \"aws_s3_bucket\" \"assets\" {}\nvariable \"region\" {}\n",
    )
    .unwrap();
    std::fs::write(
        root.join(".github/workflows/ci.yml"),
        "name: ci\non: [push, pull_request]\njobs:\n  test:\n    runs-on: ubuntu-latest\n    env:\n      LOG_LEVEL: info\n    steps:\n      - run: cargo test\n",
    )
    .unwrap();
    std::fs::write(root.join("app.py"), "def main():\n    pass\n").unwrap();
    run_ok(&root, &["index", "--quiet"]);

    let ir = run_ok(&root, &["export", "system-ir.json"]);
    // k8s deployment unit + namespace + env name (never the value)
    assert!(ir.contains("\"deployment_unit\""), "{ir}");
    assert!(ir.contains("\"api\""), "{ir}");
    assert!(ir.contains("\"prod\""), "{ir}");
    assert!(!ir.contains("super-secret-value"), "k8s env value leaked");
    assert!(ir.contains("API_KEY"), "env name kept as reference");
    // terraform store + variable
    assert!(ir.contains("\"transcripts\""), "{ir}");
    assert!(ir.contains("\"assets\""), "{ir}");
    assert!(ir.contains("\"region\""), "{ir}");
    // github actions workflow
    assert!(ir.contains("\"workflow\""), "{ir}");
    assert!(ir.contains("\"ci\""), "{ir}");
}

#[test]
fn lifecycle_and_workflow_views() {
    let repo = tempfile::TempDir::new().unwrap();
    let root = workdir(repo.path());
    std::fs::create_dir_all(root.join("orders")).unwrap();
    std::fs::write(root.join("orders/state.py"), STATE_MACHINE_PY).unwrap();
    std::fs::write(
        root.join("orders/retry.py"),
        "import tenacity\n\n@tenacity.retry\nclass Retrier:\n    def run(self):\n        pass\n\nclass Fallbacker:\n    def run(self):\n        try:\n            return self.run()\n        except Exception:\n            return None\n",
    )
    .unwrap();
    run_ok(&root, &["index", "--quiet"]);

    let flows = run_ok(&root, &["flows"]);
    assert!(
        flows.contains("orders-lifecycle"),
        "lifecycle view missing: {flows}"
    );
    assert!(
        flows.contains("orders-workflow"),
        "retry/fallback workflow view missing: {flows}"
    );

    let lc = run_ok(&root, &["context", "flow", "orders-lifecycle"]);
    assert!(lc.contains("OrderStatus"), "{lc}");
    assert!(lc.contains("advance"), "{lc}");
}

#[test]
fn runtime_ingest_and_reconcile_cli() {
    let repo = copy_fixture("http-service-python");
    run_ok(&workdir(repo.path()), &["index", "--quiet"]);

    run_ok(
        &workdir(repo.path()),
        &[
            "ingest",
            r#"[{"source":"api","target":"db","count":10},{"source":"db","target":"api","count":2,"errors":1}]"#,
        ],
    );
    let status = run_ok(&workdir(repo.path()), &["runtime", "status"]);
    assert!(status.contains("2 observed edge(s)"), "{status}");
    assert!(status.contains("api → db ×10"), "{status}");

    let rec = run_ok(&workdir(repo.path()), &["runtime", "reconcile"]);
    assert!(rec.contains("static-vs-observed"), "{rec}");
    assert!(rec.contains("matched:"), "{rec}");

    // verify pack surfaces the runtime section
    let verify = run_ok(&workdir(repo.path()), &["verify"]);
    assert!(verify.contains("RUNTIME"), "{verify}");
    assert!(verify.contains("observed edge(s)"), "{verify}");
}

#[test]
fn ci_check_policy() {
    let repo = tempfile::TempDir::new().unwrap();
    let root = workdir(repo.path());
    std::fs::create_dir_all(root.join(".scc")).unwrap();
    std::fs::write(root.join("a.py"), "def a():\n    pass\n").unwrap();
    std::fs::write(
        root.join(".scc/intent.yaml"),
        "invariants:\n  critical-one:\n    statement: must hold\n    severity: critical\n",
    )
    .unwrap();
    run_ok(&root, &["index", "--quiet"]);

    // critical invariant without enforcing test -> ci check fails at medium
    let out = run(&root, &["ci", "check"]);
    assert!(!out.status.success(), "unenforced critical invariant must fail CI");
    assert!(String::from_utf8_lossy(&out.stdout).contains("[ci:fail]"));

    // same repo passes when the invariant is declared enforced by a real test
    std::fs::write(
        root.join(".scc/intent.yaml"),
        "invariants:\n  critical-one:\n    statement: must hold\n    severity: critical\n    enforced_by: [test_critical_one]\n",
    )
    .unwrap();
    std::fs::write(
        root.join("test_a.py"),
        "def test_critical_one():\n    assert True\n",
    )
    .unwrap();
    run_ok(&root, &["index", "--quiet"]);
    let out = run(&root, &["ci", "check"]);
    assert!(out.status.success(), "enforced invariant passes CI: {}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn ccg_import_roundtrip() {
    let repo = copy_fixture("http-service-python");
    let root = workdir(repo.path());
    run_ok(&root, &["index", "--quiet"]);
    let ccg = run_ok(&root, &["export", "ccg"]);
    let path = root.join("export.ccg.json");
    std::fs::write(&path, ccg).unwrap();
    let out = run_ok(&root, &["import", "ccg", path.to_str().unwrap()]);
    assert!(
        out.contains("imported") && out.contains("symbols"),
        "ccg import report: {out}"
    );
}

#[test]
fn scip_import_creates_resolved_facts() {
    let repo = tempfile::TempDir::new().unwrap();
    let root = workdir(repo.path());
    std::fs::create_dir_all(&root).unwrap();
    run_ok(&root, &["index", "--quiet"]);
    let scip = r#"{
      "metadata": {"version": 0, "tool": {"name": "scip-python", "version": "0.1.0"}},
      "documents": [
        {
          "language": "python",
          "relative_path": "a.py",
          "occurrences": [
            {"range": [0, 5], "symbol": "scip-python python pkg mod#helper", "symbol_roles": 1},
            {"range": [10, 18], "symbol": "scip-python python pkg mod#main", "symbol_roles": 1}
          ],
          "symbols": []
        },
        {
          "language": "python",
          "relative_path": "b.py",
          "occurrences": [
            {"range": [0, 8], "symbol": "scip-python python pkg mod#caller", "symbol_roles": 1},
            {"range": [12, 18], "symbol": "scip-python python pkg mod#helper", "symbol_roles": 2}
          ],
          "symbols": []
        }
      ]
    }"#;
    let path = root.join("index.scip");
    std::fs::write(&path, scip).unwrap();
    let out = run_ok(&root, &["import", "scip", path.to_str().unwrap()]);
    assert!(out.contains("imported 3 symbols"), "{out}");
    let ir = run_ok(&root, &["export", "system-ir.json"]);
    assert!(ir.contains("\"helper\""), "{ir}");
    assert!(ir.contains("\"calls\""), "{ir}");
    // scip extractor evidence recorded
    assert!(ir.contains("\"scip\""), "{ir}");
}
