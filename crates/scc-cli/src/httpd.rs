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
            let config = crate::load_config(root)?;
            let stale = crate::stale_paths(&store)?;
            let comp = crate::compiler(&store, &config, stale)?;
            let files = json_arr(&req, "files");
            let symbols = json_arr(&req, "symbols");
            let budget = req.get("token_budget").and_then(|b| b.as_u64()).map(|b| b as usize);
            Ok((200, serde_json::to_string(&comp.ctx().task_context(goal, &files, &symbols, budget))?))
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
pub fn watch_loop(root: &Path) -> crate::Result<()> {
    watch_loop_inner(root, false)
}

fn watch_loop_inner(root: &Path, quiet: bool) -> crate::Result<()> {
    let (tx, rx) = mpsc::channel::<notify::Event>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            let _ = tx.send(ev);
        }
    })
    .map_err(|e| crate::CliError::Other(format!("watcher: {e}")))?;
    notify::Watcher::watch(&mut watcher, root, notify::RecursiveMode::Recursive)
        .map_err(|e| crate::CliError::Other(format!("watch {root:?}: {e}")))?;

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
