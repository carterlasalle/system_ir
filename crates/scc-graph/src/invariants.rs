//! Invariants and drift (EPIC-180, docs/SYSTEM_IR_SCHEMA.md §8/§10).
//!
//! Invariants come from `.scc/intent.yaml` (DECLARED). Drift findings:
//! - declared component with no files/symbols (missing)
//! - declared ownership not exercised by any write edge (violated)
//! - critical invariants lacking an enforcing test (unenforced)
//! - conflicting authoritative writers of the same store (ownership conflict)

use crate::{RealityGraph, Result};
use scc_core::kinds;
use scc_core::{entity_id, Invariant, Provenance, Severity};
use scc_store::Store;

pub fn compile_invariants(
    graph: &RealityGraph,
    intent: &[(String, serde_json::Value)],
) -> Result<Vec<Invariant>> {
    let mut out = Vec::new();
    let repo_id = &graph.repo_id;
    for (source, claim) in intent {
        if source != "invariant" {
            continue;
        }
        let name = claim["name"].as_str().unwrap_or("").to_string();
        let statement = claim["statement"].as_str().unwrap_or("").to_string();
        if name.is_empty() || statement.is_empty() {
            continue;
        }
        let severity = match claim["severity"].as_str().unwrap_or("critical") {
            "info" => Severity::Info,
            "low" => Severity::Low,
            "medium" => Severity::Medium,
            "high" => Severity::High,
            "critical" => Severity::Critical,
            _ => Severity::Critical,
        };
        let mut scope: Vec<String> = Vec::new();
        if let Some(s) = claim["scope"].as_array() {
            for x in s {
                if let Some(id) = x.as_str() {
                    // scope entries may be entity names or ids
                    let lowered = id.to_ascii_lowercase();
                    let matched = graph
                        .entities_of_kind(kinds::DATA_ENTITY)
                        .into_iter()
                        .chain(graph.entities_of_kind(kinds::DATA_STORE))
                        .chain(graph.entities_of_kind(kinds::COMPONENT))
                        .find(|e| {
                            e.name.to_ascii_lowercase() == lowered || e.id == *id
                        })
                        .map(|e| e.id.clone());
                    scope.push(matched.unwrap_or_else(|| id.to_string()));
                }
            }
        }
        let mut enforced_by: Vec<String> = Vec::new();
        if let Some(e) = claim["enforced_by"].as_array() {
            for x in e {
                if let Some(t) = x.as_str() {
                    enforced_by.push(t.to_string());
                }
            }
        }
        out.push(Invariant {
            id: entity_id(repo_id, kinds::INVARIANT, &name),
            statement,
            severity,
            scope,
            enforced_by,
            provenance: Some(Provenance::Declared),
            evidence: vec!["intent:.scc/intent.yaml".into()],
        });
    }
    Ok(out)
}

pub fn drift_findings(
    graph: &RealityGraph,
    store: &Store,
    intent: &[(String, serde_json::Value)],
    components: &[scc_core::Entity],
) -> Result<Vec<(String, String, String)>> {
    let mut findings: Vec<(String, String, String)> = Vec::new();

    // declared component missing (no files/symbols)
    for (source, claim) in intent {
        if source != "component" {
            continue;
        }
        let name = claim["name"].as_str().unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }
        let exists = components.iter().any(|c| c.name == name);
        if !exists {
            findings.push((
                "declared_component_missing".into(),
                "high".into(),
                format!("Intent declares component '{name}' but no such component exists in the repository"),
            ));
        } else {
            // declared ownership violated
            if let Some(owns) = claim["owns"].as_array() {
                let comp = components.iter().find(|c| c.name == name).unwrap();
                let owned_ids: Vec<String> = comp
                    .attributes
                    .get("owns")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|o| o.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                for target in owns {
                    if let Some(t) = target.as_str() {
                        let matched = graph
                            .entities_of_kind(kinds::DATA_STORE)
                            .into_iter()
                            .chain(graph.entities_of_kind(kinds::DATA_ENTITY))
                            .any(|e| e.name.eq_ignore_ascii_case(t));
                        if matched && !owned_ids.iter().any(|oid| {
                            graph
                                .entities
                                .get(oid)
                                .map(|e| e.name.eq_ignore_ascii_case(t))
                                .unwrap_or(false)
                        }) {
                            findings.push((
                                "declared_ownership_violated".into(),
                                "high".into(),
                                format!("Component '{name}' declares ownership of '{t}' but no write edge supports it"),
                            ));
                        }
                    }
                }
            }
        }
    }

    // critical invariants lacking enforcing tests
    for inv in store.invariants()? {
        if inv.severity == Severity::Critical && inv.enforced_by.is_empty() {
            findings.push((
                "invariant_unenforced".into(),
                "high".into(),
                format!(
                    "Critical invariant '{}' has no enforcing test declared",
                    inv.statement
                ),
            ));
        } else if !inv.enforced_by.is_empty() {
            for t in &inv.enforced_by {
                let found = graph
                    .entities_of_kind(kinds::TEST)
                    .iter()
                    .any(|e| e.name.contains(t) || e.id.ends_with(t));
                if !found {
                    findings.push((
                        "invariant_test_missing".into(),
                        "medium".into(),
                        format!(
                            "Invariant '{}' declares enforcing test '{}' but no such test exists",
                            inv.statement, t
                        ),
                    ));
                }
            }
        }
    }

    // declared flow entrypoint missing / sink unreachable
    for (source, claim) in intent {
        if source != "flow" {
            continue;
        }
        let name = claim["name"].as_str().unwrap_or("").to_string();
        let entrypoint = claim["entrypoint"].as_str().unwrap_or("").to_string();
        if name.is_empty() || entrypoint.is_empty() {
            continue;
        }
        let entry_found = graph
            .entities_of_kind(kinds::SYMBOL)
            .iter()
            .any(|e| e.name == entrypoint);
        if !entry_found {
            findings.push((
                "declared_flow_entrypoint_missing".into(),
                "high".into(),
                format!(
                    "Declared flow '{name}' names entrypoint '{entrypoint}' but no such symbol exists"
                ),
            ));
            continue;
        }
        // compiled sequence flow for this entrypoint should reach a sink
        // (>= 2 steps means the entry has at least one resolved call path)
        let flow = graph.flows.iter().find(|f| f.name == name);
        match flow {
            Some(f) if f.steps.len() < 2 => findings.push((
                "flow_sink_unreachable".into(),
                "medium".into(),
                format!(
                    "Declared flow '{name}' never reaches a resolved sink (entrypoint '{}' has no resolved call path)",
                    entrypoint
                ),
            )),
            None => findings.push((
                "declared_flow_not_compiled".into(),
                "medium".into(),
                format!("Declared flow '{name}' was not compiled into any machine view"),
            )),
            _ => {}
        }
    }

    // declared ownership target with no matching entity at all
    for (source, claim) in intent {
        if source != "component" {
            continue;
        }
        let name = claim["name"].as_str().unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }
        if let Some(owns) = claim["owns"].as_array() {
            for target in owns {
                if let Some(t) = target.as_str() {
                    let exists = graph
                        .entities_of_kind(kinds::DATA_STORE)
                        .into_iter()
                        .chain(graph.entities_of_kind(kinds::DATA_ENTITY))
                        .any(|e| e.name.eq_ignore_ascii_case(t));
                    if !exists {
                        findings.push((
                            "declared_ownership_target_missing".into(),
                            "medium".into(),
                            format!(
                                "Component '{name}' declares ownership of '{t}' but no store or data entity matches"
                            ),
                        ));
                    }
                }
            }
        }
    }

    // conflicting authoritative writers of a store
    let mut writers: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for r in graph.all_rels() {
        if r.predicate == scc_core::predicates::WRITES {
            let subject = r.subject.clone();
            if let Some(comp) = components.iter().find(|c| {
                // component contains subject via symbol_component mapping — approximated by
                // checking subject's file attribute against component paths
                graph
                    .entities
                    .get(&subject)
                    .and_then(|e| e.attributes.get("file"))
                    .and_then(|f| f.as_str())
                    .map(|f| {
                        c.attributes
                            .get("implementation")
                            .and_then(|i| i.get("paths"))
                            .and_then(|p| p.as_array())
                            .map(|paths| {
                                paths.iter().any(|p| {
                                    p.as_str().map(|pd| f.starts_with(&format!("{pd}/")) || f == pd).unwrap_or(false)
                                })
                            })
                            .unwrap_or(false)
                    })
                    .unwrap_or(false)
            }) {
                writers
                    .entry(r.object.clone())
                    .or_default()
                    .push(comp.name.clone());
            }
        }
    }
    for (store_id, comps) in writers {
        let mut uniq: Vec<String> = comps.clone();
        uniq.sort();
        uniq.dedup();
        if uniq.len() > 1 {
            findings.push((
                "conflicting_writers".into(),
                "medium".into(),
                format!(
                    "Store '{}' has multiple authoritative writers: {}",
                    store_id.split('/').next_back().unwrap_or(&store_id),
                    uniq.join(", ")
                ),
            ));
        }
    }

    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invariants_from_intent() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let g = RealityGraph::load(&store).unwrap();
        let intent = vec![(
            "invariant".to_string(),
            serde_json::json!({
                "name": "raw-immutable",
                "statement": "raw output cannot be modified",
                "severity": "critical",
            }),
        )];
        let invs = compile_invariants(&g, &intent).unwrap();
        assert_eq!(invs.len(), 1);
        assert_eq!(invs[0].severity, Severity::Critical);
        assert_eq!(invs[0].provenance, Some(Provenance::Declared));
    }
}
