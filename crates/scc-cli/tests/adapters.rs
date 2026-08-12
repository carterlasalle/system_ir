//! External adapter integration tests (SCC-202/203/204/205): beads, CBM,
//! hindsight import + pack enrichment + Context7 docs.

mod golden;
use golden::*;

// Fake Context7 server speaking the REAL v4 protocol (JSONL transport,
// resolve-library-id + query-docs) — pinned by the live suite in
// tests/context7_live.rs against @upstash/context7-mcp.
const FAKE_C7: &str = r#"import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line.startswith("{"):
        continue  # tolerate Content-Length header lines, like the real server
    msg = json.loads(line)
    m = msg.get("method")
    if m == "initialize":
        out = {"jsonrpc":"2.0","id":msg["id"],"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"fake-c7"}}}
    elif m == "tools/call":
        name = msg["params"]["name"]
        if name == "resolve-library-id":
            result = {"content":[{"type":"text","text":"- Context7-compatible library ID: /fastapi/fastapi\n- Description: FastAPI"}]}
        else:
            result = {"content":[{"type":"text","text":"docs: FastAPI route docs here"}]}
        out = {"jsonrpc":"2.0","id":msg["id"],"result":result}
    else:
        continue
    sys.stdout.write(json.dumps(out) + "\n")
    sys.stdout.flush()
"#;

#[test]
fn beads_import_and_task_enrichment() {
    let repo = tempfile::TempDir::new().unwrap();
    let root = workdir(repo.path());
    std::fs::create_dir_all(root.join(".beads")).unwrap();
    std::fs::write(root.join("a.py"), "def helper():\n    return 1\n").unwrap();
    std::fs::write(
        root.join(".beads/issues.jsonl"),
        "{\"id\":\"b1\",\"title\":\"Fix retry\",\"status\":\"in_progress\",\"dependencies\":[\"b2\"]}\n{\"id\":\"b2\",\"title\":\"Add fallback\",\"status\":\"open\"}\n",
    )
    .unwrap();
    run_ok(&root, &["index", "--quiet"]);
    let out = run_ok(&root, &["import", "beads", ".beads/issues.jsonl"]);
    assert!(out.contains("imported 2"), "{out}");

    let task = run_ok(&root, &["context", "task", "helper"]);
    assert!(task.contains("ACTIVE TASK STATE"), "{task}");
    assert!(task.contains("Fix retry"), "{task}");
    assert!(!task.contains("Add fallback"), "open beads are not active: {task}");
    assert!(task.contains("task state, not system facts"), "{task}");
}

#[test]
fn hindsight_import_and_lesson_enrichment() {
    let repo = tempfile::TempDir::new().unwrap();
    let root = workdir(repo.path());
    std::fs::create_dir_all(root.join(".scc")).unwrap();
    std::fs::write(root.join("a.py"), "def helper():\n    return 1\n").unwrap();
    std::fs::write(
        root.join(".scc/config.yaml"),
        "schema: 1\nintegrations:\n  hindsight: true\n",
    )
    .unwrap();
    std::fs::write(
        root.join("lessons.jsonl"),
        "{\"id\":\"l1\",\"text\":\"retry with backoff works\",\"tags\":[\"retry\"]}\n",
    )
    .unwrap();
    run_ok(&root, &["index", "--quiet"]);
    let out = run_ok(&root, &["import", "hindsight", "lessons.jsonl"]);
    assert!(out.contains("imported 1"), "{out}");

    let task = run_ok(&root, &["context", "task", "helper"]);
    assert!(task.contains("HINDSIGHT LESSONS"), "{task}");
    assert!(task.contains("retry with backoff works"), "{task}");
    assert!(task.contains("not verified facts"), "{task}");
}

#[test]
fn cbm_import_from_zst_snapshot() {
    let repo = tempfile::TempDir::new().unwrap();
    let root = workdir(repo.path());
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.py"), "def helper():\n    return 1\n").unwrap();
    run_ok(&root, &["index", "--quiet"]);

    // build a CBM-style graph.db.zst with python + zstd CLI
    let script = r#"
import sqlite3, subprocess, os
c = sqlite3.connect('graph.db')
c.executescript('''
CREATE TABLE symbols(id INTEGER PRIMARY KEY, name TEXT, kind TEXT, file TEXT);
INSERT INTO symbols(name, kind, file) VALUES ('helper','function','a.py'), ('main','function','b.py');
CREATE TABLE relationships(id INTEGER PRIMARY KEY, source TEXT, predicate TEXT, target TEXT);
INSERT INTO relationships(source, predicate, target) VALUES ('main','calls','helper');
''')
c.commit(); c.close()
subprocess.run(['zstd','-q','-f','graph.db','-o','graph.db.zst'], check=True)
os.remove('graph.db')
"#;
    let py = root.join("mk.py");
    std::fs::write(&py, script).unwrap();
    let python3 = std::env::var("PYTHON3")
        .unwrap_or_else(|_| "python3".to_string());
    let out = std::process::Command::new(python3)
        .arg(&py)
        .current_dir(&root)
        .output()
        .expect("python3");
    if !out.status.success() {
        eprintln!("python3/zstd unavailable — skipping");
        return;
    }
    let out = run_ok(&root, &["import", "cbm", "graph.db.zst"]);
    assert!(out.contains("imported 2"), "{out}");
    let ir = run_ok(&root, &["export", "system-ir.json"]);
    assert!(ir.contains("\"cbm\""), "{ir}");
    assert!(ir.contains("\"helper\""), "{ir}");
}

#[test]
fn context7_docs_via_mcp() {
    let repo = tempfile::TempDir::new().unwrap();
    let root = workdir(repo.path());
    std::fs::create_dir_all(root.join(".scc")).unwrap();
    std::fs::write(root.join("a.py"), "def helper():\n    return 1\n").unwrap();
    let fake = root.join("fake_c7.py");
    std::fs::write(&fake, FAKE_C7).unwrap();
    std::fs::write(
        root.join(".scc/config.yaml"),
        format!(
            "schema: 1\nintegrations:\n  context7_command: \"python3 {}\"\n",
            fake.display()
        ),
    )
    .unwrap();
    run_ok(&root, &["index", "--quiet"]);
    let out = run_ok(&root, &["context", "docs", "fastapi/fastapi"]);
    assert!(out.contains("CONTEXT7 EXTERNAL DOCUMENTATION"), "{out}");
    assert!(out.contains("FastAPI route docs"), "{out}");
}

#[test]
fn adapters_command_lists_configured_scope() {
    let repo = tempfile::TempDir::new().unwrap();
    let root = workdir(repo.path());
    std::fs::create_dir_all(root.join(".scc")).unwrap();
    std::fs::write(root.join("a.py"), "def helper():\n    return 1\n").unwrap();
    std::fs::write(
        root.join(".scc/config.yaml"),
        "schema: 1\nintegrations:\n  beads: true\n  context7_command: \"npx -y @upstash/context7-mcp\"\n",
    )
    .unwrap();
    let out = run_ok(&root, &["adapters"]);
    assert!(
        out.contains("adapter: context7  scope: network+subprocess(npx)"),
        "{out}"
    );
    assert!(out.contains("adapter: beads  scope: filesystem"), "{out}");
    // disabled integrations are not listed
    assert!(!out.contains("hindsight"), "{out}");
    // on-demand importers are always available
    assert!(out.contains("adapter: scip  scope: filesystem"), "{out}");
}

#[test]
fn lessons_add_then_import_then_list() {
    let repo = tempfile::TempDir::new().unwrap();
    let root = workdir(repo.path());
    std::fs::create_dir_all(root.join(".scc")).unwrap();
    std::fs::write(root.join("a.py"), "def helper():\n    return 1\n").unwrap();
    std::fs::write(root.join(".scc/config.yaml"), "schema: 1\nintegrations:\n  hindsight: true\n").unwrap();

    let out = run_ok(&root, &["lessons", "add", "retry with backoff works"]);
    assert!(out.contains("appended lesson-1"), "{out}");
    // second add appends
    run_ok(&root, &["lessons", "add", "always reindex before verify"]);
    let bank = std::fs::read_to_string(root.join(".scc/lessons.jsonl")).unwrap();
    let lines: Vec<&str> = bank.lines().collect();
    assert_eq!(lines.len(), 2, "{bank}");
    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["id"], "lesson-1");
    assert_eq!(first["text"], "retry with backoff works");
    assert!(first["created_at"].is_string());

    let out = run_ok(&root, &["import", "hindsight", ".scc/lessons.jsonl"]);
    assert!(out.contains("imported 2"), "{out}");
    let out = run_ok(&root, &["lessons"]);
    assert!(out.contains("retry with backoff works"), "{out}");
    assert!(out.contains("always reindex before verify"), "{out}");
}

#[test]
fn beads_command_lists_active() {
    let repo = tempfile::TempDir::new().unwrap();
    let root = workdir(repo.path());
    std::fs::create_dir_all(root.join(".beads")).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join(".beads/issues.jsonl"),
        "{\"id\":\"b1\",\"title\":\"Fix retry\",\"status\":\"in_progress\"}\n{\"id\":\"b2\",\"title\":\"Add fallback\",\"status\":\"open\"}\n",
    )
    .unwrap();
    let out = run_ok(&root, &["beads"]);
    assert!(out.contains("Fix retry"), "{out}");
    assert!(!out.contains("Add fallback"), "open beads are not active: {out}");

    // no .beads file -> clean empty listing
    let empty = tempfile::TempDir::new().unwrap();
    let empty_root = workdir(empty.path());
    std::fs::create_dir_all(&empty_root).unwrap();
    let out = run_ok(&empty_root, &["beads"]);
    assert!(out.contains("no active beads tasks"), "{out}");
}

#[test]
fn context7_unconfigured_errors_clearly() {
    let repo = tempfile::TempDir::new().unwrap();
    let root = workdir(repo.path());
    std::fs::create_dir_all(root.join(".scc")).unwrap();
    std::fs::write(root.join("a.py"), "def helper():\n    return 1\n").unwrap();
    std::fs::write(root.join(".scc/config.yaml"), "schema: 1\n").unwrap();
    run_ok(&root, &["index", "--quiet"]);
    let out = run(&root, &["context", "docs", "fastapi/fastapi"]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not configured"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
