//! Graph layer: Reality Graph loading plus the System IR compilers
//! (components, flows, invariants) and impact analysis.
//!
//! Docs mapping: scc-graph + scc-system-ir + scc-flow.

pub mod boundaries;
pub mod cochange;
pub mod components;
pub mod flowgraph;
pub mod flows;
pub mod impact;
pub mod invariants;
pub mod lifecycle;
pub mod trust;
pub mod workflow;

pub use trust::{TrustedGraphView, TrustPolicy};

impl RealityGraph {
    pub fn empty() -> RealityGraph {
        RealityGraph {
            repo_id: String::new(),
            entities: HashMap::new(),
            out: HashMap::new(),
            inn: HashMap::new(),
            components: Vec::new(),
            flows: Vec::new(),
            invariants: Vec::new(),
        }
    }
}

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

/// Map every symbol id to its component id via the component CONTAINS
/// edges (shared by the flow graph compiler and flow projections).
pub fn symbol_component_map(graph: &RealityGraph) -> HashMap<String, String> {
    let mut symbol_comp: HashMap<String, String> = HashMap::new();
    for c in &graph.components {
        for r in graph.out_pred(&c.id, scc_core::predicates::CONTAINS) {
            for sr in graph.out_pred(&r.object, scc_core::predicates::CONTAINS) {
                symbol_comp.insert(sr.object.clone(), c.id.clone());
            }
        }
    }
    symbol_comp
}

/// Staged derived compilation (P0, docs/SYSTEM_DESIGN.md §7): every stage
/// writes its output, reloads the reality graph, and only then compiles the
/// next stage, so drift and later stages can never be computed against a
/// graph that predates freshly written facts. The derived model epoch is
/// bumped *before* the first write so cached context packs are invalidated
/// even if a stage fails mid-pipeline (fail closed — no stale trusted pack
/// survives a partial recompile).
pub struct CompilationPipeline<'a> {
    store: &'a Store,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompilationStage {
    Components,
    Flows,
    Behavior,
    Invariants,
    Drift,
    Boundaries,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StageCounts {
    pub components: usize,
    pub flows: usize,
    pub invariants: usize,
    pub drift: usize,
    pub boundaries: usize,
}

impl<'a> CompilationPipeline<'a> {
    pub fn new(store: &'a Store) -> CompilationPipeline<'a> {
        CompilationPipeline { store }
    }

    pub fn run(self) -> Result<RecompileReport> {
        // invalidate epoch-keyed context caches before any derived write
        self.store
            .bump_epoch(scc_store::ModelEpochKind::Derived)?;

        // STAGE 0/1: load the base reality graph, compile components.
        let graph = RealityGraph::load(self.store)?;
        let intent = self.store.intent_claims()?;
        let comps = components::compile_components(&graph, self.store, &intent)?;
        self.store.replace_components(&comps)?;

        // STAGE 2: reload (components now visible), compile flows.
        let graph = RealityGraph::load(self.store)?;
        let (seq_flows, data_flows, arch_flow) =
            flows::compile_flows(&graph, self.store, &intent)?;
        let mut all = seq_flows;
        all.extend(data_flows);
        if let Some(a) = arch_flow {
            all.push(a);
        }
        self.store.replace_flows(&all)?;

        // STAGE 3: canonical causal flow graphs (Wave 3) — the behavioral
        // truth from which projections derive; then the behavioral views
        // (lifecycle state machines + operational workflows) which read the
        // stored sequence flows (reload).
        let graph = RealityGraph::load(self.store)?;
        let symbol_comp = symbol_component_map(&graph);
        let graphs = flowgraph::compile_flow_graphs(&graph, self.store, &intent, &symbol_comp)?;
        self.store.replace_flow_graphs(&graphs)?;
        let graph = RealityGraph::load(self.store)?;
        let mut lifecycles = lifecycle::compile_lifecycles(&graph, self.store)?;
        let mut workflows = workflow::compile_workflows(&graph, self.store)?;
        all.append(&mut lifecycles);
        all.append(&mut workflows);
        self.store.replace_flows(&all)?;

        // STAGE 4: invariants against the fully compiled model.
        let graph = RealityGraph::load(self.store)?;
        let invs = invariants::compile_invariants(&graph, &intent)?;
        self.store.replace_invariants(&invs)?;

        // STAGE 5: drift against the *stored* components (reloaded), never
        // the pre-reload in-memory list.
        let graph = RealityGraph::load(self.store)?;
        let stored_comps = self.store.components()?;
        let findings = invariants::drift_findings(&graph, self.store, &intent, &stored_comps)?;
        self.store.clear_drift_findings()?;
        for (kind, severity, message) in &findings {
            self.store
                .add_drift_finding(kind, severity, message)?;
        }

        // STAGE 6: trust-boundary crossings (derived facts).
        let graph = RealityGraph::load(self.store)?;
        let crossings = boundaries::compile_boundaries(&graph, self.store)?;
        for (rel, src) in crossings {
            self.store.insert_relationship(&rel, &src)?;
        }

        // garbage-collect evidence that lost its last reference during the
        // rebuild (docs/DATA_STRATEGY.md §6)
        self.store.sweep_orphan_evidence()?;

        Ok(RecompileReport {
            components: comps.len(),
            flows: all.len(),
            invariants: invs.len(),
            drift: findings.len(),
            boundaries: stored_comps.len(),
        })
    }
}

/// Recompile the entire derived layer (components, flows, invariants, drift)
/// from the reality graph. Idempotent; replaces derived tables in the store.
/// Equivalent to [`CompilationPipeline::run`] (kept for callers that do not
/// need stage control).
pub fn recompile(store: &Store) -> Result<RecompileReport> {
    CompilationPipeline::new(store).run()
}

#[derive(Debug, Clone, Default)]
pub struct RecompileReport {
    pub components: usize,
    pub flows: usize,
    pub invariants: usize,
    /// Number of drift findings emitted at stage 5 (against the freshly
    /// compiled model).
    pub drift: usize,
    /// Number of trust-boundary crossing edges at stage 6.
    pub boundaries: usize,
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

    #[test]
    fn pipeline_bumps_derived_epoch_and_reloads_between_stages() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();

        let epoch_before = store.model_epoch().unwrap();
        let rep = CompilationPipeline::new(&store).run().unwrap();
        let epoch_after = store.model_epoch().unwrap();

        // derived compilation invalidates the cache epoch even on an empty
        // store (fail closed before any derived write)
        assert_eq!(epoch_after.derived, epoch_before.derived + 1);
        assert!(rep.components >= 1);
        assert_eq!(rep.drift, 0, "no drift on an empty model");

        // a second run is idempotent in content but bumps again (each
        // recompile is a new derived model state)
        let rep2 = CompilationPipeline::new(&store).run().unwrap();
        assert_eq!(rep2.components, rep.components);
        assert_eq!(rep2.flows, rep.flows);
        assert_eq!(rep2.invariants, rep.invariants);
    }

    #[test]
    fn drift_uses_newly_compiled_flows() {
        // regression (P0 stage ordering): drift findings must be computed
        // against flows written by the current pipeline run, not a stale
        // pre-recompile model.
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();

        // first compile: only the empty-repo architecture flow exists
        let rep = CompilationPipeline::new(&store).run().unwrap();
        let base_flows = rep.flows;
        assert!(base_flows >= 1);

        // add a flow-affecting fact: a route handler
        let repo = store.repo_id.clone();
        let route = scc_core::entity_id(&repo, "route", "get-/api/x");
        store
            .insert_entity(
                scc_core::Entity::new(route.clone(), "route", "get-/api/x")
                    .attr("method", serde_json::json!("GET"))
                    .attr("path", serde_json::json!("/api/x"))
                    .attr("handler", serde_json::json!("handle_x")),
                &["main.py".into()],
            )
            .unwrap();
        let sym = scc_core::symbol_id(&repo, "main.py", "handle_x");
        store
            .insert_entity(&scc_core::Entity::new(sym.clone(), "symbol", "handle_x"), &["main.py".into()])
            .unwrap();
        store
            .insert_relationship(
                &scc_core::Relationship::new(
                    "rel:route",
                    sym.clone(),
                    scc_core::predicates::HANDLES,
                    route,
                    scc_core::Provenance::Extracted,
                ),
                "main.py",
            )
            .unwrap();

        // second compile: the pipeline must see its own newly written
        // component (root) and flow (get-/api/x) in later stages
        let rep2 = CompilationPipeline::new(&store).run().unwrap();
        assert!(rep2.flows > base_flows, "flows: {} vs {base_flows}", rep2.flows);
        let flows = store.flows().unwrap();
        assert!(
            flows.iter().any(|f| f.name.contains("get-/api/x")),
            "compiled flow must be present: {flows:?}"
        );
    }
}
