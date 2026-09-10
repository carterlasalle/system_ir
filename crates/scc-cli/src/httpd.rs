//! Local daemon (docs/DEPLOYMENT_AND_INFRA.md §2): loopback HTTP API per
//! docs/openapi.yaml + filesystem watcher with debounced incremental
//! re-indexing.
//!
//! Security: binds to 127.0.0.1 by default (config.security.listen). All
//! endpoints are repository read-only except `/v1/index` and
//! `/v1/runtime/traces` (documented mutation class).

use scc_store::Store;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;


/// Fail closed on non-loopback binds: SCC serves index-mutating endpoints
/// with no authentication, so LAN exposure needs explicit opt-in
/// (`SCC_ALLOW_REMOTE_LISTEN=1`). Loopback and `localhost` always pass.
// trace:v1 id=impl.crates-scc-cli-src-httpd.require-remote-opt-in work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
fn require_remote_opt_in(addr: &str) -> crate::Result<()> {
    let loopback = match addr.parse::<std::net::SocketAddr>() {
        Ok(sa) => sa.ip().is_loopback(),
        Err(_) => {
            let host = addr.strip_prefix('[').and_then(|s| s.split(']').next());
            let host = host.unwrap_or_else(|| addr.split(':').next().unwrap_or(addr));
            host == "localhost" || host == "127.0.0.1" || host == "::1"
        }
    };
    if loopback {
        return Ok(());
    }
    if std::env::var("SCC_ALLOW_REMOTE_LISTEN").as_deref() == Ok("1") {
        eprintln!("warning: unauthenticated SCC daemon on non-loopback {addr} (explicit opt-in)");
        return Ok(());
    }
    Err(crate::CliError::Other(format!(
        "refusing non-loopback bind {addr}: the SCC daemon has no authentication;          bind config.security.listen to 127.0.0.1 or set SCC_ALLOW_REMOTE_LISTEN=1 to opt in explicitly"
    )))
}

// trace:v1 id=impl.crates-scc-cli-src-httpd.serve
pub fn serve(root: &Path) -> crate::Result<()> {
    let config = crate::load_config(root)?;
    let addr = config.security.listen.clone();

    // watcher thread
    let watch_root = root.to_path_buf();
    let _watcher_handle = if config.index.watch {
        Some(std::thread::spawn(move || {
            let _ = watch_loop_inner(&watch_root, true);
        }))
    } else {
        None
    };

    // Remote listening is unauthenticated by design (loopback-only
    // product): a non-loopback bind refuses unless the operator opts in
    // explicitly. There is no auth token yet — see docs/SECURITY.md.
    require_remote_opt_in(&addr)?;

    let server = tiny_http::Server::http(&addr)
        .map_err(|e| crate::CliError::Other(format!("cannot bind {addr}: {e}")))?;
    println!("scc daemon listening on http://{addr} (root {})", root.display());
    for request in server.incoming_requests() {
        let root = root.to_path_buf();
        let addr = addr.clone();
        let _ = handle_request(root, request, &addr);
    }
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-httpd.handle-request
fn handle_request(
    root: PathBuf,
    mut request: tiny_http::Request,
    addr: &str,
) -> crate::Result<()> {
    let url = request.url().to_string();
    let method = request.method().clone();
    let mut body = String::new();
    if method == tiny_http::Method::Post {
        let mut buf = Vec::new();
        request.as_reader().take(8 * 1024 * 1024).read_to_end(&mut buf)?;
        body = String::from_utf8_lossy(&buf).to_string();
    }

    let (status, payload) = route(&root, &method.to_string(), &url, &body, addr)?;
    let response = tiny_http::Response::from_string(payload).with_status_code(status);
    let _ = request.respond(response);
    Ok(())
}
// trace:v1 id=impl.scc.http work=WORK-SCC-001 satisfies=REQ-SCC-API

// trace:v1 id=impl.crates-scc-cli-src-httpd.route
fn route(
    root: &Path,
    method: &str,
    url: &str,
    body: &str,
    addr: &str,
) -> crate::Result<(u16, String)> {
    let path = url.split('?').next().unwrap_or(url);
    let json_err = |code: u16, msg: String| -> crate::Result<(u16, String)> {
        Ok((
            code,
            serde_json::to_string(&serde_json::json!({"error": msg}))?,
        ))
    };

    match (method, path) {
        ("GET", "/v1/system") => {
            let store = crate::open_store(root)?;
            if store.snapshot_status()?.is_none() {
                return json_err(409, "not indexed; POST /v1/index first".into());
            }
            let config = crate::load_config(root)?;
            let stale = crate::stale_paths(&store)?;
            let comp = crate::compiler(&store, &config, stale)?;
            Ok((200, serde_json::to_string(&comp.ctx().system_overview())?))
        }
        ("POST", "/v1/context/task") => {
            let req: serde_json::Value = match serde_json::from_str(body) {
                Ok(v) => v,
                Err(_) => return json_err(400, "invalid JSON body".to_string()),
            };
            let goal = req.get("goal").and_then(|g| g.as_str()).unwrap_or("");
            if goal.is_empty() {
                return json_err(400, "missing required field: goal".into());
            }
            let store = crate::open_store(root)?;
            if store.snapshot_status()?.is_none() {
                return json_err(409, "not indexed".into());
            }
            let files = json_arr(&req, "files");
            let symbols = json_arr(&req, "symbols");
            let budget = req.get("token_budget").and_then(|b| b.as_u64()).map(|b| b as usize);
            // Transport parity: THE one complete task artifact — pack AND
            // surface delta, same derivation as CLI text/JSON and MCP.
            // Serialization is the only difference (structured JSON here).
            let artifact =
                crate::commands::build_task_context(root, goal, &files, &symbols, budget, false)?;
            Ok((200, serde_json::to_string(&artifact)?))
        }
        ("POST", "/v1/context/startup") => {
            let req: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::json!({}));
            let store = crate::open_store(root)?;
            if store.snapshot_status()?.is_none() {
                return json_err(409, "not indexed; POST /v1/index first".into());
            }
            let config = crate::load_config(root)?;
            let stale = crate::stale_paths(&store)?;
            let comp = crate::compiler(&store, &config, stale)?;
            let ctx = comp.ctx();
            // Transport parity: THE shared allocator; `token_budget` absent
            // selects the default total and STILL adapts.
            let budget_tokens = req.get("token_budget").and_then(|b| b.as_u64()).map(|b| b as usize);
            let budget = scc_context::startup::allocate_startup_budget(&ctx, budget_tokens);
            let startup =
                scc_context::startup::build_startup(&ctx, &budget, scc_context::startup::RENDERER_VERSION);
            // Ledger parity with CLI/MCP: record what THIS transport showed.
            let ledger_store = scc_context::context_ledger::ContextLedgerStore::new(&store);
            let mut led = ledger_store.load();
            let (syms, files, comps, flows) =
                scc_context::startup::visible_ids_from_startup(&ctx, &startup);
            led.visible_entities.extend(syms.iter().cloned());
            led.visible_symbols.extend(syms);
            led.visible_files.extend(files);
            led.visible_components.extend(comps);
            led.visible_flows.extend(flows);
            ledger_store.save(&led);
            Ok((
                200,
                serde_json::to_string(&serde_json::json!({
                    "text": scc_context::startup::render_startup(&startup),
                    "budget": budget,
                    "artifact": startup.artifact,
                }))?,
            ))
        }
        ("GET", "/v1/atlas") => {
            let store = crate::open_store(root)?;
            if store.snapshot_status()?.is_none() {
                return json_err(409, "not indexed".into());
            }
            let config = crate::load_config(root)?;
            let stale = crate::stale_paths(&store)?;
            let comp = crate::compiler(&store, &config, stale)?;
            Ok((200, serde_json::to_string(&comp.ctx().system_atlas(None))?))
        }
        ("GET", p) if p.starts_with("/v1/components/") => {
            let id = p.trim_start_matches("/v1/components/");
            let store = crate::open_store(root)?;
            if store.snapshot_status()?.is_none() {
                return json_err(409, "not indexed".into());
            }
            let config = crate::load_config(root)?;
            let stale = crate::stale_paths(&store)?;
            let comp = crate::compiler(&store, &config, stale)?;
            Ok((200, serde_json::to_string(&comp.ctx().component_context(id))?))
        }
        ("GET", p) if p.starts_with("/v1/flows/") => {
            let id = p.trim_start_matches("/v1/flows/");
            let store = crate::open_store(root)?;
            if store.snapshot_status()?.is_none() {
                return json_err(409, "not indexed".into());
            }
            let config = crate::load_config(root)?;
            let stale = crate::stale_paths(&store)?;
            let comp = crate::compiler(&store, &config, stale)?;
            Ok((200, serde_json::to_string(&comp.ctx().flow_context(id))?))
        }
        ("POST", "/v1/impact") => {
            let req: serde_json::Value = match serde_json::from_str(body) {
                Ok(v) => v,
                Err(_) => return json_err(400, "invalid JSON body".to_string()),
            };
            let store = crate::open_store(root)?;
            if store.snapshot_status()?.is_none() {
                return json_err(409, "not indexed".into());
            }
            let config = crate::load_config(root)?;
            let stale = crate::stale_paths(&store)?;
            let comp = crate::compiler(&store, &config, stale)?;
            let files = json_arr(&req, "files");
            let symbols = json_arr(&req, "symbols");
            let diff = req.get("diff").and_then(|d| d.as_str()).map(|s| s.to_string());
            Ok((
                200,
                serde_json::to_string(&comp.ctx().impact_context(
                    &files,
                    &symbols,
                    diff.as_deref(),
                ))?,
            ))
        }
        ("POST", "/v1/verify") => {
            let store = crate::open_store(root)?;
            if store.snapshot_status()?.is_none() {
                return json_err(409, "not indexed".into());
            }
            let config = crate::load_config(root)?;
            let stale = crate::stale_paths(&store)?;
            let comp = crate::compiler(&store, &config, stale)?;
            Ok((200, serde_json::to_string(&comp.ctx().verify_context())?))
        }
        ("POST", "/v1/index") => {
            crate::commands::cmd_index(root, true)?;
            let store = crate::open_store(root)?;
            let status = store.snapshot_status()?;
            Ok((
                202,
                serde_json::to_string(&serde_json::json!({
                    "status": "ok",
                    "revision": status.map(|(s, _)| s.revision).unwrap_or_default(),
                }))?,
            ))
        }
        ("GET", "/v1/index/status") => {
            let store = crate::open_store(root)?;
            match store.snapshot_status()? {
                Some((snap, files)) => Ok((
                    200,
                    serde_json::to_string(&serde_json::json!({
                        "indexed": true,
                        "revision": snap.revision,
                        "branch": snap.branch,
                        "indexed_at": snap.indexed_at,
                        "files": files,
                    }))?,
                )),
                None => Ok((
                    200,
                    serde_json::to_string(&serde_json::json!({"indexed": false}))?,
                )),
            }
        }
        ("POST", "/v1/runtime/traces") => {
            let store = crate::open_store(root)?;
            ingest_runtime(&store, body)?;
            Ok((202, serde_json::to_string(&serde_json::json!({"status": "accepted"}))?))
        }
        ("GET", "/healthz") => Ok((200, "ok".into())),
        _ => {
            let _ = addr;
            json_err(404, format!("no route for {method} {path}"))
        }
    }
}

// trace:v1 id=impl.crates-scc-cli-src-httpd.json-arr
fn json_arr(v: &serde_json::Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(|x| x.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Runtime observation ingest: OTLP/JSON traces (`resourceSpans`) or the
/// simple `[{source, target, count}]` shape. Aggregates into runtime_edges
/// (OBSERVED provenance).
// trace:v1 id=impl.crates-scc-cli-src-httpd.ingest-runtime
pub fn ingest_runtime(store: &Store, body: &str) -> crate::Result<()> {
    if body.contains("resourceSpans") {
        scc_indexer::runtime::ingest_otlp_json(store, body)
            .map_err(crate::CliError::Other)?;
        return Ok(());
    }
    scc_indexer::runtime::ingest_simple_edges(store, body)
        .map_err(crate::CliError::Other)?;
    Ok(())
}


// ---------------------------------------------------------------------------
// file watcher
// ---------------------------------------------------------------------------

/// `scc watch`: foreground watcher loop.
// trace:v1 id=impl.crates-scc-cli-src-httpd.watch-loop
pub fn watch_loop(root: &Path) -> crate::Result<()> {
    watch_loop_inner(root, false)
}

/// Refresh files whose content hash no longer matches the snapshot.
/// Hash remains authority; used when the OS watcher cannot start.
// trace:v1 id=impl.scc.cli.hash-sweep work=WORK-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing-from-unique satisfies=REQ-implement-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing,REQ-implement-fix-pr-review-comments-without-collapsing-scc-type-script-no
pub fn refresh_stale_by_hash(root: &Path) -> crate::Result<Vec<String>> {
    let store = crate::open_store(root)?;
    // Single notion of staleness: modified, deleted, AND added files
    // (scan-diff lives in `stale_paths`, shared with verify/status).
    let mut paths = crate::stale_paths(&store)?;
    drop(store);
    paths.sort();
    paths.dedup();
    if !paths.is_empty() {
        crate::commands::cmd_index_paths(root, &paths, true)?;
    }
    Ok(paths)
}

// trace:v1 id=impl.crates-scc-cli-src-httpd.watch-loop-inner
fn watch_loop_inner(root: &Path, quiet: bool) -> crate::Result<()> {
    let (tx, rx) = mpsc::channel::<notify::Event>();
    let mut watcher = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            let _ = tx.send(ev);
        }
    }) {
        Ok(w) => w,
        Err(e) => {
            if !quiet {
                eprintln!("watcher unavailable ({e}); falling back to content-hash sweep");
            }
            return hash_sweep_loop(root, quiet);
        }
    };
    if let Err(e) = notify::Watcher::watch(&mut watcher, root, notify::RecursiveMode::Recursive) {
        if !quiet {
            eprintln!("watch {root:?} failed ({e}); falling back to content-hash sweep");
        }
        drop(watcher);
        return hash_sweep_loop(root, quiet);
    }

    if !quiet {
        println!("watching {} (ctrl-c to stop)", root.display());
    }
    let mut pending: std::collections::BTreeSet<String> = Default::default();
    let mut last: std::time::Instant = std::time::Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(ev) => {
                for p in ev.paths {
                    if let Some(rel) = crate::relative_of(root, &p) {
                        pending.insert(rel);
                    }
                }
                last = std::time::Instant::now();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if pending.is_empty() {
                    continue;
                }
                if last.elapsed() < Duration::from_millis(400) {
                    continue; // debounce
                }
                let paths: Vec<String> = std::mem::take(&mut pending).into_iter().collect();
                let res = crate::commands::cmd_index_paths(root, &paths, true);
                match res {
                    Ok(()) => {}
                    Err(e) => eprintln!("reindex error: {e}"),
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

// trace:exempt reason=internal-detail
fn hash_sweep_loop(root: &Path, quiet: bool) -> crate::Result<()> {
    if !quiet {
        println!("hash-sweep watching {} (ctrl-c to stop)", root.display());
    }
    loop {
        if let Err(e) = refresh_stale_by_hash(root) {
            eprintln!("hash sweep error: {e}");
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchctx::{copy_fixture, locate_fixtures_dir};

    #[test]
    // trace:v1 id=test.scc.cli.remote-listen-gate verifies=REQ-SI-503JSBGP exercises=impl.crates-scc-cli-src-httpd.require-remote-opt-in
    fn remote_listen_fails_closed_without_opt_in() {
        assert!(require_remote_opt_in("127.0.0.1:7777").is_ok());
        assert!(require_remote_opt_in("localhost:7777").is_ok());
        assert!(require_remote_opt_in("[::1]:7777").is_ok());
        // unauthenticated daemon: non-loopback refuses without opt-in
        assert!(require_remote_opt_in("0.0.0.0:7777").is_err());
        std::env::remove_var("SCC_ALLOW_REMOTE_LISTEN");
        assert!(require_remote_opt_in("192.168.1.10:7777").is_err());
    }

    #[test]
    // trace:v1 id=test.scc.cli.hash-sweep verifies=REQ-implement-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing,REQ-implement-fix-pr-review-comments-without-collapsing-scc-type-script-no exercises=impl.scc.cli.hash-sweep
    fn hash_sweep_refreshes_edited_file() {
        let fixtures = locate_fixtures_dir().expect("fixtures");
        let src = fixtures.join("behavior-native");
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("repo");
        copy_fixture(&src, &root);
        crate::commands::cmd_index(&root, true).unwrap();
        let store = crate::open_store(&root).unwrap();
        assert!(crate::stale_paths(&store).unwrap().is_empty());
        drop(store);
        let app = root.join("app.py");
        let mut text = std::fs::read_to_string(&app).unwrap();
        text.push_str("\n# hash-sweep probe\n");
        std::fs::write(&app, text).unwrap();
        let stale = refresh_stale_by_hash(&root).unwrap();
        assert!(
            stale.iter().any(|p| p == "app.py" || p.ends_with("/app.py")),
            "edited file must be in the hash sweep: {stale:?}"
        );
        let store = crate::open_store(&root).unwrap();
        assert!(
            crate::stale_paths(&store).unwrap().is_empty(),
            "after sweep the snapshot must match disk"
        );
    }

    #[test]
    // trace:v1 id=test.scc.cli.hash-sweep-new-file verifies=REQ-implement-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing,REQ-implement-fix-pr-review-comments-without-collapsing-scc-type-script-no exercises=impl.scc.cli.hash-sweep
    fn hash_sweep_indexes_newly_created_file() {
        let fixtures = locate_fixtures_dir().expect("fixtures");
        let src = fixtures.join("behavior-native");
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("repo");
        copy_fixture(&src, &root);
        crate::commands::cmd_index(&root, true).unwrap();
        std::fs::write(root.join("fresh.py"), "def fresh():\n    return 1\n").unwrap();
        let stale = refresh_stale_by_hash(&root).unwrap();
        assert!(
            stale.iter().any(|p| p == "fresh.py" || p.ends_with("/fresh.py")),
            "new file must be in the hash sweep: {stale:?}"
        );
        let store = crate::open_store(&root).unwrap();
        let files: Vec<_> = store
            .all_files()
            .unwrap()
            .into_iter()
            .map(|(p, _, _, _, _)| p)
            .collect();
        assert!(
            files.iter().any(|p| p == "fresh.py" || p.ends_with("/fresh.py")),
            "new file must be indexed: {files:?}"
        );
    }
}
