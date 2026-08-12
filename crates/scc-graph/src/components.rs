//! Component Compiler (EPIC-040, docs/SYSTEM_DESIGN.md §7).
//!
//! Deterministic MVP signals:
//! 1. explicit intent (`.scc/intent.yaml` components with `paths`)
//! 2. package.json workspace members
//! 3. docker-compose service build contexts
//! 4. top-level directory boundaries
//!
//! Each component aggregates: responsibilities (routes owned, docstrings
//! INFERRED, intent DECLARED), ownership (store write edges), dependency
//! edges (cross-component calls), and implementation (paths/symbols).

use crate::{RealityGraph, Result};
use scc_core::kinds;
use scc_core::{entity_id, Provenance, Relationship};
use scc_store::Store;
use serde_json::json;
use std::collections::{BTreeMap, HashMap, HashSet};

pub const RELPREFIX: &str = "rel:comp:";

/// Evidence class that created a component candidate (Wave 5, plan §27-30).
/// The `boundary_kind` attribute records it on every compiled component;
/// `code-region` is the bare top-level directory fallback and is never
/// authoritative architecture.
pub const BOUNDARY_DECLARED: &str = "declared";
pub const BOUNDARY_PACKAGE: &str = "package";
pub const BOUNDARY_DEPLOYMENT: &str = "deployment";
pub const BOUNDARY_CODE_REGION: &str = "code-region";
pub const BOUNDARY_ROOT: &str = "root";

/// Authority order for `boundary_kind` when one candidate is created by
/// several sources: declared intent > deployment units > workspace
/// packages > directory fallback. Deterministic (fixed precedence).
fn boundary_rank(kind: &str) -> u8 {
    match kind {
        BOUNDARY_DECLARED => 3,
        BOUNDARY_DEPLOYMENT => 2,
        BOUNDARY_PACKAGE => 1,
        _ => 0,
    }
}

pub fn rel(parts: &[&str]) -> String {
    let mut h = blake3::Hasher::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update(b"|");
    }
    format!("{RELPREFIX}{}", &h.finalize().to_hex()[..12])
}

pub fn parse_prov(s: &str) -> Provenance {
    match s {
        "RESOLVED" => Provenance::Resolved,
        "EXTRACTED" => Provenance::Extracted,
        "DECLARED" => Provenance::Declared,
        "OBSERVED" => Provenance::Observed,
        "INFERRED" => Provenance::Inferred,
        _ => Provenance::Inferred,
    }
}

pub fn prov_rank(p: Provenance) -> u8 {
    match p {
        Provenance::Resolved => 4,
        Provenance::Observed => 4,
        Provenance::Extracted => 3,
        Provenance::Declared => 2,
        Provenance::Inferred => 1,
        Provenance::Stale => 0,
    }
}

#[derive(Debug, Clone)]
pub struct ComponentCandidate {
    pub name: String,
    pub dirs: Vec<String>,
    /// Evidence class that created this candidate (one of the
    /// `BOUNDARY_*` constants).
    pub boundary_kind: String,
}

/// Determine the component for a path: longest matching dir prefix wins.
/// Returns a `String` (candidate names are owned).
pub fn component_for_path(path: &str, candidates: &[ComponentCandidate]) -> String {
    let mut best: Option<(String, usize)> = None;
    for c in candidates {
        for d in &c.dirs {
            let d = d.trim_end_matches('/');
            if d.is_empty() {
                continue;
            }
            if path == d || path.starts_with(&format!("{d}/")) {
                let len = d.len();
                if best.as_ref().map(|(_, bl)| len > *bl).unwrap_or(true) {
                    best = Some((c.name.clone(), len));
                }
            }
        }
    }
    match best {
        Some((name, _)) => name,
        None => {
            // root-level file -> "root" component; nested -> first segment
            if path.contains('/') {
                let seg = path.split('/').next().unwrap_or("root");
                if seg == ".scc" {
                    "root".to_string()
                } else {
                    seg.to_string()
                }
            } else {
                "root".to_string()
            }
        }
    }
}

pub fn compile_components(
    graph: &RealityGraph,
    store: &Store,
    intent: &[(String, serde_json::Value)],
    pairs: &[crate::cochange::CochangePair],
) -> Result<Vec<scc_core::Entity>> {
    let repo_id = &store.repo_id;

    // ---- candidate construction (declared first so they win) ----
    let mut candidates: Vec<ComponentCandidate> = Vec::new();
    let mut declared_names: HashSet<String> = HashSet::new();
    for (source, claim) in intent {
        if source == "component" {
            let name = claim["name"].as_str().unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }
            declared_names.insert(name.clone());
            let mut dirs: Vec<String> = Vec::new();
            if let Some(paths) = claim["paths"].as_array() {
                for p in paths {
                    if let Some(s) = p.as_str() {
                        dirs.push(s.to_string());
                    }
                }
            }
            dirs.push(name.clone()); // implicit: declared name == directory
            candidates.push(ComponentCandidate {
                name,
                dirs,
                boundary_kind: BOUNDARY_DECLARED.to_string(),
            });
        }
    }
    // workspace packages
    for pkg in graph.entities_of_kind(kinds::PACKAGE) {
        if let Some(path) = pkg.attributes.get("path").and_then(|v| v.as_str()) {
            let name = pkg.name.clone();
            if let Some(c) = candidates.iter_mut().find(|c| c.name == name) {
                if !c.dirs.contains(&path.to_string()) {
                    c.dirs.push(path.to_string());
                }
                if boundary_rank(BOUNDARY_PACKAGE) > boundary_rank(&c.boundary_kind) {
                    c.boundary_kind = BOUNDARY_PACKAGE.to_string();
                }
            } else {
                candidates.push(ComponentCandidate {
                    name,
                    dirs: vec![path.to_string()],
                    boundary_kind: BOUNDARY_PACKAGE.to_string(),
                });
            }
        }
    }
    // deployment units with build contexts
    for du in graph.entities_of_kind(kinds::DEPLOYMENT_UNIT) {
        if let Some(ctx) = du.attributes.get("build_context").and_then(|v| v.as_str()) {
            if ctx == "." || ctx == "./" {
                continue;
            }
            let ctx = ctx.trim_start_matches("./");
            let name = du.name.clone();
            if let Some(c) = candidates.iter_mut().find(|c| c.name == name) {
                if !c.dirs.contains(&ctx.to_string()) {
                    c.dirs.push(ctx.to_string());
                }
                if boundary_rank(BOUNDARY_DEPLOYMENT) > boundary_rank(&c.boundary_kind) {
                    c.boundary_kind = BOUNDARY_DEPLOYMENT.to_string();
                }
            } else {
                candidates.push(ComponentCandidate {
                    name,
                    dirs: vec![ctx.to_string()],
                    boundary_kind: BOUNDARY_DEPLOYMENT.to_string(),
                });
            }
        }
    }
    // top-level source dirs so nothing is orphaned; root-level files all
    // belong to the "root" component
    let mut top_dirs: HashSet<String> = HashSet::new();
    for f in graph.entities_of_kind(kinds::FILE) {
        if f.name.contains('/') {
            if let Some(seg) = f.name.split('/').next() {
                if !seg.is_empty() {
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
                    BOUNDARY_ROOT.to_string()
                } else {
                    BOUNDARY_CODE_REGION.to_string()
                },
            });
        }
    }

    // ---- assign files to components ----
    let mut files_in_component: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for f in graph.entities_of_kind(kinds::FILE) {
        let comp = component_for_path(&f.name, &candidates);
        files_in_component
            .entry(comp.to_string())
            .or_default()
            .push(f.id.clone());
    }

    // ---- symbol → component map ----
    let mut symbol_component: HashMap<String, String> = HashMap::new();
    for (comp, files) in &files_in_component {
        for fid in files {
            for r in graph.out_pred(fid, scc_core::predicates::CONTAINS) {
                let sym_id = r.object.clone();
                symbol_component.insert(sym_id, comp.clone());
            }
        }
    }

    // ---- aggregation ----
    let mut responsibilities: BTreeMap<String, Vec<(String, Provenance, f64)>> = BTreeMap::new();
    // Ownership claims: (target entity id, provenance, confidence, evidence).
    // Write edges keep their own provenance; intent ownership stays DECLARED
    // — the compiler never promotes a claim's provenance (P0, §5).
    type OwnershipClaim = (String, Provenance, f64, Vec<String>);
    let mut owns: BTreeMap<String, Vec<OwnershipClaim>> = BTreeMap::new();
    let mut depends: BTreeMap<String, Vec<(String, Provenance, f64, u32)>> = BTreeMap::new();
    let mut symbols_per_comp: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut evidence_per_comp: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut retries_per_comp: BTreeMap<String, Vec<String>> = BTreeMap::new();

    // route ownership: handler handles route (RESOLVED responsibility)
    for (sym_id, comp) in &symbol_component {
        for r in graph.out_pred(sym_id, scc_core::predicates::HANDLES) {
            if let Some(route) = graph.entities.get(&r.object) {
                let method = route.attributes.get("method").and_then(|v| v.as_str()).unwrap_or("");
                let path = route.attributes.get("path").and_then(|v| v.as_str()).unwrap_or("");
                responsibilities.entry(comp.clone()).or_default().push((
                    format!("Handles {method} {path}"),
                    Provenance::Resolved,
                    1.0,
                ));
            }
        }
    }
    // store write ownership (RESOLVED, evidence = the write edges' evidence)
    for (sym_id, comp) in &symbol_component {
        for r in graph.out_pred(sym_id, scc_core::predicates::WRITES) {
            owns.entry(comp.clone()).or_default().push((
                r.object.clone(),
                r.provenance,
                r.confidence,
                r.evidence.clone(),
            ));
        }
    }
    // cross-component call dependencies
    for (sym_id, comp) in &symbol_component {
        for r in graph.out_pred(sym_id, scc_core::predicates::CALLS) {
            if let Some(target_comp) = symbol_component.get(&r.object) {
                if target_comp != comp {
                    let entry = depends.entry(comp.clone()).or_default();
                    if let Some((_, p, c, n)) = entry
                        .iter_mut()
                        .find(|(t, _, _, _)| t == target_comp)
                    {
                        *n += 1;
                        if prov_rank(r.provenance) > prov_rank(*p) {
                            *p = r.provenance;
                        }
                        *c = c.max(r.confidence);
                    } else {
                        entry.push((target_comp.clone(), r.provenance, r.confidence, 1));
                    }
                }
            }
        }
    }
    // symbols/evidence/retries per component (sorted for determinism —
    // aggregation iterates a HashMap)
    for (sym_id, comp) in &symbol_component {
        if let Some(e) = graph.entities.get(sym_id) {
            symbols_per_comp
                .entry(comp.clone())
                .or_default()
                .push(e.name.clone());
            evidence_per_comp
                .entry(comp.clone())
                .or_default()
                .extend(e.evidence.clone());
            if let Some(rp) = e.attributes.get("retry_policy").and_then(|v| v.as_str()) {
                retries_per_comp
                    .entry(comp.clone())
                    .or_default()
                    .push(format!("{} ({rp})", e.name));
            }
        }
    }
    for v in symbols_per_comp.values_mut() {
        v.sort();
        v.dedup();
    }
    for v in evidence_per_comp.values_mut() {
        v.sort();
        v.dedup();
    }
    for v in retries_per_comp.values_mut() {
        v.sort();
        v.dedup();
    }

    // ---- clustering evidence (Wave 5, plan §28): weighted per-candidate
    // signals feeding `clustering_score`. Deterministic by construction:
    // every loop below iterates a sorted collection (entities_of_kind,
    // files_in_component, or the sorted (symbol, component) list), never a
    // raw HashMap.
    let mut symbol_list: Vec<(String, String)> = symbol_component
        .iter()
        .map(|(s, c)| (s.clone(), c.clone()))
        .collect();
    symbol_list.sort();
    // (component, store) -> distinct symbols in the component writing it
    let mut shared_writes: BTreeMap<(String, String), HashSet<String>> = BTreeMap::new();
    // component -> HANDLES edges from its symbols (entrypoint ownership)
    let mut entrypoints: BTreeMap<String, usize> = BTreeMap::new();
    // component -> PUBLISHES/CONSUMES edges from its symbols (event ownership)
    let mut events: BTreeMap<String, usize> = BTreeMap::new();
    // component -> (internal calls, total calls) from its symbols
    let mut calls: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for (sym_id, comp) in &symbol_list {
        for r in graph.out_pred(sym_id, scc_core::predicates::WRITES) {
            // data entities (repo://r/data/store.entity) resolve to their
            // owning store so two symbols writing db.users and db.orders
            // count as shared ownership of the same store
            let target = if r.object.contains("/data/") {
                graph
                    .entities
                    .get(&r.object)
                    .and_then(|e| e.attributes.get("store"))
                    .and_then(|v| v.as_str())
                    .map(|s| entity_id(repo_id, kinds::DATA_STORE, s))
                    .unwrap_or_else(|| r.object.clone())
            } else {
                r.object.clone()
            };
            shared_writes
                .entry((comp.clone(), target))
                .or_default()
                .insert(sym_id.clone());
        }
        for _r in graph.out_pred(sym_id, scc_core::predicates::HANDLES) {
            *entrypoints.entry(comp.clone()).or_insert(0) += 1;
        }
        for _r in graph
            .out_pred(sym_id, scc_core::predicates::PUBLISHES)
            .into_iter()
            .chain(graph.out_pred(sym_id, scc_core::predicates::CONSUMES))
        {
            *events.entry(comp.clone()).or_insert(0) += 1;
        }
        for r in graph.out_pred(sym_id, scc_core::predicates::CALLS) {
            let e = calls.entry(comp.clone()).or_insert((0, 0));
            e.1 += 1;
            if symbol_component.get(&r.object) == Some(comp) {
                e.0 += 1;
            }
        }
    }
    // component -> route entities contained in its files (route ownership)
    let mut route_entities: BTreeMap<String, usize> = BTreeMap::new();
    for (comp, files) in &files_in_component {
        for fid in files {
            for r in graph.out_pred(fid, scc_core::predicates::CONTAINS) {
                if graph
                    .entities
                    .get(&r.object)
                    .map(|e| e.kind == kinds::ROUTE)
                    .unwrap_or(false)
                {
                    *route_entities.entry(comp.clone()).or_insert(0) += 1;
                }
            }
        }
    }
    // deployment units with build contexts, most specific (longest context)
    // first so `parent` picks the tightest unit; name tiebreak for
    // determinism
    let mut du_ctxs: Vec<(String, String)> = graph
        .entities_of_kind(kinds::DEPLOYMENT_UNIT)
        .into_iter()
        .filter_map(|du| {
            let ctx = du.attributes.get("build_context").and_then(|v| v.as_str())?;
            if ctx == "." || ctx == "./" {
                return None;
            }
            Some((du.name.clone(), ctx.trim_start_matches("./").to_string()))
        })
        .collect();
    du_ctxs.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
    // component -> deployment unit name whose build context covers its dirs
    let mut parent_per_comp: BTreeMap<String, String> = BTreeMap::new();
    for c in &candidates {
        for (du_name, ctx) in &du_ctxs {
            let inside = c.dirs.iter().any(|d| {
                let d = d.trim_end_matches('/');
                d == ctx.as_str() || d.starts_with(&format!("{ctx}/"))
            });
            if inside {
                parent_per_comp.insert(c.name.clone(), du_name.clone());
                break;
            }
        }
    }

    // intent responsibilities / ownership (DECLARED)
    let mut intent_resp: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut intent_owns: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (source, claim) in intent {
        if source == "component" {
            let name = claim["name"].as_str().unwrap_or("").to_string();
            if let Some(resp) = claim["responsibility"].as_array() {
                for r in resp {
                    if let Some(s) = r.as_str() {
                        intent_resp.entry(name.clone()).or_default().push(s.to_string());
                    }
                }
            }
            if let Some(o) = claim["owns"].as_array() {
                for ow in o {
                    if let Some(s) = ow.as_str() {
                        intent_owns.entry(name.clone()).or_default().push(s.to_string());
                    }
                }
            }
        }
    }

    // ---- build component entities ----
    let mut comp_names: Vec<String> = candidates
        .iter()
        .map(|c| c.name.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    comp_names.sort();

    let mut out: Vec<scc_core::Entity> = Vec::new();
    for name in comp_names {
        let id = entity_id(repo_id, kinds::COMPONENT, &name);
        let mut e = scc_core::Entity::new(id.clone(), kinds::COMPONENT, name.clone());

        let mut resp: Vec<serde_json::Value> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let push_resp = |text: String, prov: Provenance, conf: f64,
                             resp: &mut Vec<serde_json::Value>, seen: &mut HashSet<String>| {
            if !seen.insert(text.clone()) {
                return;
            }
            resp.push(json!({
                "text": text,
                "provenance": prov.as_str(),
                "confidence": conf,
            }));
        };
        if let Some(irs) = intent_resp.get(&name) {
            for s in irs {
                push_resp(s.clone(), Provenance::Declared, 1.0, &mut resp, &mut seen);
            }
        }
        if let Some(rs) = responsibilities.get(&name) {
            let mut sorted = rs.clone();
            sorted.sort_by(|a, b| {
                prov_rank(b.1)
                    .cmp(&prov_rank(a.1))
                    .then_with(|| a.0.cmp(&b.0))
            });
            for (text, prov, conf) in sorted {
                push_resp(text, prov, conf, &mut resp, &mut seen);
            }
        }
        if resp.is_empty() {
            resp.push(json!({
                "text": format!("Hosts the {} code module", name),
                "provenance": Provenance::Inferred.as_str(),
                "confidence": 0.5,
            }));
        }
        e.attr("responsibility", json!(resp));

        let cand = candidates
            .iter()
            .find(|c| c.name == name)
            .expect("every compiled component has a candidate");
        let dirs = cand.dirs.clone();
        e.attr(
            "implementation",
            json!({
                "paths": dirs,
                "symbols": symbols_per_comp.get(&name).cloned().unwrap_or_default(),
            }),
        );

        // ---- Wave 5: boundary kind + weighted clustering score ----
        let mut score: f64 = match cand.boundary_kind.as_str() {
            BOUNDARY_DEPLOYMENT => 5.0,
            BOUNDARY_PACKAGE => 4.0,
            BOUNDARY_CODE_REGION | BOUNDARY_ROOT => 1.0,
            // declared intent carries its authority in `boundary_kind`;
            // the clustering score only counts graph evidence (plan §28)
            _ => 0.0,
        };
        if shared_writes
            .iter()
            .any(|((c, _), syms)| c == &name && syms.len() >= 2)
        {
            score += 4.0; // shared data ownership
        }
        if entrypoints.get(&name).copied().unwrap_or(0) > 0 {
            score += 4.0; // entrypoint ownership (route handlers)
        }
        if route_entities.get(&name).copied().unwrap_or(0) > 0 {
            score += 3.0; // route ownership
        }
        if events.get(&name).copied().unwrap_or(0) > 0 {
            score += 3.0; // event ownership
        }
        if let Some((internal, total)) = calls.get(&name) {
            if *total > 0 {
                score += 3.0 * (*internal as f64 / *total as f64); // cohesion
            }
        }
        let dir_refs: Vec<&str> = dirs.iter().map(|d| d.as_str()).collect();
        let co_pairs = pairs
            .iter()
            .filter(|p| {
                crate::cochange::file_in_paths(&p.a, &dir_refs)
                    && crate::cochange::file_in_paths(&p.b, &dir_refs)
            })
            .count();
        score += 2.0 * co_pairs as f64; // co-change (+2 per pair inside)
        score = (score * 1000.0).round() / 1000.0;
        e.attr("boundary_kind", json!(cand.boundary_kind.clone()));
        e.attr("clustering_score", json!(score));
        if let Some(parent) = parent_per_comp.get(&name) {
            e.attr("parent", json!(parent));
        }

        // typed ownership claims: (target, provenance, confidence, evidence)
        // — intent stays DECLARED, write edges keep their own provenance
        let mut owned_claims: Vec<(String, Provenance, f64, Vec<String>)> = owns
            .get(&name)
            .cloned()
            .unwrap_or_default();
        if let Some(ios) = intent_owns.get(&name) {
            for target in ios {
                let target_l = target.to_ascii_lowercase();
                let matched = graph
                    .entities_of_kind(kinds::DATA_STORE)
                    .into_iter()
                    .chain(graph.entities_of_kind(kinds::DATA_ENTITY))
                    .find(|e| e.name.to_ascii_lowercase() == target_l)
                    .map(|e| e.id.clone());
                if let Some(mid) = matched {
                    owned_claims.push((mid, Provenance::Declared, 1.0, Vec::new()));
                }
            }
        }
        owned_claims.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| prov_rank(b.1).cmp(&prov_rank(a.1)))
        });
        let owned_json: Vec<serde_json::Value> = owned_claims
            .iter()
            .map(|(t, p, c, ev)| {
                json!({
                    "target": t,
                    "provenance": p.as_str(),
                    "confidence": c,
                    "evidence": ev,
                })
            })
            .collect();
        e.attr("owns", json!(owned_json));

        let deps: Vec<serde_json::Value> = depends
            .get(&name)
            .map(|v| {
                let mut sorted = v.clone();
                sorted.sort_by(|a, b| b.3.cmp(&a.3).then_with(|| a.0.cmp(&b.0)));
                sorted
                    .into_iter()
                    .map(|(t, p, c, n)| {
                        json!({"target": t, "provenance": p.as_str(), "confidence": c, "call_count": n})
                    })
                    .collect()
            })
            .unwrap_or_default();
        e.attr("depends_on", json!(deps));
        e.attr(
            "retries",
            json!(retries_per_comp.get(&name).cloned().unwrap_or_default()),
        );

        e.evidence = evidence_per_comp.get(&name).cloned().unwrap_or_default();
        out.push(e);
    }

    // ---- component-level relationships (derived; carry the evidence of
    // the underlying source facts) ----
    clear_component_relationships(store)?;
    let mut rels: Vec<(Relationship, String)> = Vec::new();

    // evidence aggregation helpers over the reality graph
    let sym_evidence_in_file = |fid: &str| -> Vec<String> {
        let mut ev: Vec<String> = Vec::new();
        for r in graph.out_pred(fid, scc_core::predicates::CONTAINS) {
            if let Some(e) = graph.entities.get(&r.object) {
                ev.extend(e.evidence.clone());
            }
        }
        ev
    };
    let write_evidence_to = |store_id: &str| -> Vec<String> {
        // data entities (repo://r/data/store.entity) resolve to their store
        let store_target = if store_id.contains("/data/") {
            graph
                .entities
                .get(store_id)
                .and_then(|e| e.attributes.get("store"))
                .and_then(|v| v.as_str())
                .map(|s| entity_id(repo_id, kinds::DATA_STORE, s))
                .unwrap_or_else(|| store_id.to_string())
        } else {
            store_id.to_string()
        };
        let mut ev: Vec<String> = Vec::new();
        for r in graph.in_pred(&store_target, scc_core::predicates::WRITES) {
            ev.extend(r.evidence.clone());
        }
        ev
    };
    let call_evidence_between = |from_comp: &str, to_comp: &str| -> Vec<String> {
        let mut ev: Vec<String> = Vec::new();
        for (sym_id, comp) in &symbol_component {
            if comp != from_comp {
                continue;
            }
            for r in graph.out_pred(sym_id, scc_core::predicates::CALLS) {
                if let Some(tc) = symbol_component.get(&r.object) {
                    if tc == to_comp {
                        ev.extend(r.evidence.clone());
                    }
                }
            }
        }
        ev
    };

    for e in &out {
        if let Some(files) = files_in_component.get(&e.name) {
            for fid in files {
                rels.push((
                    Relationship::new(
                        rel(&["component_contains", &e.id, fid]),
                        e.id.clone(),
                        scc_core::predicates::CONTAINS,
                        fid.clone(),
                        Provenance::Extracted,
                    )
                    .with_evidence(sym_evidence_in_file(fid)),
                    String::new(),
                ));
            }
        }
        if let Some(owned) = e.attributes.get("owns").and_then(|v| v.as_array()) {
            for o in owned {
                let target = o.get("target").and_then(|v| v.as_str());
                let prov = parse_prov(
                    o.get("provenance")
                        .and_then(|v| v.as_str())
                        .unwrap_or("INFERRED"),
                );
                let conf = o
                    .get("confidence")
                    .and_then(|v| v.as_f64())
                    .unwrap_or_else(|| prov.default_confidence());
                let claim_evidence: Vec<String> = o
                    .get("evidence")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(os) = target {
                    // provenance-preserving: the derived OWNS relationship
                    // carries the claim's own provenance (DECLARED intent
                    // never becomes a resolved ownership fact), and the rel
                    // id includes provenance so conflicting claims coexist
                    let prov_tag = prov.as_str().to_ascii_lowercase();
                    let mut evidence = claim_evidence;
                    if evidence.is_empty() {
                        evidence = write_evidence_to(os);
                    }
                    rels.push((
                        Relationship::new(
                            rel(&["component_owns", &e.id, os, &prov_tag]),
                            e.id.clone(),
                            scc_core::predicates::OWNS,
                            os.to_string(),
                            prov,
                        )
                        .with_confidence(conf)
                        .with_evidence(evidence),
                        String::new(),
                    ));
                }
            }
        }
        if let Some(deps) = e.attributes.get("depends_on").and_then(|v| v.as_array()) {
            for d in deps {
                if let Some(t) = d.get("target").and_then(|v| v.as_str()) {
                    let target_id = entity_id(repo_id, kinds::COMPONENT, t);
                    let prov = parse_prov(
                        d.get("provenance")
                            .and_then(|v| v.as_str())
                            .unwrap_or("INFERRED"),
                    );
                    rels.push((
                        Relationship::new(
                            rel(&["component_depends", &e.id, &target_id]),
                            e.id.clone(),
                            scc_core::predicates::DEPENDS_ON,
                            target_id,
                            prov,
                        )
                        .with_evidence(call_evidence_between(&e.name, t)),
                        String::new(),
                    ));
                }
            }
        }
    }
    for (r, src) in rels {
        store.insert_relationship(&r, &src)?;
    }

    Ok(out)
}

fn clear_component_relationships(store: &Store) -> Result<()> {
    let rows = store.all_relationships()?;
    let ids: Vec<String> = rows
        .into_iter()
        .filter(|r| r.id.starts_with(RELPREFIX))
        .map(|r| r.id)
        .collect();
    for id in ids {
        store.delete_relationship(&id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_ownership_stays_declared() {
        // P0 provenance rule: DECLARED intent ownership must never be
        // promoted to a resolved OWNS relationship by the component compiler.
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();

        // a data store, a file, and a symbol in it that writes the store
        let repo = store.repo_id.clone();
        let store_ent = scc_core::entity_id(&repo, kinds::DATA_STORE, "db");
        store
            .insert_entity(
                &scc_core::Entity::new(store_ent.clone(), kinds::DATA_STORE, "db"),
                &["main.py".into()],
            )
            .unwrap();
        let file = scc_core::entity_id(&repo, kinds::FILE, "main.py");
        store
            .insert_entity(
                &scc_core::Entity::new(file.clone(), kinds::FILE, "main.py"),
                &["main.py".into()],
            )
            .unwrap();
        let sym = scc_core::symbol_id(&repo, "main.py", "save");
        store
            .insert_entity(
                &scc_core::Entity::new(sym.clone(), kinds::SYMBOL, "save"),
                &["main.py".into()],
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:contains",
                    file,
                    scc_core::predicates::CONTAINS,
                    sym.clone(),
                    Provenance::Extracted,
                ),
                "main.py",
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:w",
                    sym.clone(),
                    scc_core::predicates::WRITES,
                    store_ent.clone(),
                    Provenance::Extracted,
                )
                .with_confidence(1.0),
                "main.py",
            )
            .unwrap();

        // intent: root component declares ownership of db too
        let intent = vec![(
            "component".to_string(),
            serde_json::json!({"name": "root", "owns": ["db"]}),
        )];
        let graph = RealityGraph::load(&store).unwrap();
        let comps = compile_components(&graph, &store, &intent, &[]).unwrap();
        let root_comp = comps.iter().find(|c| c.name == "root").unwrap();

        // the owns attribute carries typed claims with provenance
        let claims = root_comp.attributes.get("owns").unwrap().as_array().unwrap();
        assert_eq!(claims.len(), 2, "{claims:?}");
        let declared = claims
            .iter()
            .find(|c| c.get("provenance").and_then(|v| v.as_str()) == Some("DECLARED"))
            .expect("intent claim present");
        assert_eq!(declared["target"].as_str().unwrap(), store_ent);
        let extracted = claims
            .iter()
            .find(|c| c.get("provenance").and_then(|v| v.as_str()) == Some("EXTRACTED"))
            .expect("write-edge claim present");
        assert_eq!(extracted["target"].as_str().unwrap(), store_ent);

        // relationships: DECLARED claim stays DECLARED, never RESOLVED
        let rels = store.all_relationships().unwrap();
        let owns: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::OWNS)
            .collect();
        assert_eq!(owns.len(), 2, "{rels:?}");
        assert!(
            owns.iter().any(|r| r.provenance == Provenance::Declared),
            "declared ownership relationship must exist: {owns:?}"
        );
        assert!(
            !owns.iter().any(|r| r.provenance == Provenance::Resolved),
            "no provenance promotion allowed: {owns:?}"
        );
    }

    #[test]
    fn path_assignment() {
        let cands = vec![
            ComponentCandidate { name: "web".into(), dirs: vec!["src/web".into()], boundary_kind: BOUNDARY_PACKAGE.into() },
            ComponentCandidate { name: "api".into(), dirs: vec!["src/api".into()], boundary_kind: BOUNDARY_DECLARED.into() },
        ];
        assert_eq!(component_for_path("src/api/routes.py", &cands), "api");
        assert_eq!(component_for_path("src/web/app.ts", &cands), "web");
        assert_eq!(component_for_path("src/shared/util.py", &cands), "src");
        assert_eq!(component_for_path("README.md", &cands), "root");
    }

    /// Insert a FILE entity plus CONTAINS edges to its symbols; returns the
    /// file id and the symbol ids.
    fn insert_file_with_symbols(
        store: &Store,
        path: &str,
        symbols: &[&str],
    ) -> (String, Vec<String>) {
        let repo = store.repo_id.clone();
        let file_id = scc_core::entity_id(&repo, kinds::FILE, path);
        store
            .insert_entity(
                &scc_core::Entity::new(file_id.clone(), kinds::FILE, path),
                &[path.into()],
            )
            .unwrap();
        let mut sym_ids = Vec::new();
        for s in symbols {
            let sid = scc_core::symbol_id(&repo, path, s);
            store
                .insert_entity(
                    &scc_core::Entity::new(sid.clone(), kinds::SYMBOL, *s),
                    &[path.into()],
                )
                .unwrap();
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:contains:{}:{s}", path.replace('/', "_")),
                        file_id.clone(),
                        scc_core::predicates::CONTAINS,
                        sid.clone(),
                        Provenance::Extracted,
                    ),
                    path,
                )
                .unwrap();
            sym_ids.push(sid);
        }
        (file_id, sym_ids)
    }

    #[test]
    fn boundary_kind_classification() {
        // Wave 5: every compiled component records the evidence class that
        // created it — declared intent, workspace package, deployment-unit
        // build context, bare top-level directory, or root-level files —
        // while the entity kind stays kinds::COMPONENT.
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let repo = store.repo_id.clone();

        for f in [
            "web/app.py",
            "packages/a/util.py",
            "services/api/main.py",
            "misc/util.py",
            "README.md",
        ] {
            store
                .insert_entity(
                    &scc_core::Entity::new(
                        scc_core::entity_id(&repo, kinds::FILE, f),
                        kinds::FILE,
                        f,
                    ),
                    &[f.into()],
                )
                .unwrap();
        }
        // workspace package member
        let mut pkg = scc_core::Entity::new(
            scc_core::entity_id(&repo, kinds::PACKAGE, "pkg_a"),
            kinds::PACKAGE,
            "pkg_a",
        );
        pkg.attr("path", serde_json::json!("packages/a"));
        store
            .insert_entity(&pkg, &["packages/a/util.py".into()])
            .unwrap();
        // deployment unit with a build context
        let mut du = scc_core::Entity::new(
            scc_core::entity_id(&repo, kinds::DEPLOYMENT_UNIT, "api"),
            kinds::DEPLOYMENT_UNIT,
            "api",
        );
        du.attr("build_context", serde_json::json!("services/api"));
        store
            .insert_entity(&du, &["services/api/main.py".into()])
            .unwrap();

        let intent = vec![(
            "component".to_string(),
            serde_json::json!({"name": "web", "paths": ["web"]}),
        )];
        let graph = RealityGraph::load(&store).unwrap();
        let comps = compile_components(&graph, &store, &intent, &[]).unwrap();
        let by_name: std::collections::BTreeMap<&str, &scc_core::Entity> =
            comps.iter().map(|c| (c.name.as_str(), c)).collect();
        let kind_of = |n: &str| by_name[n].attributes["boundary_kind"].as_str().unwrap();
        assert_eq!(kind_of("web"), BOUNDARY_DECLARED);
        assert_eq!(kind_of("pkg_a"), BOUNDARY_PACKAGE);
        assert_eq!(kind_of("api"), BOUNDARY_DEPLOYMENT);
        assert_eq!(kind_of("misc"), BOUNDARY_CODE_REGION);
        assert_eq!(kind_of("services"), BOUNDARY_CODE_REGION, "dir fallback");
        assert_eq!(kind_of("root"), BOUNDARY_ROOT);
        // entity kind is never renamed; candidates keep their names
        assert_eq!(by_name["api"].kind, kinds::COMPONENT);
        assert!(by_name.contains_key("api"), "candidate name unchanged");
        // components inside a deployment unit carry the additive parent attr
        assert_eq!(by_name["api"].attributes["parent"], serde_json::json!("api"));
        assert!(
            !by_name["web"].attributes.contains_key("parent"),
            "no parent outside a deployment unit"
        );
        // every compiled component carries both new attributes
        for c in &comps {
            assert!(c.attributes.contains_key("boundary_kind"), "{}", c.name);
            assert!(c.attributes.contains_key("clustering_score"), "{}", c.name);
        }
    }

    #[test]
    fn clustering_score_deterministic_and_ranked() {
        // Wave 5 §28 weights: shared data ownership +4, entrypoint
        // ownership +4, route ownership +3, internal call cohesion +3 —
        // and a bare directory fallback scores only +1.
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let repo = store.repo_id.clone();

        let (_f1, api_syms) =
            insert_file_with_symbols(&store, "api/routes.py", &["handle_a", "handle_b"]);
        let (_f2, api_helpers) = insert_file_with_symbols(&store, "api/helpers.py", &["helper"]);
        let (_f3, _web_syms) = insert_file_with_symbols(&store, "web/app.py", &["web_index"]);

        let store_ent = scc_core::entity_id(&repo, kinds::DATA_STORE, "db");
        store
            .insert_entity(
                &scc_core::Entity::new(store_ent.clone(), kinds::DATA_STORE, "db"),
                &["api/routes.py".into()],
            )
            .unwrap();
        let routes_file = scc_core::entity_id(&repo, kinds::FILE, "api/routes.py");
        for (i, sym) in ["handle_a", "handle_b"].iter().enumerate() {
            let route = scc_core::entity_id(&repo, kinds::ROUTE, &format!("GET /api/{i}"));
            store
                .insert_entity(
                    scc_core::Entity::new(route.clone(), kinds::ROUTE, format!("GET /api/{i}"))
                        .attr("method", serde_json::json!("GET"))
                        .attr("path", serde_json::json!(format!("/api/{i}"))),
                    &["api/routes.py".into()],
                )
                .unwrap();
            // the route entity lives in the candidate's file (route ownership)
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:route_contains_{i}"),
                        routes_file.clone(),
                        scc_core::predicates::CONTAINS,
                        route.clone(),
                        Provenance::Extracted,
                    ),
                    "api/routes.py",
                )
                .unwrap();
            let sym_id = scc_core::symbol_id(&repo, "api/routes.py", sym);
            // the handler symbol owns the route (entrypoint ownership)
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:handles_{i}"),
                        sym_id.clone(),
                        scc_core::predicates::HANDLES,
                        route.clone(),
                        Provenance::Extracted,
                    ),
                    "api/routes.py",
                )
                .unwrap();
            // two distinct symbols write the same store (shared data ownership)
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:writes_{i}"),
                        sym_id,
                        scc_core::predicates::WRITES,
                        store_ent.clone(),
                        Provenance::Extracted,
                    ),
                    "api/routes.py",
                )
                .unwrap();
        }
        // internal call cohesion: handle_a -> helper, both inside api
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:call_internal",
                    api_syms[0].clone(),
                    scc_core::predicates::CALLS,
                    api_helpers[0].clone(),
                    Provenance::Extracted,
                ),
                "api/routes.py",
            )
            .unwrap();

        let intent = vec![(
            "component".to_string(),
            serde_json::json!({"name": "api", "paths": ["api"]}),
        )];

        let graph = RealityGraph::load(&store).unwrap();
        let comps = compile_components(&graph, &store, &intent, &[]).unwrap();
        let graph2 = RealityGraph::load(&store).unwrap();
        let comps2 = compile_components(&graph2, &store, &intent, &[]).unwrap();
        let score = |c: &scc_core::Entity| c.attributes["clustering_score"].as_f64().unwrap();
        for (a, b) in comps.iter().zip(comps2.iter()) {
            assert_eq!(
                a.attributes["clustering_score"],
                b.attributes["clustering_score"],
                "scores must be deterministic for {}",
                a.name
            );
        }
        let api = comps.iter().find(|c| c.name == "api").unwrap();
        let web = comps.iter().find(|c| c.name == "web").unwrap();
        assert_eq!(score(api), 14.0, "{:?}", api.attributes);
        assert_eq!(score(web), 1.0, "bare directory: +1 only");
        assert!(score(api) > score(web), "evidence-rich candidate outranks a bare dir");
        assert_eq!(api.attributes["boundary_kind"], serde_json::json!("declared"));
        assert_eq!(web.attributes["boundary_kind"], serde_json::json!("code-region"));
    }
}
