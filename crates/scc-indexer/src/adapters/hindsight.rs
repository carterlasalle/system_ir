//! Hindsight adapter (SCC-204): import durable lessons from a Hindsight
//! memory-bank export (JSON/JSONL of memories) and surface them in context
//! packs BELOW the System IR authority line — labeled as memory, never as
//! deterministic facts (docs §48).

use scc_store::Store;

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct HindsightReport {
    pub lessons: usize,
    pub errors: usize,
}

/// A durable lesson record (accepts the common Hindsight bank shapes).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Lesson {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub memory: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub importance: Option<f64>,
}

impl Lesson {
    pub fn body(&self) -> String {
        self.text
            .clone()
            .or_else(|| self.content.clone())
            .or_else(|| self.memory.clone())
            .unwrap_or_default()
    }
}

/// Import a hindsight bank export: a JSONL file (one lesson per line) or a
/// JSON array. Stored as entities kind=lesson with INFERRED provenance.
pub fn import_hindsight(store: &Store, path: &std::path::Path) -> Result<HindsightReport, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("hindsight: {e}"))?;
    let lessons = parse_lessons(&text);
    let mut report = HindsightReport {
        lessons: lessons.len(),
        ..Default::default()
    };
    let mut n = 0usize;
    for lesson in lessons {
        let body = lesson.body();
        if body.is_empty() {
            report.errors += 1;
            continue;
        }
        n += 1;
        let id = lesson
            .id
            .clone()
            .unwrap_or_else(|| format!("lesson-{n}"));
        let mut e = scc_core::Entity::new(
            scc_core::entity_id(&store.repo_id, "lesson", &id),
            "lesson",
            body.chars().take(80).collect::<String>(),
        );
        e.attr("content", serde_json::json!(body));
        e.attr("provenance", serde_json::json!("INFERRED"));
        e.attr("source", serde_json::json!("hindsight"));
        if let Some(k) = lesson.kind {
            e.attr("lesson_kind", serde_json::json!(k));
        }
        if let Some(imp) = lesson.importance {
            e.attr("importance", serde_json::json!(imp));
        }
        if !lesson.tags.is_empty() {
            e.attr("tags", serde_json::json!(lesson.tags));
        }
        store
            .insert_entity(&e, &["hindsight:bank".into()])
            .map_err(|e| e.to_string())?;
    }
    Ok(report)
}

fn parse_lessons(text: &str) -> Vec<Lesson> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if trimmed.contains('\n') {
        let lines: Vec<Lesson> = trimmed
            .lines()
            .filter_map(|l| serde_json::from_str::<Lesson>(l.trim()).ok())
            .collect();
        if !lines.is_empty() {
            return lines;
        }
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return Vec::new();
    };
    match v {
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|x| serde_json::from_value::<Lesson>(x.clone()).ok())
            .collect(),
        serde_json::Value::Object(ref o) => {
            // possible wrappers: {"memories": [...]}, {"data": [...]}
            for key in ["memories", "data", "items"] {
                if let Some(arr) = o.get(key).and_then(|v| v.as_array()) {
                    return arr
                        .iter()
                        .filter_map(|x| serde_json::from_value::<Lesson>(x.clone()).ok())
                        .collect();
                }
            }
            serde_json::from_value::<Lesson>(v.clone())
                .ok()
                .into_iter()
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Read back stored lessons (for context-pack enrichment). Returns
/// `(content, tags)` pairs, most important first.
pub fn lessons(store: &Store, limit: usize) -> Vec<(String, Vec<String>)> {
    let mut lessons: Vec<(String, Vec<String>)> = store
        .entities_by_kind("lesson")
        .unwrap_or_default()
        .into_iter()
        .map(|e| {
            let content = e
                .attributes
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or(&e.name)
                .to_string();
            let tags = e
                .attributes
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|t| t.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            (content, tags)
        })
        .collect();
    lessons.truncate(limit);
    lessons
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
    fn parses_jsonl_and_arrays() {
        let text = "{\"id\":\"1\",\"text\":\"tenacity retry works\",\"tags\":[\"retry\"]}\n{\"id\":\"2\",\"content\":\"always reindex before verify\"}\n";
        let lessons = parse_lessons(text);
        assert_eq!(lessons.len(), 2);
        assert_eq!(lessons[0].body(), "tenacity retry works");
        let arr = r#"[{"text":"a"},{"text":"b"}]"#;
        assert_eq!(parse_lessons(arr).len(), 2);
        let wrapped = r#"{"memories":[{"text":"x"}]}"#;
        assert_eq!(parse_lessons(wrapped).len(), 1);
    }

    #[test]
    fn import_and_read_back() {
        let (store, _d) = tmp_store();
        let dir = TempDir::new().unwrap();
        let f = dir.path().join("bank.jsonl");
        std::fs::write(
            &f,
            "{\"id\":\"l1\",\"text\":\"lesson one\",\"tags\":[\"a\"]}\n{\"id\":\"l2\",\"text\":\"lesson two\"}\n",
        )
        .unwrap();
        let report = import_hindsight(&store, &f).unwrap();
        assert_eq!(report.lessons, 2);
        let lessons = lessons(&store, 10);
        assert_eq!(lessons.len(), 2);
        assert_eq!(lessons[0].1, vec!["a"]);
        let entities = store.entities_by_kind("lesson").unwrap();
        assert_eq!(entities[0].attributes["provenance"], "INFERRED");
    }

    #[test]
    fn malformed_never_panics() {
        let (store, _d) = tmp_store();
        let dir = TempDir::new().unwrap();
        let f = dir.path().join("bad.json");
        std::fs::write(&f, "garbage\n{\"id\":42}\n").unwrap();
        let report = import_hindsight(&store, &f).unwrap();
        assert_eq!(report.lessons, 0);
    }
}
