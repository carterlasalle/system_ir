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

/// Row fingerprint: deterministic content hash of one entity or
/// relationship row (attributes, confidence, provenance, evidence).
// trace:exempt reason=internal-detail
fn fingerprint_row<T: serde::Serialize>(row: &T) -> String {
    scc_core::fnv1a64_hex(serde_json::to_string(row).unwrap_or_default().as_bytes())
}

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
    /// Content hash of the artifact text: detects re-render drift even
    /// when every entity id survives.
    pub artifact_hash: String,
    /// Canonical entity ids visible in the artifact.
    pub entity_ids: Vec<String>,
    /// Fingerprints of the visible entities at save time (id -> row hash):
    /// modification detection, not just presence.
    pub entity_fp: std::collections::BTreeMap<String, String>,
    /// Fingerprints of live relationships touching the visible set.
    pub rel_fp: std::collections::BTreeMap<String, String>,
    /// Fingerprints of contract-like entities (CONTRACT, ROUTE, TOPIC,
    /// CONFIGURATION kinds) at save time.
    pub contract_fp: std::collections::BTreeMap<String, String>,
    /// Fingerprints of state entities (DATA_STORE, DATA_ENTITY kinds).
    pub state_fp: std::collections::BTreeMap<String, String>,
    /// Flow graph names at save time (sorted set-compare).
    pub flow_names: Vec<String>,
    /// Budget the artifact was rendered under.
    pub budget: usize,
    /// Warnings surfaced with the artifact.
    pub warnings: Vec<String>,
}

/// Snapshot vs current-model comparison (§XXIII).
#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:v1 id=impl.crates-scc-store-src-snapshot.snapshot-diff work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct SnapshotDiff {
    /// Snapshot facts present AND unmodified in the current graph.
    pub still_valid: Vec<String>,
    /// Snapshot facts gone from the current graph.
    pub invalidated: Vec<String>,
    /// Snapshot facts present but content-changed (attributes,
    /// confidence, provenance). "Still exists" is NOT "still valid".
    pub modified_entities: Vec<String>,
    /// Visible-touching relationships added/removed/changed (ids).
    pub changed_relationships: Vec<String>,
    /// Contract-like entities added/removed/modified (ids).
    pub changed_contracts: Vec<String>,
    /// State entities added/removed/modified (ids).
    pub changed_state: Vec<String>,
    /// Flow graph names added/removed.
    pub changed_flows: Vec<String>,
    /// The rendered artifact text itself would differ on re-render.
    pub artifact_changed: bool,
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
        // Fingerprints resolve against the LIVE model at save time: the
        // snapshot pins content, not just ids. Relationships fingerprinted
        // are those touching the visible set (visible knowledge only).
        let visible: std::collections::BTreeSet<&str> =
            save.entity_ids.iter().map(|s| s.as_str()).collect();
        let mut entity_fp = std::collections::BTreeMap::new();
        let mut contract_fp = std::collections::BTreeMap::new();
        let mut state_fp = std::collections::BTreeMap::new();
        for e in self.all_entities()? {
            let fp = fingerprint_row(&e);
            if visible.contains(e.id.as_str()) {
                entity_fp.insert(e.id.clone(), fp.clone());
            }
            match e.kind.as_str() {
                k if k == scc_core::kinds::CONTRACT
                    || k == scc_core::kinds::ROUTE
                    || k == scc_core::kinds::TOPIC
                    || k == scc_core::kinds::CONFIGURATION =>
                {
                    contract_fp.insert(e.id.clone(), fp);
                }
                k if k == scc_core::kinds::DATA_STORE || k == scc_core::kinds::DATA_ENTITY => {
                    state_fp.insert(e.id.clone(), fp);
                }
                _ => {}
            }
        }
        let mut rel_fp = std::collections::BTreeMap::new();
        for r in self.all_relationships()? {
            if visible.contains(r.subject.as_str()) || visible.contains(r.object.as_str()) {
                rel_fp.insert(r.id.clone(), fingerprint_row(&r));
            }
        }
        let mut flow_names: Vec<String> = self
            .flow_graphs()
            .unwrap_or_default()
            .into_iter()
            .map(|g| g.name)
            .collect();
        flow_names.sort();
        let snap = ContextSnapshot {
            id: format!("snap-{}-{hex}", save.revision),
            created_at: scc_core::now_rfc3339(),
            task: save.task.to_string(),
            epoch: save.epoch.to_string(),
            revision: save.revision,
            artifact: save.artifact.to_string(),
            artifact_hash: scc_core::fnv1a64_hex(save.artifact.as_bytes()),
            entity_ids: save.entity_ids.to_vec(),
            entity_fp,
            rel_fp,
            contract_fp,
            state_fp,
            flow_names,
            budget: save.budget,
            warnings: save.warnings.to_vec(),
        };
        self.conn.execute(
            "INSERT INTO context_snapshots
             (id, created_at, task, epoch, revision, artifact, artifact_hash,
              entity_ids, entity_fp, rel_fp, contract_fp, state_fp, flow_names,
              budget, warnings)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(id) DO UPDATE SET
               created_at = excluded.created_at, artifact = excluded.artifact,
               artifact_hash = excluded.artifact_hash,
               entity_ids = excluded.entity_ids, entity_fp = excluded.entity_fp,
               rel_fp = excluded.rel_fp, contract_fp = excluded.contract_fp,
               state_fp = excluded.state_fp, flow_names = excluded.flow_names,
               budget = excluded.budget, warnings = excluded.warnings",
            params![
                snap.id,
                snap.created_at,
                snap.task,
                snap.epoch,
                snap.revision,
                snap.artifact,
                snap.artifact_hash,
                serde_json::to_string(&snap.entity_ids).unwrap_or_default(),
                serde_json::to_string(&snap.entity_fp).unwrap_or_default(),
                serde_json::to_string(&snap.rel_fp).unwrap_or_default(),
                serde_json::to_string(&snap.contract_fp).unwrap_or_default(),
                serde_json::to_string(&snap.state_fp).unwrap_or_default(),
                serde_json::to_string(&snap.flow_names).unwrap_or_default(),
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
            "SELECT id, created_at, task, epoch, revision, artifact, artifact_hash,
                    entity_ids, entity_fp, rel_fp, contract_fp, state_fp, flow_names,
                    budget, warnings
             FROM context_snapshots WHERE id = ?1",
        )?;
        let js = |r: &rusqlite::Row, i: usize| -> String { r.get(i).unwrap_or_default() };
        let mut rows = stmt.query_map(params![id], |r| {
            Ok(ContextSnapshot {
                id: r.get(0)?,
                created_at: r.get(1)?,
                task: r.get(2)?,
                epoch: r.get(3)?,
                revision: r.get(4)?,
                artifact: r.get(5)?,
                artifact_hash: r.get(6)?,
                entity_ids: serde_json::from_str(&js(r, 7)).unwrap_or_default(),
                entity_fp: serde_json::from_str(&js(r, 8)).unwrap_or_default(),
                rel_fp: serde_json::from_str(&js(r, 9)).unwrap_or_default(),
                contract_fp: serde_json::from_str(&js(r, 10)).unwrap_or_default(),
                state_fp: serde_json::from_str(&js(r, 11)).unwrap_or_default(),
                flow_names: serde_json::from_str(&js(r, 12)).unwrap_or_default(),
                budget: r.get::<_, i64>(13)? as usize,
                warnings: serde_json::from_str(&js(r, 14)).unwrap_or_default(),
            })
        })?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// Diff a snapshot against the current graph: presence AND content.
    /// A surviving id with a changed row is `modified`, not `still_valid`.
    /// Relationships touching the visible set, contract/state entities, flow
    /// names, and the artifact hash are compared too. Newly-relevant
    /// knowledge needs a re-render (resume), which this diff does not invent.
    // trace:v1 id=impl.crates-scc-store-src-snapshot.diff-snapshot work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    pub fn diff_snapshot(&self, id: &str) -> Result<Option<SnapshotDiff>, StoreError> {
        let Some(snap) = self.load_snapshot(id)? else {
            return Ok(None);
        };
        let mut live_fp = std::collections::BTreeMap::new();
        for e in self.all_entities()? {
            live_fp.insert(e.id.clone(), fingerprint_row(&e));
        }
        let mut still_valid = Vec::new();
        let mut invalidated = Vec::new();
        let mut modified_entities = Vec::new();
        for eid in &snap.entity_ids {
            match live_fp.get(eid) {
                None => invalidated.push(eid.clone()),
                Some(fp) => {
                    if snap.entity_fp.get(eid).map(|s| s == fp).unwrap_or(true) {
                        still_valid.push(eid.clone());
                    } else {
                        modified_entities.push(eid.clone());
                    }
                }
            }
        }
        // Relationships touching the visible set: added/removed/changed.
        let visible: std::collections::BTreeSet<&str> =
            snap.entity_ids.iter().map(|s| s.as_str()).collect();
        let mut live_rel_fp = std::collections::BTreeMap::new();
        for r in self.all_relationships()? {
            if visible.contains(r.subject.as_str()) || visible.contains(r.object.as_str()) {
                live_rel_fp.insert(r.id.clone(), fingerprint_row(&r));
            }
        }
        let mut changed_relationships: Vec<String> = live_rel_fp
            .iter()
            .filter(|(k, v)| snap.rel_fp.get(*k) != Some(*v))
            .map(|(k, _)| k.clone())
            .chain(
                snap.rel_fp
                    .keys()
                    .filter(|k| !live_rel_fp.contains_key(*k))
                    .cloned(),
            )
            .collect();
        changed_relationships.sort();
        // Contract-like and state entities: fingerprint maps, same treatment.
        let fp_diff = |old: &std::collections::BTreeMap<String, String>| -> Vec<String> {
            let mut live = std::collections::BTreeMap::new();
            for e in self.all_entities().unwrap_or_default() {
                live.insert(e.id.clone(), fingerprint_row(&e));
            }
            let mut out: Vec<String> = live
                .iter()
                .filter(|(k, v)| old.get(*k) != Some(*v))
                .map(|(k, _)| k.clone())
                .chain(old.keys().filter(|k| !live.contains_key(*k)).cloned())
                .collect();
            out.sort();
            out
        };
        // Restrict the generic entity sweep to the saved category maps:
        // an id counts as a contract/state change only if it was (or is) one.
        let mut live_kind: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
        for e in self.all_entities().unwrap_or_default() {
            live_kind.insert(e.id.clone(), e.kind.clone());
        }
        let changed_contracts: Vec<String> = fp_diff(&snap.contract_fp)
            .into_iter()
            .filter(|id| {
                snap.contract_fp.contains_key(id)
                    || live_kind.get(id).map(|k| {
                        k == scc_core::kinds::CONTRACT
                            || k == scc_core::kinds::ROUTE
                            || k == scc_core::kinds::TOPIC
                            || k == scc_core::kinds::CONFIGURATION
                    }).unwrap_or(false)
            })
            .collect();
        let changed_state: Vec<String> = fp_diff(&snap.state_fp)
            .into_iter()
            .filter(|id| {
                snap.state_fp.contains_key(id)
                    || live_kind.get(id).map(|k| {
                        k == scc_core::kinds::DATA_STORE || k == scc_core::kinds::DATA_ENTITY
                    }).unwrap_or(false)
            })
            .collect();
        let mut live_flows: Vec<String> = self
            .flow_graphs()
            .unwrap_or_default()
            .into_iter()
            .map(|g| g.name)
            .collect();
        live_flows.sort();
        let live_flow_set: std::collections::BTreeSet<&str> =
            live_flows.iter().map(|s| s.as_str()).collect();
        let saved_flow_set: std::collections::BTreeSet<&str> =
            snap.flow_names.iter().map(|s| s.as_str()).collect();
        let mut changed_flows: Vec<String> = live_flow_set
            .symmetric_difference(&saved_flow_set)
            .map(|s| s.to_string())
            .collect();
        changed_flows.sort();
        let current_revision: i64 = self
            .conn
            .query_row("SELECT COALESCE(MAX(rev), 0) FROM graph_revisions", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        // Artifact drift: the artifact is a pure function of visible
        // entity rows + touching rels, so any change on those axes means a
        // re-render would differ. (The stored artifact_hash pins the exact
        // text for external comparison; this verdict is the live signal.)
        let artifact_changed = !snap.artifact.is_empty()
            && (!invalidated.is_empty()
                || !modified_entities.is_empty()
                || !changed_relationships.is_empty());
        Ok(Some(SnapshotDiff {
            still_valid,
            invalidated,
            modified_entities,
            changed_relationships,
            changed_contracts,
            changed_state,
            changed_flows,
            artifact_changed,
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
        // same id, changed row: modified, NOT still valid (the V1 blind spot)
        let mut f2 = Entity::new("repo://t/symbol/a.py/f", "symbol", "f");
        f2.attr("confidence", serde_json::json!(0.99));
        s.insert_entity(&f2, &["a.py".into()]).unwrap();
        let d2 = s.diff_snapshot(&snap.id).unwrap().unwrap();
        assert!(d2.still_valid.is_empty(), "{d2:?}");
        assert_eq!(d2.modified_entities, vec!["repo://t/symbol/a.py/f".to_string()]);
        assert!(d2.artifact_changed, "visible rows changed: re-render would differ");
        // unknown id: honest None, not an error
        assert!(s.diff_snapshot("snap-0-deadbeef").unwrap().is_none());
    }
}
