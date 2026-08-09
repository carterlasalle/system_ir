//! Beads adapter (SCC-203): import task state from the Beads issue tracker
//! (`.beads/issues.jsonl` or `bd --json` output) and surface active task
//! state in context packs — labeled as task state, never as system facts.
//!
//! Input shapes accepted (defensively):
//!
//! - `.beads/issues.jsonl`: one JSON object per line
//! - `bd list/show --json` arrays, with or without the `BD_JSON_ENVELOPE`
//!   wrapper `{"schema_version": 1, "data": ...}`
//!
//! Any object with `id`/`title` (or `name`) is imported.

use scc_store::Store;

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct BeadsReport {
    pub tasks: usize,
    pub dependencies: usize,
    pub active: usize,
    pub errors: usize,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct BeadIssue {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    dependencies: Vec<serde_json::Value>,
    #[serde(default)]
    depends_on: Vec<serde_json::Value>,
    #[serde(default)]
    blocked_by: Vec<serde_json::Value>,
}

impl BeadIssue {
    fn display_id(&self) -> String {
        if !self.id.is_empty() {
            self.id.clone()
        } else {
            self.name.clone()
        }
    }

    fn display_title(&self) -> String {
        if !self.title.is_empty() {
            self.title.clone()
        } else {
            self.name.clone()
        }
    }

    fn deps(&self) -> Vec<String> {
        let mut out = Vec::new();
        for list in [&self.dependencies, &self.depends_on, &self.blocked_by] {
            for d in list {
                if let Some(s) = d.as_str() {
                    out.push(s.to_string());
                } else if let Some(o) = d.as_object() {
                    if let Some(id) = o.get("id").and_then(|v| v.as_str()) {
                        out.push(id.to_string());
                    } else if let Some(t) = o.get("title").and_then(|v| v.as_str()) {
                        out.push(t.to_string());
                    }
                }
            }
        }
        out
    }

    fn is_active(&self) -> bool {
        // "active" task state = in progress; plain "open" issues are queued,
        // not active.
        let s = self.status.to_ascii_lowercase();
        s.contains("in_progress") || s.contains("in-progress") || s == "active"
    }
}

/// Parse beads task records from raw text (JSONL, array, or envelope).
fn parse_records(text: &str) -> Vec<BeadIssue> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    // JSONL: multiple lines each parseable as an object
    if trimmed.contains('\n') {
        let lines: Vec<BeadIssue> = trimmed
            .lines()
            .filter_map(|l| serde_json::from_str::<BeadIssue>(l.trim()).ok())
            .collect();
        if !lines.is_empty() {
            return lines;
        }
    }
    // single object / array / envelope
    let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return Vec::new();
    };
    let data = v
        .get("data")
        .unwrap_or(&v);
    match data {
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|x| serde_json::from_value::<BeadIssue>(x.clone()).ok())
            .collect(),
        serde_json::Value::Object(_) => {
            serde_json::from_value::<BeadIssue>(data.clone()).ok().into_iter().collect()
        }
        _ => Vec::new(),
    }
}

/// Import beads records from a file (`.beads/issues.jsonl` or a `bd --json`
/// capture).
pub fn import_beads(store: &Store, path: &std::path::Path) -> Result<BeadsReport, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("beads: {e}"))?;
    let issues = parse_records(&text);
    let mut report = BeadsReport {
        tasks: issues.len(),
        ..Default::default()
    };
    let mut ids: std::collections::HashMap<String, String> = Default::default();
    for (i, issue) in issues.iter().enumerate() {
        let key = if issue.display_id().is_empty() {
            format!("bead-{}", i)
        } else {
            issue.display_id()
        };
        let id = scc_core::entity_id(&store.repo_id, "task", &key);
        let mut e = scc_core::Entity::new(id.clone(), "task", issue.display_title());
        e.attr("bead_id", serde_json::json!(key));
        e.attr("status", serde_json::json!(issue.status));
        if issue.is_active() {
            e.attr("active", serde_json::json!(true));
            report.active += 1;
        }
        store
            .insert_entity(&e, &[".beads/issues.jsonl".to_string()])
            .map_err(|e| e.to_string())?;
        ids.insert(key, id);
    }
    for (i, issue) in issues.iter().enumerate() {
        let key = if issue.display_id().is_empty() {
            format!("bead-{}", i)
        } else {
            issue.display_id()
        };
        let Some(subject) = ids.get(&key) else { continue };
        for dep in issue.deps() {
            let target = ids
                .get(&dep)
                .cloned()
                .unwrap_or_else(|| scc_core::entity_id(&store.repo_id, "task", &dep));
            let rel = scc_core::Relationship::new(
                crate::write::rel_id(&["bead_depends", subject, &target]),
                subject.clone(),
                scc_core::predicates::DEPENDS_ON,
                target,
                scc_core::Provenance::Extracted,
            );
            store
                .insert_relationship(&rel, ".beads/issues.jsonl")
                .map_err(|e| e.to_string())?;
            report.dependencies += 1;
        }
    }
    Ok(report)
}

/// Read active bead titles from a repo's `.beads/issues.jsonl` (for task-pack
/// enrichment). Returns up to `limit` active task titles.
pub fn active_beads(root: &std::path::Path, limit: usize) -> Vec<String> {
    let path = root.join(".beads/issues.jsonl");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    parse_records(&text)
        .into_iter()
        .filter(|b| b.is_active() && !b.display_title().is_empty())
        .take(limit)
        .map(|b| b.display_title())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp_store() -> (Store, TempDir) {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        (store, dir)
    }

    #[test]
    fn parses_jsonl() {
        let text = "{\"id\":\"bead-1\",\"title\":\"Fix ASR retry\",\"status\":\"in_progress\",\"dependencies\":[\"bead-2\"]}\n{\"id\":\"bead-2\",\"title\":\"Add fallback\",\"status\":\"open\"}\n";
        let issues = parse_records(text);
        assert_eq!(issues.len(), 2);
        assert!(issues[0].is_active());
        assert_eq!(issues[0].deps(), vec!["bead-2"]);
    }

    #[test]
    fn parses_envelope_and_array() {
        let env = r#"{"schema_version":1,"data":[{"id":"a","title":"A","status":"open"}]}"#;
        assert_eq!(parse_records(env).len(), 1);
        let arr = r#"[{"id":"a","title":"A"},{"id":"b","title":"B"}]"#;
        assert_eq!(parse_records(arr).len(), 2);
    }

    #[test]
    fn import_creates_tasks_and_edges() {
        let (store, _d) = tmp_store();
        let dir = TempDir::new().unwrap();
        let f = dir.path().join("issues.jsonl");
        std::fs::write(
            &f,
            "{\"id\":\"b1\",\"title\":\"one\",\"status\":\"in_progress\",\"dependencies\":[\"b2\"]}\n{\"id\":\"b2\",\"title\":\"two\",\"status\":\"open\"}\n",
        )
        .unwrap();
        let report = import_beads(&store, &f).unwrap();
        assert_eq!(report.tasks, 2);
        assert_eq!(report.active, 1);
        assert_eq!(report.dependencies, 1);
        let tasks = store.entities_by_kind("task").unwrap();
        assert_eq!(tasks.len(), 2);
        let rels = store.all_relationships().unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].predicate, scc_core::predicates::DEPENDS_ON);
    }

    #[test]
    fn malformed_input_never_panics() {
        let (store, _d) = tmp_store();
        let dir = TempDir::new().unwrap();
        let f = dir.path().join("bad.jsonl");
        std::fs::write(&f, "not json\n{\"title\": 42}\n").unwrap();
        let report = import_beads(&store, &f).unwrap();
        assert_eq!(report.tasks, 0);
    }

    #[test]
    fn active_beads_reads_repo_file() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".beads")).unwrap();
        std::fs::write(
            root.join(".beads/issues.jsonl"),
            "{\"id\":\"x\",\"title\":\"Active thing\",\"status\":\"in_progress\"}\n{\"id\":\"y\",\"title\":\"Later\",\"status\":\"todo\"}\n",
        )
        .unwrap();
        let active = active_beads(root, 10);
        assert_eq!(active, vec!["Active thing"]);
    }
}
