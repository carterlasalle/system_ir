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
    /// Hash of the semantic configuration (language backends, resolver
    /// switches): a config change without file changes still advances.
    pub semantic_config_hash: String,
    /// Hash of the actual graph rows (entity + relationship JSON): an
    /// extractor/confidence/provenance change without file changes still
    /// advances, and it is the content-identity half of dedup.
    pub graph_content_hash: String,
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
    /// Entity ids in both whose recorded row changed (attributes,
    /// confidence, provenance, evidence — anything in the row JSON).
    pub modified_entities: Vec<String>,
    /// Relationship ids present in `to` but not `from`.
    pub added_relationships: Vec<String>,
    /// Relationship ids present in `from` but not `to`.
    pub removed_relationships: Vec<String>,
    /// Relationship ids in both whose recorded row changed.
    pub modified_relationships: Vec<String>,
    /// Modified-entity counts by entity kind (contract/state/flow-visible
    /// changes surface here through their member entities; contracts,
    /// state claims, and flows derive from these rows, so no parallel
    /// versioned store is needed).
    pub modified_kinds: std::collections::BTreeMap<String, usize>,
}

// trace:exempt reason=internal-detail
impl SemanticDelta {
    /// True when the two revisions hold identical member sets.
    // trace:exempt reason=internal-detail
    pub fn is_empty(&self) -> bool {
        self.added_entities.is_empty()
            && self.removed_entities.is_empty()
            && self.modified_entities.is_empty()
            && self.added_relationships.is_empty()
            && self.removed_relationships.is_empty()
            && self.modified_relationships.is_empty()
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
        semantic_config_hash: &str,
        graph_content_hash: &str,
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
            semantic_config_hash: semantic_config_hash.to_string(),
            graph_content_hash: graph_content_hash.to_string(),
            entity_count: entities.len() as u64,
            rel_count: rels.len() as u64,
            file_count,
        };
        {
            let mut ins_rev = tx.prepare(
                "INSERT INTO graph_revisions
                 (rev, base_rev, created_at, source_hash, extractor_version,
                  semantic_config_hash, graph_content_hash,
                  entity_count, rel_count, file_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;
            ins_rev.execute(params![
                rev_row.rev,
                rev_row.base_rev,
                rev_row.created_at,
                rev_row.source_hash,
                rev_row.extractor_version,
                rev_row.semantic_config_hash,
                rev_row.graph_content_hash,
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
                    semantic_config_hash, graph_content_hash,
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
                semantic_config_hash: r.get(5)?,
                graph_content_hash: r.get(6)?,
                entity_count: r.get::<_, i64>(7)? as u64,
                rel_count: r.get::<_, i64>(8)? as u64,
                file_count: r.get::<_, i64>(9)? as u64,
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

    /// Raw recorded rows at a revision: id -> row JSON, the content
    /// half of the V2 diff. Rows decode losslessly (see
    /// [`Store::revision_members`]); comparing JSON here avoids
    /// re-serialization drift.
    // trace:exempt reason=internal-detail
    pub fn revision_rows(
        &self,
        rev: i64,
    ) -> Result<
        (
            std::collections::BTreeMap<String, String>,
            std::collections::BTreeMap<String, String>,
        ),
        StoreError,
    > {
        let mut stmt = self
            .conn
            .prepare("SELECT kind, id, row_json FROM revision_members WHERE rev = ?1")?;
        let mut entities = std::collections::BTreeMap::new();
        let mut rels = std::collections::BTreeMap::new();
        let rows = stmt.query_map(params![rev], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (kind, id, js) = row?;
            if kind == "entity" {
                entities.insert(id, js);
            } else {
                rels.insert(id, js);
            }
        }
        Ok((entities, rels))
    }

    /// Introduction/removal/modification history between two revisions.
    /// Same id in both revisions with different row JSON (attributes,
    /// confidence, provenance, evidence) reports as modified — the V1
    /// set-membership blind spot.
    // trace:v1 id=impl.crates-scc-store-src-history.semantic-diff work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    pub fn semantic_diff(&self, from: i64, to: i64) -> Result<SemanticDelta, StoreError> {
        let (fe, fr) = self.revision_rows(from)?;
        let (te, tr) = self.revision_rows(to)?;
        let ids = |v: Vec<String>| {
            let mut v = v;
            v.sort();
            v
        };
        let fset: std::collections::BTreeSet<String> = fe.keys().cloned().collect();
        let tset: std::collections::BTreeSet<String> = te.keys().cloned().collect();
        let frset: std::collections::BTreeSet<String> = fr.keys().cloned().collect();
        let trset: std::collections::BTreeSet<String> = tr.keys().cloned().collect();
        let mut modified_entities: Vec<String> = tset
            .intersection(&fset)
            .filter(|id| te.get(*id) != fe.get(*id))
            .cloned()
            .collect();
        modified_entities.sort();
        let mut modified_relationships: Vec<String> = trset
            .intersection(&frset)
            .filter(|id| tr.get(*id) != fr.get(*id))
            .cloned()
            .collect();
        modified_relationships.sort();
        // Modified counts by entity kind: decode the TO row (kinds are
        // stable row fields, never inferred here).
        let mut modified_kinds: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for id in &modified_entities {
            if let Some(js) = te.get(id) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(js) {
                    if let Some(k) = v.get("kind").and_then(|k| k.as_str()) {
                        *modified_kinds.entry(k.to_string()).or_default() += 1;
                    }
                }
            }
        }
        Ok(SemanticDelta {
            added_entities: ids(tset.difference(&fset).cloned().collect()),
            removed_entities: ids(fset.difference(&tset).cloned().collect()),
            modified_entities,
            added_relationships: ids(trset.difference(&frset).cloned().collect()),
            removed_relationships: ids(frset.difference(&trset).cloned().collect()),
            modified_relationships,
            modified_kinds,
        })
    }
}

// trace:exempt reason=internal-detail
impl Store {
    /// Content hash of the live graph rows (entity + relationship JSON,
    /// sorted): the content-identity half of revision dedup. An extractor,
    /// confidence, or provenance change advances the revision even when no
    /// source file moved.
    // trace:exempt reason=internal-detail
    pub fn graph_content_hash(&self) -> Result<String, StoreError> {
        let mut rows: Vec<String> = Vec::new();
        for e in self.all_entities()? {
            rows.push(serde_json::to_string(&e).unwrap_or_default());
        }
        for r in self.all_relationships()? {
            rows.push(serde_json::to_string(&r).unwrap_or_default());
        }
        rows.sort();
        Ok(scc_core::fnv1a64_hex(rows.join("\n").as_bytes()))
    }

    /// Record the current graph, skipping fully-identical runs: when
    /// source inventory, extractor, semantic config, AND graph content
    /// all match the head revision, that revision is returned instead of
    /// appending a duplicate. Any axis changing advances the history.
    // trace:v1 id=impl.crates-scc-store-src-history.record-current-revision work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    pub fn record_current_revision(&self) -> Result<GraphRevision, StoreError> {
        self.record_current_revision_with_config("")
    }

    /// [`record_current_revision`] with the caller's semantic-config hash
    /// (language backends, resolver switches — the indexer hashes its
    /// semantic-relevant config). Empty means "no config axis".
    // trace:v1 id=impl.crates-scc-store-src-history.record-current-revision-config work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    pub fn record_current_revision_with_config(
        &self,
        semantic_config_hash: &str,
    ) -> Result<GraphRevision, StoreError> {
        let files = self.all_files()?;
        let mut buf = String::new();
        for (path, hash, _, _, _) in files.iter() {
            buf.push_str(path);
            buf.push('\0');
            buf.push_str(hash);
            buf.push('\n');
        }
        let source_hash = scc_core::fnv1a64_hex(buf.as_bytes());
        let content_hash = self.graph_content_hash()?;
        let extractor = format!(
            "store:{};core:{}",
            crate::SCHEMA_VERSION,
            scc_core::SCHEMA_VERSION
        );
        let head: Option<GraphRevision> = self.revisions()?.into_iter().last();
        if let Some(h) = head {
            if h.source_hash == source_hash
                && h.extractor_version == extractor
                && h.semantic_config_hash == semantic_config_hash
                && h.graph_content_hash == content_hash
            {
                return Ok(h);
            }
        }
        self.record_revision(
            &source_hash,
            &extractor,
            files.len() as u64,
            semantic_config_hash,
            &content_hash,
        )
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
        // V2: same files, changed row content (confidence bump) advances
        // the history and diffs as modified, not added/removed.
        s.upsert_file("a.py", "h1", "python", "source", 10).unwrap();
        s.insert_entity(
            &Entity::new("repo://t/symbol/a.py/g", "symbol", "g"),
            &["a.py".into()],
        )
        .unwrap();
        let r4 = s.record_current_revision().unwrap();
        assert_eq!(r4.rev, 4);
        let mut g = Entity::new("repo://t/symbol/a.py/g", "symbol", "g");
        g.attr("confidence", serde_json::json!(0.77));
        s.insert_entity(&g, &["a.py".into()]).unwrap();
        let r5 = s.record_current_revision().unwrap();
        assert_eq!(r5.rev, 5, "content change without file change must advance");
        assert_ne!(r5.graph_content_hash, r4.graph_content_hash);
        let d4 = s.semantic_diff(4, 5).unwrap();
        assert!(d4.added_entities.is_empty() && d4.removed_entities.is_empty());
        assert_eq!(d4.modified_entities, vec!["repo://t/symbol/a.py/g".to_string()]);
        assert_eq!(d4.modified_kinds.get("symbol"), Some(&1));
        assert!(s.semantic_diff(5, 5).unwrap().is_empty());
        // V2: config axis — same content, changed config hash advances.
        let r6 = s.record_current_revision_with_config("cfg2").unwrap();
        assert_eq!(r6.rev, 6);
        assert_eq!(r6.semantic_config_hash, "cfg2");
        let r6b = s.record_current_revision_with_config("cfg2").unwrap();
        assert_eq!(r6b.rev, 6, "fully-identical rerun dedups");
    }
}
