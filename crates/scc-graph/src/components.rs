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
            candidates.push(ComponentCandidate { name, dirs });
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
            } else {
                candidates.push(ComponentCandidate {
                    name,
                    dirs: vec![path.to_string()],
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
            } else {
                candidates.push(ComponentCandidate {
                    name,
                    dirs: vec![ctx.to_string()],
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

        let dirs = candidates
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.dirs.clone())
            .unwrap_or_default();
        e.attr(
            "implementation",
            json!({
                "paths": dirs,
                "symbols": symbols_per_comp.get(&name).cloned().unwrap_or_default(),
            }),
        );

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
        let comps = compile_components(&graph, &store, &intent).unwrap();
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
            ComponentCandidate { name: "web".into(), dirs: vec!["src/web".into()] },
            ComponentCandidate { name: "api".into(), dirs: vec!["src/api".into()] },
        ];
        assert_eq!(component_for_path("src/api/routes.py", &cands), "api");
        assert_eq!(component_for_path("src/web/app.ts", &cands), "web");
        assert_eq!(component_for_path("src/shared/util.py", &cands), "src");
        assert_eq!(component_for_path("README.md", &cands), "root");
    }
}
