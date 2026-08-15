//! Startup context (Wave 14C): the deterministic Atlas + Surface fusion
//! handed to agents at session start, plus the task delta (Wave 14E) that
//! renders only *new* relevant APIs against the context ledger.
//!
//! Prompt-cache stability: the artifact hash is a pure function of
//! `(epoch, renderer_version, trust_policy, budget)` — no timestamps — so
//! the same epoch + config always yields byte-identical startup text.

use crate::context_ledger::novelty_penalty;
use crate::rank::{collect_lexical_candidates, entity_similarity, term_match, terms};
use crate::ContextCompiler;
use scc_core::kinds;
use scc_core::{
    estimate_tokens, ContextArtifact, ContextBudget, ContextItem, ContextLedger, SurfaceEntry,
    TaskSeed,
};
use std::collections::{BTreeMap, BTreeSet};

/// Renderer version: part of the artifact hash. Bump when the startup
/// renderer's output format changes (invalidates prompt-cache keys).
pub const RENDERER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The startup artifact: atlas + surface + coverage + omissions, with the
/// deterministic artifact hash.
// trace:exempt reason=internal-detail
pub struct StartupContext {
    pub atlas: String,
    pub surface: String,
    pub coverage: Vec<String>,
    pub omissions: Vec<String>,
    pub artifact: ContextArtifact,
}

/// Build the deterministic startup artifact: the existing System Atlas
/// content (capped at `budget.atlas`) fused with the System Surface Map
/// (capped at `budget.surface`), coverage warnings, and honest omissions.
// trace:v1 id=impl.scc.context.startup work=WORK-SCC-014 satisfies=REQ-SCC-IR
pub fn build_startup(
    compiler: &ContextCompiler,
    budget: &ContextBudget,
    renderer_version: &str,
) -> StartupContext {
    let epoch = compiler
        .store
        .cache_epoch()
        .unwrap_or_else(|_| "no-epoch".into());

    // Atlas: reuse the existing cached-atlas pipeline (system_atlas is
    // cache-keyed by epoch + stale set; a stale atlas is never served).
    let atlas_pack = compiler.system_atlas(Some(budget.atlas));
    let atlas = atlas_pack.content.clone();

    // Surface: Level 1 is task-agnostic; render at the budget cap.
    let surface_map = crate::surface::compile_surface_map(compiler);
    let surface = crate::surface::render_surface_map(&surface_map, Some(budget.surface));

    // MODEL COVERAGE: the compiler's existing warnings + stale paths +
    // a surface accounting line.
    let mut coverage = Vec::new();
    for w in compiler_warnings(compiler) {
        coverage.push(w);
    }
    let mut stale: Vec<String> = compiler.stale_paths.clone();
    stale.sort();
    stale.dedup();
    for p in &stale {
        coverage.push(format!("stale: {p}"));
    }
    coverage.push(format!(
        "surface map: {} entries, {} tokens rendered (budget {})",
        surface_map.entries.len(),
        estimate_tokens(&surface),
        budget.surface
    ));

    // OMISSIONS: the surface map's omitted list + atlas dropped sections.
    let mut omissions = Vec::new();
    for o in &surface_map.omitted {
        omissions.push(format!("surface: {} ({})", o.kind, o.reason));
    }
    for d in &atlas_pack.dropped_sections {
        omissions.push(format!("atlas section dropped: {d}"));
    }
    if atlas_pack.hard_truncated {
        omissions.push("atlas hard-truncated mid-section".into());
    }
    if atlas_pack.exceeded_soft_budget {
        omissions.push("atlas exceeded soft budget: kept complete, over budget".into());
    }
    if omissions.is_empty() {
        omissions.push("none".into());
    }

    let trust_policy = trust_policy_str(compiler.view.policy());

    // Deterministic artifact hash: blake3 over (epoch + renderer_version +
    // trust_policy + budget fields). The preimage never contains
    // timestamps or volatile state, so the hash is stable per epoch.
    let mut h = blake3::Hasher::new();
    h.update(b"startup-artifact-v1");
    h.update(epoch.as_bytes());
    h.update(renderer_version.as_bytes());
    h.update(trust_policy.as_bytes());
    h.update(budget.total.to_string().as_bytes());
    h.update(budget.atlas.to_string().as_bytes());
    h.update(budget.surface.to_string().as_bytes());
    h.update(budget.task_delta.to_string().as_bytes());
    h.update(budget.structural_source.to_string().as_bytes());
    let sha256 = h.finalize().to_hex().to_string();

    let mut artifact = ContextArtifact {
        kind: "startup".into(),
        epoch,
        renderer_version: renderer_version.to_string(),
        trust_policy,
        budget: budget.clone(),
        sha256,
        text: String::new(),
    };
    artifact.text = assemble_block(&atlas, &surface, &coverage, &omissions, &artifact);

    StartupContext {
        atlas,
        surface,
        coverage,
        omissions,
        artifact,
    }
}

/// The spec's startup block format. Pure function of the context struct, so
/// `build_startup(..).artifact.text == render_startup(&startup)` always.
// trace:exempt reason=internal-detail
pub fn render_startup(s: &StartupContext) -> String {
    assemble_block(&s.atlas, &s.surface, &s.coverage, &s.omissions, &s.artifact)
}

// trace:exempt reason=internal-detail
fn assemble_block(
    atlas: &str,
    surface: &str,
    coverage: &[String],
    omissions: &[String],
    artifact: &ContextArtifact,
) -> String {
    let mut out = String::new();
    out.push_str("# SCC SYSTEM CONTEXT\n");
    out.push_str(&format!(
        "<!-- artifact sha256:{} epoch:{} renderer:{} -->\n\n",
        artifact.sha256, artifact.epoch, artifact.renderer_version
    ));
    out.push_str("## SYSTEM ATLAS\n");
    out.push_str(atlas.trim_end());
    out.push_str("\n\n## SYSTEM SURFACE MAP\n");
    out.push_str(surface.trim_end());
    out.push_str("\n\n## MODEL COVERAGE\n");
    if coverage.is_empty() {
        out.push_str("(no warnings)\n");
    } else {
        for c in coverage {
            out.push_str(c);
            out.push('\n');
        }
    }
    out.push_str("\n## OMISSIONS\n");
    for o in omissions {
        out.push_str(o);
        out.push('\n');
    }
    out
}

/// Coverage warnings mirroring the compiler's pack warnings (deterministic):
/// not-indexed, stale count, high/critical drift.
// trace:exempt reason=internal-detail
fn compiler_warnings(compiler: &ContextCompiler) -> Vec<String> {
    let mut w = Vec::new();
    if compiler.store.snapshot_status().ok().flatten().is_none() {
        w.push("Repository is not indexed — run `scc index`.".into());
    }
    if !compiler.stale_paths.is_empty() {
        w.push(format!(
            "Model is stale: {} changed file(s) not yet re-indexed.",
            compiler.stale_paths.len()
        ));
    }
    if let Ok(findings) = compiler.store.drift_findings(true) {
        for (_, kind, sev, msg, _) in findings {
            if sev == "high" || sev == "critical" {
                w.push(format!("Drift [{kind}]: {msg}"));
            }
        }
    }
    w.truncate(6);
    w
}

// trace:exempt reason=internal-detail
fn trust_policy_str(p: &scc_graph::TrustPolicy) -> String {
    format!(
        "extracted={} resolved={} observed={} declared={} inferred={} floor={}",
        p.allow_extracted,
        p.allow_resolved,
        p.allow_observed,
        p.allow_declared,
        p.allow_inferred,
        p.min_inferred_confidence
    )
}

/// The kind-scoped id sets shown by the startup artifact (for ledger
/// recording): `(symbols, files, components, flows)`.
// trace:exempt reason=internal-detail
pub fn visible_ids_from_startup(
    compiler: &ContextCompiler,
    budget: &ContextBudget,
) -> (BTreeSet<String>, BTreeSet<String>, BTreeSet<String>, BTreeSet<String>) {
    let mut symbols = BTreeSet::new();
    let mut files = BTreeSet::new();
    let mut components = BTreeSet::new();
    let mut flows = BTreeSet::new();

    // Cache-hit in the CLI flow (build_startup already ran system_atlas).
    let atlas_pack = compiler.system_atlas(Some(budget.atlas));
    let map = crate::surface::compile_surface_map(compiler);

    for e in &map.entries {
        symbols.insert(e.symbol_id.clone());
        files.insert(e.path.clone());
        if let Some(c) = &e.component {
            components.insert(c.clone());
        }
    }
    // Classify the atlas pack's entity ids by kind (deterministic).
    for id in &atlas_pack.entity_ids {
        if let Some(e) = compiler.view.entity(id) {
            match e.kind.as_str() {
                kinds::SYMBOL => {
                    symbols.insert(id.clone());
                }
                kinds::FILE => {
                    files.insert(e.name.clone());
                }
                kinds::COMPONENT => {
                    components.insert(id.clone());
                }
                kinds::FLOW => {
                    flows.insert(id.clone());
                }
                _ => {}
            }
        }
    }
    (symbols, files, components, flows)
}

/// The task delta: TASK-FOCUS seed resolution, task PPR over the surface
/// map, and the *new* relevant APIs not already visible in the ledger,
/// budget-capped. Never re-dumps the Atlas.
// trace:exempt reason=internal-detail
pub fn task_delta(
    compiler: &ContextCompiler,
    goal: &str,
    visible: &ContextLedger,
    budget_tokens: usize,
) -> String {
    task_delta_with_ids(compiler, goal, visible, budget_tokens).0
}

/// `task_delta` plus the ids it rendered (for ledger recording).
// trace:exempt reason=internal-detail
pub fn task_delta_with_ids(
    compiler: &ContextCompiler,
    goal: &str,
    visible: &ContextLedger,
    budget_tokens: usize,
) -> (String, Vec<String>) {
    let goal_terms = terms(goal);

    // TASK-FOCUS seed resolution: the existing lexical ranker (FTS over
    // entities + symbols; graph attributes when FTS is unavailable) matches
    // goal terms against symbol names/signatures/components/flows/contracts.
    let candidates = collect_lexical_candidates(compiler.store, &compiler.view, goal, &[], 16);
    let seeds: Vec<TaskSeed> = candidates
        .iter()
        .map(|c| TaskSeed {
            kind: c.kind.clone(),
            id: c.id.clone(),
            weight: c.score,
        })
        .collect();
    let seed_ids: BTreeSet<String> = seeds.iter().map(|s| s.id.clone()).collect();

    let map = crate::surface::compile_surface_map(compiler);
    let ranker = crate::pagerank::SystemRanker::new(&compiler.view);
    let global = ranker.global_vector();
    let task = ranker.task_vector(&seeds);

    let mut items: Vec<ContextItem> = Vec::new();
    let mut line_of: BTreeMap<String, String> = BTreeMap::new();
    let mut api_ids: BTreeSet<String> = BTreeSet::new();
    for e in &map.entries {
        let changed = compiler.is_stale_path(&e.path);
        let novelty = novelty_penalty(visible, &e.symbol_id, changed);
        if novelty < 1.0 {
            // Already visible AND unchanged: not re-injected (spec).
            continue;
        }
        let idx = ranker.index_of(&e.symbol_id);
        let task_ppr = idx.and_then(|i| task.get(i).copied()).unwrap_or(0.0);
        let global_ppr = idx.and_then(|i| global.get(i).copied()).unwrap_or(0.0);
        let lexical = entry_lexical(e, &goal_terms);
        let criticality = if seed_ids.contains(&e.symbol_id) { 1.0 } else { 0.0 };
        let change_risk = if changed { 1.0 } else { 0.0 };
        let importance = crate::pagerank::final_importance(
            task_ppr,
            global_ppr,
            lexical,
            0.0,
            e.confidence as f64,
            criticality,
            change_risk,
            novelty,
            !seeds.is_empty(),
        );
        if importance <= 0.0 {
            continue;
        }
        let line = api_line(e);
        api_ids.insert(e.symbol_id.clone());
        line_of.insert(e.symbol_id.clone(), line);
        items.push(ContextItem {
            id: e.symbol_id.clone(),
            value: importance,
            token_cost: estimate_tokens(line_of.get(&e.symbol_id).unwrap()),
            required: criticality > 0.0,
            group: Some("api".into()),
        });
    }

    // Relevant test surfaces: symbols in test paths matched by the task
    // (skipped when the symbol is already an API entry — no duplicate ids).
    for e in compiler.view.entities_of_kind(kinds::SYMBOL) {
        let path = e.attributes.get("file").and_then(|v| v.as_str()).unwrap_or("");
        if api_ids.contains(&e.id) || !is_test_path(path) {
            continue;
        }
        let sim = entity_similarity(e, &goal_terms);
        if sim <= 0.0 {
            continue;
        }
        let line = format!("- {} ({path})", e.name);
        let id = e.id.clone();
        line_of.insert(id.clone(), line.clone());
        items.push(ContextItem {
            id,
            value: sim,
            token_cost: estimate_tokens(&line),
            required: false,
            group: Some("test".into()),
        });
    }

    let selected = crate::selector::select_with_budget(&items, budget_tokens);

    let mut api_lines: Vec<String> = Vec::new();
    let mut test_lines: Vec<String> = Vec::new();
    let mut rendered_ids: Vec<String> = Vec::new();
    for &idx in &selected {
        if idx >= items.len() {
            continue; // defensive: never panic on a misbehaving selector
        }
        let item = &items[idx];
        if let Some(line) = line_of.get(&item.id) {
            if item.group.as_deref() == Some("test") {
                test_lines.push(line.clone());
            } else {
                api_lines.push(line.clone());
            }
            rendered_ids.push(item.id.clone());
        }
    }

    let mut out = String::new();
    out.push_str("# SCC TASK DELTA\n");
    out.push_str(&format!("TASK-FOCUS: {goal}\n"));
    out.push_str("Relevant APIs not already visible:\n");
    if api_lines.is_empty() {
        out.push_str("(none)\n");
    } else {
        for l in &api_lines {
            out.push_str(l);
            out.push('\n');
        }
    }
    if !test_lines.is_empty() {
        out.push_str("Relevant test surfaces:\n");
        for l in &test_lines {
            out.push_str(l);
            out.push('\n');
        }
    }
    (out, rendered_ids)
}

/// Task-personalized surface map: entries re-ranked by task PPR +
/// importance (no novelty filter — this is a full map, not a delta).
// trace:exempt reason=internal-detail
pub fn task_surface(
    compiler: &ContextCompiler,
    goal: &str,
    budget_tokens: usize,
    explain: bool,
) -> String {
    task_surface_with_ids(compiler, goal, budget_tokens, explain).0
}

/// `task_surface` plus the rendered entry ids (for ledger recording).
// trace:exempt reason=internal-detail
pub fn task_surface_with_ids(
    compiler: &ContextCompiler,
    goal: &str,
    budget_tokens: usize,
    explain: bool,
) -> (String, Vec<String>) {
    let goal_terms = terms(goal);
    let candidates = collect_lexical_candidates(compiler.store, &compiler.view, goal, &[], 16);
    let seeds: Vec<TaskSeed> = candidates
        .iter()
        .map(|c| TaskSeed {
            kind: c.kind.clone(),
            id: c.id.clone(),
            weight: c.score,
        })
        .collect();
    let seed_ids: BTreeSet<String> = seeds.iter().map(|s| s.id.clone()).collect();

    let map = crate::surface::compile_surface_map(compiler);
    let ranker = crate::pagerank::SystemRanker::new(&compiler.view);
    let global = ranker.global_vector();
    let task = ranker.task_vector(&seeds);

    // (importance, entry) pairs, ties broken lexicographically.
    let mut ranked: Vec<(f64, &SurfaceEntry)> = Vec::new();
    for e in &map.entries {
        let idx = ranker.index_of(&e.symbol_id);
        let task_ppr = idx.and_then(|i| task.get(i).copied()).unwrap_or(0.0);
        let global_ppr = idx.and_then(|i| global.get(i).copied()).unwrap_or(0.0);
        let lexical = entry_lexical(e, &goal_terms);
        let criticality = if seed_ids.contains(&e.symbol_id) { 1.0 } else { 0.0 };
        let importance = crate::pagerank::final_importance(
            task_ppr,
            global_ppr,
            lexical,
            0.0,
            e.confidence as f64,
            criticality,
            0.0,
            1.0,
            !seeds.is_empty(),
        );
        if importance <= 0.0 {
            continue;
        }
        ranked.push((importance, e));
    }
    ranked.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.qualified_name.cmp(&b.1.qualified_name))
    });

    // Budget: keep top-ranked entries while the token cost fits.
    let mut used = 0usize;
    let mut by_component: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut rendered_ids: Vec<String> = Vec::new();
    for (importance, e) in ranked {
        let mut line = api_line(e);
        if explain {
            line.push_str(&format!(" [importance {importance:.3}]"));
            if !e.rank.reasons.is_empty() {
                line.push_str(&format!(" reasons: {}", e.rank.reasons.join("; ")));
            }
        }
        let cost = estimate_tokens(&line);
        if used + cost > budget_tokens && !by_component.is_empty() {
            break;
        }
        used += cost;
        by_component
            .entry(e.component.clone().unwrap_or_else(|| "uncategorized".into()))
            .or_default()
            .push(line);
        rendered_ids.push(e.symbol_id.clone());
    }

    let mut out = String::new();
    out.push_str(&format!("# SYSTEM SURFACE MAP (task-personalized: {goal})\n"));
    if by_component.is_empty() {
        out.push_str("(no task-matched APIs)\n");
        return (out, rendered_ids);
    }
    for (component, lines) in &by_component {
        out.push_str(&format!("\n## COMPONENT: {component}\n"));
        for l in lines {
            out.push_str(l);
            out.push('\n');
        }
    }
    (out, rendered_ids)
}

// trace:exempt reason=internal-detail
fn entry_lexical(e: &SurfaceEntry, goal_terms: &BTreeSet<String>) -> f64 {
    if goal_terms.is_empty() {
        return 0.0;
    }
    let name_terms = terms(&e.qualified_name);
    let sig_terms = terms(&e.source_signature);
    let name_hits = goal_terms
        .iter()
        .filter(|g| name_terms.iter().any(|n| term_match(g, n)))
        .count();
    let sig_hits = goal_terms
        .iter()
        .filter(|g| sig_terms.iter().any(|n| term_match(g, n)))
        .count();
    (name_hits * 2 + sig_hits) as f64
}

// trace:exempt reason=internal-detail
fn api_line(e: &SurfaceEntry) -> String {
    format!(
        "- {} [{}] {}:L{}-L{} — {}",
        e.qualified_name,
        e.kind.as_str(),
        e.path,
        e.range.start_line,
        e.range.end_line,
        e.source_signature
    )
}

// trace:exempt reason=internal-detail
fn is_test_path(path: &str) -> bool {
    path.split(['/', '\\', '.']).any(|seg| {
        seg == "test"
            || seg == "tests"
            || seg == "spec"
            || seg.starts_with("test_")
            || seg.ends_with("_test")
            || seg.ends_with("_spec")
            || seg.ends_with(".spec")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

// trace:exempt reason=internal-detail
    fn fixture_compiler() -> (tempfile::TempDir, crate::ContextCompiler<'static>) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Box::leak(Box::new(
            scc_store::Store::open(&dir.path().join("scc.db"), &root).unwrap(),
        ));
        let graph = Box::leak(Box::new(scc_graph::RealityGraph::load(store).unwrap()));
        let settings = crate::ContextSettings::default();
        let comp = crate::ContextCompiler::new(store, graph, settings, Vec::new());
        (dir, comp)
    }

    #[test]
// trace:exempt reason=internal-detail
    fn render_startup_emits_spec_headers() {
        let sc = StartupContext {
            atlas: "ATLAS-BODY".into(),
            surface: "SURFACE-BODY".into(),
            coverage: vec!["stale: a.py".into()],
            omissions: vec!["none".into()],
            artifact: ContextArtifact {
                kind: "startup".into(),
                epoch: "epoch:test".into(),
                renderer_version: "test".into(),
                trust_policy: "floor=0.85".into(),
                budget: ContextBudget::default(),
                sha256: "abc".into(),
                text: String::new(),
            },
        };
        let out = render_startup(&sc);
        assert!(out.contains("# SCC SYSTEM CONTEXT"));
        assert!(out.contains("## SYSTEM ATLAS"));
        assert!(out.contains("## SYSTEM SURFACE MAP"));
        assert!(out.contains("## MODEL COVERAGE"));
        assert!(out.contains("## OMISSIONS"));
        assert!(out.contains("ATLAS-BODY"));
        assert!(out.contains("SURFACE-BODY"));
        assert!(out.contains("stale: a.py"));
        assert!(out.contains("sha256:abc"));
    }

    #[test]
// trace:exempt reason=internal-detail
    fn artifact_text_equals_rendered_block() {
        let (_dir, comp) = fixture_compiler();
        let budget = ContextBudget::default();
        let sc = build_startup(&comp, &budget, "test-renderer");
        assert_eq!(sc.artifact.text, render_startup(&sc));
        assert_eq!(sc.artifact.sha256.len(), 64);
        assert_eq!(sc.artifact.epoch, comp.store.cache_epoch().unwrap_or_default());
    }
}
