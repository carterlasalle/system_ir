//! SQLite persistence for the System Context Compiler.
//!
#![allow(clippy::type_complexity)]

//! Layer mapping (docs/DATA_STRATEGY.md):
//! - L0 repository snapshot: `repositories`, `snapshots`, `files`
//! - L1 evidence: `evidence`
//! - L2 reality graph: `entities`, `relationships`, `symbols`
//! - L3 system IR: `components`, `flows`, `invariants`, `tests`
//! - L4 context indexes: FTS5 (`symbols_fts`, `entities_fts`), `context_cache`
//! - L5 history: `intent_claims`, `drift_findings`

pub use rusqlite;
use rusqlite::{params, Connection, OptionalExtension};
use scc_core::{
    Entity, Evidence, Flow, Invariant, Provenance, Relationship, Repository, Severity, Snapshot,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const SCHEMA_VERSION: u32 = 6;
pub const FTS_ESCAPE: &str = "\"";
const MIGRATIONS: &[&str] = &[
    MIGRATION_1,
    MIGRATION_2,
    MIGRATION_3,
    MIGRATION_4,
    MIGRATION_5,
    MIGRATION_6,
];

/// v4: model epoch. `context_cache.revision` becomes `epoch` — the cache is
/// keyed on the composite model state (source/semantic/evidence/intent/
/// runtime/derived generations), not the git revision alone, so any change
/// to system truth invalidates stale packs (docs/SYSTEM_DESIGN.md §5).
const MIGRATION_4: &str = r#"
ALTER TABLE context_cache RENAME COLUMN revision TO epoch;
"#;

/// v5: canonical causal flow graphs (Wave 3) — the behavioral truth from
/// which the `flows` projections are derived.
const MIGRATION_5: &str = r#"
CREATE TABLE IF NOT EXISTS flow_graphs (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  name TEXT NOT NULL,
  trigger TEXT,
  graph TEXT NOT NULL
);
"#;

/// v6: observed trace-path signatures (Wave 6) — canonical root-to-leaf
/// service paths per trace, aggregated across ingests. `count` is additive
/// per trace occurrence, `latency_ms` a count-weighted running average,
/// `errors` additive.
const MIGRATION_6: &str = r#"
CREATE TABLE IF NOT EXISTS trace_signatures (
  signature TEXT PRIMARY KEY,
  count INTEGER NOT NULL DEFAULT 1,
  latency_ms REAL NOT NULL DEFAULT 0,
  errors INTEGER NOT NULL DEFAULT 0,
  last_observed TEXT NOT NULL
);
"#;

/// v3: entity embeddings (f32 vector blobs) for the optional semantic ranker.
const MIGRATION_3: &str = r#"
CREATE TABLE IF NOT EXISTS embeddings (
  entity_id TEXT PRIMARY KEY,
  vector BLOB NOT NULL,
  model TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
"#;

/// v2: runtime edge aggregation columns (latency/error aggregates).
const MIGRATION_2: &str = r#"
ALTER TABLE runtime_edges ADD COLUMN latency_ms REAL NOT NULL DEFAULT 0;
ALTER TABLE runtime_edges ADD COLUMN errors INTEGER NOT NULL DEFAULT 0;
"#;

const MIGRATION_1: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS repositories (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  url TEXT,
  root TEXT NOT NULL,
  indexed_at TEXT
);

CREATE TABLE IF NOT EXISTS snapshots (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  revision TEXT NOT NULL,
  branch TEXT,
  indexed_at TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active',
  file_count INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS files (
  path TEXT PRIMARY KEY,
  hash TEXT NOT NULL,
  language TEXT NOT NULL DEFAULT 'unknown',
  kind TEXT NOT NULL DEFAULT 'other',
  size INTEGER NOT NULL DEFAULT 0,
  indexed_at TEXT
);

CREATE TABLE IF NOT EXISTS symbols (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  file TEXT NOT NULL,
  name TEXT NOT NULL,
  symbol_kind TEXT NOT NULL,
  signature TEXT,
  start_line INTEGER NOT NULL,
  end_line INTEGER NOT NULL,
  exported INTEGER NOT NULL DEFAULT 0,
  docstring TEXT
);
CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file);
CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);

CREATE TABLE IF NOT EXISTS imports (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  file TEXT NOT NULL,
  module TEXT NOT NULL,
  names TEXT NOT NULL DEFAULT '[]',
  line INTEGER NOT NULL DEFAULT 0,
  type TEXT NOT NULL DEFAULT 'member'
);
CREATE INDEX IF NOT EXISTS idx_imports_file ON imports(file);

CREATE TABLE IF NOT EXISTS entities (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  name TEXT NOT NULL,
  attributes TEXT NOT NULL DEFAULT '{}',
  evidence TEXT NOT NULL DEFAULT '[]',
  sources TEXT NOT NULL DEFAULT '[]'
);
CREATE INDEX IF NOT EXISTS idx_entities_kind ON entities(kind);

CREATE TABLE IF NOT EXISTS relationships (
  id TEXT PRIMARY KEY,
  subject TEXT NOT NULL,
  predicate TEXT NOT NULL,
  object TEXT NOT NULL,
  provenance TEXT NOT NULL,
  confidence REAL NOT NULL,
  evidence TEXT NOT NULL DEFAULT '[]',
  verified_at TEXT NOT NULL DEFAULT '',
  source_path TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_rel_subject ON relationships(subject);
CREATE INDEX IF NOT EXISTS idx_rel_object ON relationships(object);
CREATE INDEX IF NOT EXISTS idx_rel_predicate ON relationships(predicate);
CREATE INDEX IF NOT EXISTS idx_rel_source ON relationships(source_path);

CREATE TABLE IF NOT EXISTS evidence (
  id TEXT PRIMARY KEY,
  type TEXT NOT NULL,
  path TEXT,
  symbol TEXT,
  start_line INTEGER,
  end_line INTEGER,
  revision TEXT,
  content_hash TEXT,
  extractor TEXT,
  extractor_version TEXT
);
CREATE INDEX IF NOT EXISTS idx_evidence_path ON evidence(path);

CREATE TABLE IF NOT EXISTS components (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  responsibility TEXT NOT NULL DEFAULT '[]',
  implementation TEXT NOT NULL DEFAULT '[]',
  evidence TEXT NOT NULL DEFAULT '[]',
  attributes TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS flows (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  name TEXT NOT NULL,
  trigger TEXT,
  steps TEXT NOT NULL DEFAULT '[]',
  attributes TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS invariants (
  id TEXT PRIMARY KEY,
  statement TEXT NOT NULL,
  severity TEXT NOT NULL,
  scope TEXT NOT NULL DEFAULT '[]',
  enforced_by TEXT NOT NULL DEFAULT '[]',
  provenance TEXT NOT NULL DEFAULT 'DECLARED',
  evidence TEXT NOT NULL DEFAULT '[]'
);

CREATE TABLE IF NOT EXISTS tests (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  file TEXT NOT NULL,
  kind TEXT NOT NULL DEFAULT 'unit',
  symbol TEXT
);
CREATE INDEX IF NOT EXISTS idx_tests_file ON tests(file);

CREATE TABLE IF NOT EXISTS context_cache (
  key TEXT PRIMARY KEY,
  pack TEXT NOT NULL,
  revision TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS intent_claims (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  source TEXT NOT NULL,
  claim TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS runtime_edges (
  source TEXT NOT NULL,
  target TEXT NOT NULL,
  count INTEGER NOT NULL DEFAULT 1,
  last_observed TEXT NOT NULL,
  PRIMARY KEY (source, target)
);

CREATE TABLE IF NOT EXISTS drift_findings (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  kind TEXT NOT NULL,
  severity TEXT NOT NULL,
  message TEXT NOT NULL,
  created_at TEXT NOT NULL,
  resolved INTEGER NOT NULL DEFAULT 0
);

CREATE VIRTUAL TABLE IF NOT EXISTS symbols_fts USING fts5(
  name, signature, symbol_kind UNINDEXED, file UNINDEXED
);

CREATE VIRTUAL TABLE IF NOT EXISTS entities_fts USING fts5(
  id UNINDEXED, kind UNINDEXED, name, attributes
);
"#;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("repository not initialized: {0}")]
    NotInitialized(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, Clone, serde::Serialize)]
pub struct RuntimeEdgeRow {
    pub source: String,
    pub target: String,
    pub count: u64,
    pub latency_ms: f64,
    pub errors: u64,
    pub last_observed: String,
}

// trace:exempt reason=internal-detail

/// The independent truth sources whose generations compose the model epoch.
/// Any change to one of them invalidates previously cached context packs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// trace:exempt reason=internal-detail
pub enum ModelEpochKind {
    /// Indexed file contents (snapshot completion).
    Source,
    /// Semantic resolver promotions (LSP/SCIP edge upgrades).
    Semantic,
    /// Imported external evidence (SCIP/CCG/GitNexus/Beads/CBM/Hindsight).
    Evidence,
    /// Declared intent (`.scc/intent.yaml` claims).
    Intent,
    /// Runtime trace ingestion.
    Runtime,
    /// Derived compilation (components/flows/invariants/drift/boundaries).
    Derived,
}

impl ModelEpochKind {
    pub fn meta_key(&self) -> &'static str {
        match self {
            ModelEpochKind::Source => "source_generation",
            ModelEpochKind::Semantic => "semantic_generation",
            ModelEpochKind::Evidence => "evidence_generation",
            ModelEpochKind::Intent => "intent_generation",
            ModelEpochKind::Runtime => "runtime_generation",
            ModelEpochKind::Derived => "derived_generation",
        }
    }
}

// trace:exempt reason=internal-detail

/// Deterministic composite fingerprint of the model state. `composite()` is
/// the canonical cache-epoch string: it changes whenever any source of
/// system truth changes, so a previously fresh context pack can never be
/// served after its evidence is stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
// trace:exempt reason=internal-detail
pub struct ModelEpoch {
    pub source: u64,
    pub semantic: u64,
    pub evidence: u64,
    pub intent: u64,
    pub runtime: u64,
    pub derived: u64,
}

impl ModelEpoch {
    pub fn zero() -> ModelEpoch {
        ModelEpoch {
            source: 0,
            semantic: 0,
            evidence: 0,
            intent: 0,
            runtime: 0,
            derived: 0,
        }
    }

    /// Composite hash over every generation. Prefix identifies the scheme so
    /// a future epoch-shape change cannot collide with old keys.
    pub fn composite(&self, revision: &str) -> String {
        let mut h = blake3::Hasher::new();
        h.update(b"scc-model-epoch-v1");
        h.update(self.source.to_le_bytes().as_slice());
        h.update(self.semantic.to_le_bytes().as_slice());
        h.update(self.evidence.to_le_bytes().as_slice());
        h.update(self.intent.to_le_bytes().as_slice());
        h.update(self.runtime.to_le_bytes().as_slice());
        h.update(self.derived.to_le_bytes().as_slice());
        h.update(revision.as_bytes());
        format!("epoch:{}", &h.finalize().to_hex()[..24])
    }
}
// trace:v1 id=impl.scc.store work=WORK-SCC-001 satisfies=REQ-SCC-DATA implements=PLAN-SCC-001

pub struct Store {
    pub conn: Connection,
    pub root: PathBuf,
    /// Repository id (repo:// id component).
    pub repo_id: String,
    pub repo_name: String,
}

// trace:exempt reason=internal-detail
impl Store {
    /// Open (creating if needed) the SCC database at `path` for repository
    /// rooted at `root`. `root` must exist.
    pub fn open(path: &Path, root: &Path) -> Result<Store> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        apply_migrations(&conn)?;

        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let name = root
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "repository".to_string());
        let repo_id = scc_core::sanitize_key(&name);

        let mut store = Store {
            conn,
            root,
            repo_id,
            repo_name: name,
        };
        store.ensure_repository()?;
        Ok(store)
    }

    fn ensure_repository(&mut self) -> Result<()> {
        let existing: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM repositories WHERE id = ?1",
                params![self.repo_id],
                |r| r.get(0),
            )
            .optional()?;
        if existing.is_none() {
            self.conn.execute(
                "INSERT INTO repositories (id, name, root, indexed_at) VALUES (?1, ?2, ?3, ?4)",
                params![
                    self.repo_id,
                    self.repo_name,
                    self.root.to_string_lossy(),
                    scc_core::now_rfc3339()
                ],
            )?;
        }
        Ok(())
    }

    pub fn repository(&self) -> Repository {
        Repository {
            id: self.repo_id.clone(),
            name: self.repo_name.clone(),
            url: self
                .meta_get("remote_url")
                .ok()
                .flatten()
                .filter(|s| !s.is_empty()),
        }
    }

    // ------------------------------------------------------------------
    // meta
    // ------------------------------------------------------------------

    pub fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn meta_get(&self, key: &str) -> Result<Option<String>> {
        let v = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(v)
    }

    /// Bump one model-epoch generation. Called by every mutation path that
    /// changes system truth (index completion, LSP promotion, adapter
    /// import, intent load, runtime ingestion, derived recompilation).
    pub fn bump_epoch(&self, kind: ModelEpochKind) -> Result<()> {
        let key = kind.meta_key();
        let next: u64 = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
            + 1;
        self.meta_set(key, &next.to_string())
    }

    /// Current model epoch (all generations plus the latest snapshot
    /// revision). Never cached by the caller: it must reflect every change
    /// immediately.
    pub fn model_epoch(&self) -> Result<ModelEpoch> {
        let g = |k: &str| -> Result<u64> {
            Ok(self
                .meta_get(k)?
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0))
        };
        Ok(ModelEpoch {
            source: g(ModelEpochKind::Source.meta_key())?,
            semantic: g(ModelEpochKind::Semantic.meta_key())?,
            evidence: g(ModelEpochKind::Evidence.meta_key())?,
            intent: g(ModelEpochKind::Intent.meta_key())?,
            runtime: g(ModelEpochKind::Runtime.meta_key())?,
            derived: g(ModelEpochKind::Derived.meta_key())?,
        })
    }

    // ------------------------------------------------------------------
    // snapshots
    // ------------------------------------------------------------------

    pub fn begin_snapshot(&self, revision: &str, branch: Option<&str>) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO snapshots (revision, branch, indexed_at, status) VALUES (?1, ?2, ?3, 'active')",
            params![revision, branch, scc_core::now_rfc3339()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn finish_snapshot(&self, id: i64, file_count: usize) -> Result<()> {
        self.conn.execute(
            "UPDATE snapshots SET file_count = ?2, status = 'complete' WHERE id = ?1",
            params![id, file_count as i64],
        )?;
        // the indexed source tree changed — invalidate epoch-keyed packs
        self.bump_epoch(ModelEpochKind::Source)?;
        Ok(())
    }

    pub fn latest_snapshot(&self) -> Result<Option<Snapshot>> {
        let row = self
            .conn
            .query_row(
                "SELECT revision, branch, indexed_at FROM snapshots
                 WHERE status = 'complete' ORDER BY id DESC LIMIT 1",
                [],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        Ok(row.map(|(revision, branch, indexed_at)| Snapshot {
            revision,
            branch,
            indexed_at,
        }))
    }

    pub fn snapshot_status(&self) -> Result<Option<(Snapshot, i64)>> {
        let row = self
            .conn
            .query_row(
                "SELECT revision, branch, indexed_at, (SELECT COUNT(*) FROM files) FROM snapshots
                 WHERE status = 'complete' ORDER BY id DESC LIMIT 1",
                [],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?;
        Ok(row.map(|(revision, branch, indexed_at, files)| {
            (
                Snapshot {
                    revision,
                    branch,
                    indexed_at,
                },
                files,
            )
        }))
    }

    // ------------------------------------------------------------------
    // files
    // ------------------------------------------------------------------

    pub fn upsert_file(&self, path: &str, hash: &str, language: &str, kind: &str, size: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO files (path, hash, language, kind, size, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(path) DO UPDATE SET hash = excluded.hash, language = excluded.language,
               kind = excluded.kind, size = excluded.size, indexed_at = excluded.indexed_at",
            params![path, hash, language, kind, size as i64, scc_core::now_rfc3339()],
        )?;
        Ok(())
    }

    pub fn file(&self, path: &str) -> Result<Option<(String, String, String, u64)>> {
        let row = self
            .conn
            .query_row(
                "SELECT hash, language, kind, size FROM files WHERE path = ?1",
                params![path],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? as u64)),
            )
            .optional()?;
        Ok(row)
    }

    pub fn all_files(&self) -> Result<Vec<(String, String, String, String, u64)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, hash, language, kind, size FROM files")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)? as u64,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn delete_file(&self, path: &str) -> Result<()> {
        self.conn.execute("DELETE FROM files WHERE path = ?1", params![path])?;
        Ok(())
    }

// trace:exempt reason=internal-detail

    /// Remove everything tied to a source path: symbols, evidence,
    /// relationships derived from it, and symbol-level entities.
    ///
    /// SCHEMA/REACTIVE concept entities are NOT deleted by path: concepts
    /// are keyed by (kind, name) and may occur in many files. Their
    /// OCCURRENCE entities (per concept/path/owner/line) are deleted by
    /// path, the concept's `sources` provenance is recomputed from the
    /// surviving occurrences, and a concept with zero remaining
    /// occurrences is deleted entirely (with its edges) — so purging one
    /// file never deletes a schema another file still uses, and the
    /// derived occurrence count naturally reflects the survivors.
// trace:exempt reason=internal-detail
    pub fn purge_path(&self, path: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        // concepts this path's occurrences attach to — captured before any
        // deletion so their provenance can be recomputed afterwards
        let affected_concepts: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT DISTINCT r.object FROM relationships r
                 JOIN entities e ON e.id = r.subject
                 WHERE r.predicate = ?1 AND e.kind = ?2 AND e.sources LIKE ?3",
            )?;
            let rows = stmt.query_map(
                params![
                    scc_core::predicates::OCCURS,
                    scc_core::kinds::OCCURRENCE,
                    format!("%\"{path}\"%")
                ],
                |r| r.get::<_, String>(0),
            )?;
            let mut v = Vec::new();
            for r in rows {
                v.push(r?);
            }
            v.sort();
            v.dedup();
            v
        };
        // this path's occurrence entities (for edge + FTS cleanup)
        let occ_ids: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM entities WHERE kind = ?1 AND sources LIKE ?2",
            )?;
            let rows = stmt.query_map(
                params![scc_core::kinds::OCCURRENCE, format!("%\"{path}\"%")],
                |r| r.get::<_, String>(0),
            )?;
            let mut v = Vec::new();
            for r in rows {
                v.push(r?);
            }
            v
        };
        // symbol entities: repo://{repo}/symbol/{path}/{name}
        let ev_ids: Vec<String> = {
            let mut stmt = tx.prepare("SELECT id FROM evidence WHERE path = ?1")?;
            let rows = stmt.query_map(params![path], |r| r.get::<_, String>(0))?;
            let mut v = Vec::new();
            for r in rows {
                v.push(r?);
            }
            v
        };
        // collect the file's symbol names before deleting them
        let sym_names: Vec<String> = {
            let mut stmt = tx.prepare("SELECT name FROM symbols WHERE file = ?1")?;
            let rows = stmt.query_map(params![path], |r| r.get::<_, String>(0))?;
            let mut v = Vec::new();
            for r in rows {
                v.push(r?);
            }
            v
        };
        tx.execute("DELETE FROM symbols WHERE file = ?1", params![path])?;
        tx.execute("DELETE FROM imports WHERE file = ?1", params![path])?;
        tx.execute("DELETE FROM evidence WHERE path = ?1", params![path])?;
        // relationships pointing at this file's symbol entities (calls from
        // unchanged files into removed symbols must not dangle)
        let mut orphaned_evidence: Vec<String> = Vec::new();
        for name in sym_names {
            let sid = scc_core::symbol_id(&self.repo_id, path, &name);
            let mut stmt = tx.prepare(
                "SELECT id, evidence FROM relationships WHERE subject = ?1 OR object = ?1",
            )?;
            let rows = stmt.query_map(params![sid], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            let mut to_delete: Vec<(String, String)> = Vec::new();
            for r in rows {
                to_delete.push(r?);
            }
            drop(stmt);
            for (rid, ev_json) in to_delete {
                if let Ok(ev_ids) = serde_json::from_str::<Vec<String>>(&ev_json) {
                    orphaned_evidence.extend(ev_ids);
                }
                tx.execute("DELETE FROM relationships WHERE id = ?1", params![rid])?;
            }
        }
        // NOTE: evidence referenced only by deleted relationships is swept
        // later by `sweep_orphan_evidence` after the derived layer rebuilds
        // (component/flow edges may still reference it until then).
        let _ = orphaned_evidence;
        tx.execute(
            "DELETE FROM entities WHERE id LIKE ?1",
            params![format!("%/symbol/{path}/%")],
        )?;
        // non-concept entities whose sources include the path (stores,
        // routes, contracts, ...). SCHEMA/REACTIVE concepts are keyed by
        // (kind, name) across files and handled below from their live
        // occurrences — a shared concept must never be deleted because one
        // of its files was purged.
        tx.execute(
            "DELETE FROM entities WHERE sources LIKE ?1 AND kind NOT IN (?2, ?3)",
            params![
                format!("%\"{path}\"%"),
                scc_core::kinds::SCHEMA,
                scc_core::kinds::REACTIVE
            ],
        )?;
        tx.execute(
            "DELETE FROM relationships WHERE source_path = ?1",
            params![path],
        )?;
        // relationships referencing removed evidence ids
        for ev in ev_ids {
            tx.execute(
                "DELETE FROM relationships WHERE evidence LIKE ?1",
                params![format!("%\"{ev}\"%")],
            )?;
        }
        tx.execute(
            "DELETE FROM tests WHERE file = ?1",
            params![path],
        )?;
        // occurrence entities: delete their edges + FTS rows, then the
        // entities themselves
        for oid in &occ_ids {
            tx.execute(
                "DELETE FROM relationships WHERE subject = ?1 OR object = ?1",
                params![oid],
            )?;
            tx.execute("DELETE FROM entities_fts WHERE id = ?1", params![oid])?;
        }
        tx.execute(
            "DELETE FROM entities WHERE kind = ?1 AND sources LIKE ?2",
            params![scc_core::kinds::OCCURRENCE, format!("%\"{path}\"%")],
        )?;
        // recompute concept provenance: a concept survives as long as any
        // occurrence remains — its sources become the surviving occurrence
        // paths (deduped, sorted). With zero occurrences the concept and
        // its edges are gone.
        for concept in &affected_concepts {
            let remaining: Vec<String> = {
                let mut stmt = tx.prepare(
                    "SELECT DISTINCT json_extract(e.attributes, '$.path') FROM entities e
                     JOIN relationships r ON r.subject = e.id
                     WHERE r.predicate = ?1 AND r.object = ?2 AND e.kind = ?3
                       AND json_extract(e.attributes, '$.path') IS NOT NULL
                     ORDER BY 1",
                )?;
                let rows = stmt.query_map(
                    params![
                        scc_core::predicates::OCCURS,
                        concept,
                        scc_core::kinds::OCCURRENCE
                    ],
                    |r| r.get::<_, String>(0),
                )?;
                let mut v = Vec::new();
                for r in rows {
                    v.push(r?);
                }
                v
            };
            if remaining.is_empty() {
                tx.execute(
                    "DELETE FROM relationships WHERE subject = ?1 OR object = ?1",
                    params![concept],
                )?;
                tx.execute("DELETE FROM entities_fts WHERE id = ?1", params![concept])?;
                tx.execute("DELETE FROM entities WHERE id = ?1", params![concept])?;
            } else {
                tx.execute(
                    "UPDATE entities SET sources = ?1 WHERE id = ?2",
                    params![serde_json::to_string(&remaining)?, concept],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Remove all indexed facts (used by full reindex).
    pub fn purge_all(&self) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for table in [
            "symbols",
            "entities",
            "relationships",
            "evidence",
            "components",
            "flows",
            "invariants",
            "tests",
            "context_cache",
            "drift_findings",
        ] {
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        tx.execute("DELETE FROM files", [])?;
        tx.execute("DELETE FROM snapshots", [])?;
        tx.commit()?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // symbols
    // ------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn insert_symbol(
        &self,
        file: &str,
        name: &str,
        kind: &str,
        signature: Option<&str>,
        start_line: u32,
        end_line: u32,
        exported: bool,
        docstring: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO symbols (file, name, symbol_kind, signature, start_line, end_line, exported, docstring)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                file,
                name,
                kind,
                signature,
                start_line as i64,
                end_line as i64,
                exported as i64,
                docstring
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        self.conn.execute(
            "INSERT INTO symbols_fts (name, signature, symbol_kind, file) VALUES (?1, ?2, ?3, ?4)",
            params![name, signature.unwrap_or(""), kind, file],
        )?;
        Ok(id)
    }

    pub fn symbols_in_file(&self, file: &str) -> Result<Vec<(i64, String, String, Option<String>, u32, u32, bool, Option<String>)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, symbol_kind, signature, start_line, end_line, exported, docstring FROM symbols WHERE file = ?1 ORDER BY start_line")?;
        let rows = stmt.query_map(params![file], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, i64>(4)? as u32,
                r.get::<_, i64>(5)? as u32,
                r.get::<_, i64>(6)? != 0,
                r.get::<_, Option<String>>(7)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn symbols_named(&self, name: &str) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT file, symbol_kind, signature FROM symbols WHERE name = ?1")?;
        let rows = stmt.query_map(params![name], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // imports
    // ------------------------------------------------------------------

    pub fn insert_imports(&self, file: &str, imports: &[(String, Vec<(String, String)>, u32, String)]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM imports WHERE file = ?1", params![file])?;
        for (module, names, line, typ) in imports {
            tx.execute(
                "INSERT INTO imports (file, module, names, line, type) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![file, module, serde_json::to_string(names)?, *line as i64, typ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn imports_in_file(&self, file: &str) -> Result<Vec<(String, Vec<(String, String)>, u32, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT module, names, line, type FROM imports WHERE file = ?1 ORDER BY line")?;
        let rows = stmt.query_map(params![file], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)? as u32,
                r.get::<_, String>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (module, names, line, typ) = r?;
            out.push((
                module,
                serde_json::from_str(&names).unwrap_or_default(),
                line,
                typ,
            ));
        }
        Ok(out)
    }

    pub fn all_imports(&self) -> Result<Vec<(String, String, Vec<(String, String)>, u32, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT file, module, names, line, type FROM imports ORDER BY file, line")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)? as u32,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (file, module, names, line, typ) = r?;
            out.push((
                file,
                module,
                serde_json::from_str(&names).unwrap_or_default(),
                line,
                typ,
            ));
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // entities
    // ------------------------------------------------------------------

    pub fn insert_entity(&self, entity: &Entity, sources: &[String]) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO entities (id, kind, name, attributes, evidence, sources)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                entity.id,
                entity.kind,
                entity.name,
                serde_json::to_string(&entity.attributes)?,
                serde_json::to_string(&entity.evidence)?,
                serde_json::to_string(sources)?,
            ],
        )?;
        self.conn.execute(
            "INSERT OR REPLACE INTO entities_fts (id, kind, name, attributes) VALUES (?1, ?2, ?3, ?4)",
            params![
                entity.id,
                entity.kind,
                entity.name,
                serde_json::to_string(&entity.attributes)?
            ],
        )?;
        Ok(())
    }

    pub fn get_entity(&self, id: &str) -> Result<Option<Entity>> {
        let row = self
            .conn
            .query_row(
                "SELECT id, kind, name, attributes, evidence FROM entities WHERE id = ?1",
                params![id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;
        Ok(row.map(|(id, kind, name, attributes, evidence)| Entity {
            id,
            kind,
            name,
            attributes: serde_json::from_str(&attributes).unwrap_or_default(),
            evidence: serde_json::from_str(&evidence).unwrap_or_default(),
        }))
    }

    /// The `sources` provenance list of an entity — the repository-relative
    /// file paths that produced it. Stored separately from [`Entity`]
    /// (which carries no sources field), so this is the only reader.
    // trace:v1 id=impl.scc.store.entity_sources work=WORK-SCC-001 satisfies=REQ-SCC-IR
    pub fn entity_sources(&self, id: &str) -> Result<Vec<String>> {
        let row = self
            .conn
            .query_row(
                "SELECT sources FROM entities WHERE id = ?1",
                params![id],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(row
            .map(|s| serde_json::from_str(&s).unwrap_or_default())
            .unwrap_or_default())
    }

    /// Live OCCURRENCE entities attached to a concept (OCCURS edges),
    /// sorted by id. Concept counts and `sources` provenance are always
    /// derived from these — never from a stored, write-time-mutated counter.
    // trace:v1 id=impl.scc.store.concept_occurrences work=WORK-SCC-001 satisfies=REQ-SCC-IR
    pub fn concept_occurrences(&self, concept_id: &str) -> Result<Vec<Entity>> {
        let mut stmt = self.conn.prepare(
            "SELECT e.id, e.kind, e.name, e.attributes, e.evidence FROM entities e
             JOIN relationships r ON r.subject = e.id
             WHERE r.predicate = ?1 AND r.object = ?2 AND e.kind = ?3
             ORDER BY e.id",
        )?;
        let rows = stmt.query_map(
            params![
                scc_core::predicates::OCCURS,
                concept_id,
                scc_core::kinds::OCCURRENCE
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            },
        )?;
        let mut out = Vec::new();
        for r in rows {
            let (id, kind, name, attributes, evidence) = r?;
            out.push(Entity {
                id,
                kind,
                name,
                attributes: serde_json::from_str(&attributes).unwrap_or_default(),
                evidence: serde_json::from_str(&evidence).unwrap_or_default(),
            });
        }
        Ok(out)
    }

    pub fn entities_by_kind(&self, kind: &str) -> Result<Vec<Entity>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, kind, name, attributes, evidence FROM entities WHERE kind = ?1 ORDER BY name")?;
        let rows = stmt.query_map(params![kind], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (id, kind, name, attributes, evidence) = r?;
            out.push(Entity {
                id,
                kind,
                name,
                attributes: serde_json::from_str(&attributes).unwrap_or_default(),
                evidence: serde_json::from_str(&evidence).unwrap_or_default(),
            });
        }
        Ok(out)
    }

    pub fn all_entities(&self) -> Result<Vec<Entity>> {
        self.all_entities_impl()
    }

    fn all_entities_impl(&self) -> Result<Vec<Entity>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, kind, name, attributes, evidence FROM entities ORDER BY kind, name")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (id, kind, name, attributes, evidence) = r?;
            out.push(Entity {
                id,
                kind,
                name,
                attributes: serde_json::from_str(&attributes).unwrap_or_default(),
                evidence: serde_json::from_str(&evidence).unwrap_or_default(),
            });
        }
        Ok(out)
    }

    pub fn delete_entity(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM entities WHERE id = ?1", params![id])?;
        self.conn
            .execute("DELETE FROM entities_fts WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn delete_entities(&self, ids: &[String]) -> Result<()> {
        for id in ids {
            self.delete_entity(id)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // relationships
    // ------------------------------------------------------------------

    pub fn insert_relationship(&self, rel: &Relationship, source_path: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO relationships (id, subject, predicate, object, provenance, confidence, evidence, verified_at, source_path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                rel.id,
                rel.subject,
                rel.predicate,
                rel.object,
                rel.provenance.as_str(),
                rel.confidence,
                serde_json::to_string(&rel.evidence)?,
                rel.verified_at,
                source_path,
            ],
        )?;
        Ok(())
    }

    pub fn relationships_for(&self, subject: &str) -> Result<Vec<Relationship>> {
        self.query_relationships("SELECT * FROM relationships WHERE subject = ?1 ORDER BY id", params![subject])
    }

    pub fn relationships_to(&self, object: &str) -> Result<Vec<Relationship>> {
        self.query_relationships("SELECT * FROM relationships WHERE object = ?1 ORDER BY id", params![object])
    }

    pub fn relationships_between(&self, subject: &str, predicate: &str, object: &str) -> Result<Vec<Relationship>> {
        self.query_relationships(
            "SELECT * FROM relationships WHERE subject = ?1 AND predicate = ?2 AND object = ?3",
            params![subject, predicate, object],
        )
    }

    pub fn all_relationships(&self) -> Result<Vec<Relationship>> {
        self.query_relationships("SELECT * FROM relationships ORDER BY id", [])
    }

    /// Relationship ids with the given source path and predicate.
    pub fn relationship_ids_with_source(&self, path: &str, predicate: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM relationships WHERE source_path = ?1 AND predicate = ?2",
        )?;
        let rows = stmt.query_map(params![path, predicate], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn count_relationships(&self) -> Result<u64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM relationships", [], |r| r.get(0))?)
    }

    pub fn delete_relationship(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM relationships WHERE id = ?1", params![id])?;
        Ok(())
    }

    fn query_relationships(
        &self,
        sql: &str,
        p: impl rusqlite::Params,
    ) -> Result<Vec<Relationship>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(p, |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, f64>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, subject, predicate, object, provenance, confidence, evidence, verified_at) =
                row?;
            out.push(Relationship {
                id,
                subject,
                predicate,
                object,
                provenance: parse_provenance(&provenance),
                confidence,
                evidence: serde_json::from_str(&evidence).unwrap_or_default(),
                verified_at,
            });
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // evidence
    // ------------------------------------------------------------------

    pub fn insert_evidence(&self, ev: &Evidence) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO evidence (id, type, path, symbol, start_line, end_line, revision, content_hash, extractor, extractor_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                ev.id,
                evidence_type_str(&ev.r#type),
                ev.path,
                ev.symbol,
                ev.start_line.map(|l| l as i64),
                ev.end_line.map(|l| l as i64),
                ev.revision,
                ev.content_hash,
                ev.extractor,
                ev.extractor_version,
            ],
        )?;
        Ok(())
    }

    pub fn get_evidence(&self, id: &str) -> Result<Option<Evidence>> {
        let row = self
            .conn
            .query_row(
                "SELECT id, type, path, symbol, start_line, end_line, revision, content_hash, extractor, extractor_version FROM evidence WHERE id = ?1",
                params![id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<i64>>(4)?,
                        r.get::<_, Option<i64>>(5)?,
                        r.get::<_, Option<String>>(6)?,
                        r.get::<_, Option<String>>(7)?,
                        r.get::<_, Option<String>>(8)?,
                        r.get::<_, Option<String>>(9)?,
                    ))
                },
            )
            .optional()?;
        Ok(row.map(|(id, typ, path, symbol, sl, el, rev, hash, ext, extv)| Evidence {
            id,
            r#type: parse_evidence_type(&typ),
            path,
            symbol,
            start_line: sl.map(|v| v as u32),
            end_line: el.map(|v| v as u32),
            revision: rev,
            content_hash: hash,
            extractor: ext,
            extractor_version: extv,
        }))
    }

    pub fn all_evidence(&self) -> Result<Vec<Evidence>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, type, path, symbol, start_line, end_line, revision, content_hash, extractor, extractor_version FROM evidence ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, Option<String>>(9)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, typ, path, symbol, sl, el, rev, hash, ext, extv) = row?;
            out.push(Evidence {
                id,
                r#type: parse_evidence_type(&typ),
                path,
                symbol,
                start_line: sl.map(|v| v as u32),
                end_line: el.map(|v| v as u32),
                revision: rev,
                content_hash: hash,
                extractor: ext,
                extractor_version: extv,
            });
        }
        Ok(out)
    }

    pub fn evidence_for_path(&self, path: &str) -> Result<Vec<Evidence>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, type, path, symbol, start_line, end_line, revision, content_hash, extractor, extractor_version FROM evidence WHERE path = ?1 ORDER BY id")?;
        let rows = stmt.query_map(params![path], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, Option<String>>(9)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, typ, path, symbol, sl, el, rev, hash, ext, extv) = row?;
            out.push(Evidence {
                id,
                r#type: parse_evidence_type(&typ),
                path,
                symbol,
                start_line: sl.map(|v| v as u32),
                end_line: el.map(|v| v as u32),
                revision: rev,
                content_hash: hash,
                extractor: ext,
                extractor_version: extv,
            });
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // components / flows / invariants / tests
    // ------------------------------------------------------------------

    pub fn replace_components(&self, components: &[Entity]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM components", [])?;
        for c in components {
            tx.execute(
                "INSERT INTO components (id, name, kind, responsibility, implementation, evidence, attributes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    c.id,
                    c.name,
                    c.kind,
                    serde_json::to_string(&c.attributes.get("responsibility").cloned().unwrap_or(serde_json::json!([])))?,
                    serde_json::to_string(&c.attributes.get("implementation").cloned().unwrap_or(serde_json::json!([])))?,
                    serde_json::to_string(&c.evidence)?,
                    serde_json::to_string(&c.attributes)?,
                ],
            )?;
            tx.execute(
                "INSERT OR REPLACE INTO entities (id, kind, name, attributes, evidence, sources) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![c.id, c.kind, c.name, serde_json::to_string(&c.attributes)?, serde_json::to_string(&c.evidence)?, "[]"],
            )?;
            tx.execute(
                "INSERT OR REPLACE INTO entities_fts (id, kind, name, attributes) VALUES (?1, ?2, ?3, ?4)",
                params![c.id, c.kind, c.name, serde_json::to_string(&c.attributes)?],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn components(&self) -> Result<Vec<Entity>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, kind, responsibility, implementation, evidence, attributes FROM components ORDER BY name")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, name, kind, resp, implm, ev, attrs) = row?;
            let mut attributes: std::collections::BTreeMap<String, serde_json::Value> =
                serde_json::from_str(&attrs).unwrap_or_default();
            if let Ok(r) = serde_json::from_str::<Vec<serde_json::Value>>(&resp) {
                attributes.insert("responsibility".into(), serde_json::Value::Array(r));
            }
            if let Ok(r) = serde_json::from_str::<Vec<serde_json::Value>>(&implm) {
                attributes.insert("implementation".into(), serde_json::Value::Array(r));
            }
            out.push(Entity {
                id,
                kind,
                name,
                attributes,
                evidence: serde_json::from_str(&ev).unwrap_or_default(),
            });
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // canonical flow graphs (Wave 3)
    // ------------------------------------------------------------------

    pub fn replace_flow_graphs(&self, graphs: &[scc_core::FlowGraph]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM flow_graphs", [])?;
        for g in graphs {
            let kind = scc_core::flow_kind_str(&g.kind);
            let trigger = g.trigger.clone().unwrap_or_default();
            tx.execute(
                "INSERT OR REPLACE INTO flow_graphs (id, kind, name, trigger, graph) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![g.id, kind, g.name, trigger, serde_json::to_string(g)?],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn flow_graphs(&self) -> Result<Vec<scc_core::FlowGraph>> {
        let mut stmt = self
            .conn
            .prepare("SELECT graph FROM flow_graphs ORDER BY id")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            let json = r?;
            if let Ok(g) = serde_json::from_str(&json) {
                out.push(g);
            }
        }
        Ok(out)
    }

    pub fn replace_flows(&self, flows: &[Flow]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM flows", [])?;
        for f in flows {
            tx.execute(
                "INSERT INTO flows (id, kind, name, trigger, steps, attributes) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    f.id,
                    flow_kind_str(&f.kind),
                    f.name,
                    f.trigger,
                    serde_json::to_string(&f.steps)?,
                    serde_json::to_string(&f.attributes)?
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn flows(&self) -> Result<Vec<Flow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, kind, name, trigger, steps, attributes FROM flows ORDER BY kind, name")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, kind, name, trigger, steps, attrs) = row?;
            out.push(Flow {
                id,
                kind: parse_flow_kind(&kind),
                name,
                trigger,
                steps: serde_json::from_str(&steps).unwrap_or_default(),
                attributes: serde_json::from_str(&attrs).unwrap_or_default(),
            });
        }
        Ok(out)
    }

    pub fn flow(&self, id: &str) -> Result<Option<Flow>> {
        let row = self
            .conn
            .query_row(
                "SELECT id, kind, name, trigger, steps, attributes FROM flows WHERE id = ?1",
                params![id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()?;
        Ok(row.map(|(id, kind, name, trigger, steps, attrs)| Flow {
            id,
            kind: parse_flow_kind(&kind),
            name,
            trigger,
            steps: serde_json::from_str(&steps).unwrap_or_default(),
            attributes: serde_json::from_str(&attrs).unwrap_or_default(),
        }))
    }

    pub fn replace_invariants(&self, invariants: &[Invariant]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM invariants", [])?;
        for inv in invariants {
            tx.execute(
                "INSERT INTO invariants (id, statement, severity, scope, enforced_by, provenance, evidence)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    inv.id,
                    inv.statement,
                    severity_str(&inv.severity),
                    serde_json::to_string(&inv.scope)?,
                    serde_json::to_string(&inv.enforced_by)?,
                    inv.provenance.map(|p| p.as_str().to_string()).unwrap_or_else(|| "DECLARED".into()),
                    serde_json::to_string(&inv.evidence)?,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn invariants(&self) -> Result<Vec<Invariant>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, statement, severity, scope, enforced_by, provenance, evidence FROM invariants ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, statement, severity, scope, enforced_by, provenance, ev) = row?;
            out.push(Invariant {
                id,
                statement,
                severity: parse_severity(&severity),
                scope: serde_json::from_str(&scope).unwrap_or_default(),
                enforced_by: serde_json::from_str(&enforced_by).unwrap_or_default(),
                provenance: Some(parse_provenance(&provenance)),
                evidence: serde_json::from_str(&ev).unwrap_or_default(),
            });
        }
        Ok(out)
    }

    pub fn insert_test(&self, id: &str, name: &str, file: &str, kind: &str, symbol: Option<&str>) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO tests (id, name, file, kind, symbol) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, name, file, kind, symbol],
        )?;
        Ok(())
    }

    pub fn tests(&self) -> Result<Vec<(String, String, String, String, Option<String>)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, file, kind, symbol FROM tests ORDER BY file, name")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Delete evidence records no longer referenced by any entity,
    /// relationship, component, invariant, flow step, or flow-graph node/
    /// edge. Run after the derived layer (components/flows) rebuilds,
    /// because derived edges may briefly hold references during
    /// recompilation.
    pub fn sweep_orphan_evidence(&self) -> Result<u64> {
        // Set-based pass: collect every referenced evidence id ONCE (exact
        // id membership in the JSON arrays, no per-row LIKE scans), then
        // delete evidence rows whose id is not referenced anywhere.
        let mut referenced: HashSet<String> = HashSet::new();

        // Columns holding JSON arrays of evidence ids.
        for (table, column) in [
            ("entities", "evidence"),
            ("relationships", "evidence"),
            ("components", "evidence"),
            ("invariants", "evidence"),
        ] {
            let sql = format!("SELECT {column} FROM {table}");
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            for r in rows {
                if let Ok(ids) = serde_json::from_str::<Vec<String>>(&r?) {
                    referenced.extend(ids);
                }
            }
            drop(stmt);
        }

        // flows.steps: array of FlowStep objects, each with an evidence list.
        {
            let mut stmt = self.conn.prepare("SELECT steps FROM flows")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            for r in rows {
                if let Ok(steps) = serde_json::from_str::<Vec<scc_core::FlowStep>>(&r?) {
                    for step in steps {
                        referenced.extend(step.evidence);
                    }
                }
            }
            drop(stmt);
        }

        // flow_graphs.graph: nodes[].evidence and edges[].evidence arrays.
        {
            let mut stmt = self.conn.prepare("SELECT graph FROM flow_graphs")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            for r in rows {
                if let Ok(g) = serde_json::from_str::<scc_core::FlowGraph>(&r?) {
                    for node in &g.nodes {
                        referenced.extend(node.evidence.iter().cloned());
                    }
                    for edge in &g.edges {
                        referenced.extend(edge.evidence.iter().cloned());
                    }
                }
            }
            drop(stmt);
        }

        let mut stmt = self.conn.prepare("SELECT id FROM evidence")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut orphans: Vec<String> = Vec::new();
        for r in rows {
            let id = r?;
            if !referenced.contains(&id) {
                orphans.push(id);
            }
        }
        drop(stmt);

        let count = orphans.len() as u64;
        // Chunked deletes avoid one giant IN list.
        for chunk in orphans.chunks(500) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!("DELETE FROM evidence WHERE id IN ({placeholders})");
            self.conn
                .execute(&sql, rusqlite::params_from_iter(chunk.iter()))?;
        }
        Ok(count)
    }

    // ------------------------------------------------------------------
    // context cache
    // ------------------------------------------------------------------

    /// Canonical epoch string for cache keys: composite of every model
    /// generation plus the latest snapshot revision. A previously fresh
    /// pack can never be served after any source of system truth changed.
    pub fn cache_epoch(&self) -> Result<String> {
        let revision = self
            .latest_snapshot()?
            .map(|s| s.revision)
            .unwrap_or_else(|| "not-indexed".to_string());
        Ok(self.model_epoch()?.composite(&revision))
    }

    pub fn cache_get(&self, key: &str, epoch: &str) -> Result<Option<String>> {
        let row = self
            .conn
            .query_row(
                "SELECT pack FROM context_cache WHERE key = ?1 AND epoch = ?2",
                params![key, epoch],
                |r| r.get(0),
            )
            .optional()?;
        Ok(row)
    }

    pub fn cache_put(&self, key: &str, pack: &str, epoch: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO context_cache (key, pack, epoch, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![key, pack, epoch, scc_core::now_rfc3339()],
        )?;
        Ok(())
    }

    pub fn cache_clear(&self) -> Result<()> {
        self.conn.execute("DELETE FROM context_cache", [])?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // intent claims / drift findings
    // ------------------------------------------------------------------

    pub fn replace_intent_claims(&self, claims: &[(String, serde_json::Value)]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM intent_claims", [])?;
        for (source, claim) in claims {
            tx.execute(
                "INSERT INTO intent_claims (source, claim, created_at) VALUES (?1, ?2, ?3)",
                params![source, serde_json::to_string(claim)?, scc_core::now_rfc3339()],
            )?;
        }
        tx.commit()?;
        // declared intent changed — invalidate epoch-keyed packs
        self.bump_epoch(ModelEpochKind::Intent)?;
        Ok(())
    }

    pub fn intent_claims(&self) -> Result<Vec<(String, serde_json::Value)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT source, claim FROM intent_claims ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (source, claim) = r?;
            if let Ok(v) = serde_json::from_str(&claim) {
                out.push((source, v));
            }
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // runtime edges
    // ------------------------------------------------------------------

    pub fn runtime_edge_rows(&self) -> Result<Vec<RuntimeEdgeRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT source, target, count, latency_ms, errors, last_observed
             FROM runtime_edges ORDER BY source, target",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(RuntimeEdgeRow {
                source: r.get(0)?,
                target: r.get(1)?,
                count: r.get::<_, i64>(2)? as u64,
                latency_ms: r.get(3)?,
                errors: r.get::<_, i64>(4)? as u64,
                last_observed: r.get(5)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // trace signatures (Wave 6)
    // ------------------------------------------------------------------

    /// Record one occurrence of an observed trace-path signature. `count`
    /// increments by one (one trace occurrence), `latency_ms` is merged as a
    /// count-weighted running average, `errors` is additive. Does NOT bump
    /// any model-epoch generation: ingestion callers bump the Runtime
    /// generation once per payload.
    pub fn upsert_trace_signature(&self, signature: &str, latency_ms: f64, errors: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO trace_signatures (signature, count, latency_ms, errors, last_observed)
             VALUES (?1, 1, ?2, ?3, ?4)
             ON CONFLICT(signature) DO UPDATE SET
               count = trace_signatures.count + 1,
               latency_ms = (trace_signatures.latency_ms * trace_signatures.count + excluded.latency_ms)
                            / (trace_signatures.count + 1),
               errors = trace_signatures.errors + excluded.errors,
               last_observed = excluded.last_observed",
            params![signature, latency_ms, errors as i64, scc_core::now_rfc3339()],
        )?;
        Ok(())
    }

    /// All observed trace signatures, ordered by (count DESC, signature).
    pub fn trace_signatures(&self) -> Result<Vec<(String, u64, f64, u64, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT signature, count, latency_ms, errors, last_observed
             FROM trace_signatures ORDER BY count DESC, signature",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)? as u64,
                r.get::<_, f64>(2)?,
                r.get::<_, i64>(3)? as u64,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // embeddings
    // ------------------------------------------------------------------

    pub fn put_embedding(&self, entity_id: &str, vector: &[f32], model: &str) -> Result<()> {
        let bytes: Vec<u8> = vector
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        self.conn.execute(
            "INSERT OR REPLACE INTO embeddings (entity_id, vector, model, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![entity_id, bytes, model, scc_core::now_rfc3339()],
        )?;
        Ok(())
    }

    pub fn get_embedding(&self, entity_id: &str) -> Result<Option<(Vec<f32>, String)>> {
        let row = self
            .conn
            .query_row(
                "SELECT vector, model FROM embeddings WHERE entity_id = ?1",
                params![entity_id],
                |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?;
        Ok(row.map(|(bytes, model)| {
            let v = bytes
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            (v, model)
        }))
    }

    pub fn embedding_count(&self) -> Result<u64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM embeddings", [], |r| r.get(0))?)
    }

    pub fn add_drift_finding(&self, kind: &str, severity: &str, message: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO drift_findings (kind, severity, message, created_at, resolved) VALUES (?1, ?2, ?3, ?4, 0)",
            params![kind, severity, message, scc_core::now_rfc3339()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn drift_findings(&self, unresolved_only: bool) -> Result<Vec<(i64, String, String, String, String)>> {
        let sql = if unresolved_only {
            "SELECT id, kind, severity, message, created_at FROM drift_findings WHERE resolved = 0 ORDER BY id"
        } else {
            "SELECT id, kind, severity, message, created_at FROM drift_findings ORDER BY id"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn clear_drift_findings(&self) -> Result<()> {
        self.conn.execute("DELETE FROM drift_findings", [])?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // search (FTS5)
    // ------------------------------------------------------------------

    /// Lexical search over entities (components, routes, data, stores...).
    pub fn search_entities(&self, query: &str, limit: usize) -> Result<Vec<Entity>> {
        let q = fts_query(query);
        let mut stmt = self.conn.prepare(
            "SELECT e.id, e.kind, e.name, e.attributes, e.evidence
             FROM entities_fts f JOIN entities e ON e.id = f.id
             WHERE entities_fts MATCH ?1 ORDER BY bm25(entities_fts) LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![q, limit as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (id, kind, name, attributes, evidence) = r?;
            out.push(Entity {
                id,
                kind,
                name,
                attributes: serde_json::from_str(&attributes).unwrap_or_default(),
                evidence: serde_json::from_str(&evidence).unwrap_or_default(),
            });
        }
        Ok(out)
    }

    /// Lexical search over symbols.
    pub fn search_symbols(&self, query: &str, limit: usize) -> Result<Vec<(String, String, String, String)>> {
        let q = fts_query(query);
        let mut stmt = self.conn.prepare(
            "SELECT name, signature, symbol_kind, file FROM symbols_fts
             WHERE symbols_fts MATCH ?1 ORDER BY bm25(symbols_fts) LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![q, limit as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Substring fallback over entity names AND attributes (docstrings,
    /// signatures, responsibilities) — case-insensitive. Used when FTS prefix
    /// matching misses morphological variants or multi-term AND queries drop
    /// otherwise-strong matches.
    pub fn search_entities_like(&self, term: &str, limit: usize) -> Result<Vec<Entity>> {
        let pat = format!("%{}%", term.to_ascii_lowercase());
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, name, attributes, evidence FROM entities
             WHERE lower(name) LIKE ?1 OR lower(attributes) LIKE ?1
             ORDER BY CASE WHEN lower(name) LIKE ?1 THEN 0 ELSE 1 END, length(name)
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pat, limit as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (id, kind, name, attributes, evidence) = r?;
            out.push(Entity {
                id,
                kind,
                name,
                attributes: serde_json::from_str(&attributes).unwrap_or_default(),
                evidence: serde_json::from_str(&evidence).unwrap_or_default(),
            });
        }
        Ok(out)
    }

    /// Substring fallback over symbols (name, signature, docstring).
    pub fn search_symbols_like(&self, term: &str, limit: usize) -> Result<Vec<(String, String, String, String)>> {
        let pat = format!("%{}%", term.to_ascii_lowercase());
        let mut stmt = self.conn.prepare(
            "SELECT name, signature, symbol_kind, file FROM symbols
             WHERE lower(name) LIKE ?1 OR lower(signature) LIKE ?1 OR lower(docstring) LIKE ?1
             ORDER BY CASE WHEN lower(name) LIKE ?1 THEN 0 ELSE 1 END, length(name)
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pat, limit as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // stats
    // ------------------------------------------------------------------

    pub fn stats(&self) -> Result<HashMap<String, u64>> {
        let mut m = HashMap::new();
        for (name, sql) in [
            ("files", "SELECT COUNT(*) FROM files"),
            ("symbols", "SELECT COUNT(*) FROM symbols"),
            ("entities", "SELECT COUNT(*) FROM entities"),
            ("relationships", "SELECT COUNT(*) FROM relationships"),
            ("evidence", "SELECT COUNT(*) FROM evidence"),
            ("components", "SELECT COUNT(*) FROM components"),
            ("flows", "SELECT COUNT(*) FROM flows"),
            ("invariants", "SELECT COUNT(*) FROM invariants"),
            ("tests", "SELECT COUNT(*) FROM tests"),
        ] {
            let n: i64 = self.conn.query_row(sql, [], |r| r.get(0))?;
            m.insert(name.to_string(), n as u64);
        }
        Ok(m)
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn apply_migrations(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let current = SCHEMA_VERSION as i64;
    if version > current {
        return Err(StoreError::Sqlite(rusqlite::Error::InvalidParameterName(
            format!(
                "database schema v{version} is newer than supported v{SCHEMA_VERSION}"
            )
            .into_boxed_str()
            .to_string(),
        )));
    }
    // Apply every migration after the current version, in order.
    for (i, m) in MIGRATIONS.iter().enumerate() {
        let target = (i + 1) as i64;
        if version < target {
            conn.execute_batch(m)?;
        }
    }
    if version < current {
        conn.pragma_update(None, "user_version", current)?;
    }
    Ok(())
}

pub fn parse_provenance(s: &str) -> Provenance {
    match s {
        "EXTRACTED" => Provenance::Extracted,
        "RESOLVED" => Provenance::Resolved,
        "OBSERVED" => Provenance::Observed,
        "DECLARED" => Provenance::Declared,
        "INFERRED" => Provenance::Inferred,
        "STALE" => Provenance::Stale,
        _ => Provenance::Inferred,
    }
}

pub fn evidence_type_str(t: &scc_core::EvidenceType) -> &'static str {
    match t {
        scc_core::EvidenceType::Source => "source",
        scc_core::EvidenceType::Config => "config",
        scc_core::EvidenceType::Runtime => "runtime",
        scc_core::EvidenceType::Test => "test",
        scc_core::EvidenceType::Intent => "intent",
        scc_core::EvidenceType::History => "history",
    }
}

pub fn parse_evidence_type(s: &str) -> scc_core::EvidenceType {
    match s {
        "source" => scc_core::EvidenceType::Source,
        "config" => scc_core::EvidenceType::Config,
        "runtime" => scc_core::EvidenceType::Runtime,
        "test" => scc_core::EvidenceType::Test,
        "intent" => scc_core::EvidenceType::Intent,
        "history" => scc_core::EvidenceType::History,
        _ => scc_core::EvidenceType::Source,
    }
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

pub fn parse_flow_kind(s: &str) -> scc_core::FlowKind {
    match s {
        "architecture" => scc_core::FlowKind::Architecture,
        "workflow" => scc_core::FlowKind::Workflow,
        "sequence" => scc_core::FlowKind::Sequence,
        "dataflow" => scc_core::FlowKind::Dataflow,
        "lifecycle" => scc_core::FlowKind::Lifecycle,
        _ => scc_core::FlowKind::Sequence,
    }
}

pub fn severity_str(s: &Severity) -> &'static str {
    match s {
        Severity::Info => "info",
        Severity::Low => "low",
        Severity::Medium => "medium",
        Severity::High => "high",
        Severity::Critical => "critical",
    }
}

pub fn parse_severity(s: &str) -> Severity {
    match s {
        "info" => Severity::Info,
        "low" => Severity::Low,
        "medium" => Severity::Medium,
        "high" => Severity::High,
        "critical" => Severity::Critical,
        _ => Severity::Medium,
    }
}

/// Build a safe FTS5 MATCH expression from free-form text: quoted terms with
/// prefix matching on the last term.
fn fts_query(text: &str) -> String {
    let tokens: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-' && c != '.')
        .filter(|t| !t.is_empty())
        .map(|t| {
            let t = t.trim_matches('"');
            format!("\"{t}\"*")
        })
        .collect();
    if tokens.is_empty() {
        return "\"\"".to_string();
    }
    tokens.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    pub(crate) fn tmp_store() -> (Store, TempDir) {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        (store, dir)
    }

    #[test]
    fn migrations_apply_and_reopen() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let db = dir.path().join("scc.db");
        {
            let s = Store::open(&db, &root).unwrap();
            s.insert_entity(&Entity::new("repo://t/component/a", "component", "A"), &["x.py".into()]).unwrap();
        }
        // reopen
        let s = Store::open(&db, &root).unwrap();
        assert!(s.get_entity("repo://t/component/a").unwrap().is_some());
    }

    #[test]
    fn model_epoch_bumps_and_composites() {
        let (s, _d) = tmp_store();
        assert_eq!(s.model_epoch().unwrap(), ModelEpoch::zero());
        let e0 = s.cache_epoch().unwrap();

        // snapshot completion bumps the source generation
        let id = s.begin_snapshot("abc", None).unwrap();
        s.finish_snapshot(id, 3).unwrap();
        let e1 = s.model_epoch().unwrap();
        assert_eq!(e1.source, 1);
        assert_ne!(s.cache_epoch().unwrap(), e0);

        // semantic promotion is an independent generation
        s.bump_epoch(ModelEpochKind::Semantic).unwrap();
        let e2 = s.model_epoch().unwrap();
        assert_eq!(e2.semantic, 1);
        assert_eq!(e2.source, 1);

        // composite differs per generation combination and includes the
        // snapshot revision
        let c1 = e1.composite("abc");
        let c2 = e2.composite("abc");
        assert_ne!(c1, c2);
        assert_ne!(c1, e1.composite("def"));
        // same state -> same composite (deterministic)
        assert_eq!(c1, e1.composite("abc"));
    }

    #[test]
    fn cache_is_keyed_on_epoch() {
        let (s, _d) = tmp_store();
        let e0 = s.cache_epoch().unwrap();
        s.cache_put("task:1", "pack-A", &e0).unwrap();
        assert_eq!(s.cache_get("task:1", &e0).unwrap().as_deref(), Some("pack-A"));

        // any truth change invalidates the old epoch key
        s.bump_epoch(ModelEpochKind::Runtime).unwrap();
        let e1 = s.cache_epoch().unwrap();
        assert_ne!(e0, e1);
        assert_eq!(s.cache_get("task:1", &e1).unwrap(), None);
        // old packs remain addressable only by their own epoch
        assert_eq!(s.cache_get("task:1", &e0).unwrap().as_deref(), Some("pack-A"));
    }

    #[test]
    fn intent_replacement_bumps_intent_epoch() {
        let (s, _d) = tmp_store();
        let before = s.model_epoch().unwrap().intent;
        s.replace_intent_claims(&[("component".into(), serde_json::json!({"name": "a"}))])
            .unwrap();
        let after = s.model_epoch().unwrap();
        assert_eq!(after.intent, before + 1);
    }

    #[test]
    fn trace_signature_upsert_roundtrip_and_epoch_neutral() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let db = dir.path().join("scc.db");
        {
            let s = Store::open(&db, &root).unwrap();
            assert!(s.trace_signatures().unwrap().is_empty());

            // upserts must not bump any model-epoch generation (the
            // ingestion layer bumps the Runtime generation once per payload)
            let epoch_before = s.model_epoch().unwrap();
            s.upsert_trace_signature("root -> api -> db", 4.5, 1).unwrap();
            s.upsert_trace_signature("root -> api -> db", 5.5, 0).unwrap();
            s.upsert_trace_signature("root -> web -> api", 2.0, 0).unwrap();
            assert_eq!(s.model_epoch().unwrap(), epoch_before);

            let sigs = s.trace_signatures().unwrap();
            assert_eq!(sigs.len(), 2);
            // count increments, latency is a count-weighted running average
            // ((4.5 + 5.5) / 2), errors are additive
            assert_eq!(sigs[0].0, "root -> api -> db");
            assert_eq!(sigs[0].1, 2);
            assert!((sigs[0].2 - 5.0).abs() < 1e-9);
            assert_eq!(sigs[0].3, 1);
            assert!(!sigs[0].4.is_empty());
            // ordering: (count DESC, signature)
            assert_eq!(sigs[1].0, "root -> web -> api");
            assert_eq!(sigs[1].1, 1);
        }
        // schema v6 + rows survive reopen
        let s = Store::open(&db, &root).unwrap();
        let sigs = s.trace_signatures().unwrap();
        assert_eq!(sigs.len(), 2);
        assert_eq!(sigs[0].0, "root -> api -> db");
        assert_eq!(sigs[0].1, 2);
    }

    #[test]
    fn entity_roundtrip_and_fts() {
        let (s, _d) = tmp_store();
        let mut e = Entity::new("repo://r/component/transcript", "component", "transcript-normalizer");
        e.attr("responsibility", serde_json::json!(["normalize transcripts"]));
        s.insert_entity(&e, &["src/normalize.py".into()]).unwrap();
        let got = s.get_entity("repo://r/component/transcript").unwrap().unwrap();
        assert_eq!(got.name, "transcript-normalizer");
        let hits = s.search_entities("normalize", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "repo://r/component/transcript");
    }

    #[test]
    fn relationship_roundtrip() {
        let (s, _d) = tmp_store();
        let rel = Relationship::new(
            "rel:1",
            "repo://r/component/a",
            "calls",
            "repo://r/component/b",
            Provenance::Resolved,
        )
        .with_evidence(vec!["evidence:1".into()]);
        s.insert_relationship(&rel, "src/a.py").unwrap();
        let got = s.relationships_for("repo://r/component/a").unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].predicate, "calls");
        assert_eq!(got[0].provenance, Provenance::Resolved);
    }

    #[test]
    fn sweep_orphan_evidence_removes_only_unreferenced() {
        let (s, _d) = tmp_store();
        // three evidence rows: two referenced (entity, relationship), one not
        s.insert_evidence(&Evidence::source("evidence:ent", "src/a.py")).unwrap();
        s.insert_evidence(&Evidence::source("evidence:rel", "src/b.py")).unwrap();
        s.insert_evidence(&Evidence::source("evidence:orphan", "src/c.py")).unwrap();

        let mut e = Entity::new("repo://r/component/keep", "component", "keep");
        e.evidence = vec!["evidence:ent".into()];
        s.insert_entity(&e, &["src/a.py".into()]).unwrap();
        let rel = Relationship::new(
            "rel:1",
            "repo://r/component/keep",
            "calls",
            "repo://r/component/other",
            Provenance::Resolved,
        )
        .with_evidence(vec!["evidence:rel".into()]);
        s.insert_relationship(&rel, "src/b.py").unwrap();

        // sweep: the unreferenced row goes, the referenced rows stay
        assert_eq!(s.sweep_orphan_evidence().unwrap(), 1);
        assert!(s.get_evidence("evidence:ent").unwrap().is_some());
        assert!(s.get_evidence("evidence:rel").unwrap().is_some());
        assert!(s.get_evidence("evidence:orphan").unwrap().is_none());

        // drop the entity's reference, sweep again: now it becomes an orphan
        let e2 = Entity::new("repo://r/component/keep", "component", "keep");
        s.insert_entity(&e2, &["src/a.py".into()]).unwrap();
        assert_eq!(s.sweep_orphan_evidence().unwrap(), 1);
        assert!(s.get_evidence("evidence:ent").unwrap().is_none());
        assert!(s.get_evidence("evidence:rel").unwrap().is_some());
    }

    #[test]
    fn sweep_orphan_evidence_keeps_flow_and_component_references() {
        let (s, _d) = tmp_store();
        // derived-layer references (flow step + component) must protect
        // evidence even though entities/relationships no longer mention it
        s.insert_evidence(&Evidence::source("evidence:flow", "src/a.py")).unwrap();
        s.insert_evidence(&Evidence::source("evidence:comp", "src/b.py")).unwrap();
        s.insert_evidence(&Evidence::source("evidence:gone", "src/c.py")).unwrap();

        // component referencing evidence:comp
        let mut comp = Entity::new("repo://r/component/keep", "component", "keep");
        comp.evidence = vec!["evidence:comp".into()];
        s.replace_components(&[comp]).unwrap();

        // flow step referencing evidence:flow
        let flow = scc_core::Flow {
            id: "flow:1".into(),
            kind: scc_core::FlowKind::Workflow,
            name: "wf".into(),
            trigger: None,
            steps: vec![scc_core::FlowStep {
                id: "step:1".into(),
                order: 0,
                actor: "repo://r/component/keep".into(),
                operation: "run".into(),
                condition: None,
                r#async: None,
                timeout_ms: None,
                retry_policy: None,
                failure_outcome: None,
                provenance: None,
                evidence: vec!["evidence:flow".into()],
            }],
            attributes: Default::default(),
        };
        s.replace_flows(&[flow]).unwrap();

        assert_eq!(s.sweep_orphan_evidence().unwrap(), 1);
        assert!(s.get_evidence("evidence:flow").unwrap().is_some());
        assert!(s.get_evidence("evidence:comp").unwrap().is_some());
        assert!(s.get_evidence("evidence:gone").unwrap().is_none());
    }

    #[test]
    fn purge_path_cascades() {
        let (s, _d) = tmp_store();
        s.insert_symbol("a.py", "foo", "function", None, 1, 5, true, None).unwrap();
        s.insert_evidence(&Evidence::source("evidence:1", "a.py")).unwrap();
        let rel = Relationship::new("rel:1", "repo://r/symbol/a.py/foo", "calls", "repo://r/symbol/b.py/bar", Provenance::Extracted)
            .with_evidence(vec!["evidence:1".into()]);
        s.insert_relationship(&rel, "a.py").unwrap();
        s.purge_path("a.py").unwrap();
        assert_eq!(s.symbols_in_file("a.py").unwrap().len(), 0);
        assert_eq!(s.all_relationships().unwrap().len(), 0);
        assert!(s.get_evidence("evidence:1").unwrap().is_none());
    }

// trace:exempt reason=internal-detail

    /// Wave 13: shared concepts survive a single-file purge. The concept's
    /// `sources` provenance is recomputed from the surviving occurrences
    /// (never a stored counter), and the concept is deleted only when its
    /// last occurrence is purged.
    #[test]
// trace:exempt reason=internal-detail
    fn purge_path_recomputes_shared_concept_provenance() {
        let (s, _d) = tmp_store();
        let repo = &s.repo_id;
        let expr = "z.object({ name: z.string() })";
        let concept = scc_core::entity_id(repo, scc_core::kinds::SCHEMA, expr);
        // one concept, two occurrences (A.ts / B.ts)
        s.insert_entity(
            &Entity::new(concept.clone(), scc_core::kinds::SCHEMA, expr),
            &["a.ts".into(), "b.ts".into()],
        )
        .unwrap();
        for (path, owner) in [("a.ts", "makeA"), ("b.ts", "makeB")] {
            let occ = scc_core::occurrence_id(repo, expr, path, owner, 3);
            s.insert_entity(
                Entity::new(occ.clone(), scc_core::kinds::OCCURRENCE, format!("{expr}@{path}@{owner}@3"))
                    .attr("concept", serde_json::json!(concept))
                    .attr("path", serde_json::json!(path))
                    .attr("owner", serde_json::json!(owner))
                    .attr("line", serde_json::json!(3)),
                &[path.to_string()],
            )
            .unwrap();
            s.insert_relationship(
                &Relationship::new(
                    format!("rel:occ:{path}"),
                    occ.clone(),
                    scc_core::predicates::OCCURS,
                    concept.clone(),
                    Provenance::Extracted,
                ),
                path,
            )
            .unwrap();
        }
        // purge A: concept survives, count 1, provenance recomputed to B
        s.purge_path("a.ts").unwrap();
        let concept_ent = s
            .get_entity(&concept)
            .unwrap()
            .expect("concept survives a single-file purge");
        assert_eq!(concept_ent.kind, scc_core::kinds::SCHEMA);
        assert_eq!(s.entity_sources(&concept).unwrap(), vec!["b.ts"]);
        let occs = s.concept_occurrences(&concept).unwrap();
        assert_eq!(occs.len(), 1, "derived count drops to 1: {occs:?}");
        assert_eq!(
            occs[0].attributes.get("path").and_then(|v| v.as_str()),
            Some("b.ts")
        );
        // purge B: last occurrence gone -> concept deleted with its edges
        s.purge_path("b.ts").unwrap();
        assert!(s.get_entity(&concept).unwrap().is_none(), "concept gone");
        assert!(s.all_relationships().unwrap().is_empty(), "no dangling edges");
        // unrelated entities with the path in sources still purge
        s.insert_entity(
            &Entity::new("repo://r/store/db", "store", "db"),
            &["a.ts".into()],
        )
        .unwrap();
        s.purge_path("a.ts").unwrap();
        assert!(s.get_entity("repo://r/store/db").unwrap().is_none());
    }

    #[test]
    fn cache_revision_scoped() {
        let (s, _d) = tmp_store();
        s.cache_put("k", "pack-v1", "rev1").unwrap();
        assert_eq!(s.cache_get("k", "rev1").unwrap(), Some("pack-v1".into()));
        assert_eq!(s.cache_get("k", "rev2").unwrap(), None);
    }

    #[test]
    fn fts_escapes_punctuation() {
        let (s, _d) = tmp_store();
        let mut e = Entity::new("repo://r/route/api", "route", "GET /api/v1/items");
        e.attr("path", serde_json::json!("/api/v1/items"));
        s.insert_entity(&e, &["app.py".into()]).unwrap();
        let hits = s.search_entities("GET /api/v1/items", 10).unwrap();
        assert_eq!(hits.len(), 1);
    }
}

#[cfg(test)]
mod like_tests {
    use super::*;
    use crate::tests::tmp_store;

    #[test]
    fn entities_like_matches_attributes() {
        let (s, _d) = tmp_store();
        let mut e = Entity::new("repo://r/symbol/a.py/f", "symbol", "transcribe");
        e.attr("docstring", serde_json::json!("External ASR client with retry and fallback."));
        s.insert_entity(&e, &["a.py".into()]).unwrap();
        let hits = s.search_entities_like("retry", 6).unwrap();
        assert_eq!(hits.len(), 1, "must match docstring attributes");
        let hits2 = s.search_symbols_like("retry", 6).unwrap();
        assert_eq!(hits2.len(), 0);
    }

    #[test]
    fn symbols_like_matches_docstring() {
        let (s, _d) = tmp_store();
        s.insert_symbol("a.py", "transcribe", "function", None, 1, 2, true,
                        Some("External ASR client with retry and fallback.")).unwrap();
        let hits = s.search_symbols_like("retry", 6).unwrap();
        assert_eq!(hits.len(), 1, "must match symbol docstrings");
        assert_eq!(hits[0].0, "transcribe");
    }
}
