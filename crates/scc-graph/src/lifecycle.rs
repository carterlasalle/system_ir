//! Lifecycle Compiler (EPIC-050, P1 "system semantics", docs/EPICS_AND_TICKETS.md).
//!
//! Detects state machines deterministically from the symbol index and emits
//! one Lifecycle flow per component that owns >= 2 state-machine signals.
//!
//! Signals: class/enum symbols with state-machine-ish names, signatures that
//! mention `Enum`/`StrEnum`, and transition-verb symbol names.

use crate::components::{component_for_path, ComponentCandidate};
use crate::{RealityGraph, Result};
use scc_core::kinds;
use scc_core::{entity_id, Entity, Flow, FlowKind, FlowStep, Provenance};
use scc_store::Store;
use serde_json::json;
use std::collections::BTreeMap;

/// Component candidates built from compiled component implementation paths
/// (longest dir-prefix match wins, same rule as flows.rs).
pub fn component_candidates(graph: &RealityGraph) -> Vec<ComponentCandidate> {
    let mut candidates: Vec<ComponentCandidate> = Vec::new();
    for c in &graph.components {
        let mut dirs: Vec<String> = Vec::new();
        if let Some(paths) = c
            .attributes
            .get("implementation")
            .and_then(|i| i.get("paths"))
            .and_then(|p| p.as_array())
        {
            for p in paths {
                if let Some(s) = p.as_str() {
                    dirs.push(s.to_string());
                }
            }
        }
        if dirs.is_empty() {
            dirs.push(c.name.clone());
        }
        candidates.push(ComponentCandidate {
            name: c.name.clone(),
            dirs,
        });
    }
    candidates
}

/// A symbol is a state-machine signal when its kind/name/signature matches a
/// state-machine convention or its name is a transition verb.
fn is_signal(e: &Entity) -> bool {
    let name_l = e.name.to_ascii_lowercase();
    let sym_kind = e.attributes.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    // (a) class/enum whose name ends with State/Status/Stage or contains
    //     StateMachine/Machine/FSM
    if matches!(sym_kind, "class" | "enum")
        && (name_l.ends_with("state")
            || name_l.ends_with("status")
            || name_l.ends_with("stage")
            || name_l.contains("statemachine")
            || name_l.contains("machine")
            || name_l.contains("fsm"))
        {
            return true;
        }
    // (b) signature mentions Enum/StrEnum
    if let Some(sig) = e.attributes.get("signature").and_then(|v| v.as_str()) {
        if sig.contains("Enum") || sig.contains("StrEnum") {
            return true;
        }
    }
    // (c) transition-verb name
    is_transition_verb(&e.name)
}

/// The unqualified name of a symbol: methods are stored as
/// `Class.method`, and verb detection must look at the method part.
fn base_name(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

fn is_transition_verb(name: &str) -> bool {
    let n = base_name(name).to_ascii_lowercase();
    matches!(
        n.as_str(),
        "advance"
            | "transition"
            | "set_status"
            | "set_state"
            | "move_to"
            | "next_state"
            | "retry"
            | "cancel"
            | "fail"
            | "complete"
            | "pause"
            | "resume"
    ) || n.starts_with("mark_")
}

/// Map a transition verb to its lifecycle condition category.
fn verb_category(name: &str) -> &'static str {
    let n = base_name(name).to_ascii_lowercase();
    if n == "retry" {
        "retry"
    } else if n.starts_with("mark_") || n.starts_with("set_") {
        "event"
    } else if n.starts_with("transition")
        || n.starts_with("advance")
        || n.starts_with("move")
        || n.starts_with("next")
    {
        "transition"
    } else if n == "cancel" || n == "fail" || n == "complete" {
        "terminal outcome"
    } else {
        // pause/resume and state-named signals stay plain "state"
        "state"
    }
}

/// Emit one Lifecycle flow per component with >= 2 state-machine signals.
pub fn compile_lifecycles(graph: &RealityGraph, store: &Store) -> Result<Vec<Flow>> {
    let candidates = component_candidates(graph);
    let comp_name_to_id: BTreeMap<String, String> = graph
        .components
        .iter()
        .map(|c| (c.name.clone(), c.id.clone()))
        .collect();

    // symbol file -> component (same mapping as flows.rs)
    let mut signals_by_comp: BTreeMap<String, Vec<&Entity>> = BTreeMap::new();
    for e in graph.entities_of_kind(kinds::SYMBOL) {
        if !is_signal(e) {
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

    let mut out: Vec<Flow> = Vec::new();
    for (comp, mut signals) in signals_by_comp {
        if signals.len() < 2 {
            continue; // no empty/trivial lifecycles
        }
        signals.sort_by(|a, b| a.name.cmp(&b.name));
        let comp_id = comp_name_to_id
            .get(&comp)
            .cloned()
            .unwrap_or_else(|| format!("component:{comp}"));
        let name = format!("{comp}-lifecycle");

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
        let mut states: Vec<String> = Vec::new();
        let mut transitions: Vec<String> = Vec::new();
        for (i, s) in signals.iter().enumerate() {
            let cat = verb_category(&s.name);
            states.push(s.name.clone());
            if cat == "transition" {
                transitions.push(s.name.clone());
            }
            steps.push(FlowStep {
                id: format!("step:{}", i + 2),
                order: (i + 2) as u32,
                actor: comp_id.clone(),
                operation: s.name.clone(),
                condition: Some(cat.to_string()),
                r#async: None,
                timeout_ms: None,
                retry_policy: None,
                failure_outcome: None,
                provenance: Some(Provenance::Extracted),
                evidence: vec![s.id.clone()],
            });
        }

        let mut attributes = BTreeMap::new();
        attributes.insert("states".to_string(), json!(states));
        attributes.insert("transitions".to_string(), json!(transitions));
        attributes.insert("signals".to_string(), json!(signals.len()));
        // Wave 3 §22: this view detects state-machine SIGNALS (enums,
        // transition-verb names). It is NOT an authoritative state-machine
        // ordering — the flag keeps agents from reading it as one.
        attributes.insert("signals_only".to_string(), json!(true));
        out.push(Flow {
            id: entity_id(&store.repo_id, kinds::FLOW, &name),
            kind: FlowKind::Lifecycle,
            name,
            trigger: None,
            steps,
            attributes,
        });
    }
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

    /// Insert a symbol entity; returns its id.
    fn put_symbol(
        store: &Store,
        name: &str,
        file: &str,
        kind: &str,
        attrs: &[(&str, serde_json::Value)],
    ) -> String {
        let mut e = Entity::new(symbol_id(&store.repo_id, file, name), kinds::SYMBOL, name);
        e.attr("kind", serde_json::json!(kind));
        e.attr("file", serde_json::json!(file));
        for (k, v) in attrs {
            e.attr(k, v.clone());
        }
        store.insert_entity(&e, &[file.to_string()]).unwrap();
        e.id.clone()
    }

    #[test]
    fn python_enum_class_and_transition_methods() {
        let (_dir, store) = setup();
        put_components(&store, &[("worker", &["worker"])]);
        let state_id = put_symbol(&store, "OrderState", "worker/state_machine.py", "class", &[]);
        let advance_id = put_symbol(&store, "advance", "worker/state_machine.py", "method", &[]);
        let cancel_id = put_symbol(&store, "cancel", "worker/state_machine.py", "method", &[]);
        // non-signal noise in the same component
        put_symbol(&store, "process_order", "worker/state_machine.py", "function", &[]);
        let graph = RealityGraph::load(&store).unwrap();

        let flows = compile_lifecycles(&graph, &store).unwrap();
        assert_eq!(flows.len(), 1);
        let f = &flows[0];
        assert_eq!(f.kind, FlowKind::Lifecycle);
        assert_eq!(f.name, "worker-lifecycle");
        assert_eq!(f.id, entity_id(&store.repo_id, kinds::FLOW, "worker-lifecycle"));
        // entry + 3 signals, sorted by signal name
        assert_eq!(f.steps.len(), 4);
        let entry = &f.steps[0];
        assert_eq!(entry.actor, entity_id(&store.repo_id, kinds::COMPONENT, "worker"));
        assert_eq!(entry.operation, "worker");
        assert_eq!(entry.provenance, Some(Provenance::Inferred));
        let sigs: Vec<&FlowStep> = f.steps[1..].iter().collect();
        assert_eq!(sigs[0].operation, "OrderState");
        assert_eq!(sigs[0].condition.as_deref(), Some("state"));
        assert_eq!(sigs[0].evidence, vec![state_id]);
        assert_eq!(sigs[1].operation, "advance");
        assert_eq!(sigs[1].condition.as_deref(), Some("transition"));
        assert_eq!(sigs[1].evidence, vec![advance_id]);
        assert_eq!(sigs[2].operation, "cancel");
        assert_eq!(sigs[2].condition.as_deref(), Some("terminal outcome"));
        assert_eq!(sigs[2].evidence, vec![cancel_id]);
        for (i, s) in sigs.iter().enumerate() {
            assert_eq!(s.actor, entry.actor);
            assert_eq!(s.provenance, Some(Provenance::Extracted));
            assert_eq!(s.order, i as u32 + 2);
        }
        assert_eq!(
            f.attributes["states"],
            json!(["OrderState", "advance", "cancel"])
        );
        assert_eq!(f.attributes["transitions"], json!(["advance"]));
        assert_eq!(f.attributes["signals"], json!(3));
    }

    #[test]
    fn ts_enum_and_signature_signal() {
        let (_dir, store) = setup();
        put_components(&store, &[("web", &["web"])]);
        let status_id = put_symbol(&store, "ConnectionStatus", "web/conn.ts", "enum", &[]);
        let kind_id = put_symbol(
            &store,
            "OrderKind",
            "web/conn.ts",
            "class",
            &[("signature", serde_json::json!("class OrderKind(StrEnum)"))],
        );
        let set_id = put_symbol(&store, "set_status", "web/conn.ts", "function", &[]);
        let retry_id = put_symbol(&store, "retry", "web/conn.ts", "function", &[]);
        let graph = RealityGraph::load(&store).unwrap();

        let flows = compile_lifecycles(&graph, &store).unwrap();
        assert_eq!(flows.len(), 1);
        let f = &flows[0];
        assert_eq!(f.name, "web-lifecycle");
        // entry + ConnectionStatus + OrderKind + retry + set_status
        assert_eq!(f.steps.len(), 5);
        let ops: Vec<&str> = f.steps[1..].iter().map(|s| s.operation.as_str()).collect();
        assert_eq!(ops, vec!["ConnectionStatus", "OrderKind", "retry", "set_status"]);
        let cond = |op: &str| {
            f.steps
                .iter()
                .find(|s| s.operation == op)
                .unwrap()
                .condition
                .clone()
        };
        assert_eq!(cond("ConnectionStatus").as_deref(), Some("state"));
        assert_eq!(cond("OrderKind").as_deref(), Some("state"));
        assert_eq!(cond("retry").as_deref(), Some("retry"));
        assert_eq!(cond("set_status").as_deref(), Some("event"));
        assert_eq!(f.attributes["signals"], json!(4));
        assert_eq!(f.attributes["transitions"], json!([]));
        assert_eq!(f.attributes["states"], json!(["ConnectionStatus", "OrderKind", "retry", "set_status"]));
        let evidence: Vec<&str> = f.steps[1..]
            .iter()
            .map(|s| s.evidence[0].as_str())
            .collect();
        assert_eq!(
            evidence,
            vec![status_id.as_str(), kind_id.as_str(), retry_id.as_str(), set_id.as_str()]
        );
    }

    #[test]
    fn no_signals_emits_nothing() {
        let (_dir, store) = setup();
        put_components(&store, &[("api", &["api"])]);
        put_symbol(&store, "process_order", "api/orders.py", "function", &[]);
        put_symbol(&store, "Order", "api/orders.py", "class", &[]);
        put_symbol(&store, "dispatch", "api/orders.py", "function", &[]);
        let graph = RealityGraph::load(&store).unwrap();
        let flows = compile_lifecycles(&graph, &store).unwrap();
        assert!(flows.is_empty());
    }

    #[test]
    fn single_signal_emits_nothing() {
        let (_dir, store) = setup();
        put_components(&store, &[("api", &["api"])]);
        put_symbol(&store, "OrderState", "api/orders.py", "class", &[]);
        let graph = RealityGraph::load(&store).unwrap();
        let flows = compile_lifecycles(&graph, &store).unwrap();
        assert!(flows.is_empty());
    }

    #[test]
    fn lifecycle_per_component() {
        let (_dir, store) = setup();
        put_components(&store, &[("a", &["a"]), ("b", &["b"])]);
        // component a: 2 signals; component b: 1 signal
        put_symbol(&store, "JobState", "a/job.py", "class", &[]);
        put_symbol(&store, "advance", "a/job.py", "method", &[]);
        put_symbol(&store, "TaskState", "b/task.py", "class", &[]);
        let graph = RealityGraph::load(&store).unwrap();
        let flows = compile_lifecycles(&graph, &store).unwrap();
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].name, "a-lifecycle");
        assert_eq!(flows[0].steps[0].operation, "a");
    }
}
