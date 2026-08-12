//! Trust-boundary compiler (SCC-148): derives `crosses_boundary`
//! relationships from deployment units, component dependencies, and calls
//! to external APIs.
//!
//! Model:
//! - Deployment units (entities kind `deployment_unit`) with a
//!   `build_context` attribute map to that directory; units with only an
//!   `image` attribute map to their own name (no directory is recorded, so
//!   the unit name is the best deterministic stand-in). Units with neither
//!   attribute (e.g. pure Dockerfile units) are ignored.
//! - A component (from `store.components()`) belongs to the unit whose
//!   directory is the longest prefix match against any of the component's
//!   `implementation.paths`. Components matching no unit belong to the
//!   synthetic unit `local`.
//! - Every RESOLVED `depends_on` edge between components in different units,
//!   and every `calls` edge into an `external_api`, becomes a
//!   `crosses_boundary` relationship carrying the evidence of the underlying
//!   fact. Ids are content-derived (blake3, `rel:boundary:` prefix) and the
//!   derived set is replaced wholesale on each compile, so the output is
//!   deterministic and idempotent.

use crate::{RealityGraph, Result};
use scc_core::kinds;
use scc_core::{Entity, Provenance, Relationship};
use scc_store::Store;

pub const RELPREFIX: &str = "rel:boundary:";

fn rel_id(parts: &[&str]) -> String {
    let mut h = blake3::Hasher::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update(b"|");
    }
    format!("{RELPREFIX}{}", &h.finalize().to_hex()[..12])
}

/// `(unit name, directory)` pairs for every deployment unit that maps to a
/// directory. `build_context` wins; image-only units fall back to their name;
/// `"."`/empty build contexts are skipped (they would match everything).
fn unit_dirs(graph: &RealityGraph) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for e in graph.entities_of_kind(kinds::DEPLOYMENT_UNIT) {
        let dir = if let Some(ctx) = e.attributes.get("build_context").and_then(|v| v.as_str()) {
            let ctx = ctx.trim().trim_start_matches("./");
            if ctx.is_empty() || ctx == "." {
                continue;
            }
            ctx.to_string()
        } else if e.attributes.contains_key("image") {
            e.name.clone()
        } else {
            continue;
        };
        out.push((e.name.clone(), dir));
    }
    out
}

/// Unit owning the component whose `implementation.paths` best match `path`
/// (longest directory prefix wins), or `"local"` when nothing matches.
fn unit_for_component(comp: &Entity, units: &[(String, String)]) -> String {
    let mut best: Option<(String, usize)> = None;
    if let Some(paths) = comp
        .attributes
        .get("implementation")
        .and_then(|v| v.get("paths"))
        .and_then(|v| v.as_array())
    {
        for p in paths {
            if let Some(p) = p.as_str() {
                for (name, dir) in units {
                    if (p == dir || p.starts_with(&format!("{dir}/")))
                        && best.as_ref().map(|(_, l)| dir.len() > *l).unwrap_or(true)
                    {
                        best = Some((name.clone(), dir.len()));
                    }
                }
            }
        }
    }
    best.map(|(n, _)| n).unwrap_or_else(|| "local".to_string())
}

/// Compile the full set of trust-boundary crossings for the current reality
/// graph. Returns `(relationship, source_path)` pairs ready for
/// `store.insert_relationship`; the source path is empty (derived facts).
/// Replaces any previously compiled crossings (stale edges from removed
/// dependencies or calls do not survive a rebuild).
pub fn compile_boundaries(graph: &RealityGraph, store: &Store) -> Result<Vec<(Relationship, String)>> {
    // drop the previous derived set so removed edges don't linger
    let stale: Vec<String> = store
        .all_relationships()?
        .into_iter()
        .filter(|r| r.id.starts_with(RELPREFIX))
        .map(|r| r.id)
        .collect();
    for id in stale {
        store.delete_relationship(&id)?;
    }

    let units = unit_dirs(graph);
    let mut crossings: Vec<Relationship> = Vec::new();
    for r in store.all_relationships()? {
        if r.predicate == scc_core::predicates::DEPENDS_ON
            && r.provenance == Provenance::Resolved
        {
            // component dependency crossing a unit boundary
            let (subj, obj) = match (graph.entity(&r.subject), graph.entity(&r.object)) {
                (Some(s), Some(o)) => (s, o),
                _ => continue,
            };
            if subj.kind != kinds::COMPONENT || obj.kind != kinds::COMPONENT {
                continue;
            }
            if unit_for_component(subj, &units) == unit_for_component(obj, &units) {
                continue;
            }
            crossings.push(
                Relationship::new(
                    rel_id(&["crosses_boundary", &r.subject, &r.object]),
                    r.subject.clone(),
                    scc_core::predicates::CROSSES_BOUNDARY,
                    r.object.clone(),
                    Provenance::Extracted,
                )
                .with_confidence(0.9)
                .with_evidence(r.evidence.clone()),
            );
        } else if r.predicate == scc_core::predicates::CALLS
            && r.object.contains("/external_api/")
        {
            // call from a symbol into an external API leaves the unit
            crossings.push(
                Relationship::new(
                    rel_id(&["external_crossing", &r.subject, &r.object]),
                    r.subject.clone(),
                    scc_core::predicates::CROSSES_BOUNDARY,
                    r.object.clone(),
                    Provenance::Extracted,
                )
                .with_confidence(0.9)
                .with_evidence(r.evidence.clone()),
            );
        }
    }

    crossings.sort_by(|a, b| a.subject.cmp(&b.subject).then_with(|| a.object.cmp(&b.object)));
    Ok(crossings.into_iter().map(|r| (r, String::new())).collect())
}

/// Human-readable, sorted list of boundary crossings in the form
/// `"unitA/compA -> unitB/compB"` (external crossings read
/// `"unit/comp -> external/name"`), for `verify`/CLI/Atlas display.
///
/// P0 correctness: this is a PURE READ of the STORED `CROSSES_BOUNDARY`
/// relationships (inserted by `compile_boundaries` during recompile). It
/// never mutates the database — context generation must be side-effect free.
/// An atlas/verify run on an un-recompiled store shows the last compiled
/// crossings; run `scc index` (or the pipeline) to refresh them.
pub fn boundary_crossings(graph: &RealityGraph, store: &Store) -> Result<Vec<String>> {
    let units = unit_dirs(graph);
    let comps = store.components()?;
    let mut lines: Vec<String> = Vec::new();
    for rel in store.all_relationships()? {
        if rel.predicate != scc_core::predicates::CROSSES_BOUNDARY {
            continue;
        }
        let Some(subj) = graph.entity(&rel.subject) else {
            continue;
        };
        let obj_name = graph
            .entity(&rel.object)
            .map(|o| o.name.clone())
            .unwrap_or_else(|| {
                rel.object.rsplit('/').next().unwrap_or(&rel.object).to_string()
            });
        match subj.kind.as_str() {
            kinds::COMPONENT => {
                let ua = unit_for_component(subj, &units);
                let ub = unit_for_component(
                    graph.entity(&rel.object).unwrap_or(subj),
                    &units,
                );
                lines.push(format!("{ua}/{} -> {ub}/{obj_name}", subj.name));
            }
            kinds::SYMBOL => {
                let (unit, owner) = component_of_symbol(graph, &rel.subject, &comps)
                    .map(|c| (unit_for_component(c, &units), c.name.clone()))
                    .unwrap_or_else(|| ("local".to_string(), subj.name.clone()));
                lines.push(format!("{unit}/{owner} -> external/{obj_name}"));
            }
            _ => {}
        }
    }
    lines.sort();
    lines.dedup();
    Ok(lines)
}

/// The component whose `implementation.paths` best match the file attribute
/// of symbol `sym_id` (longest directory prefix wins).
fn component_of_symbol<'a>(
    graph: &RealityGraph,
    sym_id: &str,
    comps: &'a [Entity],
) -> Option<&'a Entity> {
    let file = graph.entity(sym_id)?.attributes.get("file")?.as_str()?;
    let mut best: Option<(&'a Entity, usize)> = None;
    for c in comps {
        if let Some(paths) = c
            .attributes
            .get("implementation")
            .and_then(|v| v.get("paths"))
            .and_then(|v| v.as_array())
        {
            for p in paths {
                if let Some(p) = p.as_str() {
                    if file == p || file.starts_with(&format!("{p}/")) {
                        let len = p.len();
                        if best.map(|(_, l)| len > l).unwrap_or(true) {
                            best = Some((c, len));
                        }
                    }
                }
            }
        }
    }
    best.map(|(c, _)| c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scc_core::{entity_id, symbol_id};

    fn store_with() -> (Store, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&tmp.path().join("scc.db"), &root).unwrap();
        (store, tmp)
    }

    fn unit(store: &Store, name: &str, dir: &str) {
        let mut e = Entity::new(
            entity_id(&store.repo_id, kinds::DEPLOYMENT_UNIT, name),
            kinds::DEPLOYMENT_UNIT,
            name,
        );
        e.attr("build_context", serde_json::json!(dir));
        store.insert_entity(&e, &[]).unwrap();
    }

    fn component(store: &Store, name: &str, paths: &[&str]) -> Entity {
        let mut e = Entity::new(
            entity_id(&store.repo_id, kinds::COMPONENT, name),
            kinds::COMPONENT,
            name,
        );
        e.attr(
            "implementation",
            serde_json::json!({ "paths": paths, "symbols": [] }),
        );
        e
    }

    fn dep(store: &Store, from: &Entity, to: &Entity, prov: Provenance) {
        let r = Relationship::new(
            format!("rel:test:{}:{}", from.name, to.name),
            from.id.clone(),
            scc_core::predicates::DEPENDS_ON,
            to.id.clone(),
            prov,
        )
        .with_evidence(vec!["evidence:test1".to_string()]);
        store.insert_relationship(&r, "").unwrap();
    }

    /// Units web (`services/web`) and api (`services/api`); components web,
    /// web-worker (both in unit web), api, util (unassigned -> local).
    /// Dependencies: web->api (cross), web->web-worker (same unit),
    /// web-worker->util (cross), api->util (cross), plus a non-resolved
    /// web->util edge that must be ignored.
    fn setup_basic(store: &Store) -> Vec<Entity> {
        unit(store, "web", "services/web");
        unit(store, "api", "services/api");
        let web = component(store, "web", &["services/web"]);
        let web_worker = component(store, "web-worker", &["services/web/worker"]);
        let api = component(store, "api", &["services/api"]);
        let util = component(store, "util", &["shared"]);
        let comps = vec![web.clone(), web_worker.clone(), api.clone(), util.clone()];
        store.replace_components(&comps).unwrap();
        dep(store, &web, &api, Provenance::Resolved);
        dep(store, &web, &web_worker, Provenance::Resolved);
        dep(store, &web_worker, &util, Provenance::Resolved);
        dep(store, &api, &util, Provenance::Resolved);
        dep(store, &web, &util, Provenance::Extracted);
        comps
    }

    #[test]
    fn display_does_not_mutate_the_store() {
        // P0 regression: boundary_crossings() is a pure read. Calling it
        // (atlas/verify) must not delete or add relationships.
        let (store, _t) = store_with();
        let _comps = setup_basic(&store);
        let graph = RealityGraph::load(&store).unwrap();

        let before = store.all_relationships().unwrap().len();
        let lines = boundary_crossings(&graph, &store).unwrap();
        let after = store.all_relationships().unwrap().len();
        assert_eq!(before, after, "display must not mutate the store");
        // the display shows the STORED crossings — none until the pipeline
        // compiles them, so an un-recompiled store renders empty
        assert!(lines.is_empty(), "{lines:?}");

        // after the pipeline compiles, the display renders them without
        // touching the database again
        crate::recompile(&store).unwrap();
        let after_compile = store.all_relationships().unwrap().len();
        assert!(after_compile > before, "compile inserts crossings");
        let before2 = store.all_relationships().unwrap().len();
        let lines2 = boundary_crossings(&graph, &store).unwrap();
        let after2 = store.all_relationships().unwrap().len();
        assert_eq!(before2, after2, "display must not mutate the store (2)");
        assert!(!lines2.is_empty(), "compiled crossings render: {lines2:?}");
    }

    #[test]
    fn crossings_only_across_units() {
        let (store, _t) = store_with();
        let comps = setup_basic(&store);
        let graph = RealityGraph::load(&store).unwrap();
        let out = compile_boundaries(&graph, &store).unwrap();
        assert_eq!(out.len(), 3, "web->api, web-worker->util, api->util");

        for (rel, src) in &out {
            assert_eq!(rel.predicate, scc_core::predicates::CROSSES_BOUNDARY);
            assert_eq!(rel.provenance, Provenance::Extracted);
            assert_eq!(rel.confidence, 0.9);
            assert_eq!(rel.evidence, vec!["evidence:test1".to_string()]);
            assert!(src.is_empty(), "derived facts carry no source path");
        }

        let find = |name: &str| comps.iter().find(|c| c.name == name).unwrap();
        let web = find("web");
        let web_worker = find("web-worker");
        let api = find("api");
        let util = find("util");

        // same-unit edge and non-resolved edge produce no crossing
        assert!(!out.iter().any(|(r, _)| r.subject == web.id && r.object == web_worker.id));
        assert!(!out.iter().any(|(r, _)| r.subject == web.id && r.object == util.id));
        // cross-unit edges are all present
        assert!(out.iter().any(|(r, _)| r.subject == web.id && r.object == api.id));
        assert!(out.iter().any(|(r, _)| r.subject == web_worker.id && r.object == util.id));
        assert!(out.iter().any(|(r, _)| r.subject == api.id && r.object == util.id));

        // idempotent: recompiling replaces, never duplicates
        let out2 = compile_boundaries(&graph, &store).unwrap();
        assert_eq!(out2.len(), 3);
    }

    #[test]
    fn external_api_call_crosses_boundary() {
        let (store, _t) = store_with();
        unit(&store, "api", "services/api");
        let api_comp = component(&store, "api", &["services/api"]);
        store.replace_components(&[api_comp]).unwrap();

        let sym = symbol_id(&store.repo_id, "services/api/app.py", "main");
        let ext = entity_id(&store.repo_id, kinds::EXTERNAL_API, "stripe.api");
        let mut se = Entity::new(sym.clone(), kinds::SYMBOL, "main");
        se.attr("file", serde_json::json!("services/api/app.py"));
        store.insert_entity(&se, &[]).unwrap();
        store
            .insert_entity(&Entity::new(ext.clone(), kinds::EXTERNAL_API, "stripe.api"), &[])
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:test:call".to_string(),
                    sym.clone(),
                    scc_core::predicates::CALLS,
                    ext.clone(),
                    Provenance::Resolved,
                )
                .with_evidence(vec!["evidence:call1".to_string()]),
                "services/api/app.py",
            )
            .unwrap();

        let graph = RealityGraph::load(&store).unwrap();
        let out = compile_boundaries(&graph, &store).unwrap();
        assert_eq!(out.len(), 1);
        let (rel, src) = &out[0];
        assert_eq!(rel.subject, sym);
        assert_eq!(rel.object, ext);
        assert_eq!(rel.predicate, scc_core::predicates::CROSSES_BOUNDARY);
        assert_eq!(rel.provenance, Provenance::Extracted);
        assert_eq!(rel.confidence, 0.9);
        assert_eq!(rel.evidence, vec!["evidence:call1".to_string()]);
        assert!(src.is_empty());
    }

    #[test]
    fn no_crossings_within_unit() {
        let (store, _t) = store_with();
        unit(&store, "svc", "src/svc");
        let a = component(&store, "a", &["src/svc/a"]);
        let b = component(&store, "b", &["src/svc/b"]);
        store.replace_components(&[a.clone(), b.clone()]).unwrap();
        dep(&store, &a, &b, Provenance::Resolved);
        let graph = RealityGraph::load(&store).unwrap();
        let out = compile_boundaries(&graph, &store).unwrap();
        assert!(out.is_empty(), "same deployment unit: no crossing");
        let lines = boundary_crossings(&graph, &store).unwrap();
        assert!(lines.is_empty());
    }

    #[test]
    fn boundary_crossings_strings() {
        let (store, _t) = store_with();
        setup_basic(&store);

        let sym = symbol_id(&store.repo_id, "services/api/app.py", "main");
        let ext = entity_id(&store.repo_id, kinds::EXTERNAL_API, "stripe.api");
        let mut se = Entity::new(sym.clone(), kinds::SYMBOL, "main");
        se.attr("file", serde_json::json!("services/api/app.py"));
        store.insert_entity(&se, &[]).unwrap();
        store
            .insert_entity(&Entity::new(ext.clone(), kinds::EXTERNAL_API, "stripe.api"), &[])
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:test:call".to_string(),
                    sym,
                    scc_core::predicates::CALLS,
                    ext,
                    Provenance::Resolved,
                ),
                "services/api/app.py",
            )
            .unwrap();

        let graph = RealityGraph::load(&store).unwrap();
        // display is a pure read of stored crossings: compile first
        let crossings = compile_boundaries(&graph, &store).unwrap();
        for (rel, src) in crossings {
            store.insert_relationship(&rel, &src).unwrap();
        }
        let lines = boundary_crossings(&graph, &store).unwrap();
        assert_eq!(
            lines,
            vec![
                "api/api -> external/stripe.api".to_string(),
                "api/api -> local/util".to_string(),
                "web/web -> api/api".to_string(),
                "web/web-worker -> local/util".to_string(),
            ]
        );
    }
}
