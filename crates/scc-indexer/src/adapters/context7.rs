//! Context7 adapter (SCC-205): external library/API documentation via
//! Context7's MCP server (stdio). Task context may name a dependency; this
//! appends labeled external docs — never mixed with repository facts.
//!
//! The MCP server command is configurable; the default is the official
//! `@upstash/context7-mcp` package via npx.

use crate::lsp::{encode_frame, read_frame};
use std::io::{BufReader, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(30);

pub struct Context7Client {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::Receiver<serde_json::Value>,
    next_id: u64,
}

/// Spawn the Context7 MCP server. `command` is the full shell command.
pub fn start(command: &str) -> Result<Context7Client, String> {
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
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
        while let Ok(v) = read_frame(&mut stdout) {
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
    let _ = client.request(
        "initialize",
        &serde_json::json!({
            "protocolVersion": "2025-06-18",
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
        let frame = encode_frame(&msg);
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
    pub fn docs_for(&mut self, dependency: &str) -> Result<String, String> {
        let search = self.call_tool("library-search", &serde_json::json!({"q": dependency}))?;
        let text = extract_text(&search);
        if text.is_empty() {
            return Err(format!("context7: no results for '{dependency}'"));
        }
        let docs = self.call_tool(
            "query-docs",
            &serde_json::json!({"library": dependency, "query": dependency}),
        )?;
        let docs_text = extract_text(&docs);
        Ok(format!(
            "<!-- CONTEXT7 EXTERNAL DOCUMENTATION for {dependency} — external, not repository facts -->\n{}\n{}",
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

    const FAKE_SERVER: &str = r#"import sys, json
def frame(obj):
    s = json.dumps(obj).encode()
    return b"Content-Length: " + str(len(s)).encode() + b"\r\n\r\n" + s
buf = b""
while True:
    chunk = sys.stdin.buffer.read(1)
    if not chunk: break
    buf += chunk
    if b"\r\n\r\n" not in buf: continue
    header, _, rest = buf.partition(b"\r\n\r\n")
    length = 0
    for line in header.split(b"\r\n"):
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":")[1].strip())
    while len(rest) < length:
        rest += sys.stdin.buffer.read(length - len(rest))
    msg = json.loads(rest.decode())
    buf = b""
    method = msg.get("method")
    if method == "initialize":
        sys.stdout.buffer.write(frame({"jsonrpc":"2.0","id":msg["id"],"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fake-c7"}}}))
    elif method == "tools/list":
        sys.stdout.buffer.write(frame({"jsonrpc":"2.0","id":msg["id"],"result":{"tools":[{"name":"library-search"},{"name":"query-docs"}]}}))
    elif method == "tools/call":
        name = msg["params"]["name"]
        if name == "library-search":
            result = {"content":[{"type":"text","text":"library: fastapi/fastapi"}]}
        else:
            result = {"content":[{"type":"text","text":"docs: FastAPI route docs here"}]}
        sys.stdout.buffer.write(frame({"jsonrpc":"2.0","id":msg["id"],"result":result}))
    sys.stdout.buffer.flush()
"#;

    #[test]
    fn client_handles_fake_mcp_server() {
        let dir = tempfile::TempDir::new().unwrap();
        let fake_path = dir.path().join("fake_c7.py");
        std::fs::write(&fake_path, FAKE_SERVER).unwrap();
        let cmd = format!("python3 {}", fake_path.display());
        let mut client = start(&cmd).unwrap();
        let out = client.docs_for("fastapi/fastapi").unwrap();
        assert!(out.contains("CONTEXT7 EXTERNAL DOCUMENTATION"), "{out}");
        assert!(out.contains("FastAPI route docs"), "{out}");
    }

    #[test]
    fn missing_server_errors() {
        assert!(start("python3 /nonexistent.py").is_err());
    }

    #[test]
    fn framing_helpers_roundtrip() {
        let msg = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"});
        let frame = encode_frame(&msg);
        let mut reader = std::io::Cursor::new(frame);
        let parsed = read_frame(&mut reader).unwrap();
        assert_eq!(parsed["id"], 1);
    }
}
