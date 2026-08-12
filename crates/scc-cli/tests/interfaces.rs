//! Export validation against `docs/system-ir.schema.json` and MCP end-to-end
//! tests (docs/TEST_PLAN.md §14, docs/API_AND_INTEGRATIONS.md §1).

mod golden;
use golden::*;
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn export_validates_against_json_schema() {
    let repo = copy_fixture("monorepo-acceptance");
    std::fs::create_dir_all(workdir(repo.path()).join(".scc")).unwrap();
    std::fs::write(
        workdir(repo.path()).join(".scc/intent.yaml"),
        "components:\n  api:\n    owns: [transcript]\n",
    )
    .unwrap();
    run_ok(&workdir(repo.path()), &["index", "--quiet"]);
    let ir = run_ok(&workdir(repo.path()), &["export", "system-ir.json"]);
    let value: serde_json::Value = serde_json::from_str(&ir).unwrap();

    let schema_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("docs/system-ir.schema.json");
    let schema_text = std::fs::read_to_string(&schema_path).unwrap();
    let schema: serde_json::Value = serde_json::from_str(&schema_text).unwrap();
    let compiled = jsonschema::validator_for(&schema).expect("schema compiles");
    let result = compiled.validate(&value);
    assert!(
        result.is_ok(),
        "System IR export violates the documented schema: {:?}",
        result.err().map(|e| e.to_string())
    );
}

#[test]
fn mcp_server_exposes_six_semantic_tools() {
    let repo = copy_fixture("http-service-python");
    run_ok(&workdir(repo.path()), &["index", "--quiet"]);

    let mut child = Command::new(scc())
        .arg("mcp")
        .current_dir(workdir(repo.path()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let send = |stdin: &mut std::process::ChildStdin, msg: &str| {
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
    };

    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test"}}}"#,
    );
    send(&mut stdin, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"system_overview","arguments":{}}}"#,
    );
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"task_context","arguments":{"goal":"transcript normalization"}}}"#,
    );
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"impact_context","arguments":{"files":["main.py"]}}}"#,
    );
    drop(stdin);

    let mut stdout = String::new();
    use std::io::Read;
    child.stdout.take().unwrap().read_to_string(&mut stdout).unwrap();
    let _ = child.wait();

    let mut by_id: std::collections::BTreeMap<i64, serde_json::Value> = Default::default();
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        if let Some(id) = v.get("id").and_then(|i| i.as_i64()) {
            by_id.insert(id, v);
        }
    }
    assert_eq!(by_id.len(), 5, "all requests answered: {stdout}");

    let tools = &by_id[&2]["result"]["tools"];
    let names: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert_eq!(
        names,
        vec![
            "system_overview",
            "system_atlas",
            "task_context",
            "component_context",
            "flow_context",
            "impact_context",
            "verify_context"
        ],
        "the seven semantic tools"
    );
    let overview_text = by_id[&3]["result"]["content"][0]["text"].as_str().unwrap();
    assert!(overview_text.contains("IDENTITY"));
    let task_text = by_id[&4]["result"]["content"][0]["text"].as_str().unwrap();
    assert!(task_text.contains("TASK"));
    assert!(task_text.contains("Normalizer"));
    let impact_text = by_id[&5]["result"]["content"][0]["text"].as_str().unwrap();
    assert!(impact_text.contains("AFFECTED COMPONENTS"));
}

#[test]
fn mcp_unknown_tool_returns_error() {
    let repo = copy_fixture("http-service-python");
    run_ok(&workdir(repo.path()), &["index", "--quiet"]);
    let mut child = Command::new(scc())
        .arg("mcp")
        .current_dir(workdir(repo.path()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"query_graph","arguments":{{}}}}}}"#
        )
        .unwrap();
        stdin.flush().unwrap();
    }
    let mut stdout = String::new();
    use std::io::Read;
    child.stdout.take().unwrap().read_to_string(&mut stdout).unwrap();
    let _ = child.wait();
    let v: serde_json::Value = serde_json::from_str(stdout.lines().next().unwrap()).unwrap();
    assert_eq!(v["result"]["isError"], true, "advanced tools are not exposed");
}

#[test]
fn context_parity_across_cli_http_mcp() {
    // P0 §10: transport must not change semantic quality — the same
    // task_context request yields the same pack content on CLI, HTTP, MCP,
    // WITH embeddings + beads + hindsight ALL enabled (the transports must
    // agree on the enriched pack, not just the default-config pack).
    let repo = copy_fixture("http-service-python");
    let dir = workdir(repo.path());
    // beads task state + hindsight lessons + inference path (loopback
    // endpoint that fails closed to lexical ranking — parity must hold
    // through the same ranker decision on every transport). The config is
    // written FIRST so every transport sees hindsight enabled.
    let port = 20000 + (std::process::id() % 20000) as u16;
    let addr = format!("127.0.0.1:{port}");
    std::fs::create_dir_all(dir.join(".scc")).unwrap();
    std::fs::create_dir_all(dir.join(".beads")).unwrap();
    std::fs::write(
        dir.join(".scc/config.yaml"),
        format!(
            "schema: 1\nindex:\n  watch: false\ninference:\n  enabled: true\n  provider: local\n  base_url: http://127.0.0.1:1\nintegrations:\n  hindsight: true\nsecurity:\n  listen: {addr}\n"
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join(".beads/issues.jsonl"),
        "{\"id\":\"b1\",\"title\":\"Fix normalization retry\",\"status\":\"in_progress\",\"dependencies\":[]}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join(".scc/lessons.jsonl"),
        "{\"id\":\"l1\",\"text\":\"raw transcripts are immutable\",\"created_at\":\"2026-01-01T00:00:00Z\"}\n",
    )
    .unwrap();
    run_ok(
        &dir,
        &["import", "hindsight", ".scc/lessons.jsonl"],
    );
    run_ok(&dir, &["index", "--quiet"]);
    let goal = "change transcript normalization";

    // CLI (JSON pack)
    let cli_json = run_ok(&dir, &["context", "task", "--json", goal]);
    let cli: serde_json::Value = serde_json::from_str(&cli_json).unwrap();
    let cli_content = cli["content"].as_str().unwrap().to_string();
    assert!(
        cli_content.contains("ACTIVE TASK STATE"),
        "beads enrichment present on CLI: {cli_content}"
    );
    assert!(
        cli_content.contains("HINDSIGHT LESSONS"),
        "hindsight enrichment present on CLI: {cli_content}"
    );
    let mut child = Command::new(scc())
        .arg("serve")
        .current_dir(&dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut ready = false;
    for _ in 0..40 {
        if std::net::TcpStream::connect(addr.as_str()).is_ok() {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(ready, "daemon did not start");
    let post = |path: &str, body: &str| -> (u16, String) {
        let mut stream = std::net::TcpStream::connect(addr.as_str()).unwrap();
        write!(
            stream,
            "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut buf = String::new();
        use std::io::Read;
        stream.read_to_string(&mut buf).unwrap();
        let status: u16 = buf.split_whitespace().nth(1).unwrap().parse().unwrap();
        let body = buf.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        (status, body)
    };
    let (s, http_body) = post(
        "/v1/context/task",
        &format!(r#"{{"goal":"{goal}"}}"#),
    );
    assert_eq!(s, 200);
    let http: serde_json::Value = serde_json::from_str(&http_body).unwrap();
    let http_content = http["content"].as_str().unwrap().to_string();
    child.kill().unwrap();
    child.wait().unwrap();

    // MCP server
    let mut child = Command::new(scc())
        .arg("mcp")
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let send = |stdin: &mut std::process::ChildStdin, msg: &str| {
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
    };
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"parity"}}}"#,
    );
    send(
        &mut stdin,
        &format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"task_context","arguments":{{"goal":"{goal}"}}}}}}"#
        ),
    );
    drop(stdin);
    let mut stdout = String::new();
    use std::io::Read;
    child.stdout.take().unwrap().read_to_string(&mut stdout).unwrap();
    let _ = child.wait();
    let mut mcp_content = String::new();
    for line in stdout.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        if v.get("id").and_then(|i| i.as_i64()) == Some(2) {
            mcp_content = v["result"]["content"][0]["text"].as_str().unwrap().to_string();
        }
    }
    assert!(!mcp_content.is_empty(), "MCP answered: {stdout}");

    // parity: identical content on every transport
    assert_eq!(cli_content, http_content, "CLI and HTTP packs differ");
    assert_eq!(cli_content, mcp_content, "CLI and MCP packs differ");
}

#[test]
fn http_daemon_endpoints() {
    let repo = copy_fixture("http-service-python");
    run_ok(&workdir(repo.path()), &["index", "--quiet"]);
    // override the listen address via config for this repo; the port is
    // derived from the process id so parallel/leaked daemons cannot collide
    let port = 20000 + (std::process::id() % 20000) as u16;
    let addr = format!("127.0.0.1:{port}");
    std::fs::write(
        workdir(repo.path()).join(".scc/config.yaml"),
        format!("schema: 1\nindex:\n  watch: false\nsecurity:\n  listen: {addr}\n"),
    )
    .unwrap();

    let mut child = Command::new(scc())
        .arg("serve")
        .current_dir(workdir(repo.path()))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    // poll until the port accepts connections
    let mut ready = false;
    for _ in 0..40 {
        if std::net::TcpStream::connect(addr.as_str()).is_ok() {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(ready, "daemon did not start");

    let get = |path: &str| -> (u16, String) {
        let mut stream = std::net::TcpStream::connect(addr.as_str()).unwrap();
        write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").unwrap();
        let mut buf = String::new();
        use std::io::Read;
        stream.read_to_string(&mut buf).unwrap();
        let status: u16 = buf
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let body = buf.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        (status, body)
    };
    let post = |path: &str, body: &str| -> (u16, String) {
        let mut stream = std::net::TcpStream::connect(addr.as_str()).unwrap();
        write!(
            stream,
            "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut buf = String::new();
        use std::io::Read;
        stream.read_to_string(&mut buf).unwrap();
        let status: u16 = buf.split_whitespace().nth(1).unwrap().parse().unwrap();
        let body = buf.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        (status, body)
    };

    let (s, _) = get("/healthz");
    assert_eq!(s, 200);
    let (s, body) = get("/v1/index/status");
    assert_eq!(s, 200);
    assert!(body.contains("\"indexed\":true"), "{body}");
    let (s, body) = get("/v1/system");
    assert_eq!(s, 200);
    assert!(body.contains("IDENTITY"), "{body}");
    let (s, body) = post("/v1/context/task", r#"{"goal":"transcript normalization"}"#);
    assert_eq!(s, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["kind"], "task");
    let (s, _body) = post("/v1/runtime/traces", r#"[{"source":"a","target":"b","count":3}]"#);
    assert_eq!(s, 202);
    let (s, body) = post("/v1/impact", r#"{"files":["main.py"]}"#);
    assert_eq!(s, 200);
    assert!(body.contains("AFFECTED COMPONENTS"), "{body}");
    let (s, body) = post("/v1/verify", "{}");
    assert_eq!(s, 200);
    assert!(body.contains("FRESHNESS"), "{body}");
    let (s, _) = get("/v1/nope");
    assert_eq!(s, 404);

    child.kill().unwrap();
    child.wait().unwrap();
}
