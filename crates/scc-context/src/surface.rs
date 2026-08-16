//! System Surface Map compiler (Wave 14, Level 1): the actual callable
//! code surface built from the TrustedGraphView — an Aider RepoMap
//! equivalent on System IR.
//!
//! Every SYMBOL entity in the trusted view becomes a [`SurfaceEntry`]
//! carrying exact signatures (source + canonical + semantic), visibility,
//! modifiers/annotations, component attribution, and the architectural
//! meaning attached to the symbol (flows, contracts, state ownership,
//! invocation surfaces, callers/callees). Deterministic and no-panic:
//! unparseable signatures degrade to name-only [`SemanticSignature`]s.

use crate::context_ledger::novelty_penalty;
use crate::rank::{term_match, terms};
use crate::ContextCompiler;
use scc_core::{
    estimate_tokens, kinds, predicates, ContextItem, ContextLedger, Provenance,
    SemanticParameter, SemanticSignature, SourceRange, SurfaceEntry, SurfaceKind,
    SurfaceOmission, SurfaceRank, SystemSurfaceMap, TaskSeed, Visibility,
};
use std::collections::{BTreeMap, BTreeSet};

/// Symbol-kind strings the indexer emits (write.rs core_symbol_kind),
/// plus "trait" for tolerance.
const SYMBOL_KINDS: [&str; 9] = [
    "function", "method", "class", "interface", "trait", "type", "const", "enum", "module",
];

/// When no semantic scorer is configured (`SurfaceRequest.semantic` is
/// `None`), the 10% semantic share of `final_importance` must not silently
/// vanish (the reviewer's phantom-weight complaint). It is reallocated
/// proportionally across the other BLEND weights: `final_importance` is
/// computed with `semantic = 0.0` and the blend renormalized by
/// `1 / (1 - SEMANTIC_WEIGHT)`, so a full-strength entry still totals 1.0
/// exactly as the advertised blend promises (the additive novelty term is
/// untouched). Deterministic and no-panic.
const REDISTRIBUTION_SCALE: f64 = 1.0 / (1.0 - crate::pagerank::SEMANTIC_WEIGHT);

/// The deepest structural compression level for the hard-max invariant:
/// 0 = full entry, 1 = first signature line only, 2 = canonical
/// abbreviated signature, 3 = symbol identity only (kind + name). Level 3
/// always fits any realistic hard max (a kind + name is a few tokens), so
/// the progressive-compression loop terminates; entries are never dropped.
const MAX_COMPRESSION: u8 = 3;

// ---------------------------------------------------------------------------
// Top-level API
// ---------------------------------------------------------------------------

/// Build the System Surface Map (Level 1) from the trusted view.
// trace:v1 id=impl.scc.surface work=WORK-SCC-014 satisfies=REQ-SCC-IR
pub fn compile_surface_map(compiler: &ContextCompiler) -> SystemSurfaceMap {
    let view = &compiler.view;

    // ---- attribution tables ----
    let mut symbol_comp_id: BTreeMap<String, String> = BTreeMap::new();
    let mut comp_names: BTreeMap<String, String> = BTreeMap::new();
    for c in view.components() {
        comp_names.insert(c.id.clone(), c.name.clone());
        for r in sorted_rels(view.out_pred(&c.id, predicates::CONTAINS)) {
            for sr in sorted_rels(view.out_pred(&r.object, predicates::CONTAINS)) {
                symbol_comp_id.insert(sr.object.clone(), c.id.clone());
            }
        }
    }
    let mut subsys_of_comp: BTreeMap<String, String> = BTreeMap::new();
    for kind in [kinds::SUBSYSTEM, kinds::SERVICE] {
        for e in view.entities_of_kind(kind) {
            for r in sorted_rels(view.out_pred(&e.id, predicates::CONTAINS)) {
                subsys_of_comp
                    .entry(r.object.clone())
                    .or_insert_with(|| e.name.clone());
            }
        }
    }

    // invocation surfaces
    let surfaces = scc_graph::flows::invocation_surfaces(view.graph);
    let mut surface_by_symbol: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for s in surfaces {
        surface_by_symbol
            .entry(s.symbol.clone())
            .or_default()
            .push((s.kind.as_str().to_string(), s.trigger.clone()));
    }

    let mut entries: Vec<SurfaceEntry> = Vec::new();
    for e in view.entities_of_kind(kinds::SYMBOL) {
        let Some(kind_str) = e.attributes.get("kind").and_then(|v| v.as_str()) else {
            continue;
        };
        if !SYMBOL_KINDS.contains(&kind_str) {
            continue;
        }
        entries.push(build_entry(
            compiler,
            e,
            kind_str,
            &symbol_comp_id,
            &comp_names,
            &subsys_of_comp,
            &surface_by_symbol,
        ));
    }
    entries.sort_by(|a, b| {
        a.qualified_name
            .cmp(&b.qualified_name)
            .then_with(|| a.id.cmp(&b.id))
    });

    let store = compiler.store;
    let mut map = SystemSurfaceMap {
        repository: store.repository().name,
        revision: compiler.revision(),
        epoch: store
            .cache_epoch()
            .unwrap_or_else(|_| "no-epoch".into()),
        entries,
        token_count: 0,
        omitted: Vec::new(),
    };
    let full = render_surface_map(&map, None);
    map.token_count = estimate_tokens(&full);
    map
}

/// Deterministic, budget-capped render of a [`SystemSurfaceMap`].
// trace:exempt reason=internal-detail
pub fn render_surface_map(map: &SystemSurfaceMap, budget_tokens: Option<usize>) -> String {
    let budget_chars = budget_tokens.map(|t| t.saturating_mul(4));
    let (body, omitted) = render_entry_groups(&map.entries, budget_chars);
    let mut out = String::from("SCC SYSTEM SURFACE MAP\n\n");
    out.push_str(&body);
    if !omitted.is_empty() {
        out.push('\n');
        out.push_str("OMITTED (token budget exceeded):\n");
        for (kind, count) in &omitted {
            out.push_str(&format!("  {count} lower-ranked {kind} definitions\n"));
        }
    }
    out
}

/// Render a set of entries grouped by (component, subsystem, path) — the
/// shared core of [`render_surface_map`] and the budget-selected subset
/// renderer. Returns the body text plus per-kind counts of entries cut by
/// the char budget (empty when no budget or nothing was cut).
// trace:exempt reason=internal-detail
fn render_entry_groups(
    entries: &[SurfaceEntry],
    budget_chars: Option<usize>,
) -> (String, BTreeMap<String, usize>) {
    group_and_render(entries, budget_chars, 0, false)
}

/// [`render_entry_groups`] with the pipeline's render options: the
/// structural compression level (0 full .. 3 identity-only) and per-entry
/// score decomposition (explain mode). `budget_chars` is the char ceiling
/// for the plain map render; the selected-subset render passes `None` (the
/// selection already enforced the token budget).
// trace:exempt reason=internal-detail
fn group_and_render(
    entries: &[SurfaceEntry],
    budget_chars: Option<usize>,
    level: u8,
    explain: bool,
) -> (String, BTreeMap<String, usize>) {
    let mut groups: BTreeMap<(String, String, String), Vec<&SurfaceEntry>> = BTreeMap::new();
    for e in entries {
        let comp = e.component.clone().unwrap_or_else(|| "(unattributed)".to_string());
        let sub = e.subsystem.clone().unwrap_or_default();
        groups.entry((comp, sub, e.path.clone())).or_default().push(e);
    }

    let mut out = String::new();
    let mut total_chars = 0usize;
    let mut omitted: BTreeMap<String, usize> = BTreeMap::new();
    let mut cut = false;

    for ((comp, sub, path), mut es) in groups {
        es.sort_by(|a, b| entry_order(a, b));
        let header = group_header(&comp, &sub, &path);
        let mut blocks: Vec<String> = Vec::new();
        let mut block_chars: usize = 0;
        for e in es {
            if cut {
                *omitted.entry(e.kind.as_str().to_string()).or_insert(0) += 1;
                continue;
            }
            let rank = if explain { Some(&e.rank) } else { None };
            let block = render_entry_opt(e, level, rank);
            let bc = block.chars().count();
            if fits(total_chars + header.chars().count() + block_chars + bc, budget_chars) {
                blocks.push(block);
                block_chars += bc;
            } else {
                cut = true;
                *omitted.entry(e.kind.as_str().to_string()).or_insert(0) += 1;
            }
        }
        if !blocks.is_empty() {
            out.push_str(&header);
            for b in blocks {
                out.push_str(&b);
            }
            total_chars += header.chars().count() + block_chars;
        }
    }
    (out, omitted)
}

// ---------------------------------------------------------------------------
// Production selection pipeline (Wave 14F)
// ---------------------------------------------------------------------------

/// The spec's global surface budget quotas (keys consumed by
/// `selector::enforce_quotas`): 30% public/entrypoint, 25% core impl, 15%
/// types/interfaces, 10% state owners, 10% contract APIs, 10% flow-critical.
// trace:exempt reason=internal-detail
pub fn surface_quotas() -> Vec<(String, f64)> {
    vec![
        ("public".to_string(), 0.30),
        ("core".to_string(), 0.25),
        ("types".to_string(), 0.15),
        ("state".to_string(), 0.10),
        ("contract".to_string(), 0.10),
        ("flow".to_string(), 0.10),
    ]
}

/// The quota bucket of an entry: public/entrypoint surfaces first, then
/// architectural meaning (state owners, contract APIs, flow participants),
/// then types, then core implementation.
// trace:exempt reason=internal-detail
fn quota_kind(e: &SurfaceEntry) -> &'static str {
    if e.exported || e.visibility == Visibility::Public || !e.invocation_surfaces.is_empty() {
        "public"
    } else if !e.state_authorities.is_empty() {
        "state"
    } else if !e.contracts.is_empty() {
        "contract"
    } else if !e.flows.is_empty() {
        "flow"
    } else if matches!(
        e.kind,
        SurfaceKind::Class
            | SurfaceKind::Interface
            | SurfaceKind::Trait
            | SurfaceKind::Enum
            | SurfaceKind::Type
    ) {
        "types"
    } else {
        "core"
    }
}

/// The names/statements of every invariant in the view — shared by the
/// required-coverage check (contracts naming an invariant are critical)
/// and by the explain reasons (`invariant-enforcing`).
// trace:exempt reason=internal-detail
fn invariant_names(view: &scc_graph::TrustedGraphView) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for inv in view.invariants() {
        let name = inv.id.rsplit('/').next().unwrap_or(&inv.id).to_string();
        names.push(name);
        names.push(inv.statement.clone());
    }
    names
}

/// Entries the pipeline MUST never omit: critical invocation surfaces
/// (non-empty `invocation_surfaces`), invariant-enforcing APIs (contracts
/// containing an invariant name), primary flow entrypoints (entrypoint of a
/// triggered flow), and state owners of critical (invariant-scoped) state.
// trace:exempt reason=internal-detail
fn required_ids(map: &SystemSurfaceMap, compiler: &ContextCompiler) -> BTreeSet<String> {
    let view = &compiler.view;
    let mut required: BTreeSet<String> = BTreeSet::new();

    // Invariant names + critical state ids (state entities named in an
    // invariant's scope are guarded by the invariant → critical).
    let inv_names = invariant_names(view);
    let mut critical_state: BTreeSet<String> = BTreeSet::new();
    for inv in view.invariants() {
        for scope_id in &inv.scope {
            if let Some(e) = view.entity(scope_id) {
                if e.kind == kinds::STATE {
                    critical_state.insert(scope_id.clone());
                }
            }
        }
    }
    let state_name_to_id: BTreeMap<String, String> = view
        .entities_of_kind(kinds::STATE)
        .into_iter()
        .map(|e| (e.name.clone(), e.id.clone()))
        .collect();

    // Primary flow entrypoints: entrypoint symbol of a triggered flow
    // (externally initiated flows are the user-facing entry flows).
    let mut primary_eps: BTreeSet<String> = BTreeSet::new();
    for f in view.flows() {
        let triggered = f.trigger.as_deref().map(|t| !t.is_empty()).unwrap_or(false);
        if triggered {
            if let Some(ep) = f.attributes.get("entrypoint").and_then(|v| v.as_str()) {
                primary_eps.insert(ep.to_string());
            }
        }
    }

    for e in &map.entries {
        // A plain public-API export is NOT a critical coverage item — the
        // ranker decides whether it earns context. Only concrete
        // invocation surfaces (http/cli/queue/process/schedule/plugin/
        // lifecycle/event...) are critical coverage; otherwise every
        // exported symbol of a large repo becomes 'required' and the
        // budget never bites (the reviewer's quota/coverage stress case).
        let mut req = e
            .invocation_surfaces
            .iter()
            .any(|s| !s.starts_with("public_api:"));
        if !req && primary_eps.contains(&e.symbol_id) {
            req = true;
        }
        if !req {
            req = e.contracts.iter().any(|c| {
                inv_names
                    .iter()
                    .any(|n| !n.is_empty() && c.to_lowercase().contains(&n.to_lowercase()))
            });
        }
        if !req {
            req = e.state_authorities.iter().any(|auth| {
                state_name_to_id
                    .get(auth)
                    .map(|id| critical_state.contains(id))
                    .unwrap_or(false)
            });
        }
        if req {
            required.insert(e.id.clone());
        }
    }
    required
}

/// The explain reasons for one entry, populated from the evidence the
/// entry carries (seeds, flows, visibility, state ownership, invariant
/// contracts, invocation surfaces, staleness) — the same signals that
/// drive the required-coverage and criticality decisions. Deterministic
/// fixed order.
// trace:exempt reason=internal-detail
fn entry_reasons(
    e: &SurfaceEntry,
    goal: Option<&str>,
    seed_ids: &BTreeSet<String>,
    inv_names: &[String],
    changed: bool,
) -> Vec<String> {
    let mut reasons: Vec<String> = Vec::new();
    if seed_ids.contains(&e.symbol_id) {
        let label = e
            .component
            .as_deref()
            .filter(|c| !c.is_empty())
            .unwrap_or(goal.unwrap_or("task"));
        reasons.push(format!("task seed: {label}"));
    }
    if !e.flows.is_empty() {
        reasons.push("primary flow participant".into());
    }
    if e.exported || e.visibility == Visibility::Public {
        reasons.push("public component surface".into());
    }
    for s in &e.state_authorities {
        reasons.push(format!("owns {s}"));
    }
    if e.contracts.iter().any(|c| {
        inv_names
            .iter()
            .any(|n| !n.is_empty() && c.to_lowercase().contains(&n.to_lowercase()))
    }) {
        reasons.push("invariant-enforcing".into());
    }
    if !e.invocation_surfaces.is_empty() {
        reasons.push("concrete invocation surface".into());
    }
    if changed {
        reasons.push("change risk: modified path".into());
    }
    reasons
}

/// Render the selected subset. No budget cut here — the selection already
/// enforced the budget, and every selected entry must render so
/// `rendered_ids` matches the text exactly. `level` is the structural
/// compression level (0 full .. 3 identity-only, hard-max overflow never
/// drops entries); `explain` renders each entry's full score
/// decomposition from its populated [`SurfaceRank`].
// trace:exempt reason=internal-detail
fn render_selected(entries: &[SurfaceEntry], level: u8, explain: bool) -> String {
    let (body, _) = group_and_render(entries, None, level, explain);
    let mut out = String::from("SCC SYSTEM SURFACE MAP\n\n");
    if explain {
        out.push_str("selection scores shown per entry\n\n");
    }
    out.push_str(&body);
    out
}

/// Lexical relevance of an entry to the goal terms: name hits count double,
/// signature hits count single (shared with the task-delta pipeline).
// trace:exempt reason=internal-detail
pub(crate) fn entry_lexical(e: &SurfaceEntry, goal_terms: &BTreeSet<String>) -> f64 {
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

/// The shared pipeline tail (MMR → token-aware quotas → soft/hard budget
/// selection → render) over ranked candidates. Required entries bypass
/// MMR/quotas (never dropped within the hard max — `policy.coverage`) but
/// still pay their token cost in the final budget selection; when
/// required entries alone exceed `policy.hard_max` the render is
/// structurally compressed (annotations/modifiers/doc lines dropped,
/// entries never dropped). Returns the render result with rendered/
/// omitted ids and per-kind omission summaries.
// trace:exempt reason=internal-detail
fn finish_selection(
    map: &SystemSurfaceMap,
    ranked: Vec<(String, f64)>,
    required: &BTreeSet<String>,
    budget: usize,
    policy: &SurfacePolicy,
    stages: &SurfacePipelineStages,
    explain: bool,
) -> scc_core::SurfaceRenderResult {
    let entry_of: BTreeMap<String, &SurfaceEntry> =
        map.entries.iter().map(|e| (e.id.clone(), e)).collect();

    // Partition: required entries survive MMR/quotas unconditionally when
    // coverage is on; with coverage off they are ordinary candidates.
    let mut required_items: Vec<(String, f64)> = Vec::new();
    let mut pool: Vec<(String, f64)> = Vec::new();
    for (id, imp) in ranked {
        if policy.coverage && required.contains(&id) {
            required_items.push((id, imp));
        } else {
            pool.push((id, imp));
        }
    }

    // Compression decision: required entries alone exceeding the hard max
    // render structurally compressed (never dropped). Once triggered, the
    // whole render uses the compressed form so token accounting matches
    // the delivered text.
    let required_tokens_full: usize = required_items
        .iter()
        .filter_map(|(id, _)| entry_of.get(id))
        .map(|e| estimate_tokens(&render_entry(e)))
        .sum();
    let compress = policy.coverage && required_tokens_full > policy.hard_max;

    // Even compressed, an enormous required set must not blow the hard max
    // (the reviewer's hard-max semantics: critical facts may exceed the
    // TARGET but never the hard maximum). When the required set alone
    // exceeds hard_max, keep the highest-importance required entries that
    // fit in hard_max — never drop them silently, the omissions block
    // reports the rest. The single highest-importance required entry is
    // always kept (its signature compresses to fit).
    if policy.coverage {
        let cost_of_c = |id: &str| -> usize {
            entry_of
                .get(id)
                .map(|e| estimate_tokens(&render_entry_compressed(e)))
                .unwrap_or(1)
        };
        let mut spent_c: usize = 0;
        let mut capped: Vec<(String, f64)> = Vec::new();
        for (id, imp) in &required_items {
            let c = cost_of_c(id);
            if spent_c + c > policy.hard_max && !capped.is_empty() {
                continue;
            }
            spent_c += c;
            capped.push((id.clone(), *imp));
        }
        if capped.is_empty() && !required_items.is_empty() {
            // pathological: even the first compressed entry alone exceeds
            // hard_max — keep the single most important one (its metadata
            // is already stripped; the identity line always remains).
            capped.push(required_items[0].clone());
        }
        if capped.len() < required_items.len() {
            required_items = capped;
        }
    }

    let cost_of = |id: &str| -> usize {
        match (entry_of.get(id), compress) {
            (Some(e), true) => estimate_tokens(&render_entry_compressed(e)),
            (Some(e), false) => estimate_tokens(&render_entry(e)),
            _ => 1,
        }
    };
    let required_spent: usize = required_items.iter().map(|(id, _)| cost_of(id)).sum();
    // The pool's soft token room: what the budget leaves after required.
    let available = budget.saturating_sub(required_spent);

    // MMR budget: how many average-pool entries the remaining tokens afford
    // (diversity caps same-component crowding before the exact token cut).
    let pool_avg = if pool.is_empty() {
        1
    } else {
        (pool.iter().map(|(id, _)| cost_of(id)).sum::<usize>() / pool.len()).max(1)
    };
    let mmr_budget = available
        .saturating_div(pool_avg)
        .max(1)
        .min(pool.len());
    let diversified: Vec<(String, f64)> = if stages.mmr && policy.mmr {
        let sim = |a: &str, b: &str| -> f64 {
            let (Some(ea), Some(eb)) = (entry_of.get(a), entry_of.get(b)) else {
                return 0.0;
            };
            let same_comp = ea.component.is_some() && ea.component == eb.component;
            let same_path = !ea.path.is_empty() && ea.path == eb.path;
            if same_comp || same_path {
                1.0
            } else {
                0.0
            }
        };
        let pool_by_id: BTreeMap<String, f64> = pool.iter().cloned().collect();
        crate::selector::mmr_diversify(&pool, sim, 0.5, mmr_budget)
            .into_iter()
            .filter_map(|id| pool_by_id.get(&id).map(|v| (id.clone(), *v)))
            .collect()
    } else {
        pool.clone()
    };

    // Token-aware quotas: caps are fractions of the pool's available
    // tokens, so the composition adapts to the budget (reviewer item 6).
    // Required entries were partitioned out and may exceed their group
    // allocation; the leftover room rebalances across the pool.
    let quota_filtered: Vec<(String, f64)> = if stages.quotas && policy.quotas {
        let kind_of: BTreeMap<String, &'static str> = map
            .entries
            .iter()
            .map(|e| (e.id.clone(), quota_kind(e)))
            .collect();
        let ids = crate::selector::enforce_quotas(
            &diversified,
            |id| kind_of.get(id).copied().unwrap_or("core"),
            &surface_quotas(),
            available,
            |id| cost_of(id),
        );
        let id_set: BTreeSet<String> = ids.into_iter().collect();
        diversified
            .into_iter()
            .filter(|(id, _)| id_set.contains(id))
            .collect()
    } else {
        diversified
    };

    // Budget selection: required first (never dropped, even when they
    // alone exceed the budget), then the quota-balanced pool by
    // value/token (or rank order with the optimizer stage off).
    let mut items: Vec<ContextItem> = Vec::new();
    for (id, imp) in &required_items {
        items.push(ContextItem {
            id: id.clone(),
            value: *imp,
            token_cost: cost_of(id),
            required: true,
            group: Some("api".into()),
        });
    }
    for (id, imp) in &quota_filtered {
        items.push(ContextItem {
            id: id.clone(),
            value: *imp,
            token_cost: cost_of(id),
            required: false,
            group: Some("api".into()),
        });
    }
    let selected = if stages.optimizer {
        crate::selector::select_with_budget(&items, budget, policy.hard_max)
    } else {
        crate::selector::select_in_order(&items, budget, policy.hard_max)
    };

    let mut rendered_ids: Vec<String> = Vec::new();
    let mut selected_entries: Vec<SurfaceEntry> = Vec::new();
    for &idx in &selected {
        if idx >= items.len() {
            continue; // defensive: never panic on a misbehaving selector
        }
        let id = items[idx].id.clone();
        if let Some(e) = entry_of.get(&id) {
            rendered_ids.push(id.clone());
            selected_entries.push((*e).clone());
        }
    }

    // Omissions: every candidate not rendered, summarized per kind (honest —
    // the artifact never silently implies completeness).
    let rendered_set: BTreeSet<String> = rendered_ids.iter().cloned().collect();
    let mut omitted_ids: Vec<String> = Vec::new();
    let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
    for e in &map.entries {
        if rendered_set.contains(&e.id) {
            continue;
        }
        omitted_ids.push(e.id.clone());
        *by_kind.entry(e.kind.as_str().to_string()).or_insert(0) += 1;
    }
    let omissions: Vec<SurfaceOmission> = by_kind
        .into_iter()
        .map(|(kind, count)| SurfaceOmission {
            count,
            kind,
            reason: "not selected within budget (diversity/quotas/token budget)".into(),
        })
        .collect();

    // Hard-max invariant against the ACTUAL rendered text: when required
    // coverage forced structural compression (level 1) but the rendered
    // text still exceeds hard_max (pathological signatures), escalate the
    // compression ladder level by level — first signature line →
    // canonical abbreviated signature → symbol identity only — re-rendering
    // until estimate_tokens(text) <= hard_max. The identity level always
    // fits any realistic hard max (kind + name is a few tokens), so the
    // loop terminates; entries are never dropped.
    let mut level: u8 = if compress { 1 } else { 0 };
    let mut text = render_selected(&selected_entries, level, explain);
    let mut token_count = estimate_tokens(&text);
    while compress && token_count > policy.hard_max && level < MAX_COMPRESSION {
        level += 1;
        text = render_selected(&selected_entries, level, explain);
        token_count = estimate_tokens(&text);
    }
    scc_core::SurfaceRenderResult {
        text,
        rendered_ids,
        omitted_ids,
        omissions,
        token_count,
    }
}

// ---------------------------------------------------------------------------
// The one authoritative surface service (Wave 15.1)
// ---------------------------------------------------------------------------

/// The surface mode: global (the historical production pipeline) or task
/// (task PPR + novelty suppression with the same pipeline tail).
#[derive(Debug, Clone, Copy)]
// trace:v1 id=impl.scc.surface.mode work=WORK-SCC-015 satisfies=REQ-SCC-IR
pub enum SurfaceMode<'a> {
    Global,
    Task {
        goal: &'a str,
        /// The context ledger for novelty suppression: entries already
        /// visible AND unchanged are not re-injected. `None` disables
        /// suppression (full task-personalized map).
        visible: Option<&'a ContextLedger>,
    },
}

/// One surface render request: the mode, the token budget, whether to
/// explain selection scores, the pipeline policy, and the optional
/// semantic scorer.
#[derive(Clone, Copy)]
// trace:v1 id=impl.scc.surface.request work=WORK-SCC-015 satisfies=REQ-SCC-IR
pub struct SurfaceRequest<'a> {
    pub mode: SurfaceMode<'a>,
    pub budget: usize,
    pub explain: bool,
    pub policy: SurfacePolicy,
    /// The optional semantic scorer (SCC-071, e.g. an embedding model):
    /// when present, its per-entity score feeds the REAL 10% semantic
    /// share of `final_importance`. When `None`, the 10% share is
    /// explicitly redistributed across the other blend weights
    /// ([`REDISTRIBUTION_SCALE`]) — never a phantom weight.
    pub semantic: Option<&'a dyn crate::rank::SemanticScorer>,
}

/// Pipeline policy knobs for one surface render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// trace:v1 id=impl.scc.surface.policy work=WORK-SCC-015 satisfies=REQ-SCC-IR
pub struct SurfacePolicy {
    /// Token-aware per-kind quotas (default true).
    pub quotas: bool,
    /// MMR diversity across components/paths (default true).
    pub mmr: bool,
    /// Required coverage never dropped within the hard max (default true).
    pub coverage: bool,
    /// Absolute token ceiling: required facts may exceed `budget` but
    /// never `hard_max`; required entries alone over the hard max are
    /// structurally compressed (annotations/modifiers/doc lines dropped —
    /// the entry is never dropped).
    pub hard_max: usize,
}

// trace:exempt reason=internal-detail  # impl grouping; the constructor below is traced
impl SurfacePolicy {
    /// The default policy for a soft `budget`: quotas/MMR/coverage on and
    /// `hard_max` = budget + 20% (min +500).
    // trace:v1 id=impl.scc.surface.policy.defaults work=WORK-SCC-015 satisfies=REQ-SCC-IR
    pub fn defaults(budget: usize) -> Self {
        SurfacePolicy {
            quotas: true,
            mmr: true,
            coverage: true,
            hard_max: budget
                .saturating_add(budget / 5)
                .max(budget.saturating_add(500)),
        }
    }
}

/// Stage toggles for the ablation matrix ([`build_surface_staged`]): the
/// same pipeline with one stage switched off. All on = [`build_surface`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// trace:v1 id=impl.scc.surface.stages work=WORK-SCC-015 satisfies=REQ-SCC-IR
pub struct SurfacePipelineStages {
    /// Lexical stage: on = PPR-blended importance; off = pure lexical
    /// scores, no PPR at all.
    pub lexical: bool,
    /// Global PPR stage: off = no global vector in the blend.
    pub global_ppr: bool,
    /// Task PPR stage: off = no task seeds (global-only blend).
    pub task_ppr: bool,
    /// MMR diversity stage: off = rank order preserved, no diversification.
    pub mmr: bool,
    /// Quota stage: off = no per-kind caps.
    pub quotas: bool,
    /// Optimizer stage: off = rank-order budget cut, no value/token
    /// reordering.
    pub optimizer: bool,
}

// trace:exempt reason=internal-detail  # Default impl grouping; the fn below is traced
impl Default for SurfacePipelineStages {
    /// All stages on: `build_surface_staged(.., &SurfacePipelineStages::default())`
    /// is exactly [`build_surface`].
    // trace:v1 id=impl.scc.surface.stages.default work=WORK-SCC-015 satisfies=REQ-SCC-IR
    fn default() -> Self {
        SurfacePipelineStages {
            lexical: true,
            global_ppr: true,
            task_ppr: true,
            mmr: true,
            quotas: true,
            optimizer: true,
        }
    }
}

/// THE one authoritative surface pipeline: compile candidates →
/// heterogeneous PPR (global or task) →
/// [`pagerank::SystemRanker::project_to_symbols`] → per-entry importance
/// (`final_importance`) → required coverage → MMR diversify → token-aware
/// quotas → soft/hard budget selection → render. Global mode is the
/// historical `select_and_render_global` pipeline; Task mode adds task
/// PPR (lexical seeds, warm global start), novelty suppression against
/// the ledger, and the same MMR/quotas/selector/render tail. Every
/// surface consumer (production, CLI, MCP, plugin, benchmark ablations)
/// routes through this service — no consumer reimplements ranking.
/// Deterministic and no-panic.
// trace:v1 id=impl.scc.surface.build work=WORK-SCC-015 satisfies=REQ-SCC-IR
pub fn build_surface(
    compiler: &ContextCompiler,
    request: SurfaceRequest<'_>,
) -> scc_core::SurfaceRenderResult {
    build_surface_staged(compiler, request, &SurfacePipelineStages::default())
}

/// [`build_surface`] with stage toggles for the ablation matrix
/// (benchmark ablations toggle stages here — they never reimplement
/// ranking). `lexical` off skips PPR entirely and scores by lexical
/// match; `global_ppr` off drops the global vector; `task_ppr` off seeds
/// no task vector; `mmr` off skips diversification; `quotas` off skips
/// per-kind caps; `optimizer` off renders in importance order up to the
/// budget. Deterministic and no-panic.
// trace:v1 id=impl.scc.surface.build-staged work=WORK-SCC-015 satisfies=REQ-SCC-IR,REQ-semantic-10-live-in-final-importance,REQ-explain-renders-score-decomposition,REQ-hard-max-invariant-on-rendered-text
pub fn build_surface_staged(
    compiler: &ContextCompiler,
    request: SurfaceRequest<'_>,
    stages: &SurfacePipelineStages,
) -> scc_core::SurfaceRenderResult {
    build_surface_staged_inner(compiler, request, stages, &mut None)
}

/// 15.2-cache-seam: cache-aware sibling of [`build_surface`] (Wave 15.2,
/// REQ-global-rank-cached-per-model-epoch). When `cache` holds a valid
/// [`crate::startup::GlobalRankCache`] (loaded by
/// `crate::startup::load_global_rank_cache`), the global PPR vector +
/// symbol projection come from the cache — skipping
/// `SystemRanker::new` (the heterogeneous node graph + adjacency +
/// rarity build) and the 50 power iterations of `global_vector()`. The
/// pipeline tail (required coverage, MMR, quotas, budget selection,
/// render) runs identically, so the output is byte-identical to
/// [`build_surface`]. On a miss (`cache` is `None`) the ranker is built
/// once and the cache is FILLED (the caller persists it via
/// `crate::startup::store_global_rank_cache`). Additive: B's `semantic`
/// field lands on [`SurfaceRequest`]/[`build_surface`] independently.
// trace:v1 id=impl.scc.surface.build-cached work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching satisfies=REQ-global-rank-cached-per-model-epoch
pub fn build_surface_cached(
    compiler: &ContextCompiler,
    request: SurfaceRequest<'_>,
    cache: &mut Option<crate::startup::GlobalRankCache>,
) -> scc_core::SurfaceRenderResult {
    build_surface_staged_inner(
        compiler,
        request,
        &SurfacePipelineStages::default(),
        cache,
    )
}

/// The shared pipeline body of [`build_surface_staged`] /
/// [`build_surface_cached`]: `cache` supplies the global PPR vector +
/// projection on hit; on miss the ranker is built once and the cache
/// filled (the caller persists it). Task PPR still constructs the
/// ranker when the cache lacks the adjacency (documented ceiling — the
/// rank cache stores the global vector + projection only).
// trace:exempt reason=internal-detail  # shared body; the traced entries are build_surface_staged / build_surface_cached
fn build_surface_staged_inner(
    compiler: &ContextCompiler,
    request: SurfaceRequest<'_>,
    stages: &SurfacePipelineStages,
    cache: &mut Option<crate::startup::GlobalRankCache>,
) -> scc_core::SurfaceRenderResult {
    let (goal, visible): (Option<&str>, Option<&ContextLedger>) = match &request.mode {
        SurfaceMode::Global => (None, None),
        SurfaceMode::Task { goal, visible } => (Some(goal), *visible),
    };
    let goal_terms = goal.map(terms).unwrap_or_default();

    // Task seeds: lexical candidate resolution (task mode only, and only
    // while the task-ppr stage is on).
    let mut seeds: Vec<TaskSeed> = Vec::new();
    let mut seed_ids: BTreeSet<String> = BTreeSet::new();
    if stages.task_ppr {
        if let Some(g) = goal {
            let candidates = crate::rank::collect_lexical_candidates(
                compiler.store,
                &compiler.view,
                g,
                &[],
                16,
            );
            seeds = candidates
                .iter()
                .map(|c| TaskSeed {
                    kind: c.kind.clone(),
                    id: c.id.clone(),
                    weight: c.score,
                })
                .collect();
            seed_ids = seeds.iter().map(|s| s.id.clone()).collect();
        }
    }

    let mut map = compile_surface_map(compiler);
    // 15.2-cache-seam: on hit, the global vector + projection come from
    // the cache (no SystemRanker::new, no 50 power iterations); on miss
    // the ranker is built once and the cache filled — node_symbol_map is
    // exactly what the pipeline consumes as `global_of`, so the render is
    // byte-identical either way. `task_ranker` carries the built ranker
    // to the task-vector step when both are needed.
    let mut global_of: BTreeMap<String, f64> = BTreeMap::new();
    let mut task_ranker: Option<crate::pagerank::SystemRanker<'_>> = None;
    if stages.global_ppr {
        if let Some(c) = cache.as_ref() {
            global_of = c.node_symbol_map.clone();
        } else {
            let ranker = crate::pagerank::SystemRanker::new(&compiler.view);
            let gv = ranker.global_vector();
            global_of = ranker.project_to_symbols(&gv).into_iter().collect();
            *cache = Some(crate::startup::GlobalRankCache {
                epoch: compiler
                    .store
                    .cache_epoch()
                    .unwrap_or_else(|_| "no-epoch".into()),
                policy: crate::startup::trust_policy_str(compiler.view.policy()),
                salt: compiler.settings.rank_salt.clone(),
                global_vector: gv,
                node_symbol_map: global_of.clone(),
                candidates_epoch: map.epoch.clone(),
                candidate_ids: map.entries.iter().map(|e| e.id.clone()).collect(),
                hits: 0,
            });
            task_ranker = Some(ranker);
        }
    }
    let task_of: BTreeMap<String, f64> = if stages.task_ppr && !seeds.is_empty() {
        // ponytail: task PPR still builds the ranker when the cache hit
        // path is active (the cache stores the global vector + projection
        // only, not the adjacency); cache the adjacency too if task-mode
        // startup latency ever matters.
        let ranker = match task_ranker {
            Some(r) => r,
            None => crate::pagerank::SystemRanker::new(&compiler.view),
        };
        ranker
            .project_to_symbols(&ranker.task_vector(&seeds))
            .into_iter()
            .collect()
    } else {
        BTreeMap::new()
    };

    let required = required_ids(&map, compiler);
    let has_task = !seeds.is_empty();
    let inv_names = invariant_names(&compiler.view);
    let mut ranked: Vec<(String, f64)> = Vec::new();
    let mut ranks: BTreeMap<String, SurfaceRank> = BTreeMap::new();
    for e in &map.entries {
        let changed = compiler.is_stale_path(&e.path);
        let novelty = match visible {
            Some(ledger) => novelty_penalty(ledger, &e.symbol_id, changed),
            None => 1.0,
        };
        if novelty < 1.0 {
            // already visible AND unchanged: not re-injected (spec)
            continue;
        }
        let task_ppr = task_of.get(&e.symbol_id).copied().unwrap_or(0.0);
        let global_ppr = global_of.get(&e.symbol_id).copied().unwrap_or(0.0);
        let lexical = entry_lexical(e, &goal_terms);
        let criticality = if seed_ids.contains(&e.symbol_id) || required.contains(&e.id) {
            1.0
        } else {
            0.0
        };
        let change_risk = if changed { 1.0 } else { 0.0 };
        // The semantic score is the REAL 10% share: the scorer rates the
        // entry's logical symbol entity against the goal. Absent a scorer
        // the share is zero and the blend is renormalized (see below) —
        // never a phantom weight.
        let semantic = match request.semantic {
            Some(scorer) => compiler
                .view
                .entity(&e.symbol_id)
                .map(|en| scorer.score(goal.unwrap_or(""), en))
                .unwrap_or(0.0)
                .clamp(0.0, 1.0),
            None => 0.0,
        };
        let (importance, semantic_component) = if stages.lexical {
            // final_importance is linear in every input, so computing the
            // blend with novelty = 0.0 and adding NOVELTY_WEIGHT * novelty
            // reproduces the documented blend exactly while keeping the
            // novelty term additive on top (final_importance's own
            // contract — the six blend weights sum to 1.0, novelty is the
            // documented +0.05 bonus).
            let blend = crate::pagerank::final_importance(
                task_ppr,
                global_ppr,
                lexical,
                semantic,
                e.confidence as f64,
                criticality,
                change_risk,
                0.0,
                has_task,
            );
            let total = match request.semantic {
                // Scorer present: the 10% semantic share is real.
                Some(_) => blend + crate::pagerank::NOVELTY_WEIGHT * novelty,
                // No scorer: the 10% share is reallocated proportionally
                // across the other blend weights (REDISTRIBUTION_SCALE) so
                // the total still reflects the advertised blend — a
                // full-strength entry still totals 1.0 + novelty instead
                // of the phantom-hole 0.9 + novelty.
                None => blend * REDISTRIBUTION_SCALE + crate::pagerank::NOVELTY_WEIGHT * novelty,
            };
            (total, semantic)
        } else {
            // lexical stage off: pure lexical scores, no PPR blend
            (lexical, 0.0)
        };
        if importance <= 0.0 && !required.contains(&e.id) {
            continue;
        }
        ranks.insert(
            e.id.clone(),
            SurfaceRank {
                task_ppr,
                global_ppr,
                lexical,
                semantic: semantic_component,
                confidence: e.confidence as f64,
                criticality,
                change_risk,
                novelty,
                total: importance,
                reasons: entry_reasons(e, goal, &seed_ids, &inv_names, changed),
            },
        );
        ranked.push((e.id.clone(), importance));
    }
    // The compiled map carries the per-entry SurfaceRank so every consumer
    // (render, MCP JSON, tests) sees the decomposition; the selected
    // clones inherit it into the render.
    for e in &mut map.entries {
        if let Some(r) = ranks.get(&e.id) {
            e.rank = r.clone();
        }
    }
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    finish_selection(
        &map,
        ranked,
        &required,
        request.budget,
        &request.policy,
        stages,
        request.explain,
    )
}

/// The FULL production global surface pipeline (historical entry point —
/// kept as a thin wrapper over [`build_surface`] so the traced contract
/// and legacy callers stay stable): global heterogeneous PPR, required
/// coverage, MMR, quotas, budget selection, render.
// trace:v1 id=impl.scc.surface.select-and-render-global work=WORK-SCC-014 satisfies=REQ-SCC-IR
pub fn select_and_render_global(
    compiler: &ContextCompiler,
    budget: usize,
) -> scc_core::SurfaceRenderResult {
    build_surface(
        compiler,
        SurfaceRequest {
            mode: SurfaceMode::Global,
            budget,
            explain: false,
            policy: SurfacePolicy::defaults(budget),
                    semantic: None,
        },
    )
}

/// The FULL production task surface pipeline (historical entry point —
/// kept as a thin wrapper over [`build_surface`] so the traced contract
/// and legacy callers stay stable): task PPR + novelty suppression
/// against the ledger + the same MMR/quotas/selector/render tail.
// trace:v1 id=impl.scc.surface.select-and-render-task work=WORK-SCC-014 satisfies=REQ-SCC-IR
pub fn select_and_render_task(
    compiler: &ContextCompiler,
    goal: &str,
    budget: usize,
    visible: &ContextLedger,
) -> scc_core::SurfaceRenderResult {
    build_surface(
        compiler,
        SurfaceRequest {
            mode: SurfaceMode::Task {
                goal,
                visible: Some(visible),
            },
            budget,
            explain: false,
            policy: SurfacePolicy::defaults(budget),
                    semantic: None,
        },
    )
}

// ---------------------------------------------------------------------------
// Entry builder
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
// trace:exempt reason=internal-detail
fn build_entry(
    compiler: &ContextCompiler,
    e: &scc_core::Entity,
    kind_str: &str,
    symbol_comp_id: &BTreeMap<String, String>,
    comp_names: &BTreeMap<String, String>,
    subsys_of_comp: &BTreeMap<String, String>,
    surface_by_symbol: &BTreeMap<String, Vec<(String, String)>>,
) -> SurfaceEntry {
    let view = &compiler.view;
    let name = e.name.clone();
    let simple = name.rsplit('.').next().unwrap_or(&name).to_string();
    let parent = attr_str(e, "parent");
    let exported = e.attributes.get("exported").and_then(|v| v.as_bool()) == Some(true);
    let file = attr_str(e, "file").unwrap_or_default();
    let start = attr_u32(e, "start_line");
    let end = attr_u32(e, "end_line");

    // Exact declaration header wins: the indexer's `decl_header` attr is
    // the full header as written (untruncated, multi-line preserved);
    // falls back to the legacy `signature` attr, then a synthesized
    // name-only signature.
    let decl = attr_str(e, "decl_header").unwrap_or_default();
    let source_sig = if decl.trim().is_empty() {
        attr_str(e, "signature").unwrap_or_default()
    } else {
        decl
    };
    let source_signature = if source_sig.trim().is_empty() {
        synthesized_signature(kind_str, &simple)
    } else {
        source_sig
    };
    let parsed_inner = parse_sig_inner(&source_signature);
    let parsed_sig = parse_signature(&source_signature, &name, parent.as_deref());

    // Overload-sensitive entry id: when the indexer recorded an
    // `overload_index` attr (0-based per same-name-in-file), the entry id
    // carries `#overload<N>` so same-name overloads stay separate entries;
    // `symbol_id` keeps pointing at the logical symbol (indexer overload
    // entity ids are `{symbol_id}` for index 0 and `{symbol_id}#{N}` for
    // N >= 1, so the suffix is stripped to recover the logical id).
    let overload = e
        .attributes
        .get("overload_index")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize);
    let (entry_id, symbol_id) = match overload {
        Some(n) => {
            let logical = if n >= 1 {
                e.id.trim_end_matches(&format!("#{n}")).to_string()
            } else {
                e.id.clone()
            };
            (format!("{}#overload{}", logical, n), logical)
        }
        None => (e.id.clone(), e.id.clone()),
    };

    let kind = map_kind(kind_str, &name, parent.as_deref());
    let qualified = if kind_str == "method" {
        match &parent {
            Some(p) if !p.is_empty() => format!("{}.{}", p, simple),
            _ => name.clone(),
        }
    } else {
        name.clone()
    };

    let visibility = entry_visibility(
        exported,
        parent.as_deref(),
        &parsed_inner.modifiers,
        parsed_inner.ret_before_name.as_deref(),
    );

    // modifiers: async/static/final/abstract/readonly (+variadic)
    const SURFACE_MODIFIERS: [&str; 5] = ["async", "static", "final", "abstract", "readonly"];
    let mut modifiers: Vec<String> = parsed_inner
        .modifiers
        .iter()
        .filter(|m| SURFACE_MODIFIERS.contains(&m.as_str()))
        .cloned()
        .collect();
    if parsed_sig.parameters.iter().any(|p| p.variadic)
        && !modifiers.iter().any(|m| m == "variadic")
    {
        modifiers.push("variadic".into());
    }

    // annotations
    let mut annotations: Vec<String> = Vec::new();
    for r in sorted_rels(view.in_pred(&e.id, predicates::ANNOTATES)) {
        if let Some(a) = view.entity(&r.subject) {
            annotations.push(a.name.clone());
        }
    }
    annotations.sort();
    annotations.dedup();

    // flows
    let mut flows: BTreeSet<String> = BTreeSet::new();
    for f in view.flows() {
        if f.steps
            .iter()
            .any(|s| step_matches(s, &e.id, &name, &qualified, &file))
        {
            flows.insert(f.name.clone());
        }
    }

    // contracts
    let mut contracts: BTreeSet<String> = BTreeSet::new();
    // http: ROUTE handler == this symbol
    for r in view.entities_of_kind(kinds::ROUTE) {
        if r.attributes.get("handler").and_then(|v| v.as_str()) == Some(&e.id) {
            let m = attr_str(r, "method").unwrap_or_default();
            let p = attr_str(r, "path").unwrap_or_default();
            if !p.is_empty() {
                contracts.insert(format!("http: {}", format!("{} {}", m, p).trim()));
            }
        }
    }
    // cli flags
    if let Some(flags) = e.attributes.get("cli_flags").and_then(|v| v.as_array()) {
        for f in flags {
            if let Some(s) = f.as_str() {
                contracts.insert(format!("cli: {}", s));
            }
        }
    }
    // event topics
    for pred in [predicates::CONSUMES, predicates::PUBLISHES] {
        for rel in sorted_rels(view.out_pred(&e.id, pred)) {
            if let Some(t) = view.entity(&rel.object) {
                if t.kind == kinds::TOPIC {
                    contracts.insert(format!("event: {}", t.name));
                }
            }
        }
    }
    // REGISTERS -> CONTRACT entities
    for rel in sorted_rels(view.out_pred(&e.id, predicates::REGISTERS)) {
        if let Some(t) = view.entity(&rel.object) {
            if t.kind == kinds::CONTRACT {
                contracts.insert(format!("register:{}", view.name_of(&t.id)));
            }
        }
    }
    // DEFINES -> SCHEMA entities
    for rel in sorted_rels(view.out_pred(&e.id, predicates::DEFINES)) {
        if let Some(t) = view.entity(&rel.object) {
            if t.kind == kinds::SCHEMA {
                contracts.insert(format!("schema:{}", t.name));
            }
        }
    }

    // state authorities
    let mut state_authorities: Vec<String> = Vec::new();
    for rel in sorted_rels(view.out_pred(&e.id, predicates::OWNS)) {
        if let Some(t) = view.entity(&rel.object) {
            if t.kind == kinds::STATE || t.kind == kinds::REACTIVE {
                state_authorities.push(t.name.clone());
            }
        }
    }
    state_authorities.sort();
    state_authorities.dedup();

    // invocation surfaces
    let mut inv: Vec<String> = Vec::new();
    if let Some(surfs) = surface_by_symbol.get(&e.id) {
        for (kind, trigger) in surfs {
            inv.push(format!("{}: {}", kind, trigger));
        }
    }
    if let Some(eps) = e.attributes.get("entrypoints").and_then(|v| v.as_array()) {
        for ep in eps {
            if let Some(s) = ep.as_str() {
                inv.push(format!("entrypoint:{}", s));
            }
        }
    }
    inv.sort();

    // callers / callees
    let mut callers: Vec<String> = Vec::new();
    for r in sorted_rels(view.in_pred(&e.id, predicates::CALLS)) {
        callers.push(view.name_of(&r.subject));
    }
    callers.sort();
    callers.dedup();
    callers.truncate(12);
    let mut callees: Vec<String> = Vec::new();
    for r in sorted_rels(view.out_pred(&e.id, predicates::CALLS)) {
        callees.push(view.name_of(&r.object));
    }
    callees.sort();
    callees.dedup();
    callees.truncate(12);

    // provenance
    let has_resolved_call = view
        .out_pred(&e.id, predicates::CALLS)
        .iter()
        .any(|r| r.provenance == Provenance::Resolved);
    let provenance = if has_resolved_call {
        Provenance::Resolved
    } else {
        Provenance::Extracted
    };
    let confidence: f32 = if has_resolved_call { 1.0 } else { 0.85 };

    // component / subsystem
    let component = symbol_comp_id
        .get(&e.id)
        .and_then(|cid| comp_names.get(cid))
        .cloned();
    let subsystem = symbol_comp_id
        .get(&e.id)
        .and_then(|cid| subsys_of_comp.get(cid))
        .cloned();

    SurfaceEntry {
        id: entry_id,
        symbol_id,
        qualified_name: qualified,
        kind,
        path: file.clone(),
        range: SourceRange::new(file, start, end),
        source_signature: source_signature.clone(),
        canonical_signature: canonicalize(&source_signature),
        semantic_signature: parsed_sig,
        visibility,
        exported,
        modifiers,
        annotations,
        component,
        subsystem,
        flows: flows.into_iter().collect(),
        contracts: contracts.into_iter().collect(),
        state_authorities,
        invocation_surfaces: inv,
        callers,
        callees,
        provenance,
        confidence,
        rank: SurfaceRank::default(),
    }
}

// ---------------------------------------------------------------------------
// Semantic signature parser
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
// trace:exempt reason=internal-detail
struct ParsedSignature {
    modifiers: Vec<String>,
    async_: bool,
    name: String,
    owner: Option<String>,
    ret_before_name: Option<String>,
    params: Vec<String>,
    generic_parameters: Vec<String>,
    returns: Option<String>,
    constraints: Vec<String>,
}

// trace:exempt reason=internal-detail
fn parse_signature(sig: &str, fallback_name: &str, parent: Option<&str>) -> SemanticSignature {
    let p = parse_sig_inner(sig);
    let name = if p.name.is_empty() {
        fallback_name.to_string()
    } else {
        p.name
    };
    let mut parameters: Vec<SemanticParameter> = Vec::new();
    for raw in &p.params {
        if let Some(sp) = parse_param(raw) {
            parameters.push(sp);
        }
    }
    let mut generic_parameters = p.generic_parameters;
    generic_parameters.sort();
    generic_parameters.dedup();
    SemanticSignature {
        name,
        owner: p.owner.or_else(|| parent.map(|s| s.to_string())),
        visibility: visibility_from_modifiers(&p.modifiers),
        async_: p.async_,
        generic_parameters,
        parameters,
        returns: p.returns,
        constraints: p.constraints,
    }
}

// trace:exempt reason=internal-detail
fn parse_sig_inner(sig: &str) -> ParsedSignature {
    let mut p = ParsedSignature::default();
    let s = sig.trim();
    if s.is_empty() {
        return p;
    }

    // 1. modifier + callable-keyword prefix
    let mut rest: &str = s;
    let mut consumed_keyword: Option<String> = None;
    while let Some((word, after)) = leading_word(rest) {
        let is_mod = is_modifier_word(&word);
        let is_kw = !is_mod && is_callable_keyword(&word);
        if !is_mod && !is_kw {
            break;
        }
        let mut after = after;
        let token = if after.starts_with('(') {
            match take_group(after, '(', ')') {
                Some((g, r)) => {
                    after = r;
                    format!("{}({})", word, g)
                }
                None => word.clone(),
            }
        } else {
            word.clone()
        };
        if is_mod {
            if word == "async" {
                p.async_ = true;
            }
            if !p.modifiers.iter().any(|m| m == &token) {
                p.modifiers.push(token);
            }
        } else {
            consumed_keyword = Some(word);
        }
        rest = after.trim_start();
    }

    // 2. Go receiver group
    let is_go = matches!(consumed_keyword.as_deref(), Some("func") | Some("function"));
    if is_go && rest.starts_with('(') {
        if let Some((group, after)) = take_group(rest, '(', ')') {
            p.owner = receiver_owner(&group);
            rest = after.trim_start();
        }
    }

    // 3. Parameter list
    let (prefix, params, tail) = match split_params(rest) {
        Some(x) => (x.0, x.1, x.2),
        None => {
            if let Some((w, _)) = leading_word(rest) {
                if is_identifier(&w) {
                    p.name = w;
                }
            }
            return p;
        }
    };
    p.params = params;

    // 4. Name + generics + java-style return type
    let prefix = prefix.trim();
    let (name_region, generics) = if prefix.ends_with('>') {
        match find_matching_open(prefix, '<', '>') {
            Some(idx) => (&prefix[..idx], Some(&prefix[idx..])),
            None => (prefix, None),
        }
    } else {
        (prefix, None)
    };
    let words: Vec<&str> = name_region.split_whitespace().collect();
    if let Some(last) = words.last() {
        if is_identifier(last) {
            p.name = last.to_string();
            if words.len() > 1 {
                p.ret_before_name = Some(words[..words.len() - 1].join(" "));
            }
        } else if let Some(first) = words.first() {
            if is_identifier(first) {
                p.name = first.to_string();
            }
        }
    }

    if let Some(g) = generics {
        let inner = g.trim_start_matches('<').trim_end_matches('>');
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for item in split_top(inner, ',') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            let (name_part, bound) = match item.find(':') {
                Some(idx) => (&item[..idx], Some(item[idx + 1..].trim())),
                None => (item, None),
            };
            let gn = name_part.trim();
            if gn.is_empty() {
                continue;
            }
            if seen.insert(gn.to_string()) {
                p.generic_parameters.push(gn.to_string());
            }
            if let Some(b) = bound {
                let c = format!("{}: {}", gn, b);
                if !c.is_empty() && seen.insert(c.clone()) {
                    p.constraints.push(c);
                }
            }
        }
    }

    // 5. Tail: returns + where/throws constraints
    let (ret_text, constraints) = split_tail(tail.trim());
    p.returns = parse_return(&ret_text, p.ret_before_name.as_deref());
    if let Some(cs) = constraints {
        for c in split_top(&cs, ',') {
            let c = c.trim();
            if !c.is_empty() && !p.constraints.iter().any(|x| x == c) {
                p.constraints.push(c.to_string());
            }
        }
    }
    p
}

// trace:exempt reason=internal-detail
fn split_tail(tail: &str) -> (String, Option<String>) {
    let t = tail.trim();
    for marker in ["where ", "throws ", " where ", " throws "] {
        if let Some(idx) = find_depth0_pattern(t, marker) {
            let before = if idx == 0 {
                String::new()
            } else {
                t[..idx].trim().to_string()
            };
            let after = t[idx + marker.len()..]
                .trim()
                .trim_end_matches(',')
                .trim()
                .to_string();
            return (before, Some(after));
        }
    }
    (t.to_string(), None)
}

// trace:exempt reason=internal-detail
fn parse_return(text: &str, ret_before: Option<&str>) -> Option<String> {
    let t = text
        .trim()
        .trim_start_matches(':')
        .trim()
        .trim_end_matches(';')
        .trim()
        .to_string();
    if t.is_empty() {
        return ret_before.map(|s| s.to_string());
    }
    for arrow in ["->", "=>"] {
        if let Some(idx) = find_depth0_pattern(&t, arrow) {
            let mut r = t[idx + arrow.len()..]
                .trim()
                .trim_end_matches(':')
                .trim()
                .trim_end_matches(';')
                .trim()
                .to_string();
            // unwrap multi-parenthesized return `(A, B)`
            if let Some((g, _)) = take_group(&r, '(', ')') {
                r = g;
            }
            if !r.is_empty() {
                // strip leading pointer/reference markers
                while let Some(stripped) = r.strip_prefix('*').or_else(|| r.strip_prefix('&')) {
                    r = stripped.trim().to_string();
                }
                return Some(r);
            }
            return ret_before.map(|s| s.to_string());
        }
    }
    if t.starts_with('(') {
        if let Some((g, _)) = take_group(&t, '(', ')') {
            return Some(g);
        }
    }
    if let Some(rb) = ret_before {
        return Some(rb.to_string());
    }
    if t.chars().all(|c| c.is_whitespace() || matches!(c, ':' | ';' | ',')) {
        return None;
    }
    // strip leading pointer/reference markers
    let mut r = t;
    while let Some(stripped) = r.strip_prefix('*').or_else(|| r.strip_prefix('&')) {
        r = stripped.trim().to_string();
    }
    if r.is_empty() { None } else { Some(r) }
}

// trace:exempt reason=internal-detail
fn parse_param(raw: &str) -> Option<SemanticParameter> {
    let mut s = raw.trim().to_string();
    if s.is_empty() {
        return None;
    }
    let variadic = s.contains("...") || s.starts_with('*') || s.starts_with("**");

    for pfx in ["&mut ", "&", "*", "mut ", "ref ", "..."] {
        if let Some(after) = s.strip_prefix(pfx) {
            s = after.trim().to_string();
            break;
        }
    }
    let head = leading_word(&s).map(|(w, _)| w).unwrap_or_default();
    if head == "self" || head == "this" || head == "Self" {
        return Some(SemanticParameter {
            name: head,
            ty: None,
            receiver: true,
            default: None,
            variadic,
        });
    }

    let (left, default) = match find_depth0_char(&s, '=') {
        Some(idx) => (s[..idx].trim().to_string(), Some(s[idx + 1..].trim().to_string())),
        None => (s, None),
    };
    if left.is_empty() {
        return None;
    }
    let (mut name, ty) = if let Some(idx) = find_depth0_char(&left, ':') {
        (left[..idx].trim().to_string(), Some(left[idx + 1..].trim().to_string()))
    } else {
        split_name_type(&left)
    };
    while let Some(stripped) = name.strip_prefix('&').or_else(|| name.strip_prefix('*')) {
        name = stripped.trim().to_string();
    }
    if let Some(stripped) = name.strip_prefix("mut ") {
        name = stripped.trim().to_string();
    }
    name = name.trim_end_matches('?').trim().to_string();
    if name.is_empty() {
        return None;
    }
    let receiver = name == "self" || name == "this";
    Some(SemanticParameter {
        name,
        ty,
        receiver,
        default,
        variadic,
    })
}

// trace:exempt reason=internal-detail
fn split_name_type(left: &str) -> (String, Option<String>) {
    let words: Vec<&str> = left.split_whitespace().collect();
    if words.is_empty() {
        return (String::new(), None);
    }
    if words.len() == 1 {
        return (words[0].to_string(), None);
    }
    let first = words[0];
    let last = words[words.len() - 1];
    let first_is_type = is_type_word(first)
        || first.chars().next().map(|c| !c.is_ascii_lowercase()).unwrap_or(false)
        || first.contains('<')
        || first.contains('[')
        || first.contains('.')
        || first.ends_with("[]");
    let last_is_plain = is_identifier(last) && !is_type_word(last);
    if first_is_type && last_is_plain {
        (last.to_string(), Some(words[..words.len() - 1].join(" ")))
    } else {
        (first.to_string(), Some(words[1..].join(" ")))
    }
}

// ---------------------------------------------------------------------------
// Visibility helpers
// ---------------------------------------------------------------------------

// trace:exempt reason=internal-detail
fn visibility_from_modifiers(mods: &[String]) -> Option<Visibility> {
    if mods.iter().any(|m| m == "pub" || m == "public") {
        Some(Visibility::Public)
    } else if mods.iter().any(|m| m == "protected") {
        Some(Visibility::Protected)
    } else if mods.iter().any(|m| m == "private") {
        Some(Visibility::Private)
    } else {
        None
    }
}

// trace:exempt reason=internal-detail
fn entry_visibility(
    exported: bool,
    parent: Option<&str>,
    mods: &[String],
    ret_before_name: Option<&str>,
) -> Visibility {
    if exported {
        return Visibility::Public;
    }
    if let Some(v) = visibility_from_modifiers(mods) {
        return v;
    }
    if parent.is_some() && ret_before_name.is_some() {
        return Visibility::Package;
    }
    Visibility::Private
}

// ---------------------------------------------------------------------------
// Kind mapping
// ---------------------------------------------------------------------------

// trace:exempt reason=internal-detail
fn map_kind(kind_str: &str, name: &str, parent: Option<&str>) -> SurfaceKind {
    match kind_str {
        "method" => {
            let simple = name.rsplit('.').next().unwrap_or(name);
            if parent.map(|p| simple == p).unwrap_or(false) || simple == "__init__" {
                SurfaceKind::Constructor
            } else {
                SurfaceKind::Method
            }
        }
        "function" => SurfaceKind::Function,
        "class" => SurfaceKind::Class,
        "interface" => SurfaceKind::Interface,
        "trait" => SurfaceKind::Trait,
        "enum" => SurfaceKind::Enum,
        "type" => SurfaceKind::Type,
        "const" => SurfaceKind::Const,
        "module" => SurfaceKind::Module,
        _ => SurfaceKind::Function,
    }
}

// ---------------------------------------------------------------------------
// Miscellaneous helpers
// ---------------------------------------------------------------------------

// trace:exempt reason=internal-detail
fn canonicalize(sig: &str) -> String {
    sig.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

// trace:exempt reason=internal-detail
fn synthesized_signature(kind_str: &str, simple: &str) -> String {
    match kind_str {
        "class" | "interface" | "trait" | "enum" | "module" => format!("{} {}", kind_str, simple),
        "type" => format!("type {}", simple),
        _ => simple.to_string(),
    }
}

// trace:exempt reason=internal-detail
fn step_matches(s: &scc_core::FlowStep, id: &str, name: &str, qualified: &str, file: &str) -> bool {
    s.actor == id || s.actor == name || s.actor == qualified || (!file.is_empty() && s.actor.contains(file))
}

// trace:exempt reason=internal-detail
fn sorted_rels(rels: Vec<&scc_core::Relationship>) -> Vec<&scc_core::Relationship> {
    let mut v = rels;
    v.sort_by(|a, b| {
        a.id.cmp(&b.id)
            .then_with(|| a.subject.cmp(&b.subject))
            .then_with(|| a.object.cmp(&b.object))
    });
    v
}

// trace:exempt reason=internal-detail
fn attr_str(e: &scc_core::Entity, key: &str) -> Option<String> {
    e.attributes.get(key).and_then(|v| match v {
        serde_json::Value::String(s) => Some(s.clone()),
        _ => None,
    })
}

// trace:exempt reason=internal-detail
fn attr_u32(e: &scc_core::Entity, key: &str) -> u32 {
    e.attributes
        .get(key)
        .and_then(|v| v.as_u64())
        .map(|n| n.min(u32::MAX as u64) as u32)
        .unwrap_or(0)
}

// trace:exempt reason=internal-detail
fn fits(total_chars: usize, budget_chars: Option<usize>) -> bool {
    budget_chars.is_none_or(|b| total_chars <= b)
}

// trace:exempt reason=internal-detail
fn entry_tier(e: &SurfaceEntry) -> u8 {
    if e.exported || e.visibility == Visibility::Public || !e.invocation_surfaces.is_empty() {
        0
    } else if e.rank.total > 0.0 {
        1
    } else {
        2
    }
}

// trace:exempt reason=internal-detail
fn entry_order(a: &SurfaceEntry, b: &SurfaceEntry) -> std::cmp::Ordering {
    entry_tier(a)
        .cmp(&entry_tier(b))
        .then_with(|| b.rank.total.partial_cmp(&a.rank.total).unwrap_or(std::cmp::Ordering::Equal))
        .then_with(|| a.kind.as_str().cmp(b.kind.as_str()))
        .then_with(|| a.qualified_name.cmp(&b.qualified_name))
        .then_with(|| a.id.cmp(&b.id))
}

// ---------------------------------------------------------------------------
// Renderer
// ---------------------------------------------------------------------------

// trace:exempt reason=internal-detail
fn render_entry(e: &SurfaceEntry) -> String {
    render_entry_opt(e, 0, None)
}

/// The structurally compressed entry block (hard-max overflow, never drops
/// the entry): kind + name + the first signature line only. The metadata
/// sections (Used by / Calls / Participates in / Contracts / Owns /
/// Invocation — the lines that carry annotation/modifier-rich detail) and
/// multi-line signature continuations / doc lines are dropped, so the
/// required set fits under the hard max.
// trace:exempt reason=internal-detail
fn render_entry_compressed(e: &SurfaceEntry) -> String {
    render_entry_opt(e, 1, None)
}

/// One entry block with the pipeline's render options. `level` is the
/// structural compression ladder: 0 = full entry, 1 = first signature
/// line only, 2 = canonical abbreviated signature, 3 = symbol identity
/// only (kind + name, always fits any realistic hard max). Levels >= 1
/// drop the metadata sections and the signature continuation/doc lines.
/// `rank` (explain mode) appends the entry's full score decomposition
/// (all eight components + total + reasons) instead of a bare importance.
// trace:exempt reason=internal-detail
fn render_entry_opt(e: &SurfaceEntry, level: u8, rank: Option<&SurfaceRank>) -> String {
    let mut out = String::new();
    let name = e.qualified_name.rsplit('.').next().unwrap_or(&e.qualified_name);
    out.push_str(&format!("  {} {}\n\n", e.kind.as_str(), name));
    if (1..3).contains(&level) {
        // level 2 uses the canonical (whitespace-normalized) signature,
        // truncated to a fixed width as an abbreviation; level 1 uses the
        // first source line; level 3 drops the signature entirely
        // (symbol identity only — always fits any realistic hard max).
        let sig = match level {
            2 => e.canonical_signature.split('\n').next().unwrap_or(""),
            _ => e.source_signature.split('\n').next().unwrap_or(""),
        };
        if !sig.is_empty() {
            let sig: String = sig.chars().take(160).collect();
            out.push_str("    ");
            out.push_str(&sig);
            out.push('\n');
        }
    } else if level == 0 {
        let sig_lines: Vec<&str> = e.source_signature.split('\n').collect();
        for line in &sig_lines {
            out.push_str("    ");
            out.push_str(line);
            out.push('\n');
        }
    }
    if level == 0 {
        let sections: [(&str, &[String]); 6] = [
            ("Used by", &e.callers),
            ("Calls", &e.callees),
            ("Participates in", &e.flows),
            ("Contracts", &e.contracts),
            ("Owns", &e.state_authorities),
            ("Invocation", &e.invocation_surfaces),
        ];
        for (label, vals) in sections {
            if vals.is_empty() {
                continue;
            }
            out.push('\n');
            out.push_str(&format!("  {label}:\n    {}\n", vals.join(", ")));
        }
    }
    if let Some(rank) = rank {
        out.push('\n');
        out.push_str(&format!("  importance: {:.3}\n", rank.total));
        out.push_str(&format!("  task_ppr: {:.3}\n", rank.task_ppr));
        out.push_str(&format!("  global_ppr: {:.3}\n", rank.global_ppr));
        out.push_str(&format!("  lexical: {:.3}\n", rank.lexical));
        out.push_str(&format!("  semantic: {:.3}\n", rank.semantic));
        out.push_str(&format!("  confidence: {:.3}\n", rank.confidence));
        out.push_str(&format!("  criticality: {:.3}\n", rank.criticality));
        out.push_str(&format!("  change_risk: {:.3}\n", rank.change_risk));
        out.push_str(&format!("  novelty: {:.3}\n", rank.novelty));
        if !rank.reasons.is_empty() {
            out.push_str("  because:\n");
            for r in &rank.reasons {
                out.push_str(&format!("    {r}\n"));
            }
        }
    }
    out.push('\n');
    out
}

// trace:exempt reason=internal-detail
fn group_header(comp: &str, sub: &str, path: &str) -> String {
    let mut h = String::new();
    h.push('\n');
    h.push_str(&comp.to_uppercase());
    h.push_str("\n\n");
    if sub.is_empty() {
        h.push_str(path);
    } else {
        h.push_str(&format!("{}  [{}]", path, sub));
    }
    h.push_str("\n\n");
    h
}

// ---------------------------------------------------------------------------
// Token-level helpers
// ---------------------------------------------------------------------------

// trace:exempt reason=internal-detail
fn is_modifier_word(w: &str) -> bool {
    matches!(
        w,
        "async" | "await" | "static" | "final" | "abstract" | "readonly"
            | "sealed" | "override" | "virtual" | "synchronized" | "native"
            | "extern" | "unsafe" | "inline" | "const" | "var" | "let" | "mutable"
            | "pub" | "public" | "private" | "protected" | "package" | "internal"
            | "open" | "suspend" | "operator" | "export" | "default" | "declare"
            | "data" | "value"
    )
}

// trace:exempt reason=internal-detail
fn is_callable_keyword(w: &str) -> bool {
    matches!(
        w,
        "fn" | "def" | "func" | "function" | "class" | "struct" | "interface"
            | "trait" | "enum" | "type" | "module"
    )
}

// trace:exempt reason=internal-detail
fn is_type_word(w: &str) -> bool {
    matches!(
        w,
        "int" | "long" | "short" | "byte" | "char" | "float" | "double"
            | "bool" | "boolean" | "string" | "str" | "void" | "unsigned"
            | "signed" | "usize" | "isize" | "u8" | "u16" | "u32" | "u64"
            | "u128" | "i8" | "i16" | "i32" | "i64" | "i128" | "any"
            | "object" | "Option" | "Result" | "Vec" | "Map" | "List" | "Set"
    )
}

// trace:exempt reason=internal-detail
fn is_identifier(w: &str) -> bool {
    let mut chars = w.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$' || c == '?')
}

// trace:exempt reason=internal-detail
fn leading_word(s: &str) -> Option<(String, &str)> {
    let s = s.trim_start();
    let mut end = 0;
    for (i, ch) in s.char_indices() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' || ch == '?' {
            end = i + ch.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        return None;
    }
    Some((s[..end].to_string(), &s[end..]))
}

// trace:exempt reason=internal-detail
fn take_group(text: &str, open: char, close: char) -> Option<(String, &str)> {
    if !text.starts_with(open) {
        return None;
    }
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    for (i, ch) in text.char_indices() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' | '`' => quote = Some(ch),
            c if c == open => depth += 1,
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    return Some((text[1..i].to_string(), &text[i + ch.len_utf8()..]));
                }
            }
            _ => {}
        }
    }
    None
}

// trace:exempt reason=internal-detail
fn find_matching_open(text: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0i32;
    for (i, ch) in text.char_indices().rev() {
        if ch == close {
            depth += 1;
        } else if ch == open {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

// trace:exempt reason=internal-detail
fn split_params(rest: &str) -> Option<(String, Vec<String>, String)> {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut open: Option<usize> = None;
    for (i, ch) in rest.char_indices() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' | '`' => quote = Some(ch),
            '(' if depth == 0 => {
                open = Some(i);
                break;
            }
            ')' if depth == 0 => return None,
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
    }
    let i0 = open?;
    let (inner, after) = take_group(&rest[i0..], '(', ')')?;
    let prefix = rest[..i0].to_string();
    let params = split_top(&inner, ',');
    Some((prefix, params, after.trim().to_string()))
}

// trace:exempt reason=internal-detail
fn split_top(text: &str, sep: char) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut paren_depth = 0i32;
    let mut angle_depth = 0i32;
    let mut quote: Option<char> = None;
    let mut prev: Option<char> = None;
    let mut cur = String::new();
    for ch in text.chars() {
        if let Some(q) = quote {
            cur.push(ch);
            if ch == q {
                quote = None;
            }
            prev = Some(ch);
            continue;
        }
        match ch {
            '"' | '\'' | '`' => {
                quote = Some(ch);
                cur.push(ch);
            }
            '(' | '[' | '{' => {
                paren_depth += 1;
                cur.push(ch);
            }
            ')' | ']' | '}' => {
                paren_depth -= 1;
                cur.push(ch);
            }
            '<' if angle_depth == 0 && prev.map(|p| p.is_alphanumeric() || p == '_' || p == '>').unwrap_or(false) => {
                angle_depth += 1;
                cur.push(ch);
            }
            '>' if angle_depth > 0 && prev != Some('-') => {
                angle_depth -= 1;
                cur.push(ch);
            }
            c if c == sep && paren_depth == 0 && angle_depth == 0 => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(ch),
        }
        prev = Some(ch);
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_string()).collect()
}

// trace:exempt reason=internal-detail
fn find_depth0_pattern(text: &str, pat: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    for (i, ch) in text.char_indices() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' | '`' => quote = Some(ch),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if depth == 0 && text[i..].starts_with(pat) {
            return Some(i);
        }
    }
    None
}

// trace:exempt reason=internal-detail
fn find_depth0_char(text: &str, target: char) -> Option<usize> {
    let mut paren_depth = 0i32;
    let mut angle_depth = 0i32;
    let mut quote: Option<char> = None;
    let mut prev: Option<char> = None;
    for (i, ch) in text.char_indices() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            prev = Some(ch);
            continue;
        }
        match ch {
            '"' | '\'' | '`' => quote = Some(ch),
            '(' | '[' | '{' => paren_depth += 1,
            ')' | ']' | '}' => paren_depth -= 1,
            '<' if angle_depth == 0 && prev.map(|p| p.is_alphanumeric() || p == '_' || p == '>').unwrap_or(false) => angle_depth += 1,
            '>' if angle_depth > 0 && prev != Some('-') => angle_depth -= 1,
            c if c == target && paren_depth == 0 && angle_depth == 0 => return Some(i),
            _ => {}
        }
        prev = Some(ch);
    }
    None
}

// trace:exempt reason=internal-detail
fn receiver_owner(group: &str) -> Option<String> {
    let inner = group.trim();
    if inner.is_empty() {
        return None;
    }
    let words: Vec<&str> = inner.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }
    let t = if words.len() <= 1 {
        words[0]
    } else {
        words[words.len() - 1]
    };
    let t = t.trim_start_matches('*').trim_start_matches('&');
    let seg = t.rsplit('.').next().unwrap_or(t);
    let seg = seg.trim().trim_start_matches('*');
    if seg.is_empty() {
        None
    } else {
        Some(seg.to_string())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use scc_core::{
        entity_id, relationship_id, symbol_id, ContextLedger, Entity, Flow, FlowKind,
        FlowStep, Relationship,
    };
    use scc_store::Store;

// trace:exempt reason=internal-detail
    fn fixture_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let repo = store.repo_id.clone();
        let path = "api/app.py";

        let fid = entity_id(&repo, kinds::FILE, path);
        store.insert_entity(&Entity::new(fid.clone(), kinds::FILE, path), &[path.to_string()]).unwrap();

        let comp_id = entity_id(&repo, kinds::COMPONENT, "api");
        store.replace_components(&[Entity::new(comp_id.clone(), kinds::COMPONENT, "api")]).unwrap();
        store.insert_relationship(
            &Relationship::new(relationship_id(1), comp_id.clone(), predicates::CONTAINS, fid.clone(), Provenance::Extracted),
            path,
        ).unwrap();

        let svc_id = entity_id(&repo, kinds::SERVICE, "core");
        store.insert_entity(&Entity::new(svc_id.clone(), kinds::SERVICE, "core"), &[path.to_string()]).unwrap();
        store.insert_relationship(
            &Relationship::new(relationship_id(2), svc_id, predicates::CONTAINS, comp_id.clone(), Provenance::Extracted),
            path,
        ).unwrap();

        let mut rid: u64 = 3;
        let mut sym = |name: &str, kind: &str, sig: Option<&str>, exported: bool, parent: Option<&str>| -> String {
            let id = symbol_id(&repo, path, name);
            let mut e = Entity::new(id.clone(), kinds::SYMBOL, name);
            e.attr("kind", serde_json::json!(kind));
            e.attr("file", serde_json::json!(path));
            if let Some(s) = sig { e.attr("signature", serde_json::json!(s)); }
            e.attr("exported", serde_json::json!(exported));
            e.attr("start_line", serde_json::json!(1u32));
            e.attr("end_line", serde_json::json!(10u32));
            if let Some(p) = parent { e.attr("parent", serde_json::json!(p)); }
            store.insert_entity(&e, &[path.to_string()]).unwrap();
            rid += 1;
            store.insert_relationship(
                &Relationship::new(relationship_id(rid), fid.clone(), predicates::CONTAINS, id.clone(), Provenance::Extracted),
                path,
            ).unwrap();
            id
        };

        let _ = sym("UserService", "class", None, true, None);
        let _ = sym("UserService.UserService", "method", Some("public UserService(String name)"), false, Some("UserService"));
        let get_id = sym("UserService.get", "method", Some("public User get(String id) throws NotFound"), false, Some("UserService"));
        let _ = sym("UserService.hash", "method", Some("String hash()"), false, Some("UserService"));
        let update_id = sym("UserService.update", "method", Some("async fn update(&mut self, patch: Json) -> bool"), false, Some("UserService"));
        let create_id = sym("create_user", "function", Some("def create_user(name: str, age: int = 0) -> User"), true, None);
        let db_id = sym("db", "function", Some("func db() *DB"), false, None);

        // annotation RestController ANNOTATES get
        let ann_id = entity_id(&repo, kinds::ANNOTATION, "RestController");
        store.insert_entity(&Entity::new(ann_id.clone(), kinds::ANNOTATION, "RestController"), &[path.to_string()]).unwrap();
        rid += 1;
        store.insert_relationship(
            &Relationship::new(relationship_id(rid), ann_id, predicates::ANNOTATES, get_id.clone(), Provenance::Extracted),
            path,
        ).unwrap();

        // route: GET /api/users -> create_user
        let route_id = entity_id(&repo, kinds::ROUTE, "GET /api/users");
        let mut re = Entity::new(route_id.clone(), kinds::ROUTE, "GET /api/users");
        re.attr("method", serde_json::json!("GET"));
        re.attr("path", serde_json::json!("/api/users"));
        re.attr("handler", serde_json::json!(create_id.clone()));
        store.insert_entity(&re, &[path.to_string()]).unwrap();

        // CALLS
        rid += 1;
        store.insert_relationship(
            &Relationship::new(relationship_id(rid), create_id.clone(), predicates::CALLS, get_id.clone(), Provenance::Extracted),
            path,
        ).unwrap();
        rid += 1;
        store.insert_relationship(
            &Relationship::new(relationship_id(rid), db_id.clone(), predicates::CALLS, create_id.clone(), Provenance::Resolved),
            path,
        ).unwrap();

        // STATE: db OWNS sessions
        let state_id = entity_id(&repo, kinds::STATE, "sessions");
        store.insert_entity(&Entity::new(state_id.clone(), kinds::STATE, "sessions"), &[path.to_string()]).unwrap();
        rid += 1;
        store.insert_relationship(
            &Relationship::new(relationship_id(rid), db_id.clone(), predicates::OWNS, state_id, Provenance::Extracted),
            path,
        ).unwrap();

        // REACTIVE: update OWNS cursor
        let rx_id = entity_id(&repo, kinds::REACTIVE, "cursor");
        store.insert_entity(&Entity::new(rx_id.clone(), kinds::REACTIVE, "cursor"), &[path.to_string()]).unwrap();
        rid += 1;
        store.insert_relationship(
            &Relationship::new(relationship_id(rid), update_id.clone(), predicates::OWNS, rx_id, Provenance::Extracted),
            path,
        ).unwrap();

        // Flow signup
        store.replace_flows(&[Flow {
            id: entity_id(&repo, kinds::FLOW, "signup"),
            kind: FlowKind::Workflow,
            name: "signup".into(),
            trigger: Some("http".into()),
            steps: vec![
                FlowStep {
                    id: "step:1".into(),
                    order: 1,
                    actor: get_id.clone(),
                    operation: "load user".into(),
                    condition: None,
                    r#async: None,
                    timeout_ms: None,
                    retry_policy: None,
                    failure_outcome: None,
                    provenance: Some(Provenance::Extracted),
                    evidence: vec![],
                },
                FlowStep {
                    id: "step:2".into(),
                    order: 2,
                    actor: "db".into(),
                    operation: "persist".into(),
                    condition: None,
                    r#async: None,
                    timeout_ms: None,
                    retry_policy: None,
                    failure_outcome: None,
                    provenance: Some(Provenance::Extracted),
                    evidence: vec![],
                },
            ],
            attributes: std::collections::BTreeMap::new(),
        }]).unwrap();

        let _ = (get_id, create_id, db_id, update_id);
        (dir, store)
    }

// trace:exempt reason=internal-detail
    fn entry<'a>(map: &'a SystemSurfaceMap, qn: &str) -> &'a SurfaceEntry {
        map.entries.iter().find(|e| e.qualified_name == qn).expect("entry not found")
    }

// trace:exempt reason=internal-detail
    fn make_ctx<'a>(store: &'a Store) -> ContextCompiler<'a> {
        let graph = Box::leak(Box::new(scc_graph::RealityGraph::load(store).unwrap()));
        ContextCompiler::new(store, graph, crate::ContextSettings::default(), Vec::new())
    }

    /// 40 private functions: 20 lexically matched by the goal term
    /// (`zeta_*` in `a/mod.py`) + 20 unmatched (`alpha_*` in `b/mod.py`),
    /// so MMR/path diversity and lexical vs PPR blending are observable.
// trace:exempt reason=internal-detail
    fn zeta_alpha_fixture() -> (tempfile::TempDir, Store) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let repo = store.repo_id.clone();
        for (path, prefix) in [("a/mod.py", "zeta"), ("b/mod.py", "alpha")] {
            for i in 0..20 {
                let name = format!("{prefix}_{i:02}");
                let id = symbol_id(&repo, path, &name);
                let mut e = Entity::new(id.clone(), kinds::SYMBOL, name);
                e.attr("kind", serde_json::json!("function"));
                e.attr("file", serde_json::json!(path));
                e.attr("signature", serde_json::json!("def f(x): ..."));
                e.attr("exported", serde_json::json!(false));
                e.attr("start_line", serde_json::json!(1u32));
                e.attr("end_line", serde_json::json!(10u32));
                store.insert_entity(&e, &[path.to_string()]).unwrap();
            }
        }
        (dir, store)
    }

    /// One required entry (exported → public-api invocation surface) with a
    /// 400-line declaration header — its full render alone exceeds any
    /// realistic hard max, so compression is observable.
// trace:exempt reason=internal-detail
    fn huge_signature_fixture() -> (tempfile::TempDir, Store) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let repo = store.repo_id.clone();
        let path = "api/app.py";
        let id = symbol_id(&repo, path, "big_fn");
        let mut e = Entity::new(id.clone(), kinds::SYMBOL, "big_fn".to_string());
        e.attr("kind", serde_json::json!("function"));
        e.attr("file", serde_json::json!(path));
        let mut decl = String::from("def big_fn(\n");
        for i in 0..400 {
            decl.push_str(&format!("    arg_{i:03}: str,\n"));
        }
        decl.push_str(") -> None:\n");
        e.attr("decl_header", serde_json::json!(decl));
        e.attr("exported", serde_json::json!(true));
        e.attr("start_line", serde_json::json!(1u32));
        e.attr("end_line", serde_json::json!(10u32));
        // A concrete invocation surface (http route handler) makes this a
        // CRITICAL coverage entry — the compression path under hard-max
        // overflow is what the test exercises. A plain public export is
        // not critical by design (the ranker decides its fate).
        e.attr("entrypoints", serde_json::json!(["http: POST /big"]));
        store.insert_entity(&e, &[path.to_string()]).unwrap();
        (dir, store)
    }

    #[test]
// trace:exempt reason=internal-detail
    fn compiles_surface_map_with_attribution() {
        let (_dir, store) = fixture_store();
        let ctx = make_ctx(&store);
        let map = compile_surface_map(&ctx);

        assert_eq!(map.entries.len(), 7);
        assert_eq!(map.repository, "repo");
        assert_eq!(map.revision, "not-indexed");
        assert!(map.token_count > 0);

        let cls = entry(&map, "UserService");
        assert_eq!(cls.kind, SurfaceKind::Class);
        assert_eq!(cls.visibility, Visibility::Public);
        assert!(cls.exported);
        assert_eq!(cls.source_signature, "class UserService");
        assert_eq!(cls.range.path, "api/app.py");
        assert_eq!(cls.component.as_deref(), Some("api"));
        assert_eq!(cls.subsystem.as_deref(), Some("core"));

        let ctor = entry(&map, "UserService.UserService");
        assert_eq!(ctor.kind, SurfaceKind::Constructor);
        assert_eq!(ctor.visibility, Visibility::Public);
        assert_eq!(ctor.semantic_signature.parameters[0].name, "name");
        assert_eq!(ctor.semantic_signature.parameters[0].ty.as_deref(), Some("String"));

        let get = entry(&map, "UserService.get");
        assert_eq!(get.kind, SurfaceKind::Method);
        assert_eq!(get.visibility, Visibility::Public);
        assert_eq!(get.annotations, vec!["RestController"]);
        assert_eq!(get.flows, vec!["signup"]);
        assert_eq!(get.callers, vec!["create_user"]);
        assert!(get.callees.is_empty());
        let gs = &get.semantic_signature;
        assert_eq!(gs.parameters[0].name, "id");
        assert_eq!(gs.parameters[0].ty.as_deref(), Some("String"));
        assert!(!gs.parameters[0].receiver);
        assert_eq!(gs.returns.as_deref(), Some("User"));
        assert!(gs.constraints.iter().any(|c| c == "NotFound"));
        assert_eq!(get.provenance, Provenance::Extracted);
        assert_eq!(get.confidence, 0.85);

        let hash = entry(&map, "UserService.hash");
        assert_eq!(hash.visibility, Visibility::Package);

        let update = entry(&map, "UserService.update");
        assert_eq!(update.visibility, Visibility::Private);
        assert_eq!(update.modifiers, vec!["async"]);
        assert_eq!(update.state_authorities, vec!["cursor"]);
        let us = &update.semantic_signature;
        assert!(us.async_);
        assert!(us.parameters[0].receiver);
        assert_eq!(us.parameters[1].name, "patch");
        assert_eq!(us.returns.as_deref(), Some("bool"));

        let create = entry(&map, "create_user");
        assert_eq!(create.kind, SurfaceKind::Function);
        assert_eq!(create.visibility, Visibility::Public);
        assert!(create.exported);
        assert_eq!(create.canonical_signature, "def create_user(name: str, age: int = 0) -> user");
        assert_eq!(create.contracts, vec!["http: GET /api/users"]);
        assert_eq!(create.callees, vec!["UserService.get"]);
        assert_eq!(create.invocation_surfaces, vec![
            "http: GET /api/users",
            "public_api: export:create_user (function)",
        ]);
        assert_eq!(create.semantic_signature.parameters[1].name, "age");
        assert_eq!(create.semantic_signature.parameters[1].default.as_deref(), Some("0"));
        assert_eq!(create.semantic_signature.returns.as_deref(), Some("User"));

        let db = entry(&map, "db");
        assert_eq!(db.visibility, Visibility::Private);
        assert_eq!(db.state_authorities, vec!["sessions"]);
        assert_eq!(db.provenance, Provenance::Resolved);
        assert_eq!(db.confidence, 1.0);
        assert_eq!(db.semantic_signature.returns.as_deref(), Some("DB"));
        assert_eq!(db.flows, vec!["signup"]);
    }

    #[test]
// trace:exempt reason=internal-detail
    fn renderer_groups_and_budget_cuts() {
        let (_dir, store) = fixture_store();
        let ctx = make_ctx(&store);
        let map = compile_surface_map(&ctx);

        let full = render_surface_map(&map, None);
        assert!(full.starts_with("SCC SYSTEM SURFACE MAP\n\n"));
        assert!(full.contains("API\n\napi/app.py  [core]"));
        assert!(full.contains("  class UserService\n"));
        assert!(full.contains("  function create_user\n"));
        assert!(full.contains("  constructor UserService\n"));
        assert!(full.contains("Used by:"));
        assert!(full.contains("http: GET /api/users"));
        assert!(!full.contains("OMITTED"));
        assert_eq!(map.token_count, estimate_tokens(&full));

        let tiny = render_surface_map(&map, Some(1));
        assert!(tiny.contains("OMITTED (token budget exceeded):"));
        assert!(tiny.contains("3 lower-ranked method definitions"));
        assert!(tiny.contains("1 lower-ranked class definitions"));
        assert!(tiny.contains("1 lower-ranked constructor definitions"));
        assert!(tiny.contains("2 lower-ranked function definitions"));
        assert!(!tiny.contains("  class UserService\n"));
    }

    #[test]
// trace:exempt reason=internal-detail
    fn semantic_parser_never_panics() {
        for sig in &["", "   ", "(", ")", "()", "=>", "->", "(((", "fn", "public", "= 42"] {
            let s = parse_signature(sig, "fallback", None);
            assert_eq!(s.name, "fallback");
        }
    }

    #[test]
// trace:exempt reason=internal-detail
    fn semantic_parser_language_tolerant() {
        // Rust with generics, where clause, receiver
        let s = parse_signature(
            "pub async fn render<T: Bound>(&self, x: T) -> String where T: Clone + Send",
            "render",
            Some("Widget"),
        );
        assert_eq!(s.name, "render");
        assert!(s.async_);
        assert!(s.generic_parameters.contains(&"T".to_string()));
        assert!(s.constraints.contains(&"T: Bound".to_string()));
        assert!(s.constraints.contains(&"T: Clone + Send".to_string()));
        assert_eq!(s.visibility, Some(Visibility::Public));
        assert_eq!(s.returns.as_deref(), Some("String"));
        assert!(s.parameters[0].receiver);
        assert_eq!(s.parameters[1].ty.as_deref(), Some("T"));
        assert_eq!(s.owner.as_deref(), Some("Widget"));

        // Go receiver + multi-return
        let g = parse_signature(
            "func (s *Store) Get(ctx context.Context) (User, error)",
            "Get",
            Some("Store"),
        );
        assert_eq!(g.name, "Get");
        assert_eq!(g.owner.as_deref(), Some("Store"));
        assert_eq!(g.parameters[0].ty.as_deref(), Some("context.Context"));
        assert_eq!(g.returns.as_deref(), Some("User, error"));
    }

    // ---- production selection pipeline ----

    #[test]
// trace:exempt reason=internal-detail
    fn global_render_selects_within_budget_and_reports_omissions() {
        let (_dir, store) = fixture_store();
        let ctx = make_ctx(&store);
        let map = compile_surface_map(&ctx);
        let n = map.entries.len();
        assert!(n >= 5);

        // Huge budget: everything renders; nothing omitted.
        let big = select_and_render_global(&ctx, 100_000);
        assert_eq!(big.rendered_ids.len(), n);
        assert!(big.omitted_ids.is_empty());
        assert!(big.omissions.is_empty());
        assert!(big.text.starts_with("SCC SYSTEM SURFACE MAP"));
        assert!(big.token_count > 0);

        // Tiny budget: required entries survive; the rest are honestly
        // omitted with per-kind summaries.
        let tiny = select_and_render_global(&ctx, 1);
        assert!(
            !tiny.rendered_ids.is_empty(),
            "required (invocation-surface) entries must survive a tiny budget"
        );
        for id in &tiny.rendered_ids {
            assert!(map.entries.iter().any(|e| &e.id == id), "{id} must be a candidate");
        }
        // rendered + omitted == all candidates, disjoint
        assert_eq!(tiny.rendered_ids.len() + tiny.omitted_ids.len(), n);
        for id in &tiny.omitted_ids {
            assert!(!tiny.rendered_ids.contains(id), "{id} both rendered and omitted");
        }
        let omitted_total: usize = tiny.omissions.iter().map(|o| o.count).sum();
        assert_eq!(omitted_total, tiny.omitted_ids.len());

        // Deterministic: same input → byte-identical text and ids.
        let tiny2 = select_and_render_global(&ctx, 1);
        assert_eq!(tiny2.text, tiny.text);
        assert_eq!(tiny2.rendered_ids, tiny.rendered_ids);
        assert_eq!(tiny2.omitted_ids, tiny.omitted_ids);
    }

    #[test]
// trace:exempt reason=internal-detail
    fn task_render_skips_visible_unchanged_entries() {
        let (_dir, store) = fixture_store();
        let ctx = make_ctx(&store);
        let map = compile_surface_map(&ctx);
        let n = map.entries.len();

        let create = entry(&map, "create_user");
        let mut visible = ContextLedger::default();
        visible.visible_symbols.insert(create.symbol_id.clone());

        let out = select_and_render_task(&ctx, "create user", 100_000, &visible);
        // The already-visible-and-unchanged entry is not re-injected.
        assert!(
            !out.rendered_ids.contains(&create.id),
            "visible unchanged entries must not be re-injected"
        );
        // Everything else (novel) renders under a huge budget.
        assert_eq!(out.rendered_ids.len(), n - 1);
        assert_eq!(out.omitted_ids.len(), 1);
        assert!(out.omitted_ids.contains(&create.id));
        assert!(out.text.starts_with("SCC SYSTEM SURFACE MAP"));
    }

    #[test]
// trace:exempt reason=internal-detail
    fn task_render_never_omits_required_even_at_zero_budget() {
        let (_dir, store) = fixture_store();
        let ctx = make_ctx(&store);
        let out = select_and_render_task(&ctx, "user", 0, &ContextLedger::default());
        // required (invocation-surface) entries survive a zero budget;
        // the rest are omitted honestly.
        assert!(!out.rendered_ids.is_empty());
        assert!(!out.omissions.is_empty() || out.omitted_ids.is_empty());
        let rendered: BTreeSet<String> = out.rendered_ids.iter().cloned().collect();
        for id in &out.omitted_ids {
            assert!(!rendered.contains(id));
        }
    }

    // ---- Wave 15.1: the one authoritative surface service ----

    #[test]
// trace:exempt reason=internal-detail
    fn build_surface_global_matches_historical_pipeline() {
        let (_dir, store) = fixture_store();
        let ctx = make_ctx(&store);
        let map = compile_surface_map(&ctx);
        let n = map.entries.len();
        assert!(n >= 5);

        let req = |budget: usize| SurfaceRequest {
            mode: SurfaceMode::Global,
            budget,
            explain: false,
            policy: SurfacePolicy::defaults(budget),
                    semantic: None,
        };

        // Huge budget: the service renders exactly the historical global
        // pipeline output — every candidate, byte-identical to the full
        // map render (the old select_and_render_global invariant).
        let big = build_surface(&ctx, req(100_000));
        let legacy = select_and_render_global(&ctx, 100_000);
        assert_eq!(big.text, legacy.text);
        assert_eq!(big.rendered_ids, legacy.rendered_ids);
        assert_eq!(big.omitted_ids, legacy.omitted_ids);
        // Selection parity with the full map: every candidate renders under
        // a huge budget (omissions empty). Wave 15.2 populates per-entry
        // SurfaceRanks, so the selected render orders each group by
        // importance while the plain map render keeps the canonical
        // kind/name order — both carry every entry (same id set).
        let mut big_ids = big.rendered_ids.clone();
        big_ids.sort();
        let mut map_ids: Vec<String> = map.entries.iter().map(|e| e.id.clone()).collect();
        map_ids.sort();
        assert_eq!(big_ids, map_ids, "the selected subset must cover every candidate");
        assert_eq!(big.rendered_ids.len(), n);
        assert!(big.omitted_ids.is_empty());

        // Tiny budget: required coverage survives; byte-identical across
        // runs (deterministic).
        let tiny = build_surface(&ctx, req(1));
        assert!(!tiny.rendered_ids.is_empty());
        assert_eq!(tiny.rendered_ids.len() + tiny.omitted_ids.len(), n);
        let tiny2 = build_surface(&ctx, req(1));
        assert_eq!(tiny.text, tiny2.text);
        assert_eq!(tiny.rendered_ids, tiny2.rendered_ids);
    }

    #[test]
// trace:exempt reason=internal-detail
    fn build_surface_task_applies_novelty_suppression() {
        let (_dir, store) = fixture_store();
        let ctx = make_ctx(&store);
        let map = compile_surface_map(&ctx);
        let create = entry(&map, "create_user");

        let mut visible = ContextLedger::default();
        visible.visible_symbols.insert(create.symbol_id.clone());
        let out = build_surface(
            &ctx,
            SurfaceRequest {
                mode: SurfaceMode::Task {
                    goal: "create user",
                    visible: Some(&visible),
                },
                budget: 100_000,
                explain: false,
                policy: SurfacePolicy::defaults(100_000),
                        semantic: None,
        },
        );
        // The already-visible-and-unchanged entry is not re-injected; the
        // rest render (huge budget); omitted ids are honest.
        assert!(!out.rendered_ids.contains(&create.id));
        assert_eq!(out.rendered_ids.len(), map.entries.len() - 1);
        assert!(out.omitted_ids.contains(&create.id));
        // Routing parity with the historical task entry point.
        let legacy = select_and_render_task(&ctx, "create user", 100_000, &visible);
        assert_eq!(out.text, legacy.text);
        assert_eq!(out.rendered_ids, legacy.rendered_ids);

        // No ledger → the full task-personalized map (nothing suppressed).
        let full = build_surface(
            &ctx,
            SurfaceRequest {
                mode: SurfaceMode::Task {
                    goal: "create user",
                    visible: None,
                },
                budget: 100_000,
                explain: false,
                policy: SurfacePolicy::defaults(100_000),
                        semantic: None,
        },
        );
        assert_eq!(full.rendered_ids.len(), map.entries.len());
    }

    #[test]
// trace:exempt reason=internal-detail
    fn token_aware_quotas_cap_the_dominant_kind() {
        // 1000 private functions: 900 plain "core" + 100 state owners. The
        // naive value/token pick fills the budget with the dominant kind;
        // token-aware quotas cap it at its share of the available TOKENS
        // and let the under-represented state kind in.
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let repo = store.repo_id.clone();
        let path = "api/app.py";
        let mut rel_id: u64 = 10_000;
        for i in 0..900 {
            let name = format!("core_fn_{i:04}");
            let id = symbol_id(&repo, path, &name);
            let mut e = Entity::new(id.clone(), kinds::SYMBOL, name);
            e.attr("kind", serde_json::json!("function"));
            e.attr("file", serde_json::json!(path));
            e.attr("signature", serde_json::json!("def f(x): ..."));
            e.attr("exported", serde_json::json!(false));
            e.attr("start_line", serde_json::json!(1u32));
            e.attr("end_line", serde_json::json!(10u32));
            store.insert_entity(&e, &[path.to_string()]).unwrap();
        }
        for i in 0..100 {
            let name = format!("state_fn_{i:04}");
            let id = symbol_id(&repo, path, &name);
            let mut e = Entity::new(id.clone(), kinds::SYMBOL, name);
            e.attr("kind", serde_json::json!("function"));
            e.attr("file", serde_json::json!(path));
            e.attr("signature", serde_json::json!("def f(x): ..."));
            e.attr("exported", serde_json::json!(false));
            e.attr("start_line", serde_json::json!(1u32));
            e.attr("end_line", serde_json::json!(10u32));
            store.insert_entity(&e, &[path.to_string()]).unwrap();
            let state_id = entity_id(&repo, kinds::STATE, &format!("st{i:04}"));
            store
                .insert_entity(
                    &Entity::new(state_id.clone(), kinds::STATE, format!("st{i:04}")),
                    &[path.to_string()],
                )
                .unwrap();
            rel_id += 1;
            store
                .insert_relationship(
                    &Relationship::new(
                        relationship_id(rel_id),
                        id,
                        predicates::OWNS,
                        state_id,
                        Provenance::Extracted,
                    ),
                    path,
                )
                .unwrap();
        }
        let ctx = make_ctx(&store);
        let map = compile_surface_map(&ctx);
        assert_eq!(map.entries.len(), 1000);

        let budget = 2000usize;
        let hard_max = SurfacePolicy::defaults(budget).hard_max;
        let req = |quotas: bool| SurfaceRequest {
            mode: SurfaceMode::Global,
            budget,
            explain: false,
            policy: SurfacePolicy {
                quotas,
                mmr: false,
                coverage: true,
                hard_max,
            },
                    semantic: None,
        };
        let naive = build_surface(&ctx, req(false));
        let balanced = build_surface(&ctx, req(true));
        let count = |r: &scc_core::SurfaceRenderResult, prefix: &str| {
            r.rendered_ids.iter().filter(|id| id.contains(prefix)).count()
        };
        let naive_core = count(&naive, "core_fn_");
        let bal_core = count(&balanced, "core_fn_");
        let bal_state = count(&balanced, "state_fn_");
        assert!(
            bal_core < naive_core,
            "quotas must cut the dominant kind: {bal_core} !< {naive_core}"
        );
        assert!(bal_state > 0, "quotas must admit the under-represented kind");
    }

    #[test]
// trace:exempt reason=internal-detail
    fn hard_max_soft_overflow_renders_but_required_overflow_compresses() {
        // Soft overflow: required entries exceed the budget but stay under
        // the hard max → they render in FULL (metadata sections intact,
        // entries never dropped).
        let (_dir, store) = fixture_store();
        let ctx = make_ctx(&store);
        let soft = build_surface(
            &ctx,
            SurfaceRequest {
                mode: SurfaceMode::Global,
                budget: 5,
                explain: false,
                policy: SurfacePolicy::defaults(5), // hard_max = 505
                        semantic: None,
        },
        );
        assert!(!soft.rendered_ids.is_empty());
        assert!(
            soft.text.contains("Used by:"),
            "under the hard max the full entry blocks must render"
        );
        assert!(soft.token_count > 5, "required may exceed the soft budget");
        assert!(soft.token_count <= 505, "required never exceeds the hard max");

        // Hard overflow: the required entry alone exceeds the hard max →
        // structurally compressed: the entry stays, metadata/annotation
        // lines are dropped.
        let (_dir2, store2) = huge_signature_fixture();
        let ctx2 = make_ctx(&store2);
        let hard = build_surface(
            &ctx2,
            SurfaceRequest {
                mode: SurfaceMode::Global,
                budget: 100,
                explain: false,
                policy: SurfacePolicy::defaults(100), // hard_max = 600
                        semantic: None,
        },
        );
        assert_eq!(hard.rendered_ids.len(), 1, "the required entry is never dropped");
        assert!(
            hard.text.contains("function big_fn"),
            "the compressed entry keeps its identity"
        );
        assert!(!hard.text.contains("Used by:"), "metadata sections are dropped");
        assert!(hard.token_count <= 600, "the compressed render fits the hard max");
    }

    #[test]
// trace:exempt reason=internal-detail
    fn staged_toggles_change_output_deterministically() {
        let (_dir, store) = zeta_alpha_fixture();
        let ctx = make_ctx(&store);

        // Tight budget: MMR on vs off changes the selection.
        let budget = 80usize;
        let hard_max = SurfacePolicy::defaults(budget).hard_max;
        let req = |_stages: &SurfacePipelineStages| SurfaceRequest {
            mode: SurfaceMode::Task {
                goal: "zeta",
                visible: None,
            },
            budget,
            explain: false,
            policy: SurfacePolicy {
                quotas: false,
                mmr: true,
                coverage: true,
                hard_max,
            },
                    semantic: None,
        };
        let render =
            |stages: &SurfacePipelineStages| build_surface_staged(&ctx, req(stages), stages);
        let has_b = |r: &scc_core::SurfaceRenderResult| {
            r.rendered_ids.iter().any(|id| id.contains("b/mod.py"))
        };

        let full = render(&SurfacePipelineStages::default());
        // Determinism: identical stages → byte-identical output.
        let full2 = render(&SurfacePipelineStages::default());
        assert_eq!(full.text, full2.text);
        assert_eq!(full.rendered_ids, full2.rendered_ids);

        // MMR off: the rank-order cut stays on the first path. MMR on:
        // diversity pulls the second path in.
        let no_mmr_stages = SurfacePipelineStages {
            mmr: false,
            ..SurfacePipelineStages::default()
        };
        let no_mmr = render(&no_mmr_stages);
        assert_ne!(full.text, no_mmr.text);
        assert!(has_b(&full), "MMR must diversify across paths");
        assert!(!has_b(&no_mmr), "rank-order cut must stay on the first path");

        // Lexical stage off (skip PPR entirely): only lexically matched
        // entries rank; the PPR blend also admits low-lexical entries.
        let no_lex_stages = SurfacePipelineStages {
            lexical: false,
            ..SurfacePipelineStages::default()
        };
        let no_lex = build_surface_staged(
            &ctx,
            SurfaceRequest {
                mode: SurfaceMode::Task {
                    goal: "zeta",
                    visible: None,
                },
                budget: 100_000,
                explain: false,
                policy: SurfacePolicy::defaults(100_000),
                        semantic: None,
        },
            &no_lex_stages,
        );
        let lex_on = build_surface_staged(
            &ctx,
            SurfaceRequest {
                mode: SurfaceMode::Task {
                    goal: "zeta",
                    visible: None,
                },
                budget: 100_000,
                explain: false,
                policy: SurfacePolicy::defaults(100_000),
                        semantic: None,
        },
            &SurfacePipelineStages::default(),
        );
        assert_eq!(
            no_lex.rendered_ids.len(),
            20,
            "pure lexical skips unmatched entries"
        );
        assert_eq!(lex_on.rendered_ids.len(), 40, "the PPR blend ranks the full pool");
        assert_ne!(no_lex.text, lex_on.text);
    }

    // ---- overload-sensitive entries + decl_header ----

    #[test]
// trace:exempt reason=internal-detail
    fn overload_entries_get_distinct_ids_and_logical_symbol_id() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let repo = store.repo_id.clone();
        let path = "api/app.py";
        let base = symbol_id(&repo, path, "handle");
        for (n, id) in [(0u64, base.clone()), (1, format!("{}#1", base))] {
            let mut e = Entity::new(id.clone(), kinds::SYMBOL, "handle");
            e.attr("kind", serde_json::json!("function"));
            e.attr("file", serde_json::json!(path));
            e.attr("signature", serde_json::json!("def handle(x): ..."));
            e.attr("exported", serde_json::json!(true));
            e.attr("start_line", serde_json::json!(1u32));
            e.attr("end_line", serde_json::json!(10u32));
            e.attr("overload_index", serde_json::json!(n));
            store.insert_entity(&e, &[path.to_string()]).unwrap();
        }
        let ctx = make_ctx(&store);
        let map = compile_surface_map(&ctx);
        let handles: Vec<&SurfaceEntry> = map
            .entries
            .iter()
            .filter(|e| e.qualified_name == "handle")
            .collect();
        assert_eq!(handles.len(), 2, "same-name overloads stay separate entries");
        let mut ids: Vec<String> = handles.iter().map(|e| e.id.clone()).collect();
        ids.sort();
        assert_eq!(
            ids,
            vec![format!("{}#overload0", base), format!("{}#overload1", base)]
        );
        for e in handles {
            assert_eq!(e.symbol_id, base, "symbol_id stays the logical symbol");
        }
    }

    #[test]
// trace:exempt reason=internal-detail
    fn decl_header_wins_over_legacy_signature() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let repo = store.repo_id.clone();
        let path = "api/app.py";
        let id = symbol_id(&repo, path, "process");
        let mut e = Entity::new(id.clone(), kinds::SYMBOL, "process");
        e.attr("kind", serde_json::json!("function"));
        e.attr("file", serde_json::json!(path));
        e.attr("signature", serde_json::json!("def process(x) -> str"));
        e.attr(
            "decl_header",
            serde_json::json!("async def process(\n    x: int,\n) -> str:"),
        );
        e.attr("exported", serde_json::json!(true));
        e.attr("start_line", serde_json::json!(1u32));
        e.attr("end_line", serde_json::json!(10u32));
        store.insert_entity(&e, &[path.to_string()]).unwrap();
        let ctx = make_ctx(&store);
        let map = compile_surface_map(&ctx);
        let proc = entry(&map, "process");
        assert_eq!(
            proc.source_signature,
            "async def process(\n    x: int,\n) -> str:",
            "decl_header (exact header) must win over the legacy signature attr"
        );
        assert_eq!(proc.modifiers, vec!["async"]);
    }

    // ---- Wave 15.2: live semantic 10%, explain decomposition, hard-max ----

    /// Two symmetric private functions (`alpha_fn`, `beta_fn`) in one file
    /// with identical signatures and no graph edges — the base blend is
    /// identical for both, so the semantic scorer is the ONLY differentiator
    /// and the redistribution rule is observable.
// trace:exempt reason=unit-test-fixture
    fn symmetric_two_fn_fixture() -> (tempfile::TempDir, Store) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let repo = store.repo_id.clone();
        let path = "api/app.py";
        for name in ["alpha_fn", "beta_fn"] {
            let id = symbol_id(&repo, path, name);
            let mut e = Entity::new(id.clone(), kinds::SYMBOL, name.to_string());
            e.attr("kind", serde_json::json!("function"));
            e.attr("file", serde_json::json!(path));
            e.attr("signature", serde_json::json!("def f(x): ..."));
            e.attr("exported", serde_json::json!(false));
            e.attr("start_line", serde_json::json!(1u32));
            e.attr("end_line", serde_json::json!(10u32));
            store.insert_entity(&e, &[path.to_string()]).unwrap();
        }
        (dir, store)
    }

    /// One required entry (concrete http invocation surface) whose
    /// declaration header is a SINGLE 400-argument line — even the
    /// level-1 compressed render (first signature line) exceeds any small
    /// hard max, so the progressive ladder must descend to symbol
    /// identity.
// trace:exempt reason=unit-test-fixture
    fn pathological_signature_fixture() -> (tempfile::TempDir, Store) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let repo = store.repo_id.clone();
        let path = "api/app.py";
        let id = symbol_id(&repo, path, "big_fn");
        let mut e = Entity::new(id.clone(), kinds::SYMBOL, "big_fn".to_string());
        e.attr("kind", serde_json::json!("function"));
        e.attr("file", serde_json::json!(path));
        let mut decl = String::from("def big_fn(");
        for i in 0..400 {
            decl.push_str(&format!("arg_{i:03}: str, "));
        }
        decl.push_str(") -> None:");
        e.attr("decl_header", serde_json::json!(decl));
        e.attr("exported", serde_json::json!(true));
        e.attr("start_line", serde_json::json!(1u32));
        e.attr("end_line", serde_json::json!(10u32));
        e.attr("entrypoints", serde_json::json!(["http: POST /big"]));
        store.insert_entity(&e, &[path.to_string()]).unwrap();
        (dir, store)
    }

    /// Scores nothing — the phantom-hole baseline for the redistribution
    /// rule (semantic = 0.0 with NO renormalization).
// trace:exempt reason=unit-test-mock
    struct ZeroScorer;
// trace:exempt reason=unit-test-mock
    impl crate::rank::SemanticScorer for ZeroScorer {
// trace:exempt reason=unit-test-mock
        fn score(&self, _goal: &str, _entity: &scc_core::Entity) -> f64 {
            0.0
        }
    }

    /// Scores `score` for any entity whose name contains `target`.
// trace:exempt reason=unit-test-mock
    struct NameScorer {
        target: &'static str,
        score: f64,
    }
// trace:exempt reason=unit-test-mock
    impl crate::rank::SemanticScorer for NameScorer {
// trace:exempt reason=unit-test-mock
        fn score(&self, _goal: &str, entity: &scc_core::Entity) -> f64 {
            if entity.name.contains(self.target) {
                self.score
            } else {
                0.0
            }
        }
    }

    /// Parse the explain render into per-entry (name, component) blocks:
    /// a block starts at a `  <kind> <name>` line where `<kind>` is a
    /// SurfaceKind word (signature lines like `    public User get(...)`
    /// are NOT headers — they carry no leading kind) and collects the
    /// following `  <key>: <value>` component lines. Deterministic.
// trace:exempt reason=unit-test-helper
    fn explain_blocks(text: &str) -> Vec<(String, BTreeMap<String, f64>)> {
        const KINDS: [&str; 11] = [
            "function", "method", "constructor", "class", "interface",
            "trait", "type", "enum", "const", "module", "record",
        ];
        let mut blocks: Vec<(String, BTreeMap<String, f64>)> = Vec::new();
        for line in text.lines() {
            // Entry headers carry EXACTLY two leading spaces (signature
            // lines and section content are indented deeper, so a
            // `    class UserService` signature can never be a header).
            if !line.starts_with("   ") {
                if let Some(rest) = line.strip_prefix("  ") {
                    let first = rest.split_whitespace().next().unwrap_or("");
                    if KINDS.contains(&first) {
                        let name = rest.split_whitespace().nth(1).unwrap_or("").to_string();
                        blocks.push((name, BTreeMap::new()));
                        continue;
                    }
                }
            }
            if let Some((_, comps)) = blocks.last_mut() {
                if let Some((k, v)) = line.trim_start().split_once(':') {
                    if let Ok(num) = v.trim().parse::<f64>() {
                        comps.insert(k.trim().to_string(), num);
                    }
                }
            }
        }
        blocks
    }

    #[test]
// trace:exempt reason=unit-test
    fn explain_renders_full_score_decomposition() {
        let (_dir, store) = fixture_store();
        let ctx = make_ctx(&store);
        let request = SurfaceRequest {
            mode: SurfaceMode::Task {
                goal: "create user",
                visible: None,
            },
            budget: 100_000,
            explain: true,
            policy: SurfacePolicy::defaults(100_000),
            semantic: None,
        };
        let out = build_surface(&ctx, request);
        // All eight components + total + reasons render per entry — never
        // a bare `importance:` (the reviewer's --explain complaint).
        for key in [
            "importance:",
            "task_ppr:",
            "global_ppr:",
            "lexical:",
            "semantic:",
            "confidence:",
            "criticality:",
            "change_risk:",
            "novelty:",
            "because:",
        ] {
            assert!(out.text.contains(key), "explain must render {key:?}");
        }
        // Reasons populated from the entry's own evidence.
        assert!(out.text.contains("task seed:"), "seed evidence -> reason");
        assert!(out.text.contains("primary flow participant"), "flow evidence -> reason");
        assert!(out.text.contains("public component surface"), "visibility evidence -> reason");
        assert!(out.text.contains("owns "), "state-authority evidence -> reason");
        assert!(out.text.contains("concrete invocation surface"), "invocation evidence -> reason");
        // No phantom semantic: without a scorer the component is honestly 0.
        for (_, comps) in explain_blocks(&out.text) {
            assert_eq!(comps.get("semantic").copied().unwrap_or(-1.0), 0.0);
        }
        // Deterministic: same request -> byte-identical explain text.
        let out2 = build_surface(&ctx, request);
        assert_eq!(out.text, out2.text);
    }

    #[test]
// trace:exempt reason=unit-test
    fn semantic_none_redistributes_and_some_reranks() {
        let (_dir, store) = symmetric_two_fn_fixture();
        let ctx = make_ctx(&store);
        let alpha = symbol_id(&store.repo_id, "api/app.py", "alpha_fn");
        let beta = symbol_id(&store.repo_id, "api/app.py", "beta_fn");

        // (a) semantic=None: the 10% share is REALLOCATED, never a phantom.
        let none = build_surface(
            &ctx,
            SurfaceRequest {
                mode: SurfaceMode::Global,
                budget: 100_000,
                explain: true,
                policy: SurfacePolicy::defaults(100_000),
                semantic: None,
            },
        );
        let zero = build_surface(
            &ctx,
            SurfaceRequest {
                mode: SurfaceMode::Global,
                budget: 100_000,
                explain: true,
                policy: SurfacePolicy::defaults(100_000),
                semantic: Some(&ZeroScorer),
            },
        );
        let none_blocks = explain_blocks(&none.text);
        let zero_blocks = explain_blocks(&zero.text);
        assert!(!none_blocks.is_empty());
        assert_eq!(none_blocks.len(), zero_blocks.len());
        // The symmetric fixture renders the same entries in the same order
        // (the renormalization scales every total uniformly), so the
        // per-entry totals pair up.
        for ((nname, ncomps), (zname, zcomps)) in none_blocks.iter().zip(&zero_blocks) {
            assert_eq!(nname, zname);
            let n = ncomps.get("importance").copied().unwrap();
            let z = zcomps.get("importance").copied().unwrap();
            assert!(
                n > z,
                "renormalized total must exceed the phantom-hole total for {nname}"
            );
            // The full formula, from the rendered components: total ==
            // REDISTRIBUTION_SCALE * blend + NOVELTY_WEIGHT * novelty,
            // where blend = final_importance(..., semantic=0, novelty=0).
            let blend = crate::pagerank::final_importance(
                ncomps.get("task_ppr").copied().unwrap_or(0.0),
                ncomps.get("global_ppr").copied().unwrap_or(0.0),
                ncomps.get("lexical").copied().unwrap_or(0.0),
                0.0,
                ncomps.get("confidence").copied().unwrap_or(0.0),
                ncomps.get("criticality").copied().unwrap_or(0.0),
                ncomps.get("change_risk").copied().unwrap_or(0.0),
                0.0,
                false, // global mode: no task focus
            );
            let expected = blend * REDISTRIBUTION_SCALE
                + crate::pagerank::NOVELTY_WEIGHT
                    * ncomps.get("novelty").copied().unwrap_or(0.0);
            assert!(
                (n - expected).abs() < 0.002,
                "{nname}: rendered {n:.6} != renormalized {expected:.6} (blend {blend:.6})"
            );
        }

        // (b) semantic=Some with a real scorer: the 10% is LIVE. The beta
        // scorer flips the tie — beta_fn overtakes alpha_fn by exactly the
        // semantic weight, and the rendered order changes.
        let real = build_surface(
            &ctx,
            SurfaceRequest {
                mode: SurfaceMode::Global,
                budget: 100_000,
                explain: true,
                policy: SurfacePolicy::defaults(100_000),
                semantic: Some(&NameScorer {
                    target: "beta_fn",
                    score: 1.0,
                }),
            },
        );
        let blocks = explain_blocks(&real.text);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].0, "beta_fn", "semantic 10% must rerank beta first");
        let a = blocks
            .iter()
            .find(|(n, _)| *n == "alpha_fn")
            .and_then(|(_, c)| c.get("importance").copied())
            .unwrap();
        let b = blocks
            .iter()
            .find(|(n, _)| *n == "beta_fn")
            .and_then(|(_, c)| c.get("importance").copied())
            .unwrap();
        assert!(
            (b - a - crate::pagerank::SEMANTIC_WEIGHT).abs() < 0.002,
            "beta must lead alpha by the real 10%: {b:.6} - {a:.6}"
        );
        // Tie-break check on the None run: alpha first (id order).
        assert_eq!(none_blocks[0].0, "alpha_fn");
        assert_eq!(none.rendered_ids[0], alpha);
        assert_eq!(real.rendered_ids[0], beta);
    }

    #[test]
// trace:exempt reason=unit-test
    fn pathological_required_entry_compresses_to_symbol_identity() {
        // One required entry whose compressed form still exceeds the hard
        // max: the progressive ladder re-renders until it fits — the last
        // resort (symbol identity) always fits, and the hard-max invariant
        // holds on the ACTUAL rendered text.
        let (_dir, store) = pathological_signature_fixture();
        let ctx = make_ctx(&store);
        let hard_max = 20usize;
        let out = build_surface(
            &ctx,
            SurfaceRequest {
                mode: SurfaceMode::Global,
                budget: 1,
                explain: false,
                policy: SurfacePolicy {
                    quotas: true,
                    mmr: true,
                    coverage: true,
                    hard_max,
                },
                semantic: None,
            },
        );
        assert_eq!(out.rendered_ids.len(), 1, "the required entry is never dropped");
        assert!(out.text.contains("function big_fn"), "identity line must render");
        assert!(
            !out.text.contains("def big_fn("),
            "the 400-arg signature must be dropped at the identity level"
        );
        assert!(
            out.token_count <= hard_max,
            "hard-max invariant on the rendered text: {} > {hard_max}",
            out.token_count
        );
    }
}