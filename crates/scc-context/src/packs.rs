//! Context pack builders (docs/CONTEXT_COMPILER.md §8, §9).
//!
//! Section priority contract: invariants (10), ownership (10), directly
//! affected contracts (9), known failure/retry behavior (9), stale warnings
//! (always appended) may never be cut for budget. Lower-priority sections
//! are dropped before truncation.

use crate::rank::{collect_lexical_candidates, terms};
use crate::{ContextCompiler, ContextPack};
use scc_core::kinds;
use scc_core::{entity_id, estimate_tokens, Provenance, Severity};
use scc_graph::RealityGraph;
use std::collections::{BTreeSet, HashMap, HashSet};

struct Section {
    title: String,
    body: String,
    /// 10 = never cut, 9 = never cut, 5 = lowest
    priority: u8,
}

impl Section {
    fn new(title: &str, body: String, priority: u8) -> Section {
        Section {
            title: title.to_string(),
            body,
            priority,
        }
    }
}

fn render(sections: Vec<Section>, budget: usize, warnings: Vec<String>) -> String {
    let mut sections = sections;
    // assemble; then drop lowest-priority sections while over budget
    let mut content = assemble(&sections);
    let mut tokens = estimate_tokens(&content);
    while tokens > budget {
        // find the lowest-priority droppable section (priority < 9)
        let min_priority = sections
            .iter()
            .map(|s| s.priority)
            .min()
            .unwrap_or(10);
        if min_priority >= 9 {
            break; // cannot drop anything else
        }
        let idx = sections
            .iter()
            .position(|s| s.priority == min_priority)
            .unwrap();
        sections.remove(idx);
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
    if estimate_tokens(&content) > budget {
        content = scc_core::truncate_to_budget(&content, budget);
    }
    content
}

fn assemble(sections: &[Section]) -> String {
    let mut out = String::new();
    for s in sections {
        out.push_str(&format!("# {}\n{}\n\n", s.title, s.body));
    }
    out.trim_end().to_string()
}

fn entity_name(graph: &RealityGraph, id: &str) -> String {
    graph
        .entities
        .get(id)
        .map(|e| e.name.clone())
        .unwrap_or_else(|| {
            id.rsplit('/').next().unwrap_or(id).to_string()
        })
}

fn component_short(graph: &RealityGraph, id: &str) -> String {
    let name = entity_name(graph, id);
    let name = name.strip_prefix("component:").unwrap_or(&name);
    name.to_string()
}

fn format_evidence_tags(
    ctx: &ContextCompiler,
    entity_ids: &[String],
) -> String {
    let counts = ctx.evidence_summary(entity_ids);
    if counts.is_empty() {
        return String::new();
    }
    let mut parts: Vec<String> = counts
        .iter()
        .map(|(k, v)| format!("{v} {k}"))
        .collect();
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
        .graph
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
        .graph
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
        .graph
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
        .graph
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
        .graph
        .flows
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
    pack.content = render(sections, ctx.settings.startup_tokens, warnings);
    pack.tokens = estimate_tokens(&pack.content);
    pack.budget = ctx.settings.startup_tokens;
    pack.truncated = pack.tokens > pack.budget;
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
    let mut pack = ContextPack::new("task", &ctx.revision());
    let goal_terms = terms(goal);

    // ---- candidate generation ----
    let candidates = collect_lexical_candidates(ctx.store, ctx.graph, goal, symbols, 24);
    let entity_ids: Vec<String> = candidates.iter().map(|c| c.id.clone()).collect();

    // symbol -> file -> component
    let mut symbol_files: HashMap<String, String> = HashMap::new();
    for e in ctx.graph.entities_of_kind(kinds::SYMBOL) {
        if let Some(f) = e.attributes.get("file").and_then(|v| v.as_str()) {
            symbol_files.insert(e.id.clone(), f.to_string());
        }
    }
    // component containing each file
    let mut file_component: HashMap<String, String> = HashMap::new();
    for c in ctx.store.components().unwrap_or_default() {
        for r in ctx.graph.out_pred(&c.id, scc_core::predicates::CONTAINS) {
            file_component.insert(r.object.clone(), c.id.clone());
        }
    }
    // symbol -> component
    let mut symbol_component: HashMap<String, String> = HashMap::new();
    for (sid, f) in &symbol_files {
        if let Some(cid) = file_component
            .get(&entity_id(&ctx.graph.repo_id, kinds::FILE, f))
        {
            symbol_component.insert(sid.clone(), cid.clone());
        }
    }

    // affected components = candidates' components + file args' components
    let mut affected_comps: BTreeSet<String> = BTreeSet::new();
    for c in &candidates {
        if c.kind == kinds::SYMBOL {
            if let Some(cid) = symbol_component.get(&c.id) {
                affected_comps.insert(cid.clone());
            }
        } else if c.kind == kinds::COMPONENT {
            affected_comps.insert(c.id.clone());
        } else if c.kind == kinds::FILE {
            if let Some(cid) = file_component.get(&c.id) {
                affected_comps.insert(cid.clone());
            }
        } else if c.kind == kinds::ROUTE {
            if let Some(h) = ctx
                .graph
                .entities
                .get(&c.id)
                .and_then(|e| e.attributes.get("handler"))
                .and_then(|v| v.as_str())
            {
                if let Some(cid) = symbol_component.get(h) {
                    affected_comps.insert(cid.clone());
                }
            }
        }
    }
    for f in files {
        let fid = entity_id(&ctx.graph.repo_id, kinds::FILE, f);
        if let Some(cid) = file_component.get(&fid) {
            affected_comps.insert(cid.clone());
        }
    }

    // flows mentioning affected components
    let mut affected_flows: Vec<(String, f64)> = Vec::new();
    for f in &ctx.graph.flows {
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
        for r in ctx.graph.out_pred(cid, scc_core::predicates::DEPENDS_ON) {
            downstream.insert(r.object.clone());
        }
        for r in ctx.graph.in_pred(cid, scc_core::predicates::DEPENDS_ON) {
            upstream.insert(r.subject.clone());
        }
    }

    // contracts: routes handled by symbols in affected comps
    let mut contracts: BTreeSet<String> = BTreeSet::new();
    let affected_syms: HashSet<&String> = symbol_component
        .iter()
        .filter(|(_, c)| affected_comps.contains(*c))
        .map(|(s, _)| s)
        .collect();
    for sid in &affected_syms {
        for r in ctx.graph.out_pred(sid, scc_core::predicates::HANDLES) {
            contracts.insert(r.object.clone());
        }
    }

    // ownership: stores owned by affected comps
    let mut owned_stores: BTreeSet<String> = BTreeSet::new();
    for cid in &affected_comps {
        for r in ctx.graph.out_pred(cid, scc_core::predicates::OWNS) {
            owned_stores.insert(r.object.clone());
        }
    }

    // invariants scoped to affected entities
    let mut inv_ids: BTreeSet<String> = BTreeSet::new();
    for inv in &ctx.graph.invariants {
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

    // tests exercising affected symbols, plus tests whose file imports an
    // affected file (the test may exercise behavior through imports that
    // token-matching misses)
    let mut tests: BTreeSet<String> = BTreeSet::new();
    for sid in &affected_syms {
        for r in ctx.graph.out_pred(sid, scc_core::predicates::TESTED_BY) {
            tests.insert(r.object.clone());
        }
    }
    {
        let affected_files: BTreeSet<String> = affected_syms
            .iter()
            .filter_map(|sid| {
                ctx.graph
                    .entities
                    .get(sid.as_str())
                    .and_then(|e| e.attributes.get("file"))
                    .and_then(|v| v.as_str())
                    .map(|f| f.to_string())
            })
            .collect();
        for (id, _name, file, _kind, _sym) in ctx.store.tests().unwrap_or_default() {
            if affected_files.is_empty() {
                break;
            }
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
                tests.insert(id.clone());
            }
        }
    }

    // retries/failures in affected components
    let mut retries: Vec<String> = Vec::new();
    for cid in &affected_comps {
        if let Some(c) = ctx.graph.entities.get(cid) {
            if let Some(rs) = c.attributes.get("retries").and_then(|v| v.as_array()) {
                for r in rs {
                    if let Some(s) = r.as_str() {
                        retries.push(format!("{s} [in {}]", entity_name(ctx.graph, cid)));
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
        format!(
            "Goal: {goal}\nExplicit files: {files_disp}\nExplicit symbols: {symbols_disp}",
        ),
        10,
    ));

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
        if let Some(c) = ctx.graph.entities.get(cid) {
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
            if let Some(f) = ctx.graph.flows.iter().find(|f| &f.id == fid) {
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
                    .map(|c| component_short(ctx.graph, c))
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
                    .map(|c| component_short(ctx.graph, c))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        },
        9,
    ));

    // DATA OWNERSHIP
    let mut data_body = String::new();
    for store_id in &owned_stores {
        let name = entity_name(ctx.graph, store_id);
        let writers: Vec<String> = ctx
            .graph
            .in_pred(store_id, scc_core::predicates::WRITES)
            .into_iter()
            .map(|r| {
                let comp = symbol_component.get(&r.subject).cloned();
                comp.map(|c| component_short(ctx.graph, &c))
                    .unwrap_or_else(|| entity_name(ctx.graph, &r.subject))
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
        if let Some(r) = ctx.graph.entities.get(rid) {
            contract_body.push_str(&format!("- {}\n", r.name));
        }
    }
    if !contract_body.is_empty() {
        sections.push(Section::new("CONTRACTS", contract_body, 9));
    }

    // INVARIANTS
    let mut inv_body = String::new();
    for id in &inv_ids {
        if let Some(inv) = ctx.graph.invariants.iter().find(|i| &i.id == id) {
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
        if let Some(c) = ctx.graph.entities.get(cid) {
            if let Some(paths) = c
                .attributes
                .get("implementation")
                .and_then(|i| i.get("paths"))
                .and_then(|p| p.as_array())
            {
                let ps: Vec<&str> = paths
                    .iter()
                    .filter_map(|p| p.as_str())
                    .collect();
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
                .graph
                .entities
                .get(&c.id)
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

    // TESTS
    let mut test_body = String::new();
    for tid in &tests {
        test_body.push_str(&format!("- {}\n", entity_name(ctx.graph, tid)));
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
            if let Some(e) = ctx.graph.entities.get(&c.id) {
                if let Some(f) = e.attributes.get("file").and_then(|v| v.as_str()) {
                    ids.push(entity_id(&ctx.graph.repo_id, kinds::FILE, f));
                }
            }
        }
    }
    ids.extend(downstream.iter().cloned());
    ids.extend(tests.iter().cloned());
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
    pack.content = render(sections, budget, all_warnings);
    pack.budget = budget;
    pack.tokens = estimate_tokens(&pack.content);
    pack.truncated = pack.tokens > budget;
    pack.compression_policy = Some(compression_policy(goal));
    pack
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

pub fn component(ctx: &ContextCompiler, id_or_name: &str) -> ContextPack {
    let mut pack = ContextPack::new("component", &ctx.revision());
    let comp = ctx
        .store
        .components()
        .unwrap_or_default()
        .into_iter()
        .find(|c| c.id == id_or_name || c.name == id_or_name);
    let Some(comp) = comp else {
        pack.content = format!(
            "# COMPONENT NOT FOUND\nNo component matches '{id_or_name}'.\n"
        );
        pack.warnings.push(format!("unknown component: {id_or_name}"));
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
        if resp.is_empty() { "(none)".into() } else { format!("{}\n", resp.join("\n")) },
        10,
    ));

    let paths: Vec<String> = comp
        .attributes
        .get("implementation")
        .and_then(|i| i.get("paths"))
        .and_then(|p| p.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    let symbols: Vec<String> = comp
        .attributes
        .get("implementation")
        .and_then(|i| i.get("symbols"))
        .and_then(|p| p.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
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
                .filter_map(|x| x.as_str().map(|s| entity_name(ctx.graph, s)))
                .collect()
        })
        .unwrap_or_default();
    sections.push(Section::new(
        "OWNS",
        if owned.is_empty() { "(none)".into() } else { format!("{}\n", owned.join(", ")) },
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
        if deps.is_empty() { "(none)".into() } else { format!("{}\n", deps.join("\n")) },
        8,
    ));

    let retries: Vec<String> = comp
        .attributes
        .get("retries")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
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
    for f in &ctx.graph.flows {
        if f.steps.iter().any(|s| s.actor.contains(&comp.id)) {
            flows.push(format!("- {} [{}]", f.name, f.trigger.clone().unwrap_or_default()));
        }
    }
    if !flows.is_empty() {
        sections.push(Section::new("FLOWS", format!("{}\n", flows.join("\n")), 8));
    }

    // tests
    let mut tests: Vec<String> = Vec::new();
    for sym in &symbols {
        for e in ctx.graph.entities_of_kind(kinds::SYMBOL) {
            if e.name == *sym {
                for r in ctx.graph.out_pred(&e.id, scc_core::predicates::TESTED_BY) {
                    tests.push(entity_name(ctx.graph, &r.object));
                }
            }
        }
    }
    tests.sort();
    tests.dedup();
    if !tests.is_empty() {
        sections.push(Section::new(
            "TESTS",
            format!("{}\n", tests.join("\n")),
            7,
        ));
    }

    sections.push(Section::new(
        "EVIDENCE",
        format_evidence_tags(ctx, std::slice::from_ref(&comp.id)),
        5,
    ));

    pack.content = render(sections, usize::MAX, ctx_warnings(ctx));
    pack.tokens = estimate_tokens(&pack.content);
    pack
}

// ---------------------------------------------------------------------------
// flow_context
// ---------------------------------------------------------------------------

fn render_flow(ctx: &ContextCompiler, fid: &str, compact: bool) -> String {
    let Some(f) = ctx.graph.flows.iter().find(|f| f.id == fid) else {
        return format!("(flow {fid} not found)");
    };
    let mut body = String::new();
    if let Some(t) = &f.trigger {
        body.push_str(&format!("Trigger: {t}\n"));
    }
    let mut prev_actor: Option<String> = None;
    for s in &f.steps {
        let actor = component_short(ctx.graph, &s.actor);
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
            let parts: Vec<String> = ev_tags
                .iter()
                .map(|(k, v)| format!("{v} {k}"))
                .collect();
            body.push_str(&parts.join(", "));
        }
        body.push('\n');
    }
    body
}

pub fn flow(ctx: &ContextCompiler, id_or_name: &str) -> ContextPack {
    let mut pack = ContextPack::new("flow", &ctx.revision());
    let f = ctx
        .graph
        .flows
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

    pack.content = render(sections, usize::MAX, ctx_warnings(ctx));
    pack.tokens = estimate_tokens(&pack.content);
    pack
}

// ---------------------------------------------------------------------------
// impact_context
// ---------------------------------------------------------------------------

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

    let imp = match scc_graph::impact::compute_impact(ctx.graph, ctx.store, &files, &symbols) {
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
                    .map(|c| component_short(ctx.graph, c))
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
                        ctx.graph
                            .flows
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
                    .map(|c| component_short(ctx.graph, c))
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
                    .map(|c| component_short(ctx.graph, c))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        },
        9,
    ));

    let contracts: Vec<String> = imp
        .contracts
        .iter()
        .map(|c| entity_name(ctx.graph, c))
        .collect();
    sections.push(Section::new(
        "CONTRACTS",
        if contracts.is_empty() { "(none)".into() } else { format!("{}\n", contracts.join(", ")) },
        9,
    ));

    let data: Vec<String> = imp
        .data
        .iter()
        .map(|d| entity_name(ctx.graph, d))
        .collect();
    sections.push(Section::new(
        "DATA",
        if data.is_empty() { "(none)".into() } else { format!("{}\n", data.join(", ")) },
        9,
    ));

    let invs: Vec<String> = imp
        .invariants
        .iter()
        .map(|i| {
            ctx.graph
                .invariants
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
        .map(|t| entity_name(ctx.graph, t))
        .collect();
    sections.push(Section::new(
        "TESTS",
        if tests.is_empty() { "(none)".into() } else { format!("{}\n", tests.join(", ")) },
        7,
    ));

    sections.push(Section::new("RISK", format!("{}\n", imp.risk.to_uppercase()), 10));
    if !imp.notes.is_empty() {
        sections.push(Section::new(
            "NOTES",
            format!("{}\n", imp.notes.join("\n")),
            8,
        ));
    }

    pack.entity_ids = imp.components.clone();
    pack.content = render(sections, usize::MAX, ctx_warnings(ctx));
    pack.tokens = estimate_tokens(&pack.content);
    pack
}

// ---------------------------------------------------------------------------
// verify_context
// ---------------------------------------------------------------------------

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
    for r in ctx.graph.all_rels() {
        // endpoints may be external_api, component, flow ids — check known
        // namespaces
        let known = |id: &str| -> bool {
            ctx.graph.entities.contains_key(id)
                || id.contains("/external_api/")
                || id.contains("/component/")
                || id.contains("/flow/")
                || id.contains("/invariant/")
        };
        if !known(&r.subject) {
            dangling += 1;
            if dangling <= 5 {
                inv.push_str(&format!("dangling subject: {} — {}\n", r.subject, r.predicate));
            }
        }
        if !known(&r.object) {
            dangling += 1;
            if dangling <= 5 {
                inv.push_str(&format!("dangling object: {} — {}\n", r.predicate, r.object));
            }
        }
    }
    inv.push_str(&format!("Dangling references: {dangling}\n"));
    // 2. RESOLVED facts must have evidence
    let mut no_evidence = 0usize;
    for r in ctx.graph.all_rels() {
        if r.provenance == Provenance::Resolved && r.evidence.is_empty() {
            no_evidence += 1;
        }
    }
    inv.push_str(&format!(
        "RESOLVED facts without evidence: {no_evidence}\n"
    ));
    // 3. critical invariants unenforced
    let unenforced = ctx
        .graph
        .invariants
        .iter()
        .filter(|i| i.severity == Severity::Critical && i.enforced_by.is_empty())
        .count();
    inv.push_str(&format!("Critical invariants without enforcing tests: {unenforced}\n"));
    // 4. inferred claims
    let inferred = ctx
        .graph
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
    for r in ctx.graph.all_rels() {
        if r.predicate == scc_core::predicates::DEPENDS_ON && r.confidence < 0.8 {
            lc += 1;
            if lc <= 8 {
                low_conf.push_str(&format!(
                    "- {} → {} ({:.2})\n",
                    component_short(ctx.graph, &r.subject),
                    component_short(ctx.graph, &r.object),
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
    if let Ok(crossings) = scc_graph::boundaries::boundary_crossings(ctx.graph, ctx.store) {
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

    pack.content = render(sections, usize::MAX, Vec::new());
    pack.tokens = estimate_tokens(&pack.content);
    pack
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
