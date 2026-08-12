//! TrustedGraphView (P0): the only way the Context Compiler may query the
//! reality graph. Enforces the trust contract (docs/SYSTEM_DESIGN.md §5):
//!
//! - STALE facts (evidence whose file changed since indexing) are excluded
//!   from every trusted traversal and reported as warnings.
//! - Provenance policy: extracted/resolved/observed/declared facts are
//!   trusted by default; INFERRED facts below the confidence floor are
//!   excluded unless `include_low_confidence_inference` is set.
//! - No transformation may strengthen a claim: the view only *filters*, it
//!   never rewrites provenance.

use crate::RealityGraph;
use scc_core::{Entity, Flow, Invariant, Provenance, Relationship};
use scc_store::Store;
use std::collections::{HashMap, HashSet};

/// Trust policy applied to graph queries. STALE facts are always excluded;
/// the remaining provenance classes are gated here.
#[derive(Debug, Clone)]
pub struct TrustPolicy {
    pub allow_extracted: bool,
    pub allow_resolved: bool,
    pub allow_observed: bool,
    pub allow_declared: bool,
    pub allow_inferred: bool,
    pub min_inferred_confidence: f64,
}

impl Default for TrustPolicy {
    fn default() -> Self {
        TrustPolicy {
            allow_extracted: true,
            allow_resolved: true,
            allow_observed: true,
            allow_declared: true,
            allow_inferred: true,
            min_inferred_confidence: 0.85,
        }
    }
}

impl TrustPolicy {
    /// Policy derived from the context settings: lowering the inferred
    /// confidence floor to zero when low-confidence inference is explicitly
    /// requested (still labeled INFERRED, still evidence-linked).
    pub fn with_inferred_floor(mut self, floor: f64) -> Self {
        self.min_inferred_confidence = floor;
        self
    }

    pub fn allows(&self, prov: Provenance, confidence: f64) -> bool {
        match prov {
            Provenance::Stale => false,
            Provenance::Extracted => self.allow_extracted,
            Provenance::Resolved => self.allow_resolved,
            Provenance::Observed => self.allow_observed,
            Provenance::Declared => self.allow_declared,
            Provenance::Inferred => {
                self.allow_inferred && confidence >= self.min_inferred_confidence
            }
        }
    }
}

/// Filtered, policy-governed view over the reality graph.
pub struct TrustedGraphView<'a> {
    pub graph: &'a RealityGraph,
    stale_paths: HashSet<String>,
    /// evidence ids whose path is in `stale_paths`
    stale_evidence: HashSet<String>,
    /// entity ids carrying any stale evidence
    stale_entities: HashSet<String>,
    policy: TrustPolicy,
}

impl<'a> TrustedGraphView<'a> {
    pub fn new(
        graph: &'a RealityGraph,
        store: &'a Store,
        stale_paths: &[String],
        policy: TrustPolicy,
    ) -> TrustedGraphView<'a> {
        let stale_paths: HashSet<String> = stale_paths.iter().cloned().collect();
        // map evidence id -> path once per view construction
        let evidence_paths: HashMap<String, String> = store
            .all_evidence()
            .ok()
            .map(|evs| {
                evs.into_iter()
                    .filter_map(|e| e.path.map(|p| (e.id, p)))
                    .collect()
            })
            .unwrap_or_default();

        let mut stale_evidence: HashSet<String> = HashSet::new();
        for (id, path) in &evidence_paths {
            if stale_paths.contains(path) {
                stale_evidence.insert(id.clone());
            }
        }

        let mut stale_entities: HashSet<String> = HashSet::new();
        for e in graph.entities.values() {
            if e.evidence.iter().any(|ev| stale_evidence.contains(ev)) {
                stale_entities.insert(e.id.clone());
            }
        }

        TrustedGraphView {
            graph,
            stale_paths,
            stale_evidence,
            stale_entities,
            policy,
        }
    }

    pub fn policy(&self) -> &TrustPolicy {
        &self.policy
    }

    pub fn is_stale_path(&self, path: &str) -> bool {
        self.stale_paths.contains(path)
    }

    pub fn stale_paths(&self) -> Vec<String> {
        let mut v: Vec<String> = self.stale_paths.iter().cloned().collect();
        v.sort();
        v
    }

    pub fn is_stale_evidence(&self, id: &str) -> bool {
        self.stale_evidence.contains(id)
    }

    pub fn is_stale_entity(&self, id: &str) -> bool {
        self.stale_entities.contains(id)
    }

    fn rel_allowed(&self, r: &Relationship) -> bool {
        if r.evidence.iter().any(|ev| self.stale_evidence.contains(ev)) {
            return false;
        }
        self.policy.allows(r.provenance, r.confidence)
    }

    // ---- entity access (staleness-filtered; names remain readable) ----

    /// Trusted entity lookup: `None` for entities whose evidence is stale.
    pub fn entity(&self, id: &str) -> Option<&Entity> {
        self.graph
            .entities
            .get(id)
            .filter(|_| !self.is_stale_entity(id))
    }

    /// Display name of an entity id; falls back to the id's last segment.
    /// Entity staleness does not hide the name — callers decide whether the
    /// entity may appear in trusted sections.
    pub fn name_of(&self, id: &str) -> String {
        self.graph
            .entities
            .get(id)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| id.rsplit('/').next().unwrap_or(id).to_string())
    }

    /// All entities, staleness-filtered (unfiltered raw map for display
    /// helpers that need presence only).
    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.graph
            .entities
            .values()
            .filter(|e| !self.is_stale_entity(&e.id))
    }

    /// Entities of a kind (staleness-filtered, name-sorted for determinism).
    pub fn entities_of_kind(&self, kind: &str) -> Vec<&Entity> {
        let mut v: Vec<&Entity> = self
            .graph
            .entities
            .values()
            .filter(|e| e.kind == kind && !self.is_stale_entity(&e.id))
            .collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    // ---- relationship access (staleness + policy filtered) ----

    pub fn out_edges(&self, id: &str) -> Vec<&Relationship> {
        self.graph
            .out
            .get(id)
            .map(|v| v.iter().filter(|r| self.rel_allowed(r)).collect())
            .unwrap_or_default()
    }

    pub fn in_edges(&self, id: &str) -> Vec<&Relationship> {
        self.graph
            .inn
            .get(id)
            .map(|v| v.iter().filter(|r| self.rel_allowed(r)).collect())
            .unwrap_or_default()
    }

    pub fn out_pred(&self, id: &str, predicate: &str) -> Vec<&Relationship> {
        self.graph
            .out
            .get(id)
            .map(|v| {
                v.iter()
                    .filter(|r| r.predicate == predicate && self.rel_allowed(r))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn in_pred(&self, id: &str, predicate: &str) -> Vec<&Relationship> {
        self.graph
            .inn
            .get(id)
            .map(|v| {
                v.iter()
                    .filter(|r| r.predicate == predicate && self.rel_allowed(r))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// All trusted relationships, sorted by id for determinism.
    pub fn all_rels(&self) -> Vec<&Relationship> {
        let mut v: Vec<&Relationship> = self
            .graph
            .out
            .values()
            .flatten()
            .filter(|r| self.rel_allowed(r))
            .collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    // ---- derived tables (staleness-filtered) ----

    /// Staleness-filtered flows. A flow is stale when any of its steps'
    /// evidence is stale.
    pub fn flows(&self) -> Vec<Flow> {
        let mut out: Vec<Flow> = Vec::new();
        for f in &self.graph.flows {
            let stale = f
                .steps
                .iter()
                .any(|s| s.evidence.iter().any(|ev| self.stale_evidence.contains(ev)));
            if !stale {
                out.push(f.clone());
            }
        }
        out
    }

    pub fn invariants(&self) -> Vec<Invariant> {
        let mut out: Vec<Invariant> = Vec::new();
        for i in &self.graph.invariants {
            let stale = i.evidence.iter().any(|ev| self.stale_evidence.contains(ev));
            if !stale {
                out.push(i.clone());
            }
        }
        out
    }

    pub fn components(&self) -> Vec<Entity> {
        let mut out: Vec<Entity> = Vec::new();
        for c in &self.graph.components {
            let stale = c
                .evidence
                .iter()
                .any(|ev| self.stale_evidence.contains(ev));
            if !stale {
                out.push(c.clone());
            }
        }
        out
    }

    // ---- staleness warnings ----

    /// Deterministic stale-fact warnings for pack footers.
    pub fn stale_warnings(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        let mut paths: Vec<&String> = self.stale_paths.iter().collect();
        paths.sort();
        for p in paths {
            v.push(format!("{p} changed since indexing — its facts are excluded (STALE)"));
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scc_core::{Evidence, EvidenceType};
    use scc_store::Store;

    fn setup() -> (Store, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        (store, dir)
    }

    fn entity(id: &str, kind: &str, evidence: Vec<&str>) -> scc_core::Entity {
        let evidence: Vec<String> = evidence.into_iter().map(|s| s.to_string()).collect();
        let _ = kind;
        let mut e = scc_core::Entity::new(id, "component", id.rsplit('/').next().unwrap_or(id));
        e.evidence = evidence;
        e
    }

    fn view<'a>(
        store: &'a Store,
        graph: &'a RealityGraph,
        stale: &[String],
    ) -> TrustedGraphView<'a> {
        TrustedGraphView::new(graph, store, stale, TrustPolicy::default())
    }

    #[test]
    fn stale_facts_are_excluded_and_warned() {
        let (store, _d) = setup();
        let ev = Evidence {
            id: "evidence:1".into(),
            r#type: EvidenceType::Source,
            path: Some("main.py".into()),
            symbol: None,
            start_line: None,
            end_line: None,
            revision: None,
            content_hash: None,
            extractor: Some("test".into()),
            extractor_version: None,
        };
        store.insert_evidence(&ev).unwrap();

        // graph: sym calls other; both facts point at main.py evidence
        let sym = entity("repo://r/symbol/a", "symbol", vec![]);
        let rel = scc_core::Relationship::new(
            "rel:1",
            sym.id.clone(),
            scc_core::predicates::CALLS,
            "repo://r/symbol/b",
            Provenance::Extracted,
        )
        .with_evidence(vec!["evidence:1".to_string()]);
        let mut graph = RealityGraph {
            repo_id: "r".into(),
            entities: [(sym.id.clone(), sym.clone())].into_iter().collect(),
            out: [(sym.id.clone(), vec![rel.clone()])].into_iter().collect(),
            inn: [(rel.object.clone(), vec![rel])].into_iter().collect(),
            components: vec![],
            flows: vec![],
            invariants: vec![],
        };
        graph.components = vec![entity(
            "repo://r/component/c",
            "component",
            vec!["evidence:1"],
        )];

        // fresh view: everything visible
        let v = view(&store, &graph, &[]);
        assert_eq!(v.out_edges(&sym.id).len(), 1);
        assert!(v.entity(&sym.id).is_some());
        assert_eq!(v.components().len(), 1);
        assert!(v.stale_warnings().is_empty());

        // stale view: fact excluded, warning surfaced; the symbol itself
        // carries no stale evidence (only its CALLS fact does), so it stays
        // visible — but the component whose evidence is stale is hidden
        let v = view(&store, &graph, &["main.py".to_string()]);
        assert!(v.out_edges(&sym.id).is_empty());
        assert!(v.entity(&sym.id).is_some());
        assert!(v.components().is_empty());
        let warns = v.stale_warnings();
        assert_eq!(warns.len(), 1);
        assert!(warns[0].contains("main.py"));
    }

    #[test]
    fn policy_gates_inferred_confidence() {
        let (store, _d) = setup();
        let sym = entity("repo://r/symbol/a", "symbol", vec![]);
        let low = scc_core::Relationship::new(
            "rel:low",
            sym.id.clone(),
            scc_core::predicates::CALLS,
            "repo://r/symbol/b",
            Provenance::Inferred,
        )
        .with_confidence(0.5);
        let high = scc_core::Relationship::new(
            "rel:high",
            sym.id.clone(),
            scc_core::predicates::CALLS,
            "repo://r/symbol/c",
            Provenance::Inferred,
        )
        .with_confidence(0.9);
        let graph = RealityGraph {
            repo_id: "r".into(),
            entities: [(sym.id.clone(), sym.clone())].into_iter().collect(),
            out: [(sym.id.clone(), vec![low.clone(), high.clone()])].into_iter().collect(),
            inn: [(low.object.clone(), vec![low]), (high.object.clone(), vec![high])]
                .into_iter()
                .collect(),
            components: vec![],
            flows: vec![],
            invariants: vec![],
        };

        // default floor 0.85: low-confidence inference excluded
        let v = view(&store, &graph, &[]);
        let edges = v.out_edges(&sym.id);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].id, "rel:high");

        // explicit floor 0.0: all labeled inference trusted
        let v = TrustedGraphView::new(
            &graph,
            &store,
            &[],
            TrustPolicy::default().with_inferred_floor(0.0),
        );
        assert_eq!(v.out_edges(&sym.id).len(), 2);
    }
}
