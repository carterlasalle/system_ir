//! Versioned reality graph (mission §XIX): durable per-index revisions with
//! source-hash/extractor provenance, introduction/removal history, and
//! transactional updates — all in SQLite, no extra infrastructure.
//!
//! Model: every successful index records one [`GraphRevision`] plus the full
//! member id + row sets (`revision_members`). History is a chain of complete
//! snapshots (simple and correct; storage cost is one row-set per index —
//! fine for normal repos, noted for very large ones). Diffs replay sets,
//! never the live tables, so a historical view is stable under later edits.
//!
//! Complementary to [`ModelEpoch`](crate::ModelEpoch): a revision is a
//! persistent history position; the epoch is the identity of the active
//! compiled view. See docs/DATA_STRATEGY.md L5.

use crate::{Entity, Relationship, Store, StoreError};
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// One durable graph revision: what source, what extractor, what changed.
#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:v1 id=impl.crates-scc-store-src-history.graph-revision work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct GraphRevision {
    /// Monotonic history position (1-based).
    pub rev: i64,
    /// Previous revision, 0 for genesis.
    pub base_rev: i64,
    /// When this revision was recorded (RFC3339).
    pub created_at: String,
    /// Content hash of the indexed file inventory (path+hash list).
    pub source_hash: String,
    /// Extractor/schema versions that produced this revision.
    pub extractor_version: String,
    /// Live counts at record time.
    pub entity_count: u64,
    /// Live counts at record time.
    pub rel_count: u64,
    /// Live counts at record time.
    pub file_count: u64,
}

/// Introduction/removal delta between two revisions, grouped by id.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
// trace:v1 id=impl.crates-scc-store-src-history.semantic-delta work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct SemanticDelta {
    /// Entity ids present in `to` but not `from`.
    pub added_entities: Vec<String>,
    /// Entity ids present in `from` but not `to`.
    pub removed_entities: Vec<String>,
    /// Relationship ids present in `to` but not `from`.
    pub added_relationships: Vec<String>,
    /// Relationship ids present in `from` but not `to`.
    pub removed_relationships: Vec<String>,
}

// trace:exempt reason=internal-detail
impl SemanticDelta {
    /// True when the two revisions hold identical member sets.
    // trace:exempt reason=internal-detail
    pub fn is_empty(&self) -> bool {
        self.added_entities.is_empty()
            && self.removed_entities.is_empty()
            && self.added_relationships.is_empty()
            && self.removed_relationships.is_empty()
    }
}

// trace:exempt reason=internal-detail
// trace:exempt reason=internal-detail
impl Store {
    /// Record the current graph as a new revision, transactionally:
    /// revision row + full member row sets commit atomically, so a crash
    /// never leaves a revision with half its members.
    // trace:v1 id=impl.crates-scc-store-src-history.record-revision work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    pub fn record_revision(
        &self,
        source_hash: &str,
        extractor_version: &str,
        file_count: u64,
    ) -> Result<GraphRevision, StoreError> {
        let entities = self.all_entities()?;
        let rels = self.all_relationships()?;
        let base: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(rev), 0) FROM graph_revisions",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let tx = self.conn.unchecked_transaction()?;
        let rev_row = GraphRevision {
            rev: base + 1,
            base_rev: base,
            created_at: scc_core::now_rfc3339(),
            source_hash: source_hash.to_string(),
            extractor_version: extractor_version.to_string(),
            entity_count: entities.len() as u64,
            rel_count: rels.len() as u64,
            file_count,
        };
        {
            let mut ins_rev = tx.prepare(
                "INSERT INTO graph_revisions
                 (rev, base_rev, created_at, source_hash, extractor_version,
                  entity_count, rel_count, file_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            ins_rev.execute(params![
                rev_row.rev,
                rev_row.base_rev,
                rev_row.created_at,
                rev_row.source_hash,
                rev_row.extractor_version,
                rev_row.entity_count as i64,
                rev_row.rel_count as i64,
                rev_row.file_count as i64,
            ])?;
            let mut ins_mem = tx.prepare(
                "INSERT INTO revision_members (rev, kind, id, row_json)
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for e in &entities {
                ins_mem.execute(params![
                    rev_row.rev,
                    "entity",
                    e.id,
                    serde_json::to_string(e).unwrap_or_default(),
                ])?;
            }
            for r in &rels {
                ins_mem.execute(params![
                    rev_row.rev,
                    "relationship",
                    r.id,
                    serde_json::to_string(r).unwrap_or_default(),
                ])?;
            }
        }
        tx.commit()?;
        Ok(rev_row)
    }

    /// All recorded revisions, oldest first.
    // trace:v1 id=impl.crates-scc-store-src-history.revisions work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    pub fn revisions(&self) -> Result<Vec<GraphRevision>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT rev, base_rev, created_at, source_hash, extractor_version,
                    entity_count, rel_count, file_count
             FROM graph_revisions ORDER BY rev",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(GraphRevision {
                rev: r.get(0)?,
                base_rev: r.get(1)?,
                created_at: r.get(2)?,
                source_hash: r.get(3)?,
                extractor_version: r.get(4)?,
                entity_count: r.get::<_, i64>(5)? as u64,
                rel_count: r.get::<_, i64>(6)? as u64,
                file_count: r.get::<_, i64>(7)? as u64,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::from)
    }

    /// Member id sets at a revision: the representative historical view.
    /// Rows are the recorded JSON; ids absent from the live graph decode
    /// as tombstones (present historically, gone now).
    // trace:v1 id=impl.crates-scc-store-src-history.revision-members work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    pub fn revision_members(
        &self,
        rev: i64,
    ) -> Result<(Vec<Entity>, Vec<Relationship>), StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT kind, row_json FROM revision_members WHERE rev = ?1",
        )?;
        let mut entities = Vec::new();
        let mut rels = Vec::new();
        let rows = stmt.query_map(params![rev], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (kind, js) = row?;
            if kind == "entity" {
                if let Ok(e) = serde_json::from_str::<Entity>(&js) {
                    entities.push(e);
                }
            } else if let Ok(r) = serde_json::from_str::<Relationship>(&js) {
                rels.push(r);
            }
        }
        Ok((entities, rels))
    }

    /// Introduction/removal history between two revisions, by id.
    // trace:v1 id=impl.crates-scc-store-src-history.semantic-diff work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    pub fn semantic_diff(&self, from: i64, to: i64) -> Result<SemanticDelta, StoreError> {
        let (fe, fr) = self.revision_members(from)?;
        let (te, tr) = self.revision_members(to)?;
        let ids = |v: Vec<String>| {
            let mut v = v;
            v.sort();
            v
        };
        let fset: std::collections::BTreeSet<String> = fe.into_iter().map(|e| e.id).collect();
        let tset: std::collections::BTreeSet<String> = te.into_iter().map(|e| e.id).collect();
        let frset: std::collections::BTreeSet<String> = fr.into_iter().map(|r| r.id).collect();
        let trset: std::collections::BTreeSet<String> = tr.into_iter().map(|r| r.id).collect();
        Ok(SemanticDelta {
            added_entities: ids(tset.difference(&fset).cloned().collect()),
            removed_entities: ids(fset.difference(&tset).cloned().collect()),
            added_relationships: ids(trset.difference(&frset).cloned().collect()),
            removed_relationships: ids(frset.difference(&trset).cloned().collect()),
        })
    }
}

// trace:exempt reason=internal-detail
impl Store {
    /// Record the current graph, skipping content-identical runs: when the
    /// file-inventory hash matches the head revision, that revision is
    /// returned instead of appending a duplicate.
    // trace:v1 id=impl.crates-scc-store-src-history.record-current-revision work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    pub fn record_current_revision(&self) -> Result<GraphRevision, StoreError> {
        let files = self.all_files()?;
        let mut buf = String::new();
        for (path, hash, _, _, _) in files.iter() {
            buf.push_str(path);
            buf.push('\0');
            buf.push_str(hash);
            buf.push('\n');
        }
        let source_hash = scc_core::fnv1a64_hex(buf.as_bytes());
        let head: Option<GraphRevision> = self.revisions()?.into_iter().last();
        if let Some(h) = head {
            if h.source_hash == source_hash {
                return Ok(h);
            }
        }
        let extractor = format!(
            "store:{};core:{}",
            crate::SCHEMA_VERSION,
            scc_core::SCHEMA_VERSION
        );
        self.record_revision(&source_hash, &extractor, files.len() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::tmp_store;

    #[test]
    // trace:v1 id=test.scc.store.history-records-and-diffs verifies=REQ-SI-503JSBGP exercises=impl.crates-scc-store-src-history.record-current-revision
    fn revisions_record_intro_removal_and_diff() {
        let (s, _d) = tmp_store();
        let r1 = s.record_current_revision().unwrap();
        assert_eq!(r1.rev, 1);
        assert_eq!(r1.base_rev, 0);
        assert!(!r1.source_hash.is_empty());
        assert!(!r1.extractor_version.is_empty());
        // content-identical rerun: no duplicate revision
        let r1b = s.record_current_revision().unwrap();
        assert_eq!(r1b.rev, 1);
        // introduce an entity (with its file row, as indexing always does)
        s.upsert_file("a.py", "h1", "python", "source", 10).unwrap();
        s.insert_entity(
            &Entity::new("repo://t/symbol/a.py/f", "symbol", "f"),
            &["a.py".into()],
        )
        .unwrap();
        let r2 = s.record_current_revision().unwrap();
        assert_eq!((r2.rev, r2.base_rev), (2, 1));
        let d = s.semantic_diff(1, 2).unwrap();
        assert_eq!(d.added_entities, vec!["repo://t/symbol/a.py/f".to_string()]);
        assert!(d.removed_entities.is_empty());
        assert!(!d.is_empty());
        // historical view still sees rev-1 membership (empty of f)
        let (e1, _) = s.revision_members(1).unwrap();
        assert!(e1.is_empty());
        let (e2, _) = s.revision_members(2).unwrap();
        assert_eq!(e2.len(), 1);
        // remove it again: removal history
        s.delete_entity("repo://t/symbol/a.py/f").unwrap();
        s.delete_file("a.py").unwrap();
        let r3 = s.record_current_revision().unwrap();
        assert_eq!(r3.rev, 3);
        let d2 = s.semantic_diff(2, 3).unwrap();
        assert_eq!(d2.removed_entities, vec!["repo://t/symbol/a.py/f".to_string()]);
        assert!(s.semantic_diff(3, 3).unwrap().is_empty());
        // revision log is durable and ordered
        let revs = s.revisions().unwrap();
        assert_eq!(revs.len(), 3);
        assert_eq!(revs[0].base_rev, 0);
    }
}
