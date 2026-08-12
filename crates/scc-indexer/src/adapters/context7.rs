//! Context7 adapter (SCC-205): external library/API documentation via
//! Context7's MCP server (stdio). Task context may name a dependency; this
//! appends labeled external docs — never mixed with repository facts.
//!
//! The MCP server command is configurable; the default is the official
//! `@upstash/context7-mcp` package via npx.

use crate::lsp::encode_frame;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(30);

/// Read one MCP message tolerating both real-world stdio transports
/// (P0 §16, verified live against @upstash/context7-mcp v4.0.2):
///
/// - JSONL: each message is one JSON line (`\n`-terminated, no headers);
///   this is what the actual Context7 server emits.
/// - Content-Length framing: header lines then a fixed-size body; this is
///   what LSP-style servers and SCC's fake test server emit.
fn read_message<R: BufRead>(reader: &mut R) -> Result<serde_json::Value, String> {
    let mut first = String::new();
    if reader.read_line(&mut first).map_err(|e| format!("read: {e}"))? == 0 {
        return Err("server closed stdout".to_string());
    }
    let first = first.trim_end_matches(['\r', '\n']);
    if first.to_ascii_lowercase().starts_with("content-length:") {
        // Content-Length framing: consume the remaining headers, then body.
        let mut content_length: Option<usize> = None;
        let mut parse_header = |line: &str| {
            if let Some((k, v)) = line.split_once(':') {
                if k.trim().eq_ignore_ascii_case("content-length") {
                    content_length = v.trim().parse().ok();
                }
            }
        };
        parse_header(first);
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).map_err(|e| format!("read: {e}"))? == 0 {
                return Err("server closed mid-headers".to_string());
            }
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                break;
            }
            parse_header(line);
        }
        let n = content_length.ok_or("content-length missing")?;
        let mut body = vec![0u8; n];
        reader
            .read_exact(&mut body)
            .map_err(|e| format!("read body: {e}"))?;
        serde_json::from_slice(&body).map_err(|e| format!("bad JSON frame: {e}"))
    } else {
        // JSONL: the first line is the whole message.
        serde_json::from_str(first).map_err(|e| format!("bad JSONL frame: {e}"))
    }
}

pub struct Context7Client {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::Receiver<serde_json::Value>,
    next_id: u64,
}

/// Spawn the Context7 MCP server. `command` is the full shell command;
/// `cwd` is the repository root the server runs in. The child runs under
/// the sandboxed environment (SCC-225): only PATH/HOME/TMPDIR/LANG/LC_ALL
/// and SCC_* are inherited — never arbitrary parent env (no API keys).
pub fn start(command: &str, cwd: &Path) -> Result<Context7Client, String> {
    let mut cmd = super::sandboxed_command(command, cwd);
    // stderr stays /dev/null on purpose: the MCP protocol is stdout-only and
    // the child is a sandboxed subprocess — its diagnostics must not
    // interleave the JSONL stream (a real server crash surfaces as a
    // response timeout instead of corrupted frames).
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("context7 spawn: {e}"))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "context7 stdin unavailable".to_string())?;
    let stdout = BufReader::new(
        child
            .stdout
            .take()
            .ok_or_else(|| "context7 stdout unavailable".to_string())?,
    );
    // persistent reader thread: every message (responses and notifications)
    // goes to the channel; recv() scans for the expected id.
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut stdout = stdout;
        while let Ok(v) = read_message(&mut stdout) {
            if tx.send(v).is_err() {
                break;
            }
        }
    });

    let mut client = Context7Client {
        child,
        stdin,
        rx,
        next_id: 1,
    };
    // Protocol compatibility (P0 §16): the real Context7 server (v4.x)
    // answers `2024-11-05` but silently ignores `2025-06-18`; verified live
    // against @upstash/context7-mcp. Never guess the version — the live
    // suite (tests/context7_live.rs) pins this behavior.
    let _ = client.request(
        "initialize",
        &serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "scc", "version": env!("CARGO_PKG_VERSION")}
        }),
    )?;
    client.notify("notifications/initialized", &serde_json::json!({}))?;
    Ok(client)
}

impl Context7Client {
    fn send(
        &mut self,
        method: &str,
        params: &serde_json::Value,
        id: Option<u64>,
    ) -> std::io::Result<()> {
        let mut msg = serde_json::json!({"jsonrpc": "2.0", "method": method, "params": params});
        if let Some(id) = id {
            msg["id"] = serde_json::json!(id);
        }
        let mut frame = encode_frame(&msg);
        // Real-server compatibility (P0 §16, verified against
        // @upstash/context7-mcp v4.0.2): its stdio transport reads the
        // header via readline and requires the body to be line-terminated;
        // without the trailing newline the server never sees the message.
        frame.push(b'\n');
        self.stdin.write_all(&frame)?;
        self.stdin.flush()?;
        Ok(())
    }

    fn recv(&mut self, expected: u64) -> Result<serde_json::Value, String> {
        let deadline = std::time::Instant::now() + TIMEOUT;
        loop {
            let remaining = deadline
                .saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err("context7 response timeout".to_string());
            }
            match self.rx.recv_timeout(remaining) {
                Ok(v) => {
                    if let Some(id) = v.get("id").and_then(|i| i.as_u64()) {
                        if id != expected {
                            continue; // stale response — ignore
                        }
                        if let Some(err) = v.get("error") {
                            return Err(format!("context7: {err}"));
                        }
                        return Ok(v
                            .get("result")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null));
                    }
                    // notification — ignore
                }
                Err(_) => return Err("context7 response timeout".to_string()),
            }
        }
    }

    fn request(
        &mut self,
        method: &str,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(method, params, Some(id))
            .map_err(|e| format!("context7 write: {e}"))?;
        self.recv(id)
    }

    fn notify(&mut self, method: &str, params: &serde_json::Value) -> Result<(), String> {
        self.send(method, params, None)
            .map_err(|e| format!("context7 write: {e}"))
    }

    fn call_tool(
        &mut self,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.request(
            "tools/call",
            &serde_json::json!({"name": name, "arguments": args}),
        )
    }

    /// Look up a library by `owner/name` and fetch its docs. Returns labeled
    /// markdown (external documentation — never repository facts).
    ///
    /// Real-server protocol (P0 §16, verified against @upstash/context7-mcp
    /// v4.0.2): `resolve-library-id` with `{libraryName, query}` returns a
    /// text list of libraries; the first `Context7-compatible library ID`
    /// feeds `query-docs` with `{libraryId, query}`.
    pub fn docs_for(&mut self, dependency: &str) -> Result<String, String> {
        let resolved = self.call_tool(
            "resolve-library-id",
            &serde_json::json!({"libraryName": dependency, "query": dependency}),
        )?;
        let text = extract_text(&resolved);
        if text.is_empty() {
            return Err(format!("context7: no results for '{dependency}'"));
        }
        let library_id = text
            .lines()
            .find_map(|l| {
                let t = l.trim().trim_start_matches("- ").trim();
                t.strip_prefix("Context7-compatible library ID:")
                    .map(|s| s.trim().to_string())
            })
            .ok_or_else(|| {
                format!("context7: resolve-library-id returned no library id: {text}")
            })?;
        let docs = self.call_tool(
            "query-docs",
            &serde_json::json!({"libraryId": library_id, "query": dependency}),
        )?;
        let docs_text = extract_text(&docs);
        Ok(format!(
            "<!-- CONTEXT7 EXTERNAL DOCUMENTATION for {dependency} ({library_id}) — external, not repository facts -->\n{}\n{}",
            text, docs_text
        ))
    }
}

fn extract_text(v: &serde_json::Value) -> String {
    v.get("content")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_else(|| v.to_string())
}

impl Drop for Context7Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fake server speaking the REAL Context7 v4 protocol (JSONL transport,
    // resolve-library-id + query-docs tools) so the unit test pins the same
    // contract the live suite (tests/context7_live.rs) verifies.
    const FAKE_SERVER: &str = r#"import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line.startswith("{"):
        continue  # tolerate Content-Length header lines, like the real server
    msg = json.loads(line)
    method = msg.get("method")
    if method == "initialize":
        out = {"jsonrpc":"2.0","id":msg["id"],"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"fake-c7"}}}
    elif method == "tools/list":
        out = {"jsonrpc":"2.0","id":msg["id"],"result":{"tools":[{"name":"resolve-library-id"},{"name":"query-docs"}]}}
    elif method == "tools/call":
        name = msg["params"]["name"]
        if name == "resolve-library-id":
            args = msg["params"].get("arguments", {})
            assert "libraryName" in args and "query" in args
            result = {"content":[{"type":"text","text":"- Context7-compatible library ID: /fastapi/fastapi\n- Description: FastAPI"}]}
        else:
            args = msg["params"].get("arguments", {})
            assert args.get("libraryId") == "/fastapi/fastapi"
            result = {"content":[{"type":"text","text":"docs: FastAPI route docs here"}]}
        out = {"jsonrpc":"2.0","id":msg["id"],"result":result}
    else:
        continue
    sys.stdout.write(json.dumps(out) + "\n")
    sys.stdout.flush()
"#;

    #[test]
    fn client_handles_fake_mcp_server() {
        let dir = tempfile::TempDir::new().unwrap();
        let fake_path = dir.path().join("fake_c7.py");
        std::fs::write(&fake_path, FAKE_SERVER).unwrap();
        let cmd = format!("python3 {}", fake_path.display());
        let mut client = start(&cmd, dir.path()).unwrap();
        let out = client.docs_for("fastapi/fastapi").unwrap();
        assert!(out.contains("CONTEXT7 EXTERNAL DOCUMENTATION"), "{out}");
        assert!(out.contains("/fastapi/fastapi"), "{out}");
        assert!(out.contains("FastAPI route docs"), "{out}");
    }

    #[test]
    fn client_speaks_jsonl_transport() {
        // the real Context7 server emits JSONL (no Content-Length headers);
        // the client must tolerate both transports
        let dir = tempfile::TempDir::new().unwrap();
        let fake_path = dir.path().join("fake_c7_jsonl.py");
        std::fs::write(&fake_path, FAKE_SERVER).unwrap();
        let cmd = format!("python3 {}", fake_path.display());
        let mut client = start(&cmd, dir.path()).unwrap();
        let out = client.docs_for("fastapi/fastapi").unwrap();
        assert!(out.contains("FastAPI route docs"), "{out}");
    }

    #[test]
    fn missing_server_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(start("python3 /nonexistent.py", dir.path()).is_err());
    }

    // SCC-225 sandbox enforcement: the child subprocess must inherit ONLY
    // the allowlisted variables (PATH/HOME/TMPDIR/LANG/LC_ALL/SCC_*) — a
    // secret in the parent environment must never reach it.
    #[test]
    fn sandboxed_command_strips_inherited_secrets() {
        // A secret sitting in the parent environment...
        std::env::set_var("SECRET_TOKEN", "scc-test-hunter2");
        let dir = tempfile::TempDir::new().unwrap();
        let fake = dir.path().join("dump_env.py");
        std::fs::write(
            &fake,
            "import json, os, sys\nsys.stdout.write(json.dumps(sorted(os.environ.keys())) + \"\\n\")\n",
        )
        .unwrap();
        let cmd = format!("python3 {}", fake.display());
        let output = super::super::sandboxed_command(&cmd, dir.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "fake env dump failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let keys: Vec<String> =
            serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap().trim()).unwrap();
        assert!(
            keys.iter().all(|k| !k.contains("SECRET")),
            "secret leaked into the child environment: {keys:?}"
        );
        assert!(
            keys.iter().any(|k| k == "PATH"),
            "PATH must be preserved for npx/python3: {keys:?}"
        );
        assert!(
            keys.iter().any(|k| k == "HOME"),
            "HOME must be preserved for the npx cache: {keys:?}"
        );
        // allowlist only — nothing else from the parent may leak through.
        // The shell itself injects PWD/SHLVL/_ (and __CF_USER_TEXT_ENCODING
        // on macOS) after exec; those are not parent inheritance.
        let shell_init = ["PWD", "SHLVL", "_", "__CF_USER_TEXT_ENCODING"];
        let leaked: Vec<&String> = keys
            .iter()
            .filter(|k| {
                !matches!(k.as_str(), "PATH" | "HOME" | "TMPDIR" | "LANG" | "LC_ALL")
                    && !k.starts_with("SCC_")
                    && !shell_init.contains(&k.as_str())
            })
            .collect();
        assert!(
            leaked.is_empty(),
            "non-allowlisted variable reached the child: {leaked:?}"
        );
    }

    #[test]
    fn framing_helpers_roundtrip() {
        let msg = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"});
        // Content-Length framing parses
        let frame = encode_frame(&msg);
        let mut reader = std::io::Cursor::new(frame);
        let parsed = read_message(&mut reader).unwrap();
        assert_eq!(parsed["id"], 1);
        // JSONL framing parses
        let mut reader = std::io::Cursor::new(format!("{}\n", msg).into_bytes());
        let parsed = read_message(&mut reader).unwrap();
        assert_eq!(parsed["id"], 1);
    }
}
