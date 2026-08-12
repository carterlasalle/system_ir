//! LSP-based definition resolution for TypeScript/JavaScript (SCC-121):
//! upgrade EXTRACTED candidate call edges to RESOLVED using
//! `typescript-language-server` speaking LSP 3.17 over stdio with JSON-RPC
//! 2.0 Content-Length framing.
//!
//! Same contract as `crate::lsp` (the pyright adapter): the native resolver
//! produces `calls` relationships with provenance `EXTRACTED` when an import
//! root looks external (e.g. a tsconfig `paths` alias the native module
//! resolver cannot see, or a member re-exported through a barrel file).
//! tsserver performs real binding analysis and resolves the same call site
//! to the concrete definition — the adapter replaces the EXTRACTED edge with
//! a RESOLVED edge to the true target symbol, carrying fresh `lsp-tsserver`
//! evidence.
//!
//! Protocol notes (verified against typescript-language-server 5.3.0 +
//! typescript 7.0.2 on macOS):
//! - tsserver is launched with `--stdio` only. The `--cancellationReceive`
//!   flag is pyright-specific and MUST NOT be passed here.
//! - The initialize/initialized handshake is identical to the pyright
//!   adapter; the reader thread forwards responses, auto-replies to
//!   server-initiated requests (workspace/configuration etc.), and skips
//!   notifications (logMessage, publishDiagnostics, $/progress).
//! - Definition responses are LocationLink arrays; the adapter accepts
//!   Location | LocationLink | array | null.
//! - The call-site position must sit on the callee name token (character 0 —
//!   leading indentation — resolves to nothing), so the adapter locates the
//!   callee text inside the source line.
#![allow(clippy::too_many_arguments)]

use crate::lsp::{uri_to_path, LspResult, REQUEST_TIMEOUT};
use crate::write::{evidence_id, rel_id};
use scc_core::{Evidence, EvidenceType, Provenance, Relationship};
use scc_store::Store;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Evidence extractor name stamped on upgraded edges.
pub const LSP_EXTRACTOR: &str = "lsp-tsserver";

enum ServerMsg {
    /// A response to one of our requests (`id` + body).
    Response(Value),
    /// A server-initiated request that must be answered (`id` + body).
    Request(Value),
    /// The input stream ended or became unreadable.
    Eof(String),
}

/// Result of a single call-site definition query.
enum DefOutcome {
    Resolved(String, String),
    Empty,
    Error,
}

/// Classified response of one definition request.
enum QueryOutcome {
    Target(Target),
    Empty,
    Error,
}

/// JSON-RPC/LSP client driving one typescript-language-server process.
pub struct TsLspResolver {
    child: Option<Child>,
    writer: Box<dyn Write + Send>,
    rx: Receiver<ServerMsg>,
    pending: HashMap<u64, Value>,
    next_id: u64,
    /// Absolute workspace root (cwd of the server, `file://` prefix base).
    workspace: PathBuf,
    ts_version: String,
    opened: HashSet<String>,
    stderr_join: Option<JoinHandle<()>>,
    /// Cold-start budget: retries left while the configured TS project may
    /// still be loading (see [`Self::definition_for`]).
    cold_retries: u32,
}

impl Drop for TsLspResolver {
    fn drop(&mut self) {
        // Kill the child; the reader thread then observes EOF and exits.
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// ---------------------------------------------------------------------------
// server lifecycle
// ---------------------------------------------------------------------------

/// Version of the installed `typescript-language-server` (`X.Y.Z`), or `None`
/// when the binary is absent.
pub fn tsserver_version() -> Option<String> {
    if let Ok(out) = Command::new("typescript-language-server").arg("--version").output() {
        if out.status.success() {
            return parse_version(&String::from_utf8_lossy(&out.stdout));
        }
    }
    None
}

fn parse_version(text: &str) -> Option<String> {
    // "5.3.0" / "typescript-language-server 5.3.0"
    for tok in text.split_whitespace() {
        if tok.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return Some(tok.to_string());
        }
    }
    None
}

/// Spawn typescript-language-server (cwd = `workspace_root`) and complete
/// the initialize handshake. Returns `Err` when tsserver is missing or the
/// handshake fails.
pub fn start_tsserver(workspace_root: &Path) -> Result<TsLspResolver, String> {
    let version = tsserver_version().ok_or_else(|| {
        "tsserver not found — install with: npm install -g typescript-language-server typescript"
            .to_string()
    })?;
    let workspace = std::fs::canonicalize(workspace_root)
        .unwrap_or_else(|_| workspace_root.to_path_buf());

    let mut cmd = Command::new("typescript-language-server");
    cmd.arg("--stdio")
        .current_dir(&workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("cannot spawn typescript-language-server ({e})"))?;

    let stdin = child.stdin.take().ok_or("tsserver stdin unavailable")?;
    let stdout = child.stdout.take().ok_or("tsserver stdout unavailable")?;
    let mut stderr = child.stderr.take().ok_or("tsserver stderr unavailable")?;

    // Drain stderr so tsserver never blocks on a full log pipe.
    let stderr_join = std::thread::spawn(move || {
        let _ = std::io::copy(&mut stderr, &mut std::io::sink());
    });

    let mut resolver = TsLspResolver::with_io(
        &workspace,
        version,
        Box::new(stdout),
        Box::new(stdin),
        Some(child),
    )?;
    resolver.stderr_join = Some(stderr_join);
    Ok(resolver)
}

impl TsLspResolver {
    /// Build a resolver over arbitrary reader/writer transports and run the
    /// initialize handshake. `child` is optional (killed on drop).
    fn with_io(
        workspace: &Path,
        version: String,
        reader: Box<dyn Read + Send>,
        writer: Box<dyn Write + Send>,
        child: Option<Child>,
    ) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(reader);
            loop {
                match crate::lsp::read_frame(&mut reader) {
                    Ok(v) => {
                        let id = v.get("id").cloned();
                        if v.get("method").is_some() {
                            if id.is_some() {
                                let _ = tx.send(ServerMsg::Request(v));
                            }
                        } else if id.is_some() {
                            let _ = tx.send(ServerMsg::Response(v));
                        }
                        // notifications and malformed frames are skipped
                    }
                    Err(e) => {
                        let _ = tx.send(ServerMsg::Eof(e));
                        break;
                    }
                }
            }
        });
        let mut resolver = TsLspResolver {
            child,
            writer: Box::new(writer),
            rx,
            pending: HashMap::new(),
            next_id: 1,
            workspace: workspace.to_path_buf(),
            ts_version: version,
            opened: HashSet::new(),
            stderr_join: None,
            cold_retries: 5,
        };
        resolver.initialize()?;
        Ok(resolver)
    }

    /// The resolved typescript-language-server version string (evidence
    /// `extractor_version`).
    pub fn tsserver_version(&self) -> &str {
        &self.ts_version
    }

    fn send(&mut self, msg: &Value) -> Result<(), String> {
        self.writer
            .write_all(&crate::lsp::encode_frame(msg))
            .map_err(|e| format!("write to LSP server: {e}"))?;
        self.writer
            .flush()
            .map_err(|e| format!("flush LSP server: {e}"))
    }

    /// Send a request and wait (up to [`REQUEST_TIMEOUT`]) for its response.
    /// Server-initiated requests are answered with an empty result while
    /// waiting; out-of-order responses are buffered in `pending`.
    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))?;
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        loop {
            if let Some(v) = self.pending.remove(&id) {
                return Ok(v);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!("timed out waiting for LSP response to {method}"));
            }
            match self.rx.recv_timeout(remaining) {
                Ok(ServerMsg::Response(v)) => {
                    if let Some(rid) = v.get("id").and_then(Value::as_u64) {
                        if rid != id {
                            self.pending.insert(rid, v);
                        } else {
                            return Ok(v);
                        }
                    }
                }
                Ok(ServerMsg::Request(v)) => {
                    // workspace/configuration, client/registerCapability etc.
                    // tsserver treats a null result as "no configuration".
                    if let Some(rid) = v.get("id") {
                        let _ = self.send(&json!({"jsonrpc": "2.0", "id": rid, "result": null}));
                    }
                }
                Ok(ServerMsg::Eof(e)) => {
                    return Err(format!("LSP server exited: {e}"));
                }
                Err(_) => {
                    return Err(format!("timed out waiting for LSP response to {method}"));
                }
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.send(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
    }

    fn initialize(&mut self) -> Result<(), String> {
        let root_uri = format!("file://{}", self.workspace.display());
        let resp = self.request(
            "initialize",
            json!({
                "processId": null,
                "rootUri": root_uri,
                "rootPath": self.workspace.to_string_lossy(),
                "workspaceFolders": [{"uri": root_uri, "name": "scc"}],
                "capabilities": {"workspace": {"workspaceFolders": true}},
            }),
        )?;
        if let Some(err) = resp.get("error") {
            return Err(format!("LSP initialize error: {err}"));
        }
        self.notify("initialized", json!({}))?;
        Ok(())
    }

    fn file_uri(&self, file: &str) -> String {
        format!("file://{}/{}", self.workspace.display(), file)
    }

    /// Ensure the file is open in the server (idempotent per resolver).
    fn open_file(&mut self, store: &Store, file: &str) {
        if self.opened.contains(file) {
            return;
        }
        if let Ok(content) = std::fs::read_to_string(store.root.join(file)) {
            let _ = self.notify(
                "textDocument/didOpen",
                json!({
                    "textDocument": {
                        "uri": self.file_uri(file),
                        "languageId": "typescript",
                        "version": 1,
                        "text": content,
                    }
                }),
            );
        }
        self.opened.insert(file.to_string());
    }

    /// Locate the callee name token inside `line_text`; falls back to 0.
    fn callee_char(line_text: &str, callee: &str) -> usize {
        let needle = callee.rsplit('.').next().unwrap_or(callee);
        if !needle.is_empty() {
            if let Some(idx) = line_text.rfind(needle) {
                return idx;
            }
        }
        0
    }

    /// Query one call site and return the resolved target path + symbol name.
    /// `DefOutcome::Error` marks an LSP error response (counted as `errors`);
    /// `DefOutcome::Empty` marks null/degenerate/out-of-workspace results.
    ///
    /// tsserver answers definition requests from the *inferred* project while
    /// the configured project (tsconfig) is still loading, which makes
    /// tsconfig `paths` aliases and barrel re-exports resolve to the local
    /// import binding instead of the declaration. The adapter therefore
    /// retries empty results a few times at session start (cold-start budget),
    /// and additionally follows binding sites (see [`Self::definition_once`]).
    fn definition_for(
        &mut self,
        store: &Store,
        file: &str,
        line: u32,
        callee: &str,
    ) -> Result<DefOutcome, String> {
        let mut outcome = self.definition_once(store, file, line, callee)?;
        while matches!(outcome, DefOutcome::Empty) && self.cold_retries > 0 {
            self.cold_retries -= 1;
            std::thread::sleep(Duration::from_millis(400));
            outcome = self.definition_once(store, file, line, callee)?;
        }
        // A cross-file resolution proves the configured project is warm;
        // stop spending the cold-start budget. Same-file resolutions (local
        // calls) can succeed in the inferred project, so they don't count.
        if matches!(&outcome, DefOutcome::Resolved(path, _) if path != file) {
            self.cold_retries = 0;
        }
        Ok(outcome)
    }

    /// One definition attempt: candidate positions plus import/export
    /// binding hops. tsserver resolves call-site references to *imported*
    /// symbols to the local import binding (the `import { x } ...` clause)
    /// rather than the declaration when the module comes through a tsconfig
    /// `paths` alias or a barrel re-export. When a definition lands on an
    /// import/export line, the adapter re-queries at that token (up to two
    /// hops) until the chain reaches a declaration.
    fn definition_once(
        &mut self,
        store: &Store,
        file: &str,
        line: u32,
        callee: &str,
    ) -> Result<DefOutcome, String> {
        self.open_file(store, file);
        let line_text = std::fs::read_to_string(store.root.join(file))
            .ok()
            .map(|s| {
                s.lines()
                    .nth(line.saturating_sub(1) as usize)
                    .unwrap_or_default()
                    .to_string()
            })
            .unwrap_or_default();
        let char = Self::callee_char(&line_text, callee);

        // Primary candidate: the LSP-spec 0-based position (evidence lines are
        // 1-based). The same-line character-0 fallback covers servers that
        // fail to honor the callee token column. A next-line fallback is
        // deliberately NOT included: it lands on unrelated symbols (a
        // definition on the following line) and fabricates bogus upgrades.
        let mut candidates = Vec::new();
        let push = |v: &mut Vec<(u32, usize)>, l: u32, c: usize| {
            if !v.contains(&(l, c)) {
                v.push((l, c));
            }
        };
        push(&mut candidates, line.saturating_sub(1), char);
        push(&mut candidates, line.saturating_sub(1), 0);

        for (qline, qchar) in candidates {
            let mut target = match self.query_def(file, qline, qchar)? {
                QueryOutcome::Target(t) => t,
                QueryOutcome::Error => return Ok(DefOutcome::Error),
                QueryOutcome::Empty => continue,
            };
            // Follow import/export binding sites to the real declaration.
            for _hop in 0..2 {
                if !self.is_binding_site(&target) {
                    break;
                }
                match self.query_def(file, target.line, target.char)? {
                    QueryOutcome::Target(t)
                        if t.uri != target.uri || t.line != target.line =>
                    {
                        target = t;
                    }
                    QueryOutcome::Error => return Ok(DefOutcome::Error),
                    _ => break,
                }
            }
            let Some(rel_path) = self.repo_relative(&target.uri) else {
                continue; // outside the workspace
            };
            let target_line = target.line;
            let Some(symbol) = find_containing_symbol(store, &rel_path, target_line) else {
                continue;
            };
            return Ok(DefOutcome::Resolved(rel_path, symbol));
        }
        Ok(DefOutcome::Empty)
    }

    /// Send one definition request and classify the response.
    fn query_def(&mut self, file: &str, qline: u32, qchar: usize) -> Result<QueryOutcome, String> {
        let resp = self.request(
            "textDocument/definition",
            json!({
                "textDocument": {"uri": self.file_uri(file)},
                "position": {"line": qline, "character": qchar},
            }),
        )?;
        if resp.get("error").is_some() {
            return Ok(QueryOutcome::Error);
        }
        Ok(parse_definition_target(&resp["result"], qline, &self.file_uri(file))
            .map(QueryOutcome::Target)
            .unwrap_or(QueryOutcome::Empty))
    }

    /// Whether a definition target sits on an import/export statement — i.e.
    /// it is a binding/barrel site rather than a declaration.
    fn is_binding_site(&self, target: &Target) -> bool {
        let Some(abs) = uri_to_path(&target.uri) else {
            return false;
        };
        let text = std::fs::read_to_string(&abs).unwrap_or_default();
        let line = text.lines().nth(target.line as usize).unwrap_or_default();
        line.contains("import") || line.contains("export")
    }

    fn repo_relative(&self, uri: &str) -> Option<String> {
        let abs = uri_to_path(uri)?;
        let ws = self.workspace.to_string_lossy();
        let rel = abs.strip_prefix(ws.as_ref())?.trim_start_matches('/');
        if rel.is_empty() {
            return None;
        }
        Some(rel.to_string())
    }

    /// Upgrade all EXTRACTED `calls` edges whose source path is `file`.
    ///
    /// Fatal protocol failures return `Err`; per-call-site failures are
    /// counted in [`LspResult`].
    pub fn resolve_call_definitions(
        &mut self,
        store: &Store,
        file: &str,
    ) -> Result<LspResult, String> {
        let mut out = LspResult::default();
        let rel_ids: HashSet<String> = store
            .relationship_ids_with_source(file, scc_core::predicates::CALLS)
            .map_err(|e| e.to_string())?
            .into_iter()
            .collect();
        if rel_ids.is_empty() {
            return Ok(out);
        }
        let rels: Vec<Relationship> = store
            .all_relationships()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|r| {
                r.predicate == scc_core::predicates::CALLS
                    && r.provenance == Provenance::Extracted
                    && rel_ids.contains(&r.id)
            })
            .collect();

        for rel in &rels {
            let Some(ev_id) = rel.evidence.first() else {
                out.unresolved += 1;
                continue;
            };
            let Some(ev) = store.get_evidence(ev_id).map_err(|e| e.to_string())? else {
                out.unresolved += 1;
                continue;
            };
            let Some(line) = ev.start_line else {
                out.unresolved += 1;
                continue;
            };
            let callee = ev.symbol.clone().unwrap_or_default();

            match self.definition_for(store, file, line, &callee)? {
                DefOutcome::Resolved(target_path, target_name) => {
                    self.apply_upgrade(store, rel, file, &target_path, &target_name, &callee, line)?;
                    out.upgraded += 1;
                }
                DefOutcome::Error => {
                    out.errors += 1;
                    out.unresolved += 1;
                }
                DefOutcome::Empty => {
                    out.unresolved += 1;
                }
            }
        }
        Ok(out)
    }

    fn apply_upgrade(
        &self,
        store: &Store,
        rel: &Relationship,
        file: &str,
        target_path: &str,
        target_name: &str,
        callee: &str,
        line: u32,
    ) -> Result<(), String> {
        let repo_id = store.repository().id;
        let new_object = scc_core::symbol_id(&repo_id, target_path, target_name);
        let ev = Evidence {
            id: evidence_id(file, "call", callee, line),
            r#type: EvidenceType::Source,
            path: Some(file.to_string()),
            symbol: Some(callee.to_string()),
            start_line: Some(line),
            end_line: None,
            revision: None,
            content_hash: None,
            extractor: Some(LSP_EXTRACTOR.to_string()),
            extractor_version: Some(self.ts_version.clone()),
        };
        store.insert_evidence(&ev).map_err(|e| e.to_string())?;
        // Remove the EXTRACTED edge (its id encodes the old object), then
        // insert the RESOLVED edge under the id scheme from crate::write.
        store
            .delete_relationship(&rel.id)
            .map_err(|e| e.to_string())?;
        let new_rel = Relationship::new(
            rel_id(&["calls", &rel.subject, &new_object]),
            rel.subject.clone(),
            scc_core::predicates::CALLS,
            new_object,
            Provenance::Resolved,
        )
        .with_confidence(0.99)
        .with_evidence(vec![ev.id.clone()]);
        store
            .insert_relationship(&new_rel, file)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Parse an LSP definition result (Location | LocationLink | array | null)
/// into (uri, line, token char). Returns `None` for null/empty results and
/// for degenerate "definitions" that point at the query position itself.
fn parse_definition_target(
    result: &Value,
    qline: u32,
    query_uri: &str,
) -> Option<Target> {
    fn pick(v: &Value) -> Option<(&str, u64, u64)> {
        let uri = v.get("uri").or_else(|| v.get("targetUri"))?.as_str()?;
        // Token range: `range` for a Location, `targetSelectionRange` for a
        // LocationLink (the range of the identifier itself).
        let range = v
            .get("targetSelectionRange")
            .or_else(|| v.get("range"))
            .or_else(|| v.get("targetRange"))?;
        let start = range.get("start")?;
        let line = start.get("line")?.as_u64()?;
        let character = start.get("character")?.as_u64()?;
        Some((uri, line, character))
    }
    let entry = match result {
        Value::Null => return None,
        Value::Array(items) => items.iter().find_map(pick)?,
        Value::Object(_) => pick(result)?,
        _ => return None,
    };
    let (uri, line, character) = entry;
    // A definition cannot sit on the queried position (servers return the
    // queried node itself when binding is incomplete).
    if uri == query_uri && line == u64::from(qline) {
        return None;
    }
    Some(Target {
        uri: uri.to_string(),
        line: line as u32,
        char: character as usize,
    })
}

#[derive(Debug)]
struct Target {
    uri: String,
    line: u32,
    char: usize,
}

/// Find the smallest symbol in `file` whose [start_line, end_line] contains
/// `line` (LSP 0-based → store 1-based).
fn find_containing_symbol(store: &Store, file: &str, line: u32) -> Option<String> {
    let want = line + 1;
    let syms = store.symbols_in_file(file).ok()?;
    syms.iter()
        .filter(|(_, _, _, _, start, end, _, _)| *start <= want && want <= *end)
        .min_by_key(|(_, _, _, _, start, end, _, _)| end - start)
        .map(|(_, name, _, _, _, _, _, _)| name.clone())
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use scc_core::entity_id;
    use scc_core::kinds;

    // ---------------------------------------------------------------
    // mock-server upgrade test (no tsserver required)
    // ---------------------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn mock_server_upgrades_extracted_edge() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("lib/util")).unwrap();
        std::fs::write(
            root.join("lib/util/impl.ts"),
            "export function helper(x: number): number {\n    return x + 1;\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/main.ts"),
            "import { helper } from \"@app/util\";\n\nexport function main(): number {\n    return helper(1);\n}\n",
        )
        .unwrap();

        let store = seed_store(root);
        let repo = store.repository().id;

        // Fake tsserver: answers initialize, then returns a fixed LocationLink
        // pointing at lib/util/impl.ts line 0 (0-based) for every definition
        // request.
        let (client_reader, server_writer) = std::os::unix::net::UnixStream::pair().unwrap();
        let (server_reader, client_writer) = std::os::unix::net::UnixStream::pair().unwrap();
        let root_owned = root.to_path_buf();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(server_reader);
            let mut writer = server_writer;
            while let Ok(msg) = crate::lsp::read_frame(&mut reader) {
                let method = msg.get("method").and_then(Value::as_str);
                let id = msg.get("id");
                if method == Some("initialize") {
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {"capabilities": {"definitionProvider": true}},
                    });
                    let _ = writer.write_all(&crate::lsp::encode_frame(&resp));
                } else if method == Some("textDocument/definition") {
                    let uri = format!("file://{}/lib/util/impl.ts", root_owned.display());
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": [{
                            "targetUri": uri,
                            "targetRange": {
                                "start": {"line": 0, "character": 0},
                                "end": {"line": 1, "character": 0},
                            },
                            "targetSelectionRange": {
                                "start": {"line": 0, "character": 16},
                                "end": {"line": 0, "character": 22},
                            },
                            "originSelectionRange": {
                                "start": {"line": 3, "character": 11},
                                "end": {"line": 3, "character": 17},
                            }
                        }]
                    });
                    let _ = writer.write_all(&crate::lsp::encode_frame(&resp));
                }
                let _ = writer.flush();
            }
        });

        let mut resolver = TsLspResolver::with_io(
            root,
            "9.9.9".to_string(),
            Box::new(client_reader),
            Box::new(client_writer),
            None,
        )
        .unwrap();

        let result = resolver.resolve_call_definitions(&store, "src/main.ts").unwrap();
        assert_eq!(result.upgraded, 1, "{result:?}");
        assert_eq!(result.unresolved, 0);
        assert_eq!(result.errors, 0);

        let target = scc_core::symbol_id(&repo, "lib/util/impl.ts", "helper");
        let rels = store.all_relationships().unwrap();
        let upgraded: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS)
            .collect();
        assert_eq!(upgraded.len(), 1);
        assert_eq!(upgraded[0].object, target);
        assert_eq!(upgraded[0].provenance, Provenance::Resolved);
        assert_eq!(upgraded[0].confidence, 0.99);
        assert_eq!(upgraded[0].evidence.len(), 1);

        let ev = store.get_evidence(&upgraded[0].evidence[0]).unwrap().unwrap();
        assert_eq!(ev.extractor.as_deref(), Some(LSP_EXTRACTOR));
        assert_eq!(ev.extractor_version.as_deref(), Some("9.9.9"));
        assert_eq!(ev.symbol.as_deref(), Some("helper"));
        assert_eq!(ev.start_line, Some(4));

        // the old EXTRACTED edge is gone
        assert!(!rels.iter().any(|r| r.provenance == Provenance::Extracted));
    }

    fn seed_store(root: &std::path::Path) -> Store {
        let db = root.join(".scc-test");
        std::fs::create_dir_all(&db).unwrap();
        let store = Store::open(&db.join("scc.db"), root).unwrap();
        let repo = store.repository().id;
        let file_id = entity_id(&repo, kinds::FILE, "src/main.ts");
        store
            .insert_entity(
                &scc_core::Entity::new(file_id.clone(), kinds::FILE, "src/main.ts"),
                &["src/main.ts".to_string()],
            )
            .unwrap();
        let main_id = scc_core::symbol_id(&repo, "src/main.ts", "main");
        store
            .insert_entity(
                &scc_core::Entity::new(main_id.clone(), kinds::SYMBOL, "main"),
                &["src/main.ts".to_string()],
            )
            .unwrap();
        store
            .insert_symbol("src/main.ts", "main", "function", None, 3, 5, true, None)
            .unwrap();
        let ext = entity_id(&repo, kinds::EXTERNAL_API, "-app/util");
        store
            .insert_entity(
                &scc_core::Entity::new(ext.clone(), kinds::EXTERNAL_API, "-app/util"),
                &["src/main.ts".to_string()],
            )
            .unwrap();
        // impl.ts target symbol
        store
            .insert_symbol("lib/util/impl.ts", "helper", "function", None, 1, 2, true, None)
            .unwrap();
        let ev = Evidence {
            id: evidence_id("src/main.ts", "call", "helper", 4),
            r#type: EvidenceType::Source,
            path: Some("src/main.ts".to_string()),
            symbol: Some("helper".to_string()),
            start_line: Some(4),
            end_line: None,
            revision: None,
            content_hash: None,
            extractor: Some("scc-native".to_string()),
            extractor_version: Some("0.0.0".to_string()),
        };
        store.insert_evidence(&ev).unwrap();
        let rel = Relationship::new(
            rel_id(&["calls", &main_id, &ext]),
            main_id,
            scc_core::predicates::CALLS,
            ext,
            Provenance::Extracted,
        )
        .with_confidence(0.8)
        .with_evidence(vec![ev.id]);
        store.insert_relationship(&rel, "src/main.ts").unwrap();
        store
    }

    // ---------------------------------------------------------------
    // optional integration test (requires tsserver on PATH)
    // ---------------------------------------------------------------

    #[test]
    fn tsserver_resolves_reexported_member() {
        let Some(_version) = tsserver_version() else {
            eprintln!("typescript-language-server not installed — skipping integration test");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("lib/util")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("tsconfig.json"),
            r#"{
  "compilerOptions": {
    "baseUrl": ".",
    "paths": { "@app/util": ["lib/util/index.ts"] }
  },
  "include": ["src/**/*", "lib/**/*"]
}
"#,
        )
        .unwrap();
        std::fs::write(root.join("lib/util/index.ts"), "export { helper } from \"./impl\";\n")
            .unwrap();
        std::fs::write(
            root.join("lib/util/impl.ts"),
            "export function helper(x: number): number {\n    return x + 1;\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/main.ts"),
            "import { helper } from \"@app/util\";\n\nexport function main(): number {\n    return helper(1);\n}\n",
        )
        .unwrap();

        let store = Store::open(&root.join("scc.db"), root).unwrap();
        let config = crate::config::Config::default();
        let indexer = crate::Indexer::new(store, config);
        indexer.index().unwrap();

        let store = Store::open(&root.join("scc.db"), root).unwrap();
        // native resolver cannot see through the tsconfig paths alias: the
        // call must be stored as an EXTRACTED edge to external_api
        let rels = store.all_relationships().unwrap();
        let pre: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS)
            .collect();
        assert_eq!(pre.len(), 1, "fixture must store one EXTRACTED call edge");
        assert_eq!(pre[0].provenance, Provenance::Extracted, "native resolver must miss the alias");

        let mut resolver = start_tsserver(root).unwrap();
        let result = resolver.resolve_call_definitions(&store, "src/main.ts").unwrap();
        assert!(
            result.upgraded >= 1,
            "tsserver should resolve the re-exported member: {result:?}"
        );

        let rels = store.all_relationships().unwrap();
        let target = scc_core::symbol_id(
            &store.repository().id,
            "lib/util/impl.ts",
            "helper",
        );
        let found = rels.iter().find(|r| {
            r.predicate == scc_core::predicates::CALLS && r.object == target
        });
        assert!(found.is_some(), "missing RESOLVED edge to {target}");
        let found = found.unwrap();
        assert_eq!(found.provenance, Provenance::Resolved);
        let ev = store.get_evidence(&found.evidence[0]).unwrap().unwrap();
        assert_eq!(ev.extractor.as_deref(), Some(LSP_EXTRACTOR));
        assert_eq!(ev.symbol.as_deref(), Some("helper"));
        assert_eq!(ev.start_line, Some(4));
    }
}
