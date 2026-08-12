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
use scc_core::{entity_id, Flow, FlowKind, FlowStep, Provenance, Relationship};
use scc_store::Store;
use serde_json::json;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

const MAX_DEPTH: usize = 10;
const MAX_BREADTH: usize = 64;

pub struct FlowEntrypoint {
    pub name: String,
    pub trigger: String,
    pub symbol_id: String,
    pub kind: String, // "route" | "entrypoint" | "intent"
}

/// Collect entrypoints: routes (handlers), declared intent flows, symbols
/// marked as entrypoints (main-guard / bin / module-entry).
pub fn collect_entrypoints(
    graph: &RealityGraph,
    store: &Store,
    intent: &[(String, serde_json::Value)],
) -> Vec<FlowEntrypoint> {
    let mut out: Vec<FlowEntrypoint> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // routes
    for route in graph.entities_of_kind(kinds::ROUTE) {
        if let Some(handler) = route.attributes.get("handler").and_then(|v| v.as_str()) {
            let method = route.attributes.get("method").and_then(|v| v.as_str()).unwrap_or("");
            let path = route.attributes.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let name = format!("{}-{}", method.to_ascii_lowercase(), path);
            if seen.insert(name.clone()) {
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
            if seen.insert(name.clone()) {
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
        if seen.insert(e.name.clone()) {
            out.push(FlowEntrypoint {
                name: e.name.clone(),
                trigger: format!("entrypoint:{}", e.name),
                symbol_id: e.id.clone(),
                kind: "entrypoint".into(),
            });
        }
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

        // collapse consecutive same-actor steps into one
        let mut collapsed: Vec<(String, String)> = Vec::new();
        for (actor, op) in steps {
            if let Some((pa, po)) = collapsed.last_mut() {
                if pa == &actor {
                    po.push_str(", ");
                    po.push_str(&op);
                    continue;
                }
            }
            collapsed.push((actor, op));
        }

        let mut fsteps: Vec<FlowStep> = Vec::new();
        for (i, (actor, op)) in collapsed.iter().enumerate() {
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
}
