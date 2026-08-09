//! PreCompact checkpoint (docs §125–§127): transient task state persisted to
//! `.scc/checkpoint.json` so compaction is transparent.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    if !modified.is_empty() {
        let graph = scc_graph::RealityGraph::load(&store)?;
        if let Ok(imp) = scc_graph::impact::compute_impact(&graph, &store, &modified, &[]) {
            cp.affected.components = imp.components;
            cp.affected.flows = imp.flows;
            cp.affected.contracts = imp.contracts;
            cp.affected.invariants = imp.invariants;
        }
    }
    cp.files.modified = modified;

    let path = crate::checkpoint_path(root);
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
    std::fs::write(&path, serde_json::to_string_pretty(&cp)?)?;
    Ok(cp)
}

/// Load and render the checkpoint as markdown for session rehydration.
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
