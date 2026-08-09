//! Graph layer: Reality Graph loading plus the System IR compilers
//! (components, flows, invariants) and impact analysis.
//!
//! Docs mapping: scc-graph + scc-system-ir + scc-flow.

pub mod boundaries;
pub mod cochange;
pub mod components;
pub mod flows;
pub mod impact;
pub mod invariants;
pub mod lifecycle;
pub mod workflow;

use scc_core::{Entity, Flow, Invariant, Relationship};
use scc_store::Store;
use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum GraphError {
    #[error("store: {0}")]
    Store(#[from] scc_store::StoreError),
}

pub type Result<T> = std::result::Result<T, GraphError>;

/// In-memory view of the reality graph.
pub struct RealityGraph {
    pub repo_id: String,
    pub entities: HashMap<String, Entity>,
    /// out edges by subject
    pub out: HashMap<String, Vec<Relationship>>,
    /// in edges by object
    pub inn: HashMap<String, Vec<Relationship>>,
    pub components: Vec<Entity>,
    pub flows: Vec<Flow>,
    pub invariants: Vec<Invariant>,
}

impl RealityGraph {
    pub fn load(store: &Store) -> Result<RealityGraph> {
        let mut entities = HashMap::new();
        for e in store.all_entities()? {
            entities.insert(e.id.clone(), e);
        }
        let mut out: HashMap<String, Vec<Relationship>> = HashMap::new();
        let mut inn: HashMap<String, Vec<Relationship>> = HashMap::new();
        for r in store.all_relationships()? {
            out.entry(r.subject.clone()).or_default().push(r.clone());
            inn.entry(r.object.clone()).or_default().push(r);
        }
        Ok(RealityGraph {
            repo_id: store.repo_id.clone(),
            entities,
            out,
            inn,
            components: store.components()?,
            flows: store.flows()?,
            invariants: store.invariants()?,
        })
    }

    pub fn entity(&self, id: &str) -> Option<&Entity> {
        self.entities.get(id)
    }

    pub fn out_edges(&self, id: &str) -> Vec<&Relationship> {
        self.out.get(id).map(|v| v.iter().collect()).unwrap_or_default()
    }

    pub fn in_edges(&self, id: &str) -> Vec<&Relationship> {
        self.inn.get(id).map(|v| v.iter().collect()).unwrap_or_default()
    }

    pub fn out_pred(&self, id: &str, predicate: &str) -> Vec<&Relationship> {
        self.out
            .get(id)
            .map(|v| {
                v.iter()
                    .filter(|r| r.predicate == predicate)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn in_pred(&self, id: &str, predicate: &str) -> Vec<&Relationship> {
        self.inn
            .get(id)
            .map(|v| {
                v.iter()
                    .filter(|r| r.predicate == predicate)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Entities of a given kind (sorted by name for determinism).
    pub fn entities_of_kind(&self, kind: &str) -> Vec<&Entity> {
        let mut v: Vec<&Entity> = self
            .entities
            .values()
            .filter(|e| e.kind == kind)
            .collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }
}

/// Recompile the entire derived layer (components, flows, invariants, drift)
/// from the reality graph. Idempotent; replaces derived tables in the store.
pub fn recompile(store: &Store) -> Result<RecompileReport> {
    let graph = RealityGraph::load(store)?;
    let intent = store.intent_claims()?;

    let comps = components::compile_components(&graph, store, &intent)?;
    store.replace_components(&comps)?;

    // Reload: flows/invariants/drift must see the freshly stored components.
    let graph = RealityGraph::load(store)?;
    let (seq_flows, data_flows, arch_flow) = flows::compile_flows(&graph, store, &intent)?;
    let mut all = seq_flows;
    all.extend(data_flows);
    if let Some(a) = arch_flow {
        all.push(a);
    }
    store.replace_flows(&all)?;

    // Behavioral views (docs/PRD.md §7 P1): lifecycle state machines and
    // operational workflows. compile_workflows reads the sequence flows we
    // just stored, so it must run after replace_flows.
    let mut lifecycles = lifecycle::compile_lifecycles(&graph, store)?;
    let mut workflows = workflow::compile_workflows(&graph, store)?;
    all.append(&mut lifecycles);
    all.append(&mut workflows);
    store.replace_flows(&all)?;

    let invs = invariants::compile_invariants(&graph, &intent)?;
    store.replace_invariants(&invs)?;

    let findings = invariants::drift_findings(&graph, store, &intent, &comps)?;
    store.clear_drift_findings()?;
    for (kind, severity, message) in findings {
        store.add_drift_finding(&kind, &severity, &message)?;
    }

    // trust-boundary compilation (SCC-148): crossing edges from deployment
    // units and external API calls. Derived facts; the compiler replaces the
    // previous set and returns (rel, source) pairs for insertion.
    let crossings = boundaries::compile_boundaries(&graph, store)?;
    for (rel, src) in crossings {
        store.insert_relationship(&rel, &src)?;
    }

    // garbage-collect evidence that lost its last reference during the
    // rebuild (docs/DATA_STRATEGY.md §6)
    store.sweep_orphan_evidence()?;

    Ok(RecompileReport {
        components: comps.len(),
        flows: all.len(),
        invariants: invs.len(),
    })
}

#[derive(Debug, Clone, Default)]
pub struct RecompileReport {
    pub components: usize,
    pub flows: usize,
    pub invariants: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_empty_store() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let g = RealityGraph::load(&store).unwrap();
        assert!(g.entities.is_empty());
        let rep = recompile(&store).unwrap();
        assert_eq!(rep.components, 1, "empty repos still get a root component");
    }
}
