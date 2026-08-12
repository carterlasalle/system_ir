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
use crate::{ContextCompiler, ContextPack};
use scc_core::{
    AtlasComponent, AtlasEntrypoint, AtlasFlow, AtlasInvariant, AtlasOwnershipClaim, FlowKind,
    SystemAtlas,
};
use std::collections::{BTreeMap, BTreeSet};

/// Structured atlas compilation — pure data, no rendering.
pub fn build_atlas(ctx: &ContextCompiler) -> SystemAtlas {
    let view = &ctx.view;
    let store = ctx.store;
    let snapshot = store.latest_snapshot().ok().flatten();
    let repo = store.repository();

    let purpose = store.meta_get("purpose").ok().flatten().unwrap_or_default();

    // ---- components ----
    let mut components: Vec<AtlasComponent> = Vec::new();
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

        let mut owns: Vec<AtlasOwnershipClaim> = Vec::new();
        if let Some(oa) = c.attributes.get("owns").and_then(|v| v.as_array()) {
            for o in oa {
                if let (Some(t), Some(p)) = (
                    o.get("target").and_then(|v| v.as_str()),
                    o.get("provenance").and_then(|v| v.as_str()),
                ) {
                    owns.push(AtlasOwnershipClaim {
                        target: entity_name(view, t),
                        provenance: p.to_string(),
                    });
                }
            }
        }

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
        if e.attributes.contains_key("entrypoints") {
            entrypoints.push(AtlasEntrypoint {
                name: e.name.clone(),
                kind: "entrypoint".into(),
                trigger: format!("entrypoint:{}", e.name),
                symbol: e.id.clone(),
            });
        }
    }
    entrypoints.sort_by(|a, b| a.name.cmp(&b.name));

    // ---- contracts ----
    let mut contracts: BTreeSet<String> = BTreeSet::new();
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
        if !path.is_empty() {
            contracts.insert(format!("{method} {path}").trim().to_string());
        }
    }

    // ---- flows (condensed, markers preserved) ----
    let mut flows: Vec<AtlasFlow> = Vec::new();
    let mut async_boundaries: BTreeSet<String> = BTreeSet::new();
    for f in view.flows() {
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
                if let Some(next) = f.steps.iter().find(|x| x.order > s.order) {
                    async_boundaries.insert(format!(
                        "{} --async--> {}",
                        actor,
                        entity_name(view, &next.actor)
                    ));
                }
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
        flows.push(AtlasFlow {
            name: f.name.clone(),
            kind: f.kind,
            trigger: f.trigger.clone(),
            steps,
        });
    }

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
        contracts: contracts.into_iter().collect(),
        flows,
        invariants,
        deployment_units,
        external_systems,
        trust_boundaries,
        async_boundaries: async_boundaries.into_iter().collect(),
        implementation_map,
        evidence_summary,
        warnings,
    }
}

/// Render the atlas as compact structured text (agent-facing).
pub fn render_atlas(ctx: &ContextCompiler, atlas: &SystemAtlas, budget: usize) -> ContextPack {
    let mut pack = ContextPack::new("atlas", &atlas.revision);
    let mut sections: Vec<Section> = Vec::new();

    // SYSTEM PURPOSE (never cut)
    let mut purpose = String::new();
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

    // ARCHITECTURE (component blocks)
    let mut arch = String::new();
    for c in &atlas.components {
        arch.push_str(&format!("\n{}", c.name.to_uppercase()));
        if !c.purpose.is_empty() {
            arch.push_str(&format!("\nPurpose: {}", c.purpose));
        }
        if !c.implementation.is_empty() {
            arch.push_str(&format!("\nImplementation: {}", c.implementation.join(", ")));
        }
        if !c.consumes.is_empty() {
            arch.push_str(&format!("\nConsumes: {}", c.consumes.join(", ")));
        }
        if !c.produces.is_empty() {
            arch.push_str(&format!("\nProduces: {}", c.produces.join(", ")));
        }
        if !c.upstream.is_empty() {
            arch.push_str(&format!("\nUpstream: {}", c.upstream.join(", ")));
        }
        if !c.downstream.is_empty() {
            arch.push_str(&format!("\nDownstream: {}", c.downstream.join(", ")));
        }
        if !c.owns.is_empty() {
            let owned: Vec<String> = c
                .owns
                .iter()
                .map(|o| format!("{} ({})", o.target, o.provenance))
                .collect();
            arch.push_str(&format!("\nOwns: {}", owned.join(", ")));
        }
        arch.push('\n');
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

    // DATA OWNERSHIP (never cut)
    let mut ownership = String::new();
    for c in &atlas.components {
        for o in &c.owns {
            ownership.push_str(&format!(
                "{} owns {} ({})\n",
                c.name, o.target, o.provenance
            ));
        }
    }
    sections.push(Section::new("DATA OWNERSHIP", ownership, 10));

    // CONTRACTS (never cut)
    sections.push(Section::new(
        "CONTRACTS",
        if atlas.contracts.is_empty() {
            "(none)".into()
        } else {
            atlas.contracts.join("\n")
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

    let warnings = atlas.warnings.clone();
    finish(&mut pack, sections, budget, warnings);
    pack.entity_ids = comp_ids(ctx);
    pack
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

    #[test]
    fn kinds_stringify() {
        assert_eq!(flow_kind_str(FlowKind::Sequence), "sequence");
        assert_eq!(severity_str(scc_core::Severity::Critical), "CRITICAL");
    }
}
