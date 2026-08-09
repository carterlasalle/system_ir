//! Workflow Compiler (EPIC-050, P1 "system semantics", docs/EPICS_AND_TICKETS.md).
//!
//! Operational workflow views over the compiled sequence flows:
//! 1. intent-declared workflows (re-emitted sequence flows, kind=workflow)
//! 2. branching sequences (collapsed multi-operation steps / branches attr)
//! 3. retry/fallback components (>= 2 retry/fallback/backoff signals)

use crate::components::component_for_path;
use crate::lifecycle::component_candidates;
use crate::{RealityGraph, Result};
use scc_core::kinds;
use scc_core::{entity_id, Entity, Flow, FlowKind, FlowStep, Provenance};
use scc_store::Store;
use serde_json::json;
use std::collections::{BTreeMap, HashSet};

/// Compiled workflow views: intent-declared clones, branching sequence
/// views, and retry/fallback component workflows. Sorted by name.
pub fn compile_workflows(graph: &RealityGraph, store: &Store) -> Result<Vec<Flow>> {
    let mut out: Vec<Flow> = Vec::new();
    let sequences: Vec<Flow> = store
        .flows()?
        .into_iter()
        .filter(|f| f.kind == FlowKind::Sequence)
        .collect();

    // 1. intent-declared workflows: re-emit the compiled sequence flow with
    //    the same name as a Workflow view (new id, workflow namespace).
    let declared: HashSet<String> = store
        .intent_claims()?
        .into_iter()
        .filter(|(source, _)| source == "flow")
        .filter_map(|(_, claim)| {
            if claim.get("kind").and_then(|v| v.as_str()) == Some("workflow") {
                claim.get("name").and_then(|v| v.as_str()).map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect();
    for seq in &sequences {
        if declared.contains(&seq.name) {
            let mut clone = seq.clone();
            clone.kind = FlowKind::Workflow;
            clone.id = entity_id(&store.repo_id, kinds::WORKFLOW, &seq.name);
            out.push(clone);
        }
    }

    // 2. branching sequences: any collapsed multi-operation step (", ") or a
    //    "branches" attribute yields a workflow view; the branch step is
    //    appended when a collapsed step was actually found.
    for seq in &sequences {
        let has_collapsed = seq.steps.iter().any(|s| s.operation.contains(", "));
        let has_branches = seq.attributes.contains_key("branches");
        if !has_collapsed && !has_branches {
            continue;
        }
        let name = format!("{}-workflow", seq.name);
        let mut steps = seq.steps.clone();
        if has_collapsed {
            steps.push(FlowStep {
                id: format!("step:{}", steps.len() + 1),
                order: (steps.len() + 1) as u32,
                actor: "system".to_string(),
                operation: "branch".to_string(),
                condition: Some("multi-path".to_string()),
                r#async: None,
                timeout_ms: None,
                retry_policy: None,
                failure_outcome: None,
                provenance: Some(Provenance::Inferred),
                evidence: Vec::new(),
            });
        }
        out.push(Flow {
            id: entity_id(&store.repo_id, kinds::FLOW, &name),
            kind: FlowKind::Workflow,
            name,
            trigger: seq.trigger.clone(),
            steps,
            attributes: seq.attributes.clone(),
        });
    }

    // 3. retry/fallback components: >= 2 symbols carrying a retry_policy
    //    attribute or a retry/fallback/backoff name.
    let candidates = component_candidates(graph);
    let comp_name_to_id: BTreeMap<String, String> = graph
        .components
        .iter()
        .map(|c| (c.name.clone(), c.id.clone()))
        .collect();
    let mut signals_by_comp: BTreeMap<String, Vec<&Entity>> = BTreeMap::new();
    for e in graph.entities_of_kind(kinds::SYMBOL) {
        let name_l = e.name.to_ascii_lowercase();
        if !e.attributes.contains_key("retry_policy")
            && !name_l.contains("retry")
            && !name_l.contains("fallback")
            && !name_l.contains("backoff")
        {
            continue;
        }
        let comp = e
            .attributes
            .get("file")
            .and_then(|v| v.as_str())
            .map(|f| component_for_path(f, &candidates))
            .unwrap_or_else(|| "root".to_string());
        signals_by_comp.entry(comp).or_default().push(e);
    }
    for (comp, mut signals) in signals_by_comp {
        if signals.len() < 2 {
            continue;
        }
        signals.sort_by(|a, b| a.name.cmp(&b.name));
        let comp_id = comp_name_to_id
            .get(&comp)
            .cloned()
            .unwrap_or_else(|| format!("component:{comp}"));
        let name = format!("{comp}-workflow");

        let mut steps: Vec<FlowStep> = Vec::new();
        steps.push(FlowStep {
            id: "step:1".to_string(),
            order: 1,
            actor: comp_id.clone(),
            operation: comp.clone(),
            condition: None,
            r#async: None,
            timeout_ms: None,
            retry_policy: None,
            failure_outcome: None,
            provenance: Some(Provenance::Inferred),
            evidence: Vec::new(),
        });
        let mut retries = 0usize;
        let mut fallbacks = 0usize;
        for (i, s) in signals.iter().enumerate() {
            let name_l = s.name.to_ascii_lowercase();
            let is_retry = s.attributes.contains_key("retry_policy")
                || name_l.contains("retry")
                || name_l.contains("backoff");
            let is_fallback = name_l.contains("fallback");
            if is_retry {
                retries += 1;
            }
            if is_fallback {
                fallbacks += 1;
            }
            steps.push(FlowStep {
                id: format!("step:{}", i + 2),
                order: (i + 2) as u32,
                actor: comp_id.clone(),
                operation: s.name.clone(),
                condition: None,
                r#async: None,
                timeout_ms: None,
                retry_policy: s
                    .attributes
                    .get("retry_policy")
                    .and_then(|v| v.as_str())
                    .map(|p| p.to_string()),
                failure_outcome: if is_fallback {
                    Some("fallback".to_string())
                } else {
                    None
                },
                provenance: Some(Provenance::Extracted),
                evidence: Vec::new(),
            });
        }

        let mut attributes = BTreeMap::new();
        attributes.insert("retries".to_string(), json!(retries));
        attributes.insert("fallbacks".to_string(), json!(fallbacks));
        out.push(Flow {
            id: entity_id(&store.repo_id, kinds::FLOW, &name),
            kind: FlowKind::Workflow,
            name,
            trigger: None,
            steps,
            attributes,
        });
    }

    // deterministic order; flows table PK is the id, so drop accidental
    // duplicates (first occurrence wins, order is stable)
    let mut seen: HashSet<String> = HashSet::new();
    out.retain(|f| seen.insert(f.id.clone()));
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scc_core::symbol_id;

    fn setup() -> (tempfile::TempDir, Store) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        (dir, store)
    }

    fn put_components(store: &Store, comps: &[(&str, &[&str])]) {
        let entities: Vec<Entity> = comps
            .iter()
            .map(|(name, paths)| {
                let mut c = Entity::new(
                    entity_id(&store.repo_id, kinds::COMPONENT, name),
                    kinds::COMPONENT,
                    *name,
                );
                c.attr("implementation", json!({ "paths": paths, "symbols": [] }));
                c
            })
            .collect();
        store.replace_components(&entities).unwrap();
    }

    fn put_symbol(
        store: &Store,
        name: &str,
        file: &str,
        attrs: &[(&str, serde_json::Value)],
    ) {
        let mut e = Entity::new(symbol_id(&store.repo_id, file, name), kinds::SYMBOL, name);
        e.attr("kind", serde_json::json!("function"));
        e.attr("file", serde_json::json!(file));
        for (k, v) in attrs {
            e.attr(k, v.clone());
        }
        store.insert_entity(&e, &[file.to_string()]).unwrap();
    }

    fn seq_flow(store: &Store, name: &str, ops: &[&str], attrs: serde_json::Value) -> Flow {
        let steps: Vec<FlowStep> = ops
            .iter()
            .enumerate()
            .map(|(i, op)| FlowStep {
                id: format!("step:{}", i + 1),
                order: (i + 1) as u32,
                actor: "actor".to_string(),
                operation: op.to_string(),
                condition: None,
                r#async: None,
                timeout_ms: None,
                retry_policy: None,
                failure_outcome: None,
                provenance: Some(Provenance::Resolved),
                evidence: Vec::new(),
            })
            .collect();
        Flow {
            id: entity_id(&store.repo_id, kinds::FLOW, name),
            kind: FlowKind::Sequence,
            name: name.to_string(),
            trigger: Some("t".to_string()),
            steps,
            attributes: serde_json::from_value(attrs).unwrap(),
        }
    }

    #[test]
    fn intent_declared_workflow_clone() {
        let (_dir, store) = setup();
        let seq = seq_flow(&store, "onboard", &["validate", "create"], json!({}));
        store.replace_flows(&[seq]).unwrap();
        store
            .replace_intent_claims(&[(
                "flow".to_string(),
                json!({ "name": "onboard", "kind": "workflow", "entrypoint": "onboard_user" }),
            )])
            .unwrap();
        let graph = RealityGraph::load(&store).unwrap();

        let flows = compile_workflows(&graph, &store).unwrap();
        assert_eq!(flows.len(), 1);
        let f = &flows[0];
        assert_eq!(f.kind, FlowKind::Workflow);
        assert_eq!(f.name, "onboard");
        assert_eq!(f.id, entity_id(&store.repo_id, kinds::WORKFLOW, "onboard"));
        assert_ne!(f.id, entity_id(&store.repo_id, kinds::FLOW, "onboard"));
        assert_eq!(f.steps.len(), 2);
        assert_eq!(f.steps[0].operation, "validate");
        assert_eq!(f.steps[1].operation, "create");
    }

    #[test]
    fn non_workflow_intent_is_ignored() {
        let (_dir, store) = setup();
        let seq = seq_flow(&store, "checkout", &["validate"], json!({}));
        store.replace_flows(&[seq]).unwrap();
        store
            .replace_intent_claims(&[(
                "flow".to_string(),
                json!({ "name": "checkout", "kind": "sequence", "entrypoint": "checkout_fn" }),
            )])
            .unwrap();
        let graph = RealityGraph::load(&store).unwrap();
        let flows = compile_workflows(&graph, &store).unwrap();
        assert!(flows.is_empty());
    }

    #[test]
    fn collapsed_step_yields_branch_workflow() {
        let (_dir, store) = setup();
        // "charge, refund" is a collapsed multi-operation step
        let seq = seq_flow(
            &store,
            "checkout",
            &["validate", "charge, refund"],
            json!({}),
        );
        store.replace_flows(&[seq]).unwrap();
        let graph = RealityGraph::load(&store).unwrap();

        let flows = compile_workflows(&graph, &store).unwrap();
        assert_eq!(flows.len(), 1);
        let f = &flows[0];
        assert_eq!(f.kind, FlowKind::Workflow);
        assert_eq!(f.name, "checkout-workflow");
        assert_eq!(f.steps.len(), 3);
        assert_eq!(f.steps[0].operation, "validate");
        assert_eq!(f.steps[1].operation, "charge, refund");
        assert_eq!(f.steps[2].operation, "branch");
        assert_eq!(f.steps[2].condition.as_deref(), Some("multi-path"));
        assert_eq!(f.steps[2].order, 3);
    }

    #[test]
    fn branches_attribute_yields_workflow_without_branch_step() {
        let (_dir, store) = setup();
        let seq = seq_flow(
            &store,
            "pipeline",
            &["validate", "execute"],
            json!({ "branches": ["fast", "full"] }),
        );
        store.replace_flows(&[seq]).unwrap();
        let graph = RealityGraph::load(&store).unwrap();

        let flows = compile_workflows(&graph, &store).unwrap();
        assert_eq!(flows.len(), 1);
        let f = &flows[0];
        assert_eq!(f.name, "pipeline-workflow");
        assert_eq!(f.steps.len(), 2, "no collapsed step -> no branch step appended");
        assert_eq!(f.attributes["branches"], json!(["fast", "full"]));
    }

    #[test]
    fn retry_fallback_component_workflow() {
        let (_dir, store) = setup();
        put_components(&store, &[("ingest", &["ingest"])]);
        put_symbol(&store, "retry_upload", "ingest/upload.py", &[]);
        put_symbol(
            &store,
            "retry_download",
            "ingest/download.py",
            &[("retry_policy", serde_json::json!("exponential"))],
        );
        put_symbol(&store, "fallback_queue", "ingest/queue.py", &[]);
        // below the >= 2 threshold for this component
        put_components(&store, &[("ingest", &["ingest"]), ("jobs", &["jobs"])]);
        put_symbol(&store, "retry_job", "jobs/run.py", &[]);
        let graph = RealityGraph::load(&store).unwrap();

        let flows = compile_workflows(&graph, &store).unwrap();
        assert_eq!(flows.len(), 1);
        let f = &flows[0];
        assert_eq!(f.kind, FlowKind::Workflow);
        assert_eq!(f.name, "ingest-workflow");
        assert_eq!(f.id, entity_id(&store.repo_id, kinds::FLOW, "ingest-workflow"));
        assert_eq!(f.steps.len(), 4);
        assert_eq!(f.steps[0].operation, "ingest");
        assert_eq!(f.steps[0].provenance, Some(Provenance::Inferred));
        assert_eq!(f.steps[0].actor, entity_id(&store.repo_id, kinds::COMPONENT, "ingest"));
        let step = |op: &str| f.steps.iter().find(|s| s.operation == op).unwrap();
        assert_eq!(step("retry_upload").retry_policy, None);
        assert_eq!(step("retry_upload").failure_outcome, None);
        assert_eq!(step("retry_download").retry_policy.as_deref(), Some("exponential"));
        assert_eq!(step("retry_download").failure_outcome, None);
        assert_eq!(step("fallback_queue").retry_policy, None);
        assert_eq!(step("fallback_queue").failure_outcome.as_deref(), Some("fallback"));
        for s in &f.steps[1..] {
            assert_eq!(s.provenance, Some(Provenance::Extracted));
        }
        assert_eq!(f.attributes["retries"], json!(2));
        assert_eq!(f.attributes["fallbacks"], json!(1));
    }

    #[test]
    fn workflow_views_sorted_by_name() {
        let (_dir, store) = setup();
        // retry component "b" (flow "b-workflow") + branch flow "a" (flow "a-workflow")
        put_components(&store, &[("b", &["b"])]);
        put_symbol(&store, "retry_one", "b/x.py", &[]);
        put_symbol(&store, "backoff_two", "b/y.py", &[]);
        let seq = seq_flow(&store, "a", &["x, y"], json!({}));
        store.replace_flows(&[seq]).unwrap();
        let graph = RealityGraph::load(&store).unwrap();

        let flows = compile_workflows(&graph, &store).unwrap();
        assert_eq!(flows.len(), 2);
        let names: Vec<&str> = flows.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["a-workflow", "b-workflow"]);
        assert_eq!(flows[0].steps.last().unwrap().operation, "branch");
    }
}
