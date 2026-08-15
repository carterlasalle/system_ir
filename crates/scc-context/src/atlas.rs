//! System Atlas compiler (Wave 2): the full-system architecture artifact
//! injected into coding agents at session start (docs/SYSTEM_DESIGN.md §8).
//!
//! Builds a structured [`scc_core::SystemAtlas`] from the trusted view, then
//! renders it as compact structured text. The atlas is the product: the
//! agent should know the architecture *before* its first coding task.
//!
//! Trust contract: every fact comes from the TrustedGraphView — STALE facts
//! are excluded and surfaced as warnings; low-confidence inference is
//! excluded unless `include_low_confidence_inference` is set.

use crate::packs::{entity_name, finish, Section};
use scc_graph::TrustedGraphView;
use crate::{ContextCompiler, ContextPack};
use scc_core::{
    Archetype, AtlasComponent, AtlasEntrypoint, AtlasFlow, AtlasHierarchyNode, AtlasInvariant,
    AtlasOwnershipClaim, ContractSubclass, FlowKind, SystemAtlas,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Structured atlas compilation — pure data, no rendering.
// trace:v1 id=impl.scc.atlas work=WORK-SCC-001 satisfies=REQ-SCC-CTX
pub fn build_atlas(ctx: &ContextCompiler) -> SystemAtlas {
    let view = &ctx.view;
    let store = ctx.store;
    let snapshot = store.latest_snapshot().ok().flatten();
    let repo = store.repository();

    let purpose = store.meta_get("purpose").ok().flatten().unwrap_or_default();

    // ---- components ----
    let mut components: Vec<AtlasComponent> = Vec::new();
    // data stores / data entities written by component symbols (WRITES-derived)
    let mut data_stores: BTreeSet<String> = BTreeSet::new();
    for c in view.components() {
        let purpose_text = c
            .attributes
            .get("responsibility")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|r| r.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();

        let implementation_attr = c
            .attributes
            .get("implementation")
            .cloned()
            .unwrap_or(serde_json::json!({}));
        let implementation_paths: Vec<String> = implementation_attr
            .get("paths")
            .and_then(|p| p.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        // the full implementation fact: directory paths AND member symbol
        // names (the component compiler attributes every contained symbol).
        // The structured model carries both; the render shows only the
        // paths (`implementation_paths`) to stay compact.
        let mut implementation: Vec<String> = implementation_paths.clone();
        let mut symbols: Vec<String> = implementation_attr
            .get("symbols")
            .and_then(|s| s.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        symbols.sort();
        symbols.dedup();
        implementation.extend(symbols.iter().cloned());

        let mut upstream: BTreeSet<String> = BTreeSet::new();
        let mut downstream: BTreeSet<String> = BTreeSet::new();
        for r in view.in_pred(&c.id, scc_core::predicates::DEPENDS_ON) {
            upstream.insert(entity_name(view, &r.subject));
        }
        for r in view.out_pred(&c.id, scc_core::predicates::DEPENDS_ON) {
            downstream.insert(entity_name(view, &r.object));
        }

        let mut failure_behavior: Vec<String> = Vec::new();
        if let Some(rs) = c.attributes.get("retries").and_then(|v| v.as_array()) {
            failure_behavior.extend(rs.iter().filter_map(|x| x.as_str().map(String::from)));
        }
        for r in view.out_pred(&c.id, scc_core::predicates::CROSSES_BOUNDARY) {
            failure_behavior.push(format!("crosses boundary -> {}", entity_name(view, &r.object)));
        }

        let mut consumes: BTreeSet<String> = BTreeSet::new();
        let mut produces: BTreeSet<String> = BTreeSet::new();
        for pred in [
            scc_core::predicates::READS,
            scc_core::predicates::CONSUMES,
            scc_core::predicates::QUERIES,
        ] {
            for r in view.out_pred(&c.id, pred) {
                consumes.insert(entity_name(view, &r.object));
            }
        }
        for pred in [
            scc_core::predicates::PRODUCES,
            scc_core::predicates::PUBLISHES,
            scc_core::predicates::WRITES,
        ] {
            for r in view.out_pred(&c.id, pred) {
                produces.insert(entity_name(view, &r.object));
            }
        }

        // Ownership claims come from the component compiler's `owns` attr
        // (write-edge derived + declared intent, provenance preserved).
        // Claims targeting data-store / data-entity entities additionally
        // surface in the atlas DATA STORES list, using the full store
        // reference (`db.users`) so data entities stay attributed to their
        // store.
        let mut owns: Vec<AtlasOwnershipClaim> = Vec::new();
        if let Some(oa) = c.attributes.get("owns").and_then(|v| v.as_array()) {
            for o in oa {
                let Some(t) = o.get("target").and_then(|v| v.as_str()) else {
                    continue;
                };
                let p = o.get("provenance").and_then(|v| v.as_str()).unwrap_or("");
                let is_store_target = view
                    .entity(t)
                    .map(|e| {
                        e.kind == scc_core::kinds::DATA_STORE
                            || e.kind == scc_core::kinds::DATA_ENTITY
                    })
                    .unwrap_or(false);
                let target_name = match view.entity(t) {
                    Some(e) if e.kind == scc_core::kinds::DATA_STORE => e.name.clone(),
                    Some(e) if e.kind == scc_core::kinds::DATA_ENTITY => e
                        .attributes
                        .get("store")
                        .and_then(|v| v.as_str())
                        .map(|s| format!("{s}.{}", e.name))
                        .unwrap_or_else(|| e.name.clone()),
                    _ => entity_name(view, t),
                };
                if is_store_target {
                    data_stores.insert(target_name.clone());
                }
                owns.push(AtlasOwnershipClaim {
                    target: target_name,
                    provenance: p.to_string(),
                });
            }
        }
        // defensive dedupe: the same (target, provenance) pair may repeat
        // across claims
        let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
        owns.retain(|o| seen.insert((o.target.clone(), o.provenance.clone())));

        // Ontology phase: hierarchical layer + immediate container (set by
        // the component compiler's clusterer; defaulted for pre-ontology
        // components so grouping stays total and deterministic).
        let layer = c
            .attributes
            .get("layer")
            .and_then(|v| v.as_str())
            .unwrap_or("component")
            .to_string();
        let parent = c
            .attributes
            .get("parent")
            .and_then(|v| v.as_str())
            .map(String::from);

        components.push(AtlasComponent {
            name: c.name.clone(),
            purpose: purpose_text,
            implementation,
            implementation_paths,
            symbols,
            consumes: consumes.into_iter().collect(),
            produces: produces.into_iter().collect(),
            upstream: upstream.into_iter().collect(),
            downstream: downstream.into_iter().collect(),
            failure_behavior,
            owns,
            layer,
            parent,
        });
    }
    components.sort_by(|a, b| a.name.cmp(&b.name));

    // ---- entrypoints ----
    let mut entrypoints: Vec<AtlasEntrypoint> = Vec::new();
    for r in view.entities_of_kind(scc_core::kinds::ROUTE) {
        let method = r
            .attributes
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let path = r
            .attributes
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let handler = r
            .attributes
            .get("handler")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        entrypoints.push(AtlasEntrypoint {
            name: r.name.clone(),
            kind: "route".into(),
            trigger: format!("{method} {path}"),
            symbol: handler,
        });
    }
    for e in view.entities_of_kind(scc_core::kinds::SYMBOL) {
        let Some(kinds) = e.attributes.get("entrypoints").and_then(|v| v.as_array()) else {
            continue;
        };
        if kinds.is_empty() {
            continue;
        }
        // extractor contract: entrypoint kinds are strings ("main-guard",
        // "cli-subcommand", ...); cli-subcommand entrypoints render as
        // `name [cli-subcommand]` instead of the generic kind
        let kind = if kinds
            .iter()
            .any(|k| k.as_str() == Some("cli-subcommand"))
        {
            "cli-subcommand"
        } else {
            "entrypoint"
        };
        entrypoints.push(AtlasEntrypoint {
            name: e.name.clone(),
            kind: kind.into(),
            trigger: format!("entrypoint:{}", e.name),
            symbol: e.id.clone(),
        });
    }
    // Wave 9: invocation-surface seeds (public exports → public_api, queue
    // consumers → queue, framework callbacks → framework_callback,
    // lifecycle callbacks → lifecycle, event handlers → event). Additive and
    // deterministic (invocation_surfaces sorts its output). Deduped by
    // (name, kind): a symbol that is several surfaces at once (exported AND
    // callback registrar) renders under each kind — the atlas has no
    // unique-id constraint.
    let mut surface_names: BTreeSet<(String, String)> = entrypoints
        .iter()
        .map(|e| (e.name.clone(), e.kind.clone()))
        .collect();
    for s in scc_graph::flows::invocation_surfaces(view.graph) {
        let name = view.name_of(&s.symbol);
        let kind = s.kind.as_str().to_string();
        if !surface_names.insert((name.clone(), kind.clone())) {
            continue;
        }
        entrypoints.push(AtlasEntrypoint {
            name,
            kind,
            trigger: s.trigger.clone(),
            symbol: s.symbol.clone(),
        });
    }
    entrypoints.sort_by(|a, b| a.name.cmp(&b.name));

    // ---- contracts (Wave 9: first-class, typed) ----
    let mut contracts: Vec<scc_core::Contract> = Vec::new();
    let mut contract_seen: BTreeMap<(String, String), usize> = BTreeMap::new();

    // http: ROUTE entities (producer = handler symbol)
    for r in view.entities_of_kind(scc_core::kinds::ROUTE) {
        let method = r
            .attributes
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let path = r
            .attributes
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if path.is_empty() {
            continue;
        }
        let handler = r
            .attributes
            .get("handler")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let mut consumers: BTreeSet<String> = BTreeSet::new();
        for pred in [
            scc_core::predicates::HANDLES,
            scc_core::predicates::CONSUMES,
            scc_core::predicates::READS,
        ] {
            for rel in view.in_pred(&r.id, pred) {
                consumers.insert(entity_name(view, &rel.subject));
            }
        }
        push_contract(
            &mut contracts,
            &mut contract_seen,
            scc_core::Contract {
                id: r.id.clone(),
                kind: "http".into(),
                subclass: ContractSubclass::Http,
                producer: handler,
                consumers: consumers.into_iter().collect(),
                operations: vec![format!("{method} {path}").trim().to_string()],
                evidence: r.evidence.clone(),
            },
        );
    }

    // cli: SYMBOL entities carrying `cli_flags: ["--flag", ...]` attrs
    // (producer = the owning symbol)
    for e in view.entities_of_kind(scc_core::kinds::SYMBOL) {
        let Some(flags) = e.attributes.get("cli_flags").and_then(|v| v.as_array()) else {
            continue;
        };
        let mut ops: Vec<String> = flags
            .iter()
            .filter_map(|f| f.as_str().map(String::from))
            .collect();
        ops.sort();
        ops.dedup();
        if ops.is_empty() {
            continue;
        }
        push_contract(
            &mut contracts,
            &mut contract_seen,
            scc_core::Contract {
                id: scc_core::entity_id(&store.repo_id, scc_core::kinds::CONTRACT, &format!("cli:{}", e.name)),
                kind: "cli".into(),
                subclass: ContractSubclass::Cli,
                producer: e.id.clone(),
                consumers: Vec::new(),
                operations: ops,
                evidence: e.evidence.clone(),
            },
        );
    }

    // event: TOPIC entities with PUBLISHES/SUBSCRIBES edges (producer = the
    // topic; consumers = the publishing/subscribing symbols)
    for t in view.entities_of_kind(scc_core::kinds::TOPIC) {
        let mut consumers: BTreeSet<String> = BTreeSet::new();
        let mut any = false;
        for pred in [
            scc_core::predicates::PUBLISHES,
            scc_core::predicates::SUBSCRIBES,
            scc_core::predicates::CONSUMES,
        ] {
            for rel in view.in_pred(&t.id, pred) {
                consumers.insert(entity_name(view, &rel.subject));
                any = true;
            }
        }
        if !any {
            continue;
        }
        push_contract(
            &mut contracts,
            &mut contract_seen,
            scc_core::Contract {
                id: t.id.clone(),
                kind: "event".into(),
                subclass: ContractSubclass::Event,
                producer: t.id.clone(),
                consumers: consumers.into_iter().collect(),
                operations: vec![t.name.clone()],
                evidence: t.evidence.clone(),
            },
        );
    }

    // config: CONFIGURATION entities (producer = the owning symbol via
    // CONFIGURED_BY; consumers = READS edges + the configured-by symbols)
    for c in view.entities_of_kind(scc_core::kinds::CONFIGURATION) {
        let mut owners: Vec<String> = view
            .out_pred(&c.id, scc_core::predicates::CONFIGURED_BY)
            .into_iter()
            .map(|r| r.object.clone())
            .collect();
        owners.sort();
        owners.dedup();
        let producer = owners
            .first()
            .cloned()
            .unwrap_or_else(|| c.id.clone());
        let mut consumers: BTreeSet<String> = BTreeSet::new();
        for pred in [
            scc_core::predicates::READS,
            scc_core::predicates::CONSUMES,
            scc_core::predicates::HANDLES,
        ] {
            for rel in view.in_pred(&c.id, pred) {
                consumers.insert(entity_name(view, &rel.subject));
            }
        }
        for o in &owners {
            consumers.insert(entity_name(view, o));
        }
        push_contract(
            &mut contracts,
            &mut contract_seen,
            scc_core::Contract {
                id: c.id.clone(),
                kind: "config".into(),
                subclass: ContractSubclass::Configuration,
                producer,
                consumers: consumers.into_iter().collect(),
                operations: vec![c.name.clone()],
                evidence: c.evidence.clone(),
            },
        );
    }

    // subclass contracts: CONTRACT entities carrying a first-class
    // registration kind (serialization/extension/plugin/rpc/message/schema/
    // call/factory/builder/...). Framework-specific registration kinds
    // (`include_router`, `add_middleware`, ...) map to None and stay
    // public-api: EXPORT entities whose export kind is a callable signature
    // (function/method/constructor) — the "public fn signature" surface.
    // The EXPORT entity's symbol is the EXPORTS edge subject; consumers are
    // the symbols that call it.
    for e in view.entities_of_kind(scc_core::kinds::EXPORT) {
        let kind_attr = e
            .attributes
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !matches!(kind_attr, "function" | "method" | "constructor") {
            continue;
        }
        let symbol = view
            .in_pred(&e.id, scc_core::predicates::EXPORTS)
            .into_iter()
            .next()
            .map(|r| r.subject.clone())
            .unwrap_or_default();
        let mut consumers: BTreeSet<String> = BTreeSet::new();
        for pred in [
            scc_core::predicates::CALLS,
            scc_core::predicates::CONSUMES,
            scc_core::predicates::HANDLES,
        ] {
            for rel in view.in_pred(&symbol, pred) {
                consumers.insert(entity_name(view, &rel.subject));
            }
        }
        push_contract(
            &mut contracts,
            &mut contract_seen,
            scc_core::Contract {
                id: e.id.clone(),
                kind: "public-api".into(),
                subclass: ContractSubclass::PublicApi,
                producer: symbol,
                consumers: consumers.into_iter().collect(),
                operations: vec![e.name.clone()],
                evidence: e.evidence.clone(),
            },
        );
    }

    // subclass contracts: CONTRACT entities carrying a first-class
    // registration kind (serialization/extension/plugin/rpc/message/schema/
    // call/factory/builder/...). Framework-specific registration kinds
    // (`include_router`, `add_middleware`, ...) map to None and stay
    // framework semantics (FRAMEWORK SEMANTICS), never first-class
    // contracts. Annotations are per-symbol framework semantics too — they
    // render under FRAMEWORK SEMANTICS, not here. Producer = the
    // registering symbol (REGISTERS subject); consumers = symbols consuming
    // the surface.
    for ce in view.entities_of_kind(scc_core::kinds::CONTRACT) {
        let Some(kind_attr) = ce
            .attributes
            .get("kind")
            .and_then(|v| v.as_str())
            .map(str::to_string)
        else {
            continue;
        };
        let Some(subclass) = ContractSubclass::from_kind_str(&kind_attr) else {
            continue;
        };
        let producer = view
            .in_pred(&ce.id, scc_core::predicates::REGISTERS)
            .into_iter()
            .next()
            .map(|r| r.subject.clone())
            .unwrap_or_default();
        let mut consumers: BTreeSet<String> = BTreeSet::new();
        for pred in [
            scc_core::predicates::CONSUMES,
            scc_core::predicates::READS,
            scc_core::predicates::HANDLES,
        ] {
            for rel in view.in_pred(&ce.id, pred) {
                consumers.insert(entity_name(view, &rel.subject));
            }
        }
        push_contract(
            &mut contracts,
            &mut contract_seen,
            scc_core::Contract {
                id: ce.id.clone(),
                kind: subclass.as_str().to_string(),
                subclass,
                producer,
                consumers: consumers.into_iter().collect(),
                operations: vec![ce.name.clone()],
                evidence: ce.evidence.clone(),
            },
        );
    }
    // schema: SCHEMA concepts (Wave 11/13) — the schema name, its composed
    // parents (COMPOSES edges, schema→parent) and validation lines (the
    // defining owner's VALIDATES edges) render per-subclass with the
    // `schema:` prefix: `schema: User`, `schema: User extends Base`,
    // `schema: User validates`. Producer = the most frequent occurrence
    // owner (a symbol, never the concept/expr itself).
    //
    // Inline constructions (name == expr, the `z.object({...})` test and
    // handler forms) are frequency-capped: only the *repeated* DSL surface
    // (count >= 2, top 40 by count) renders — one-off test schemas are
    // noise, not architecture, and would flood the contracts layer. The
    // count is DERIVED from the live OCCURRENCE entities — never a stored,
    // write-time-mutated counter.
    let mut inline: Vec<(usize, String)> = Vec::new();
    for s in view.entities_of_kind(scc_core::kinds::SCHEMA) {
        let count = scc_graph::state::occurrence_count(view.graph, &s.id);
        let expr = s
            .attributes
            .get("expr")
            .and_then(|v| v.as_str())
            .map(|e| e.to_string());
        let is_inline = expr.as_deref() == Some(s.name.as_str());
        let producer = scc_graph::state::occurrence_producer(view.graph, &s.id)
            .unwrap_or_else(|| s.id.clone());
        if is_inline {
            inline.push((count, s.id.clone()));
            continue;
        }
        let owner = view
            .in_pred(&s.id, scc_core::predicates::DEFINES)
            .into_iter()
            .next()
            .map(|r| r.subject.clone());
        let mut ops: Vec<String> = vec![s.name.clone()];
        // the defining expression (`z.object({ name: z.string() })`)
        // renders as `schema: <name> = <expr>` when the extractor
        // captured one — the concrete code form a human would quote.
        // Inline constructions use the expression itself as the name
        // (`schema: z.object({...})`); never render `X = X`.
        if let Some(e) = &expr {
            if !e.is_empty() && e != &s.name {
                ops.push(format!("{} = {}", s.name, e));
            }
        }
        let mut composed: Vec<String> = view
            .out_pred(&s.id, scc_core::predicates::COMPOSES)
            .into_iter()
            .map(|r| format!("{} extends {}", s.name, entity_name(view, &r.object)))
            .collect();
        composed.sort();
        composed.dedup();
        ops.extend(composed);
        if let Some(owner_id) = &owner {
            let mut validated: Vec<String> = view
                .out_pred(owner_id, scc_core::predicates::VALIDATES)
                .into_iter()
                .map(|_| format!("{} validates", s.name))
                .collect();
            validated.sort();
            validated.dedup();
            ops.extend(validated);
        }
        push_contract(
            &mut contracts,
            &mut contract_seen,
            scc_core::Contract {
                id: s.id.clone(),
                kind: "schema".into(),
                subclass: ContractSubclass::Schema,
                producer,
                consumers: Vec::new(),
                operations: ops,
                evidence: s.evidence.clone(),
            },
        );
    }
    // repeated inline DSL forms (count desc, id asc for determinism)
    inline.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    for (count, id) in inline.into_iter().take(40) {
        if count < 2 {
            continue;
        }
        let Some(s) = view.entity(&id) else { continue };
        let producer = scc_graph::state::occurrence_producer(view.graph, &s.id)
            .unwrap_or_else(|| s.id.clone());
        push_contract(
            &mut contracts,
            &mut contract_seen,
            scc_core::Contract {
                id: s.id.clone(),
                kind: "schema".into(),
                subclass: ContractSubclass::Schema,
                producer,
                consumers: Vec::new(),
                operations: vec![s.name.clone()],
                evidence: s.evidence.clone(),
            },
        );
    }
    // deterministic order for the machine model: per-subclass groups,
    // then by operation, then producer
    contracts.sort_by(|a, b| {
        a.subclass
            .as_str()
            .cmp(b.subclass.as_str())
            .then(a.operations.join("\u{1}").cmp(&b.operations.join("\u{1}")))
            .then(a.producer.cmp(&b.producer))
    });

    // ---- flows: SEQUENCES project from the canonical FlowGraph (P1 §18);
    // the old linear flows table is never the atlas's sequence source ----
    let mut flows: Vec<AtlasFlow> = Vec::new();
    let mut async_boundaries: BTreeSet<String> = BTreeSet::new();
    for g in store.flow_graphs().unwrap_or_default() {
        if g.kind != scc_core::FlowKind::Sequence {
            continue;
        }
        let steps = project_flow_graph(view, &g, &mut async_boundaries);
        if steps.is_empty() {
            continue;
        }
        flows.push(AtlasFlow {
            name: g.name.clone(),
            kind: g.kind,
            trigger: g.trigger.clone(),
            steps,
        });
    }
    // derived views (workflow/dataflow/lifecycle) still come from the
    // derived compilers — but lifecycle is SIGNALS, never authoritative
    for f in view.flows() {
        if f.kind == scc_core::FlowKind::Sequence {
            continue; // sequences come from the canonical graphs
        }
        let mut steps: Vec<String> = Vec::new();
        let mut prev_actor: Option<String> = None;
        for s in &f.steps {
            let actor = entity_name(view, &s.actor);
            let mut line = if prev_actor.as_deref() == Some(actor.as_str()) {
                format!("  -> {}", s.operation)
            } else {
                format!("{}: {}", actor, s.operation)
            };
            if s.r#async == Some(true) {
                line.push_str(" [async]");
            }
            if let Some(c) = &s.condition {
                line.push_str(&format!(" (if {c})"));
            }
            if let Some(rp) = &s.retry_policy {
                line.push_str(&format!(" [retry: {rp}]"));
            }
            if let Some(fo) = &s.failure_outcome {
                line.push_str(&format!(" [fail: {fo}]"));
            }
            steps.push(line);
            prev_actor = Some(actor);
        }
        if f.attributes.get("signals_only").and_then(|v| v.as_bool()) == Some(true) {
            flows.push(AtlasFlow {
                name: format!("{} (LIFECYCLE SIGNALS — NOT VERIFIED TRANSITIONS)", f.name),
                kind: f.kind,
                trigger: f.trigger.clone(),
                steps,
            });
        } else {
            flows.push(AtlasFlow {
                name: f.name.clone(),
                kind: f.kind,
                trigger: f.trigger.clone(),
                steps,
            });
        }
    }
    flows.sort_by(|a, b| a.name.cmp(&b.name));

    // ---- invariants ----
    let invariants: Vec<AtlasInvariant> = view
        .invariants()
        .into_iter()
        .map(|i| AtlasInvariant {
            statement: i.statement,
            severity: i.severity,
        })
        .collect();

    // ---- deployment / externals / trust boundaries ----
    let deployment_units: Vec<String> = view
        .entities_of_kind(scc_core::kinds::DEPLOYMENT_UNIT)
        .into_iter()
        .map(|e| {
            let img = e
                .attributes
                .get("image")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if img.is_empty() {
                e.name.clone()
            } else {
                format!("{} ({})", e.name, img)
            }
        })
        .collect();
    let external_systems: Vec<String> = view
        .entities_of_kind(scc_core::kinds::EXTERNAL_API)
        .into_iter()
        .map(|e| e.name.clone())
        .collect();
    let trust_boundaries: Vec<String> =
        scc_graph::boundaries::boundary_crossings(view.graph, store)
            .unwrap_or_default();

    // ---- implementation map ----
    let mut implementation_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for c in &components {
        implementation_map.insert(c.name.clone(), c.implementation_paths.clone());
    }

    // ---- evidence + warnings + freshness ----
    let comp_ids: Vec<String> = view
        .components()
        .into_iter()
        .map(|c| c.id.clone())
        .collect();
    let evidence_summary = ctx.evidence_summary(&comp_ids);

    let mut warnings: Vec<String> = Vec::new();
    let stale = view.stale_paths();
    if snapshot.is_some() {
        if stale.is_empty() {
            warnings.push("Model is FRESH".into());
        } else {
            warnings.push(format!(
                "Model is stale: {} changed file(s) not yet re-indexed.",
                stale.len()
            ));
        }
    } else {
        warnings.push("Repository is NOT indexed — run `scc index`.".into());
    }
    warnings.extend(view.stale_warnings());

    let freshness = if snapshot.is_none() {
        "NOT INDEXED".to_string()
    } else if stale.is_empty() {
        "FRESH".to_string()
    } else {
        format!("STALE ({})", stale.len())
    };

    // ---- Ontology phase: archetype + STATE & DATA AUTHORITY + hierarchy ----
    let archetype = Some(scc_graph::archetype::detect_archetype(view.graph, store));

    // symbol -> component name over the *stored* components (the state
    // compiler attributes ownership per component from the fact layer)
    let mut symbol_comp: HashMap<String, String> = HashMap::new();
    for c in view.components() {
        for r in view.out_pred(&c.id, scc_core::predicates::CONTAINS) {
            for sr in view.out_pred(&r.object, scc_core::predicates::CONTAINS) {
                symbol_comp.insert(sr.object.clone(), c.name.clone());
            }
        }
    }
    let state_authority = scc_graph::state::compile_state_authority(view.graph, &symbol_comp);

    // hierarchical containers: services first, then subsystems; members are
    // direct member entity ids (component ids or nested subsystem ids)
    let mut hierarchy: Vec<AtlasHierarchyNode> = Vec::new();
    for kind in [scc_core::kinds::SERVICE, scc_core::kinds::SUBSYSTEM] {
        for e in view.graph.entities_of_kind(kind) {
            let mut members: Vec<String> = view
                .graph
                .out_pred(&e.id, scc_core::predicates::CONTAINS)
                .into_iter()
                .map(|r| r.object.clone())
                .collect();
            members.sort();
            hierarchy.push(AtlasHierarchyNode {
                id: e.id.clone(),
                name: e.name.clone(),
                kind: kind.to_string(),
                members,
            });
        }
    }
    hierarchy.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.cmp(&b.name)));

    // ---- STATE & DATA AUTHORITY: structured bridge ----
    // The state compiler's per-component claims (mutable fields, STATE/
    // REGISTRY entities, configuration targets, topics, middleware/registry
    // registrations, store writes) become component `owns` claims too, so
    // the state fact layer is part of the machine model — not just rendered
    // text. Provenance preserved; deduped by (target, provenance).
    let state_claims = scc_graph::state::compile_state_claims(view.graph, &symbol_comp);
    for claim in state_claims {
        let Some(c) = components
            .iter_mut()
            .find(|c| c.name == claim.component)
        else {
            continue;
        };
        let seen_claim = (claim.target.clone(), claim.provenance.clone());
        if !c.owns.iter().any(|o| {
            o.target == seen_claim.0 && o.provenance == seen_claim.1
        }) {
            c.owns.push(AtlasOwnershipClaim {
                target: claim.target,
                provenance: claim.provenance,
            });
        }
    }
    for c in &mut components {
        c.owns.sort_by(|a, b| a.target.cmp(&b.target).then(a.provenance.cmp(&b.provenance)));
    }

    // ---- PUBLIC API (Wave 10): exports grouped by component ----
    // EXPORT entities (extractor-emitted public-export facts) plus symbols
    // the extractor statically marked `exported: true` at module level.
    // Grouped by the exporting symbol's component.
    let mut public_api: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    // EXPORT entities: the exporting symbol is the EXPORTS relationship
    // subject; its name matches the export entity name.
    for r in view.all_rels() {
        if r.predicate != scc_core::predicates::EXPORTS {
            continue;
        }
        let Some(comp) = symbol_comp.get(&r.subject) else { continue };
        if let Some(name) = view.entity(&r.object).map(|e| e.name.clone()) {
            if !name.is_empty() {
                public_api.entry(comp.clone()).or_default().insert(name);
            }
        }
    }
    // exported module-level symbols (`exported: true`, no `.` in the name)
    for e in view.entities_of_kind(scc_core::kinds::SYMBOL) {
        if e.name.is_empty() || e.name.starts_with('_') || e.name.contains('.') {
            continue;
        }
        if e.attributes.get("exported").and_then(|v| v.as_bool()) != Some(true) {
            continue;
        }
        let Some(comp) = symbol_comp.get(&e.id) else { continue };
        public_api.entry(comp.clone()).or_default().insert(e.name.clone());
    }
    let public_api: BTreeMap<String, Vec<String>> = public_api
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect();

    // ---- FRAMEWORK SEMANTICS (Wave 10): annotations / registrations /
    // callbacks grouped by component ----
    let mut framework_semantics: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    // annotations: ANNOTATION entity ANNOTATES target symbol
    for a in view.entities_of_kind(scc_core::kinds::ANNOTATION) {
        let mut rels = view.out_pred(&a.id, scc_core::predicates::ANNOTATES);
        rels.sort_by(|x, y| x.object.cmp(&y.object));
        for r in rels {
            if let Some(comp) = symbol_comp.get(&r.object) {
                let target = entity_name(view, &r.object);
                framework_semantics
                    .entry(comp.clone())
                    .or_default()
                    .insert(format!("annotates {target} ({})", a.name));
            }
        }
    }
    // REGISTERS + HANDLES_CALLBACK: symbol -> target
    for pred in [
        scc_core::predicates::REGISTERS,
        scc_core::predicates::HANDLES_CALLBACK,
    ] {
        let mut rels = view.all_rels().to_vec();
        rels.sort_by(|x, y| {
            x.subject
                .cmp(&y.subject)
                .then(x.object.cmp(&y.object))
                .then(x.id.cmp(&y.id))
        });
        for r in rels {
            if r.predicate != pred {
                continue;
            }
            let Some(comp) = symbol_comp.get(&r.subject) else { continue };
            let target = entity_name(view, &r.object);
            let line = if pred == scc_core::predicates::REGISTERS {
                format!("registers {target}")
            } else {
                format!("handles callback {target}")
            };
            framework_semantics.entry(comp.clone()).or_default().insert(line);
        }
    }
    let framework_semantics: BTreeMap<String, Vec<String>> = framework_semantics
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect();

    // ---- PIPELINE (Wave 10): phase-named symbols grouped by stage ----
    // Rendered only for the CompilerLanguageTool archetype: symbols whose
    // name contains a phase verb, plus phase-named files (`1-parse`-style
    // stage directories). Grouped by stage; bounded.
    let pipeline = build_pipeline(view, archetype);

    // ---- LANDMARKS (Wave 10): notable exports + annotated targets,
    // bounded (~40) ----
    let landmarks = build_landmarks(view, &public_api, &symbol_comp);

    SystemAtlas {
        repository: repo.name,
        revision: snapshot
            .as_ref()
            .map(|s| s.revision.clone())
            .unwrap_or_else(|| "not-indexed".to_string()),
        indexed_at: snapshot
            .map(|s| s.indexed_at)
            .unwrap_or_default(),
        freshness,
        purpose,
        components,
        entrypoints,
        contracts,
        coverage: compute_coverage(ctx),
        flows,
        invariants,
        deployment_units,
        external_systems,
        trust_boundaries,
        async_boundaries: async_boundaries.into_iter().collect(),
        implementation_map,
        data_stores: data_stores.into_iter().collect(),
        archetype,
        state_authority,
        hierarchy,
        evidence_summary,
        warnings,
        public_api,
        framework_semantics,
        pipeline,
        landmarks,
    }
}

/// Phase-stage verbs for the PIPELINE section (CompilerLanguageTool
/// archetype): a symbol whose name contains a stage verb is a phase symbol.
const PIPELINE_STAGES: [(&str, &[&str]); 5] = [
    ("parse", &["parse", "parser", "lexer", "lex", "tokenize", "tokeniser", "ast"]),
    ("analyze", &["analyze", "analyse", "analysis"]),
    ("transform", &["transform", "lower", "resolve", "resolveconfig"]),
    ("generate", &["generate", "generator", "codegen", "compile", "compiler"]),
    ("emit", &["emit", "print", "format", "formatdoc", "serialize"]),
];

/// PIPELINE (Wave 10): phase-named symbols grouped by stage, plus
/// phase-named file paths (`1-parse`-style stage dirs). Only rendered for
/// the CompilerLanguageTool archetype. Deterministic: sorted by
/// (stage-rank, name); bounded to keep the section compact.
fn build_pipeline(view: &TrustedGraphView, archetype: Option<scc_core::Archetype>) -> Vec<String> {
    if archetype != Some(scc_core::Archetype::CompilerLanguageTool) {
        return Vec::new();
    }
    let mut lines: Vec<(usize, String)> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let stage_of = |name: &str| -> Option<usize> {
        let lower = name.to_ascii_lowercase();
        PIPELINE_STAGES
            .iter()
            .position(|(_, verbs)| verbs.iter().any(|v| lower.contains(v)))
    };
    // phase-named symbols (module-level and method symbols)
    for e in view.entities_of_kind(scc_core::kinds::SYMBOL) {
        if e.name.is_empty() {
            continue;
        }
        let Some(rank) = stage_of(&e.name) else { continue };
        if !seen.insert(e.name.clone()) {
            continue;
        }
        lines.push((rank, e.name.clone()));
    }
    // phase-named files: `phases/1-parse/index.js` or a `1-parse`-style
    // directory segment, or a path segment containing a stage verb
    for f in view.entities_of_kind(scc_core::kinds::FILE) {
        let name = f.name.clone();
        let lower = name.to_ascii_lowercase();
        let mut rank: Option<usize> = None;
        for (i, (_, verbs)) in PIPELINE_STAGES.iter().enumerate() {
            // digit-prefixed stage dirs: `1-parse`, `2-analyze`, `3-transform`
            let numbered = verbs.iter().any(|v| {
                lower.contains(&format!("/{v}"))
                    || lower
                        .split('/')
                        .any(|seg| {
                            let seg = seg.trim_start_matches(|c: char| c.is_ascii_digit());
                            seg.trim_start_matches(['-', '_']).starts_with(v)
                        })
            });
            if numbered {
                rank = Some(i);
                break;
            }
        }
        if rank.is_none() && lower.contains("/phases/") {
            rank = Some(0); // compiler phase tree without a matched verb
        }
        if let Some(r) = rank {
            if seen.insert(name.clone()) {
                lines.push((r, name));
            }
        }
    }
    lines.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut out: Vec<String> = Vec::new();
    let mut current_stage: Option<&str> = None;
    for (rank, name) in lines.into_iter().take(96) {
        let stage = PIPELINE_STAGES[rank].0;
        if current_stage != Some(stage) {
            out.push(format!("[{}]", stage));
            current_stage = Some(stage);
        }
        out.push(format!("  {name}"));
    }
    out
}

/// LANDMARKS (Wave 10): notable exports + annotated targets, bounded (~40).
/// Exports: the component-sorted public API, preferring classes then
/// functions, capped. Annotated targets: symbols an ANNOTATION/REGISTERS
/// fact targets (framework-decorated code). Deterministic: sorted.
fn build_landmarks(
    view: &TrustedGraphView,
    public_api: &BTreeMap<String, Vec<String>>,
    symbol_comp: &HashMap<String, String>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    // notable exports: exported classes first, then other exports, capped
    let mut classes: Vec<String> = Vec::new();
    let mut others: Vec<String> = Vec::new();
    for names in public_api.values() {
        for n in names {
            let kind = view
                .entities_of_kind(scc_core::kinds::SYMBOL)
                .into_iter()
                .find(|e| e.name == *n)
                .and_then(|e| e.attributes.get("kind"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let is_class = matches!(
                kind,
                "class" | "struct" | "trait" | "interface" | "enum" | "type" | "module" | "model"
            );
            if is_class {
                classes.push(n.clone());
            } else {
                others.push(n.clone());
            }
        }
    }
    classes.sort();
    others.sort();
    let mut pool: Vec<String> = classes;
    pool.extend(others);
    for n in pool.into_iter().take(24) {
        if seen.insert(n.clone()) {
            out.push(format!("export {}", n));
        }
    }
    // annotated targets (framework-decorated symbols), capped
    let mut targets: Vec<String> = Vec::new();
    for a in view.entities_of_kind(scc_core::kinds::ANNOTATION) {
        for r in view.out_pred(&a.id, scc_core::predicates::ANNOTATES) {
            if let Some(comp) = symbol_comp.get(&r.object) {
                let name = entity_name(view, &r.object);
                if !name.is_empty() && !name.starts_with('_') {
                    targets.push(format!("{name} (@{})", a.name));
                }
                let _ = comp;
            }
        }
    }
    targets.sort();
    targets.dedup();
    for t in targets.into_iter().take(16) {
        if seen.insert(t.clone()) {
            out.push(t);
        }
    }
    out
}

/// Push one contract, merging consumers/evidence when the same
/// (subclass, operations) surface was already recorded (e.g. the same CLI
/// flag owned by two symbols). Deterministic: consumers/evidence stay
/// sorted.
fn push_contract(
    contracts: &mut Vec<scc_core::Contract>,
    seen: &mut BTreeMap<(String, String), usize>,
    c: scc_core::Contract,
) {
    let key = (c.subclass.as_str().to_string(), c.operations.join("\u{1}"));
    if let Some(&idx) = seen.get(&key) {
        let existing = &mut contracts[idx];
        for s in c.consumers {
            if !existing.consumers.contains(&s) {
                existing.consumers.push(s);
            }
        }
        for e in c.evidence {
            if !existing.evidence.contains(&e) {
                existing.evidence.push(e);
            }
        }
        existing.consumers.sort();
        existing.evidence.sort();
        return;
    }
    seen.insert(key, contracts.len());
    contracts.push(c);
}

/// Languages with a real extractor (the indexer's language map). Files in
/// any other language are scanned but never parsed — the honest `unparsed`
/// remainder of the coverage map.
const EXTRACTOR_LANGUAGES: [&str; 6] = [
    "python",
    "typescript",
    "javascript",
    "go",
    "java",
    "rust",
];

/// Deterministic model-coverage facts (Wave 9): what the model knows AND
/// what it does not. Every line is computed from the trusted view + store —
/// no heuristics, no fabrication; when a quantity is unobservable the line
/// says so explicitly.
fn compute_coverage(ctx: &ContextCompiler) -> BTreeMap<String, String> {
    let view = &ctx.view;
    let store = ctx.store;
    let mut out: BTreeMap<String, String> = BTreeMap::new();

    // ---- parsed source files % ----
    let files = store.all_files().unwrap_or_default();
    let total = files.len();
    let parsed = files
        .iter()
        .filter(|(_, _, lang, _, _)| EXTRACTOR_LANGUAGES.contains(&lang.as_str()))
        .count();
    let pct = parsed.checked_mul(100).map(|n| n / total.max(1)).unwrap_or(0);
    out.insert(
        "parsed_source_files".to_string(),
        format!("{pct}% ({parsed}/{total})"),
    );

    // ---- exported API identified ----
    let exports = view.entities_of_kind(scc_core::kinds::EXPORT);
    let export_edges = view
        .all_rels()
        .iter()
        .filter(|r| r.predicate == scc_core::predicates::EXPORTS)
        .count();
    out.insert(
        "exported_api".to_string(),
        if exports.is_empty() {
            "none (no EXPORTS evidence)".to_string()
        } else {
            format!("{} export entit{} ({} EXPORTS edges)", exports.len(), if exports.len() == 1 { "y" } else { "ies" }, export_edges)
        },
    );

    // ---- call targets resolved % ----
    // RESOLVED (compiler/LSP proof) + EXTRACTED calls with a target that
    // resolves to an existing entity (symbol or external API), over every
    // stored CALLS edge. Unresolved calls are never persisted, so the
    // interesting limit is the LSP-vs-candidate split plus the
    // external/dynamic receiver count below.
    let calls: Vec<&scc_core::Relationship> = view
        .all_rels()
        .into_iter()
        .filter(|r| r.predicate == scc_core::predicates::CALLS)
        .collect();
    let total_calls = calls.len();
    let lsp_resolved = calls
        .iter()
        .filter(|r| r.provenance == scc_core::Provenance::Resolved)
        .count();
    let with_target = calls
        .iter()
        .filter(|r| {
            matches!(
                r.provenance,
                scc_core::Provenance::Resolved | scc_core::Provenance::Extracted
            ) && view.entity(&r.object).is_some()
        })
        .count();
    let pct = with_target.checked_mul(100).map(|n| n / total_calls.max(1)).unwrap_or(0);
    out.insert(
        "call_targets_resolved".to_string(),
        if pct >= 100 {
            format!("{pct}% ({with_target}/{total_calls}, {lsp_resolved} LSP-RESOLVED)")
        } else {
            format!("{pct}% ({with_target}/{total_calls}, {lsp_resolved} LSP-RESOLVED) — exploration still justified in unresolved regions")
        },
    );

    // ---- dynamic receivers unresolved ----
    // Calls whose target is not a local symbol (external/dynamic receivers)
    // are stored with the external target; unknown-receiver calls are not
    // persisted at all — reported honestly as such.
    let unresolved = calls
        .iter()
        .filter(|r| {
            r.provenance != scc_core::Provenance::Resolved
                && view
                    .entity(&r.object)
                    .map(|e| e.kind != scc_core::kinds::SYMBOL)
                    .unwrap_or(true)
        })
        .count();
    out.insert(
        "dynamic_receivers_unresolved".to_string(),
        format!(
            "{unresolved} (calls whose target is not a local symbol; unknown-receiver calls are not persisted)"
        ),
    );

    // ---- invocation surfaces ----
    let surfaces = scc_graph::flows::invocation_surfaces(view.graph);
    let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
    for s in &surfaces {
        *by_kind.entry(s.kind.as_str()).or_insert(0) += 1;
    }
    let summary: Vec<String> = by_kind
        .iter()
        .map(|(k, v)| format!("{k} {v}"))
        .collect();
    out.insert(
        "invocation_surfaces".to_string(),
        format!("{} ({})", surfaces.len(), summary.join(", ")),
    );

    // ---- framework registrations unknown ----
    let known_regs = view
        .all_rels()
        .iter()
        .filter(|r| r.predicate == scc_core::predicates::REGISTERS)
        .count();
    out.insert(
        "framework_registrations_unknown".to_string(),
        format!("0 ({known_regs} known registrations — unknown surfaces only reported with registry evidence)"),
    );

    // ---- stale evidence ----
    let stale = view.stale_paths();
    out.insert(
        "stale_evidence".to_string(),
        if stale.is_empty() {
            "0 (model FRESH)".to_string()
        } else {
            format!("{} changed file(s)", stale.len())
        },
    );

    // ---- unparsed files ----
    let unparsed = files
        .iter()
        .filter(|(_, _, lang, _, _)| !EXTRACTOR_LANGUAGES.contains(&lang.as_str()))
        .count();
    out.insert(
        "unparsed_files".to_string(),
        format!("{unparsed} (config/docs/infra — scanned but not source-parsed)"),
    );

    // ---- model epoch generations ----
    let epoch = store.model_epoch().unwrap_or(scc_store::ModelEpoch::zero());
    let gens = epoch.source
        + epoch.semantic
        + epoch.evidence
        + epoch.intent
        + epoch.runtime
        + epoch.derived;
    out.insert(
        "model_epoch_generations".to_string(),
        format!(
            "{gens} (source {}, semantic {}, evidence {}, intent {}, runtime {}, derived {})",
            epoch.source, epoch.semantic, epoch.evidence, epoch.intent, epoch.runtime, epoch.derived
        ),
    );

    out
}

/// Render the atlas as compact structured text (agent-facing).
// trace:v1 id=impl.scc.atlas.render work=WORK-SCC-001 satisfies=REQ-SCC-CTX
pub fn render_atlas(ctx: &ContextCompiler, atlas: &SystemAtlas, budget: usize) -> ContextPack {
    let mut pack = ContextPack::new("atlas", &atlas.revision);
    let mut sections: Vec<Section> = Vec::new();

    // SYSTEM PURPOSE (never cut); the ARCHETYPE header is the ontology
    // phase's one-line classification of the repository.
    let mut purpose = String::new();
    purpose.push_str(&format!(
        "ARCHETYPE: {}\n",
        atlas
            .archetype
            .map(|a| a.as_str())
            .unwrap_or(Archetype::Unknown.as_str())
    ));
    if !atlas.purpose.is_empty() {
        purpose.push_str(&format!(
            "[SYSTEM PURPOSE — from README, DOCUMENTATION not fact]\n{}\n",
            atlas.purpose
        ));
    }
    if !atlas.entrypoints.is_empty() {
        purpose.push_str("ENTRYPOINTS\n");
        // bounded render: the full structured list stays in the machine
        // model; the agent-facing artifact caps the listing (a framework
        // repo can have thousands of export surfaces).
        const EP_RENDER_CAP: usize = 200;
        for e in atlas.entrypoints.iter().take(EP_RENDER_CAP) {
            // Framework-surface kinds render as compact `kind: name` lines
            // (`queue: consume_order`, `schedule: daily_job`,
            // `plugin: register_hook`, `lifecycle: @BeforeAll` — the
            // lifecycle line names the hook annotation). Classic
            // http/cli/public_api/route/entrypoint lines keep the
            // `name [kind] — trigger` form.
            match e.kind.as_str() {
                "queue" | "schedule" | "plugin" | "lifecycle" => {
                    let label = if e.kind == "lifecycle" {
                        e.trigger
                            .strip_prefix("lifecycle:")
                            .map(|a| format!("@{a}"))
                            .unwrap_or_else(|| e.name.clone())
                    } else {
                        e.name.clone()
                    };
                    purpose.push_str(&format!("  {}: {}\n", e.kind, label));
                }
                _ => {
                    if e.trigger == e.name {
                        purpose.push_str(&format!("  {} [{}]\n", e.name, e.kind));
                    } else {
                        purpose.push_str(&format!("  {} [{}] — {}\n", e.name, e.kind, e.trigger));
                    }
                }
            }
        }
        if atlas.entrypoints.len() > EP_RENDER_CAP {
            purpose.push_str(&format!(
                "  ... +{} more (full list in the machine model)\n",
                atlas.entrypoints.len() - EP_RENDER_CAP
            ));
        }
    }
    sections.push(Section::new("SYSTEM PURPOSE", purpose, 10));

    // ARCHITECTURE (component blocks), grouped by layer: services first,
    // then subsystems, then unmerged components, code-regions last — with
    // `parent` indentation under service/subsystem headers. The flat block
    // format is preserved inside each group.
    let mut arch = String::new();
    let mut rendered: BTreeSet<String> = BTreeSet::new(); // names under containers
    let comp_block = |c: &AtlasComponent, indent: &str| -> String {
        let mut out = format!("\n{}{}", indent, c.name.to_uppercase());
        if !c.purpose.is_empty() {
            out.push_str(&format!("\n{}Purpose: {}", indent, c.purpose));
        }
        if !c.implementation_paths.is_empty() {
            out.push_str(&format!(
                "\n{}Implementation: {}",
                indent,
                c.implementation_paths.join(", ")
            ));
            if !c.symbols.is_empty() {
                out.push_str(&format!(" ({} member symbols)", c.symbols.len()));
            }
        }
        if !c.consumes.is_empty() {
            out.push_str(&format!("\n{}Consumes: {}", indent, c.consumes.join(", ")));
        }
        if !c.produces.is_empty() {
            out.push_str(&format!("\n{}Produces: {}", indent, c.produces.join(", ")));
        }
        if !c.upstream.is_empty() {
            out.push_str(&format!("\n{}Upstream: {}", indent, c.upstream.join(", ")));
        }
        if !c.downstream.is_empty() {
            out.push_str(&format!("\n{}Downstream: {}", indent, c.downstream.join(", ")));
        }
        if !c.owns.is_empty() {
            let owned: Vec<String> = c
                .owns
                .iter()
                .map(|o| format!("{} ({})", o.target, o.provenance))
                .collect();
            out.push_str(&format!("\n{}Owns: {}", indent, owned.join(", ")));
        }
        out.push('\n');
        out
    };
    let name_of = |id: &str| -> Option<String> {
        ctx.view
            .entity(id)
            .map(|e| e.name.clone())
    };
    // services first: nested subsystems, then directly-contained components
    for svc in atlas.hierarchy.iter().filter(|n| n.kind == "service") {
        arch.push_str(&format!("\nSERVICE {}\n", svc.name.to_uppercase()));
        for m in &svc.members {
            let Some(sub) = atlas.hierarchy.iter().find(|n| &n.id == m) else {
                continue;
            };
            if sub.kind != "subsystem" {
                continue;
            }
            arch.push_str(&format!("  SUBSYSTEM {}\n", sub.name.to_uppercase()));
            for cm in &sub.members {
                if let Some(name) = name_of(cm) {
                    rendered.insert(name.clone());
                    if let Some(c) = atlas.components.iter().find(|c| c.name == name) {
                        arch.push_str(&format!("    {}\n", comp_block(c, "    ").trim_end()));
                    }
                }
            }
            arch.push('\n');
        }
        for m in &svc.members {
            if atlas.hierarchy.iter().any(|n| &n.id == m) {
                continue; // subsystems rendered above
            }
            if let Some(name) = name_of(m) {
                rendered.insert(name.clone());
                if let Some(c) = atlas.components.iter().find(|c| c.name == name) {
                    arch.push_str(&format!("  {}\n", comp_block(c, "  ").trim_end()));
                }
            }
        }
    }
    // standalone subsystems (not nested inside a service)
    for sub in atlas.hierarchy.iter().filter(|n| n.kind == "subsystem") {
        if atlas
            .hierarchy
            .iter()
            .any(|n| n.kind == "service" && n.members.contains(&sub.id))
        {
            continue;
        }
        arch.push_str(&format!("\nSUBSYSTEM {}\n", sub.name.to_uppercase()));
        for cm in &sub.members {
            if let Some(name) = name_of(cm) {
                rendered.insert(name.clone());
                if let Some(c) = atlas.components.iter().find(|c| c.name == name) {
                    arch.push_str(&format!("  {}\n", comp_block(c, "  ").trim_end()));
                }
            }
        }
        arch.push('\n');
    }
    // unmerged components (evidence-backed), then bare code regions
    for layer in ["component", "code_region"] {
        for c in &atlas.components {
            if rendered.contains(&c.name) {
                continue;
            }
            if c.layer != layer {
                continue;
            }
            arch.push_str(&comp_block(c, ""));
        }
    }
    sections.push(Section::new("ARCHITECTURE", arch, 9));

    // PRIMARY FLOWS (never cut); the architecture view is the ARCHITECTURE
    // section itself — skip it here to avoid duplicating the system. The
    // rendered section is bounded so a chain-rich repo's FLOWS view stays
    // compact: the deepest flows (most step lines) render first, capped at
    // FLOW_RENDER_CAP flows and FLOW_RENDER_STEP_CAP lines each. The
    // machine model (`atlas.flows`) carries the full inventory, so the
    // structured behavior layer is unaffected by the render cap.
    const FLOW_RENDER_CAP: usize = 32;
    const FLOW_RENDER_STEP_CAP: usize = 16;
    let mut flows = String::new();
    let mut render_flows: Vec<&AtlasFlow> = atlas
        .flows
        .iter()
        .filter(|f| f.kind != FlowKind::Architecture)
        .collect();
    render_flows.sort_by(|a, b| {
        b.steps
            .len()
            .cmp(&a.steps.len())
            .then(a.name.cmp(&b.name))
    });
    for f in render_flows.into_iter().take(FLOW_RENDER_CAP) {
        flows.push_str(&format!(
            "\n{} [{}]",
            f.name,
            flow_kind_str(f.kind)
        ));
        if let Some(t) = &f.trigger {
            flows.push_str(&format!("\nTrigger: {t}"));
        }
        for s in f.steps.iter().take(FLOW_RENDER_STEP_CAP) {
            flows.push_str(&format!("\n{s}"));
        }
        if f.steps.len() > FLOW_RENDER_STEP_CAP {
            flows.push_str(&format!(
                "\n... +{} more steps",
                f.steps.len() - FLOW_RENDER_STEP_CAP
            ));
        }
        flows.push('\n');
    }
    sections.push(Section::new("FLOWS", flows, 9));

    // STATE & DATA AUTHORITY (never cut): six subsections — DATA
    // OWNERSHIP (persistent: the write-derived + declared owns claims and
    // the DATA STORES list), RUNTIME STATE, REACTIVE STATE,
    // CONFIGURATION, CACHES, DERIVED / REGISTRIES. Falls back to the
    // legacy DATA OWNERSHIP title when the state compiler found no state
    // at all.
    let has_state = atlas
        .state_authority
        .values()
        .any(|v| !v.is_empty());
    let mut state_body = String::new();
    if has_state {
        state_body.push_str("DATA OWNERSHIP\n");
    }
    for c in &atlas.components {
        for o in &c.owns {
            state_body.push_str(&format!(
                "{} owns {} ({})\n",
                c.name, o.target, o.provenance
            ));
        }
    }
    if !atlas.data_stores.is_empty() {
        state_body.push_str("\nDATA STORES\n");
        for s in &atlas.data_stores {
            state_body.push_str(&format!("  {s}\n"));
        }
    }
    for section in [
        scc_graph::state::S_RUNTIME,
        scc_graph::state::S_REACTIVE,
        scc_graph::state::S_CONFIGURATION,
        scc_graph::state::S_CACHES,
        scc_graph::state::S_DERIVED,
    ] {
        if let Some(lines) = atlas.state_authority.get(section) {
            if !lines.is_empty() {
                state_body.push_str(&format!(
                    "\n{}\n",
                    scc_graph::state::section_label(section)
                ));
                for l in lines {
                    state_body.push_str(&format!("  {l}\n"));
                }
            }
        }
    }
    sections.push(Section::new(
        if has_state {
            "STATE & DATA AUTHORITY"
        } else {
            "DATA OWNERSHIP"
        },
        state_body,
        10,
    ));

    // CONTRACTS (never cut) — rendered as per-subclass groups: one
    // `{subclass}: {operation}` line per operation, sorted so each subclass
    // family (http/cli/event/config/public-api/extension/serialization/...)
    // clusters together. Preserves the classic contract strings (route
    // `GET /api/x`, flag `--paging`, event `user.created`, config key
    // `DEBUG`) so pre-Wave-9 consumers keep matching.
    sections.push(Section::new(
        "CONTRACTS",
        if atlas.contracts.is_empty() {
            "(none)".into()
        } else {
            let mut lines: Vec<String> = Vec::new();
            for c in &atlas.contracts {
                let prefix = c.subclass.as_str();
                for op in &c.operations {
                    lines.push(format!("{prefix}: {op}"));
                }
            }
            lines.sort();
            lines.join("\n")
        },
        9,
    ));

    // PUBLIC API (Wave 10): exports grouped by component — compact
    // `component: exports A, B, C` lines from the semantic fact layer
    // (EXPORT entities + exported module-level symbols). Per-component
    // render is bounded (the structured model carries the full list).
    sections.push(Section::new(
        "PUBLIC API",
        if atlas.public_api.is_empty() {
            "(none)".into()
        } else {
            const API_RENDER_CAP: usize = 64;
            let mut lines: Vec<String> = Vec::new();
            for (comp, exports) in &atlas.public_api {
                if exports.is_empty() {
                    continue;
                }
                let shown: Vec<&str> = exports.iter().take(API_RENDER_CAP).map(|s| s.as_str()).collect();
                let mut line = format!("{}: exports {}", comp, shown.join(", "));
                if exports.len() > API_RENDER_CAP {
                    line.push_str(&format!(" (+{} more)", exports.len() - API_RENDER_CAP));
                }
                lines.push(line);
            }
            lines.join("\n")
        },
        6,
    ));

    // FRAMEWORK SEMANTICS (Wave 10): annotations on targets,
    // route/bean/middleware registrations, lifecycle callbacks — grouped
    // by component. Per-component render is bounded.
    sections.push(Section::new(
        "FRAMEWORK SEMANTICS",
        if atlas.framework_semantics.is_empty() {
            "(none)".into()
        } else {
            const SEM_RENDER_CAP: usize = 48;
            let mut lines: Vec<String> = Vec::new();
            for (comp, facts) in &atlas.framework_semantics {
                for f in facts.iter().take(SEM_RENDER_CAP) {
                    lines.push(format!("{comp}: {f}"));
                }
                if facts.len() > SEM_RENDER_CAP {
                    lines.push(format!(
                        "{comp}: (+{} more)",
                        facts.len() - SEM_RENDER_CAP
                    ));
                }
            }
            lines.join("\n")
        },
        6,
    ));

    // PIPELINE (Wave 10, CompilerLanguageTool archetype): phase-named
    // symbols grouped by stage.
    sections.push(Section::new(
        "PIPELINE",
        if atlas.pipeline.is_empty() {
            "(none)".into()
        } else {
            atlas.pipeline.join("\n")
        },
        6,
    ));

    // LANDMARKS (Wave 10, priority 5 — bounded ~40): notable exports and
    // annotated targets, one zoom level deeper than the component list.
    sections.push(Section::new(
        "LANDMARKS",
        if atlas.landmarks.is_empty() {
            "(none)".into()
        } else {
            let mut lines = atlas.landmarks.clone();
            lines.sort();
            lines.join("\n")
        },
        5,
    ));

    // CRITICAL INVARIANTS (never cut)
    let mut inv = String::new();
    for i in &atlas.invariants {
        inv.push_str(&format!("- [{}] {}\n", severity_str(i.severity), i.statement));
    }
    sections.push(Section::new("CRITICAL INVARIANTS", inv, 10));

    // FAILURE / RETRY (never cut)
    let mut failure = String::new();
    for c in &atlas.components {
        for fb in &c.failure_behavior {
            failure.push_str(&format!("{}: {}\n", c.name, fb));
        }
    }
    sections.push(Section::new("FAILURE / RETRY", failure, 9));

    // DEPLOYMENT
    sections.push(Section::new(
        "DEPLOYMENT",
        if atlas.deployment_units.is_empty() {
            "(none)".into()
        } else {
            atlas.deployment_units.join("\n")
        },
        7,
    ));

    // TRUST BOUNDARIES
    sections.push(Section::new(
        "TRUST BOUNDARIES",
        if atlas.trust_boundaries.is_empty() {
            "(none)".into()
        } else {
            atlas.trust_boundaries.join("\n")
        },
        7,
    ));

    // ASYNC BOUNDARIES
    sections.push(Section::new(
        "ASYNC BOUNDARIES",
        if atlas.async_boundaries.is_empty() {
            "(none)".into()
        } else {
            atlas.async_boundaries.join("\n")
        },
        7,
    ));

    // EXTERNAL SYSTEMS
    sections.push(Section::new(
        "EXTERNAL SYSTEMS",
        if atlas.external_systems.is_empty() {
            "(none)".into()
        } else {
            atlas.external_systems.join("\n")
        },
        7,
    ));

    // IMPLEMENTATION MAP
    let mut impl_map = String::new();
    for (name, paths) in &atlas.implementation_map {
        if !paths.is_empty() {
            impl_map.push_str(&format!("{}: {}\n", name, paths.join(", ")));
        }
    }
    sections.push(Section::new("IMPLEMENTATION MAP", impl_map, 6));

    // EVIDENCE STATUS
    let ev: Vec<String> = atlas
        .evidence_summary
        .iter()
        .map(|(k, v)| format!("{v} {k}"))
        .collect();
    sections.push(Section::new(
        "EVIDENCE STATUS",
        if ev.is_empty() { "(none)".into() } else { ev.join(", ") },
        8,
    ));

    // RUNTIME (Wave 6): observed trace-path signatures + three-way drift
    // (declared vs static vs observed). Priority 8 — droppable before any
    // critical section, so a tight budget never hides invariants.
    let mut runtime = String::new();
    let sigs = ctx.store.trace_signatures().unwrap_or_default();
    if !sigs.is_empty() {
        runtime.push_str("OBSERVED PATH\n");
        // store order: (count DESC, signature) — deterministic top-10
        for (signature, count, latency_ms, errors, _last) in sigs.into_iter().take(10) {
            runtime.push_str(&format!(
                "{signature} ({count} reqs, avg {latency_ms:.1} ms, {errors} err)\n"
            ));
        }
    }
    const THREE_WAY_KINDS: [&str; 3] = [
        "undeclared_observed",
        "declared_unobserved",
        "static_unobserved",
    ];
    for (_, kind, _sev, msg, _) in ctx.store.drift_findings(true).unwrap_or_default() {
        if !THREE_WAY_KINDS.contains(&kind.as_str()) {
            continue;
        }
        let label = match kind.as_str() {
            "undeclared_observed" => "undeclared observed",
            "declared_unobserved" => "declared unobserved",
            "static_unobserved" => "static unobserved",
            _ => kind.as_str(),
        };
        runtime.push_str(&format!("DRIFT {label}: {msg}\n"));
    }
    if runtime.is_empty() {
        runtime.push_str("(none)\n");
    }
    sections.push(Section::new("RUNTIME", runtime, 8));

    // MODEL COVERAGE (Wave 9): the explicit uncertainty/coverage map — what
    // the model knows AND what it does not. Priority 7: droppable before
    // any critical section, so a tight budget never hides invariants.
    let mut coverage = String::new();
    for (k, v) in &atlas.coverage {
        coverage.push_str(&format!("{k}: {v}\n"));
    }
    sections.push(Section::new(
        "MODEL COVERAGE",
        if coverage.is_empty() {
            "(none)".into()
        } else {
            coverage
        },
        7,
    ));

    let warnings = atlas.warnings.clone();
    finish(&mut pack, sections, budget, warnings);
    pack.entity_ids = comp_ids(ctx);
    pack
}

/// Project one canonical FlowGraph into ordered step lines for the atlas.
/// Walks from the entrypoints along POLICY-ALLOWED edges (the trust view's
/// provenance policy applies to derived edges too), marking edge kinds:
/// branch / retry / error / async / publish / consume / join. Never
/// flattens alternate paths into false sequential causality.
fn project_flow_graph(
    view: &TrustedGraphView,
    g: &scc_core::FlowGraph,
    async_boundaries: &mut BTreeSet<String>,
) -> Vec<String> {
    let policy = view.policy();
    let edge_ok = |e: &scc_core::FlowEdge| -> bool {
        match e.provenance {
            None => true,
            Some(p) => policy.allows(p, e.confidence),
        }
    };
    let mut lines: Vec<String> = Vec::new();
    let mut visited: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    let mut queue: std::collections::VecDeque<u32> =
        g.entrypoints.iter().copied().collect();
    while let Some(n) = queue.pop_front() {
        if !visited.insert(n) {
            continue;
        }
        let Some(node) = g.nodes.get(n as usize) else { continue };
        lines.push(format!("{}: {}", entity_name(view, &node.actor), node.operation));
        let mut outs: Vec<&scc_core::FlowEdge> = g
            .edges
            .iter()
            .filter(|e| e.from == n && edge_ok(e))
            .collect();
        outs.sort_by(|a, b| edge_rank(a.kind).cmp(&edge_rank(b.kind)).then(a.to.cmp(&b.to)));
        for e in outs {
            let Some(target) = g.nodes.get(e.to as usize) else { continue };
            let mut line = format!(
                "  -> {}: {}",
                entity_name(view, &target.actor),
                target.operation
            );
            match e.kind {
                scc_core::FlowEdgeKind::Branch => line.push_str(" (branch)"),
                scc_core::FlowEdgeKind::Retry => line.push_str(" [retry]"),
                scc_core::FlowEdgeKind::Error => line.push_str(" [error]"),
                scc_core::FlowEdgeKind::Async => line.push_str(" [async]"),
                scc_core::FlowEdgeKind::Publish => line.push_str(" [publish]"),
                scc_core::FlowEdgeKind::Consume => line.push_str(" [consume]"),
                scc_core::FlowEdgeKind::Join => line.push_str(" (join)"),
                scc_core::FlowEdgeKind::Fallback => line.push_str(" [fallback]"),
                scc_core::FlowEdgeKind::Timeout => line.push_str(" [timeout]"),
                scc_core::FlowEdgeKind::Compensation => line.push_str(" [compensate]"),
                _ => {}
            }
            if let Some(c) = &e.condition {
                line.push_str(&format!(" ({c})"));
            }
            lines.push(line);
            if e.kind == scc_core::FlowEdgeKind::Async {
                async_boundaries.insert(format!(
                    "{} --async--> {}",
                    entity_name(view, &node.actor),
                    entity_name(view, &target.actor)
                ));
            }
            queue.push_back(e.to);
        }
    }
    lines
}

fn edge_rank(k: scc_core::FlowEdgeKind) -> u8 {
    match k {
        scc_core::FlowEdgeKind::Next => 0,
        scc_core::FlowEdgeKind::Async => 1,
        scc_core::FlowEdgeKind::Branch => 2,
        scc_core::FlowEdgeKind::Join => 3,
        scc_core::FlowEdgeKind::Retry => 4,
        scc_core::FlowEdgeKind::Fallback => 5,
        scc_core::FlowEdgeKind::Error => 6,
        scc_core::FlowEdgeKind::Publish => 7,
        scc_core::FlowEdgeKind::Consume => 8,
        scc_core::FlowEdgeKind::Return => 9,
        scc_core::FlowEdgeKind::Timeout => 10,
        scc_core::FlowEdgeKind::Compensation => 11,
    }
}

fn comp_ids(ctx: &ContextCompiler) -> Vec<String> {
    ctx.view.components().into_iter().map(|c| c.id.clone()).collect()
}

fn flow_kind_str(k: FlowKind) -> &'static str {
    match k {
        FlowKind::Architecture => "architecture",
        FlowKind::Workflow => "workflow",
        FlowKind::Sequence => "sequence",
        FlowKind::Dataflow => "dataflow",
        FlowKind::Lifecycle => "lifecycle",
    }
}

fn severity_str(s: scc_core::Severity) -> &'static str {
    match s {
        scc_core::Severity::Info => "INFO",
        scc_core::Severity::Low => "LOW",
        scc_core::Severity::Medium => "MEDIUM",
        scc_core::Severity::High => "HIGH",
        scc_core::Severity::Critical => "CRITICAL",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scc_core::{
        entity_id, kinds, predicates, relationship_id, symbol_id, Entity, Provenance,
        Relationship,
    };
    use scc_store::Store;

    fn test_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let repo = "repo";

        // files: 2 parsed python + 1 unparsed json
        store.upsert_file("app.py", "h1", "python", "source", 10).unwrap();
        store.upsert_file("lib.py", "h2", "python", "source", 10).unwrap();
        store.upsert_file("config.json", "h3", "json", "config", 10).unwrap();

        // symbols
        let mk = |n: &str| symbol_id(repo, "app.py", n);
        for n in ["handler", "worker", "reader"] {
            let mut e = Entity::new(mk(n), kinds::SYMBOL, n);
            e.attr("file", serde_json::json!("app.py"));
            store.insert_entity(&e, &["app.py".to_string()]).unwrap();
        }
        // cli flags on worker
        let mut w = store.get_entity(&mk("worker")).unwrap().unwrap();
        w.attributes
            .insert("cli_flags".into(), serde_json::json!(["--queue", "--verbose"]));
        store.insert_entity(&w, &["app.py".to_string()]).unwrap();

        // route with handler
        let route_id = entity_id(repo, kinds::ROUTE, "GET /api/x");
        let mut re = Entity::new(route_id.clone(), kinds::ROUTE, "GET /api/x");
        re.attr("method", serde_json::json!("GET"));
        re.attr("path", serde_json::json!("/api/x"));
        re.attr("handler", serde_json::json!(mk("handler")));
        store.insert_entity(&re, &["app.py".to_string()]).unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    relationship_id(1),
                    mk("handler"),
                    predicates::HANDLES,
                    route_id,
                    Provenance::Extracted,
                ),
                "app.py",
            )
            .unwrap();

        // export: handler EXPORTS export(handler, function)
        let exp_id = entity_id(repo, kinds::EXPORT, "handler");
        let mut ex = Entity::new(exp_id.clone(), kinds::EXPORT, "handler");
        ex.attr("kind", serde_json::json!("function"));
        store.insert_entity(&ex, &["app.py".to_string()]).unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    relationship_id(2),
                    mk("handler"),
                    predicates::EXPORTS,
                    exp_id,
                    Provenance::Extracted,
                ),
                "app.py",
            )
            .unwrap();

        // configuration DEBUG configured-by reader
        let cfg_id = entity_id(repo, kinds::CONFIGURATION, "DEBUG");
        store
            .insert_entity(
                &Entity::new(cfg_id.clone(), kinds::CONFIGURATION, "DEBUG"),
                &["app.py".to_string()],
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    relationship_id(3),
                    cfg_id,
                    predicates::CONFIGURED_BY,
                    mk("reader"),
                    Provenance::Extracted,
                ),
                "app.py",
            )
            .unwrap();

        // calls: one EXTRACTED to a local symbol, one EXTRACTED to a missing
        // external target, one RESOLVED (LSP proof)
        store
            .insert_relationship(
                &Relationship::new(
                    relationship_id(4),
                    mk("handler"),
                    predicates::CALLS,
                    mk("worker"),
                    Provenance::Extracted,
                ),
                "app.py",
            )
            .unwrap();
        let ext_id = entity_id(repo, kinds::EXTERNAL_API, "os");
        store
            .insert_relationship(
                &Relationship::new(
                    relationship_id(5),
                    mk("handler"),
                    predicates::CALLS,
                    ext_id,
                    Provenance::Extracted,
                ),
                "app.py",
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    relationship_id(6),
                    mk("worker"),
                    predicates::CALLS,
                    mk("reader"),
                    Provenance::Resolved,
                ),
                "app.py",
            )
            .unwrap();

        // topic jobs + worker SUBSCRIBES
        let topic_id = entity_id(repo, kinds::TOPIC, "jobs");
        store
            .insert_entity(
                &Entity::new(topic_id.clone(), kinds::TOPIC, "jobs"),
                &["app.py".to_string()],
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    relationship_id(7),
                    mk("worker"),
                    predicates::SUBSCRIBES,
                    topic_id,
                    Provenance::Extracted,
                ),
                "app.py",
            )
            .unwrap();

        let _graph = scc_graph::RealityGraph::load(&store).unwrap();
        (dir, store)
    }

    #[test]
    fn kinds_stringify() {
        assert_eq!(flow_kind_str(FlowKind::Sequence), "sequence");
        assert_eq!(severity_str(scc_core::Severity::Critical), "CRITICAL");
    }

    #[test]
    // # trace:exempt — unit test (tests are not trace-worthy behavior)
    fn contracts_and_coverage_from_fact_layer() {
        let (_dir, store) = test_store();
        let graph = scc_graph::RealityGraph::load(&store).unwrap();
        let ctx = ContextCompiler::new(&store, &graph, crate::ContextSettings::default(), Vec::new());
        let atlas = build_atlas(&ctx);

        // contract subclasses: http (route), cli (flags), config (DEBUG),
        // event (topic jobs), public-api (exported function signature) —
        // no annotations in this fixture
        let kinds_found: BTreeSet<String> = atlas.contracts.iter().map(|c| c.kind.clone()).collect();
        assert_eq!(
            kinds_found,
            BTreeSet::from([
                "http".to_string(),
                "cli".to_string(),
                "config".to_string(),
                "event".to_string(),
                "public-api".to_string()
            ]),
            "contract kinds: {:?}",
            atlas.contracts
        );
        // every contract carries the typed subclass, and the machine-model
        // kind agrees with the subclass render prefix
        for c in &atlas.contracts {
            assert_eq!(c.kind, c.subclass.as_str(), "kind agrees with subclass: {c:?}");
        }
        let http = atlas.contracts.iter().find(|c| c.kind == "http").unwrap();
        assert_eq!(http.operations, vec!["GET /api/x"]);
        assert_eq!(http.subclass, scc_core::ContractSubclass::Http);
        assert!(
            http.consumers.iter().any(|c| c == "handler"),
            "handler consumes the route: {:?}",
            http.consumers
        );
        let cli = atlas.contracts.iter().find(|c| c.kind == "cli").unwrap();
        assert_eq!(cli.operations, vec!["--queue", "--verbose"]);
        assert_eq!(cli.subclass, scc_core::ContractSubclass::Cli);
        let cfg = atlas.contracts.iter().find(|c| c.kind == "config").unwrap();
        assert_eq!(cfg.operations, vec!["DEBUG"]);
        assert_eq!(cfg.subclass, scc_core::ContractSubclass::Configuration);
        assert!(
            cfg.consumers.iter().any(|c| c == "reader"),
            "reader consumes DEBUG: {:?}",
            cfg.consumers
        );
        let api = atlas.contracts.iter().find(|c| c.kind == "public-api").unwrap();
        assert_eq!(api.operations, vec!["handler"]);
        assert_eq!(api.subclass, scc_core::ContractSubclass::PublicApi);

        // rendered CONTRACTS lines are `{subclass}: {operation}` per group
        let lines: Vec<String> = atlas
            .contracts
            .iter()
            .flat_map(|c| {
                c.operations
                    .iter()
                    .map(|op| format!("{}: {}", c.subclass.as_str(), op))
            })
            .collect();
        for want in [
            "http: GET /api/x",
            "cli: --queue",
            "config: DEBUG",
            "event: jobs",
            "public-api: handler",
        ] {
            assert!(lines.contains(&want.to_string()), "missing {want}: {lines:?}");
        }

        // coverage map: honest, deterministic numbers from the store
        assert_eq!(
            atlas.coverage.get("parsed_source_files").unwrap(),
            "66% (2/3)"
        );
        assert_eq!(
            atlas.coverage.get("unparsed_files").unwrap(),
            "1 (config/docs/infra — scanned but not source-parsed)"
        );
        assert!(
            atlas.coverage.get("exported_api").unwrap().starts_with("1 export entity"),
            "{:?}",
            atlas.coverage.get("exported_api")
        );
        // 3 calls: 2 with existing targets (worker symbol, reader symbol),
        // 1 external target with no entity → 66%, 1 LSP-RESOLVED
        assert_eq!(
            atlas.coverage.get("call_targets_resolved").unwrap(),
            "66% (2/3, 1 LSP-RESOLVED) — exploration still justified in unresolved regions"
        );
        assert_eq!(
            atlas.coverage.get("dynamic_receivers_unresolved").unwrap(),
            "1 (calls whose target is not a local symbol; unknown-receiver calls are not persisted)"
        );
        // invocation surfaces: public_api (handler) + queue (worker) +
        // http (handler via route) + cli (worker cli_flags)
        assert_eq!(
            atlas.coverage.get("invocation_surfaces").unwrap(),
            "4 (cli 1, http 1, public_api 1, queue 1)",
            "{:?}",
            atlas.coverage.get("invocation_surfaces")
        );
        assert_eq!(atlas.coverage.get("stale_evidence").unwrap(), "0 (model FRESH)");
        assert!(atlas.coverage.contains_key("model_epoch_generations"));
        assert!(atlas.coverage.contains_key("framework_registrations_unknown"));

        // atlas entrypoints carry the surface kinds
        let ep_kinds: BTreeSet<&str> = atlas.entrypoints.iter().map(|e| e.kind.as_str()).collect();
        assert!(ep_kinds.contains("public_api"), "{ep_kinds:?}");
        assert!(ep_kinds.contains("queue"), "{ep_kinds:?}");
        assert!(ep_kinds.contains("http"), "http surface: {ep_kinds:?}");
        assert!(ep_kinds.contains("cli"), "cli surface: {ep_kinds:?}");
    }

// trace:exempt reason=internal-detail

    /// Wave 11: schema contracts (SCHEMA entities + DEFINES/COMPOSES/
    /// VALIDATES edges) render under CONTRACTS with the `schema:` prefix,
    /// and reactive state (REACTIVE entities + OWNS edges) renders under
    /// STATE & DATA AUTHORITY's REACTIVE STATE subsection, attributed to
    /// the owning symbol's component.
    #[test]

// trace:exempt reason=internal-detail
    fn schema_and_reactive_render_in_atlas() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let repo = store.repo_id.clone();

        // component api (api/app.py) with symbol UserService — components
        // live in the `components` table (RealityGraph::load reads
        // store.components())
        let comp_id = entity_id(&repo, kinds::COMPONENT, "api");
        store
            .replace_components(&[scc_core::Entity::new(comp_id.clone(), kinds::COMPONENT, "api")])
            .unwrap();
        let fid = entity_id(&repo, kinds::FILE, "api/app.py");
        store
            .insert_entity(
                &Entity::new(fid.clone(), kinds::FILE, "api/app.py"),
                &["api/app.py".to_string()],
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:c:api",
                    comp_id,
                    predicates::CONTAINS,
                    fid.clone(),
                    Provenance::Extracted,
                ),
                "api/app.py",
            )
            .unwrap();
        let svc = symbol_id(&repo, "api/app.py", "UserService");
        store
            .insert_entity(
                &Entity::new(svc.clone(), kinds::SYMBOL, "UserService"),
                &["api/app.py".to_string()],
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:f:svc",
                    fid,
                    predicates::CONTAINS,
                    svc.clone(),
                    Provenance::Extracted,
                ),
                "api/app.py",
            )
            .unwrap();

        // schema User (UserService DEFINES it, COMPOSES Base, VALIDATES
        // CreateUser) with one occurrence owned by the User symbol
        let base = entity_id(&repo, kinds::SCHEMA, "Base");
        store
            .insert_entity(
                &Entity::new(base.clone(), kinds::SCHEMA, "Base"),
                &["api/app.py".to_string()],
            )
            .unwrap();
        let user = entity_id(&repo, kinds::SCHEMA, "User");
        store
            .insert_entity(
                &Entity::new(user.clone(), kinds::SCHEMA, "User"),
                &["api/app.py".to_string()],
            )
            .unwrap();
        let user_occ = scc_core::occurrence_id(&repo, "User", "api/app.py", "User", 1);
        store
            .insert_entity(
                Entity::new(
                    user_occ.clone(),
                    scc_core::kinds::OCCURRENCE,
                    "User@api/app.py@User@1",
                )
                .attr("concept", serde_json::json!(user))
                .attr("path", serde_json::json!("api/app.py"))
                .attr("owner", serde_json::json!("User"))
                .attr("line", serde_json::json!(1)),
                &["api/app.py".to_string()],
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:occ:user",
                    user_occ,
                    scc_core::predicates::OCCURS,
                    user.clone(),
                    Provenance::Extracted,
                ),
                "api/app.py",
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:defines",
                    svc.clone(),
                    predicates::DEFINES,
                    user.clone(),
                    Provenance::Extracted,
                ),
                "api/app.py",
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:composes",
                    user,
                    predicates::COMPOSES,
                    base,
                    Provenance::Extracted,
                ),
                "api/app.py",
            )
            .unwrap();
        let target = entity_id(&repo, kinds::SYMBOL, "CreateUser");
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:validates",
                    svc.clone(),
                    predicates::VALIDATES,
                    target,
                    Provenance::Extracted,
                ),
                "api/app.py",
            )
            .unwrap();

        // reactive state count: one concept, one occurrence owned by
        // UserService (Wave 13 — identity is per concept/path/owner/line)
        let count = entity_id(&repo, kinds::REACTIVE, "count");
        store
            .insert_entity(
                &Entity::new(count.clone(), kinds::REACTIVE, "count"),
                &["api/app.py".to_string()],
            )
            .unwrap();
        let count_occ = scc_core::occurrence_id(&repo, "count", "api/app.py", "UserService", 1);
        store
            .insert_entity(
                Entity::new(
                    count_occ.clone(),
                    scc_core::kinds::OCCURRENCE,
                    "count@api/app.py@UserService@1",
                )
                .attr("concept", serde_json::json!(count))
                .attr("path", serde_json::json!("api/app.py"))
                .attr("owner", serde_json::json!("UserService"))
                .attr("line", serde_json::json!(1))
                .attr("access", serde_json::json!("state")),
                &["api/app.py".to_string()],
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:occ:count",
                    count_occ.clone(),
                    scc_core::predicates::OCCURS,
                    count.clone(),
                    Provenance::Extracted,
                ),
                "api/app.py",
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:owns:count",
                    svc,
                    predicates::OWNS,
                    count_occ,
                    Provenance::Extracted,
                ),
                "api/app.py",
            )
            .unwrap();

        let graph = scc_graph::RealityGraph::load(&store).unwrap();
        let ctx = ContextCompiler::new(&store, &graph, crate::ContextSettings::default(), Vec::new());
        let atlas = build_atlas(&ctx);

        // schema contract: name + composition + validation operations
        // (find the User schema — the Base schema is a plain name-only
        // contract)
        let schema = atlas
            .contracts
            .iter()
            .find(|c| c.subclass == scc_core::ContractSubclass::Schema && c.operations.first() == Some(&"User".to_string()))
            .expect("schema contract for User");
        assert_eq!(schema.kind, "schema");
        assert_eq!(
            schema.operations,
            vec![
                "User".to_string(),
                "User extends Base".to_string(),
                "User validates".to_string()
            ],
            "{schema:?}"
        );
        // Wave 13 (e): the producer is the occurrence owner symbol — never
        // the concept/expr itself
        assert_eq!(schema.producer, "User", "producer = owner symbol: {schema:?}");

        // reactive state attributed to the owning symbol's component
        assert!(
            atlas
                .state_authority
                .get(scc_graph::state::S_REACTIVE)
                .map(|lines| lines.iter().any(|l| l == "api owns reactive: count [state] (EXTRACTED)"))
                .unwrap_or(false),
            "{:?}",
            atlas.state_authority
        );

        // rendered atlas lines
        let pack = render_atlas(&ctx, &atlas, usize::MAX);
        for want in [
            "schema: User",
            "schema: User extends Base",
            "schema: User validates",
            "REACTIVE STATE",
            "api owns reactive: count [state] (EXTRACTED)",
        ] {
            assert!(pack.content.contains(want), "missing {want:?} in:\n{}", pack.content);
        }
    }

    /// A repo with component-attributed symbols exercising the Wave 10
    /// fact-layer sections: exported classes/methods, module exports,
    /// annotations, registrations, callbacks, and state facts.
    fn fact_layer_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let repo = store.repo_id.clone();

        // components api (api/app.py) and web (web/app.py)
        let mk_comp = |store: &Store, name: &str, file: &str| -> String {
            let id = entity_id(&repo, kinds::COMPONENT, name);
            store
                .insert_entity(
                    &scc_core::Entity::new(id.clone(), kinds::COMPONENT, name),
                    &[file.to_string()],
                )
                .unwrap();
            let fid = entity_id(&repo, kinds::FILE, file);
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:c:{name}"),
                        id.clone(),
                        predicates::CONTAINS,
                        fid,
                        Provenance::Extracted,
                    ),
                    file,
                )
                .unwrap();
            id
        };
        mk_comp(&store, "api", "api/app.py");
        mk_comp(&store, "web", "web/app.py");
        // components live in the `components` table (RealityGraph::load
        // reads store.components()) — replace with the full list, carrying
        // the component compiler's `implementation` fact (paths + member
        // symbols).
        let mut api_comp = scc_core::Entity::new(
            entity_id(&repo, kinds::COMPONENT, "api"),
            kinds::COMPONENT,
            "api",
        );
        api_comp.attr(
            "implementation",
            serde_json::json!({
                "paths": ["api"],
                "symbols": ["App", "App.get", "include_router", "_secret"],
            }),
        );
        let mut web_comp = scc_core::Entity::new(
            entity_id(&repo, kinds::COMPONENT, "web"),
            kinds::COMPONENT,
            "web",
        );
        web_comp.attr(
            "implementation",
            serde_json::json!({
                "paths": ["web"],
                "symbols": ["handle_page", "on_message"],
            }),
        );
        store.replace_components(&[api_comp, web_comp]).unwrap();
        store
            .insert_entity(
                &Entity::new(entity_id(&repo, kinds::FILE, "api/app.py"), kinds::FILE, "api/app.py"),
                &["api/app.py".into()],
            )
            .unwrap();
        store
            .insert_entity(
                &Entity::new(entity_id(&repo, kinds::FILE, "web/app.py"), kinds::FILE, "web/app.py"),
                &["web/app.py".into()],
            )
            .unwrap();

        let mk_sym = |path: &str, name: &str, attrs: serde_json::Value| -> String {
            let id = symbol_id(&repo, path, name);
            let mut e = Entity::new(id.clone(), kinds::SYMBOL, name);
            if let Some(obj) = attrs.as_object() {
                for (k, v) in obj {
                    e.attr(k, v.clone());
                }
            }
            store.insert_entity(&e, &[path.to_string()]).unwrap();
            let fid = entity_id(&repo, kinds::FILE, path);
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:f:{path}:{name}"),
                        fid,
                        predicates::CONTAINS,
                        id.clone(),
                        Provenance::Extracted,
                    ),
                    path,
                )
                .unwrap();
            id
        };

        // exported class `App` with public method `App.get` (framework class)
        let app_id = mk_sym(
            "api/app.py",
            "App",
            serde_json::json!({"kind": "class", "exported": true}),
        );
        let app_get = mk_sym(
            "api/app.py",
            "App.get",
            serde_json::json!({"kind": "method", "parent": "App", "exported": false}),
        );
        // exported module-level function + underscore-private one
        mk_sym(
            "api/app.py",
            "include_router",
            serde_json::json!({"kind": "function", "exported": true}),
        );
        mk_sym(
            "api/app.py",
            "_secret",
            serde_json::json!({"kind": "function", "exported": true}),
        );
        // web component symbol
        let web_handle = mk_sym(
            "web/app.py",
            "handle_page",
            serde_json::json!({"kind": "function", "exported": true}),
        );

        // EXPORT entity for include_router (EXPORTS edge)
        let exp_id = entity_id(&repo, kinds::EXPORT, "include_router");
        store
            .insert_entity(
                &Entity::new(exp_id.clone(), kinds::EXPORT, "include_router"),
                &["api/app.py".into()],
            )
            .unwrap();
        let include_id = symbol_id(&repo, "api/app.py", "include_router");
        store
            .insert_relationship(
                &Relationship::new(
                    relationship_id(100),
                    include_id,
                    predicates::EXPORTS,
                    exp_id,
                    Provenance::Extracted,
                ),
                "api/app.py",
            )
            .unwrap();

        // annotation: @Get on App.get
        let ann_id = entity_id(&repo, kinds::ANNOTATION, "Get");
        store
            .insert_entity(
                &Entity::new(ann_id.clone(), kinds::ANNOTATION, "Get"),
                &["api/app.py".into()],
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    relationship_id(101),
                    ann_id,
                    predicates::ANNOTATES,
                    app_get.clone(),
                    Provenance::Extracted,
                ),
                "api/app.py",
            )
            .unwrap();

        // registration: App registers middleware
        let mw_id = entity_id(&repo, kinds::MIDDLEWARE, "RequestLogger");
        store
            .insert_entity(
                &Entity::new(mw_id.clone(), kinds::MIDDLEWARE, "RequestLogger"),
                &["api/app.py".into()],
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    relationship_id(102),
                    app_id.clone(),
                    predicates::REGISTERS,
                    mw_id,
                    Provenance::Extracted,
                ),
                "api/app.py",
            )
            .unwrap();

        // callback: web_handle HANDLES_CALLBACK on_message (a SYMBOL target)
        let cb_sym = mk_sym(
            "web/app.py",
            "on_message",
            serde_json::json!({"kind": "function", "exported": false}),
        );
        store
            .insert_relationship(
                &Relationship::new(
                    relationship_id(103),
                    web_handle.clone(),
                    predicates::HANDLES_CALLBACK,
                    cb_sym,
                    Provenance::Extracted,
                ),
                "web/app.py",
            )
            .unwrap();

        // state facts: mutable field CONTAINS-ed by App + config
        let field_id = entity_id(&repo, kinds::FIELD, "App.cache");
        store
            .insert_entity(
                Entity::new(field_id.clone(), kinds::FIELD, "App.cache")
                    .attr("mutable", serde_json::json!(true))
                    .attr("owner", serde_json::json!("App")),
                &["api/app.py".into()],
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    relationship_id(104),
                    app_id.clone(),
                    predicates::CONTAINS,
                    field_id,
                    Provenance::Extracted,
                ),
                "api/app.py",
            )
            .unwrap();
        let cfg_id = entity_id(&repo, kinds::CONFIGURATION, "DEBUG");
        store
            .insert_entity(
                &Entity::new(cfg_id.clone(), kinds::CONFIGURATION, "DEBUG"),
                &["api/app.py".into()],
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    relationship_id(105),
                    cfg_id,
                    predicates::CONFIGURED_BY,
                    app_id,
                    Provenance::Extracted,
                ),
                "api/app.py",
            )
            .unwrap();

        let _graph = scc_graph::RealityGraph::load(&store).unwrap();
        (dir, store)
    }

    #[test]
    fn fact_layer_sections_grouped_by_component() {
        let (_dir, store) = fact_layer_store();
        let graph = scc_graph::RealityGraph::load(&store).unwrap();
        let ctx = ContextCompiler::new(&store, &graph, crate::ContextSettings::default(), Vec::new());
        let atlas = build_atlas(&ctx);

        // PUBLIC API grouped by component: api has App (exported class) +
        // include_router (export entity + module export); web has
        // handle_page. `_secret` is excluded (leading underscore).
        let api_exports = atlas.public_api.get("api").expect("api exports");
        assert!(api_exports.contains(&"App".to_string()), "{api_exports:?}");
        assert!(api_exports.contains(&"include_router".to_string()), "{api_exports:?}");
        assert!(!api_exports.iter().any(|e| e == "_secret"), "{api_exports:?}");
        let web_exports = atlas.public_api.get("web").expect("web exports");
        assert!(web_exports.contains(&"handle_page".to_string()), "{web_exports:?}");

        // FRAMEWORK SEMANTICS: annotation, registration, callback per comp
        let api_facts = atlas.framework_semantics.get("api").expect("api facts");
        assert!(
            api_facts.iter().any(|f| f.contains("annotates App.get (Get)")),
            "{api_facts:?}"
        );
        assert!(
            api_facts.iter().any(|f| f.contains("registers RequestLogger")),
            "{api_facts:?}"
        );
        let web_facts = atlas.framework_semantics.get("web").expect("web facts");
        assert!(
            web_facts.iter().any(|f| f.contains("handles callback on_message")),
            "{web_facts:?}"
        );

        // component implementation carries member symbols; paths stay pure
        let api = atlas.components.iter().find(|c| c.name == "api").unwrap();
        assert!(api.symbols.contains(&"App".to_string()), "{:?}", api.symbols);
        assert!(api.symbols.contains(&"App.get".to_string()), "{:?}", api.symbols);
        assert!(api.implementation.contains(&"App".to_string()), "{:?}", api.implementation);
        assert_eq!(api.implementation_paths, vec!["api".to_string()]);
        // paths-only in the implementation map (compact render)
        assert_eq!(
            atlas.implementation_map.get("api").unwrap(),
            &vec!["api".to_string()]
        );

        // STATE & DATA AUTHORITY structured bridge: mutable field + config
        // targets surface as component owns claims
        let api_owns: Vec<&str> = api.owns.iter().map(|o| o.target.as_str()).collect();
        assert!(
            api_owns.contains(&"App.cache"),
            "mutable field owns claim: {api_owns:?}"
        );
        assert!(api_owns.contains(&"DEBUG"), "config owns claim: {api_owns:?}");

        // LANDMARKS bounded: exports (App, include_router, handle_page)
        assert!(atlas.landmarks.len() <= 40, "landmarks bounded: {:?}", atlas.landmarks.len());
        assert!(atlas.landmarks.iter().any(|l| l.contains("App")), "{:?}", atlas.landmarks);

        // rendered atlas carries the new section headers
        let pack = render_atlas(&ctx, &atlas, usize::MAX);
        assert!(pack.content.contains("# PUBLIC API"), "{}", pack.content);
        assert!(pack.content.contains("# FRAMEWORK SEMANTICS"), "{}", pack.content);
        assert!(pack.content.contains("# LANDMARKS"), "{}", pack.content);
        assert!(pack.content.contains("api: exports App, include_router"), "{}", pack.content);
    }

    #[test]
    fn pipeline_only_for_compiler_language_tool() {
        let (_dir, store) = fact_layer_store();
        let graph = scc_graph::RealityGraph::load(&store).unwrap();
        let ctx = ContextCompiler::new(&store, &graph, crate::ContextSettings::default(), Vec::new());
        let atlas = build_atlas(&ctx);
        // not a compiler repo → no pipeline lines
        assert!(atlas.pipeline.is_empty(), "{:?}", atlas.pipeline);

        // phase-named symbols group by stage when the archetype fires
        let parse_id = symbol_id(&store.repo_id, "api/app.py", "parse");
        let mut e = store
            .get_entity(&parse_id)
            .ok()
            .flatten()
            .unwrap_or_else(|| {
                let ent = Entity::new(parse_id.clone(), kinds::SYMBOL, "parse");
                store.insert_entity(&ent, &["api/app.py".into()]).unwrap();
                ent
            });
        e.attr("exported", serde_json::json!(true));
        store.insert_entity(&e, &["api/app.py".into()]).unwrap();
        let graph = scc_graph::RealityGraph::load(&store).unwrap();
        let ctx = ContextCompiler::new(&store, &graph, crate::ContextSettings::default(), Vec::new());
        let pipeline = build_pipeline(&ctx.view, Some(scc_core::Archetype::CompilerLanguageTool));
        assert!(
            pipeline.iter().any(|l| l.trim() == "parse"),
            "parse symbol in pipeline: {pipeline:?}"
        );
        assert!(
            pipeline.iter().any(|l| l == "[parse]"),
            "stage header: {pipeline:?}"
        );
    }
}
