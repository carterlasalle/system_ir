//! Flow Compiler (EPIC-050, docs/FLOW_COMPILER.md).
//!
//! Compiles machine-readable Sequence and Data Flow views from entrypoints,
//! plus one Architecture view per repository.
//!
//! Traversal: BFS through RESOLVED call edges, capped in depth and breadth,
//! recording store/topic/external access; then abstracted to
//! component-level steps (collapse consecutive same-actor hops).

use crate::components::{component_for_path, ComponentCandidate, prov_rank};
use crate::{RealityGraph, Result};
use scc_core::kinds;
use scc_core::{
    entity_id, Flow, FlowKind, FlowStep, InvocationSurface, InvocationSurfaceKind, Provenance,
    Relationship,
};
use scc_store::Store;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

const MAX_DEPTH: usize = 10;
const MAX_BREADTH: usize = 64;

pub struct FlowEntrypoint {
    pub name: String,
    pub trigger: String,
    pub symbol_id: String,
    pub kind: String, // "route" | "entrypoint" | "intent" | surface kinds
}

/// JUnit lifecycle annotation names (java extractor emits these as
/// ANNOTATION facts; the annotated method is a Lifecycle invocation
/// surface).
const LIFECYCLE_ANNOTATIONS: [&str; 8] = [
    "Before",
    "After",
    "BeforeClass",
    "AfterClass",
    "BeforeAll",
    "AfterAll",
    "BeforeEach",
    "AfterEach",
];

/// Last path segment of an entity id — the display-name fallback when the
/// referenced entity does not exist in the graph (matches `name_of`).
fn last_segment(id: &str) -> String {
    id.rsplit('/').next().unwrap_or(id).to_string()
}

/// Seed invocation surfaces from the Wave 9 semantic-fact layer
/// (deterministic — sorted by (kind, symbol, trigger)):
/// - public exports: `symbol EXPORTS export` relationships → PublicApi
/// - exported module-level symbols + methods of exported classes (the
///   `exported: true` fact attribute / class-parent attribution) → PublicApi
/// - queue consumers: `symbol SUBSCRIBES topic` relationships → Queue
/// - framework callbacks: `owner HANDLES_CALLBACK callback` → FrameworkCallback
/// - lifecycle callbacks: JUnit @Before*/@After* annotation facts → Lifecycle
/// - event handlers: `symbol CONSUMES|PUBLISHES topic` relationships → Event
pub fn invocation_surfaces(graph: &RealityGraph) -> Vec<InvocationSurface> {
    let mut out: Vec<InvocationSurface> = Vec::new();
    // dedup key: (kind, symbol id) — keep the first occurrence after a
    // deterministic sort so multi-trigger symbols pick a stable trigger
    let mut seen: HashSet<(InvocationSurfaceKind, String)> = HashSet::new();

    // relationships processed in a deterministic order
    let mut rels: Vec<&Relationship> = graph.all_rels();
    rels.sort_by(|a, b| {
        a.subject
            .cmp(&b.subject)
            .then(a.predicate.cmp(&b.predicate))
            .then(a.object.cmp(&b.object))
    });

    // public exports → PublicApi
    for r in rels.iter().copied() {
        if r.predicate != scc_core::predicates::EXPORTS {
            continue;
        }
        let Some(sym) = graph.entity(&r.subject) else { continue };
        if sym.name.is_empty() {
            continue;
        }
        let kind_attr = graph
            .entity(&r.object)
            .and_then(|e| e.attributes.get("kind"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let key = (InvocationSurfaceKind::PublicApi, r.subject.clone());
        if seen.insert(key.clone()) {
            out.push(InvocationSurface {
                symbol: r.subject.clone(),
                kind: InvocationSurfaceKind::PublicApi,
                trigger: if kind_attr.is_empty() {
                    format!("export:{}", sym.name)
                } else {
                    format!("export:{} ({kind_attr})", sym.name)
                },
            });
        }
    }

    // exported module-level symbols + methods of exported classes → PublicApi.
    // The extractors record visibility statically (`exported: true`) and
    // attribute methods to their parent class; a method of an exported class
    // is part of the class's public surface. Deterministic: symbols are
    // iterated by id.
    let exported_classes = exported_class_names(graph);
    let mut exported_syms: Vec<&scc_core::Entity> = graph
        .entities_of_kind(kinds::SYMBOL)
        .into_iter()
        .filter(|e| {
            let name = e.name.as_str();
            if name.is_empty() || name.starts_with('_') {
                return false;
            }
            let exported = e
                .attributes
                .get("exported")
                .and_then(|v| v.as_bool())
                == Some(true);
            if exported {
                return !name.contains('.');
            }
            // method of an exported class
            let parent = e
                .attributes
                .get("parent")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            !parent.is_empty()
                && exported_classes.contains(parent)
                && e.attributes.get("kind").and_then(|v| v.as_str()) == Some("method")
        })
        .collect();
    exported_syms.sort_by(|a, b| a.name.cmp(&b.name));
    for e in exported_syms {
        let key = (InvocationSurfaceKind::PublicApi, e.id.clone());
        if seen.insert(key.clone()) {
            let kind = e
                .attributes
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            out.push(InvocationSurface {
                symbol: e.id.clone(),
                kind: InvocationSurfaceKind::PublicApi,
                trigger: if kind.is_empty() {
                    format!("export:{}", e.name)
                } else {
                    format!("export:{} ({kind})", e.name)
                },
            });
        }
    }

    // queue consumers → Queue
    for r in rels.iter().copied() {
        if r.predicate != scc_core::predicates::SUBSCRIBES {
            continue;
        }
        if graph.entity(&r.subject).is_none() {
            continue;
        }
        let target = graph
            .entity(&r.object)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| last_segment(&r.object));
        let key = (InvocationSurfaceKind::Queue, r.subject.clone());
        if seen.insert(key.clone()) {
            out.push(InvocationSurface {
                symbol: r.subject.clone(),
                kind: InvocationSurfaceKind::Queue,
                trigger: format!("subscribe:{target}"),
            });
        }
    }

    // framework callbacks → FrameworkCallback
    for r in rels.iter().copied() {
        if r.predicate != scc_core::predicates::HANDLES_CALLBACK {
            continue;
        }
        if graph.entity(&r.subject).is_none() {
            continue;
        }
        let cb = graph
            .entity(&r.object)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| last_segment(&r.object));
        let key = (InvocationSurfaceKind::FrameworkCallback, r.subject.clone());
        if seen.insert(key.clone()) {
            out.push(InvocationSurface {
                symbol: r.subject.clone(),
                kind: InvocationSurfaceKind::FrameworkCallback,
                trigger: format!("callback:{cb}"),
            });
        }
    }

    // lifecycle callbacks (JUnit @Before*/@After* annotation facts) →
    // Lifecycle: the annotation entity names the hook, the ANNOTATES
    // target is the lifecycle method.
    for a in graph.entities_of_kind(kinds::ANNOTATION) {
        if !LIFECYCLE_ANNOTATIONS.contains(&a.name.as_str()) {
            continue;
        }
        for r in graph.out_pred(&a.id, scc_core::predicates::ANNOTATES) {
            let key = (InvocationSurfaceKind::Lifecycle, r.object.clone());
            if seen.insert(key.clone()) {
                out.push(InvocationSurface {
                    symbol: r.object.clone(),
                    kind: InvocationSurfaceKind::Lifecycle,
                    trigger: format!("lifecycle:{}", a.name),
                });
            }
        }
    }

    // event handlers → Event (topics with CONSUMES/PUBLISHES edges)
    for t in graph.entities_of_kind(kinds::TOPIC) {
        let mut handlers: BTreeMap<String, String> = BTreeMap::new(); // symbol -> trigger
        for pred in [
            scc_core::predicates::CONSUMES,
            scc_core::predicates::PUBLISHES,
        ] {
            for r in graph.in_pred(&t.id, pred) {
                handlers.entry(r.subject.clone()).or_insert_with(|| format!("event:{}", t.name));
            }
        }
        let mut handlers: Vec<(String, String)> = handlers.into_iter().collect();
        handlers.sort();
        for (sym, trigger) in handlers {
            let key = (InvocationSurfaceKind::Event, sym.clone());
            if seen.insert(key.clone()) {
                out.push(InvocationSurface {
                    symbol: sym,
                    kind: InvocationSurfaceKind::Event,
                    trigger,
                });
            }
        }
    }

    out.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then(a.symbol.cmp(&b.symbol))
            .then(a.trigger.cmp(&b.trigger))
    });
    out
}

/// Module-level symbol kinds that count as an exported *class* (a method
/// parent attribution target). The extractor's `exported: true` fact on a
/// top-level class/struct/trait/interface/type marks the class as public API.
const CLASS_KINDS: [&str; 13] = [
    "class",
    "struct",
    "trait",
    "interface",
    "enum",
    "type",
    "module",
    "protocol",
    "dataclass",
    "object",
    "decorator",
    "exception",
    "model",
];

/// Names of exported classes (deterministic: sorted by id over symbol
/// entities). Methods of these classes are public-api surfaces.
fn exported_class_names(graph: &RealityGraph) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    for e in graph.entities_of_kind(kinds::SYMBOL) {
        let name = e.name.as_str();
        if name.is_empty() || name.contains('.') {
            continue;
        }
        let kind = e
            .attributes
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !CLASS_KINDS.contains(&kind) {
            continue;
        }
        if e.attributes.get("exported").and_then(|v| v.as_bool()) == Some(true) {
            out.insert(name.to_string());
        }
    }
    out
}

/// Public-api flow seeds are budgeted in two deterministic classes so the
/// FLOWS view stays compact (the full surface list still reaches the atlas
/// entrypoints) while the budget spends itself on behavior:
///
/// - **chain surfaces** seed first: an exported symbol with outgoing
///   EXTRACTED/RESOLVED CALLS edges is a behavior-flow candidate. Within
///   the class the order is by resolved-chain length (deepest first), then
///   symbol id — NOT alphabetical file order, which let benchmark/script/
///   error-constant noise crowd out the library's real API in chain-rich
///   repos (e.g. a framework repo with hundreds of exported symbols).
/// - **leaf surfaces** (single-step exports) fill the remainder.
const PUBLIC_API_CHAIN_SEED_CAP: usize = 48;
const PUBLIC_API_LEAF_SEED_CAP: usize = 16;

/// Length of the longest call chain `walk_calls` reaches from `sym` (node
/// count; 1 when the symbol has no evidence-grade call edges). Cheap for
/// leaves (no edges -> immediate return).
fn chain_length(graph: &RealityGraph, sym: &str) -> usize {
    walk_calls(graph, sym)
        .into_iter()
        .map(|p| p.len())
        .max()
        .unwrap_or(1)
}

/// Collect entrypoints: routes (handlers), declared intent flows, symbols
/// marked as entrypoints (main-guard / bin / module-entry).
pub fn collect_entrypoints(
    graph: &RealityGraph,
    store: &Store,
    intent: &[(String, serde_json::Value)],
) -> Vec<FlowEntrypoint> {
    let mut out: Vec<FlowEntrypoint> = Vec::new();
    // Flow ids key on the entrypoint name via `entity_id`, which sanitizes
    // names to lowercase; names that differ only by case (`main` / `Main`)
    // collide on the store's UNIQUE flows.id. Dedup on the canonical flow
    // id so the id constraint is the invariant, never the raw spelling.
    let mut seen: HashSet<String> = HashSet::new();
    let flow_key = |name: &str| entity_id(&store.repo_id, kinds::FLOW, name);

    // routes
    for route in graph.entities_of_kind(kinds::ROUTE) {
        if let Some(handler) = route.attributes.get("handler").and_then(|v| v.as_str()) {
            let method = route.attributes.get("method").and_then(|v| v.as_str()).unwrap_or("");
            let path = route.attributes.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let name = format!("{}-{}", method.to_ascii_lowercase(), path);
            if seen.insert(flow_key(&name)) {
                out.push(FlowEntrypoint {
                    name,
                    trigger: format!("{method} {path}"),
                    symbol_id: handler.to_string(),
                    kind: "route".into(),
                });
            }
        }
    }

    // intent flows
    for (source, claim) in intent {
        if source == "flow" {
            let name = claim["name"].as_str().unwrap_or("").to_string();
            let entrypoint = claim["entrypoint"].as_str().unwrap_or("").to_string();
            if name.is_empty() || entrypoint.is_empty() {
                continue;
            }
            // find symbol by name anywhere in the repo
            let symbol_id = find_symbol_by_name(graph, &entrypoint);
            let trigger = claim
                .get("trigger")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| entrypoint.clone());
            if seen.insert(flow_key(&name)) {
                out.push(FlowEntrypoint {
                    name,
                    trigger,
                    symbol_id: symbol_id.unwrap_or_default(),
                    kind: "intent".into(),
                });
            }
        }
    }

    // symbols with entrypoint attributes
    for e in graph.entities_of_kind(kinds::SYMBOL) {
        let has_ep = e.attributes.contains_key("entrypoints");
        if !has_ep {
            continue;
        }
        if seen.insert(flow_key(&e.name)) {
            out.push(FlowEntrypoint {
                name: e.name.clone(),
                trigger: format!("entrypoint:{}", e.name),
                symbol_id: e.id.clone(),
                kind: "entrypoint".into(),
            });
        }
    }

    // Wave 9/10: invocation-surface seeds (public exports, exported-module
    // and exported-class-method surfaces, queue consumers, framework
    // callbacks, lifecycle callbacks, event handlers) — additive and
    // deterministic. Name-dedup keeps flow ids unique (a symbol already
    // seeded as an entrypoint does not seed a second flow).
    //
    // The public-api bulk (every exported symbol in a framework repo) is
    // bounded by class so the FLOWS view stays compact; the full surface
    // list still reaches the atlas entrypoints. Chain surfaces (exports
    // whose walk reaches real callees) seed FIRST — the behavior flows —
    // ordered by chain length descending, then leaf exports; this keeps
    // the cap from being spent on alphabetical file-order noise
    // (benchmarks, scripts, error constants) at the expense of the
    // library's real API. Non-public-api surfaces (queue consumers,
    // framework/lifecycle callbacks, event handlers) are naturally few and
    // seed uncapped in invocation_surfaces' deterministic order.
    let mut surfaces = invocation_surfaces(graph);
    let mut public_api: Vec<InvocationSurface> = Vec::new();
    let mut others: Vec<InvocationSurface> = Vec::new();
    for s in surfaces.drain(..) {
        if s.kind == InvocationSurfaceKind::PublicApi {
            public_api.push(s);
        } else {
            others.push(s);
        }
    }
    let mut chained: Vec<(usize, InvocationSurface)> = Vec::new();
    let mut leaves: Vec<InvocationSurface> = Vec::new();
    for s in public_api {
        let len = chain_length(graph, &s.symbol);
        if len >= 2 {
            chained.push((len, s));
        } else {
            leaves.push(s);
        }
    }
    chained.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(a.1.symbol.cmp(&b.1.symbol))
            .then(a.1.trigger.cmp(&b.1.trigger))
    });
    leaves.sort_by(|a, b| a.symbol.cmp(&b.symbol).then(a.trigger.cmp(&b.trigger)));
    let mut seed_public = |s: &InvocationSurface, out: &mut Vec<FlowEntrypoint>| {
        let Some(e) = graph.entity(&s.symbol) else { return };
        if e.name.is_empty() {
            return;
        }
        if seen.insert(flow_key(&e.name)) {
            out.push(FlowEntrypoint {
                name: e.name.clone(),
                trigger: s.trigger.clone(),
                symbol_id: s.symbol.clone(),
                kind: s.kind.as_str().to_string(),
            });
        }
    };
    let mut chain_seeded = 0usize;
    for (_, s) in chained {
        if chain_seeded >= PUBLIC_API_CHAIN_SEED_CAP {
            break;
        }
        let before = out.len();
        seed_public(&s, &mut out);
        if out.len() > before {
            chain_seeded += 1;
        }
    }
    let mut leaf_seeded = 0usize;
    for s in leaves {
        if leaf_seeded >= PUBLIC_API_LEAF_SEED_CAP {
            break;
        }
        let before = out.len();
        seed_public(&s, &mut out);
        if out.len() > before {
            leaf_seeded += 1;
        }
    }
    for s in others {
        seed_public(&s, &mut out);
    }
    let _ = store;
    out
}

pub fn find_symbol_by_name(graph: &RealityGraph, name: &str) -> Option<String> {
    graph
        .entities_of_kind(kinds::SYMBOL)
        .into_iter()
        .find(|e| e.name == name)
        .map(|e| e.id.clone())
}

/// Walk resolved call edges from an entry symbol, returning all reachable
/// call paths (each capped by MAX_DEPTH).
pub(crate) fn walk_calls(
    graph: &RealityGraph,
    entry: &str,
) -> Vec<Vec<String>> {
    let mut paths: Vec<Vec<String>> = Vec::new();
    let mut queue: VecDeque<(String, Vec<String>)> = VecDeque::new();
    queue.push_back((entry.to_string(), vec![entry.to_string()]));
    let mut visited: HashMap<String, usize> = HashMap::new();
    let mut breadth = 0;
    while let Some((sym, path)) = queue.pop_front() {
        breadth += 1;
        if breadth > MAX_BREADTH {
            break;
        }
        if path.len() > MAX_DEPTH {
            paths.push(path);
            continue;
        }
        let call_targets: Vec<String> = graph
            .out_pred(&sym, scc_core::predicates::CALLS)
            .into_iter()
            // evidence-grade edges only: EXTRACTED (native candidates) and
            // RESOLVED (LSP/SCIP proof) — never INFERRED/STALE
            .filter(|r| matches!(r.provenance, Provenance::Extracted | Provenance::Resolved))
            .map(|r| r.object.clone())
            .collect();
        if call_targets.is_empty() {
            paths.push(path);
            continue;
        }
        let mut any_new = false;
        for t in &call_targets {
            let seen_at = visited.get(t).copied().unwrap_or(usize::MAX);
            if seen_at <= path.len() {
                continue; // cycle or already explored closer
            }
            visited.insert(t.clone(), path.len());
            any_new = true;
            let mut np = path.clone();
            np.push(t.clone());
            queue.push_back((t.clone(), np));
        }
        if !any_new {
            paths.push(path);
        }
    }
    paths
}

pub fn compile_flows(
    graph: &RealityGraph,
    store: &Store,
    intent: &[(String, serde_json::Value)],
) -> Result<(Vec<Flow>, Vec<Flow>, Option<Flow>)> {
    // component candidates for path mapping (same rules as component compiler)
    let mut candidates: Vec<ComponentCandidate> = Vec::new();
    for (source, claim) in intent {
        if source == "component" {
            let name = claim["name"].as_str().unwrap_or("").to_string();
            let mut dirs: Vec<String> = Vec::new();
            if let Some(paths) = claim["paths"].as_array() {
                for p in paths {
                    if let Some(s) = p.as_str() {
                        dirs.push(s.to_string());
                    }
                }
            }
            dirs.push(name.clone());
            candidates.push(ComponentCandidate {
                name,
                dirs,
                boundary_kind: crate::components::BOUNDARY_DECLARED.to_string(),
            });
        }
    }
    let mut top_dirs: HashSet<String> = HashSet::new();
    for f in graph.entities_of_kind(kinds::FILE) {
        if f.name.contains('/') {
            if let Some(seg) = f.name.split('/').next() {
                if seg != ".scc" {
                    top_dirs.insert(seg.to_string());
                }
            }
        }
    }
    top_dirs.insert("root".to_string());
    for d in &top_dirs {
        if !candidates.iter().any(|c| c.name == *d) {
            candidates.push(ComponentCandidate {
                name: d.clone(),
                dirs: vec![d.clone()],
                boundary_kind: if d == "root" {
                    crate::components::BOUNDARY_ROOT.to_string()
                } else {
                    crate::components::BOUNDARY_CODE_REGION.to_string()
                },
            });
        }
    }

    // symbol -> component entity id
    let mut symbol_comp: HashMap<String, String> = HashMap::new();
    let comp_name_to_id: HashMap<String, String> = graph
        .entities_of_kind(kinds::COMPONENT)
        .into_iter()
        .map(|c| (c.name.clone(), c.id.clone()))
        .collect();
    for e in graph.entities_of_kind(kinds::SYMBOL) {
        if let Some(file) = e.attributes.get("file").and_then(|v| v.as_str()) {
            let comp = component_for_path(file, &candidates);
            let cid = comp_name_to_id
                .get(&comp)
                .cloned()
                .unwrap_or_else(|| format!("component:{comp}"));
            symbol_comp.insert(e.id.clone(), cid);
        }
    }

    // store/topic access per symbol
    let mut store_access: HashMap<String, Vec<(String, String, Provenance)>> = HashMap::new();
    for r in graph.all_rels() {
        if matches!(
            r.predicate.as_str(),
            "reads" | "writes" | "queries" | "publishes" | "subscribes"
        ) {
            store_access
                .entry(r.subject.clone())
                .or_default()
                .push((r.predicate.clone(), r.object.clone(), r.provenance));
        }
    }

    let entrypoints = collect_entrypoints(graph, store, intent);
    let mut sequences: Vec<Flow> = Vec::new();
    let mut dataflows: Vec<Flow> = Vec::new();

    for ep in entrypoints {
        if ep.symbol_id.is_empty() {
            // declared entrypoint missing → drift handled elsewhere
            continue;
        }
        let paths = walk_calls(graph, &ep.symbol_id);
        // merge all paths into one ordered step list: keep a canonical
        // traversal order = entry first, then order of first appearance
        let mut step_evidence: BTreeMap<(String, String), (Provenance, Vec<String>)> = BTreeMap::new();
        let step_async: HashSet<String> = HashSet::new();
        let mut step_retries: HashMap<String, String> = HashMap::new();
        let mut store_steps: Vec<(String, String, Provenance)> = Vec::new();

        let push_step = |actor: &str, op: &str, prov: Provenance, ev_ids: Vec<String>,
                             steps: &mut Vec<(String, String)>,
                             seen: &mut HashMap<(String, String), usize>,
                             meta: &mut BTreeMap<(String, String), (Provenance, Vec<String>)>| {
            let key = (actor.to_string(), op.to_string());
            if let Some(idx) = seen.get(&key) {
                let existing = meta.get_mut(&key).unwrap();
                if prov_rank(prov) > prov_rank(existing.0) {
                    existing.0 = prov;
                }
                for e in ev_ids {
                    if !existing.1.contains(&e) {
                        existing.1.push(e);
                    }
                }
                let _ = idx;
                return;
            }
            seen.insert(key.clone(), steps.len());
            meta.insert(key.clone(), (prov, ev_ids));
            steps.push(key);
        };

        let mut seen_steps: HashMap<(String, String), usize> = HashMap::new();
        let mut steps: Vec<(String, String)> = Vec::new();

        // entry step
        let entry_actor = symbol_comp
            .get(&ep.symbol_id)
            .cloned()
            .unwrap_or_else(|| "component:root".into());
        let entry_op = graph
            .entities
            .get(&ep.symbol_id)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| ep.name.clone());
        push_step(
            &entry_actor,
            &entry_op,
            Provenance::Resolved,
            Vec::new(),
            &mut steps,
            &mut seen_steps,
            &mut step_evidence,
        );

        for path in &paths {
            for (i, sym) in path.iter().enumerate() {
                if i == 0 {
                    continue;
                }
                let prev = &path[i - 1];
                let rel = graph
                    .out_pred(prev, scc_core::predicates::CALLS)
                    .into_iter()
                    .find(|r| r.object == *sym)
                    .cloned();
                let prov = rel
                    .as_ref()
                    .map(|r| r.provenance)
                    .unwrap_or(Provenance::Inferred);
                let ev_ids = rel.as_ref().map(|r| r.evidence.clone()).unwrap_or_default();
                let actor = symbol_comp
                    .get(sym)
                    .cloned()
                    .unwrap_or_else(|| "component:root".into());
                let op = graph
                    .entities
                    .get(sym)
                    .map(|e| e.name.clone())
                    .unwrap_or_else(|| sym.clone());
                push_step(
                    &actor,
                    &op,
                    prov,
                    ev_ids,
                    &mut steps,
                    &mut seen_steps,
                    &mut step_evidence,
                );
                // retry policy on the callee
                if let Some(e) = graph.entities.get(sym) {
                    if let Some(rp) = e.attributes.get("retry_policy").and_then(|v| v.as_str()) {
                        step_retries.insert(op.clone(), rp.to_string());
                    }
                }
                // store/topic access by callee
                if let Some(accesses) = store_access.get(sym) {
                    for (pred, obj, sprov) in accesses {
                        if let Some(target) = graph.entities.get(obj) {
                            store_steps.push((target.name.clone(), pred.clone(), *sprov));
                        }
                    }
                }
            }
        }

        // Each operation is its own step (P1 §20): collapsing consecutive
        // same-actor operations into a comma-joined string destroyed the
        // per-operation evidence keys and produced text that looked like
        // branching. Canonical per-operation steps keep provenance and
        // evidence attached.
        let mut fsteps: Vec<FlowStep> = Vec::new();
        for (i, (actor, op)) in steps.iter().enumerate() {
            let meta = step_evidence.get(&(actor.clone(), op.clone()));
            let prov = meta.map(|(p, _)| *p);
            let ev = meta.map(|(_, e)| e.clone()).unwrap_or_default();
            let mut fs = FlowStep {
                id: format!("step:{}", i + 1),
                order: (i + 1) as u32,
                actor: actor.clone(),
                operation: op.clone(),
                condition: None,
                r#async: if step_async.contains(op) { Some(true) } else { None },
                timeout_ms: None,
                retry_policy: step_retries.get(op).cloned(),
                failure_outcome: None,
                provenance: prov,
                evidence: ev,
            };
            // store steps attached as sub-ops in the operation string
            let related: Vec<String> = store_steps
                .iter()
                .filter(|(n, _, _)| op.contains(n) || n.is_empty())
                .map(|(n, p, _)| format!("{n}:{p}"))
                .collect();
            if !related.is_empty() {
                fs.condition = Some(format!("stores: {}", related.join(", ")));
            }
            fsteps.push(fs);
        }

        let flow_id = entity_id(&store.repo_id, kinds::FLOW, &ep.name);
        let mut attrs: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        attrs.insert("entrypoint".into(), json!(ep.symbol_id));
        attrs.insert("kind".into(), json!(ep.kind));
        let seq = Flow {
            id: flow_id.clone(),
            kind: FlowKind::Sequence,
            name: ep.name.clone(),
            trigger: Some(ep.trigger.clone()),
            steps: fsteps,
            attributes: attrs.clone(),
        };
        // merge dataflow steps into a dataflow flow
        if !store_steps.is_empty() {
            let df_id = entity_id(&store.repo_id, kinds::FLOW, &format!("{}-data", ep.name));
            let mut dfs: Vec<FlowStep> = Vec::new();
            let mut seen_df: HashSet<(String, String)> = HashSet::new();
            for (name, pred, prov) in &store_steps {
                if !seen_df.insert((name.clone(), pred.clone())) {
                    continue;
                }
                let op = match pred.as_str() {
                    "writes" => "write",
                    "reads" => "read",
                    "queries" => "query",
                    "publishes" => "publish",
                    "subscribes" => "subscribe",
                    _ => pred.as_str(),
                };
                dfs.push(FlowStep {
                    id: format!("step:{}", dfs.len() + 1),
                    order: (dfs.len() + 1) as u32,
                    actor: format!("store:{name}"),
                    operation: op.to_string(),
                    condition: None,
                    r#async: Some(pred == "publishes" || pred == "subscribes"),
                    timeout_ms: None,
                    retry_policy: None,
                    failure_outcome: None,
                    provenance: Some(*prov),
                    evidence: Vec::new(),
                });
            }
            if !dfs.is_empty() {
                dataflows.push(Flow {
                    id: df_id,
                    kind: FlowKind::Dataflow,
                    name: format!("{}-data", ep.name),
                    trigger: Some(ep.trigger),
                    steps: dfs,
                    attributes: attrs,
                });
            }
        }
        sequences.push(seq);
    }

    // ---- architecture view ----
    let arch = compile_architecture(graph, store);

    Ok((sequences, dataflows, arch))
}

/// Architecture flow: one step per component with its dependencies and
/// responsibilities.
fn compile_architecture(_graph: &RealityGraph, store: &Store) -> Option<Flow> {
    let comps = store.components().ok()?;
    if comps.is_empty() {
        return None;
    }
    let mut steps: Vec<FlowStep> = Vec::new();
    for c in comps {
        let resp: Vec<String> = c
            .attributes
            .get("responsibility")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|r| r.get("text").and_then(|t| t.as_str()).map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let deps: Vec<String> = c
            .attributes
            .get("depends_on")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|d| d.get("target").and_then(|t| t.as_str()).map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        steps.push(FlowStep {
            id: format!("step:{}", steps.len() + 1),
            order: (steps.len() + 1) as u32,
            actor: c.id.clone(),
            operation: resp.first().cloned().unwrap_or_else(|| c.name.clone()),
            condition: if deps.is_empty() {
                None
            } else {
                Some(format!("depends_on: {}", deps.join(", ")))
            },
            r#async: None,
            timeout_ms: None,
            retry_policy: None,
            failure_outcome: None,
            provenance: Some(Provenance::Resolved),
            evidence: c.evidence.clone(),
        });
    }
    Some(Flow {
        id: entity_id(&store.repo_id, kinds::FLOW, "architecture"),
        kind: FlowKind::Architecture,
        name: "architecture".into(),
        trigger: Some("system".into()),
        steps,
        attributes: BTreeMap::new(),
    })
}

impl RealityGraph {
    pub fn all_rels(&self) -> Vec<&Relationship> {
        self.out.values().flatten().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_calls_caps_depth_and_cycles() {
        // build a tiny graph manually
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let _g = RealityGraph::load(&store).unwrap();
        // a -> b -> c -> a (cycle), d
        let mk = |n: &str| scc_core::symbol_id("repo", "x.py", n);
        for (s, t) in [
            ("a", "b"),
            ("b", "c"),
            ("c", "a"),
            ("a", "d"),
        ] {
            let r = Relationship::new(
                crate::components::rel(&["t", s, t]),
                mk(s),
                "calls",
                mk(t),
                Provenance::Resolved,
            );
            store.insert_relationship(&r, "x.py").unwrap();
        }
        let g = RealityGraph::load(&store).unwrap();
        let paths = walk_calls(&g, &mk("a"));
        assert!(!paths.is_empty());
        // every path starts at a and is cycle-free
        for p in &paths {
            assert_eq!(p[0], mk("a"));
            assert!(p.len() <= MAX_DEPTH + 1);
        }
    }

    #[test]
    fn invocation_surfaces_seed_from_semantic_facts() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let repo = "repo";
        let mk_sym = |n: &str| scc_core::symbol_id(repo, "app.py", n);

        // symbols
        for n in ["create_app", "worker", "setup", "handler", "listener"] {
            let mut e = scc_core::Entity::new(mk_sym(n), kinds::SYMBOL, n);
            e.attr("file", serde_json::json!("app.py"));
            store.insert_entity(&e, &["app.py".to_string()]).unwrap();
        }
        // public export: create_app EXPORTS export(create_app, kind=function)
        let exp_id = scc_core::entity_id(repo, kinds::EXPORT, "create_app");
        let mut exp = scc_core::Entity::new(exp_id.clone(), kinds::EXPORT, "create_app");
        exp.attr("kind", serde_json::json!("function"));
        store.insert_entity(&exp, &["app.py".to_string()]).unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    scc_core::relationship_id(1),
                    mk_sym("create_app"),
                    scc_core::predicates::EXPORTS,
                    exp_id,
                    Provenance::Extracted,
                ),
                "app.py",
            )
            .unwrap();
        // queue consumer: worker SUBSCRIBES topic(jobs)
        let topic_id = scc_core::entity_id(repo, kinds::TOPIC, "jobs");
        store
            .insert_entity(&scc_core::Entity::new(topic_id.clone(), kinds::TOPIC, "jobs"), &["app.py".to_string()])
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    scc_core::relationship_id(2),
                    mk_sym("worker"),
                    scc_core::predicates::SUBSCRIBES,
                    topic_id.clone(),
                    Provenance::Extracted,
                ),
                "app.py",
            )
            .unwrap();
        // event handler: handler CONSUMES topic(jobs)
        store
            .insert_relationship(
                &Relationship::new(
                    scc_core::relationship_id(3),
                    mk_sym("handler"),
                    scc_core::predicates::CONSUMES,
                    topic_id,
                    Provenance::Extracted,
                ),
                "app.py",
            )
            .unwrap();
        // framework callback: listener HANDLES_CALLBACK setup
        store
            .insert_relationship(
                &Relationship::new(
                    scc_core::relationship_id(4),
                    mk_sym("listener"),
                    scc_core::predicates::HANDLES_CALLBACK,
                    mk_sym("setup"),
                    Provenance::Extracted,
                ),
                "app.py",
            )
            .unwrap();
        // lifecycle: annotation Before ANNOTATES setup
        let ann_id = scc_core::entity_id(repo, kinds::ANNOTATION, "Before");
        store
            .insert_entity(&scc_core::Entity::new(ann_id.clone(), kinds::ANNOTATION, "Before"), &["app.py".to_string()])
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    scc_core::relationship_id(5),
                    ann_id,
                    scc_core::predicates::ANNOTATES,
                    mk_sym("setup"),
                    Provenance::Extracted,
                ),
                "app.py",
            )
            .unwrap();

        let g = RealityGraph::load(&store).unwrap();
        let surfaces = invocation_surfaces(&g);

        let kinds_of = |k: InvocationSurfaceKind| -> Vec<String> {
            surfaces
                .iter()
                .filter(|s| s.kind == k)
                .map(|s| format!("{}:{}", s.symbol, s.trigger))
                .collect()
        };
        assert_eq!(
            kinds_of(InvocationSurfaceKind::PublicApi),
            vec![format!("{}:export:create_app (function)", mk_sym("create_app"))],
            "public export surfaces: {surfaces:?}"
        );
        assert_eq!(
            kinds_of(InvocationSurfaceKind::Queue),
            vec![format!("{}:subscribe:jobs", mk_sym("worker"))],
            "queue surfaces: {surfaces:?}"
        );
        assert_eq!(
            kinds_of(InvocationSurfaceKind::FrameworkCallback),
            vec![format!("{}:callback:setup", mk_sym("listener"))],
            "callback surfaces: {surfaces:?}"
        );
        assert_eq!(
            kinds_of(InvocationSurfaceKind::Lifecycle),
            vec![format!("{}:lifecycle:Before", mk_sym("setup"))],
            "lifecycle surfaces: {surfaces:?}"
        );
        assert_eq!(
            kinds_of(InvocationSurfaceKind::Event),
            vec![format!("{}:event:jobs", mk_sym("handler"))],
            "event surfaces: {surfaces:?}"
        );

        // deterministic ordering (InvocationSurfaceKind enum order)
        let kinds: Vec<&str> = surfaces.iter().map(|s| s.kind.as_str()).collect();
        let mut sorted = kinds.clone();
        sorted.sort_by_key(|k| match *k {
            "process" => 0,
            "http" => 1,
            "cli" => 2,
            "public_api" => 3,
            "event" => 4,
            "queue" => 5,
            "schedule" => 6,
            "plugin" => 7,
            "framework_callback" => 8,
            "lifecycle" => 9,
            _ => 10,
        });
        assert_eq!(kinds, sorted, "surfaces must be deterministically ordered");

        // collect_entrypoints picks the surfaces up (kind strings carried)
        let eps = collect_entrypoints(&g, &store, &[]);
        let surface_kinds: Vec<String> = eps
            .iter()
            .filter(|e| !matches!(e.kind.as_str(), "route" | "entrypoint" | "intent"))
            .map(|e| e.kind.clone())
            .collect();
        for want in ["public_api", "queue", "framework_callback", "lifecycle", "event"] {
            assert!(
                surface_kinds.iter().any(|k| k == want),
                "entrypoints must include {want}: {surface_kinds:?}"
            );
        }
    }

    #[test]
    fn public_api_chain_surfaces_seed_before_leaves() {
        // Exported symbols in one file: `deep` reaches a 3-node chain,
        // `mid` a 2-node chain, `leaf` has no call edges. The chain class
        // must seed before the leaf class, deepest first — even though the
        // leaf sorts before both by symbol id (the old id-order cap spent
        // itself on exactly this noise).
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let mk = |n: &str| scc_core::symbol_id("repo", "x.py", n);

        for n in ["deep", "deep2", "mid", "mid2", "leaf", "leaf2"] {
            let mut e = scc_core::Entity::new(mk(n), kinds::SYMBOL, n);
            e.attr("file", serde_json::json!("x.py"));
            e.attr("exported", serde_json::json!(true));
            store.insert_entity(&e, &["x.py".to_string()]).unwrap();
        }
        // deep -> deep2 -> (leaf2 as a third hop); mid -> mid2
        let mut rid = 0usize;
        let mut call = |s: &str, t: &str| {
            rid += 1;
            store
                .insert_relationship(
                    &Relationship::new(
                        scc_core::relationship_id(rid as u64),
                        s.to_string(),
                        scc_core::predicates::CALLS,
                        t.to_string(),
                        Provenance::Resolved,
                    ),
                    "x.py",
                )
                .unwrap();
        };
        call(&mk("deep"), &mk("deep2"));
        call(&mk("deep2"), &mk("leaf2"));
        call(&mk("mid"), &mk("mid2"));

        let g = RealityGraph::load(&store).unwrap();
        let eps = collect_entrypoints(&g, &store, &[]);
        let pub_names: Vec<String> = eps
            .iter()
            .filter(|e| e.kind == "public_api")
            .map(|e| e.name.clone())
            .collect();
        // deepest chain first, then the 2-node chain, then leaves — the
        // chain targets are themselves exported, so deep2 (a 2-node chain
        // via its edge to leaf2) and mid (2-node) rank in the chain class;
        // mid2 (no outgoing edges) is a leaf.
        assert_eq!(
            pub_names,
            vec!["deep", "deep2", "mid", "leaf", "leaf2", "mid2"],
            "chain surfaces (deepest first) seed before leaves: {pub_names:?}"
        );
    }

    #[test]
    fn public_api_seed_budgets_are_bounded_by_class() {
        // CHAIN_CAP + 5 chained exports and LEAF_CAP + 5 leaves: exactly
        // the chain budget is spent on chained surfaces (id order), then
        // the leaf budget on leaves; nothing beyond.
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let mk = |n: &str| scc_core::symbol_id("repo", "x.py", n);

        let mut rid = 0usize;
        for i in 0..(PUBLIC_API_CHAIN_SEED_CAP + 5) {
            let c = format!("chain{i:02}");
            let t = format!("target{i:02}");
            for n in [&c, &t] {
                let mut e = scc_core::Entity::new(mk(n), kinds::SYMBOL, n);
                e.attr("file", serde_json::json!("x.py"));
                e.attr("exported", serde_json::json!(true));
                store.insert_entity(&e, &["x.py".to_string()]).unwrap();
            }
            rid += 1;
            store
                .insert_relationship(
                    &Relationship::new(
                        scc_core::relationship_id(rid as u64),
                        mk(&c),
                        scc_core::predicates::CALLS,
                        mk(&t),
                        Provenance::Extracted,
                    ),
                    "x.py",
                )
                .unwrap();
        }
        for i in 0..(PUBLIC_API_LEAF_SEED_CAP + 5) {
            let n = format!("leaf{i:02}");
            let mut e = scc_core::Entity::new(mk(&n), kinds::SYMBOL, &n);
            e.attr("file", serde_json::json!("x.py"));
            e.attr("exported", serde_json::json!(true));
            store.insert_entity(&e, &["x.py".to_string()]).unwrap();
        }

        let g = RealityGraph::load(&store).unwrap();
        let eps = collect_entrypoints(&g, &store, &[]);
        let pub_names: Vec<String> = eps
            .iter()
            .filter(|e| e.kind == "public_api")
            .map(|e| e.name.clone())
            .collect();
        assert_eq!(pub_names.len(), PUBLIC_API_CHAIN_SEED_CAP + PUBLIC_API_LEAF_SEED_CAP);
        let chains: Vec<&String> = pub_names.iter().filter(|n| n.starts_with("chain")).collect();
        let leaves: Vec<&String> = pub_names.iter().filter(|n| n.starts_with("leaf")).collect();
        assert_eq!(chains.len(), PUBLIC_API_CHAIN_SEED_CAP, "chain budget fully spent");
        assert_eq!(leaves.len(), PUBLIC_API_LEAF_SEED_CAP, "leaf budget fully spent");
        // deterministic id order within each class
        assert!(chains.windows(2).all(|w| w[0] < w[1]), "{pub_names:?}");
        assert!(leaves.windows(2).all(|w| w[0] < w[1]), "{pub_names:?}");
        // chain class entirely precedes the leaf class
        let first_leaf = pub_names.iter().position(|n| n.starts_with("leaf")).unwrap();
        assert!(chains.iter().all(|n| {
            pub_names.iter().position(|p| p == *n).unwrap() < first_leaf
        }));
    }
}
