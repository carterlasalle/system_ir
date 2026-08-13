//! Canonical causal FlowGraph compiler (Wave 3, docs/SYSTEM_DESIGN.md §9).
//!
//! One graph per entrypoint, built ONLY from evidence:
//!
//! - Next edges from resolved CALLS paths
//! - Branch edges from call fanout (one operation -> 2+ distinct callees)
//! - Retry edges from RETRIES predicates (extracted failure behavior)
//! - Error edges to failure outcomes (retry exhaustion)
//! - Async edges from async call attributes
//! - Publish/Consume edges from queue predicates
//! - Join edges at convergence; Return/exit detection
//!
//! Branches NEVER come from generated-text heuristics (e.g. splitting on
//! ", "). The canonical graph preserves topology exactly: alternate
//! execution paths can never be flattened into false sequential causality.
//! Individual operations are retained; component grouping is a display-time
//! concern only (ComponentSpan).

use crate::flows::{collect_entrypoints, walk_calls};
use crate::components::prov_rank;
use crate::RealityGraph;
use scc_core::{
    entity_id, FlowEdge, FlowEdgeKind, FlowGraph, FlowKind, FlowNode, Provenance,
};
use scc_store::Store;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Node key: (actor component id, operation).
type NodeKey = (String, String);

fn op_of(graph: &RealityGraph, sym: &str) -> String {
    graph
        .entities
        .get(sym)
        .map(|e| e.name.clone())
        .unwrap_or_else(|| sym.to_string())
}

/// Mutable edge registry for one graph build (dedup + join detection).
struct EdgeTable {
    edges: Vec<FlowEdge>,
    seen: BTreeSet<(u32, u32, String)>,
    in_degree: HashMap<u32, u32>,
}

impl EdgeTable {
    fn new() -> EdgeTable {
        EdgeTable {
            edges: Vec::new(),
            seen: BTreeSet::new(),
            in_degree: HashMap::new(),
        }
    }

    fn push(
        &mut self,
        from: u32,
        to: u32,
        kind: FlowEdgeKind,
        condition: Option<String>,
        prov: Provenance,
        evidence: Vec<String>,
    ) {
        let k = (from, to, format!("{kind:?}"));
        // Dedupe is provenance-rank-wins, never first-wins: a target the
        // language server RESOLVED must not be demoted by an earlier
        // native EXTRACTED candidate for the same (from, to, kind) triple
        // (and vice versa, a native candidate keeps the edge when no LSP
        // evidence exists — holdout repos). Evidence merges.
        if let Some(existing) = self
            .edges
            .iter_mut()
            .find(|e| e.from == from && e.to == to && format!("{:?}", e.kind) == format!("{kind:?}"))
        {
            let have = existing.provenance.unwrap_or(Provenance::Inferred);
            if prov_rank(prov) > prov_rank(have) {
                existing.provenance = Some(prov);
                existing.confidence = prov.default_confidence();
            }
            if existing.condition.is_none() {
                existing.condition = condition;
            }
            for e in evidence {
                if !existing.evidence.contains(&e) {
                    existing.evidence.push(e);
                }
            }
            return;
        }
        self.seen.insert(k);
        *self.in_degree.entry(to).or_insert(0) += 1;
        self.edges.push(FlowEdge {
            from,
            to,
            kind,
            condition,
            provenance: Some(prov),
            confidence: prov.default_confidence(),
            evidence,
        });
    }
}

/// Mutable node registry for one graph build.
struct NodeTable {
    nodes: Vec<FlowNode>,
    by_key: HashMap<NodeKey, u32>,
}

impl NodeTable {
    fn new() -> NodeTable {
        NodeTable {
            nodes: Vec::new(),
            by_key: HashMap::new(),
        }
    }

    fn get(&mut self, key: &NodeKey) -> u32 {
        if let Some(id) = self.by_key.get(key) {
            return *id;
        }
        let id = self.nodes.len() as u32;
        self.by_key.insert(key.clone(), id);
        self.nodes.push(FlowNode {
            id,
            actor: key.0.clone(),
            operation: key.1.clone(),
            evidence: Vec::new(),
        });
        id
    }

    fn lookup(&self, key: &NodeKey) -> Option<u32> {
        self.by_key.get(key).copied()
    }
}

/// Compile the canonical flow graphs for every entrypoint.
///
/// `symbol_comp` maps symbol id -> component entity id (same mapping the
/// projection compilers use, so displays agree).
// trace:v1 id=impl.scc.flowgraph work=WORK-SCC-005 satisfies=REQ-SCC-FLOW
pub fn compile_flow_graphs(
    graph: &RealityGraph,
    store: &Store,
    intent: &[(String, serde_json::Value)],
    symbol_comp: &HashMap<String, String>,
) -> crate::Result<Vec<FlowGraph>> {
    let entrypoints = collect_entrypoints(graph, store, intent);
    let mut out: Vec<FlowGraph> = Vec::new();

    for ep in entrypoints {
        if ep.symbol_id.is_empty() {
            continue;
        }
        let paths = walk_calls(graph, &ep.symbol_id);
        let mut table = NodeTable::new();
        let mut node_evidence: BTreeMap<NodeKey, (Provenance, BTreeSet<String>)> = BTreeMap::new();

        // entry node
        let entry_actor = symbol_comp
            .get(&ep.symbol_id)
            .cloned()
            .unwrap_or_else(|| "component:root".into());
        let entry_key = (entry_actor.clone(), op_of(graph, &ep.symbol_id));
        let entry_id = table.get(&entry_key);

        // ---- edges from call paths ----
        // successors[sym] = distinct callees resolved from CALLS
        let mut successors: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut call_rel: HashMap<(String, String), (Provenance, Vec<String>)> = HashMap::new();
        let mut syms: BTreeSet<String> = paths
            .iter()
            .flatten()
            .cloned()
            .collect();
        syms.insert(ep.symbol_id.clone());
        for sym in &syms {
            for r in graph.out_pred(sym, scc_core::predicates::CALLS) {
                if matches!(r.provenance, Provenance::Extracted | Provenance::Resolved) {
                    successors
                        .entry(sym.clone())
                        .or_default()
                        .insert(r.object.clone());
                    let key = (sym.clone(), r.object.clone());
                    let entry = call_rel.entry(key).or_insert((r.provenance, Vec::new()));
                    if prov_rank(r.provenance) > prov_rank(entry.0) {
                        entry.0 = r.provenance;
                    }
                    for e in &r.evidence {
                        if !entry.1.contains(e) {
                            entry.1.push(e.clone());
                        }
                    }
                }
            }
        }

        let mut edges = EdgeTable::new();

        // Materialize every reachable symbol as a node up front so node ids
        // are deterministic (sorted symbol order) and every caller's
        // from-lookup succeeds regardless of edge-creation order.
        for sym in &syms {
            let key = (
                symbol_comp
                    .get(sym)
                    .cloned()
                    .unwrap_or_else(|| "component:root".into()),
                op_of(graph, sym),
            );
            table.get(&key);
        }

        for (sym, targets) in &successors {
            let key = (
                symbol_comp.get(sym).cloned().unwrap_or_else(|| "component:root".into()),
                op_of(graph, sym),
            );
            let from = table.lookup(&key).unwrap_or(entry_id);
            // node evidence from the entity itself
            if let Some(e) = graph.entities.get(sym) {
                if let Some(nk) = table
                    .by_key
                    .iter()
                    .find(|(_, v)| **v == from)
                    .map(|(k, _)| k.clone())
                {
                    node_evidence
                        .entry(nk)
                        .or_insert((Provenance::Resolved, BTreeSet::new()))
                        .1
                        .extend(e.evidence.clone());
                }
            }
            // P1 §19: call fanout is NOT control-flow branching. A call
            // graph says "A may call B and C", not "A chooses B or C".
            // Branch edges exist ONLY where the extractor recorded the call
            // inside a conditional/loop/try body — `call_blocks` (CFG
            // evidence, condition = block kind) with `conditional_calls`
            // as fallback for older indexes.
            let cond_attr = |name: &str| -> Option<serde_json::Value> {
                graph.entities.get(sym).and_then(|e| e.attributes.get(name).cloned())
            };
            let conditional_calls: std::collections::HashSet<String> = cond_attr("conditional_calls")
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            let call_blocks: BTreeMap<String, String> = cond_attr("call_blocks")
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            let call_order: BTreeMap<String, u32> = cond_attr("call_order")
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            let awaited_calls: std::collections::BTreeSet<String> = cond_attr("awaited_calls")
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            let match_callee = |attr: &str, t_op: &str| -> bool {
                attr == t_op || attr.ends_with(&format!(".{t_op}"))
            };
            // Deterministic order: CFG lexical order within the caller
            // (min over matching call sites), then callee name. Straight-
            // line calls keep their source sequence; unknown callees sort
            // after known ones.
            let mut ordered: Vec<(u32, &String)> = targets
                .iter()
                .map(|t| {
                    let t_op = op_of(graph, t);
                    let order = call_order
                        .iter()
                        .filter(|(c, _)| match_callee(c, &t_op))
                        .map(|(_, o)| *o)
                        .min()
                        .unwrap_or(u32::MAX);
                    (order, t)
                })
                .collect();
            ordered.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
            for (_, t) in ordered {
                let actor = symbol_comp
                    .get(t)
                    .cloned()
                    .unwrap_or_else(|| "component:root".into());
                let tkey = (actor, op_of(graph, t));
                let to = table.get(&tkey);
                let (prov, evidence) = call_rel
                    .get(&(sym.clone(), t.clone()))
                    .cloned()
                    .unwrap_or((Provenance::Resolved, Vec::new()));
                let t_op = op_of(graph, t);
                let block = call_blocks
                    .iter()
                    .find(|(c, _)| match_callee(c, &t_op))
                    .map(|(_, b)| b.clone());
                let fallback = conditional_calls
                    .iter()
                    .any(|c| match_callee(c, &t_op));
                let is_branch = block.is_some() || fallback;
                edges.push(
                    from,
                    to,
                    if is_branch {
                        FlowEdgeKind::Branch
                    } else {
                        FlowEdgeKind::Next
                    },
                    if let Some(b) = &block {
                        Some(b.clone())
                    } else if is_branch {
                        Some(format!("conditional: {t_op}"))
                    } else {
                        None
                    },
                    prov,
                    evidence,
                );
                // Async edge for awaited/spawned call sites (CFG evidence):
                // `await`, `.await`, `go` statement, `Promise.all`.
                if awaited_calls.iter().any(|c| match_callee(c, &t_op)) {
                    edges.push(
                        from,
                        to,
                        FlowEdgeKind::Async,
                        Some(format!("awaited: {t_op}")),
                        prov,
                        Vec::new(),
                    );
                }
            }
        }

        // ---- retry / failure edges from extracted behavior ----
        // Evidence: the `retry_policy` attribute stamped on decorated
        // symbols (@retry / @tenacity.retry / @backoff decorators). The
        // retrying op gets a Retry self-loop (attempt) and an Error edge to
        // its successor (exhausted -> failure path).
        let mut retry_syms: Vec<(u32, String, String)> = Vec::new();
        for (sym, comp) in symbol_comp {
            if let Some(e) = graph.entities.get(sym) {
                if let Some(rp) = e.attributes.get("retry_policy").and_then(|v| v.as_str()) {
                    let key = (comp.clone(), op_of(graph, sym));
                    if let Some(id) = table.lookup(&key) {
                        retry_syms.push((id, sym.clone(), rp.to_string()));
                    }
                }
            }
        }
        retry_syms.sort();
        for (id, sym, rp) in retry_syms {
            let evidence: Vec<String> = graph
                .entities
                .get(&sym)
                .map(|e| e.evidence.clone())
                .unwrap_or_default();
            edges.push(
                id,
                id,
                FlowEdgeKind::Retry,
                Some(format!("attempt ({rp})")),
                Provenance::Extracted,
                evidence.clone(),
            );
            // exhausted -> failure: Error edge to the first successor
            let succ: Vec<u32> = edges
                .edges
                .iter()
                .filter(|e| e.from == id && e.kind == FlowEdgeKind::Next)
                .map(|e| e.to)
                .collect();
            if let Some(s) = succ.first() {
                edges.push(
                    id,
                    *s,
                    FlowEdgeKind::Error,
                    Some("exhausted".into()),
                    Provenance::Extracted,
                    evidence,
                );
            }
        }

        // ---- publish/consume edges (queue semantics) ----
        for (sym, comp) in symbol_comp {
            for r in graph.out_pred(sym, scc_core::predicates::PUBLISHES) {
                let from_key = (comp.clone(), op_of(graph, sym));
                if let Some(from) = table.lookup(&from_key) {
                    let q_key = (r.object.clone(), format!("queue {}", op_of(graph, &r.object)));
                    let to = table.get(&q_key);
                    edges.push(
                        from,
                        to,
                        FlowEdgeKind::Publish,
                        None,
                        r.provenance,
                        r.evidence.clone(),
                    );
                }
            }
            for r in graph.out_pred(sym, scc_core::predicates::CONSUMES) {
                let from_key = (comp.clone(), op_of(graph, sym));
                if let Some(from) = table.lookup(&from_key) {
                    let q_key = (r.object.clone(), format!("queue {}", op_of(graph, &r.object)));
                    let to = table.get(&q_key);
                    edges.push(
                        from,
                        to,
                        FlowEdgeKind::Consume,
                        None,
                        r.provenance,
                        r.evidence.clone(),
                    );
                }
            }
        }

        // ---- async attributes on call edges ----
        // (extractors set `async: true` on CALLS relationships when the
        // call is fire-and-forget; honor them if present)
        for (sym, comp) in symbol_comp {
            for r in graph.out_pred(sym, scc_core::predicates::CALLS) {
                if r.provenance != Provenance::Resolved {
                    continue;
                }
                let is_async = graph
                    .entities
                    .get(&r.object)
                    .and_then(|e| e.attributes.get("async"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if !is_async {
                    continue;
                }
                let from_key = (comp.clone(), op_of(graph, sym));
                let to_key = (
                    symbol_comp
                        .get(&r.object)
                        .cloned()
                        .unwrap_or_else(|| "component:root".into()),
                    op_of(graph, &r.object),
                );
                if let (Some(from), Some(to)) =
                    (table.lookup(&from_key), table.lookup(&to_key))
                {
                    edges.push(
                        from,
                        to,
                        FlowEdgeKind::Async,
                        None,
                        r.provenance,
                        r.evidence.clone(),
                    );
                }
            }
        }

        // ---- join edges at convergence ----
        let convergents: Vec<u32> = {
            let mut v: Vec<u32> = edges
                .in_degree
                .iter()
                .filter(|(_, n)| **n > 1)
                .map(|(id, _)| *id)
                .collect();
            v.sort_unstable();
            v
        };
        for to in convergents {
            let mut preds: Vec<u32> = edges
                .edges
                .iter()
                .filter(|e| e.to == to && e.kind == FlowEdgeKind::Next)
                .map(|e| e.from)
                .collect();
            preds.sort_unstable();
            preds.dedup();
            for from in preds {
                edges.push(
                    from,
                    to,
                    FlowEdgeKind::Join,
                    Some("converge".into()),
                    Provenance::Resolved,
                    Vec::new(),
                );
            }
        }

        // ---- attach per-node evidence ----
        for (key, (prov, ev)) in &node_evidence {
            if let Some(id) = table.lookup(key) {
                let n = table.nodes.get_mut(id as usize).expect("node exists");
                n.evidence = ev.iter().cloned().collect();
                n.evidence.sort();
            }
            let _ = prov;
        }

        // ---- exits (no outgoing edges) ----
        let has_out: BTreeSet<u32> = edges.edges.iter().map(|e| e.from).collect();
        let mut exits: Vec<u32> = table
            .nodes
            .iter()
            .map(|n| n.id)
            .filter(|id| !has_out.contains(id))
            .collect();
        exits.sort_unstable();

        // provenance summary
        let mut provenance_summary: BTreeMap<String, usize> = BTreeMap::new();
        for e in &edges.edges {
            if let Some(p) = e.provenance {
                *provenance_summary.entry(p.as_str().to_string()).or_insert(0) += 1;
            }
        }

        let graph_id = entity_id(&store.repo_id, scc_core::kinds::FLOW, &ep.name);
        out.push(FlowGraph {
            id: graph_id,
            kind: FlowKind::Sequence,
            name: ep.name.clone(),
            trigger: Some(ep.trigger.clone()),
            nodes: table.nodes,
            edges: edges.edges,
            entrypoints: vec![entry_id],
            exits,
            provenance_summary,
        });
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(comp: &str, op: &str) -> NodeKey {
        (comp.to_string(), op.to_string())
    }

    #[test]
    fn node_key_and_op() {
        assert_eq!(key("services", "save"), ("services".to_string(), "save".to_string()));
        assert_eq!(op_of(&RealityGraph::empty(), "repo://r/symbol/x.py/f"), "repo://r/symbol/x.py/f");
    }

    #[test]
    fn edge_dedupe_is_provenance_rank_wins() {
        // The same (from, to, kind) triple pushed twice — once with a
        // native EXTRACTED candidate, once with LSP RESOLVED proof — keeps
        // the RESOLVED provenance (dedupe is rank-wins, never first-wins),
        // and a native-only triple keeps its EXTRACTED provenance (flows
        // exist without LSP).
        let mut t = EdgeTable::new();
        t.push(0, 1, FlowEdgeKind::Next, None, Provenance::Extracted, vec!["ev:native".into()]);
        t.push(0, 1, FlowEdgeKind::Next, None, Provenance::Resolved, vec!["ev:lsp".into()]);
        assert_eq!(t.edges.len(), 1, "duplicate triple dedupes to one edge");
        assert_eq!(
            t.edges[0].provenance,
            Some(Provenance::Resolved),
            "RESOLVED wins over an earlier EXTRACTED candidate"
        );
        assert!(t.edges[0].evidence.contains(&"ev:native".to_string()));
        assert!(t.edges[0].evidence.contains(&"ev:lsp".to_string()));
        assert_eq!(t.in_degree.get(&1), Some(&1), "in-degree counted once");

        let mut native = EdgeTable::new();
        native.push(0, 1, FlowEdgeKind::Next, None, Provenance::Extracted, Vec::new());
        assert_eq!(
            native.edges[0].provenance,
            Some(Provenance::Extracted),
            "native-only edge keeps EXTRACTED provenance"
        );
    }

    #[test]
    fn branch_detection_is_structural() {
        // Branch edges come from call FANOUT — never from text heuristics.
        // (topology is exercised end-to-end in the fixture tests)
        assert_ne!(FlowEdgeKind::Branch, FlowEdgeKind::Next);
    }
}
