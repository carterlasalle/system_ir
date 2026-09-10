//! Durable context snapshots (mission §XXII): checkpoints of RENDERED agent
//! knowledge — what was actually shown, not all candidates considered.
//!
//! Complements [`ContextLedger`](crate) novelty semantics (which the
//! snapshot never weakens): the ledger answers what is new *now*; a
//! snapshot pins what was visible *then* so later model changes can be
//! diffed (still-valid vs invalidated facts, §XXIII).

use crate::{Store, StoreError};
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// A durable checkpoint of one rendered artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:v1 id=impl.crates-scc-store-src-snapshot.context-snapshot work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct ContextSnapshot {
    /// Deterministic id: `snap-{revision}-{fnv(task+epoch)}`.
    pub id: String,
    /// Creation time (RFC3339).
    pub created_at: String,
    /// Task the artifact was rendered for.
    pub task: String,
    /// Active compiled-view identity at render time (not a source revision).
    pub epoch: String,
    /// Persistent history position at render time.
    pub revision: i64,
    /// Rendered artifact text (visible knowledge only).
    pub artifact: String,
    /// Canonical entity ids visible in the artifact.
    pub entity_ids: Vec<String>,
    /// Budget the artifact was rendered under.
    pub budget: usize,
    /// Warnings surfaced with the artifact.
    pub warnings: Vec<String>,
}

/// Snapshot vs current-model comparison (§XXIII).
#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:v1 id=impl.crates-scc-store-src-snapshot.snapshot-diff work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct SnapshotDiff {
    /// Snapshot facts still present in the current graph.
    pub still_valid: Vec<String>,
    /// Snapshot facts gone from the current graph.
    pub invalidated: Vec<String>,
    /// Revision the snapshot was taken at.
    pub snapshot_revision: i64,
    /// Current head revision (0 when history was never recorded).
    pub current_revision: i64,
}

/// Parameters for [`Store::save_snapshot`] (kept as one struct so the
/// seven-field render call stays under the complexity budget).
// trace:exempt reason=internal-detail
pub struct SnapshotSave<'a> {
    /// Task the artifact was rendered for.
    pub task: &'a str,
    /// Active compiled-view identity at render time.
    pub epoch: &'a str,
    /// Persistent history position at render time.
    pub revision: i64,
    /// Rendered artifact text (visible knowledge only).
    pub artifact: &'a str,
    /// Canonical entity ids visible in the artifact.
    pub entity_ids: &'a [String],
    /// Budget the artifact was rendered under.
    pub budget: usize,
    /// Warnings surfaced with the artifact.
    pub warnings: &'a [String],
}

// trace:exempt reason=internal-detail
impl Store {
    /// Persist a rendered artifact as a durable checkpoint. Idempotent:
    /// same (task, epoch) resolves to the same id and replaces it.
    // trace:v1 id=impl.crates-scc-store-src-snapshot.save-snapshot work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    pub fn save_snapshot(&self, save: SnapshotSave<'_>) -> Result<ContextSnapshot, StoreError> {
        let hex =
            scc_core::fnv1a64_hex(format!("{}\0{}", save.task, save.epoch).as_bytes());
        let snap = ContextSnapshot {
            id: format!("snap-{}-{hex}", save.revision),
            created_at: scc_core::now_rfc3339(),
            task: save.task.to_string(),
            epoch: save.epoch.to_string(),
            revision: save.revision,
            artifact: save.artifact.to_string(),
            entity_ids: save.entity_ids.to_vec(),
            budget: save.budget,
            warnings: save.warnings.to_vec(),
        };
        self.conn.execute(
            "INSERT INTO context_snapshots
             (id, created_at, task, epoch, revision, artifact, entity_ids, budget, warnings)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
               created_at = excluded.created_at, artifact = excluded.artifact,
               entity_ids = excluded.entity_ids, budget = excluded.budget,
               warnings = excluded.warnings",
            params![
                snap.id,
                snap.created_at,
                snap.task,
                snap.epoch,
                snap.revision,
                snap.artifact,
                serde_json::to_string(&snap.entity_ids).unwrap_or_default(),
                snap.budget as i64,
                serde_json::to_string(&snap.warnings).unwrap_or_default(),
            ],
        )?;
        Ok(snap)
    }

    /// Load a snapshot by id. Survives later model changes: the row is
    /// never mutated by indexing, only replaced by an explicit re-save.
    // trace:v1 id=impl.crates-scc-store-src-snapshot.load-snapshot work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    pub fn load_snapshot(&self, id: &str) -> Result<Option<ContextSnapshot>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, task, epoch, revision, artifact,
                    entity_ids, budget, warnings
             FROM context_snapshots WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], |r| {
            Ok(ContextSnapshot {
                id: r.get(0)?,
                created_at: r.get(1)?,
                task: r.get(2)?,
                epoch: r.get(3)?,
                revision: r.get(4)?,
                artifact: r.get(5)?,
                entity_ids: serde_json::from_str(&r.get::<_, String>(6)?)
                    .unwrap_or_default(),
                budget: r.get::<_, i64>(7)? as usize,
                warnings: serde_json::from_str(&r.get::<_, String>(8)?)
                    .unwrap_or_default(),
            })
        })?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// Diff a snapshot against the current graph: still-valid vs
    /// invalidated facts by canonical id. Newly-relevant knowledge needs
    /// a re-render (resume), which this diff does not invent.
    // trace:v1 id=impl.crates-scc-store-src-snapshot.diff-snapshot work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    pub fn diff_snapshot(&self, id: &str) -> Result<Option<SnapshotDiff>, StoreError> {
        let Some(snap) = self.load_snapshot(id)? else {
            return Ok(None);
        };
        let live: std::collections::BTreeSet<String> = self
            .all_entities()?
            .into_iter()
            .map(|e| e.id)
            .collect();
        let mut still_valid = Vec::new();
        let mut invalidated = Vec::new();
        for eid in &snap.entity_ids {
            if live.contains(eid) {
                still_valid.push(eid.clone());
            } else {
                invalidated.push(eid.clone());
            }
        }
        let current_revision: i64 = self
            .conn
            .query_row("SELECT COALESCE(MAX(rev), 0) FROM graph_revisions", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        Ok(Some(SnapshotDiff {
            still_valid,
            invalidated,
            snapshot_revision: snap.revision,
            current_revision,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{tests::tmp_store, Entity};

    #[test]
    // trace:v1 id=test.scc.store.snapshot-save-diff verifies=REQ-SI-503JSBGP exercises=impl.crates-scc-store-src-snapshot.save-snapshot
    fn snapshots_persist_diff_and_survive() {
        let (s, _d) = tmp_store();
        s.upsert_file("a.py", "h1", "python", "source", 10).unwrap();
        for id in ["repo://t/symbol/a.py/f", "repo://t/symbol/a.py/g"] {
            s.insert_entity(&Entity::new(id, "symbol", "f"), &["a.py".into()])
                .unwrap();
        }
        let r1 = s.record_current_revision().unwrap();
        let epoch = "epoch:test";
        let snap = s
            .save_snapshot(SnapshotSave {
                task: "do the thing",
                epoch,
                revision: r1.rev,
                artifact: "pack showing f and g",
                entity_ids: &[
                    "repo://t/symbol/a.py/f".into(),
                    "repo://t/symbol/a.py/g".into(),
                ],
                budget: 100,
                warnings: &[],
            })
            .unwrap();
        assert_eq!(snap.revision, r1.rev);
        assert_eq!(snap.epoch, epoch);
        // idempotent re-save of the same render
        let snap2 = s
            .save_snapshot(SnapshotSave {
                task: "do the thing",
                epoch,
                revision: r1.rev,
                artifact: "pack showing f and g",
                entity_ids: &[
                    "repo://t/symbol/a.py/f".into(),
                    "repo://t/symbol/a.py/g".into(),
                ],
                budget: 100,
                warnings: &[],
            })
            .unwrap();
        assert_eq!(snap.id, snap2.id);
        // resume: load returns the stored render
        let loaded = s.load_snapshot(&snap.id).unwrap().unwrap();
        assert_eq!(loaded.artifact, "pack showing f and g");
        // everything valid while the model is unchanged
        let d0 = s.diff_snapshot(&snap.id).unwrap().unwrap();
        assert!(d0.invalidated.is_empty());
        assert_eq!(d0.still_valid.len(), 2);
        // model change invalidates exactly the removed fact
        s.delete_entity("repo://t/symbol/a.py/g").unwrap();
        s.upsert_file("a.py", "h2", "python", "source", 10).unwrap();
        let _r2 = s.record_current_revision().unwrap();
        let d1 = s.diff_snapshot(&snap.id).unwrap().unwrap();
        assert_eq!(d1.still_valid, vec!["repo://t/symbol/a.py/f".to_string()]);
        assert_eq!(d1.invalidated, vec!["repo://t/symbol/a.py/g".to_string()]);
        assert_eq!(d1.snapshot_revision, r1.rev);
        assert_eq!(d1.current_revision, r1.rev + 1);
        // unknown id: honest None, not an error
        assert!(s.diff_snapshot("snap-0-deadbeef").unwrap().is_none());
    }
}
