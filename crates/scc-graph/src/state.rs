//! STATE & DATA AUTHORITY (Ontology phase): deterministic attribution of
//! state ownership per component from the fact layer.
//!
//! Six subsections:
//! - `persistent`: stores/data entities via WRITES edges (the existing
//!   component `owns` claims).
//! - `runtime`: FIELD facts with `mutable=true`, STATE entities, REGISTRY
//!   entities.
//! - `reactive`: REACTIVE entities (svelte/vue/react/mobx/signals state)
//!   via OWNS edges.
//! - `configuration`: CONFIGURED_BY relationships (Configuration facts).
//! - `caches`: WRITES/READS to cache-technology stores.
//! - `derived`: PUBLISHES/SUBSCRIBES/CONSUMES topics + middleware/registry
//!   registrations.
//!
//! Every line is `COMPONENT <verb> TARGET (PROV)`-style, deterministically
//! sorted. Provenance is preserved verbatim from the underlying
//! relationship — nothing is promoted.

use crate::RealityGraph;
use scc_core::{kinds, predicates, Provenance};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const S_PERSISTENT: &str = "persistent";
pub const S_RUNTIME: &str = "runtime";
pub const S_REACTIVE: &str = "reactive";
pub const S_CONFIGURATION: &str = "configuration";
pub const S_CACHES: &str = "caches";
pub const S_DERIVED: &str = "derived";

/// Deterministic render order of the STATE & DATA AUTHORITY subsections.
pub const STATE_SECTIONS: [&str; 6] = [
    S_PERSISTENT,
    S_RUNTIME,
    S_REACTIVE,
    S_CONFIGURATION,
    S_CACHES,
    S_DERIVED,
];

/// Human-readable subsection header for a section key.
// # trace:exempt — subsection header label map, no behavior of its own
pub fn section_label(section: &str) -> &'static str {
    match section {
        S_PERSISTENT => "DATA OWNERSHIP",
        S_RUNTIME => "RUNTIME STATE",
        S_REACTIVE => "REACTIVE STATE",
        S_CONFIGURATION => "CONFIGURATION",
        S_CACHES => "CACHES",
        S_DERIVED => "DERIVED / REGISTRIES",
        _ => "STATE",
    }
}

/// Cache-technology store detection (store_refs with cache tech): an
/// explicit technology hint or a cache-ish store name.
pub fn is_cache_store(name: &str, technology: Option<&str>) -> bool {
    if let Some(t) = technology {
        let t = t.to_ascii_lowercase();
        if matches!(t.as_str(), "redis" | "memcached" | "valkey") {
            return true;
        }
    }
    let n = name.to_ascii_lowercase();
    n.contains("cache") || n.contains("redis") || n.contains("memcache")
}

/// Attribute state ownership per component from the fact layer.
///
/// `symbol_comp` maps symbol entity id -> component name (the component
/// compiler builds it from the *current* candidate boundaries, so the
/// result never depends on stale stored components).
///
/// Returns `section -> sorted "COMP verb TARGET (PROV)" lines`. Section
/// keys are [`STATE_SECTIONS`]; the map is a `BTreeMap` and every line set
/// is sorted — output is deterministic for identical input.
// trace:v1 id=impl.scc.state.authority work=WORK-SCC-005 satisfies=REQ-SCC-IR
pub fn compile_state_authority(
    graph: &RealityGraph,
    symbol_comp: &HashMap<String, String>,
) -> BTreeMap<String, Vec<String>> {
    let mut sections: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    // every section key exists (empty sections stay empty) so callers can
    // iterate STATE_SECTIONS unconditionally
    for k in STATE_SECTIONS {
        sections.entry(k.to_string()).or_default();
    }
    let mut push = |section: &str, line: String| {
        sections
            .entry(section.to_string())
            .or_default()
            .insert(line);
    };

    let prov_str = |p: &Provenance| p.as_str().to_string();

    // symbol id -> entity (for target resolution)
    let comp_of = |sym_id: &str| symbol_comp.get(sym_id).cloned();

    // ---- per-symbol edges (persistent / caches / derived) ----
    let mut symbol_ids: Vec<&String> = symbol_comp.keys().collect();
    symbol_ids.sort();
    for sym_id in symbol_ids {
        let Some(comp) = comp_of(sym_id) else { continue };
        let mut rels: Vec<&scc_core::Relationship> = graph.out_edges(sym_id);
        rels.sort_by(|a, b| {
            a.predicate
                .cmp(&b.predicate)
                .then_with(|| a.object.cmp(&b.object))
                .then_with(|| a.id.cmp(&b.id))
        });
        for r in rels {
            let target = match graph.entities.get(&r.object) {
                Some(e) => e,
                None => continue,
            };
            match r.predicate.as_str() {
                predicates::WRITES => {
                    if is_cache_store(&target.name, tech(graph, &r.object)) {
                        push(
                            S_CACHES,
                            format!("{} writes {} ({})", comp, target.name, prov_str(&r.provenance)),
                        );
                    } else if target.kind == kinds::DATA_STORE || target.kind == kinds::DATA_ENTITY {
                        push(
                            S_PERSISTENT,
                            format!("{} owns {} ({})", comp, store_ref(graph, &r.object), prov_str(&r.provenance)),
                        );
                    }
                }
                predicates::READS => {
                    if target.kind == kinds::DATA_STORE && is_cache_store(&target.name, tech(graph, &r.object)) {
                        push(
                            S_CACHES,
                            format!("{} reads {} ({})", comp, target.name, prov_str(&r.provenance)),
                        );
                    }
                }
                predicates::PUBLISHES => {
                    push(
                        S_DERIVED,
                        format!("{} publishes {} ({})", comp, target.name, prov_str(&r.provenance)),
                    );
                }
                predicates::SUBSCRIBES => {
                    push(
                        S_DERIVED,
                        format!("{} subscribes {} ({})", comp, target.name, prov_str(&r.provenance)),
                    );
                }
                predicates::CONSUMES => {
                    push(
                        S_DERIVED,
                        format!("{} consumes {} ({})", comp, target.name, prov_str(&r.provenance)),
                    );
                }
                predicates::REGISTERS => {
                    let is_mw_registry = target.kind == kinds::MIDDLEWARE
                        || target.kind == kinds::REGISTRY
                        || (target.kind == kinds::CONTRACT
                            && target
                                .attributes
                                .get("kind")
                                .and_then(|v| v.as_str())
                                .map(|k| matches!(k, "middleware" | "registry"))
                                .unwrap_or(false));
                    if is_mw_registry {
                        push(
                            S_DERIVED,
                            format!("{} registers {} ({})", comp, target.name, prov_str(&r.provenance)),
                        );
                    }
                }
                _ => {}
            }
        }
    }

    // ---- runtime state: mutable FIELD facts + STATE/REGISTRY entities ----
    let mut runtime_entities: Vec<&scc_core::Entity> = graph
        .entities_of_kind(kinds::FIELD)
        .into_iter()
        .filter(|f| f.attributes.get("mutable").and_then(|v| v.as_bool()) == Some(true))
        .collect();
    runtime_entities.extend(graph.entities_of_kind(kinds::STATE));
    runtime_entities.extend(graph.entities_of_kind(kinds::REGISTRY));
    runtime_entities.sort_by(|a, b| a.id.cmp(&b.id));
    for e in runtime_entities {
        // FIELD facts are CONTAINS-ed by their owning symbol; STATE/REGISTRY
        // entities too — resolve the owner symbol, then the component.
        let mut owner: Option<String> = None;
        let mut prov: Option<Provenance> = None;
        let mut rels = graph.in_pred(&e.id, predicates::CONTAINS);
        rels.sort_by(|a, b| a.id.cmp(&b.id));
        for r in rels {
            if let Some(c) = comp_of(&r.subject) {
                owner = Some(c);
                prov = Some(r.provenance);
                break;
            }
        }
        let Some(comp) = owner else { continue };
        let tag = match e.kind.as_str() {
            kinds::FIELD => "mutable",
            kinds::STATE => "state",
            _ => "registry",
        };
        push(
            S_RUNTIME,
            format!(
                "{} owns {} ({tag}) ({})",
                comp,
                e.name,
                prov.map(|p| prov_str(&p)).unwrap_or_else(|| "EXTRACTED".to_string())
            ),
        );
    }

    // ---- configuration: CONFIGURED_BY (config -> owner symbol) ----
    for cfg in graph.entities_of_kind(kinds::CONFIGURATION) {
        let mut rels = graph.out_pred(&cfg.id, predicates::CONFIGURED_BY);
        rels.sort_by(|a, b| a.id.cmp(&b.id));
        for r in rels {
            if let Some(comp) = comp_of(&r.object) {
                push(
                    S_CONFIGURATION,
                    format!(
                        "{} configured_by {} ({})",
                        comp,
                        cfg.name,
                        prov_str(&r.provenance)
                    ),
                );
            }
        }
    }

    // ---- reactive state: REACTIVE entities (Wave 11) owned via OWNS
    // edges (svelte $state, vue ref/reactive, react useState, mobx,
    // signals). The owner symbol's component carries the line.
    for e in graph.entities_of_kind(kinds::REACTIVE) {
        let access = e
            .attributes
            .get("access")
            .and_then(|v| v.as_str())
            .unwrap_or("state");
        let mut rels = graph.in_pred(&e.id, predicates::OWNS);
        rels.sort_by(|a, b| a.id.cmp(&b.id));
        for r in rels {
            if let Some(comp) = comp_of(&r.subject) {
                push(
                    S_REACTIVE,
                    format!(
                        "{} owns reactive: {} [{}] ({})",
                        comp,
                        e.name,
                        access,
                        prov_str(&r.provenance)
                    ),
                );
            }
        }
    }

    sections
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect()
}

/// Groups of symbol ids that SHARE state authority: distinct symbols
/// writing the same store (data entities resolve to their owning store, so
/// `db.users` and `db.orders` count as one target), read by the same
/// CONFIGURED_BY configuration target, or owning the same REACTIVE state
/// entity (Wave 11 — symbols mutating the same reactive state cohere).
/// This is the shared-state-authority signal for the semantic clustering
/// graph (+4 per pair inside a group).
///
/// Deterministic: groups are built over sorted symbol ids and each group is
/// a sorted `BTreeSet`; groups with fewer than 2 symbols (no pair) are
/// omitted. Pure function of the graph — nothing is stored or promoted.
// trace:v1 id=impl.scc.state.groups work=WORK-SCC-005 satisfies=REQ-SCC-IR
pub fn state_authority_groups(graph: &RealityGraph) -> Vec<BTreeSet<String>> {
    // store target -> symbols writing it (data entities resolve to their
    // owning store so writes to db.users and db.orders share authority)
    let mut store_syms: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut sym_ids: Vec<&String> = graph
        .entities_of_kind(kinds::SYMBOL)
        .into_iter()
        .map(|e| &e.id)
        .collect();
    sym_ids.sort();
    for sym in sym_ids {
        for r in graph.out_pred(sym, predicates::WRITES) {
            let target = if r.object.contains("/data/") {
                graph
                    .entities
                    .get(&r.object)
                    .and_then(|e| e.attributes.get("store"))
                    .and_then(|v| v.as_str())
                    .map(|s| scc_core::entity_id(&graph.repo_id, kinds::DATA_STORE, s))
                    .unwrap_or_else(|| r.object.clone())
            } else {
                r.object.clone()
            };
            store_syms
                .entry(target)
                .or_default()
                .insert(sym.clone());
        }
    }
    // configuration target -> symbols it configures (CONFIGURED_BY)
    let mut cfg_syms: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for cfg in graph.entities_of_kind(kinds::CONFIGURATION) {
        let mut rels = graph.out_pred(&cfg.id, predicates::CONFIGURED_BY);
        rels.sort_by(|a, b| a.id.cmp(&b.id));
        for r in rels {
            cfg_syms
                .entry(cfg.id.clone())
                .or_default()
                .insert(r.object.clone());
        }
    }
    // reactive entity -> symbols owning it (OWNS edges)
    let mut reactive_syms: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for rs in graph.entities_of_kind(kinds::REACTIVE) {
        let mut rels = graph.in_pred(&rs.id, predicates::OWNS);
        rels.sort_by(|a, b| a.id.cmp(&b.id));
        for r in rels {
            reactive_syms
                .entry(rs.id.clone())
                .or_default()
                .insert(r.subject.clone());
        }
    }
    let mut out: Vec<BTreeSet<String>> = Vec::new();
    for group in store_syms
        .values()
        .chain(cfg_syms.values())
        .chain(reactive_syms.values())
    {
        if group.len() >= 2 {
            out.push(group.clone());
        }
    }
    out.sort();
    out
}

/// One structured state-ownership claim: `component` owns/reads/registers
/// `target` (evidence `provenance`). The structured bridge from the STATE &
/// DATA AUTHORITY compiler into the atlas component `owns` claims, so the
/// state fact layer is part of the machine model (not just rendered text).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct StateClaim {
    pub component: String,
    pub target: String,
    pub provenance: String,
}

/// Emit per-component state claims over the same fact sets as
/// [`compile_state_authority`]: WRITES to stores/caches, mutable FIELD /
/// STATE / REGISTRY owners, CONFIGURED_BY configuration targets, topics
/// (PUBLISHES/SUBSCRIBES/CONSUMES), and middleware/registry REGISTERS.
/// Deterministic: sorted by (component, target, provenance).
// trace:v1 id=impl.scc.state work=WORK-SCC-005 satisfies=REQ-SCC-IR
pub fn compile_state_claims(
    graph: &RealityGraph,
    symbol_comp: &HashMap<String, String>,
) -> Vec<StateClaim> {
    let mut claims: BTreeSet<StateClaim> = BTreeSet::new();
    let comp_of = |sym_id: &str| symbol_comp.get(sym_id).cloned();
    let prov_str = |p: &Provenance| p.as_str().to_string();

    // per-symbol edges (writes/reads/publishes/subscribes/consumes/registers)
    let mut symbol_ids: Vec<&String> = symbol_comp.keys().collect();
    symbol_ids.sort();
    for sym_id in symbol_ids {
        let Some(comp) = comp_of(sym_id) else { continue };
        let mut rels: Vec<&scc_core::Relationship> = graph.out_edges(sym_id);
        rels.sort_by(|a, b| {
            a.predicate
                .cmp(&b.predicate)
                .then_with(|| a.object.cmp(&b.object))
                .then_with(|| a.id.cmp(&b.id))
        });
        for r in rels {
            let Some(target) = graph.entities.get(&r.object) else { continue };
            let tgt = match r.predicate.as_str() {
                predicates::WRITES => {
                    if target.kind == kinds::DATA_STORE || target.kind == kinds::DATA_ENTITY {
                        Some(store_ref(graph, &r.object))
                    } else {
                        Some(target.name.clone())
                    }
                }
                predicates::PUBLISHES
                | predicates::SUBSCRIBES
                | predicates::CONSUMES
                | predicates::READS => Some(target.name.clone()),
                predicates::REGISTERS => {
                    let is_mw_registry = target.kind == kinds::MIDDLEWARE
                        || target.kind == kinds::REGISTRY
                        || (target.kind == kinds::CONTRACT
                            && target
                                .attributes
                                .get("kind")
                                .and_then(|v| v.as_str())
                                .map(|k| matches!(k, "middleware" | "registry"))
                                .unwrap_or(false));
                    if is_mw_registry {
                        Some(target.name.clone())
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(t) = tgt {
                claims.insert(StateClaim {
                    component: comp.clone(),
                    target: t,
                    provenance: prov_str(&r.provenance),
                });
            }
        }
    }

    // runtime state: mutable FIELD facts + STATE/REGISTRY entities
    let mut runtime_entities: Vec<&scc_core::Entity> = graph
        .entities_of_kind(kinds::FIELD)
        .into_iter()
        .filter(|f| f.attributes.get("mutable").and_then(|v| v.as_bool()) == Some(true))
        .collect();
    runtime_entities.extend(graph.entities_of_kind(kinds::STATE));
    runtime_entities.extend(graph.entities_of_kind(kinds::REGISTRY));
    runtime_entities.sort_by(|a, b| a.id.cmp(&b.id));
    for e in runtime_entities {
        let mut rels = graph.in_pred(&e.id, predicates::CONTAINS);
        rels.sort_by(|a, b| a.id.cmp(&b.id));
        for r in rels {
            if let Some(comp) = comp_of(&r.subject) {
                claims.insert(StateClaim {
                    component: comp,
                    target: e.name.clone(),
                    provenance: prov_str(&r.provenance),
                });
                break;
            }
        }
    }

    // configuration: CONFIGURED_BY (config -> owner symbol)
    for cfg in graph.entities_of_kind(kinds::CONFIGURATION) {
        let mut rels = graph.out_pred(&cfg.id, predicates::CONFIGURED_BY);
        rels.sort_by(|a, b| a.id.cmp(&b.id));
        for r in rels {
            if let Some(comp) = comp_of(&r.object) {
                claims.insert(StateClaim {
                    component: comp,
                    target: cfg.name.clone(),
                    provenance: prov_str(&r.provenance),
                });
            }
        }
    }

    // reactive state: REACTIVE entities owned via OWNS edges (the owner
    // symbol's component carries the `reactive: name [access]` claim)
    for e in graph.entities_of_kind(kinds::REACTIVE) {
        let access = e
            .attributes
            .get("access")
            .and_then(|v| v.as_str())
            .unwrap_or("state");
        let mut rels = graph.in_pred(&e.id, predicates::OWNS);
        rels.sort_by(|a, b| a.id.cmp(&b.id));
        for r in rels {
            if let Some(comp) = comp_of(&r.subject) {
                claims.insert(StateClaim {
                    component: comp,
                    target: format!("reactive: {} [{}]", e.name, access),
                    provenance: prov_str(&r.provenance),
                });
            }
        }
    }

    let mut out: Vec<StateClaim> = claims.into_iter().collect();
    out.sort();
    out
}

/// Resolve a write target to a human store reference: data entities render
/// as `store.entity`, stores as their name.
fn store_ref(graph: &RealityGraph, id: &str) -> String {    match graph.entities.get(id) {
        Some(e) if e.kind == kinds::DATA_ENTITY => e
            .attributes
            .get("store")
            .and_then(|v| v.as_str())
            .map(|s| format!("{s}.{}", e.name))
            .unwrap_or_else(|| e.name.clone()),
        Some(e) => e.name.clone(),
        None => id.to_string(),
    }
}

fn tech<'a>(graph: &'a RealityGraph, id: &str) -> Option<&'a str> {
    graph
        .entities
        .get(id)
        .and_then(|e| e.attributes.get("technology"))
        .and_then(|v| v.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scc_core::{entity_id, symbol_id, Entity, Relationship};
    use scc_store::Store;

    fn open() -> (tempfile::TempDir, Store) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        (dir, store)
    }


    fn sym(store: &Store, path: &str, name: &str) -> String {
        let id = symbol_id(&store.repo_id, path, name);
        store
            .insert_entity(&Entity::new(id.clone(), kinds::SYMBOL, name), &[path.into()])
            .unwrap();
        id
    }

    fn component(store: &Store, name: &str, files: &[&str]) {
        // components live in the `components` table (RealityGraph::load
        // reads store.components()), so insert them through
        // replace_components like the real pipeline does.
        let id = entity_id(&store.repo_id, kinds::COMPONENT, name);
        let mut existing: Vec<scc_core::Entity> = store.components().unwrap();
        if let Some(c) = existing.iter_mut().find(|c| c.name == name) {
            // keep prior CONTAINS edges; just ensure presence
            c.id = id.clone();
        } else {
            existing.push(scc_core::Entity::new(id.clone(), kinds::COMPONENT, name));
        }
        store.replace_components(&existing).unwrap();
        for f in files {
            let fid = entity_id(&store.repo_id, kinds::FILE, f);
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:c:{name}:{f}"),
                        id.clone(),
                        predicates::CONTAINS,
                        fid,
                        Provenance::Extracted,
                    ),
                    f,
                )
                .unwrap();
        }
    }

    fn attach(store: &Store, comp: &str, sym_id: &str, path: &str) {
        // symbol lives in a file of the component: file CONTAINS symbol
        let fid = entity_id(&store.repo_id, kinds::FILE, path);
        store
            .insert_relationship(
                &Relationship::new(
                    format!("rel:fc:{comp}:{sym_id}"),
                    fid,
                    predicates::CONTAINS,
                    sym_id.to_string(),
                    Provenance::Extracted,
                ),
                path,
            )
            .unwrap();
    }

    #[test]
    fn attributes_state_ownership_per_component() {
        let (_dir, store) = open();
        let repo = store.repo_id.clone();

        component(&store, "api", &["api/app.py"]);
        component(&store, "web", &["web/app.py"]);

        // api writes db.users (persistent) and a cache, owns mutable field,
        // reads config, publishes a topic, registers middleware
        let api_writer = sym(&store, "api/app.py", "create_user");
        attach(&store, "api", &api_writer, "api/app.py");
        let store_ent = entity_id(&repo, kinds::DATA_STORE, "db");
        store
            .insert_entity(
                &Entity::new(store_ent.clone(), kinds::DATA_STORE, "db"),
                &["api/app.py".into()],
            )
            .unwrap();
        let user = entity_id(&repo, kinds::DATA_ENTITY, "db.users");
        store
            .insert_entity(
                Entity::new(user.clone(), kinds::DATA_ENTITY, "users")
                    .attr("store", serde_json::json!("db")),
                &["api/app.py".into()],
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:w:users",
                    api_writer.clone(),
                    predicates::WRITES,
                    user,
                    Provenance::Extracted,
                )
                .with_confidence(1.0),
                "api/app.py",
            )
            .unwrap();
        let cache = entity_id(&repo, kinds::DATA_STORE, "redis");
        store
            .insert_entity(
                Entity::new(cache.clone(), kinds::DATA_STORE, "redis")
                    .attr("technology", serde_json::json!("redis")),
                &["api/app.py".into()],
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:r:cache",
                    api_writer.clone(),
                    predicates::READS,
                    cache,
                    Provenance::Extracted,
                ),
                "api/app.py",
            )
            .unwrap();
        // mutable field on class Cart inside api/app.py
        let cart = sym(&store, "api/app.py", "Cart");
        attach(&store, "api", &cart, "api/app.py");
        let field = entity_id(&repo, kinds::FIELD, "Cart.items");
        store
            .insert_entity(
                Entity::new(field.clone(), kinds::FIELD, "Cart.items")
                    .attr("mutable", serde_json::json!(true))
                    .attr("owner", serde_json::json!("Cart")),
                &["api/app.py".into()],
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:f:items",
                    cart,
                    predicates::CONTAINS,
                    field,
                    Provenance::Extracted,
                ),
                "api/app.py",
            )
            .unwrap();
        // configuration
        let cfg = entity_id(&repo, kinds::CONFIGURATION, "DEBUG");
        store
            .insert_entity(
                &Entity::new(cfg.clone(), kinds::CONFIGURATION, "DEBUG"),
                &["api/app.py".into()],
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:cfg",
                    cfg,
                    predicates::CONFIGURED_BY,
                    api_writer.clone(),
                    Provenance::Extracted,
                ),
                "api/app.py",
            )
            .unwrap();
        // topic publish
        let topic = entity_id(&repo, kinds::TOPIC, "user.created");
        store
            .insert_entity(
                Entity::new(topic.clone(), kinds::TOPIC, "user.created")
                    .attr("store", serde_json::json!("kafka")),
                &["api/app.py".into()],
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:p:topic",
                    api_writer.clone(),
                    predicates::PUBLISHES,
                    topic,
                    Provenance::Extracted,
                ),
                "api/app.py",
            )
            .unwrap();
        // middleware registration
        let mw = entity_id(&repo, kinds::MIDDLEWARE, "RequestLogger");
        store
            .insert_entity(
                &Entity::new(mw.clone(), kinds::MIDDLEWARE, "RequestLogger"),
                &["api/app.py".into()],
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:reg:mw",
                    api_writer,
                    predicates::REGISTERS,
                    mw,
                    Provenance::Extracted,
                ),
                "api/app.py",
            )
            .unwrap();

        // web only reads a plain store (not cache) -> no state claims
        let web_reader = sym(&store, "web/app.py", "list_items");
        attach(&store, "web", &web_reader, "web/app.py");
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:r:web",
                    web_reader,
                    predicates::READS,
                    store_ent,
                    Provenance::Extracted,
                ),
                "web/app.py",
            )
            .unwrap();

        let graph = RealityGraph::load(&store).unwrap();
        let mut symbol_comp: HashMap<String, String> = HashMap::new();
        for c in &graph.components {
            for r in graph.out_pred(&c.id, predicates::CONTAINS) {
                for sr in graph.out_pred(&r.object, predicates::CONTAINS) {
                    symbol_comp.insert(sr.object.clone(), c.name.clone());
                }
            }
        }

        let state = compile_state_authority(&graph, &symbol_comp);
        // every section key present (empty sections stay empty)
        for k in STATE_SECTIONS {
            assert!(state.contains_key(k), "missing section {k}: {state:?}");
        }
        let persistent = &state[S_PERSISTENT];
        assert_eq!(persistent.len(), 1, "{persistent:?}");
        assert_eq!(persistent[0], "api owns db.users (EXTRACTED)");
        let runtime = &state[S_RUNTIME];
        assert_eq!(runtime.len(), 1, "{runtime:?}");
        assert_eq!(runtime[0], "api owns Cart.items (mutable) (EXTRACTED)");
        let config = &state[S_CONFIGURATION];
        assert_eq!(config[0], "api configured_by DEBUG (EXTRACTED)");
        let caches = &state[S_CACHES];
        assert_eq!(caches[0], "api reads redis (EXTRACTED)");
        let derived = &state[S_DERIVED];
        assert!(derived.iter().any(|l| l.starts_with("api publishes user.created")), "{derived:?}");
        assert!(derived.iter().any(|l| l == "api registers RequestLogger (EXTRACTED)"), "{derived:?}");

        // web has NO state claims anywhere
        for k in STATE_SECTIONS {
            for line in &state[k] {
                assert!(!line.starts_with("web "), "web must not own state: {line}");
            }
        }

        // determinism: identical graph -> identical output
        let graph2 = RealityGraph::load(&store).unwrap();
        let state2 = compile_state_authority(&graph2, &symbol_comp);
        assert_eq!(state, state2);
    }

    #[test]
    fn cache_store_heuristics() {
        assert!(is_cache_store("redis", Some("redis")));
        assert!(is_cache_store("cache", None));
        assert!(is_cache_store("user-cache", None));
        assert!(is_cache_store("kv", Some("valkey")));
        assert!(!is_cache_store("db", Some("postgres")));
        assert!(!is_cache_store("orders", None));
    }

    /// Wave 11: symbols OWNS-ing the same REACTIVE entity form a
    /// shared-state authority group (the +4 clustering signal), and the
    /// REACTIVE STATE section attributes the state to the owner symbol's
    /// component.
    #[test]
    // # trace:exempt — unit test (tests are not trace-worthy behavior)
    fn reactive_state_owners_group_and_attribute() {
        let (_dir, store) = open();
        let repo = store.repo_id.clone();
        component(&store, "api", &["api/app.py"]);
        let a = sym(&store, "api/app.py", "store_a");
        attach(&store, "api", &a, "api/app.py");
        let b = sym(&store, "api/app.py", "store_b");
        attach(&store, "api", &b, "api/app.py");

        let rs = entity_id(&repo, kinds::REACTIVE, "count");
        store
            .insert_entity(
                Entity::new(rs.clone(), kinds::REACTIVE, "count")
                    .attr("access", serde_json::json!("state")),
                &["api/app.py".into()],
            )
            .unwrap();
        for (i, s) in [a.clone(), b.clone()].iter().enumerate() {
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:owns:{i}"),
                        s.clone(),
                        predicates::OWNS,
                        rs.clone(),
                        Provenance::Extracted,
                    ),
                    "api/app.py",
                )
                .unwrap();
        }

        // shared-reactive-ownership group: both owners in one group
        let groups = state_authority_groups(&RealityGraph::load(&store).unwrap());
        assert!(
            groups.iter().any(|g| g.contains(&a) && g.contains(&b) && g.len() == 2),
            "reactive owners must group: {groups:?}"
        );

        // section attribution: both owners render under REACTIVE STATE
        let graph = RealityGraph::load(&store).unwrap();
        let mut symbol_comp: HashMap<String, String> = HashMap::new();
        for c in &graph.components {
            for r in graph.out_pred(&c.id, predicates::CONTAINS) {
                for sr in graph.out_pred(&r.object, predicates::CONTAINS) {
                    symbol_comp.insert(sr.object.clone(), c.name.clone());
                }
            }
        }
        let state = compile_state_authority(&graph, &symbol_comp);
        assert_eq!(
            state[S_REACTIVE],
            vec!["api owns reactive: count [state] (EXTRACTED)".to_string()],
            "{:?}",
            state[S_REACTIVE]
        );

        // structured claims bridge carries the same attribution
        let claims = compile_state_claims(&graph, &symbol_comp);
        assert!(
            claims.iter().any(|c| c.target == "reactive: count [state]"),
            "claims missing reactive state: {claims:?}"
        );
    }

    #[test]
    fn non_mutable_fields_are_not_runtime_state() {
        let (_dir, store) = open();
        let repo = store.repo_id.clone();
        component(&store, "api", &["api/app.py"]);
        let cart = sym(&store, "api/app.py", "Cart");
        attach(&store, "api", &cart, "api/app.py");
        let field = entity_id(&repo, kinds::FIELD, "Cart.name");
        store
            .insert_entity(
                Entity::new(field.clone(), kinds::FIELD, "Cart.name")
                    .attr("mutable", serde_json::json!(false))
                    .attr("owner", serde_json::json!("Cart")),
                &["api/app.py".into()],
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:f:name",
                    cart,
                    predicates::CONTAINS,
                    field,
                    Provenance::Extracted,
                ),
                "api/app.py",
            )
            .unwrap();
        let graph = RealityGraph::load(&store).unwrap();
        let mut symbol_comp = HashMap::new();
        for c in &graph.components {
            for r in graph.out_pred(&c.id, predicates::CONTAINS) {
                for sr in graph.out_pred(&r.object, predicates::CONTAINS) {
                    symbol_comp.insert(sr.object.clone(), c.name.clone());
                }
            }
        }
        let state = compile_state_authority(&graph, &symbol_comp);
        let runtime = state.get(S_RUNTIME).map(|v| v.as_slice()).unwrap_or(&[]);
        assert!(runtime.is_empty(), "immutable field is not runtime state: {runtime:?}");
    }

    /// Wave 9 symbol→state authority: module-level globals (owned by the
    /// module symbol) and class statics are mutable FIELD facts attributed
    /// to their component in the RUNTIME STATE section + claims bridge.
    #[test]
    fn module_global_and_static_state_attribute_to_component() {
        let (_dir, store) = open();
        let repo = store.repo_id.clone();
        component(&store, "api", &["api/app.py"]);

        // module symbol (file stem) owns a module-level mutable global
        let module = sym(&store, "api/app.py", "app");
        attach(&store, "api", &module, "api/app.py");
        let field = entity_id(&repo, kinds::FIELD, "app.DEFAULT_TIMEOUT");
        store
            .insert_entity(
                Entity::new(field.clone(), kinds::FIELD, "app.DEFAULT_TIMEOUT")
                    .attr("mutable", serde_json::json!(true))
                    .attr("owner", serde_json::json!("app")),
                &["api/app.py".into()],
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:f:dt",
                    module,
                    predicates::CONTAINS,
                    field,
                    Provenance::Extracted,
                ),
                "api/app.py",
            )
            .unwrap();

        // static field on a class in the same file
        let cfg = sym(&store, "api/app.py", "Config");
        attach(&store, "api", &cfg, "api/app.py");
        let stat = entity_id(&repo, kinds::FIELD, "Config.retries");
        store
            .insert_entity(
                Entity::new(stat.clone(), kinds::FIELD, "Config.retries")
                    .attr("mutable", serde_json::json!(true))
                    .attr("owner", serde_json::json!("Config")),
                &["api/app.py".into()],
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:f:r",
                    cfg,
                    predicates::CONTAINS,
                    stat,
                    Provenance::Extracted,
                ),
                "api/app.py",
            )
            .unwrap();

        let graph = RealityGraph::load(&store).unwrap();
        let mut symbol_comp: HashMap<String, String> = HashMap::new();
        for c in &graph.components {
            for r in graph.out_pred(&c.id, predicates::CONTAINS) {
                for sr in graph.out_pred(&r.object, predicates::CONTAINS) {
                    symbol_comp.insert(sr.object.clone(), c.name.clone());
                }
            }
        }

        let state = compile_state_authority(&graph, &symbol_comp);
        let runtime = &state[S_RUNTIME];
        assert!(
            runtime
                .iter()
                .any(|l| l == "api owns app.DEFAULT_TIMEOUT (mutable) (EXTRACTED)"),
            "module global missing from runtime state: {runtime:?}"
        );
        assert!(
            runtime
                .iter()
                .any(|l| l == "api owns Config.retries (mutable) (EXTRACTED)"),
            "class static missing from runtime state: {runtime:?}"
        );

        // structured claims bridge carries the same attributions
        let claims = compile_state_claims(&graph, &symbol_comp);
        assert!(
            claims
                .iter()
                .any(|c| c.component == "api" && c.target == "app.DEFAULT_TIMEOUT"),
            "claims missing module global: {claims:?}"
        );
        assert!(
            claims
                .iter()
                .any(|c| c.component == "api" && c.target == "Config.retries"),
            "claims missing class static: {claims:?}"
        );
    }
}
