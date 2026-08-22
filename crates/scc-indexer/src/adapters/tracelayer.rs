//! TraceLayer adapter: import trace markers from source files and emit
//! structured facts in System IR — requirements, work items, implementation
//! links, test verification links, and decision/ADR links.
//!
//! Input: Source files containing trace markers (grep-friendly format).
//! Output: System IR entities and relationships for traceability.

use scc_store::Store;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Import report for TraceLayer markers.
#[derive(Debug, Clone, Default, serde::Serialize)]
// trace:v1 id=impl.crates-scc-indexer-src-adapters-tracelayer.trace-layer-report
pub struct TraceLayerReport {
    pub requirements: usize,
    pub work_items: usize,
    pub implementations: usize,
    pub tests: usize,
    pub decisions: usize,
    pub relationships: usize,
    pub errors: usize,
}

/// Parsed trace marker.
#[derive(Debug, Clone)]
// trace:v1 id=impl.crates-scc-indexer-src-adapters-tracelayer.trace-marker
struct TraceMarker {
    id: String,
    properties: HashMap<String, String>,
}

// trace:v1 id=impl.crates-scc-indexer-src-adapters-tracelayer.trace-marker-impl
impl TraceMarker {
    /// Extract a property value.
    // trace:v1 id=impl.crates-scc-indexer-src-adapters-tracelayer.get
    fn get(&self, key: &str) -> Option<&str> {
        self.properties.get(key).map(|s| s.as_str())
    }

    /// Check if this is a requirement marker.
    // trace:v1 id=impl.crates-scc-indexer-src-adapters-tracelayer.is-requirement
    fn is_requirement(&self) -> bool {
        self.get("type") == Some("requirement")
    }

    /// Check if this is a decision/ADR marker.
    // trace:v1 id=impl.crates-scc-indexer-src-adapters-tracelayer.is-decision
    fn is_decision(&self) -> bool {
        self.get("type") == Some("decision")
    }

    /// Check if this is an implementation marker (satisfies a requirement).
    // trace:v1 id=impl.crates-scc-indexer-src-adapters-tracelayer.is-implementation
    fn is_implementation(&self) -> bool {
        self.get("satisfies").is_some()
    }

    /// Check if this is a test marker (verifies a requirement).
    // trace:v1 id=impl.crates-scc-indexer-src-adapters-tracelayer.is-test
    fn is_test(&self) -> bool {
        self.get("verifies").is_some()
    }

    /// Get the work item ID.
    // trace:v1 id=impl.crates-scc-indexer-src-adapters-tracelayer.work-id
    fn work_id(&self) -> Option<&str> {
        self.get("work")
    }

    /// Get the satisfied requirement IDs.
    // trace:v1 id=impl.crates-scc-indexer-src-adapters-tracelayer.satisfies
    fn satisfies(&self) -> Vec<String> {
        self.get("satisfies")
            .map(|s| s.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default()
    }

    /// Get the verified requirement IDs.
    // trace:v1 id=impl.crates-scc-indexer-src-adapters-tracelayer.verifies
    fn verifies(&self) -> Vec<String> {
        self.get("verifies")
            .map(|s| s.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default()
    }

    /// Get the exercised implementation IDs.
    // trace:v1 id=impl.crates-scc-indexer-src-adapters-tracelayer.exercises
    fn exercises(&self) -> Vec<String> {
        self.get("exercises")
            .map(|s| s.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default()
    }

    /// Get the addressed requirement IDs.
    // trace:v1 id=impl.crates-scc-indexer-src-adapters-tracelayer.addresses
    fn addresses(&self) -> Vec<String> {
        self.get("addresses")
            .map(|s| s.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default()
    }
}

/// Parse trace markers from a file.
// trace:v1 id=impl.crates-scc-indexer-src-adapters-tracelayer.parse-markers
fn parse_markers(content: &str) -> Vec<TraceMarker> {
    let mut markers = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Look for trace marker (single-line comment format)
        if let Some(marker_str) = trimmed
            .strip_prefix("// trace:v1 ")
            .or_else(|| trimmed.strip_prefix("# trace:v1 "))
            .or_else(|| trimmed.strip_prefix("<!-- trace:v1 "))
            .or_else(|| trimmed.strip_prefix("trace:v1 "))
        {
            // Remove trailing comment marker if present
            let marker_str = marker_str
                .strip_suffix(" -->")
                .unwrap_or(marker_str);

            // Parse key=value pairs
            let mut properties = HashMap::new();
            let mut id = None;

            for part in marker_str.split_whitespace() {
                if let Some((key, value)) = part.split_once('=') {
                    properties.insert(key.to_string(), value.to_string());
                    if key == "id" {
                        id = Some(value.to_string());
                    }
                }
            }

            if let Some(id) = id {
                markers.push(TraceMarker { id, properties });
            }
        }
    }

    markers
}

/// Import trace markers from a source file.
// trace:v1 id=impl.crates-scc-indexer-src-adapters-tracelayer.import-tracelayer
pub fn import_tracelayer(store: &Store, path: &Path) -> Result<TraceLayerReport, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("tracelayer: failed to read {}: {e}", path.display()))?;

    let markers = parse_markers(&content);
    let mut report = TraceLayerReport::default();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let path_str = path.to_string_lossy().to_string();

    // First pass: create entities
    for marker in &markers {
        if seen_ids.contains(&marker.id) {
            continue;
        }
        seen_ids.insert(marker.id.clone());

        if marker.is_requirement() {
            // Create requirement entity
            let title = marker.get("title").unwrap_or(&marker.id);
            let entity_id = scc_core::entity_id(&store.repo_id, "requirement", &marker.id);
            let mut entity = scc_core::Entity::new(entity_id, "requirement", title);
            entity.attr("trace_id", serde_json::json!(marker.id));
            if let Some(work) = marker.work_id() {
                entity.attr("work", serde_json::json!(work));
            }
            store
                .insert_entity(&entity, std::slice::from_ref(&path_str))
                .map_err(|e| e.to_string())?;
            report.requirements += 1;
        } else if marker.is_decision() {
            // Create decision entity
            let title = marker.get("title").unwrap_or(&marker.id);
            let entity_id = scc_core::entity_id(&store.repo_id, "decision", &marker.id);
            let mut entity = scc_core::Entity::new(entity_id, "decision", title);
            entity.attr("trace_id", serde_json::json!(marker.id));
            store
                .insert_entity(&entity, std::slice::from_ref(&path_str))
                .map_err(|e| e.to_string())?;
            report.decisions += 1;
        } else if marker.is_implementation() {
            // Create implementation entity
            let entity_id = scc_core::entity_id(&store.repo_id, "implementation", &marker.id);
            let mut entity = scc_core::Entity::new(entity_id, "implementation", &marker.id);
            entity.attr("trace_id", serde_json::json!(marker.id));
            if let Some(work) = marker.work_id() {
                entity.attr("work", serde_json::json!(work));
            }
            store
                .insert_entity(&entity, std::slice::from_ref(&path_str))
                .map_err(|e| e.to_string())?;
            report.implementations += 1;
        } else if marker.is_test() {
            // Create test entity
            let entity_id = scc_core::entity_id(&store.repo_id, "test", &marker.id);
            let mut entity = scc_core::Entity::new(entity_id, "test", &marker.id);
            entity.attr("trace_id", serde_json::json!(marker.id));
            store
                .insert_entity(&entity, std::slice::from_ref(&path_str))
                .map_err(|e| e.to_string())?;
            report.tests += 1;
        } else if marker.work_id().is_some() {
            // Create work item entity
            let work_id = marker.work_id().unwrap();
            let entity_id = scc_core::entity_id(&store.repo_id, "work", work_id);
            let mut entity = scc_core::Entity::new(entity_id, "work", work_id);
            entity.attr("trace_id", serde_json::json!(marker.id));
            store
                .insert_entity(&entity, std::slice::from_ref(&path_str))
                .map_err(|e| e.to_string())?;
            report.work_items += 1;
        }
    }

    // Second pass: create relationships
    for marker in &markers {
        let subject_id = scc_core::entity_id(&store.repo_id, entity_type(marker), &marker.id);

        // satisfies relationships (implementation -> requirement)
        for req_id in marker.satisfies() {
            let object_id = scc_core::entity_id(&store.repo_id, "requirement", &req_id);
            let rel_id = scc_core::predicates::IMPLEMENTED_BY;
            let rel = scc_core::Relationship::new(
                crate::write::rel_id(&[&subject_id, rel_id, &object_id]),
                subject_id.clone(),
                rel_id,
                object_id,
                scc_core::Provenance::Extracted,
            );
            store
                .insert_relationship(&rel, &path_str)
                .map_err(|e| e.to_string())?;
            report.relationships += 1;
        }

        // verifies relationships (test -> requirement)
        for req_id in marker.verifies() {
            let object_id = scc_core::entity_id(&store.repo_id, "requirement", &req_id);
            let rel_id = scc_core::predicates::TESTED_BY;
            let rel = scc_core::Relationship::new(
                crate::write::rel_id(&[&subject_id, rel_id, &object_id]),
                subject_id.clone(),
                rel_id,
                object_id,
                scc_core::Provenance::Extracted,
            );
            store
                .insert_relationship(&rel, &path_str)
                .map_err(|e| e.to_string())?;
            report.relationships += 1;
        }

        // exercises relationships (test -> implementation) - uses IMPLEMENTED_BY as proxy
        for impl_id in marker.exercises() {
            let object_id = scc_core::entity_id(&store.repo_id, "implementation", &impl_id);
            let rel_id = scc_core::predicates::IMPLEMENTED_BY;
            let rel = scc_core::Relationship::new(
                crate::write::rel_id(&[&subject_id, rel_id, &object_id]),
                subject_id.clone(),
                rel_id,
                object_id,
                scc_core::Provenance::Extracted,
            );
            store
                .insert_relationship(&rel, &path_str)
                .map_err(|e| e.to_string())?;
            report.relationships += 1;
        }

        // addresses relationships (decision -> requirement) - uses IMPLEMENTED_BY as proxy
        for req_id in marker.addresses() {
            let object_id = scc_core::entity_id(&store.repo_id, "requirement", &req_id);
            let rel_id = scc_core::predicates::IMPLEMENTED_BY;
            let rel = scc_core::Relationship::new(
                crate::write::rel_id(&[&subject_id, rel_id, &object_id]),
                subject_id.clone(),
                rel_id,
                object_id,
                scc_core::Provenance::Extracted,
            );
            store
                .insert_relationship(&rel, &path_str)
                .map_err(|e| e.to_string())?;
            report.relationships += 1;
        }
    }

    Ok(report)
}

/// Get entity type for a marker.
// trace:v1 id=impl.crates-scc-indexer-src-adapters-tracelayer.entity-type
fn entity_type(marker: &TraceMarker) -> &'static str {
    if marker.is_requirement() {
        "requirement"
    } else if marker.is_decision() {
        "decision"
    } else if marker.is_implementation() {
        "implementation"
    } else if marker.is_test() {
        "test"
    } else if marker.work_id().is_some() {
        "work"
    } else {
        "unknown"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // trace:v1 id=impl.crates-scc-indexer-src-adapters-tracelayer.test-parse-markers
    fn test_parse_markers() {
        // trace:exempt reason=test-data - test string contains trace markers for testing
        let content = "// trace:v1 id=REQ-SCC-001 type=requirement title=\"System IR schema\"\npub fn system_ir_schema() {}\n\n// trace:v1 id=impl.scc.core work=WORK-SCC-001 satisfies=REQ-SCC-001\npub fn core_implementation() {}\n\n// trace:v1 id=test.scc.core verifies=REQ-SCC-001 exercises=impl.scc.core\nfn test_core() {}\n\n// trace:v1 id=ADR-0042 type=decision addresses=REQ-SCC-001\n// Architecture decision\n";

        let markers = parse_markers(content);
        assert_eq!(markers.len(), 4);

        // Check requirement marker
        let req = &markers[0];
        assert_eq!(req.id, "REQ-SCC-001");
        assert!(req.is_requirement());
        assert_eq!(req.get("title"), Some("System IR schema"));

        // Check implementation marker
        let impl_marker = &markers[1];
        assert_eq!(impl_marker.id, "impl.scc.core");
        assert!(impl_marker.is_implementation());
        assert_eq!(impl_marker.satisfies(), vec!["REQ-SCC-001"]);

        // Check test marker
        let test_marker = &markers[2];
        assert_eq!(test_marker.id, "test.scc.core");
        assert!(test_marker.is_test());
        assert_eq!(test_marker.verifies(), vec!["REQ-SCC-001"]);
        assert_eq!(test_marker.exercises(), vec!["impl.scc.core"]);

        // Check decision marker
        let decision = &markers[3];
        assert_eq!(decision.id, "ADR-0042");
        assert!(decision.is_decision());
        assert_eq!(decision.addresses(), vec!["REQ-SCC-001"]);
    }
}
