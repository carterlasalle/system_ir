//! Impact analysis (docs/API_AND_INTEGRATIONS.md §2 `impact_context`).
//!
//! Given files/symbols/diff, determine: affected components, flows,
//! upstream/downstream consumers, contracts (routes), data, invariants,
//! tests, and a risk assessment.

use crate::components::component_for_path;
use crate::{trust::TrustedGraphView, Result};
use scc_core::kinds;
use scc_core::Severity;
use scc_store::Store;
use std::collections::{BTreeSet, HashSet};

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Impact {
    pub files: Vec<String>,
    pub components: Vec<String>,          // component ids
    pub flows: Vec<String>,               // flow ids
    pub upstream: Vec<String>,            // component ids that depend on affected
    pub downstream: Vec<String>,          // component ids affected depends on
    pub contracts: Vec<String>,           // route ids
    pub data: Vec<String>,                // store/data entity ids
    pub invariants: Vec<String>,          // invariant ids
    pub tests: Vec<String>,               // test entity ids
    pub risk: String,                     // low | medium | high
    #[serde(default)]
    pub notes: Vec<String>,
}

pub fn compute_impact(
    view: &TrustedGraphView,
    store: &Store,
    files: &[String],
    symbols: &[String],
) -> Result<Impact> {
    let graph = &view.graph;
    let mut imp = Impact::default();

    let file_ids: HashSet<String> = files
        .iter()
        .map(|f| scc_core::entity_id(&graph.repo_id, kinds::FILE, f))
        .collect();
    let sym_ids: HashSet<String> = symbols
        .iter()
        .map(|s| scc_core::symbol_id(&graph.repo_id, "?", s))
        .collect();
    // symbols may be given as plain names — resolve against the index
    let mut resolved_sym_ids: HashSet<String> = HashSet::new();
    for s in symbols {
        let matches: Vec<String> = graph
            .entities_of_kind(kinds::SYMBOL)
            .into_iter()
            .filter(|e| e.name == *s)
            .map(|e| e.id.clone())
            .collect();
        if matches.is_empty() {
            // exact entity id?
            if view.entity(s).is_some() {
                resolved_sym_ids.insert(s.clone());
            }
        } else {
            resolved_sym_ids.extend(matches);
        }
    }
    for id in &sym_ids {
        // symbol_id with "?" file is a miss; resolved ones are real
        if !id.ends_with("/?/") && view.entity(id).is_some() {
            resolved_sym_ids.insert(id.clone());
        }
    }
    // file symbol ids: all symbols whose file attribute is one of the files
    for e in view.entities_of_kind(kinds::SYMBOL) {
        if let Some(f) = e.attributes.get("file").and_then(|v| v.as_str()) {
            if file_ids.contains(&scc_core::entity_id(&graph.repo_id, kinds::FILE, f)) {
                resolved_sym_ids.insert(e.id.clone());
            }
        }
    }

    // affected components: components containing affected files or symbols
    let mut affected_comps: BTreeSet<String> = BTreeSet::new();
    let comps = store.components()?;
    for c in &comps {
        let paths: Vec<String> = c
            .attributes
            .get("implementation")
            .and_then(|i| i.get("paths"))
            .and_then(|p| p.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let symbols_list: Vec<String> = c
            .attributes
            .get("implementation")
            .and_then(|i| i.get("symbols"))
            .and_then(|p| p.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        for f in files {
            let seg = component_for_path(f, &component_candidates(&comps));
            if seg == c.name {
                affected_comps.insert(c.id.clone());
            }
        }
        for s in &symbols_list {
            let resolved: HashSet<&str> = resolved_sym_ids.iter().map(|x| x.as_str()).collect();
            if resolved.is_empty() {
                continue;
            }
            let sym_names: HashSet<String> = graph
                .entities_of_kind(kinds::SYMBOL)
                .into_iter()
                .filter(|e| resolved.contains(e.id.as_str()))
                .map(|e| e.name.clone())
                .collect();
            if sym_names.contains(s) {
                affected_comps.insert(c.id.clone());
            }
        }
        let _ = paths;
    }
    // also via contains relationships
    for c in &comps {
        for r in view.out_pred(&c.id, scc_core::predicates::CONTAINS) {
            if file_ids.contains(&r.object) {
                affected_comps.insert(c.id.clone());
            }
        }
    }

    imp.components = affected_comps.iter().cloned().collect();

    // flows containing affected components or their symbols
    let mut affected_syms: HashSet<String> = HashSet::new();
    for cid in &affected_comps {
        for r in view.out_pred(cid, scc_core::predicates::CONTAINS) {
            // file ids, expand to symbols
            for sr in view.out_pred(&r.object, scc_core::predicates::CONTAINS) {
                affected_syms.insert(sr.object.clone());
            }
        }
    }
    affected_syms.extend(resolved_sym_ids);

    for flow in &view.flows() {
        let steps_mention = flow.steps.iter().any(|s| {
            affected_comps.iter().any(|c| s.actor.contains(c))
                || affected_syms.iter().any(|sid| s.operation.contains(sid))
                || files.iter().any(|f| s.actor.contains(f))
        });
        if steps_mention {
            imp.flows.push(flow.id.clone());
        }
    }
    // entrypoint attribute on flows
    for flow in &view.flows() {
        if let Some(ep) = flow.attributes.get("entrypoint").and_then(|v| v.as_str()) {
            if affected_syms.contains(ep) && !imp.flows.contains(&flow.id) {
                imp.flows.push(flow.id.clone());
            }
        }
    }

    // upstream (depend on affected) / downstream (affected depends on)
    for cid in &affected_comps {
        for r in view.out_pred(cid, scc_core::predicates::DEPENDS_ON) {
            imp.downstream.push(r.object.clone());
        }
        for r in view.in_pred(cid, scc_core::predicates::DEPENDS_ON) {
            imp.upstream.push(r.subject.clone());
        }
    }
    imp.upstream.sort();
    imp.upstream.dedup();
    imp.downstream.sort();
    imp.downstream.dedup();

    // contracts: routes handled by affected symbols
    for sid in &affected_syms {
        for r in view.out_pred(sid, scc_core::predicates::HANDLES) {
            imp.contracts.push(r.object.clone());
        }
    }

    // data: stores owned by affected components + accessed by affected symbols
    for cid in &affected_comps {
        for r in view.out_pred(cid, scc_core::predicates::OWNS) {
            imp.data.push(r.object.clone());
        }
    }
    for sid in &affected_syms {
        for pred in ["reads", "writes", "queries"] {
            for r in view.out_pred(sid, pred) {
                imp.data.push(r.object.clone());
            }
        }
    }
    imp.data.sort();
    imp.data.dedup();

    // invariants whose scope intersects affected entities
    for inv in &view.invariants() {
        let scoped = inv.scope.iter().any(|s| {
            affected_comps.contains(s) || imp.data.contains(s)
        });
        if scoped {
            imp.invariants.push(inv.id.clone());
        }
    }

    // tests covering affected symbols
    for sid in &affected_syms {
        for r in view.out_pred(sid, scc_core::predicates::TESTED_BY) {
            imp.tests.push(r.object.clone());
        }
    }
    imp.tests.sort();
    imp.tests.dedup();

    imp.files = files.to_vec();

    // risk: high if critical invariants affected or contracts changed;
    // medium if flows affected; else low
    let critical_invariants = imp.invariants.iter().filter(|iid| {
        graph
            .invariants
            .iter()
            .find(|i| i.id == **iid)
            .map(|i| i.severity == Severity::Critical)
            .unwrap_or(false)
    }).count();
    if critical_invariants > 0 || !imp.contracts.is_empty() {
        imp.risk = "high".into();
        if critical_invariants > 0 {
            imp.notes.push(format!(
                "{critical_invariants} critical invariant(s) in scope of the change"
            ));
        }
        if !imp.contracts.is_empty() {
            imp.notes.push(format!(
                "{} API contract(s) (routes) affected — consumers may break",
                imp.contracts.len()
            ));
        }
    } else if !imp.flows.is_empty() || !imp.tests.is_empty() {
        imp.risk = "medium".into();
    } else {
        imp.risk = "low".into();
    }
    if !imp.tests.is_empty() {
        imp.notes.push(format!("{} test(s) exercise the affected code", imp.tests.len()));
    }

    Ok(imp)
}

fn component_candidates(comps: &[scc_core::Entity]) -> Vec<crate::components::ComponentCandidate> {
    comps
        .iter()
        .map(|c| {
            let mut dirs: Vec<String> = c
                .attributes
                .get("implementation")
                .and_then(|i| i.get("paths"))
                .and_then(|p| p.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            if dirs.is_empty() {
                dirs.push(c.name.clone());
            }
            crate::components::ComponentCandidate {
                name: c.name.clone(),
                dirs,
                boundary_kind: c
                    .attributes
                    .get("boundary_kind")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| crate::components::BOUNDARY_CODE_REGION.to_string()),
            }
        })
        .collect()
}

/// Files/symbols in the current diff (git diff --name-only).
pub fn diff_files(store: &Store, base: Option<&str>) -> Result<Vec<String>> {
    let root = &store.root;
    let mut cmd = std::process::Command::new("git");
    cmd.args(["diff", "--name-only", "--diff-filter=ACMRT"]);
    if let Some(b) = base {
        cmd.arg(format!("{b}...HEAD"));
    } else {
        cmd.arg("HEAD");
    }
    cmd.arg("--");
    let out = cmd.current_dir(root).output().map_err(|e| {
        scc_store::StoreError::NotInitialized(format!("git diff failed: {e}"))
    })?;
    let mut files = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let line = line.trim();
        if !line.is_empty() {
            files.push(line.to_string());
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_impact() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let g = crate::RealityGraph::load(&store).unwrap();
        let v = TrustedGraphView::new(&g, &store, &[], crate::TrustPolicy::default());
        let imp = compute_impact(&v, &store, &[], &[]).unwrap();
        assert!(imp.components.is_empty());
        assert_eq!(imp.risk, "low");
    }
}
