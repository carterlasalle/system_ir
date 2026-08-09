//! SQLite persistence for the System Context Compiler.
//!
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
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const SCHEMA_VERSION: u32 = 2;
pub const FTS_ESCAPE: &str = "\"";
const MIGRATIONS: &[&str] = &[MIGRATION_1, MIGRATION_2];

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

pub struct Store {
    pub conn: Connection,
    pub root: PathBuf,
    /// Repository id (repo:// id component).
    pub repo_id: String,
    pub repo_name: String,
}

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

    /// Remove everything tied to a source path: symbols, evidence,
    /// relationships derived from it, and symbol-level entities.
    pub fn purge_path(&self, path: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
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
        tx.execute(
            "DELETE FROM entities WHERE sources LIKE ?1",
            params![format!("%\"{path}\"%")],
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

    /// Delete evidence records no longer referenced by any entity or
    /// relationship. Run after the derived layer (components/flows) rebuilds,
    /// because derived edges may briefly hold references during recompilation.
    pub fn sweep_orphan_evidence(&self) -> Result<u64> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM evidence e WHERE NOT EXISTS (
                 SELECT 1 FROM entities x WHERE x.evidence LIKE '%' || e.id || '%'
             ) AND NOT EXISTS (
                 SELECT 1 FROM relationships y WHERE y.evidence LIKE '%' || e.id || '%'
             )",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut ids: Vec<String> = Vec::new();
        for r in rows {
            ids.push(r?);
        }
        drop(stmt);
        let count = ids.len() as u64;
        for id in ids {
            self.conn
                .execute("DELETE FROM evidence WHERE id = ?1", params![id])?;
        }
        Ok(count)
    }

    // ------------------------------------------------------------------
    // context cache
    // ------------------------------------------------------------------

    pub fn cache_get(&self, key: &str, revision: &str) -> Result<Option<String>> {
        let row = self
            .conn
            .query_row(
                "SELECT pack FROM context_cache WHERE key = ?1 AND revision = ?2",
                params![key, revision],
                |r| r.get(0),
            )
            .optional()?;
        Ok(row)
    }

    pub fn cache_put(&self, key: &str, pack: &str, revision: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO context_cache (key, pack, revision, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![key, pack, revision, scc_core::now_rfc3339()],
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
