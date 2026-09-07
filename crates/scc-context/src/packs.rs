//! Context pack builders (docs/CONTEXT_COMPILER.md §8, §9).
//!
//! Section priority contract: invariants (10), ownership (10), directly
//! affected contracts (9), known failure/retry behavior (9), stale warnings
//! (always appended) may never be cut for budget. Lower-priority sections
//! are dropped before truncation.

use crate::rank::terms;
use crate::{ContextCompiler, ContextPack};
use scc_core::kinds;
use scc_core::{
    entity_id, estimate_tokens, path_matches_locus, route_query, truncate_to_budget, Provenance,
    Severity,
};
use scc_graph::TrustedGraphView;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Debug, Clone)]
// trace:exempt reason=internal-detail
pub(crate) struct Section {
    pub(crate) title: String,
    pub(crate) body: String,
    /// 10 = never cut, 9 = never cut, 5 = lowest
    pub(crate) priority: u8,
}

impl Section {
    pub(crate) fn new(title: &str, body: String, priority: u8) -> Section {
        Section {
            title: title.to_string(),
            body,
            priority,
        }
    }
}

/// What the renderer actually did to fit the budget — honest accounting
/// (P0): the pack is never silently cut; if the minimum safe result still
/// exceeds the budget, `exceeded_soft_budget` is set and the content stays
/// complete.
#[derive(Debug, Clone, Default)]
struct RenderOutcome {
    original_tokens: usize,
    dropped_sections: Vec<String>,
    hard_truncated: bool,
    exceeded_soft_budget: bool,
}

// trace:exempt reason=internal-detail
fn render(sections: Vec<Section>, budget: usize, warnings: Vec<String>) -> (String, RenderOutcome) {
    let mut sections = sections;
    let mut outcome = RenderOutcome {
        original_tokens: estimate_tokens(&assemble(&sections)),
        ..Default::default()
    };
    // assemble; then drop lowest-priority sections while over budget.
    // Sections with priority >= 9 (invariants, ownership, contracts,
    // failure behavior) are never dropped and never hard-truncated.
    let mut content = assemble(&sections);
    let mut tokens = estimate_tokens(&content);
    while tokens > budget {
        let min_priority = sections.iter().map(|s| s.priority).min().unwrap_or(10);
        if min_priority >= 9 {
            break; // cannot drop anything else
        }
        let idx = sections
            .iter()
            .position(|s| s.priority == min_priority)
            .unwrap();
        let dropped = sections.remove(idx).title;
        if !outcome.dropped_sections.contains(&dropped) {
            outcome.dropped_sections.push(dropped);
        }
        content = assemble(&sections);
        tokens = estimate_tokens(&content);
        if sections.is_empty() {
            break;
        }
    }
    // warnings always appended (never cut): they are short
    for w in warnings {
        content.push_str(&format!("\n⚠ WARNING: {w}\n"));
    }
    outcome.hard_truncated = false;
    outcome.exceeded_soft_budget = estimate_tokens(&content) > budget;
    (content, outcome)
}
// trace:v1 id=impl.scc.packs work=WORK-SCC-001 satisfies=REQ-SCC-CTX

/// Render a pack and record honest budget accounting on it.
pub(crate) fn finish(
    pack: &mut ContextPack,
    sections: Vec<Section>,
    budget: usize,
    warnings: Vec<String>,
) {
    let (content, outcome) = render(sections, budget, warnings);
    pack.content = content;
    pack.budget = budget;
    pack.tokens = estimate_tokens(&pack.content);
    pack.original_tokens = outcome.original_tokens;
    pack.dropped_sections = outcome.dropped_sections;
    pack.hard_truncated = outcome.hard_truncated;
    pack.exceeded_soft_budget = outcome.exceeded_soft_budget;
    pack.truncated =
        pack.hard_truncated || !pack.dropped_sections.is_empty() || pack.exceeded_soft_budget;
}

/// Inspectable fixed-quota packer. Production [`finish`] stays adaptive.
/// Truncated buckets are named in `dropped_sections` as `quota:<bucket>`.
// trace:v1 id=impl.scc.context.finish-rollover work=WORK-ripwire-lessons-phase3 satisfies=REQ-budget-rollover
pub(crate) fn finish_with_rollover(
    pack: &mut ContextPack,
    sections: Vec<Section>,
    budget: usize,
    warnings: Vec<String>,
) {
    let original = estimate_tokens(&assemble(&sections));
    let mut buckets: [Vec<Section>; 6] = Default::default();
    for s in sections {
        buckets[quota_bucket(&s.title)].push(s);
    }
    let used: [usize; 6] = std::array::from_fn(|i| estimate_tokens(&assemble(&buckets[i])));
    let filled = crate::budget::fill_task_context_quotas(budget, &used);
    let mut kept: Vec<Section> = Vec::new();
    let mut dropped: Vec<String> = Vec::new();
    for (i, secs) in buckets.into_iter().enumerate() {
        let assembled = assemble(&secs);
        let grant = filled[i].granted;
        if filled[i].truncated > 0 {
            dropped.push(format!("quota:{}", filled[i].name));
            let fitted = truncate_to_budget(&assembled, grant.max(1));
            kept.push(Section::new(
                &format!("{} (truncated)", filled[i].name),
                fitted,
                5,
            ));
        } else {
            kept.extend(secs);
        }
    }
    for w in warnings {
        kept.push(Section::new("WARNING", format!("{w}\n"), 10));
    }
    pack.content = assemble(&kept);
    pack.budget = budget;
    pack.tokens = estimate_tokens(&pack.content);
    pack.original_tokens = original;
    pack.dropped_sections = dropped;
    pack.hard_truncated = pack
        .dropped_sections
        .iter()
        .any(|d| d.starts_with("quota:"));
    pack.exceeded_soft_budget = pack.tokens > budget;
    pack.truncated = pack.hard_truncated || pack.exceeded_soft_budget;
}

// trace:exempt reason=internal-detail
fn quota_bucket(title: &str) -> usize {
    match title {
        "TASK"
        | "SYSTEM ROLE"
        | "RELEVANT COMPONENTS"
        | "DATA OWNERSHIP"
        | "LOCUS"
        | "IDENTITY"
        | "COMPONENTS" => 0,
        "UPSTREAM" | "DOWNSTREAM" | "CONTRACTS" => 1,
        "IMPLEMENTATION" => 2,
        "PRIMARY FLOW" | "SECONDARY FLOWS" | "FLOWS" => 3,
        "TESTS" | "INVARIANTS" => 4,
        _ => 5,
    }
}

fn assemble(sections: &[Section]) -> String {
    let mut out = String::new();
    for s in sections {
        out.push_str(&format!("# {}\n{}\n\n", s.title, s.body));
    }
    out.trim_end().to_string()
}

pub(crate) fn entity_name(view: &TrustedGraphView, id: &str) -> String {
    view.name_of(id)
}

fn component_short(view: &TrustedGraphView, id: &str) -> String {
    let name = entity_name(view, id);
    let name = name.strip_prefix("component:").unwrap_or(&name);
    name.to_string()
}

// trace:exempt reason=internal-detail
fn format_evidence_tags(ctx: &ContextCompiler, entity_ids: &[String]) -> String {
    let counts = ctx.evidence_summary(entity_ids);
    if counts.is_empty() {
        return String::new();
    }
    let mut parts: Vec<String> = counts.iter().map(|(k, v)| format!("{v} {k}")).collect();
    parts.sort();
    format!("[evidence: {}]", parts.join(", "))
}

// ---------------------------------------------------------------------------
// system_overview / startup capsule
// ---------------------------------------------------------------------------

pub fn overview(ctx: &ContextCompiler) -> ContextPack {
    let mut pack = ContextPack::new("overview", &ctx.revision());
    let mut sections: Vec<Section> = Vec::new();

    let repo = ctx.store.repository();
    let snapshot = ctx.store.latest_snapshot().ok().flatten();
    let stats = ctx.store.stats().unwrap_or_default();

    // IDENTITY
    let languages: Vec<String> = {
        let mut m: BTreeSet<String> = BTreeSet::new();
        for (_, _, lang, _, _) in ctx.store.all_files().unwrap_or_default() {
            if lang != "other" && lang != "unknown" {
                m.insert(lang);
            }
        }
        m.into_iter().collect()
    };
    let purpose = ctx
        .store
        .meta_get("purpose")
        .ok()
        .flatten()
        .unwrap_or_default();
    let mut ident = String::new();
    ident.push_str(&format!("Repository: {} ({})\n", repo.name, repo.id));
    if let Some(s) = &snapshot {
        ident.push_str(&format!("Revision: {}\n", s.revision));
        if let Some(b) = &s.branch {
            ident.push_str(&format!("Branch: {}\n", b));
        }
        ident.push_str(&format!("Indexed at: {}\n", s.indexed_at));
    } else {
        ident.push_str("Index status: NOT INDEXED\n");
    }
    ident.push_str(&format!("Languages: {}\n", languages.join(", ")));
    let eps: Vec<String> = ctx
        .view
        .entities_of_kind(kinds::SYMBOL)
        .into_iter()
        .filter(|e| e.attributes.contains_key("entrypoints"))
        .map(|e| e.name.clone())
        .take(10)
        .collect();
    if !eps.is_empty() {
        ident.push_str(&format!("Entrypoints: {}\n", eps.join(", ")));
    }
    if !purpose.is_empty() {
        ident.push_str(&format!(
            "\n[SYSTEM PURPOSE — from README, DOCUMENTATION not fact]\n{purpose}\n"
        ));
    }
    sections.push(Section::new("IDENTITY", ident, 10));

    // COMPONENTS
    let mut comps = String::new();
    for c in ctx.store.components().unwrap_or_default() {
        let resp = c
            .attributes
            .get("responsibility")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|r| r.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("");
        comps.push_str(&format!("- {}: {}\n", c.name, resp));
    }
    sections.push(Section::new("COMPONENTS", comps, 9));

    // BOUNDARIES / deployment units + externals
    let dus: Vec<String> = ctx
        .view
        .entities_of_kind(kinds::DEPLOYMENT_UNIT)
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
    let exts: Vec<String> = ctx
        .view
        .entities_of_kind(kinds::EXTERNAL_API)
        .into_iter()
        .map(|e| e.name.clone())
        .collect();
    let mut bound = String::new();
    if !dus.is_empty() {
        bound.push_str(&format!("Deployment units: {}\n", dus.join(", ")));
    }
    if !exts.is_empty() {
        bound.push_str(&format!("External systems: {}\n", exts.join(", ")));
    }
    if bound.is_empty() {
        bound.push_str("(none detected)\n");
    }
    sections.push(Section::new("BOUNDARIES", bound, 8));

    // STORES
    let stores: Vec<String> = ctx
        .view
        .entities_of_kind(kinds::DATA_STORE)
        .into_iter()
        .map(|e| {
            let tech = e
                .attributes
                .get("technology")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if tech.is_empty() {
                e.name.clone()
            } else {
                format!("{} ({tech})", e.name)
            }
        })
        .collect();
    sections.push(Section::new(
        "STORES",
        if stores.is_empty() {
            "(none detected)".into()
        } else {
            format!("{}\n", stores.join(", "))
        },
        8,
    ));

    // FLOWS
    let flows: Vec<String> = ctx
        .view
        .flows()
        .iter()
        .map(|f| {
            let trig = f
                .trigger
                .as_ref()
                .map(|t| format!(" [{t}]"))
                .unwrap_or_default();
            format!("- {} ({}){trig}", f.name, flow_kind_str(f.kind))
        })
        .collect();
    sections.push(Section::new(
        "FLOWS",
        if flows.is_empty() {
            "(none compiled)".into()
        } else {
            format!("{}\n", flows.join("\n"))
        },
        8,
    ));

    // INVARIANTS
    let invs = ctx.store.invariants().unwrap_or_default();
    let mut inv_body = String::new();
    for inv in invs {
        inv_body.push_str(&format!(
            "- [{}] {}\n",
            severity_str(inv.severity),
            inv.statement
        ));
    }
    if inv_body.is_empty() {
        inv_body.push_str("(none declared)\n");
    }
    sections.push(Section::new("INVARIANTS", inv_body, 10));

    // EVIDENCE STATUS
    let mut ev = String::new();
    for (k, v) in &stats {
        ev.push_str(&format!("{k}: {v}\n"));
    }
    sections.push(Section::new("INDEX STATUS", ev, 5));

    let warnings = ctx_warnings(ctx);
    finish(&mut pack, sections, ctx.settings.startup_tokens, warnings);
    pack
}

fn ctx_warnings(ctx: &ContextCompiler) -> Vec<String> {
    let mut w = Vec::new();
    if ctx.store.snapshot_status().ok().flatten().is_none() {
        w.push("Repository is not indexed — run `scc index`.".into());
    }
    if !ctx.stale_paths.is_empty() {
        w.push(format!(
            "Model is stale: {} changed file(s) not yet re-indexed.",
            ctx.stale_paths.len()
        ));
    }
    if let Ok(findings) = ctx.store.drift_findings(true) {
        for (_, kind, sev, msg, _) in findings {
            if sev == "high" || sev == "critical" {
                w.push(format!("Drift [{kind}]: {msg}"));
            }
        }
    }
    w.truncate(6);
    w
}

// ---------------------------------------------------------------------------
// task_context
// ---------------------------------------------------------------------------

pub fn task(
    ctx: &ContextCompiler,
    goal: &str,
    files: &[String],
    symbols: &[String],
    budget: usize,
) -> ContextPack {
    task_with_rankers(ctx, goal, files, symbols, budget, None, None)
}

/// `task` with optional semantic scorer + reranker (SCC-071).
// trace:exempt reason=internal-detail
pub fn task_with_rankers(
    ctx: &ContextCompiler,
    goal: &str,
    files: &[String],
    symbols: &[String],
    budget: usize,
    scorer: Option<&dyn crate::rank::SemanticScorer>,
    reranker: Option<&dyn crate::rank::Reranker>,
) -> ContextPack {
    let mut pack = ContextPack::new("task", &ctx.revision());
    let goal_terms = terms(goal);

    let locus = resolve_goal_loci(ctx, goal);
    let mut files_buf: Vec<String> = files.to_vec();
    let mut symbols_buf: Vec<String> = symbols.to_vec();
    if let Some(loc) = &locus {
        for f in &loc.files {
            if !files_buf.iter().any(|x| x == f) {
                files_buf.push(f.clone());
            }
        }
        for s in &loc.symbols {
            if !symbols_buf.iter().any(|x| x == s) {
                symbols_buf.push(s.clone());
            }
        }
    }
    let files = files_buf.as_slice();
    let symbols = symbols_buf.as_slice();

    // ---- candidate generation ----
    let candidates = crate::rank::collect_lexical_candidates_full(
        ctx.store, &ctx.view, goal, symbols, 24, scorer, reranker,
    );
    let entity_ids: Vec<String> = candidates.iter().map(|c| c.id.clone()).collect();

    // symbol -> file -> component
    let mut symbol_files: HashMap<String, String> = HashMap::new();
    for e in ctx.view.entities_of_kind(kinds::SYMBOL) {
        if let Some(f) = e.attributes.get("file").and_then(|v| v.as_str()) {
            symbol_files.insert(e.id.clone(), f.to_string());
        }
    }
    // Every component that CONTAINS a file: merged clusters *and* member
    // regions. Last-write-wins hid `root`/`services` after type narrowing
    // merged them into `root+services`.
    let mut file_components: HashMap<String, BTreeSet<String>> = HashMap::new();
    for c in ctx.store.components().unwrap_or_default() {
        for r in ctx.view.out_pred(&c.id, scc_core::predicates::CONTAINS) {
            file_components
                .entry(r.object.clone())
                .or_default()
                .insert(c.id.clone());
        }
    }
    let mut symbol_components: HashMap<String, BTreeSet<String>> = HashMap::new();
    for (sid, f) in &symbol_files {
        if let Some(cids) =
            file_components.get(&entity_id(&ctx.view.graph.repo_id, kinds::FILE, f))
        {
            symbol_components.insert(sid.clone(), cids.clone());
        }
    }

    // affected components = candidates' components + file args' components
    let mut affected_comps: BTreeSet<String> = BTreeSet::new();
    for c in &candidates {
        if c.kind == kinds::SYMBOL {
            if let Some(cids) = symbol_components.get(&c.id) {
                affected_comps.extend(cids.iter().cloned());
            }
        } else if c.kind == kinds::COMPONENT {
            affected_comps.insert(c.id.clone());
        } else if c.kind == kinds::FILE {
            if let Some(cids) = file_components.get(&c.id) {
                affected_comps.extend(cids.iter().cloned());
            }
        } else if c.kind == kinds::ROUTE {
            if let Some(h) = ctx
                .view
                .entity(&c.id)
                .and_then(|e| e.attributes.get("handler"))
                .and_then(|v| v.as_str())
            {
                if let Some(cids) = symbol_components.get(h) {
                    affected_comps.extend(cids.iter().cloned());
                }
            }
        }
    }
    for f in files {
        let fid = entity_id(&ctx.view.graph.repo_id, kinds::FILE, f);
        if let Some(cids) = file_components.get(&fid) {
            affected_comps.extend(cids.iter().cloned());
        }
    }

    // flows mentioning affected components
    let mut affected_flows: Vec<(String, f64)> = Vec::new();
    for f in &ctx.view.flows() {
        let mut score = 0.0;
        let mentions = f.steps.iter().any(|s| {
            let hit = affected_comps.iter().any(|c| s.actor.contains(c));
            if hit {
                score += 2.0;
            }
            hit
        });
        // goal terms in flow name/trigger
        let ftext = format!("{} {}", f.name, f.trigger.clone().unwrap_or_default());
        let ft = terms(&ftext);
        score += ft.intersection(&goal_terms).count() as f64 * 1.5;
        // concrete behavior flows beat the system-wide architecture view
        // when choosing a primary flow
        if f.kind != scc_core::FlowKind::Architecture {
            score += 0.5;
        }
        if mentions || score > 0.0 {
            affected_flows.push((f.id.clone(), score));
        }
    }
    affected_flows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // upstream / downstream
    let mut upstream: BTreeSet<String> = BTreeSet::new();
    let mut downstream: BTreeSet<String> = BTreeSet::new();
    for cid in &affected_comps {
        for r in ctx.view.out_pred(cid, scc_core::predicates::DEPENDS_ON) {
            downstream.insert(r.object.clone());
        }
        for r in ctx.view.in_pred(cid, scc_core::predicates::DEPENDS_ON) {
            upstream.insert(r.subject.clone());
        }
    }

    // contracts: routes handled by symbols in affected comps
    let mut contracts: BTreeSet<String> = BTreeSet::new();
    let affected_syms: HashSet<&String> = symbol_components
        .iter()
        .filter(|(_, cids)| cids.iter().any(|c| affected_comps.contains(c)))
        .map(|(s, _)| s)
        .collect();
    for sid in &affected_syms {
        for r in ctx.view.out_pred(sid, scc_core::predicates::HANDLES) {
            contracts.insert(r.object.clone());
        }
    }

    // ownership: stores owned by affected comps
    let mut owned_stores: BTreeSet<String> = BTreeSet::new();
    for cid in &affected_comps {
        for r in ctx.view.out_pred(cid, scc_core::predicates::OWNS) {
            owned_stores.insert(r.object.clone());
        }
    }

    // invariants scoped to affected entities
    let mut inv_ids: BTreeSet<String> = BTreeSet::new();
    for inv in &ctx.view.invariants() {
        if inv
            .scope
            .iter()
            .any(|s| affected_comps.contains(s) || owned_stores.contains(s))
            || inv.enforced_by.iter().any(|t| {
                goal_terms
                    .iter()
                    .any(|g| t.to_ascii_lowercase().contains(g))
            })
        {
            inv_ids.insert(inv.id.clone());
        }
    }

    // tests_to_run: each test carries why it was selected
    let tests = collect_tests_to_run(ctx, &affected_syms, &affected_comps, &owned_stores);

    // retries/failures in affected components
    let mut retries: Vec<String> = Vec::new();
    for cid in &affected_comps {
        if let Some(c) = ctx.view.entity(cid) {
            if let Some(rs) = c.attributes.get("retries").and_then(|v| v.as_array()) {
                for r in rs {
                    if let Some(s) = r.as_str() {
                        retries.push(format!("{s} [in {}]", entity_name(&ctx.view, cid)));
                    }
                }
            }
        }
    }

    // ---- sections ----
    let mut sections: Vec<Section> = Vec::new();

    let files_disp = if files.is_empty() {
        "(none)".to_string()
    } else {
        files.join(", ")
    };
    let symbols_disp = if symbols.is_empty() {
        "(none)".to_string()
    } else {
        symbols.join(", ")
    };
    sections.push(Section::new(
        "TASK",
        format!("Goal: {goal}\nExplicit files: {files_disp}\nExplicit symbols: {symbols_disp}",),
        10,
    ));
    if let Some(loc) = &locus {
        sections.push(Section::new("LOCUS", loc.body.clone(), 9));
    }

    // SYSTEM ROLE
    let purpose = ctx
        .store
        .meta_get("purpose")
        .ok()
        .flatten()
        .unwrap_or_default();
    let mut role = String::new();
    if !purpose.is_empty() {
        role.push_str(&format!(
            "[SYSTEM PURPOSE — DOCUMENTATION from README]\n{purpose}\n\n"
        ));
    }
    let all_comps = ctx.store.components().unwrap_or_default();
    let top_comps: Vec<&scc_core::Entity> = all_comps
        .iter()
        .filter(|c| affected_comps.contains(&c.id))
        .collect();
    for c in top_comps {
        let resp = c
            .attributes
            .get("responsibility")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|r| r.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("");
        role.push_str(&format!("{}: {}\n", c.name, resp));
    }
    if role.is_empty() {
        role.push_str("(no system role compiled)\n");
    }
    sections.push(Section::new("SYSTEM ROLE", role, 9));

    // RELEVANT COMPONENTS
    let mut comp_body = String::new();
    for cid in &affected_comps {
        if let Some(c) = ctx.view.entity(cid) {
            let resp = c
                .attributes
                .get("responsibility")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|r| r.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("");
            comp_body.push_str(&format!("- {}: {}\n", c.name, resp));
        }
    }
    if comp_body.is_empty() {
        for c in candidates.iter().take(6) {
            if c.kind == kinds::COMPONENT || c.kind == kinds::SYMBOL {
                comp_body.push_str(&format!("- {} ({}) [{:.2}]\n", c.name, c.kind, c.score));
            }
        }
    }
    sections.push(Section::new("RELEVANT COMPONENTS", comp_body, 10));

    // PRIMARY FLOW
    if let Some((fid, _)) = affected_flows.first() {
        let body = render_flow(ctx, fid, true);
        sections.push(Section::new("PRIMARY FLOW", body, 9));
    }

    // SECONDARY FLOWS
    if affected_flows.len() > 1 {
        let mut body = String::new();
        for (fid, _) in affected_flows.iter().skip(1).take(4) {
            if let Some(f) = ctx.view.flows().iter().find(|f| &f.id == fid) {
                body.push_str(&format!(
                    "- {} [{}]\n",
                    f.name,
                    f.trigger.clone().unwrap_or_default()
                ));
            }
        }
        sections.push(Section::new("SECONDARY FLOWS", body, 8));
    }

    // UPSTREAM / DOWNSTREAM
    sections.push(Section::new(
        "UPSTREAM",
        if upstream.is_empty() {
            "(none)".into()
        } else {
            format!(
                "{}\n",
                upstream
                    .iter()
                    .map(|c| component_short(&ctx.view, c))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        },
        9,
    ));
    sections.push(Section::new(
        "DOWNSTREAM",
        if downstream.is_empty() {
            "(none)".into()
        } else {
            format!(
                "{}\n",
                downstream
                    .iter()
                    .map(|c| component_short(&ctx.view, c))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        },
        9,
    ));

    // DATA OWNERSHIP
    let mut data_body = String::new();
    for store_id in &owned_stores {
        let name = entity_name(&ctx.view, store_id);
        let writers: Vec<String> = ctx
            .view
            .in_pred(store_id, scc_core::predicates::WRITES)
            .into_iter()
            .map(|r| {
                symbol_components
                    .get(&r.subject)
                    .map(|cids| {
                        let mut names: Vec<String> = cids
                            .iter()
                            .map(|c| component_short(&ctx.view, c))
                            .collect();
                        names.sort();
                        names.dedup();
                        names.join(", ")
                    })
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| entity_name(&ctx.view, &r.subject))
            })
            .collect();
        let writers_disp = if writers.is_empty() {
            "?".to_string()
        } else {
            writers.join(", ")
        };
        data_body.push_str(&format!("- {name}: owner(s) {writers_disp}\n"));
    }
    if data_body.is_empty() {
        data_body.push_str("(no ownership compiled)\n");
    }
    sections.push(Section::new("DATA OWNERSHIP", data_body, 10));

    // CONTRACTS
    let mut contract_body = String::new();
    for rid in &contracts {
        if let Some(r) = ctx.view.entity(rid) {
            contract_body.push_str(&format!("- {}\n", r.name));
        }
    }
    if !contract_body.is_empty() {
        sections.push(Section::new("CONTRACTS", contract_body, 9));
    }

    // INVARIANTS
    let mut inv_body = String::new();
    for id in &inv_ids {
        if let Some(inv) = ctx.view.invariants().iter().find(|i| &i.id == id) {
            inv_body.push_str(&format!(
                "- [{}] {} {}\n",
                severity_str(inv.severity),
                inv.statement,
                if inv.enforced_by.is_empty() {
                    "(no enforcing test)"
                } else {
                    ""
                }
            ));
        }
    }
    if !inv_body.is_empty() {
        sections.push(Section::new("INVARIANTS", inv_body, 10));
    }

    // FAILURE / RETRY / FALLBACK
    if !retries.is_empty() {
        sections.push(Section::new(
            "FAILURE / RETRY",
            format!("{}\n", retries.join("\n")),
            9,
        ));
    }

    // IMPLEMENTATION
    let mut impl_body = String::new();
    for cid in &affected_comps {
        if let Some(c) = ctx.view.entity(cid) {
            if let Some(paths) = c
                .attributes
                .get("implementation")
                .and_then(|i| i.get("paths"))
                .and_then(|p| p.as_array())
            {
                let ps: Vec<&str> = paths.iter().filter_map(|p| p.as_str()).collect();
                if !ps.is_empty() {
                    impl_body.push_str(&format!("{}: {}\n", c.name, ps.join(", ")));
                }
            }
        }
    }
    let mut seen_syms: std::collections::HashSet<String> = std::collections::HashSet::new();
    for c in candidates.iter() {
        if c.kind == kinds::SYMBOL && seen_syms.insert(c.id.clone()) {
            let file = ctx
                .view
                .entity(&c.id)
                .and_then(|e| e.attributes.get("file"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !file.is_empty() {
                impl_body.push_str(&format!("{} ({file})\n", c.name));
            }
        }
        if seen_syms.len() >= 12 {
            break;
        }
    }
    if !impl_body.is_empty() {
        sections.push(Section::new("IMPLEMENTATION", impl_body, 7));
    }

    // TESTS TO RUN (with file locations and reasons)
    let mut test_body = String::new();
    for (tid, reasons) in &tests {
        test_body.push_str(&format_test_to_run(ctx, tid, reasons));
    }
    if !test_body.is_empty() {
        sections.push(Section::new("TESTS", test_body, 7));
    }

    // RECENT CHANGES
    if !files.is_empty() {
        sections.push(Section::new(
            "RECENT CHANGES",
            format!("{}\n", files.join("\n")),
            6,
        ));
    }

    // EVIDENCE STATUS
    let mut ids: Vec<String> = Vec::new();
    ids.extend(entity_ids.clone());
    ids.extend(affected_comps.iter().cloned());
    // expansion for recall: files containing CANDIDATE symbols (not every
    // symbol in an affected component — that over-broadens) + downstream
    // components (docs/CONTEXT_COMPILER.md §6)
    for c in &candidates {
        if c.kind == kinds::SYMBOL {
            if let Some(e) = ctx.view.entity(&c.id) {
                if let Some(f) = e.attributes.get("file").and_then(|v| v.as_str()) {
                    ids.push(entity_id(&ctx.view.graph.repo_id, kinds::FILE, f));
                }
            }
        }
    }
    ids.extend(downstream.iter().cloned());
    ids.extend(tests.keys().cloned());
    // the files containing included tests (agents must find them)
    for tid in tests.keys() {
        if let Some(f) = ctx
            .view
            .entity(tid)
            .and_then(|e| e.attributes.get("file"))
            .and_then(|v| v.as_str())
        {
            ids.push(entity_id(&ctx.view.graph.repo_id, kinds::FILE, f));
        }
    }
    ids.extend(inv_ids.iter().cloned());
    ids.sort();
    ids.dedup();
    let ev_summary = ctx.evidence_summary(&ids);
    let mut ev_body = String::new();
    for (k, v) in &ev_summary {
        ev_body.push_str(&format!("{k}: {v}\n"));
    }
    if ev_body.is_empty() {
        ev_body.push_str("(no evidence linked)\n");
    }
    sections.push(Section::new("EVIDENCE STATUS", ev_body, 5));

    let analysis_quality = ctx
        .store
        .meta_get("analysis_quality")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<scc_core::AnalysisQuality>(&s).ok());
    if let Some(ref q) = analysis_quality {
        sections.push(Section::new("ANALYSIS QUALITY", q.compact_line() + "\n", 4));
    }

    // Exact source last: bodies fill leftover budget and are the first
    // section dropped. Never a fifth context level — this is Level 3
    // inside Task Context, after semantic sections.
    let exact = exact_source_tail(ctx, files, &candidates);
    if !exact.is_empty() {
        sections.push(Section::new("EXACT SOURCE", exact, 1));
    }

    let warnings = ctx_warnings(ctx);
    let stale_note = ctx
        .stale_paths
        .iter()
        .map(|p| format!("stale: {p}"))
        .collect::<Vec<_>>();
    let mut all_warnings = warnings;
    all_warnings.extend(stale_note);

    pack.entity_ids = ids;
    pack.evidence_summary = ev_summary;
    pack.analysis_quality = analysis_quality;
    match ctx.settings.pack_allocator {
        crate::PackAllocator::AdaptivePriority => {
            finish(&mut pack, sections, budget, all_warnings);
        }
        crate::PackAllocator::FixedRollover => {
            finish_with_rollover(&mut pack, sections, budget, all_warnings);
        }
    }
    pack.compression_policy = Some(compression_policy(goal));
    pack
}

const EXACT_SOURCE_FILES: usize = 6;
const EXACT_SOURCE_LINES: usize = 40;

/// Level-3 exact excerpts for Task Context. Truncation is disclosed
/// (`shown=`/`total=`/`capped=`). Priority 1 so semantic sections win.
// trace:v1 id=impl.scc.context.exact-source-last work=WORK-ripwire-lessons-phase1 satisfies=REQ-exact-source-dominance
fn exact_source_tail(
    ctx: &ContextCompiler,
    files: &[String],
    candidates: &[crate::rank::ScoredEntity],
) -> String {
    let mut paths: Vec<String> = Vec::new();
    let mut push = |p: &str| {
        if p.is_empty() {
            return;
        }
        if !paths.iter().any(|x| x == p) {
            paths.push(p.to_string());
        }
    };
    for f in files {
        push(f);
    }
    for c in candidates {
        if c.kind != kinds::SYMBOL {
            continue;
        }
        if let Some(f) = ctx
            .view
            .entity(&c.id)
            .and_then(|e| e.attributes.get("file"))
            .and_then(|v| v.as_str())
        {
            push(f);
        }
    }
    paths.truncate(EXACT_SOURCE_FILES);
    let mut body = String::new();
    let root = &ctx.store.root;
    for path in &paths {
        let Ok(text) = std::fs::read_to_string(root.join(path)) else {
            continue;
        };
        let total = text.lines().count();
        if total == 0 {
            continue;
        }
        let shown = total.min(EXACT_SOURCE_LINES);
        let capped = u8::from(shown < total);
        body.push_str(&format!(
            "# {path} shown={shown} total={total} capped={capped}\n"
        ));
        for (i, line) in text.lines().enumerate() {
            if i >= shown {
                break;
            }
            body.push_str(line);
            body.push('\n');
        }
    }
    body
}

/// RTK output-compression policy derived from the task (docs §49/§11):
/// task touches tests -> preserve failures; performance investigation ->
/// disable log compression; otherwise standard policy.
fn compression_policy(goal: &str) -> serde_json::Value {
    let g = goal.to_ascii_lowercase();
    let tests = ["test", "tests", "spec", "suite"]
        .iter()
        .any(|t| g.contains(t));
    let perf = ["perf", "performance", "latency", "slow", "benchmark"]
        .iter()
        .any(|t| g.contains(t));
    if tests {
        serde_json::json!({
            "preserve": ["failures", "test output", "stack traces"],
            "compress": ["passing output", "setup logs"],
            "rationale": "task touches tests"
        })
    } else if perf {
        serde_json::json!({
            "preserve": ["timings", "warnings", "errors"],
            "compress": ["logs"],
            "disable_log_compression": true,
            "rationale": "performance investigation"
        })
    } else {
        serde_json::json!({
            "preserve": ["errors", "failures", "warnings"],
            "compress": ["passing output", "repetitive logs"],
            "rationale": "standard"
        })
    }
}

// ---------------------------------------------------------------------------
// component_context
// ---------------------------------------------------------------------------

// trace:exempt reason=internal-detail
pub fn component(ctx: &ContextCompiler, id_or_name: &str) -> ContextPack {
    let mut pack = ContextPack::new("component", &ctx.revision());
    let comp = ctx
        .store
        .components()
        .unwrap_or_default()
        .into_iter()
        .find(|c| c.id == id_or_name || c.name == id_or_name);
    let Some(comp) = comp else {
        pack.content = format!("# COMPONENT NOT FOUND\nNo component matches '{id_or_name}'.\n");
        pack.warnings
            .push(format!("unknown component: {id_or_name}"));
        return pack;
    };
    pack.entity_ids.push(comp.id.clone());

    let mut sections: Vec<Section> = Vec::new();

    let resp: Vec<String> = comp
        .attributes
        .get("responsibility")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .map(|r| {
                    let text = r.get("text").and_then(|t| t.as_str()).unwrap_or("");
                    let prov = r.get("provenance").and_then(|p| p.as_str()).unwrap_or("");
                    format!("- {text} [{prov}]")
                })
                .collect()
        })
        .unwrap_or_default();
    sections.push(Section::new(
        "RESPONSIBILITY",
        if resp.is_empty() {
            "(none)".into()
        } else {
            format!("{}\n", resp.join("\n"))
        },
        10,
    ));

    let paths: Vec<String> = comp
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
    let symbols: Vec<String> = comp
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
    let mut impl_body = String::new();
    if !paths.is_empty() {
        impl_body.push_str(&format!("Paths: {}\n", paths.join(", ")));
    }
    if !symbols.is_empty() {
        impl_body.push_str(&format!("Symbols: {}\n", symbols.join(", ")));
    }
    sections.push(Section::new("IMPLEMENTATION", impl_body, 8));

    let owned: Vec<String> = comp
        .attributes
        .get("owns")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| entity_name(&ctx.view, s)))
                .collect()
        })
        .unwrap_or_default();
    sections.push(Section::new(
        "OWNS",
        if owned.is_empty() {
            "(none)".into()
        } else {
            format!("{}\n", owned.join(", "))
        },
        10,
    ));

    let deps: Vec<String> = comp
        .attributes
        .get("depends_on")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .map(|d| {
                    let t = d.get("target").and_then(|x| x.as_str()).unwrap_or("");
                    let n = d.get("call_count").and_then(|x| x.as_u64()).unwrap_or(0);
                    format!("- {t} ({n} call edge(s))")
                })
                .collect()
        })
        .unwrap_or_default();
    sections.push(Section::new(
        "DEPENDS_ON",
        if deps.is_empty() {
            "(none)".into()
        } else {
            format!("{}\n", deps.join("\n"))
        },
        8,
    ));

    let retries: Vec<String> = comp
        .attributes
        .get("retries")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    if !retries.is_empty() {
        sections.push(Section::new(
            "RETRIES",
            format!("{}\n", retries.join("\n")),
            9,
        ));
    }

    // flows this component participates in
    let mut flows: Vec<String> = Vec::new();
    for f in &ctx.view.flows() {
        if f.steps.iter().any(|s| s.actor.contains(&comp.id)) {
            flows.push(format!(
                "- {} [{}]",
                f.name,
                f.trigger.clone().unwrap_or_default()
            ));
        }
    }
    if !flows.is_empty() {
        sections.push(Section::new("FLOWS", format!("{}\n", flows.join("\n")), 8));
    }

    // tests
    let mut tests: Vec<String> = Vec::new();
    for sym in &symbols {
        for e in ctx.view.entities_of_kind(kinds::SYMBOL) {
            if e.name == *sym {
                for r in ctx.view.out_pred(&e.id, scc_core::predicates::TESTED_BY) {
                    tests.push(entity_name(&ctx.view, &r.object));
                }
            }
        }
    }
    tests.sort();
    tests.dedup();
    if !tests.is_empty() {
        sections.push(Section::new("TESTS", format!("{}\n", tests.join("\n")), 7));
    }

    sections.push(Section::new(
        "EVIDENCE",
        format_evidence_tags(ctx, std::slice::from_ref(&comp.id)),
        5,
    ));

    finish(&mut pack, sections, usize::MAX, ctx_warnings(ctx));
    pack
}

// ---------------------------------------------------------------------------
// flow_context
// ---------------------------------------------------------------------------

// trace:exempt reason=internal-detail
fn render_flow(ctx: &ContextCompiler, fid: &str, compact: bool) -> String {
    let flows = ctx.view.flows();
    let Some(f) = flows.iter().find(|f| f.id == fid) else {
        return format!("(flow {fid} not found)");
    };
    let mut body = String::new();
    if let Some(t) = &f.trigger {
        body.push_str(&format!("Trigger: {t}\n"));
    }
    // Wave 3 §22: lifecycle views detect state-machine signals; they are
    // never presented as authoritative state-machine ordering.
    if f.attributes.get("signals_only").and_then(|v| v.as_bool()) == Some(true) {
        body.push_str("(state-machine signals — NOT an authoritative lifecycle)\n");
    }
    let mut prev_actor: Option<String> = None;
    for s in &f.steps {
        let actor = component_short(&ctx.view, &s.actor);
        let mut line = if prev_actor.as_ref() == Some(&actor) {
            format!("  → {}", s.operation)
        } else {
            format!("{}. {}: {}", s.order, actor, s.operation)
        };
        if let Some(c) = &s.condition {
            line.push_str(&format!(" ({c})"));
        }
        if let Some(rp) = &s.retry_policy {
            line.push_str(&format!(" [retry: {rp}]"));
        }
        if s.r#async == Some(true) {
            line.push_str(" [async]");
        }
        body.push_str(&line);
        body.push('\n');
        prev_actor = Some(actor);
    }
    if compact {
        // keep it tight: strip per-step provenance
        return body;
    } else {
        body.push_str("\nEvidence: ");
        let mut ev_ids: Vec<String> = Vec::new();
        for s in &f.steps {
            ev_ids.extend(s.evidence.clone());
        }
        let ev_tags = ctx.evidence_summary(&ev_ids);
        if ev_tags.is_empty() {
            body.push_str("(none)");
        } else {
            let parts: Vec<String> = ev_tags.iter().map(|(k, v)| format!("{v} {k}")).collect();
            body.push_str(&parts.join(", "));
        }
        body.push('\n');
    }
    body
}

pub fn flow(ctx: &ContextCompiler, id_or_name: &str) -> ContextPack {
    let mut pack = ContextPack::new("flow", &ctx.revision());
    let f = ctx
        .view
        .flows()
        .iter()
        .find(|f| f.id == id_or_name || f.name == id_or_name)
        .cloned();
    let Some(f) = f else {
        pack.content = format!("# FLOW NOT FOUND\nNo flow matches '{id_or_name}'.\n");
        pack.warnings.push(format!("unknown flow: {id_or_name}"));
        return pack;
    };
    pack.entity_ids.push(f.id.clone());
    let mut sections: Vec<Section> = Vec::new();

    let steps = render_flow(ctx, &f.id, false);
    sections.push(Section::new("STEPS", steps, 10));

    let mut attrs = String::new();
    for (k, v) in &f.attributes {
        attrs.push_str(&format!("{k}: {v}\n"));
    }
    if !attrs.is_empty() {
        sections.push(Section::new("ATTRIBUTES", attrs, 6));
    }

    finish(&mut pack, sections, usize::MAX, ctx_warnings(ctx));
    pack
}

// ---------------------------------------------------------------------------
// impact_context
// ---------------------------------------------------------------------------

// trace:v1 id=impl.scc.context.impact-forgotten work=WORK-ripwire-lessons-phase6 satisfies=REQ-forgotten-impact-partners
pub fn impact(
    ctx: &ContextCompiler,
    files: &[String],
    symbols: &[String],
    diff_base: Option<&str>,
) -> ContextPack {
    let mut pack = ContextPack::new("impact", &ctx.revision());
    let mut files = files.to_vec();
    let symbols = symbols.to_vec();
    if let Some(base) = diff_base {
        if files.is_empty() && symbols.is_empty() {
            match scc_graph::impact::diff_files(ctx.store, Some(base)) {
                Ok(d) => files = d,
                Err(e) => pack.warnings.push(format!("git diff failed: {e}")),
            }
        }
    }

    let imp = match scc_graph::impact::compute_impact(&ctx.view, ctx.store, &files, &symbols) {
        Ok(i) => i,
        Err(e) => {
            pack.content = format!("# IMPACT ERROR\n{e}\n");
            pack.warnings.push(e.to_string());
            return pack;
        }
    };

    let mut sections: Vec<Section> = Vec::new();

    sections.push(Section::new(
        "AFFECTED COMPONENTS",
        if imp.components.is_empty() {
            "(none)".into()
        } else {
            format!(
                "{}\n",
                imp.components
                    .iter()
                    .map(|c| component_short(&ctx.view, c))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        },
        10,
    ));

    sections.push(Section::new(
        "FLOWS",
        if imp.flows.is_empty() {
            "(none)".into()
        } else {
            format!(
                "{}\n",
                imp.flows
                    .iter()
                    .map(|f| {
                        ctx.view
                            .flows()
                            .iter()
                            .find(|x| &x.id == f)
                            .map(|x| x.name.clone())
                            .unwrap_or_else(|| f.clone())
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        },
        9,
    ));

    sections.push(Section::new(
        "UPSTREAM",
        if imp.upstream.is_empty() {
            "(none)".into()
        } else {
            format!(
                "{}\n",
                imp.upstream
                    .iter()
                    .map(|c| component_short(&ctx.view, c))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        },
        9,
    ));
    sections.push(Section::new(
        "DOWNSTREAM",
        if imp.downstream.is_empty() {
            "(none)".into()
        } else {
            format!(
                "{}\n",
                imp.downstream
                    .iter()
                    .map(|c| component_short(&ctx.view, c))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        },
        9,
    ));

    let contracts: Vec<String> = imp
        .contracts
        .iter()
        .map(|c| entity_name(&ctx.view, c))
        .collect();
    sections.push(Section::new(
        "CONTRACTS",
        if contracts.is_empty() {
            "(none)".into()
        } else {
            format!("{}\n", contracts.join(", "))
        },
        9,
    ));

    let data: Vec<String> = imp.data.iter().map(|d| entity_name(&ctx.view, d)).collect();
    sections.push(Section::new(
        "DATA",
        if data.is_empty() {
            "(none)".into()
        } else {
            format!("{}\n", data.join(", "))
        },
        9,
    ));

    let invs: Vec<String> = imp
        .invariants
        .iter()
        .map(|i| {
            ctx.view
                .invariants()
                .iter()
                .find(|x| &x.id == i)
                .map(|x| format!("[{}] {}", severity_str(x.severity), x.statement))
                .unwrap_or_else(|| i.clone())
        })
        .collect();
    if !invs.is_empty() {
        sections.push(Section::new(
            "INVARIANTS",
            format!("{}\n", invs.join("\n")),
            10,
        ));
    }

    let tests: Vec<String> = imp
        .tests
        .iter()
        .map(|t| entity_name(&ctx.view, t))
        .collect();
    sections.push(Section::new(
        "TESTS",
        if tests.is_empty() {
            "(none)".into()
        } else {
            format!("{}\n", tests.join(", "))
        },
        7,
    ));

    sections.push(Section::new(
        "RISK",
        format!("{}\n", imp.risk.to_uppercase()),
        10,
    ));
    if !imp.forgotten_partners.is_empty() {
        let mut body = String::new();
        for p in &imp.forgotten_partners {
            body.push_str(&format!(
                "{} ({} ×{} with {}) — historical; not EXTRACTED impact\n",
                p.partner, p.reason, p.commits, p.file
            ));
        }
        sections.push(Section::new("FORGOTTEN PARTNERS", body, 6));
    }
    if !imp.notes.is_empty() {
        sections.push(Section::new(
            "NOTES",
            format!("{}\n", imp.notes.join("\n")),
            8,
        ));
    }

    pack.entity_ids = imp.components.clone();
    finish(&mut pack, sections, usize::MAX, ctx_warnings(ctx));
    pack
}

// ---------------------------------------------------------------------------
// verify_context
// ---------------------------------------------------------------------------

// trace:exempt reason=internal-detail
pub fn verify(ctx: &ContextCompiler) -> ContextPack {
    let mut pack = ContextPack::new("verify", &ctx.revision());
    let mut sections: Vec<Section> = Vec::new();

    let snapshot = ctx.store.snapshot_status().ok().flatten();
    let mut status = String::new();
    if let Some((s, files)) = &snapshot {
        status.push_str(&format!("Revision: {}\n", s.revision));
        if let Some(b) = &s.branch {
            status.push_str(&format!("Branch: {b}\n"));
        }
        status.push_str(&format!("Indexed at: {}\n", s.indexed_at));
        status.push_str(&format!("Files indexed: {files}\n"));
    } else {
        status.push_str("NOT INDEXED\n");
    }
    sections.push(Section::new("SNAPSHOT", status, 10));

    // freshness
    let mut fresh = String::new();
    if ctx.stale_paths.is_empty() {
        fresh.push_str("Fresh: all indexed files match the working tree.\n");
    } else {
        fresh.push_str(&format!(
            "STALE: {} file(s) changed since indexing:\n",
            ctx.stale_paths.len()
        ));
        for p in ctx.stale_paths.iter().take(20) {
            fresh.push_str(&format!("- {p}\n"));
        }
    }
    sections.push(Section::new("FRESHNESS", fresh, 10));

    // stale facts
    let mut stale_facts = String::new();
    let mut stale_count = 0usize;
    for ev in ctx.store.all_evidence().unwrap_or_default() {
        if let Some(p) = &ev.path {
            if ctx.is_stale_path(p) {
                stale_count += 1;
            }
        }
    }
    stale_facts.push_str(&format!(
        "{} evidence record(s) reference changed files.\n",
        stale_count
    ));
    sections.push(Section::new("STALE FACTS", stale_facts, 10));

    // graph invariants
    let mut inv = String::new();
    // 1. dangling references
    let mut dangling = 0usize;
    for r in ctx.view.all_rels() {
        // endpoints may be external_api, component, flow ids — check known
        // namespaces
        let known = |id: &str| -> bool {
            ctx.view.entity(id).is_some()
                || id.contains("/external_api/")
                || id.contains("/component/")
                || id.contains("/flow/")
                || id.contains("/invariant/")
        };
        if !known(&r.subject) {
            dangling += 1;
            if dangling <= 5 {
                inv.push_str(&format!(
                    "dangling subject: {} — {}\n",
                    r.subject, r.predicate
                ));
            }
        }
        if !known(&r.object) {
            dangling += 1;
            if dangling <= 5 {
                inv.push_str(&format!(
                    "dangling object: {} — {}\n",
                    r.predicate, r.object
                ));
            }
        }
    }
    inv.push_str(&format!("Dangling references: {dangling}\n"));
    // 2. RESOLVED facts must have evidence
    let mut no_evidence = 0usize;
    for r in ctx.view.all_rels() {
        if r.provenance == Provenance::Resolved && r.evidence.is_empty() {
            no_evidence += 1;
        }
    }
    inv.push_str(&format!("RESOLVED facts without evidence: {no_evidence}\n"));
    // 3. critical invariants unenforced
    let unenforced = ctx
        .view
        .invariants()
        .iter()
        .filter(|i| i.severity == Severity::Critical && i.enforced_by.is_empty())
        .count();
    inv.push_str(&format!(
        "Critical invariants without enforcing tests: {unenforced}\n"
    ));
    // 4. inferred claims
    let inferred = ctx
        .view
        .all_rels()
        .iter()
        .filter(|r| r.provenance == Provenance::Inferred)
        .count();
    inv.push_str(&format!("Inferred claims (labeled): {inferred}\n"));
    sections.push(Section::new("GRAPH INVARIANTS", inv, 10));

    // drift findings
    let findings = ctx.store.drift_findings(true).unwrap_or_default();
    let mut drift = String::new();
    if findings.is_empty() {
        drift.push_str("No drift findings.\n");
    }
    for (_, kind, sev, msg, _) in &findings {
        drift.push_str(&format!("- [{sev}] {kind}: {msg}\n"));
    }
    sections.push(Section::new("DRIFT", drift, 10));

    // conflicts: conflicting writers recorded as drift; also low-confidence deps
    let mut low_conf = String::new();
    let mut lc = 0usize;
    for r in ctx.view.all_rels() {
        if r.predicate == scc_core::predicates::DEPENDS_ON && r.confidence < 0.8 {
            lc += 1;
            if lc <= 8 {
                low_conf.push_str(&format!(
                    "- {} → {} ({:.2})\n",
                    component_short(&ctx.view, &r.subject),
                    component_short(&ctx.view, &r.object),
                    r.confidence
                ));
            }
        }
    }
    if lc > 0 {
        low_conf.push_str(&format!("… {lc} low-confidence dependency edge(s)\n"));
        sections.push(Section::new("LOW-CONFIDENCE DEPENDENCIES", low_conf, 8));
    }

    // trust boundaries (docs/PRD.md §7, EPIC-148)
    if let Ok(crossings) = scc_graph::boundaries::boundary_crossings(ctx.view.graph, ctx.store) {
        if !crossings.is_empty() {
            let mut b = String::new();
            b.push_str(&format!("{} boundary crossing(s):\n", crossings.len()));
            for c in crossings.iter().take(12) {
                b.push_str(&format!("- {c}\n"));
            }
            sections.push(Section::new("BOUNDARIES", b, 8));
        }
    }

    // runtime observations (docs/FLOW_COMPILER.md §8)
    let runtime_edges = ctx.store.runtime_edge_rows().unwrap_or_default();
    if !runtime_edges.is_empty() {
        let mut rt = String::new();
        let total: u64 = runtime_edges.iter().map(|e| e.count).sum();
        let errs: u64 = runtime_edges.iter().map(|e| e.errors).sum();
        rt.push_str(&format!(
            "{} observed edge(s), {} total observations, {} error(s).\n",
            runtime_edges.len(),
            total,
            errs
        ));
        for e in runtime_edges.iter().take(12) {
            rt.push_str(&format!(
                "- {} → {} ×{} (avg {:.1} ms, {} err)\n",
                e.source, e.target, e.count, e.latency_ms, e.errors
            ));
        }
        sections.push(Section::new("RUNTIME", rt, 8));
    }

    // verdict
    let mut verdict = String::new();
    let ok = ctx.stale_paths.is_empty()
        && dangling == 0
        && no_evidence == 0
        && unenforced == 0
        && findings.is_empty();
    if ok {
        verdict.push_str("VERIFIED: model is fresh, consistent, and drift-free.\n");
    } else {
        verdict.push_str("ISSUES FOUND: review the sections above before trusting context.\n");
        if !ctx.stale_paths.is_empty() {
            verdict.push_str("  → re-index changed files (`scc index`)\n");
        }
        if unenforced > 0 {
            verdict.push_str("  → critical invariants need enforcing tests or declaration\n");
        }
        if dangling > 0 || no_evidence > 0 {
            verdict.push_str("  → graph integrity violated; re-index\n");
        }
    }
    sections.push(Section::new("VERDICT", verdict, 10));

    finish(&mut pack, sections, usize::MAX, Vec::new());
    pack
}

// trace:exempt reason=internal-detail
struct LocusHits {
    files: Vec<String>,
    symbols: Vec<String>,
    body: String,
}

/// Map stack/error FILE:LINE frames onto indexed files and innermost symbols.
// trace:v1 id=impl.scc.context.stack-locus work=WORK-ripwire-lessons-phase3 satisfies=REQ-stack-locus-ingest
fn resolve_goal_loci(ctx: &crate::ContextCompiler, goal: &str) -> Option<LocusHits> {
    let plan = route_query(goal);
    if !plan.prefer_locus || plan.loci.is_empty() {
        return None;
    }
    let mut files: Vec<String> = Vec::new();
    let mut symbols: Vec<String> = Vec::new();
    let mut body = String::new();
    for loc in &plan.loci {
        let mapped_file = ctx
            .view
            .entities_of_kind(kinds::FILE)
            .into_iter()
            .map(|e| e.name.clone())
            .find(|p| path_matches_locus(p, &loc.path));
        let mapped_file = mapped_file.or_else(|| {
            ctx.view
                .entities_of_kind(kinds::SYMBOL)
                .into_iter()
                .find_map(|e| {
                    e.attributes
                        .get("file")
                        .and_then(|v| v.as_str())
                        .filter(|p| path_matches_locus(p, &loc.path))
                        .map(|p| p.to_string())
                })
        });
        match mapped_file {
            None => {
                body.push_str(&format!(
                    "- {}:{} — unmapped (not fabricated)\n",
                    loc.path, loc.line
                ));
            }
            Some(path) => {
                if !files.iter().any(|f| f == &path) {
                    files.push(path.clone());
                }
                let enclosed = innermost_enclosing_symbol(ctx, &path, loc.line);
                if let Some(name) = &enclosed {
                    if !symbols.iter().any(|s| s == name) {
                        symbols.push(name.clone());
                    }
                }
                match enclosed {
                    Some(name) => {
                        body.push_str(&format!("- {path}:{} → symbol {name}\n", loc.line))
                    }
                    None => body.push_str(&format!(
                        "- {path}:{} → file (no enclosing symbol)\n",
                        loc.line
                    )),
                }
            }
        }
    }
    Some(LocusHits {
        files,
        symbols,
        body,
    })
}

// trace:exempt reason=internal-detail
fn innermost_enclosing_symbol(
    ctx: &crate::ContextCompiler,
    file: &str,
    line: u32,
) -> Option<String> {
    let mut best: Option<(u32, String)> = None;
    for e in ctx.view.entities_of_kind(kinds::SYMBOL) {
        let Some(f) = e.attributes.get("file").and_then(|v| v.as_str()) else {
            continue;
        };
        if !path_matches_locus(f, file) && f != file {
            continue;
        }
        let start = e
            .attributes
            .get("start_line")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let end = e
            .attributes
            .get("end_line")
            .and_then(|v| v.as_u64())
            .unwrap_or(u64::from(start)) as u32;
        if start == 0 || line < start || line > end {
            continue;
        }
        let span = end.saturating_sub(start);
        if best.as_ref().map(|(s, _)| span < *s).unwrap_or(true) {
            best = Some((span, e.name.clone()));
        }
    }
    best.map(|(_, n)| n)
}

/// tests_to_run: test id → reasons (`direct`, `import`, `contract`, `state`).
// trace:v1 id=impl.scc.context.tests-to-run work=WORK-ripwire-lessons-phase3 satisfies=REQ-tests-to-run-reasons
fn collect_tests_to_run(
    ctx: &crate::ContextCompiler,
    affected_syms: &HashSet<&String>,
    affected_comps: &BTreeSet<String>,
    owned_stores: &BTreeSet<String>,
) -> BTreeMap<String, BTreeSet<&'static str>> {
    let mut tests: BTreeMap<String, BTreeSet<&'static str>> = BTreeMap::new();
    for sid in affected_syms {
        for r in ctx.view.out_pred(sid, scc_core::predicates::TESTED_BY) {
            tests.entry(r.object.clone()).or_default().insert("direct");
        }
        for pred in [
            scc_core::predicates::HANDLES,
            scc_core::predicates::IMPLEMENTS,
        ] {
            for r in ctx.view.out_pred(sid, pred) {
                let Some(target) = ctx.view.entity(&r.object) else {
                    continue;
                };
                if target.kind != kinds::CONTRACT && target.kind != kinds::ROUTE {
                    continue;
                }
                for t in ctx
                    .view
                    .out_pred(&r.object, scc_core::predicates::TESTED_BY)
                {
                    tests
                        .entry(t.object.clone())
                        .or_default()
                        .insert("contract");
                }
            }
        }
    }
    let affected_files: BTreeSet<String> = affected_syms
        .iter()
        .filter_map(|sid| {
            ctx.view
                .entity(sid.as_str())
                .and_then(|e| e.attributes.get("file"))
                .and_then(|v| v.as_str())
                .map(|f| f.to_string())
        })
        .collect();
    if !affected_files.is_empty() {
        for (id, _name, file, _kind, _sym) in ctx.store.tests().unwrap_or_default() {
            let imports = ctx.store.imports_in_file(&file).unwrap_or_default();
            let hits_affected = imports.iter().any(|(module, _names, _line, _typ)| {
                let target = resolve_module_ref(&file, module);
                affected_files.iter().any(|f| {
                    *f == target
                        || f.starts_with(&format!("{target}."))
                        || *f == format!("{target}/__init__.py")
                })
            });
            if hits_affected {
                tests.entry(id).or_default().insert("import");
            }
        }
    }
    let mut state_ids: BTreeSet<String> = BTreeSet::new();
    for sid in owned_stores {
        if ctx.view.entity(sid).map(|e| e.kind.as_str()) == Some(kinds::STATE) {
            state_ids.insert(sid.clone());
        }
    }
    for cid in affected_comps {
        for r in ctx.view.out_pred(cid, scc_core::predicates::OWNS) {
            if ctx.view.entity(&r.object).map(|e| e.kind.as_str()) == Some(kinds::STATE) {
                state_ids.insert(r.object.clone());
            }
        }
    }
    if !state_ids.is_empty() {
        for (id, _name, _file, _kind, _sym) in ctx.store.tests().unwrap_or_default() {
            let reads = ctx
                .view
                .out_pred(&id, scc_core::predicates::READS)
                .iter()
                .any(|r| state_ids.contains(&r.object));
            let writes = ctx
                .view
                .out_pred(&id, scc_core::predicates::WRITES)
                .iter()
                .any(|r| state_ids.contains(&r.object));
            if reads || writes {
                tests.entry(id).or_default().insert("state");
            }
        }
    }
    tests
}

// trace:exempt reason=internal-detail
fn format_test_to_run(
    ctx: &crate::ContextCompiler,
    tid: &str,
    reasons: &BTreeSet<&'static str>,
) -> String {
    let name = entity_name(&ctx.view, tid);
    let file = ctx
        .view
        .entity(tid)
        .and_then(|e| e.attributes.get("file"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let why = reasons.iter().copied().collect::<Vec<_>>().join(", ");
    if file.is_empty() {
        format!("- {name} — {why}\n")
    } else {
        format!("- {name} ({file}) — {why}\n")
    }
}

/// Resolve a module specifier (relative or dotted) to a repo-relative path
/// prefix, mirroring the indexer's import normalization.
fn resolve_module_ref(from_file: &str, module: &str) -> String {
    if !module.starts_with('.') {
        return module.replace('.', "/");
    }
    let dir = from_file.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let joined = if dir.is_empty() {
        module.to_string()
    } else {
        format!("{dir}/{module}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => continue,
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn flow_kind_str(k: scc_core::FlowKind) -> &'static str {
    match k {
        scc_core::FlowKind::Architecture => "architecture",
        scc_core::FlowKind::Workflow => "workflow",
        scc_core::FlowKind::Sequence => "sequence",
        scc_core::FlowKind::Dataflow => "dataflow",
        scc_core::FlowKind::Lifecycle => "lifecycle",
    }
}

fn severity_str(s: Severity) -> &'static str {
    match s {
        Severity::Info => "info",
        Severity::Low => "low",
        Severity::Medium => "medium",
        Severity::High => "high",
        Severity::Critical => "critical",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scc_core::{Entity, Provenance, Relationship};
    use scc_store::Store;

    #[test]
    // trace:v1 id=test.scc.context.tests-to-run verifies=REQ-tests-to-run-reasons exercises=impl.scc.context.tests-to-run
    fn tests_to_run_records_direct_reason() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let sid = "s:fn".to_string();
        let tid = "t:test_fn".to_string();
        let mut se = Entity::new(&sid, kinds::SYMBOL, "handleList");
        se.attr("file", serde_json::json!("src/a.py"));
        store.insert_entity(&se, &["src/a.py".into()]).unwrap();
        let mut te = Entity::new(&tid, kinds::TEST, "test_handle_list");
        te.attr("file", serde_json::json!("tests/test_a.py"));
        store
            .insert_entity(&te, &["tests/test_a.py".into()])
            .unwrap();
        let rel = Relationship::new(
            "rel:tb",
            sid.clone(),
            scc_core::predicates::TESTED_BY,
            tid.clone(),
            Provenance::Extracted,
        );
        store.insert_relationship(&rel, "src/a.py").unwrap();
        let graph = scc_graph::RealityGraph::load(&store).unwrap();
        let ctx = crate::ContextCompiler::new(
            &store,
            &graph,
            crate::ContextSettings::default(),
            Vec::new(),
        );
        let affected: HashSet<&String> = [&sid].into_iter().collect();
        let found = collect_tests_to_run(&ctx, &affected, &BTreeSet::new(), &BTreeSet::new());
        let reasons = found
            .get(&tid)
            .expect("direct TESTED_BY must list the test");
        assert!(reasons.contains("direct"));
        let line = format_test_to_run(&ctx, &tid, reasons);
        assert!(line.contains("direct"), "{line}");
        assert!(line.contains("test_handle_list"), "{line}");
        assert!(line.contains("tests/test_a.py"), "{line}");
    }

    #[test]
    // trace:v1 id=test.scc.context.tests-to-run-import verifies=REQ-tests-to-run-reasons exercises=impl.scc.context.tests-to-run
    fn tests_to_run_records_import_reason() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let sid = "s:fn".to_string();
        let tid = "t:import_test".to_string();
        let mut se = Entity::new(&sid, kinds::SYMBOL, "handleList");
        se.attr("file", serde_json::json!("src/a.py"));
        store.insert_entity(&se, &["src/a.py".into()]).unwrap();
        let mut te = Entity::new(&tid, kinds::TEST, "test_via_import");
        te.attr("file", serde_json::json!("tests/test_a.py"));
        store
            .insert_entity(&te, &["tests/test_a.py".into()])
            .unwrap();
        store
            .insert_test(&tid, "test_via_import", "tests/test_a.py", "unit", None)
            .unwrap();
        store
            .insert_imports(
                "tests/test_a.py",
                &[("src.a".into(), vec![], 1, "module".into())],
            )
            .unwrap();
        let graph = scc_graph::RealityGraph::load(&store).unwrap();
        let ctx = crate::ContextCompiler::new(
            &store,
            &graph,
            crate::ContextSettings::default(),
            Vec::new(),
        );
        let affected: HashSet<&String> = [&sid].into_iter().collect();
        let found = collect_tests_to_run(&ctx, &affected, &BTreeSet::new(), &BTreeSet::new());
        let reasons = found
            .get(&tid)
            .expect("import of affected file must list the test");
        assert!(reasons.contains("import"));
        assert!(!reasons.contains("direct"));
    }

    #[test]
    // trace:v1 id=test.scc.context.tests-to-run-contract verifies=REQ-tests-to-run-reasons exercises=impl.scc.context.tests-to-run
    fn tests_to_run_records_contract_reason() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let sid = "s:handler".to_string();
        let cid = "c:list".to_string();
        let tid = "t:contract".to_string();
        let mut se = Entity::new(&sid, kinds::SYMBOL, "handleList");
        se.attr("file", serde_json::json!("src/a.py"));
        store.insert_entity(&se, &["src/a.py".into()]).unwrap();
        store
            .insert_entity(
                &Entity::new(&cid, kinds::CONTRACT, "GET /list"),
                &["src/a.py".into()],
            )
            .unwrap();
        let mut te = Entity::new(&tid, kinds::TEST, "test_list_route");
        te.attr("file", serde_json::json!("tests/test_routes.py"));
        store
            .insert_entity(&te, &["tests/test_routes.py".into()])
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:h",
                    sid.clone(),
                    scc_core::predicates::HANDLES,
                    cid.clone(),
                    Provenance::Extracted,
                ),
                "src/a.py",
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:ct",
                    cid.clone(),
                    scc_core::predicates::TESTED_BY,
                    tid.clone(),
                    Provenance::Extracted,
                ),
                "tests/test_routes.py",
            )
            .unwrap();
        let graph = scc_graph::RealityGraph::load(&store).unwrap();
        let ctx = crate::ContextCompiler::new(
            &store,
            &graph,
            crate::ContextSettings::default(),
            Vec::new(),
        );
        let affected: HashSet<&String> = [&sid].into_iter().collect();
        let found = collect_tests_to_run(&ctx, &affected, &BTreeSet::new(), &BTreeSet::new());
        let reasons = found
            .get(&tid)
            .expect("contract TESTED_BY must list the test");
        assert!(reasons.contains("contract"));
        assert!(!reasons.contains("direct"));
    }

    #[test]
    // trace:v1 id=test.scc.context.tests-to-run-state verifies=REQ-tests-to-run-reasons exercises=impl.scc.context.tests-to-run
    fn tests_to_run_records_state_reason() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let comp = "comp:orders".to_string();
        let st = "state:orders_db".to_string();
        let tid = "t:state".to_string();
        store
            .insert_entity(
                &Entity::new(&comp, kinds::COMPONENT, "orders"),
                &["src/a.py".into()],
            )
            .unwrap();
        store
            .insert_entity(
                &Entity::new(&st, kinds::STATE, "orders_db"),
                &["src/a.py".into()],
            )
            .unwrap();
        let mut te = Entity::new(&tid, kinds::TEST, "test_orders_state");
        te.attr("file", serde_json::json!("tests/test_state.py"));
        store
            .insert_entity(&te, &["tests/test_state.py".into()])
            .unwrap();
        store
            .insert_test(
                &tid,
                "test_orders_state",
                "tests/test_state.py",
                "unit",
                None,
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:owns",
                    comp.clone(),
                    scc_core::predicates::OWNS,
                    st.clone(),
                    Provenance::Extracted,
                ),
                "src/a.py",
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:rd",
                    tid.clone(),
                    scc_core::predicates::READS,
                    st.clone(),
                    Provenance::Extracted,
                ),
                "tests/test_state.py",
            )
            .unwrap();
        let graph = scc_graph::RealityGraph::load(&store).unwrap();
        let ctx = crate::ContextCompiler::new(
            &store,
            &graph,
            crate::ContextSettings::default(),
            Vec::new(),
        );
        let comps: BTreeSet<String> = [comp].into_iter().collect();
        let found = collect_tests_to_run(&ctx, &HashSet::new(), &comps, &BTreeSet::new());
        let reasons = found.get(&tid).expect("state READS must list the test");
        assert!(reasons.contains("state"));
    }

    #[test]
    // trace:v1 id=test.scc.context.stack-locus verifies=REQ-stack-locus-ingest exercises=impl.scc.context.stack-locus
    fn stack_locus_seeds_file_and_innermost_symbol() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/app.py"), "def inner():\n    x = 1\n").unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let fe = Entity::new("f:app", kinds::FILE, "src/app.py");
        store.insert_entity(&fe, &["src/app.py".into()]).unwrap();
        let mut outer = Entity::new("s:outer", kinds::SYMBOL, "outer");
        outer.attr("file", serde_json::json!("src/app.py"));
        outer.attr("start_line", serde_json::json!(1u32));
        outer.attr("end_line", serde_json::json!(20u32));
        store.insert_entity(&outer, &["src/app.py".into()]).unwrap();
        let mut inner = Entity::new("s:inner", kinds::SYMBOL, "inner");
        inner.attr("file", serde_json::json!("src/app.py"));
        inner.attr("start_line", serde_json::json!(10u32));
        inner.attr("end_line", serde_json::json!(12u32));
        store.insert_entity(&inner, &["src/app.py".into()]).unwrap();
        let graph = scc_graph::RealityGraph::load(&store).unwrap();
        let ctx = crate::ContextCompiler::new(
            &store,
            &graph,
            crate::ContextSettings::default(),
            Vec::new(),
        );
        let goal = "Traceback (most recent call last):\n  File \"src/app.py\", line 11, in inner\nValueError: boom\n";
        let hits = resolve_goal_loci(&ctx, goal).expect("locus");
        assert!(hits.files.iter().any(|f| f == "src/app.py"));
        assert_eq!(hits.symbols, vec!["inner".to_string()]);
        assert!(hits.body.contains("inner"));
        let miss = resolve_goal_loci(
            &ctx,
            "Traceback (most recent call last):\n  File \"src/missing.py\", line 1, in x\nValueError: x\n",
        )
        .unwrap();
        assert!(miss.body.contains("unmapped"));
        assert!(miss.files.is_empty());
    }

    #[test]
    // trace:v1 id=test.scc.context.finish-rollover verifies=REQ-budget-rollover exercises=impl.scc.context.finish-rollover
    fn finish_with_rollover_discloses_truncated_quota() {
        let mut pack = ContextPack::new("task", "rev");
        let huge = "word ".repeat(4000);
        let sections = vec![
            Section::new("TASK", "goal\n".into(), 10),
            Section::new("IMPLEMENTATION", huge, 7),
        ];
        finish_with_rollover(&mut pack, sections, 200, Vec::new());
        assert!(
            pack.dropped_sections.iter().any(|d| d == "quota:source"),
            "{:?}",
            pack.dropped_sections
        );
        assert!(pack.truncated);
        assert!(pack.content.contains("truncated") || pack.hard_truncated);
    }

    #[test]
    // trace:v1 id=test.scc.context.impact-forgotten verifies=REQ-forgotten-impact-partners exercises=impl.scc.context.impact-forgotten
    fn impact_pack_discloses_forgotten_partners_without_merging_them() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("src")).unwrap();
        fn git(root: &std::path::Path, args: &[&str]) {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{args:?} {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        git(&root, &["init", "-q"]);
        git(&root, &["config", "user.email", "test@example.com"]);
        git(&root, &["config", "user.name", "SCC Test"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        for i in 0..2 {
            std::fs::write(root.join("src/a.py"), format!("a = {i}\n")).unwrap();
            std::fs::write(root.join("src/b.py"), format!("b = {i}\n")).unwrap();
            git(&root, &["add", "-A"]);
            git(&root, &["commit", "-q", "-m", &format!("c{i}")]);
        }
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let graph = scc_graph::RealityGraph::load(&store).unwrap();
        let ctx = crate::ContextCompiler::new(
            &store,
            &graph,
            crate::ContextSettings::default(),
            Vec::new(),
        );
        let pack = impact(&ctx, &["src/a.py".into()], &[], None);
        assert!(
            pack.content.contains("FORGOTTEN PARTNERS"),
            "missing section: {}",
            pack.content
        );
        assert!(pack.content.contains("src/b.py"), "{}", pack.content);
        assert!(pack.content.contains("cochange"), "{}", pack.content);
        assert!(
            !pack.content.contains("AFFECTED COMPONENTS\nsrc/b.py"),
            "partner must not become affected: {}",
            pack.content
        );
        assert!(pack.content.contains("historical; not EXTRACTED impact"));
    }

    #[test]
    // trace:v1 id=test.scc.context.exact-source-last verifies=REQ-exact-source-dominance exercises=impl.scc.context.exact-source-last
    fn task_pack_puts_exact_source_last_and_drops_it_first() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("src")).unwrap();
        let src = "def handle_list():\n    return [1, 2, 3]\n";
        std::fs::write(root.join("src/a.py"), src).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let mut se = Entity::new("s:fn", kinds::SYMBOL, "handle_list");
        se.attr("file", serde_json::json!("src/a.py"));
        store.insert_entity(&se, &["src/a.py".into()]).unwrap();
        let graph = scc_graph::RealityGraph::load(&store).unwrap();
        let ctx = crate::ContextCompiler::new(
            &store,
            &graph,
            crate::ContextSettings::default(),
            Vec::new(),
        );
        let fat = task(&ctx, "handle list", &["src/a.py".into()], &[], 50_000);
        let idx_exact = fat
            .content
            .find("EXACT SOURCE")
            .expect("exact source must appear when budget allows");
        let idx_task = fat.content.find("TASK").expect("task header");
        assert!(
            idx_exact > idx_task,
            "exact source must follow semantic sections"
        );
        assert!(
            fat.content.contains("shown=2 total=2 capped=0"),
            "{}",
            fat.content
        );
        assert!(fat.content.contains("def handle_list()"), "{}", fat.content);
        let thin = task(&ctx, "handle list", &["src/a.py".into()], &[], 80);
        assert!(
            thin.dropped_sections.iter().any(|s| s == "EXACT SOURCE"),
            "exact source is priority 1 and must drop first: {:?}",
            thin.dropped_sections
        );
        assert!(
            !thin.content.contains("EXACT SOURCE"),
            "dropped exact source must not remain in content: {}",
            thin.content
        );
    }
}
