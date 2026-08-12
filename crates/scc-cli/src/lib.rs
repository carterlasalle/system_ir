//! System Context Compiler CLI, daemon, MCP server, and Claude Code plugin.

pub mod bench;
pub mod benchagent;
pub mod benchctx;
pub mod benchres;
pub mod checkpoint;
pub mod commands;
pub mod compress;
pub mod embed_cli;
pub mod httpd;
pub mod mcp;
pub mod plugin;
pub mod plugin_hermes;
pub mod resolve;

use scc_context::ContextCompiler;
use scc_graph::RealityGraph;
use scc_indexer::Config;
use scc_store::Store;
use std::path::{Path, PathBuf};

pub const SCC_DIR: &str = ".scc";
pub const DB_FILE: &str = "scc.db";
pub const CONFIG_FILE: &str = "config.yaml";
pub const CHECKPOINT_FILE: &str = "checkpoint.json";

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("store: {0}")]
    Store(#[from] scc_store::StoreError),
    #[error("index: {0}")]
    Index(#[from] scc_indexer::IndexError),
    #[error("graph: {0}")]
    Graph(#[from] scc_graph::GraphError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("config: {0}")]
    Config(#[from] scc_indexer::config::ConfigError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, CliError>;

/// SCC state directory: the repo's `.scc/` by default; `SCC_STATE_DIR`
/// relocates writable state (database, checkpoint) so the repository itself
/// can be mounted read-only (docs/DEPLOYMENT_AND_INFRA.md §3: read-only repo
/// + writable SCC data volume).
pub fn state_dir(root: &Path) -> PathBuf {
    match std::env::var("SCC_STATE_DIR") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => scc_dir(root),
    }
}

pub fn scc_dir(root: &Path) -> PathBuf {
    root.join(SCC_DIR)
}

pub fn db_path(root: &Path) -> PathBuf {
    state_dir(root).join(DB_FILE)
}

/// Config stays in the repo (read-only is fine): it is repository intent,
/// not SCC state.
pub fn config_path(root: &Path) -> PathBuf {
    scc_dir(root).join(CONFIG_FILE)
}

pub fn checkpoint_path(root: &Path) -> PathBuf {
    state_dir(root).join(CHECKPOINT_FILE)
}

/// Locate the repository root: walk up from cwd looking for `.git` or an
/// existing `.scc` dir; otherwise use cwd.
pub fn find_root(start: &Path) -> PathBuf {
    let mut dir = Some(start.to_path_buf());
    while let Some(d) = dir {
        if d.join(".git").exists() || d.join(SCC_DIR).exists() {
            return d;
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }
    start.to_path_buf()
}

pub fn load_config(root: &Path) -> Result<Config> {
    let p = config_path(root);
    if p.exists() {
        Ok(Config::load(&p)?)
    } else {
        Ok(Config::default())
    }
}

pub fn open_store(root: &Path) -> Result<Store> {
    let dir = state_dir(root);
    std::fs::create_dir_all(&dir)?;
    Ok(Store::open(&db_path(root), root)?)
}

pub fn recompile(store: &Store) -> Result<scc_graph::RecompileReport> {
    Ok(scc_graph::recompile(store)?)
}

/// Compute repository-relative paths whose content hash no longer matches
/// the indexed snapshot (deleted files count as stale).
pub fn stale_paths(store: &Store) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for (path, hash, _lang, _kind, _size) in store.all_files()? {
        let full = store.root.join(&path);
        let current = match std::fs::read(&full) {
            Ok(b) => scc_indexer::scan::hash_bytes(&b),
            Err(_) => String::new(), // deleted
        };
        if current != hash {
            out.push(path);
        }
    }
    Ok(out)
}

pub struct Compiler<'a> {
    pub store: &'a Store,
    pub graph: RealityGraph,
    pub settings: scc_context::ContextSettings,
    pub stale: Vec<String>,
}

/// Build a ready compiler with freshness state.
pub fn compiler<'a>(
    store: &'a Store,
    config: &Config,
    stale: Vec<String>,
) -> Result<Compiler<'a>> {
    let graph = RealityGraph::load(store)?;
    let settings = scc_context::ContextSettings {
        startup_tokens: config.context.startup_tokens,
        task_tokens: config.context.task_tokens,
        atlas_tokens: config.context.atlas_tokens,
        include_low_confidence_inference: config.context.include_low_confidence_inference,
        rank_salt: format!(
            "{}:{}:{}",
            config.inference.enabled,
            config.inference.embedding_model,
            config.inference.rerank_model
        ),
    };
    Ok(Compiler {
        store,
        graph,
        settings,
        stale,
    })
}

impl Compiler<'_> {
    /// Construct a ContextCompiler borrowing this compiler's graph.
    pub fn ctx(&self) -> ContextCompiler<'_> {
        ContextCompiler::new(
            self.store,
            &self.graph,
            self.settings.clone(),
            self.stale.clone(),
        )
    }
}

pub fn index_and_recompile(root: &Path, config: &Config) -> Result<scc_indexer::IndexReport> {
    let indexer = scc_indexer::Indexer::new(open_store(root)?, config.clone());
    let report = indexer.index()?;
    let store = open_store(root)?;
    recompile(&store)?;
    Ok(report)
}

// ---------------------------------------------------------------------------
// export (docs/DATA_STRATEGY.md §11)
// ---------------------------------------------------------------------------

pub fn export_ir(store: &Store) -> Result<scc_core::SystemIr> {
    let repository = store.repository();
    let snapshot = store
        .latest_snapshot()?
        .unwrap_or(scc_core::Snapshot {
            revision: "not-indexed".into(),
            branch: None,
            indexed_at: scc_core::now_rfc3339(),
        });
    let mut ir = scc_core::SystemIr::empty(repository, snapshot);
    // entities: everything except the derived component copies (they are
    // already stored as entities by replace_components — dedupe)
    let mut seen = std::collections::HashSet::new();
    for e in store.all_entities()? {
        if seen.insert(e.id.clone()) {
            ir.entities.push(e);
        }
    }
    ir.relationships = store.all_relationships()?;
    ir.flows = store.flows()?;
    ir.invariants = store.invariants()?;
    ir.evidence = store.all_evidence()?;
    Ok(ir)
}

/// JSONL export: one JSON object per line (repository, snapshot, then
/// entities/relationships/flows/invariants/evidence records).
pub fn export_jsonl(ir: &scc_core::SystemIr) -> Result<Vec<String>> {
    let mut out = Vec::new();
    out.push(serde_json::to_string(&serde_json::json!({
        "type": "repository", "repository": ir.repository
    }))?);
    out.push(serde_json::to_string(&serde_json::json!({
        "type": "snapshot", "snapshot": ir.snapshot, "schema_version": ir.schema_version
    }))?);
    for e in &ir.entities {
        out.push(serde_json::to_string(&serde_json::json!({"type": "entity", "entity": e}))?);
    }
    for r in &ir.relationships {
        out.push(serde_json::to_string(&serde_json::json!({"type": "relationship", "relationship": r}))?);
    }
    for f in &ir.flows {
        out.push(serde_json::to_string(&serde_json::json!({"type": "flow", "flow": f}))?);
    }
    for i in &ir.invariants {
        out.push(serde_json::to_string(&serde_json::json!({"type": "invariant", "invariant": i}))?);
    }
    for e in &ir.evidence {
        out.push(serde_json::to_string(&serde_json::json!({"type": "evidence", "evidence": e}))?);
    }
    Ok(out)
}

/// Narsil-CCG-compatible layered export (docs §44): L0 manifest, L1
/// architecture, L2 symbols.
pub fn export_ccg(ir: &scc_core::SystemIr) -> Result<serde_json::Value> {
    let l1: Vec<serde_json::Value> = ir
        .entities
        .iter()
        .filter(|e| {
            e.kind == kinds::COMPONENT
                || e.kind == kinds::SERVICE
                || e.kind == kinds::DATA_STORE
                || e.kind == kinds::DEPLOYMENT_UNIT
                || e.kind == kinds::EXTERNAL_API
        })
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "name": e.name,
                "kind": e.kind,
                "attributes": e.attributes,
            })
        })
        .collect();
    let l2: Vec<serde_json::Value> = ir
        .entities
        .iter()
        .filter(|e| e.kind == kinds::SYMBOL)
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "name": e.name,
                "kind": e.attributes.get("kind").cloned().unwrap_or(serde_json::json!("symbol")),
                "file": e.attributes.get("file").cloned().unwrap_or_default(),
            })
        })
        .collect();
    Ok(serde_json::json!({
        "schema": "ccg",
        "producer": "scc",
        "repository": ir.repository,
        "snapshot": ir.snapshot,
        "layers": {
            "L0": {
                "manifest": {
                    "repository": ir.repository.name,
                    "revision": ir.snapshot.revision,
                    "entity_count": ir.entities.len(),
                    "relationship_count": ir.relationships.len(),
                }
            },
            "L1": { "architecture": l1 },
            "L2": { "symbols": l2 },
        }
    }))
}

pub fn flow_kind_str(k: &scc_core::FlowKind) -> &'static str {
    match k {
        scc_core::FlowKind::Architecture => "architecture",
        scc_core::FlowKind::Workflow => "workflow",
        scc_core::FlowKind::Sequence => "sequence",
        scc_core::FlowKind::Dataflow => "dataflow",
        scc_core::FlowKind::Lifecycle => "lifecycle",
    }
}

pub use scc_core::kinds;

/// Repo-relative path of a file under root, or None if it escapes.
pub fn relative_of(root: &Path, abs: &Path) -> Option<String> {
    let root_c = root.canonicalize().ok()?;
    let abs_c = abs.canonicalize().ok()?;
    let rel = abs_c.strip_prefix(&root_c).ok()?;
    if rel.as_os_str().is_empty() {
        return None;
    }
    let s = rel.to_string_lossy().replace('\\', "/");
    if s.starts_with(".scc/") {
        return None;
    }
    Some(s)
}
