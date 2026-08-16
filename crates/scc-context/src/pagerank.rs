//! Personalized PageRank over the System IR reference graph (Wave 14B).
//!
//! Layering invariants: ranking NEVER creates architectural truth — it only
//! normalizes relationships that already exist as trusted facts in the
//! `TrustedGraphView` and walks them. `FlowGraph` remains the causal
//! authority; this module never orders flow steps.
//!
//! Structure:
//! - [`build_reference_graph`] normalizes existing trusted relationships into
//!   [`ReferenceEdge`]s (never invents new relationships).
//! - [`RankEdgeKind`] is the rank-edge ontology: CONTAINS/PARTICIPATES_IN/
//!   HANDLES/DEFINES enter the PageRank adjacency with deliberate direction
//!   and weight (membership evidence, the member→container, flow→
//!   participant, route→handler and schema→definer RANKING TRANSITIONS,
//!   invocation/definition edges) — the mechanism that connects components,
//!   flows, routes and schemas to the rank graph.
//! - [`SystemRanker`] runs personalized PageRank (power iteration, damping
//!   0.85, 50 iterations) over a HETEROGENEOUS node universe — every
//!   rankable entity (symbol, component, subsystem, service, flow,
//!   contract, state, reactive, route, topic, queue, store, schema, file),
//!   not symbols alone. Edges survive when either endpoint is a rankable
//!   entity, so OWNS→STATE, REGISTERS→CONTRACT, PUBLISHES/SUBSCRIBES→TOPIC
//!   and READS/WRITES→DATA_STORE feed PageRank directly. The global vector
//!   is architecturally seeded; task vectors personalize from `TaskSeed`s.
//! - [`SystemRanker::project_to_symbols`] projects the heterogeneous node
//!   scores back to surface-relevant symbol scores (a symbol's score = its
//!   own + 0.4 × the scores of the entities it owns/registers/publishes/
//!   reads/writes/participates-in/handles/defines, bonus capped at 0.5) —
//!   the mechanism that carries component/flow/contract/state/route/schema
//!   importance to the surface.
//! - [`architectural_specificity`] boosts public facades/entrypoints/state
//!   owners/contract endpoints/flow participants and penalizes generic
//!   utilities, generated/vendored code, and ubiquitous symbols — keeping
//!   central utilities from dominating the ranking.
//! - [`final_importance`] is the spec's blend of task/global PPR with
//!   lexical, semantic, confidence, criticality, change-risk and novelty
//!   signals.
//!
//! Everything is deterministic (fixed iteration count, sorted indices,
//! id-ordered traversals) and no-panic (clamped weights, guarded
//! division).

use scc_core::{
    kinds, predicates, Provenance, ReferenceEdge, ReferenceKind, Relationship, SourceRange,
    SurfaceEntry, TaskSeed,
};
use scc_graph::TrustedGraphView;
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Edge weight constants (spec §2)
// ---------------------------------------------------------------------------

/// Weight category for flow-participation edges (flows are trusted facts;
/// the ranker never invents flow ordering).
pub const FLOW_PARTICIPATION: f64 = 1.5;
/// Weight category for invocation edges (`invokes`, callbacks).
pub const INVOCATION: f64 = 1.5;
/// Weight category for public-API edges (`exports`).
pub const PUBLIC_API: f64 = 1.4;
/// Weight category for state/reactive ownership edges (`owns`).
pub const OWNS_STATE: f64 = 1.4;
/// Weight category for data-flow edges (`reads`/`writes`/`produces`/`consumes`).
pub const PRODUCES_CONSUMES: f64 = 1.3;
/// Weight category for `implements` edges.
pub const IMPLEMENTS: f64 = 1.25;
/// Weight category for `inherits` edges.
pub const EXTENDS: f64 = 1.2;
/// Weight category for symbol-resolved `calls` edges.
pub const CALLS_RESOLVED: f64 = 1.2;
/// Weight category for contract-registration edges (`registers`).
pub const CONTRACT_PARTICIPATION: f64 = 1.2;
/// Weight category for `publishes`/`subscribes` edges.
pub const SUBSCRIBES_PUBLISHES: f64 = 1.2;
/// Neutral dependency weight (unmapped predicates, `depends_on`).
pub const DEPENDS_ON: f64 = 1.0;
/// Weight category for text-extracted (unresolved) `calls` edges.
pub const CALLS_EXTRACTED: f64 = 0.9;
/// Weight category for `imports` edges.
pub const IMPORTS: f64 = 0.6;
/// Rank-edge ontology weights — deliberate direction/weight pairs for the
/// predicates that carry membership/participation/invocation/definition
/// evidence into PageRank (see [`RankEdgeKind`]): `contains` container →
/// member membership evidence (0.4); the reverse member → container
/// RANKING TRANSITION (0.8) that derives container importance from its
/// members; `participates_in` symbol → flow (0.6) plus the reverse flow →
/// participant RANKING TRANSITION (1.2) that lets flow importance reach
/// the participants (the mechanism that connects flow nodes into the rank
/// graph); `handles` symbol → route (1.5, invocation-strong) plus the
/// reverse route → handler RANKING TRANSITION (1.5); `defines` symbol →
/// schema (1.2) plus the reverse schema → definer RANKING TRANSITION
/// (1.2). The reverse directions are pure ranking edges — NEVER Reality
/// Graph relationships.
pub const RANK_MEMBERSHIP: f64 = 0.4;
pub const RANK_MEMBER_OF: f64 = 0.8;
pub const RANK_PARTICIPATES: f64 = 0.6;
pub const RANK_FLOW_REACHES_PARTICIPANT: f64 = 1.2;
pub const RANK_HANDLES: f64 = 1.5;
pub const RANK_DEFINES: f64 = 1.2;
pub const RANK_HANDLED_BY: f64 = 1.5;
pub const RANK_DEFINED_BY: f64 = 1.2;

/// Provenance weights (spec §2): STALE facts weigh zero (the trusted view
/// already excludes them; the zero is a belt-and-suspenders floor).
pub const PROV_OBSERVED: f64 = 1.0;
pub const PROV_RESOLVED: f64 = 1.0;
pub const PROV_EXTRACTED: f64 = 0.85;
pub const PROV_DECLARED: f64 = 0.8;
pub const PROV_INFERRED: f64 = 0.6;
pub const PROV_STALE: f64 = 0.0;

/// PageRank damping factor.
pub const DAMPING_FACTOR: f64 = 0.85;
/// Fixed power-iteration count (deterministic).
pub const POWER_ITERATIONS: usize = 50;
/// Warm-start blend of the global vector into the task vector's start state.
pub const WARM_START_GLOBAL_BLEND: f64 = 0.3;

/// Ubiquity threshold: a symbol referenced by more distinct sources than
/// this is "ubiquitous" and gets penalized by [`architectural_specificity`].
pub const UBIQUITY_THRESHOLD: usize = 20;

/// Generic utility name tokens that make a symbol architecturally generic.
const GENERIC_NAMES: [&str; 7] = [
    "utils", "logger", "errors", "common", "types", "helpers", "config",
];

/// Entity kinds that count as invocation surfaces (route/endpoint/event/
/// topic/queue handlers are reachable from outside the process).
const INVOCATION_KINDS: [&str; 5] = [
    kinds::ROUTE,
    kinds::ENDPOINT,
    kinds::EVENT,
    kinds::TOPIC,
    kinds::QUEUE,
];

/// Entity kinds that count as contract endpoints for registration edges.
const CONTRACT_KINDS: [&str; 2] = [kinds::CONTRACT, kinds::REGISTRY];

/// The HETEROGENEOUS PageRank universe: every entity kind that can carry
/// surface-relevant importance. Symbol-only ranking let component/flow/
/// contract/state importance die at the boundary; these kinds participate
/// as first-class nodes (real strings from `scc_core::kinds`). SYSTEM is
/// included so the top of the containment hierarchy (System →
/// Subsystem/…) is a first-class rank node, not a dead endpoint.
const RANKABLE_KINDS: [&str; 15] = [
    kinds::SYMBOL,
    kinds::COMPONENT,
    kinds::SUBSYSTEM,
    kinds::SERVICE,
    kinds::SYSTEM,
    kinds::FLOW,
    kinds::CONTRACT,
    kinds::STATE,
    kinds::REACTIVE,
    kinds::ROUTE,
    kinds::TOPIC,
    kinds::QUEUE,
    kinds::DATA_STORE,
    kinds::SCHEMA,
    kinds::FILE,
];

/// Predicates whose non-symbol targets a symbol projects onto: the
/// entities whose PageRank score lifts the owning symbol's surface score.
/// HANDLES/DEFINES are included so a hot Route/Schema node (task seed or
/// PPR) reaches its handler/definer symbol — the reviewer's
/// route-hot → handler-reached scenario.
const PROJECTION_PREDICATES: [&str; 8] = [
    predicates::OWNS,
    predicates::REGISTERS,
    predicates::PUBLISHES,
    predicates::READS,
    predicates::WRITES,
    predicates::PARTICIPATES_IN,
    predicates::HANDLES,
    predicates::DEFINES,
];

/// Projection factor: a symbol's score += 0.4 × the sum of the scores of
/// the entities it OWNS/REGISTERS/PUBLISHES/READS/WRITES/PARTICIPATES_IN/
/// HANDLES/DEFINES.
pub const PROJECTION_BONUS_FACTOR: f64 = 0.4;
/// The projection bonus is capped at 0.5 per symbol (a state-heavy symbol
/// cannot accumulate an unbounded lift).
pub const PROJECTION_BONUS_CAP: f64 = 0.5;

// ---------------------------------------------------------------------------
// Weight helpers
// ---------------------------------------------------------------------------

/// Provenance weight per spec §2.
// trace:exempt reason=internal-detail
pub fn provenance_weight(provenance: Provenance) -> f64 {
    match provenance {
        Provenance::Observed => PROV_OBSERVED,
        Provenance::Resolved => PROV_RESOLVED,
        Provenance::Extracted => PROV_EXTRACTED,
        Provenance::Declared => PROV_DECLARED,
        Provenance::Inferred => PROV_INFERRED,
        Provenance::Stale => PROV_STALE,
    }
}

/// Rarity of a target symbol: `log(total_symbols / (1 + in_degree))`
/// clamped to [0.25, 1.5]. Occurrence = in-degree count of the target
/// (distinct referencing sources). A ubiquitous `logger`-ish symbol is
/// rare-except-common and gets downweighted; a distinctive symbol is rare
/// and gets boosted.
// trace:exempt reason=internal-detail
pub fn rarity(total_symbols: usize, in_degree: usize) -> f64 {
    if total_symbols == 0 {
        return 1.0;
    }
    let r = (total_symbols as f64 / (1 + in_degree) as f64).ln();
    r.clamp(0.25, 1.5)
}

/// Predicate weight category for an SCC predicate. `calls` is refined by
/// provenance: symbol-resolved calls weigh more than text-extracted ones.
// trace:exempt reason=internal-detail
pub fn predicate_weight(predicate: &str, provenance: Provenance) -> f64 {
    match predicate {
        predicates::CALLS => match provenance {
            Provenance::Resolved => CALLS_RESOLVED,
            _ => CALLS_EXTRACTED,
        },
        predicates::INVOKES | predicates::HANDLES_CALLBACK => INVOCATION,
        predicates::EXPORTS => PUBLIC_API,
        predicates::OWNS => OWNS_STATE,
        predicates::READS | predicates::WRITES => PRODUCES_CONSUMES,
        predicates::IMPLEMENTS | predicates::IMPLEMENTED_BY => IMPLEMENTS,
        predicates::INHERITS => EXTENDS,
        predicates::REGISTERS => CONTRACT_PARTICIPATION,
        predicates::PUBLISHES | predicates::SUBSCRIBES => SUBSCRIBES_PUBLISHES,
        predicates::IMPORTS => IMPORTS,
        predicates::CONTAINS => RANK_MEMBERSHIP,
        _ => DEPENDS_ON,
    }
}

/// Full edge weight (spec §2): `predicate_weight * provenance_weight *
/// confidence * rarity`. `target_in_degree` is the in-degree count of the
/// target symbol (distinct referencing sources).
// trace:exempt reason=internal-detail
pub fn edge_weight(
    predicate: &str,
    provenance: Provenance,
    confidence: f64,
    total_symbols: usize,
    target_in_degree: usize,
) -> f64 {
    predicate_weight(predicate, provenance)
        * provenance_weight(provenance)
        * confidence.clamp(0.0, 1.0)
        * rarity(total_symbols, target_in_degree)
}

// ---------------------------------------------------------------------------
// Reference normalization
// ---------------------------------------------------------------------------

/// Map an SCC predicate to a [`ReferenceKind`], given the target entity
/// kind (needed to scope `owns` to state/reactive per the spec). `None`
/// means the predicate is not part of the normalized reference surface —
/// it is left alone, never invented.
// trace:exempt reason=internal-detail
fn reference_kind(predicate: &str, target_kind: &str) -> Option<ReferenceKind> {
    match predicate {
        predicates::CALLS | predicates::INVOKES | predicates::HANDLES_CALLBACK => {
            Some(ReferenceKind::Call)
        }
        predicates::READS => Some(ReferenceKind::Read),
        predicates::WRITES => Some(ReferenceKind::Write),
        predicates::OWNS if target_kind == kinds::STATE || target_kind == kinds::REACTIVE => {
            Some(ReferenceKind::Write)
        }
        predicates::IMPLEMENTS | predicates::IMPLEMENTED_BY => Some(ReferenceKind::Implement),
        predicates::INHERITS => Some(ReferenceKind::Extend),
        predicates::REGISTERS | predicates::PUBLISHES | predicates::SUBSCRIBES => {
            Some(ReferenceKind::Register)
        }
        predicates::IMPORTS => Some(ReferenceKind::Import),
        predicates::EXPORTS => Some(ReferenceKind::Export),
        predicates::DECORATES | predicates::ANNOTATES => Some(ReferenceKind::Decorate),
        _ => None,
    }
}

/// Evidence file/line for a relationship, when available. The trusted view
/// does not expose store evidence rows, so this uses the symbol entities'
/// recorded `file`/`start_line`/`end_line` attributes (the same source
/// evidence the extractors recorded at index time). Subject first, then
/// object; deduplicated; empty when neither endpoint is a symbol.
// trace:exempt reason=internal-detail
fn locations_for(view: &TrustedGraphView, rel: &Relationship) -> Vec<SourceRange> {
    let mut locs: Vec<SourceRange> = Vec::new();
    for id in [&rel.subject, &rel.object] {
        let Some(e) = view.entity(id) else { continue };
        if e.kind != kinds::SYMBOL {
            continue;
        }
        let Some(path) = e.attributes.get("file").and_then(|v| v.as_str()) else {
            continue;
        };
        let start = e
            .attributes
            .get("start_line")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let end = e
            .attributes
            .get("end_line")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let start = start.max(1);
        let loc = SourceRange::new(path, start, end.max(start));
        if !locs.contains(&loc) {
            locs.push(loc);
        }
    }
    locs
}

/// Normalize existing trusted relationships into reference edges. Never
/// invents new relationships: every edge traces to one trusted fact.
///
/// Mapping (actual predicates from `scc_core::predicates`):
/// - `calls`/`invokes`/`handles_callback` → Call
/// - `reads` → Read; `writes` → Write
/// - `owns` (state/reactive entity) → Write
/// - `implements`/`implemented_by` → Implement
/// - `inherits` → Extend
/// - `registers`/`publishes`/`subscribes` → Register
/// - `imports` → Import; `exports` → Export
/// - `decorates`/`annotates` → Decorate
///
/// Confidence passes through from the relationship (default 1.0); locations
/// are the evidence file/line when available. Deterministic: the trusted
/// view returns relationships sorted by id.
// trace:v1 id=impl.scc.pagerank work=WORK-SCC-014 satisfies=REQ-SCC-IR
pub fn build_reference_graph(view: &TrustedGraphView) -> Vec<ReferenceEdge> {
    let mut out: Vec<ReferenceEdge> = Vec::new();
    for rel in view.all_rels() {
        let target_kind = view
            .entity(&rel.object)
            .map(|e| e.kind.as_str())
            .unwrap_or("");
        let kind = match reference_kind(&rel.predicate, target_kind) {
            Some(k) => k,
            None => continue,
        };
        // `implemented_by` is stored interface→class; normalize to class→interface.
        let (source, target) = if rel.predicate == predicates::IMPLEMENTED_BY {
            (rel.object.clone(), rel.subject.clone())
        } else {
            (rel.subject.clone(), rel.object.clone())
        };
        out.push(ReferenceEdge {
            source_symbol: source,
            target_symbol: target,
            kind,
            locations: locations_for(view, rel),
            provenance: rel.provenance,
            confidence: rel.confidence as f32,
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Rank-edge ontology
// ---------------------------------------------------------------------------

/// A directed rank edge in the heterogeneous PageRank adjacency: which
/// trusted predicates enter the rank graph and in which direction/with
/// which weight. Distinct from [`ReferenceKind`] (the normalized
/// reference surface — these edges are the RANK-edge ontology, never
/// `reference_kind` doubling). Every edge traces to one trusted
/// relationship; the reverse directions of `contains`/`participates_in`/
/// `handles`/`defines` are explicit RANKING TRANSITIONS — edges that
/// exist only so container/flow/route/schema importance reaches its
/// members/participants/handlers/definers — and are labeled as such,
/// never as Reality Graph relationships.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// trace:v1 id=impl.crates-scc-context-src-pagerank.rank-edge-kind work=WORK-rank-edge-ontology-c-o-n-t-a-i-n-s-p-a-r-t-i-c-i-p-a-t-e-s-i-n-h-a-n-d-l-e-s-d-e-f-i-n-e-s-enter-the-p-p-r-adjacency satisfies=REQ-rank-edges-heterogeneous-predicates-enter-page-rank
pub enum RankEdgeKind {
    /// Container → member: a kind earlier in [`CONTAINMENT_HIERARCHY`]
    /// (System/Subsystem/Service/Component/File) CONTAINS a kind later in
    /// it (Subsystem/Service/Component/File/Symbol) — membership
    /// evidence, weight 0.4.
    Contains,
    /// RANKING TRANSITION (never a Reality Graph relationship): member →
    /// its container, so container importance is derived from the
    /// members' aggregate (weight 0.8).
    MemberOf,
    /// `Symbol` PARTICIPATES_IN a `Flow` (weight 0.6).
    ParticipatesIn,
    /// RANKING TRANSITION: `Flow` → its participant symbol, so flow
    /// importance reaches the participants — the mechanism that connects
    /// flow nodes into the rank graph (weight 1.2).
    FlowReachesParticipant,
    /// `Symbol` HANDLES a `Route` (invocation-strong, weight 1.5).
    Handles,
    /// RANKING TRANSITION: `Route` → its handler symbol, so route
    /// importance reaches the handler — the reverse of `Handles`
    /// (weight 1.5).
    HandledBy,
    /// `Symbol` DEFINES a `Schema` (weight 1.2).
    Defines,
    /// RANKING TRANSITION: `Schema` → its defining symbol, so schema
    /// importance reaches the definer — the reverse of `Defines`
    /// (weight 1.2).
    DefinedBy,
}

// trace:exempt reason=internal-detail
impl RankEdgeKind {
    /// The reverse ranking transition of this edge: `Contains` ↔
    /// `MemberOf`, `ParticipatesIn` ↔ `FlowReachesParticipant`,
    /// `Handles` ↔ `HandledBy`, `Defines` ↔ `DefinedBy`. Every evidence
    /// edge has a ranking-only reverse — it propagates container/flow/
    /// route/schema importance back to members/participants/handlers/
    /// definers and NEVER claims a Reality Graph relationship.
    // trace:exempt reason=internal-detail
    pub fn reverse_ranking_transition(self) -> Option<RankEdgeKind> {
        match self {
            RankEdgeKind::Contains => Some(RankEdgeKind::MemberOf),
            RankEdgeKind::MemberOf => Some(RankEdgeKind::Contains),
            RankEdgeKind::ParticipatesIn => Some(RankEdgeKind::FlowReachesParticipant),
            RankEdgeKind::FlowReachesParticipant => Some(RankEdgeKind::ParticipatesIn),
            RankEdgeKind::Handles => Some(RankEdgeKind::HandledBy),
            RankEdgeKind::HandledBy => Some(RankEdgeKind::Handles),
            RankEdgeKind::Defines => Some(RankEdgeKind::DefinedBy),
            RankEdgeKind::DefinedBy => Some(RankEdgeKind::Defines),
        }
    }

    /// Base weight of the rank edge (before provenance × confidence).
    // trace:exempt reason=internal-detail
    pub fn weight(self) -> f64 {
        match self {
            RankEdgeKind::Contains => RANK_MEMBERSHIP,
            RankEdgeKind::MemberOf => RANK_MEMBER_OF,
            RankEdgeKind::ParticipatesIn => RANK_PARTICIPATES,
            RankEdgeKind::FlowReachesParticipant => RANK_FLOW_REACHES_PARTICIPANT,
            RankEdgeKind::Handles => RANK_HANDLES,
            RankEdgeKind::HandledBy => RANK_HANDLED_BY,
            RankEdgeKind::Defines => RANK_DEFINES,
            RankEdgeKind::DefinedBy => RANK_DEFINED_BY,
        }
    }
}

/// The full containment hierarchy, top → bottom, real strings from
/// `scc_core::kinds` (all six kinds exist there). A CONTAINS edge whose
/// subject precedes its object in this list is container → member
/// membership evidence; the stored member → container direction is the
/// reverse RANKING TRANSITION (member importance flows up). Covers every
/// hierarchy pair — (System, Subsystem), (Subsystem, Service),
/// (Subsystem, Component), (Service, Component), (Component, File),
/// (Component, Symbol), (File, Symbol) and every transitive container
/// pair — not just the adjacent ones.
const CONTAINMENT_HIERARCHY: [&str; 6] = [
    kinds::SYSTEM,
    kinds::SUBSYSTEM,
    kinds::SERVICE,
    kinds::COMPONENT,
    kinds::FILE,
    kinds::SYMBOL,
];

/// Position of a kind in [`CONTAINMENT_HIERARCHY`]; `None` for kinds
/// outside the containment hierarchy (they never fire containment edges).
// trace:exempt reason=internal-detail
fn containment_position(kind: &str) -> Option<usize> {
    CONTAINMENT_HIERARCHY.iter().position(|k| *k == kind)
}

/// Map a (predicate, subject kind, target kind) triple to its rank-edge
/// kind, when the relationship is one of the explicit rank-edge
/// predicates (real strings from `scc_core::predicates`). Direction is
/// deliberate: only the documented endpoint kinds fire — `contains`
/// only between kinds in [`CONTAINMENT_HIERARCHY`] with the container
/// before the member (the stored member → container direction maps to
/// the `MemberOf` RANKING TRANSITION), `participates_in` only between a
/// symbol and a flow (stored flow → symbol maps to
/// `FlowReachesParticipant`), `handles` only symbol ↔ route (stored
/// route → symbol maps to the `HandledBy` RANKING TRANSITION), `defines`
/// only symbol ↔ schema (stored schema → symbol maps to `DefinedBy`).
// trace:exempt reason=internal-detail
pub fn rank_edge_kind(
    predicate: &str,
    subject_kind: &str,
    target_kind: &str,
) -> Option<RankEdgeKind> {
    match predicate {
        predicates::CONTAINS => {
            let (Some(sp), Some(tp)) = (
                containment_position(subject_kind),
                containment_position(target_kind),
            ) else {
                return None;
            };
            if sp < tp {
                Some(RankEdgeKind::Contains)
            } else if tp < sp {
                Some(RankEdgeKind::MemberOf)
            } else {
                None
            }
        }
        predicates::PARTICIPATES_IN
            if subject_kind == kinds::SYMBOL && target_kind == kinds::FLOW =>
        {
            Some(RankEdgeKind::ParticipatesIn)
        }
        predicates::PARTICIPATES_IN
            if subject_kind == kinds::FLOW && target_kind == kinds::SYMBOL =>
        {
            Some(RankEdgeKind::FlowReachesParticipant)
        }
        predicates::HANDLES if subject_kind == kinds::SYMBOL && target_kind == kinds::ROUTE => {
            Some(RankEdgeKind::Handles)
        }
        // Reverse of `handles`: RANKING TRANSITION route → handler symbol.
        predicates::HANDLES if subject_kind == kinds::ROUTE && target_kind == kinds::SYMBOL => {
            Some(RankEdgeKind::HandledBy)
        }
        predicates::DEFINES if subject_kind == kinds::SYMBOL && target_kind == kinds::SCHEMA => {
            Some(RankEdgeKind::Defines)
        }
        // Reverse of `defines`: RANKING TRANSITION schema → definer symbol.
        predicates::DEFINES if subject_kind == kinds::SCHEMA && target_kind == kinds::SYMBOL => {
            Some(RankEdgeKind::DefinedBy)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// SystemRanker
// ---------------------------------------------------------------------------

/// Personalized PageRank over the HETEROGENEOUS entity reference graph.
///
/// Nodes are every rankable entity (id-sorted for determinism): symbols,
/// components, subsystems, services, flows, contracts, state, reactive,
/// routes, topics, queues, stores, schemas and files. Edges are trusted
/// relationships whose both endpoints are rankable entities and whose
/// predicate normalizes to a reference kind; weights follow the spec
/// formula. The global vector is computed once per epoch (Main caches the
/// `SystemRanker`); task vectors personalize from `TaskSeed`s with a
/// warm start blended from the global vector.
// trace:exempt reason=internal-detail
pub struct SystemRanker<'a> {
    view: &'a TrustedGraphView<'a>,
    /// Sorted rankable entity ids; index i in every vector == `nodes[i]`.
    nodes: Vec<String>,
    /// Entity kinds, parallel to `nodes`.
    kinds: Vec<String>,
    index: HashMap<String, usize>,
    /// Row-normalized out-edge adjacency: (target index, weight).
    adjacency: Vec<Vec<(usize, f64)>>,
    /// Distinct referencing sources per node (rarity/ubiquity input).
    in_degree: Vec<usize>,
    /// For each node: indices of the non-symbol entities it projects onto
    /// (OWNS/REGISTERS/PUBLISHES/READS/WRITES/PARTICIPATES_IN/HANDLES/
    /// DEFINES targets — HANDLES/DEFINES so a hot Route/Schema node
    /// reaches its handler/definer symbol).
    projection: Vec<Vec<usize>>,
    /// Precomputed global (architecturally seeded) PageRank vector.
    global: Vec<f64>,
}

// trace:exempt reason=internal-detail
impl<'a> SystemRanker<'a> {
    /// Build the ranker from the trusted view. Deterministic; O(E) edges.
// trace:exempt reason=internal-detail
    pub fn new(view: &'a TrustedGraphView<'a>) -> SystemRanker<'a> {
        let mut pairs: Vec<(String, String)> = view
            .entities()
            .filter(|e| RANKABLE_KINDS.contains(&e.kind.as_str()))
            .map(|e| (e.id.clone(), e.kind.clone()))
            .collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        let nodes: Vec<String> = pairs.iter().map(|(id, _)| id.clone()).collect();
        let kinds: Vec<String> = pairs.iter().map(|(_, k)| k.clone()).collect();
        let index: HashMap<String, usize> = nodes
            .iter()
            .enumerate()
            .map(|(i, s)| (s.clone(), i))
            .collect();
        let n = nodes.len();

        // Pass 1: collect edges + per-target distinct sources. An edge
        // survives when BOTH endpoints are rankable entities (the universe
        // is heterogeneous: non-symbol endpoints are first-class nodes) and
        // the predicate normalizes to a reference kind OR carries a
        // rank-edge transition (the deliberate direction/weight ontology —
        // CONTAINS/PARTICIPATES_IN/HANDLES/DEFINES; see [`RankEdgeKind`]).
        let mut edges: Vec<(usize, usize, f64)> = Vec::new();
        let mut in_sources: Vec<HashSet<usize>> = vec![HashSet::new(); n];
        for rel in view.all_rels() {
            let (Some(&si), Some(&ti)) = (index.get(&rel.subject), index.get(&rel.object)) else {
                continue;
            };
            let subject_kind = view
                .entity(&rel.subject)
                .map(|e| e.kind.as_str())
                .unwrap_or("");
            let target_kind = view
                .entity(&rel.object)
                .map(|e| e.kind.as_str())
                .unwrap_or("");
            // Normalized reference-surface edges (the existing surface:
            // calls/reads/writes/owns/…).
            let reference_weight = if reference_kind(&rel.predicate, target_kind).is_some() {
                Some(
                    predicate_weight(&rel.predicate, rel.provenance)
                        * provenance_weight(rel.provenance)
                        * rel.confidence.clamp(0.0, 1.0),
                )
            } else {
                None
            };
            // Rank-edge transitions (the deliberate direction/weight
            // ontology): the stored direction plus the reverse ranking
            // transition when the ontology defines one.
            if let Some(kind) = rank_edge_kind(&rel.predicate, subject_kind, target_kind) {
                let w = kind.weight()
                    * provenance_weight(rel.provenance)
                    * rel.confidence.clamp(0.0, 1.0);
                in_sources[ti].insert(si);
                edges.push((si, ti, w));
                if let Some(rev) = kind.reverse_ranking_transition() {
                    let rw = rev.weight()
                        * provenance_weight(rel.provenance)
                        * rel.confidence.clamp(0.0, 1.0);
                    in_sources[si].insert(ti);
                    edges.push((ti, si, rw));
                }
            }
            if let Some(w) = reference_weight {
                in_sources[ti].insert(si);
                edges.push((si, ti, w));
            }
        }
        let in_degree: Vec<usize> = in_sources.iter().map(|s| s.len()).collect();

        // Aggregate parallel edges (sum weights), apply rarity, row-normalize.
        let mut agg: Vec<HashMap<usize, f64>> = vec![HashMap::new(); n];
        for (si, ti, w) in edges {
            let r = rarity(n, in_degree[ti]);
            *agg[si].entry(ti).or_insert(0.0) += w * r;
        }
        let mut adjacency: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
        for (i, row) in agg.iter().enumerate() {
            let sum: f64 = row.values().sum();
            if sum > 0.0 {
                let mut v: Vec<(usize, f64)> =
                    row.iter().map(|(j, w)| (*j, w / sum)).collect();
                v.sort_by_key(|(j, _)| *j);
                adjacency[i] = v;
            }
        }

        // Projection edges: non-symbol targets of the symbol's
        // OWNS/REGISTERS/PUBLISHES/READS/WRITES/PARTICIPATES_IN/HANDLES/
        // DEFINES rels (HANDLES/DEFINES so a hot Route/Schema node
        // reaches its handler/definer symbol).
        let mut projection: Vec<Vec<usize>> = vec![Vec::new(); n];
        for rel in view.all_rels() {
            if !PROJECTION_PREDICATES.contains(&rel.predicate.as_str()) {
                continue;
            }
            let (Some(&si), Some(&ti)) = (index.get(&rel.subject), index.get(&rel.object)) else {
                continue;
            };
            if kinds[ti] == kinds::SYMBOL {
                continue; // the bonus carries entity importance, not symbol
            }
            projection[si].push(ti);
        }
        for p in projection.iter_mut() {
            p.sort_unstable();
            p.dedup();
        }

        let ranker = SystemRanker {
            view,
            nodes,
            kinds,
            index,
            adjacency,
            in_degree,
            projection,
            global: Vec::new(),
        };
        let global = Self::ppr(&ranker.adjacency, &ranker.global_personalization());
        let mut ranker = ranker;
        ranker.global = global;
        ranker
    }

    /// Rankable entity ids in rank order (index i in every vector maps to
    /// `nodes()[i]`; symbols are a subset).
// trace:exempt reason=internal-detail
    pub fn nodes(&self) -> &[String] {
        &self.nodes
    }

    /// Ranked symbol entity ids (subset of [`SystemRanker::nodes`]),
    /// id-sorted. Symbol-only callers use this to walk the symbol slice of
    /// the heterogeneous vectors.
// trace:exempt reason=internal-detail
    pub fn symbols(&self) -> Vec<String> {
        self.nodes
            .iter()
            .zip(self.kinds.iter())
            .filter(|(_, k)| k.as_str() == kinds::SYMBOL)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Node index for a rankable entity id, if it is in the universe.
// trace:exempt reason=internal-detail
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.index.get(id).copied()
    }

    /// Distinct referencing sources of a node id (0 when absent).
// trace:exempt reason=internal-detail
    pub fn in_degree(&self, id: &str) -> usize {
        self.index
            .get(id)
            .map(|i| self.in_degree[*i])
            .unwrap_or(0)
    }

    /// Precomputed global PageRank vector (architecturally seeded; see
    /// [`SystemRanker::global_personalization`]). Main caches the ranker per
    /// epoch and reads this.
// trace:exempt reason=internal-detail
    pub fn global_vector(&self) -> Vec<f64> {
        self.global.clone()
    }

    /// Project the heterogeneous node scores to surface-relevant symbol
    /// scores: a symbol's score = its own score + 0.4 × the sum of the
    /// scores of the entities it OWNS/REGISTERS/PUBLISHES/READS/WRITES/
    /// PARTICIPATES_IN/HANDLES/DEFINES, with the total bonus capped at
    /// 0.5. This is how component/flow/contract/state importance reaches
    /// the surface — and, via the HANDLES/DEFINES projection, how a hot
    /// Route/Schema node (task seed or PPR) reaches its handler/definer
    /// symbol. Deterministic: nodes and projection edges are id-sorted.
    ///
    /// Returns `(symbol id, projected score)` pairs, id-sorted.
// trace:v1 id=impl.scc.pagerank.project-to-symbols work=WORK-SCC-014 satisfies=REQ-SCC-IR
    pub fn project_to_symbols(&self, vector: &[f64]) -> Vec<(String, f64)> {
        let mut scores: Vec<f64> = Vec::with_capacity(self.nodes.len());
        for (i, _id) in self.nodes.iter().enumerate() {
            let own = vector.get(i).copied().unwrap_or(0.0);
            let mut bonus = 0.0;
            for &j in self.projection[i].iter() {
                bonus += vector.get(j).copied().unwrap_or(0.0);
            }
            scores.push(own + (PROJECTION_BONUS_FACTOR * bonus).min(PROJECTION_BONUS_CAP));
        }
        let mut out: Vec<(String, f64)> = Vec::new();
        for (i, id) in self.nodes.iter().enumerate() {
            if self.kinds[i] == kinds::SYMBOL {
                out.push((id.clone(), scores[i]));
            }
        }
        out
    }

    /// Task-personalized PageRank vector. Seeds come from `TaskSeed`s whose
    /// `id` is a rankable entity id; the seed vector carries `weight` at
    /// each matched index. Warm start: 0.3 × global + 0.7 × normalized
    /// seeds. Unresolvable seed sets fall back to the global vector (never
    /// panic).
// trace:exempt reason=internal-detail
    pub fn task_vector(&self, seeds: &[TaskSeed]) -> Vec<f64> {
        let n = self.nodes.len();
        if n == 0 {
            return Vec::new();
        }
        let mut s = vec![0.0; n];
        let mut found = false;
        for seed in seeds {
            if let Some(&i) = self.index.get(&seed.id) {
                s[i] += seed.weight.max(0.0);
                found = true;
            }
        }
        if !found {
            return self.global.clone();
        }
        let sum: f64 = s.iter().sum();
        if sum <= 0.0 {
            return self.global.clone();
        }
        let s: Vec<f64> = s.iter().map(|v| v / sum).collect();
        let mut start = vec![0.0; n];
        for i in 0..n {
            start[i] = WARM_START_GLOBAL_BLEND * self.global[i] + (1.0 - WARM_START_GLOBAL_BLEND) * s[i];
        }
        Self::ppr_with(&self.adjacency, &s, &start)
    }

    /// Architectural seed vector (weight 1.0 per seed): exported symbols,
    /// symbols with entrypoint attributes, invocation-surface handlers,
    /// primary entrypoints, state owners, contract producers, and flow
    /// entrypoints. Applied to every node id (non-symbol nodes simply do
    /// not carry symbol attributes).
// trace:exempt reason=internal-detail
    fn global_personalization(&self) -> Vec<f64> {
        let n = self.nodes.len();
        let mut seeds = vec![0.0; n];
        // Flow entrypoints: flows record the entry symbol id in attributes.
        let mut flow_eps: HashSet<String> = HashSet::new();
        for f in self.view.flows() {
            if let Some(ep) = f.attributes.get("entrypoint").and_then(|v| v.as_str()) {
                flow_eps.insert(ep.to_string());
            }
        }
        for (i, id) in self.nodes.iter().enumerate() {
            let mut seed = false;
            if let Some(e) = self.view.entity(id) {
                let exported = e
                    .attributes
                    .get("exported")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let has_entrypoints = e
                    .attributes
                    .get("entrypoints")
                    .and_then(|v| v.as_array())
                    .map(|a| !a.is_empty())
                    .unwrap_or(false);
                seed = seed || exported || has_entrypoints;
            }
            if flow_eps.contains(id.as_str()) {
                seed = true;
            }
            for r in self.view.out_edges(id) {
                let tk = self
                    .view
                    .entity(&r.object)
                    .map(|e| e.kind.as_str())
                    .unwrap_or("");
                match r.predicate.as_str() {
                    predicates::HANDLES if INVOCATION_KINDS.contains(&tk) => seed = true,
                    predicates::OWNS if tk == kinds::STATE || tk == kinds::REACTIVE => seed = true,
                    predicates::REGISTERS | predicates::PUBLISHES
                        if CONTRACT_KINDS.contains(&tk) =>
                    {
                        seed = true;
                    }
                    _ => {}
                }
            }
            if seed {
                seeds[i] = 1.0;
            }
        }
        seeds
    }

    /// Personalized power iteration: `r' = (1-d)·s + d·(M^T r +
    /// dangling/n)`, `POWER_ITERATIONS` times, starting from `start`.
// trace:exempt reason=internal-detail
    fn ppr_with(adjacency: &[Vec<(usize, f64)>], personalization: &[f64], start: &[f64]) -> Vec<f64> {
        let n = adjacency.len();
        if n == 0 {
            return Vec::new();
        }
        let mut r = start.to_vec();
        let d = DAMPING_FACTOR;
        for _ in 0..POWER_ITERATIONS {
            let mut nr = vec![0.0; n];
            let mut dangling = 0.0;
            for (i, row) in adjacency.iter().enumerate() {
                let ri = r[i];
                if row.is_empty() {
                    dangling += ri;
                } else {
                    for (j, w) in row {
                        nr[*j] += ri * w;
                    }
                }
            }
            let d_mass = d * dangling / n as f64;
            for k in 0..n {
                nr[k] = (1.0 - d) * personalization[k] + d * nr[k] + d_mass;
            }
            r = nr;
        }
        r
    }

    /// Global run: personalization = normalized architectural seeds; start
    /// state = the same seed distribution (uniform when no seed fires).
// trace:exempt reason=internal-detail
    fn ppr(adjacency: &[Vec<(usize, f64)>], seeds: &[f64]) -> Vec<f64> {
        let n = adjacency.len();
        if n == 0 {
            return Vec::new();
        }
        let sum: f64 = seeds.iter().sum();
        let s: Vec<f64> = if sum > 0.0 {
            seeds.iter().map(|v| v / sum).collect()
        } else {
            vec![1.0 / n as f64; n]
        };
        Self::ppr_with(adjacency, &s, &s)
    }
}

// ---------------------------------------------------------------------------
// Architectural specificity
// ---------------------------------------------------------------------------

/// Is the qualified name or path segment a generic utility token?
// trace:exempt reason=internal-detail
fn is_generic_utility(qualified_name: &str, path: &str) -> bool {
    let mut segs: Vec<String> = Vec::new();
    for part in path.split('/') {
        for piece in part.split('.') {
            if !piece.is_empty() {
                segs.push(piece.to_lowercase());
            }
        }
    }
    for part in qualified_name.split(['.', ':']) {
        if !part.is_empty() {
            segs.push(part.to_lowercase());
        }
    }
    segs.iter().any(|s| GENERIC_NAMES.contains(&s.as_str()))
}

/// Does the path point into test code (`test`/`tests`/`spec` segments,
/// `test_*`/`*_test` files)?
// trace:exempt reason=internal-detail
fn path_has_test(path: &str) -> bool {
    path.split('/').any(|seg| {
        let seg = seg.to_lowercase();
        seg == "test"
            || seg == "tests"
            || seg == "spec"
            || seg.starts_with("test_")
            || seg.ends_with("_test")
    })
}

/// Does the path point at generated code (`generated`/`gen_*` segments,
/// protobuf markers)?
// trace:exempt reason=internal-detail
fn is_generated(path: &str) -> bool {
    path.split('/').any(|seg| {
        let seg = seg.to_lowercase();
        seg.contains("generated")
            || seg.starts_with("gen_")
            || seg.ends_with("_pb2.py")
            || seg.ends_with("_pb.go")
    })
}

/// Does the path point into vendored code?
// trace:exempt reason=internal-detail
fn is_vendored(path: &str) -> bool {
    path.split('/').any(|seg| {
        matches!(
            seg,
            "vendor" | "third_party" | "node_modules" | ".venv" | "site-packages" | "bower_components"
        )
    })
}

/// Ubiquity: distinct referencing sources of the symbol in the reference
/// graph (only predicates that normalize to a reference kind).
// trace:exempt reason=internal-detail
fn ubiquity(view: &TrustedGraphView, symbol_id: &str) -> usize {
    let mut sources: HashSet<&str> = HashSet::new();
    for r in view.in_edges(symbol_id) {
        let tk = view
            .entity(&r.object)
            .map(|e| e.kind.as_str())
            .unwrap_or("");
        if reference_kind(&r.predicate, tk).is_some() {
            sources.insert(&r.subject);
        }
    }
    sources.len()
}

/// Architectural specificity of a surface entry: a multiplier in
/// [0.25, 1.25] that boosts public facades/entrypoints/state owners/
/// contract endpoints/flow participants and penalizes generic utilities,
/// test/generated/vendored code, and ubiquitous symbols (in-degree above
/// [`UBIQUITY_THRESHOLD`]). Apply it to PPR values before blending so
/// central utilities never dominate the ranking.
// trace:exempt reason=internal-detail
pub fn architectural_specificity(entry: &SurfaceEntry, view: &TrustedGraphView) -> f64 {
    let mut score: f64 = 1.0;
    if entry.exported {
        score += 0.15;
    }
    if !entry.invocation_surfaces.is_empty() {
        score += 0.10;
    }
    if !entry.state_authorities.is_empty() {
        score += 0.10;
    }
    if !entry.contracts.is_empty() {
        score += 0.10;
    }
    if !entry.flows.is_empty() {
        score += 0.05;
    }
    // Public facade: exported member of a component.
    if entry.exported && entry.component.is_some() {
        score += 0.05;
    }

    let mut factor = 1.0;
    if is_generic_utility(&entry.qualified_name, &entry.path) {
        factor *= 0.5;
    }
    if path_has_test(&entry.path) {
        factor *= 0.5;
    }
    if is_generated(&entry.path) {
        factor *= 0.5;
    }
    if is_vendored(&entry.path) {
        factor *= 0.5;
    }
    if ubiquity(view, &entry.symbol_id) > UBIQUITY_THRESHOLD {
        factor *= 0.5;
    }

    (score * factor).clamp(0.25, 1.25)
}

// ---------------------------------------------------------------------------
// Final importance blend
// ---------------------------------------------------------------------------

/// 30% task PPR.
pub const TASK_PPR_WEIGHT: f64 = 0.30;
/// 20% global PPR (50% when no task focus).
pub const GLOBAL_PPR_WEIGHT: f64 = 0.20;
/// 15% lexical overlap.
pub const LEXICAL_WEIGHT: f64 = 0.15;
/// 10% semantic relevance.
pub const SEMANTIC_WEIGHT: f64 = 0.10;
/// 10% evidence confidence.
pub const CONFIDENCE_WEIGHT: f64 = 0.10;
/// 10% criticality.
pub const CRITICALITY_WEIGHT: f64 = 0.10;
/// 5% change/risk.
pub const CHANGE_RISK_WEIGHT: f64 = 0.05;
/// Novelty bonus weight (additive term).
pub const NOVELTY_WEIGHT: f64 = 0.05;
/// Global PPR weight when there is no task focus (task share moves to
/// global: 20% + 30% = 50%).
pub const NO_TASK_GLOBAL_WEIGHT: f64 = 0.50;

/// The spec's final importance blend. All inputs are expected in [0, 1];
/// the novelty term is additive on top. With no task focus, the task-PPR
/// share moves to the global vector (global gets 50%).
///
/// `has_task` — is this ranking task-focused (true) or the startup/global
/// surface (false)?
#[allow(clippy::too_many_arguments)]
// trace:exempt reason=internal-detail
pub fn final_importance(
    task_ppr: f64,
    global_ppr: f64,
    lexical: f64,
    semantic: f64,
    confidence: f64,
    criticality: f64,
    change_risk: f64,
    novelty: f64,
    has_task: bool,
) -> f64 {
    let (tw, gw) = if has_task {
        (TASK_PPR_WEIGHT, GLOBAL_PPR_WEIGHT)
    } else {
        (0.0, NO_TASK_GLOBAL_WEIGHT)
    };
    tw * task_ppr
        + gw * global_ppr
        + LEXICAL_WEIGHT * lexical
        + SEMANTIC_WEIGHT * semantic
        + CONFIDENCE_WEIGHT * confidence
        + CRITICALITY_WEIGHT * criticality
        + CHANGE_RISK_WEIGHT * change_risk
        + NOVELTY_WEIGHT * novelty
}

#[cfg(test)]
mod tests {
    use super::*;
    use scc_core::{entity_id, Entity, Relationship, Visibility};
    use scc_graph::{RealityGraph, TrustPolicy};
    use scc_store::Store;
    use std::collections::HashMap;

// trace:exempt reason=internal-detail
    fn fixture(
        entities: Vec<Entity>,
        rels: Vec<Relationship>,
    ) -> (tempfile::TempDir, Store, RealityGraph) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        for e in &entities {
            store.insert_entity(e, &["src/fixture.ts".to_string()]).unwrap();
        }
        for r in &rels {
            store.insert_relationship(r, "src/fixture.ts").unwrap();
        }
        let mut out: HashMap<String, Vec<Relationship>> = HashMap::new();
        let mut inn: HashMap<String, Vec<Relationship>> = HashMap::new();
        for r in &rels {
            out.entry(r.subject.clone()).or_default().push(r.clone());
            inn.entry(r.object.clone()).or_default().push(r.clone());
        }
        let graph = RealityGraph {
            repo_id: "r".into(),
            entities: entities.into_iter().map(|e| (e.id.clone(), e)).collect(),
            out,
            inn,
            components: vec![],
            flows: vec![],
            invariants: vec![],
        };
        (dir, store, graph)
    }

// trace:exempt reason=internal-detail
    fn sym(repo: &str, path: &str, name: &str) -> Entity {
        let mut e = Entity::new(
            scc_core::symbol_id(repo, path, name),
            kinds::SYMBOL,
            name,
        );
        e.attr("file", serde_json::json!(path));
        e.attr("exported", serde_json::json!(false));
        e.attr("start_line", serde_json::json!(1));
        e.attr("end_line", serde_json::json!(10));
        e
    }

// trace:exempt reason=internal-detail
    fn entity(id: &str, kind: &str, name: &str) -> Entity {
        Entity::new(id, kind, name)
    }

// trace:exempt reason=internal-detail
    fn rel(n: u64, subject: &str, pred: &str, object: &str, prov: Provenance) -> Relationship {
        Relationship::new(format!("rel:{n}"), subject, pred, object, prov)
    }

    // ---- (a) reference normalization maps the right kinds ----

    #[test]
// trace:exempt reason=internal-detail
    fn reference_normalization_maps_kinds() {
        let (a, b, i, c, s, t, m, x, d) = (
            scc_core::symbol_id("r", "src/a.ts", "A"),
            scc_core::symbol_id("r", "src/b.ts", "B"),
            scc_core::symbol_id("r", "src/i.ts", "I"),
            "repo://r/contract/c",
            "repo://r/state/s",
            "repo://r/topic/t",
            "repo://r/module/m",
            "repo://r/export/x",
            "repo://r/annotation/d",
        );
        let mut a_ent = sym("r", "src/a.ts", "A");
        a_ent.attr("file", serde_json::json!("src/a.ts"));
        a_ent.attr("start_line", serde_json::json!(10));
        a_ent.attr("end_line", serde_json::json!(20));
        let b_ent = sym("r", "src/b.ts", "B");
        let i_ent = sym("r", "src/i.ts", "I");
        let c_ent = entity(c, kinds::CONTRACT, "c");
        let s_ent = entity(s, kinds::STATE, "s");
        let t_ent = entity(t, kinds::TOPIC, "t");
        let m_ent = entity(m, kinds::MODULE, "m");
        let x_ent = entity(x, kinds::EXPORT, "x");
        let d_ent = entity(d, kinds::ANNOTATION, "d");

        let mut rels = vec![
            rel(1, &a, predicates::CALLS, &b, Provenance::Extracted),
            rel(2, &a, predicates::INVOKES, &b, Provenance::Extracted),
            rel(3, &a, predicates::HANDLES_CALLBACK, &b, Provenance::Extracted),
            rel(4, &a, predicates::IMPLEMENTS, &i, Provenance::Extracted),
            rel(5, &i, predicates::IMPLEMENTED_BY, &a, Provenance::Extracted),
            rel(6, &a, predicates::INHERITS, &b, Provenance::Extracted),
            rel(7, &a, predicates::REGISTERS, c, Provenance::Extracted),
            rel(8, &a, predicates::PUBLISHES, t, Provenance::Extracted),
            rel(9, &b, predicates::SUBSCRIBES, t, Provenance::Extracted),
            rel(10, &a, predicates::OWNS, s, Provenance::Extracted),
            rel(11, &a, predicates::READS, s, Provenance::Extracted),
            rel(12, &b, predicates::WRITES, s, Provenance::Extracted),
            rel(13, &a, predicates::IMPORTS, m, Provenance::Extracted),
            rel(14, &a, predicates::EXPORTS, x, Provenance::Extracted),
            rel(15, &a, predicates::DECORATES, d, Provenance::Extracted),
            // OWNS of a non-state entity must NOT normalize to Write.
            rel(16, &a, predicates::OWNS, c, Provenance::Extracted),
            // Confidence passthrough.
            rel(17, &a, predicates::CALLS, d, Provenance::Extracted),
        ];
        rels[16].confidence = 0.7;
        let (_dir, store, graph) = fixture(
            vec![a_ent.clone(), b_ent, i_ent, c_ent, s_ent, t_ent, m_ent, x_ent, d_ent],
            rels,
        );
        let view = TrustedGraphView::new(&graph, &store, &[], TrustPolicy::default());
        let edges = build_reference_graph(&view);

        let kinds_between = |src: &str, tgt: &str| -> Vec<ReferenceKind> {
            let mut v: Vec<ReferenceKind> = edges
                .iter()
                .filter(|e| e.source_symbol == src && e.target_symbol == tgt)
                .map(|e| e.kind)
                .collect();
            v.sort();
            v
        };

        // calls/invokes/handles_callback → Call (3 edges) and
        // inherits → Extend (all A -> B edges).
        assert_eq!(
            kinds_between(&a, &b),
            vec![
                ReferenceKind::Call,
                ReferenceKind::Call,
                ReferenceKind::Call,
                ReferenceKind::Extend
            ]
        );
        // implements + implemented_by → Implement (both directions normalize
        // to A -> I).
        assert_eq!(kinds_between(&a, &i), vec![ReferenceKind::Implement; 2]);
        // registers/publishes/subscribes → Register.
        assert_eq!(kinds_between(&a, c), vec![ReferenceKind::Register]);
        assert_eq!(kinds_between(&a, t), vec![ReferenceKind::Register]);
        assert_eq!(kinds_between(&b, t), vec![ReferenceKind::Register]);
        // owns(state) → Write; reads → Read; writes → Write.
        let mut as_edges = kinds_between(&a, s);
        as_edges.sort();
        assert_eq!(as_edges, vec![ReferenceKind::Read, ReferenceKind::Write]);
        assert_eq!(kinds_between(&b, s), vec![ReferenceKind::Write]);
        // imports → Import; exports → Export; decorates → Decorate.
        assert_eq!(kinds_between(&a, m), vec![ReferenceKind::Import]);
        assert_eq!(kinds_between(&a, x), vec![ReferenceKind::Export]);
        assert_eq!(kinds_between(&a, d), vec![ReferenceKind::Call, ReferenceKind::Decorate]);
        // OWNS of a contract is NOT normalized (no Write A->C).
        assert!(!kinds_between(&a, c).contains(&ReferenceKind::Write));

        // Locations: evidence file/line from the subject symbol attrs.
        let call = edges
            .iter()
            .find(|e| e.source_symbol == a && e.target_symbol == b && e.kind == ReferenceKind::Call)
            .unwrap();
        assert!(call.locations.contains(&SourceRange::new("src/a.ts", 10, 20)));
        assert!(call.locations.contains(&SourceRange::new("src/b.ts", 1, 10)));

        // Confidence passthrough: default 1.0; explicit 0.7 preserved.
        assert_eq!(call.confidence, 1.0);
        let low = edges
            .iter()
            .find(|e| e.source_symbol == a && e.target_symbol == d && e.kind == ReferenceKind::Call)
            .unwrap();
        assert!((low.confidence - 0.7).abs() < 1e-6);
    }

    // ---- (b) rarity downweights ubiquitous vs distinctive ----

    #[test]
// trace:exempt reason=internal-detail
    fn rarity_downweights_ubiquitous() {
        // Distinctive target (in-degree 1) is far rarer than a ubiquitous
        // logger-ish target (in-degree 50).
        let logger = rarity(100, 50);
        let distinctive = rarity(100, 1);
        assert!(logger < distinctive);
        assert!((logger - (100.0_f64 / 51.0).ln()).abs() < 1e-9);

        // Clamp bounds.
        assert_eq!(rarity(100, 10_000), 0.25);
        assert_eq!(rarity(100, 0), 1.5);
        assert_eq!(rarity(0, 0), 1.0);

        // Full edge weight: ubiquitous target downweights the edge.
        let w_common = edge_weight(predicates::CALLS, Provenance::Extracted, 1.0, 100, 50);
        let w_rare = edge_weight(predicates::CALLS, Provenance::Extracted, 1.0, 100, 1);
        assert!(w_common < w_rare);

        // Provenance: STALE weighs zero; RESOLVED calls weigh more than
        // EXTRACTED calls.
        assert_eq!(edge_weight(predicates::CALLS, Provenance::Stale, 1.0, 100, 1), 0.0);
        let resolved = edge_weight(predicates::CALLS, Provenance::Resolved, 1.0, 100, 1);
        let extracted = edge_weight(predicates::CALLS, Provenance::Extracted, 1.0, 100, 1);
        assert!(resolved > extracted);
    }

    // ---- (c) central utility pollution is corrected by specificity ----

    #[test]
// trace:exempt reason=internal-detail
    fn utility_pollution_ranks_below_public_api() {
        let api_id = scc_core::symbol_id("r", "src/api/order.ts", "OrderApi");
        let util_id = scc_core::symbol_id("r", "src/utils/helpers.ts", "helpers");
        let mut api_ent = sym("r", "src/api/order.ts", "OrderApi");
        api_ent.attr("exported", serde_json::json!(true));
        api_ent.attr("entrypoints", serde_json::json!(["http"]));
        let util_ent = sym("r", "src/utils/helpers.ts", "helpers");

        let mut entities = vec![api_ent.clone(), util_ent.clone()];
        let mut rels: Vec<Relationship> = Vec::new();
        let mut n = 1u64;
        // 40 callers all invoke the util symbol.
        for i in 0..40 {
            let caller = scc_core::symbol_id("r", "src/callers/c.rs", &format!("c{i}"));
            entities.push(sym("r", "src/callers/c.rs", &format!("c{i}")));
            rels.push(rel(
                n,
                &caller,
                predicates::CALLS,
                &util_id,
                Provenance::Extracted,
            ));
            n += 1;
        }
        // Two callers reach the public API.
        let c1 = scc_core::symbol_id("r", "src/callers/c.rs", "c0");
        let c2 = scc_core::symbol_id("r", "src/callers/c.rs", "c1");
        rels.push(rel(n, &c1, predicates::CALLS, &api_id, Provenance::Extracted));
        n += 1;
        rels.push(rel(n, &c2, predicates::CALLS, &api_id, Provenance::Extracted));

        let (_dir, store, graph) = fixture(entities, rels);
        let view = TrustedGraphView::new(&graph, &store, &[], TrustPolicy::default());
        let ranker = SystemRanker::new(&view);
        let g = ranker.global_vector();
        let util_idx = ranker.index_of(&util_id).unwrap();
        let api_idx = ranker.index_of(&api_id).unwrap();

        // Specificity: util is generic + ubiquitous (40 distinct callers >
        // threshold) → floor; api is an exported entrypoint facade → cap.
        let util_entry = surface_entry(&util_id, "helpers", "src/utils/helpers.ts", false);
        let api_entry = surface_entry(&api_id, "OrderApi", "src/api/order.ts", true);
        let spec_util = architectural_specificity(&util_entry, &view);
        let spec_api = architectural_specificity(&api_entry, &view);
        assert_eq!(spec_util, 0.25);
        assert_eq!(spec_api, 1.25);
        assert!(spec_api > spec_util);

        // After specificity, the public API ranks above the central utility
        // even though the utility is referenced by far more callers.
        assert!(g[api_idx] * spec_api > g[util_idx] * spec_util);
    }

    // ---- (d) task personalization lifts billing seeds above unrelated ----

    #[test]
// trace:exempt reason=internal-detail
    fn task_personalization_ranks_billing_above_unrelated() {
        let billing_client = scc_core::symbol_id("r", "src/billing/client.ts", "BillingClient");
        let billing_retry = scc_core::symbol_id("r", "src/billing/client.ts", "BillingClient.retry");
        let billing_worker = scc_core::symbol_id("r", "src/billing/worker.ts", "BillingWorker");
        let billing_process = scc_core::symbol_id("r", "src/billing/worker.ts", "BillingWorker.process");
        let auth_service = scc_core::symbol_id("r", "src/auth/service.ts", "AuthService");
        let auth_login = scc_core::symbol_id("r", "src/auth/service.ts", "AuthService.login");
        let logger = scc_core::symbol_id("r", "src/logger.ts", "Logger");
        let logger_log = scc_core::symbol_id("r", "src/logger.ts", "Logger.log");

        let entities = vec![
            sym("r", "src/billing/client.ts", "BillingClient"),
            sym("r", "src/billing/client.ts", "BillingClient.retry"),
            sym("r", "src/billing/worker.ts", "BillingWorker"),
            sym("r", "src/billing/worker.ts", "BillingWorker.process"),
            sym("r", "src/auth/service.ts", "AuthService"),
            sym("r", "src/auth/service.ts", "AuthService.login"),
            sym("r", "src/logger.ts", "Logger"),
            sym("r", "src/logger.ts", "Logger.log"),
        ];
        let rels = vec![
            rel(1, &billing_client, predicates::CALLS, &billing_retry, Provenance::Extracted),
            rel(2, &billing_worker, predicates::CALLS, &billing_process, Provenance::Extracted),
            rel(3, &billing_process, predicates::CALLS, &billing_retry, Provenance::Extracted),
            rel(4, &auth_service, predicates::CALLS, &auth_login, Provenance::Extracted),
            rel(5, &logger, predicates::CALLS, &logger_log, Provenance::Extracted),
        ];

        let (_dir, store, graph) = fixture(entities, rels);
        let view = TrustedGraphView::new(&graph, &store, &[], TrustPolicy::default());
        let ranker = SystemRanker::new(&view);

        let seeds = vec![
            TaskSeed {
                kind: "symbol".into(),
                id: billing_retry.clone(),
                weight: 1.0,
            },
            TaskSeed {
                kind: "symbol".into(),
                id: billing_process.clone(),
                weight: 1.0,
            },
        ];
        let tv = ranker.task_vector(&seeds);
        let i_retry = ranker.index_of(&billing_retry).unwrap();
        let i_process = ranker.index_of(&billing_process).unwrap();
        let i_login = ranker.index_of(&auth_login).unwrap();
        let i_log = ranker.index_of(&logger_log).unwrap();

        // Billing seeds rank above unrelated symbols.
        assert!(tv[i_retry] > tv[i_login]);
        assert!(tv[i_retry] > tv[i_log]);
        assert!(tv[i_process] > tv[i_login]);
        assert!(tv[i_process] > tv[i_log]);
        // Both seeds beat a symbol that merely calls into the unrelated
        // cluster's leaf.
        assert!(tv[i_retry] > tv[ranker.index_of(&logger).unwrap()]);

        // Unresolvable seeds degrade to the global vector (no panic).
        let junk = vec![TaskSeed { kind: "symbol".into(), id: "repo://r/symbol/nope/Nope".into(), weight: 1.0 }];
        let fallback = ranker.task_vector(&junk);
        let g = ranker.global_vector();
        assert_eq!(fallback, g);
        assert_eq!(fallback.len(), tv.len());
    }

    // ---- helper for SurfaceEntry fixtures ----

// trace:exempt reason=internal-detail
    fn surface_entry(symbol_id: &str, name: &str, path: &str, exported: bool) -> SurfaceEntry {
        let mut entry = SurfaceEntry {
            id: symbol_id.to_string(),
            symbol_id: symbol_id.to_string(),
            qualified_name: name.to_string(),
            kind: scc_core::SurfaceKind::Function,
            path: path.to_string(),
            range: SourceRange::new(path, 1, 1),
            source_signature: String::new(),
            canonical_signature: String::new(),
            semantic_signature: scc_core::SemanticSignature::default(),
            visibility: Visibility::Public,
            exported,
            modifiers: vec![],
            annotations: vec![],
            component: Some("Order".to_string()),
            subsystem: None,
            flows: vec![],
            contracts: vec![],
            state_authorities: vec![],
            invocation_surfaces: vec![],
            callers: vec![],
            callees: vec![],
            provenance: Provenance::Extracted,
            confidence: 1.0,
            rank: scc_core::SurfaceRank::default(),
        };
        if exported {
            entry.invocation_surfaces.push("http".into());
            entry.flows.push("f1".into());
            entry.contracts.push("c1".into());
        }
        entry
    }

    // ---- final_importance blend ----

    #[test]
// trace:exempt reason=internal-detail
    fn final_importance_blend_constants() {
        // All signals at 1.0 with task focus: 0.30+0.20+0.15+0.10+0.10+
        // 0.10+0.05+0.05 = 1.05 (novelty is additive).
        let v = final_importance(1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, true);
        assert!((v - 1.05).abs() < 1e-9);

        // Task focus weights task PPR over global.
        let tasky = final_importance(1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, true);
        assert!((tasky - 0.30).abs() < 1e-9);

        // No task: task share moves to global (50%).
        let globaly = final_importance(1.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, false);
        assert!((globaly - 0.25).abs() < 1e-9);

        // No task with full global: 0.50 + 0.15 + 0.10 + 0.10 + 0.10 + 0.05
        // + 0.05 = 1.05.
        let full = final_importance(1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, false);
        assert!((full - 1.05).abs() < 1e-9);
    }

    // ---- architectural_specificity penalties ----

    #[test]
// trace:exempt reason=internal-detail
    fn specificity_penalizes_generic_test_and_vendored() {
        let (_dir, store, graph) = fixture(vec![sym("r", "src/x.ts", "X")], vec![]);
        let view = TrustedGraphView::new(&graph, &store, &[], TrustPolicy::default());

        let util = surface_entry("s1", "logger", "src/utils/logger.ts", false);
        // Generic utility name/path → 0.5 (symbol "s1" is not in the graph,
        // so no ubiquity penalty; 0.5 is the single-penalty floor here).
        assert_eq!(architectural_specificity(&util, &view), 0.5);

        let testy = surface_entry("s2", "TestHelper", "tests/unit/helper_test.rs", false);
        assert_eq!(architectural_specificity(&testy, &view), 0.5);

        let vendored = surface_entry("s3", "Dep", "vendor/dep/src/lib.rs", false);
        assert_eq!(architectural_specificity(&vendored, &view), 0.5);

        let generated = surface_entry("s4", "Proto", "src/generated/models.rs", false);
        assert_eq!(architectural_specificity(&generated, &view), 0.5);

        // Plain exported facade with no penalties hits the cap.
        let facade = surface_entry("s5", "OrderApi", "src/api/order.ts", true);
        assert_eq!(architectural_specificity(&facade, &view), 1.25);
    }

    // ---- heterogeneous universe: non-symbol entities participate ----

    #[test]
// trace:exempt reason=internal-detail
    fn ranker_universe_is_heterogeneous() {
        let owner = scc_core::symbol_id("r", "src/a.ts", "A");
        let state_id = entity_id("r", kinds::STATE, "sessions");
        let contract_id = entity_id("r", kinds::CONTRACT, "c1");
        let topic_id = entity_id("r", kinds::TOPIC, "orders");
        let store_id = entity_id("r", kinds::DATA_STORE, "pg");
        let flow_id = entity_id("r", kinds::FLOW, "signup");
        let file_id = entity_id("r", kinds::FILE, "src/a.ts");

        let mut owner_ent = sym("r", "src/a.ts", "A");
        owner_ent.attr("exported", serde_json::json!(true));
        owner_ent.attr("entrypoints", serde_json::json!(["http"]));
        let entities = vec![
            owner_ent,
            entity(&state_id, kinds::STATE, "sessions"),
            entity(&contract_id, kinds::CONTRACT, "c1"),
            entity(&topic_id, kinds::TOPIC, "orders"),
            entity(&store_id, kinds::DATA_STORE, "pg"),
            entity(&flow_id, kinds::FLOW, "signup"),
            entity(&file_id, kinds::FILE, "src/a.ts"),
        ];
        let n = 1u64;
        let rels = vec![
            rel(n, &owner, predicates::OWNS, &state_id, Provenance::Extracted),
            rel(n + 1, &owner, predicates::REGISTERS, &contract_id, Provenance::Extracted),
            rel(n + 2, &owner, predicates::PUBLISHES, &topic_id, Provenance::Extracted),
            rel(n + 3, &owner, predicates::READS, &store_id, Provenance::Extracted),
            rel(n + 4, &owner, predicates::WRITES, &store_id, Provenance::Extracted),
            rel(n + 5, &owner, predicates::PARTICIPATES_IN, &flow_id, Provenance::Extracted),
        ];

        let (_dir, store, graph) = fixture(entities, rels);
        let view = TrustedGraphView::new(&graph, &store, &[], TrustPolicy::default());
        let ranker = SystemRanker::new(&view);

        // Every rankable entity kind is a node.
        for id in [
            owner.as_str(),
            state_id.as_str(),
            contract_id.as_str(),
            topic_id.as_str(),
            store_id.as_str(),
            flow_id.as_str(),
            file_id.as_str(),
        ] {
            assert!(ranker.index_of(id).is_some(), "{id} must be a node");
        }
        // Non-rankable kinds (module) are not nodes.
        assert!(ranker.index_of(&entity_id("r", kinds::MODULE, "m")).is_none());

        // The heterogeneous vectors are indexed over all nodes.
        let g = ranker.global_vector();
        assert_eq!(g.len(), ranker.nodes().len());

        // Non-symbol nodes carry real PPR mass (the edges feed them).
        let owner_i = ranker.index_of(&owner).unwrap();
        let state_i = ranker.index_of(&state_id).unwrap();
        assert!(g[state_i] > 0.0, "state node must receive PPR mass");
        assert!(g[owner_i] > 0.0);

        // Symbols() returns exactly the symbol nodes.
        let syms = ranker.symbols();
        assert_eq!(syms, vec![owner.clone()]);
    }

    #[test]
// trace:exempt reason=internal-detail
    fn project_to_symbols_lifts_owned_entity_scores() {
        let owner = scc_core::symbol_id("r", "src/a.ts", "A");
        let other = scc_core::symbol_id("r", "src/b.ts", "B");
        let state_id = entity_id("r", kinds::STATE, "sessions");
        let contract_id = entity_id("r", kinds::CONTRACT, "c1");

        let mut owner_ent = sym("r", "src/a.ts", "A");
        owner_ent.attr("exported", serde_json::json!(true));
        owner_ent.attr("entrypoints", serde_json::json!(["http"]));
        let entities = vec![
            owner_ent,
            sym("r", "src/b.ts", "B"),
            entity(&state_id, kinds::STATE, "sessions"),
            entity(&contract_id, kinds::CONTRACT, "c1"),
        ];
        let rels = vec![
            rel(1, &owner, predicates::OWNS, &state_id, Provenance::Extracted),
            rel(2, &owner, predicates::REGISTERS, &contract_id, Provenance::Extracted),
            // B's OWNS of the same state also projects (both owners lift)
            rel(3, &other, predicates::OWNS, &state_id, Provenance::Extracted),
        ];
        let (_dir, store, graph) = fixture(entities, rels);
        let view = TrustedGraphView::new(&graph, &store, &[], TrustPolicy::default());
        let ranker = SystemRanker::new(&view);

        let g = ranker.global_vector();
        let projected = ranker.project_to_symbols(&g);
        let score_of = |id: &str| -> f64 {
            projected
                .iter()
                .find(|(s, _)| s == id)
                .map(|(_, v)| *v)
                .unwrap_or(f64::NAN)
        };

        let own_a = score_of(&owner);
        let own_b = score_of(&other);
        let state_score = g[ranker.index_of(&state_id).unwrap()];
        let contract_score = g[ranker.index_of(&contract_id).unwrap()];

        // A owns state + registers contract → bonus = 0.4 × (state+contract),
        // capped at 0.5. B only owns the state → smaller bonus.
        let bonus_a = (0.4 * (state_score + contract_score)).min(0.5);
        let bonus_b = (0.4 * state_score).min(0.5);
        assert!((own_a - (g[ranker.index_of(&owner).unwrap()] + bonus_a)).abs() < 1e-9);
        assert!((own_b - (g[ranker.index_of(&other).unwrap()] + bonus_b)).abs() < 1e-9);
        // The owner with the richer entity set ranks above the plain owner.
        assert!(own_a > own_b);

        // Cap: a symbol owning a huge-massed state cannot exceed own + 0.5.
        let mut big_v = vec![0.0; g.len()];
        big_v[ranker.index_of(&state_id).unwrap()] = 100.0;
        big_v[ranker.index_of(&contract_id).unwrap()] = 100.0;
        let capped = ranker.project_to_symbols(&big_v);
        let cap_a = capped.iter().find(|(s, _)| s == &owner).unwrap().1;
        let own_a0 = big_v[ranker.index_of(&owner).unwrap()];
        assert!((cap_a - (own_a0 + 0.5)).abs() < 1e-9);

        // Deterministic: same input → identical output.
        let again = ranker.project_to_symbols(&g);
        assert_eq!(again, projected);
    }

    // ---- (e) rank-edge ontology: CONTAINS/PARTICIPATES_IN/HANDLES/DEFINES ----

    #[test]
// trace:exempt reason=internal-detail
    fn rank_edge_kind_direction_and_weights() {
        // Deliberate direction mapping (real predicate strings):
        // contains only between kinds in the containment hierarchy with
        // the container before the member (any ordered pair — adjacent or
        // transitive); participates_in only symbol ↔ flow; handles only
        // symbol ↔ route; defines only symbol ↔ schema.
        // Every reviewer-named hierarchy pair fires: System→Subsystem,
        // Subsystem→Service, Subsystem→Component, Service→Component,
        // Component→File, Component→Symbol, File→Symbol — plus every
        // transitive container pair — with the stored member → container
        // direction mapping to the MemberOf ranking transition.
        for (container, member) in [
            (kinds::SYSTEM, kinds::SUBSYSTEM),
            (kinds::SUBSYSTEM, kinds::SERVICE),
            (kinds::SUBSYSTEM, kinds::COMPONENT),
            (kinds::SERVICE, kinds::COMPONENT),
            (kinds::COMPONENT, kinds::FILE),
            (kinds::COMPONENT, kinds::SYMBOL),
            (kinds::FILE, kinds::SYMBOL),
            // Transitive container pairs (the generalization covers ANY
            // ordered pair, not a hardcoded adjacency set).
            (kinds::SYSTEM, kinds::SERVICE),
            (kinds::SYSTEM, kinds::FILE),
            (kinds::SUBSYSTEM, kinds::SYMBOL),
            (kinds::SERVICE, kinds::FILE),
        ] {
            assert_eq!(
                rank_edge_kind(predicates::CONTAINS, container, member),
                Some(RankEdgeKind::Contains)
            );
            assert_eq!(
                rank_edge_kind(predicates::CONTAINS, member, container),
                Some(RankEdgeKind::MemberOf)
            );
        }
        // Stored member → container direction maps to the ranking transition.
        assert_eq!(
            rank_edge_kind(predicates::CONTAINS, kinds::SYMBOL, kinds::COMPONENT),
            Some(RankEdgeKind::MemberOf)
        );
        assert_eq!(
            rank_edge_kind(predicates::PARTICIPATES_IN, kinds::SYMBOL, kinds::FLOW),
            Some(RankEdgeKind::ParticipatesIn)
        );
        assert_eq!(
            rank_edge_kind(predicates::PARTICIPATES_IN, kinds::FLOW, kinds::SYMBOL),
            Some(RankEdgeKind::FlowReachesParticipant)
        );
        assert_eq!(
            rank_edge_kind(predicates::HANDLES, kinds::SYMBOL, kinds::ROUTE),
            Some(RankEdgeKind::Handles)
        );
        assert_eq!(
            rank_edge_kind(predicates::DEFINES, kinds::SYMBOL, kinds::SCHEMA),
            Some(RankEdgeKind::Defines)
        );
        // Reverse transitions: stored route → symbol maps to HandledBy,
        // stored schema → symbol maps to DefinedBy (ranking-only).
        assert_eq!(
            rank_edge_kind(predicates::HANDLES, kinds::ROUTE, kinds::SYMBOL),
            Some(RankEdgeKind::HandledBy)
        );
        assert_eq!(
            rank_edge_kind(predicates::DEFINES, kinds::SCHEMA, kinds::SYMBOL),
            Some(RankEdgeKind::DefinedBy)
        );

        // Non-target endpoint kinds never fire (no `reference_kind` drift).
        assert_eq!(rank_edge_kind(predicates::CONTAINS, kinds::COMPONENT, kinds::FLOW), None);
        assert_eq!(rank_edge_kind(predicates::CONTAINS, kinds::SYMBOL, kinds::CONTRACT), None);
        assert_eq!(rank_edge_kind(predicates::CONTAINS, kinds::SYMBOL, kinds::FLOW), None);
        // Same-kind containment is not a hierarchy edge.
        assert_eq!(rank_edge_kind(predicates::CONTAINS, kinds::COMPONENT, kinds::COMPONENT), None);
        assert_eq!(rank_edge_kind(predicates::CONTAINS, kinds::SYMBOL, kinds::SYMBOL), None);
        assert_eq!(rank_edge_kind(predicates::PARTICIPATES_IN, kinds::SYMBOL, kinds::CONTRACT), None);
        assert_eq!(rank_edge_kind(predicates::HANDLES, kinds::SYMBOL, kinds::ENDPOINT), None);
        assert_eq!(rank_edge_kind(predicates::DEFINES, kinds::SYMBOL, kinds::CONTRACT), None);
        assert_eq!(rank_edge_kind(predicates::CALLS, kinds::SYMBOL, kinds::SYMBOL), None);

        // Deliberate weights.
        assert_eq!(RankEdgeKind::Contains.weight(), RANK_MEMBERSHIP);
        assert_eq!(RankEdgeKind::MemberOf.weight(), RANK_MEMBER_OF);
        assert_eq!(RankEdgeKind::ParticipatesIn.weight(), RANK_PARTICIPATES);
        assert_eq!(RankEdgeKind::FlowReachesParticipant.weight(), RANK_FLOW_REACHES_PARTICIPANT);
        assert_eq!(RankEdgeKind::Handles.weight(), RANK_HANDLES);
        assert_eq!(RankEdgeKind::HandledBy.weight(), RANK_HANDLED_BY);
        assert_eq!(RankEdgeKind::Defines.weight(), RANK_DEFINES);
        assert_eq!(RankEdgeKind::DefinedBy.weight(), RANK_DEFINED_BY);

        // The reverse edges are RANKING transitions (explicitly labeled,
        // never Reality Graph relationships): every evidence edge —
        // contains, participates_in, handles, defines — has a ranking-only
        // reverse that propagates container/flow/route/schema importance
        // back to members/participants/handlers/definers.
        assert_eq!(RankEdgeKind::Contains.reverse_ranking_transition(), Some(RankEdgeKind::MemberOf));
        assert_eq!(RankEdgeKind::MemberOf.reverse_ranking_transition(), Some(RankEdgeKind::Contains));
        assert_eq!(RankEdgeKind::ParticipatesIn.reverse_ranking_transition(), Some(RankEdgeKind::FlowReachesParticipant));
        assert_eq!(RankEdgeKind::FlowReachesParticipant.reverse_ranking_transition(), Some(RankEdgeKind::ParticipatesIn));
        assert_eq!(RankEdgeKind::Handles.reverse_ranking_transition(), Some(RankEdgeKind::HandledBy));
        assert_eq!(RankEdgeKind::HandledBy.reverse_ranking_transition(), Some(RankEdgeKind::Handles));
        assert_eq!(RankEdgeKind::Defines.reverse_ranking_transition(), Some(RankEdgeKind::DefinedBy));
        assert_eq!(RankEdgeKind::DefinedBy.reverse_ranking_transition(), Some(RankEdgeKind::Defines));
    }

    #[test]
// trace:exempt reason=internal-detail
    fn contains_edges_spread_component_importance_both_ways() {
        // A component with two members: the membership edges (comp → m1,
        // comp → m2) spread component importance DOWN to the members, and
        // the reverse RANKING TRANSITIONS (m1 → comp, m2 → comp) derive
        // the component's importance UP from its members. An isolated
        // symbol (x) sits at the dangling floor for contrast.
        let comp_id = entity_id("r", kinds::COMPONENT, "orders");
        let m1 = scc_core::symbol_id("r", "src/orders/a.ts", "OrderService");
        let m2 = scc_core::symbol_id("r", "src/orders/b.ts", "OrderRepo");
        let x = scc_core::symbol_id("r", "src/other.ts", "Unrelated");

        let entities = vec![
            entity(&comp_id, kinds::COMPONENT, "orders"),
            sym("r", "src/orders/a.ts", "OrderService"),
            sym("r", "src/orders/b.ts", "OrderRepo"),
            sym("r", "src/other.ts", "Unrelated"),
        ];
        let rels = vec![
            rel(1, &comp_id, predicates::CONTAINS, &m1, Provenance::Extracted),
            rel(2, &comp_id, predicates::CONTAINS, &m2, Provenance::Extracted),
        ];
        let (_dir, store, graph) = fixture(entities, rels);
        let view = TrustedGraphView::new(&graph, &store, &[], TrustPolicy::default());
        let ranker = SystemRanker::new(&view);
        let g = ranker.global_vector();

        let c = g[ranker.index_of(&comp_id).unwrap()];
        let s1 = g[ranker.index_of(&m1).unwrap()];
        let s2 = g[ranker.index_of(&m2).unwrap()];
        let sx = g[ranker.index_of(&x).unwrap()];

        // Both ways: the component ranks above its members (derived from
        // them via the ranking transitions — a dangling-only component
        // would sit at the uniform floor), and the members rank above the
        // isolated symbol (component importance reaches them via the
        // membership edges).
        assert!(c > 0.3, "component importance derived from members: {c}");
        assert!(s1 > sx && s2 > sx, "members lifted above the isolated symbol");
        assert!(c > s1 && c > s2, "component spreads importance down to members");
    }

    #[test]
// trace:exempt reason=internal-detail
    fn participates_in_connects_flow_nodes() {
        // A flow with a participant symbol becomes CONNECTED: the
        // symbol → flow edge feeds the flow node, and the reverse
        // flow → participant RANKING TRANSITION carries flow importance
        // back to the participant. A bare flow (no participants) stays at
        // the dangling floor.
        let s = scc_core::symbol_id("r", "src/a.ts", "A");
        let f1 = entity_id("r", kinds::FLOW, "signup");
        let f2 = entity_id("r", kinds::FLOW, "checkout");

        let entities = vec![
            sym("r", "src/a.ts", "A"),
            entity(&f1, kinds::FLOW, "signup"),
            entity(&f2, kinds::FLOW, "checkout"),
        ];
        let rels = vec![rel(1, &s, predicates::PARTICIPATES_IN, &f1, Provenance::Extracted)];
        let (_dir, store, graph) = fixture(entities, rels);
        let view = TrustedGraphView::new(&graph, &store, &[], TrustPolicy::default());
        let ranker = SystemRanker::new(&view);
        let g = ranker.global_vector();

        let f1v = g[ranker.index_of(&f1).unwrap()];
        let f2v = g[ranker.index_of(&f2).unwrap()];
        let sv = g[ranker.index_of(&s).unwrap()];

        // Previously the flow node was disconnected (no adjacency) and sat
        // at the dangling floor; with the rank edges it receives directed
        // mass from its participant.
        assert!(f1v > 0.3, "flow with a participant receives PPR mass: {f1v}");
        assert!(f1v > f2v, "connected flow ranks above the bare flow");
        // Flow importance reaches the participant (reverse transition).
        assert!(sv > f2v, "flow importance reaches the participant: {sv} vs {f2v}");
    }

    #[test]
// trace:exempt reason=internal-detail
    fn handles_and_defines_edges_feed_ppr() {
        // HANDLES (symbol → route) and DEFINES (symbol → schema) edges
        // feed PageRank: the route and schema nodes receive directed mass
        // from their handling/defining symbols, above the isolated
        // baseline. The handler is an invocation-surface seed (HANDLES →
        // ROUTE) and the definer is an exported symbol — exactly the
        // production shape — so both cycles receive inflow; the reverse
        // RANKING TRANSITIONS (route → handler, schema → definer) make
        // both pairs connected nodes rather than dangling sinks.
        let h = scc_core::symbol_id("r", "src/api.ts", "OrderHandler");
        let route = entity_id("r", kinds::ROUTE, "/orders");
        let d = scc_core::symbol_id("r", "src/models.ts", "Order");
        let schema = entity_id("r", kinds::SCHEMA, "order");
        let x = scc_core::symbol_id("r", "src/util.ts", "Helper");

        let mut d_ent = sym("r", "src/models.ts", "Order");
        d_ent.attr("exported", serde_json::json!(true));
        let entities = vec![
            sym("r", "src/api.ts", "OrderHandler"),
            entity(&route, kinds::ROUTE, "/orders"),
            d_ent,
            entity(&schema, kinds::SCHEMA, "order"),
            sym("r", "src/util.ts", "Helper"),
        ];
        let rels = vec![
            rel(1, &h, predicates::HANDLES, &route, Provenance::Extracted),
            rel(2, &d, predicates::DEFINES, &schema, Provenance::Extracted),
        ];
        let (_dir, store, graph) = fixture(entities, rels);
        let view = TrustedGraphView::new(&graph, &store, &[], TrustPolicy::default());
        let ranker = SystemRanker::new(&view);
        let g = ranker.global_vector();

        let rv = g[ranker.index_of(&route).unwrap()];
        let sv = g[ranker.index_of(&schema).unwrap()];
        let xv = g[ranker.index_of(&x).unwrap()];

        assert!(rv > xv, "route receives mass via the HANDLES edge: {rv} vs {xv}");
        assert!(sv > xv, "schema receives mass via the DEFINES edge: {sv} vs {xv}");
    }

    // ---- (f) full containment hierarchy: File→Symbol, Subsystem→Component ----

    #[test]
// trace:exempt reason=internal-detail
    fn file_contains_symbol_lifts_member() {
        // The reviewer's fracture: a FILE → SYMBOL containment pair was a
        // dead node — `contains` only fired between Component/Subsystem/
        // Service containers and their members, so a lone file (no
        // component) never connected its symbol. With the full hierarchy,
        // the file receives the symbol's importance (MemberOf) and the
        // symbol receives the file's (Contains): the member is lifted
        // above the isolated floor.
        let file_id = entity_id("r", kinds::FILE, "src/orders.ts");
        let sym_id = scc_core::symbol_id("r", "src/orders.ts", "OrderService");
        let x = scc_core::symbol_id("r", "src/other.ts", "Unrelated");

        let entities = vec![
            entity(&file_id, kinds::FILE, "src/orders.ts"),
            sym("r", "src/orders.ts", "OrderService"),
            sym("r", "src/other.ts", "Unrelated"),
        ];
        let rels = vec![rel(1, &file_id, predicates::CONTAINS, &sym_id, Provenance::Extracted)];
        let (_dir, store, graph) = fixture(entities, rels);
        let view = TrustedGraphView::new(&graph, &store, &[], TrustPolicy::default());
        let ranker = SystemRanker::new(&view);
        let g = ranker.global_vector();

        let fv = g[ranker.index_of(&file_id).unwrap()];
        let sv = g[ranker.index_of(&sym_id).unwrap()];
        let xv = g[ranker.index_of(&x).unwrap()];

        // The file is no longer a dead node (it derives mass from its
        // symbol via the MemberOf ranking transition) and the contained
        // symbol ranks above the isolated symbol.
        assert!(fv > 0.3, "file node derives importance from its symbol: {fv}");
        assert!(sv > xv, "contained symbol lifted above the isolated one: {sv} vs {xv}");
        assert!(fv > xv, "file lifted above the isolated symbol: {fv} vs {xv}");
    }

    #[test]
// trace:exempt reason=internal-detail
    fn subsystem_component_containment_spreads_both_ways() {
        // A Subsystem CONTAINS a Component: membership evidence flows
        // subsystem → component (down) and the reverse RANKING TRANSITION
        // component → subsystem derives the subsystem's importance from
        // its component (up). A bare component with no subsystem sits at
        // the dangling floor.
        let sub_id = entity_id("r", kinds::SUBSYSTEM, "billing");
        let comp_id = entity_id("r", kinds::COMPONENT, "orders");
        let bare = entity_id("r", kinds::COMPONENT, "auth");

        let entities = vec![
            entity(&sub_id, kinds::SUBSYSTEM, "billing"),
            entity(&comp_id, kinds::COMPONENT, "orders"),
            entity(&bare, kinds::COMPONENT, "auth"),
        ];
        let rels = vec![rel(1, &sub_id, predicates::CONTAINS, &comp_id, Provenance::Extracted)];
        let (_dir, store, graph) = fixture(entities, rels);
        let view = TrustedGraphView::new(&graph, &store, &[], TrustPolicy::default());
        let ranker = SystemRanker::new(&view);
        let g = ranker.global_vector();

        let subv = g[ranker.index_of(&sub_id).unwrap()];
        let compv = g[ranker.index_of(&comp_id).unwrap()];
        let barev = g[ranker.index_of(&bare).unwrap()];

        // Both ways: the contained component receives the subsystem's
        // membership evidence (compv > barev) and the subsystem derives
        // importance from its component (subv > barev).
        assert!(compv > barev, "contained component ranks above the bare one: {compv} vs {barev}");
        assert!(subv > barev, "subsystem derives importance from its component: {subv} vs {barev}");
        assert!(subv > 0.3, "subsystem is a first-class rank node: {subv}");
    }

    // ---- (g) projection through HandledBy/DefinedBy ----

    #[test]
// trace:exempt reason=internal-detail
    fn hot_route_projects_to_handler_symbol() {
        // The reviewer's exact scenario: the task mentions /health → the
        // route node is hot (task seed) → its handler symbol is reached.
        // Two mechanisms fire: the HandledBy ranking transition carries
        // route mass to the handler in the PPR vector, and the HANDLES
        // projection adds 0.4 × the hot route's score (capped at 0.5) to
        // the handler's surface score.
        let handler = scc_core::symbol_id("r", "src/router.ts", "build_router");
        let route = entity_id("r", kinds::ROUTE, "/health");
        let other = scc_core::symbol_id("r", "src/util.ts", "Helper");

        let entities = vec![
            sym("r", "src/router.ts", "build_router"),
            entity(&route, kinds::ROUTE, "/health"),
            sym("r", "src/util.ts", "Helper"),
        ];
        let rels = vec![rel(1, &handler, predicates::HANDLES, &route, Provenance::Extracted)];
        let (_dir, store, graph) = fixture(entities, rels);
        let view = TrustedGraphView::new(&graph, &store, &[], TrustPolicy::default());
        let ranker = SystemRanker::new(&view);

        // Hot route: the task seed lands on the route node itself.
        let seeds = vec![TaskSeed {
            kind: "route".into(),
            id: route.clone(),
            weight: 1.0,
        }];
        let tv = ranker.task_vector(&seeds);
        let projected = ranker.project_to_symbols(&tv);
        let score_of = |id: &str| -> f64 {
            projected
                .iter()
                .find(|(s, _)| s == id)
                .map(|(_, v)| *v)
                .unwrap_or(f64::NAN)
        };

        let route_mass = tv[ranker.index_of(&route).unwrap()];
        let handler_own = tv[ranker.index_of(&handler).unwrap()];
        let handler_score = score_of(&handler);
        let other_score = score_of(&other);

        // The handler's surface score = its own PPR (which already grew
        // via the HandledBy transition) + 0.4 × the hot route's score,
        // capped at 0.5 — the exact projection pattern.
        let expected_bonus = (PROJECTION_BONUS_FACTOR * route_mass).min(PROJECTION_BONUS_CAP);
        assert!((handler_score - (handler_own + expected_bonus)).abs() < 1e-9);
        assert!(route_mass > 0.1, "seeded route is hot: {route_mass}");
        assert!(handler_score > other_score, "hot route lifts its handler above unrelated symbols");
    }

    #[test]
// trace:exempt reason=internal-detail
    fn hot_schema_projects_to_definer_symbol() {
        // A hot Schema node (task seed) reaches its defining symbol: the
        // DefinedBy ranking transition feeds the definer in the PPR vector
        // and the DEFINES projection adds 0.4 × the schema's score (capped
        // at 0.5) to the definer's surface score.
        let definer = scc_core::symbol_id("r", "src/models.ts", "Order");
        let schema = entity_id("r", kinds::SCHEMA, "order");
        let other = scc_core::symbol_id("r", "src/util.ts", "Helper");

        let entities = vec![
            sym("r", "src/models.ts", "Order"),
            entity(&schema, kinds::SCHEMA, "order"),
            sym("r", "src/util.ts", "Helper"),
        ];
        let rels = vec![rel(1, &definer, predicates::DEFINES, &schema, Provenance::Extracted)];
        let (_dir, store, graph) = fixture(entities, rels);
        let view = TrustedGraphView::new(&graph, &store, &[], TrustPolicy::default());
        let ranker = SystemRanker::new(&view);

        let seeds = vec![TaskSeed {
            kind: "schema".into(),
            id: schema.clone(),
            weight: 1.0,
        }];
        let tv = ranker.task_vector(&seeds);
        let projected = ranker.project_to_symbols(&tv);
        let score_of = |id: &str| -> f64 {
            projected
                .iter()
                .find(|(s, _)| s == id)
                .map(|(_, v)| *v)
                .unwrap_or(f64::NAN)
        };

        let schema_mass = tv[ranker.index_of(&schema).unwrap()];
        let definer_own = tv[ranker.index_of(&definer).unwrap()];
        let definer_score = score_of(&definer);
        let other_score = score_of(&other);

        let expected_bonus = (PROJECTION_BONUS_FACTOR * schema_mass).min(PROJECTION_BONUS_CAP);
        assert!((definer_score - (definer_own + expected_bonus)).abs() < 1e-9);
        assert!(schema_mass > 0.1, "seeded schema is hot: {schema_mass}");
        assert!(definer_score > other_score, "hot schema lifts its definer above unrelated symbols");
    }
}
