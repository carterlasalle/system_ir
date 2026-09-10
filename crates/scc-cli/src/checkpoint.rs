//! PreCompact checkpoint (docs §125–§127): transient task state persisted to
//! `.scc/checkpoint.json` so compaction is transparent.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct Checkpoint {
    pub task: TaskRef,
    pub system_ir_revision: String,
    pub affected: Affected,
    pub files: Files,
    pub tests: Tests,
    #[serde(default)]
    pub decisions: Vec<String>,
    #[serde(default)]
    pub next_actions: Vec<String>,
    #[serde(default)]
    pub created_at: String,
    /// Durable ContextSnapshot id pinned at capture (semantic rehydration:
    /// `checkpoint load` diffs it so compaction sees what survived).
    #[serde(default)]
    pub snapshot_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskRef {
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub bead: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Affected {
    #[serde(default)]
    pub components: Vec<String>,
    #[serde(default)]
    pub flows: Vec<String>,
    #[serde(default)]
    pub contracts: Vec<String>,
    #[serde(default)]
    pub invariants: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Files {
    #[serde(default)]
    pub modified: Vec<String>,
    #[serde(default)]
    pub inspected: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tests {
    #[serde(default)]
    pub passed: Vec<String>,
    #[serde(default)]
    pub failed: Vec<String>,
    #[serde(default)]
    pub not_run: Vec<String>,
}

/// Capture the current checkpoint: git-modified files + current system IR
/// revision + affected system entities derived from the working tree.
// trace:v1 id=impl.scc.checkpoint work=WORK-SCC-001 satisfies=REQ-SCC-API
pub fn capture(root: &Path) -> crate::Result<Checkpoint> {
    let store = crate::open_store(root)?;
    let revision = store
        .latest_snapshot()?
        .map(|s| s.revision)
        .unwrap_or_default();

    // modified files via git
    let modified = git_modified(root);

    // affected entities via impact on the modified files
    let mut cp = Checkpoint {
        system_ir_revision: revision,
        created_at: scc_core::now_rfc3339(),
        ..Default::default()
    };
    // goal/bead come from the active Beads task (task state, not system
    // facts — the checkpoint is transient session state, §126)
    if let Some((bead, goal)) = scc_indexer::adapters::beads::active_bead(root) {
        cp.task.bead = Some(bead);
        cp.task.goal = goal;
    }
    if !modified.is_empty() {
        let graph = scc_graph::RealityGraph::load(&store)?;
        let view = scc_graph::TrustedGraphView::new(
            &graph,
            &store,
            &[],
            scc_graph::TrustPolicy::default(),
        );
        if let Ok(imp) = scc_graph::impact::compute_impact(&view, &store, &modified, &[]) {
            cp.affected.components = imp.components;
            cp.affected.flows = imp.flows;
            cp.affected.contracts = imp.contracts;
            cp.affected.invariants = imp.invariants;
        }
    }
    cp.files.modified = modified;

    // Durable snapshot link: pin affected ids + checkpoint text so
    // `checkpoint load` (OMP PreCompact rehydration) reports still-valid
    // vs invalidated vs modified facts, not just task state. Impact
    // fields are already entity ids (components/flows/routes/invariants).
    {
        let mut entity_ids: Vec<String> = Vec::new();
        entity_ids.extend(cp.affected.components.iter().cloned());
        entity_ids.extend(cp.affected.flows.iter().cloned());
        entity_ids.extend(cp.affected.contracts.iter().cloned());
        entity_ids.extend(cp.affected.invariants.iter().cloned());
        entity_ids.sort();
        entity_ids.dedup();
        let epoch = store.cache_epoch().unwrap_or_else(|_| "no-epoch".into());
        let head = store
            .revisions()
            .map(|rs| rs.into_iter().last().map(|r| r.rev).unwrap_or(0))
            .unwrap_or(0);
        let task = if cp.task.goal.is_empty() {
            cp.task.bead.clone().unwrap_or_else(|| "checkpoint".into())
        } else {
            cp.task.goal.clone()
        };
        if let Ok(artifact) = serde_json::to_string_pretty(&cp) {
            if let Ok(snap) = store.save_snapshot(scc_store::snapshot::SnapshotSave {
                task: &task,
                epoch: &epoch,
                revision: head,
                artifact: &artifact,
                entity_ids: &entity_ids,
                budget: 0,
                warnings: &[],
            }) {
                cp.snapshot_id = Some(snap.id);
            }
        }
    }

    let path = crate::checkpoint_path(root);
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
    std::fs::write(&path, serde_json::to_string_pretty(&cp)?)?;
    Ok(cp)
}

/// Load and render the checkpoint as markdown for session rehydration.
// trace:v1 id=impl.scc.checkpoint.load work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn load(root: &Path) -> crate::Result<Option<String>> {
    let path = crate::checkpoint_path(root);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cp: Checkpoint = serde_json::from_str(&text)?;
    let mut out = String::from("# TASK CHECKPOINT (restored)\n\n");
    if !cp.task.goal.is_empty() {
        out.push_str(&format!("## Goal\n{}\n\n", cp.task.goal));
    }
    if !cp.system_ir_revision.is_empty() {
        out.push_str(&format!("System IR revision: {}\n\n", cp.system_ir_revision));
    }
    if !cp.affected.components.is_empty() {
        out.push_str(&format!("Affected components: {}\n\n", cp.affected.components.join(", ")));
    }
    if !cp.affected.flows.is_empty() {
        out.push_str(&format!("Affected flows: {}\n\n", cp.affected.flows.join(", ")));
    }
    if !cp.affected.contracts.is_empty() {
        out.push_str(&format!("Affected contracts: {}\n\n", cp.affected.contracts.join(", ")));
    }
    if !cp.affected.invariants.is_empty() {
        out.push_str(&format!("Affected invariants: {}\n\n", cp.affected.invariants.join(", ")));
    }
    if !cp.files.modified.is_empty() {
        out.push_str(&format!(
            "Modified files:\n{}\n\n",
            cp.files.modified
                .iter()
                .map(|f| format!("- {f}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if !cp.decisions.is_empty() {
        out.push_str(&format!(
            "Decisions:\n{}\n\n",
            cp.decisions
                .iter()
                .map(|d| format!("- {d}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if !cp.next_actions.is_empty() {
        out.push_str(&format!(
            "Next actions:\n{}\n\n",
            cp.next_actions
                .iter()
                .map(|n| format!("- {n}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    // Semantic rehydration verdict: what the pinned snapshot says about
    // model drift since capture (still-valid vs invalidated vs modified).
    if let Some(sid) = cp.snapshot_id.as_deref() {
        match crate::open_store(root).and_then(|store| {
            store
                .diff_snapshot(sid)
                .map_err(crate::CliError::from)
        }) {
            Ok(Some(d)) => {
                out.push_str(&format!(
                    "## Snapshot validity (model drift since checkpoint)\n{} still valid, {} invalidated, {} modified{}\n\n",
                    d.still_valid.len(),
                    d.invalidated.len(),
                    d.modified_entities.len(),
                    if d.artifact_changed {
                        " — context would re-render differently"
                    } else {
                        ""
                    },
                ));
                for f in d.invalidated.iter().chain(d.modified_entities.iter()).take(10) {
                    out.push_str(&format!("- {f}\n"));
                }
                if d.invalidated.len() + d.modified_entities.len() > 10 {
                    out.push_str(&format!(
                        "- … ({} more)\n",
                        d.invalidated.len() + d.modified_entities.len() - 10
                    ));
                }
                out.push('\n');
            }
            Ok(None) => {
                out.push_str("## Snapshot validity\ncheckpoint snapshot no longer stored\n\n");
            }
            Err(e) => {
                out.push_str(&format!("## Snapshot validity\nsnapshot unreadable: {e}\n\n"));
            }
        }
    }
    Ok(Some(out))
}

fn git_modified(root: &Path) -> Vec<String> {
    let out = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output();
    let Ok(out) = out else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let l = l.trim_start();
            if l.len() < 4 {
                return None;
            }
            let (status, path) = l.split_at(3);
            if status.contains('?') {
                return None;
            }
            let path = path.trim();
            if path.is_empty() || path.starts_with(".scc/") {
                return None;
            }
            Some(path.to_string())
        })
        .collect()
}
