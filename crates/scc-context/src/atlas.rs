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
    AtlasOwnershipClaim, FlowKind, SystemAtlas,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Structured atlas compilation — pure data, no rendering.
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

        let implementation: Vec<String> = c
            .attributes
            .get("implementation")
            .and_then(|i| i.get("paths"))
            .and_then(|p| p.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

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
                producer,
                consumers: consumers.into_iter().collect(),
                operations: vec![c.name.clone()],
                evidence: c.evidence.clone(),
            },
        );
    }

    // annotation: ANNOTATION entities (producer = the annotation; consumers
    // = the annotated symbols)
    for a in view.entities_of_kind(scc_core::kinds::ANNOTATION) {
        let mut consumers: BTreeSet<String> = BTreeSet::new();
        for rel in view.out_pred(&a.id, scc_core::predicates::ANNOTATES) {
            consumers.insert(entity_name(view, &rel.object));
        }
        push_contract(
            &mut contracts,
            &mut contract_seen,
            scc_core::Contract {
                id: a.id.clone(),
                kind: "annotation".into(),
                producer: a.id.clone(),
                consumers: consumers.into_iter().collect(),
                operations: vec![a.name.clone()],
                evidence: a.evidence.clone(),
            },
        );
    }
    // deterministic order for the machine model
    contracts.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
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
        implementation_map.insert(c.name.clone(), c.implementation.clone());
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
    }
}

/// Push one contract, merging consumers/evidence when the same
/// (kind, operations) surface was already recorded (e.g. the same CLI flag
/// owned by two symbols). Deterministic: consumers/evidence stay sorted.
fn push_contract(
    contracts: &mut Vec<scc_core::Contract>,
    seen: &mut BTreeMap<(String, String), usize>,
    c: scc_core::Contract,
) {
    let key = (c.kind.clone(), c.operations.join("\u{1}"));
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
        for e in &atlas.entrypoints {
            if e.trigger == e.name {
                purpose.push_str(&format!("  {} [{}]\n", e.name, e.kind));
            } else {
                purpose.push_str(&format!("  {} [{}] — {}\n", e.name, e.kind, e.trigger));
            }
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
        if !c.implementation.is_empty() {
            out.push_str(&format!("\n{}Implementation: {}", indent, c.implementation.join(", ")));
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
    // section itself — skip it here to avoid duplicating the system
    let mut flows = String::new();
    for f in atlas
        .flows
        .iter()
        .filter(|f| f.kind != FlowKind::Architecture)
    {
        flows.push_str(&format!(
            "\n{} [{}]",
            f.name,
            flow_kind_str(f.kind)
        ));
        if let Some(t) = &f.trigger {
            flows.push_str(&format!("\nTrigger: {t}"));
        }
        for s in &f.steps {
            flows.push_str(&format!("\n{s}"));
        }
        flows.push('\n');
    }
    sections.push(Section::new("FLOWS", flows, 9));

    // STATE & DATA AUTHORITY (never cut): five subsections — DATA
    // OWNERSHIP (persistent: the write-derived + declared owns claims and
    // the DATA STORES list), RUNTIME STATE, CONFIGURATION, CACHES,
    // DERIVED / REGISTRIES. Falls back to the legacy DATA OWNERSHIP title
    // when the state compiler found no state at all.
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

    // CONTRACTS (never cut) — rendered as `{kind}: {operation}` lines from
    // the first-class Contract model (Wave 9), preserving the contract
    // strings (route `GET /api/x`, flag `--paging`, event `user.created`,
    // config key `DEBUG`) so pre-Wave-9 consumers keep matching.
    sections.push(Section::new(
        "CONTRACTS",
        if atlas.contracts.is_empty() {
            "(none)".into()
        } else {
            let mut lines: Vec<String> = Vec::new();
            for c in &atlas.contracts {
                for op in &c.operations {
                    lines.push(format!("{}: {}", c.kind, op));
                }
            }
            lines.sort();
            lines.join("\n")
        },
        9,
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
    fn contracts_and_coverage_from_fact_layer() {
        let (_dir, store) = test_store();
        let graph = scc_graph::RealityGraph::load(&store).unwrap();
        let ctx = ContextCompiler::new(&store, &graph, crate::ContextSettings::default(), Vec::new());
        let atlas = build_atlas(&ctx);

        // contract kinds: http (route), cli (flags), config (DEBUG),
        // event (topic jobs) — no annotations in this fixture
        let kinds_found: BTreeSet<String> = atlas.contracts.iter().map(|c| c.kind.clone()).collect();
        assert_eq!(
            kinds_found,
            BTreeSet::from([
                "http".to_string(),
                "cli".to_string(),
                "config".to_string(),
                "event".to_string()
            ]),
            "contract kinds: {:?}",
            atlas.contracts
        );
        let http = atlas.contracts.iter().find(|c| c.kind == "http").unwrap();
        assert_eq!(http.operations, vec!["GET /api/x"]);
        assert!(
            http.consumers.iter().any(|c| c == "handler"),
            "handler consumes the route: {:?}",
            http.consumers
        );
        let cli = atlas.contracts.iter().find(|c| c.kind == "cli").unwrap();
        assert_eq!(cli.operations, vec!["--queue", "--verbose"]);
        let cfg = atlas.contracts.iter().find(|c| c.kind == "config").unwrap();
        assert_eq!(cfg.operations, vec!["DEBUG"]);
        assert!(
            cfg.consumers.iter().any(|c| c == "reader"),
            "reader consumes DEBUG: {:?}",
            cfg.consumers
        );

        // rendered CONTRACTS lines are `{kind}: {operation}`
        let lines: Vec<String> = atlas
            .contracts
            .iter()
            .flat_map(|c| c.operations.iter().map(|op| format!("{}: {}", c.kind, op)))
            .collect();
        for want in ["http: GET /api/x", "cli: --queue", "config: DEBUG", "event: jobs"] {
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
        // invocation surfaces: public_api (handler) + queue (worker)
        assert!(
            atlas.coverage.get("invocation_surfaces").unwrap().starts_with("2 ("),
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
    }
}
