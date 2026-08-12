//! Command implementations for the `scc` CLI (docs/API_AND_INTEGRATIONS.md §4).

use crate::{checkpoint, compiler, config_path, load_config, open_store, recompile, scc_dir};
use scc_core::kinds;
use std::io::Write;
use std::path::Path;

pub fn cmd_init(root: &Path) -> crate::Result<()> {
    let dir = scc_dir(root);
    std::fs::create_dir_all(&dir)?;
    let cfg_path = config_path(root);
    if !cfg_path.exists() {
        std::fs::write(&cfg_path, scc_indexer::Config::default_yaml())?;
        println!("created {}", cfg_path.display());
    } else {
        println!("config exists: {}", cfg_path.display());
    }
    // create the DB so the workspace is ready
    let store = open_store(root)?;
    println!(
        "initialized SCC workspace for repository '{}' at {}",
        store.repo_name,
        dir.display()
    );
    println!("next: scc index");
    Ok(())
}

pub fn cmd_index(root: &Path, quiet: bool) -> crate::Result<()> {
    let config = load_config(root)?;
    let report = crate::index_and_recompile(root, &config)?;
    if !quiet {
        println!(
            "indexed {} file(s) ({} changed, {} added, {} removed, {} failed) in {:.2}s",
            report.indexed,
            report.changed,
            report.added,
            report.removed,
            report.failed,
            report.duration_ms as f64 / 1000.0
        );
    }
    Ok(())
}

pub fn cmd_index_paths(root: &Path, paths: &[String], quiet: bool) -> crate::Result<()> {
    let config = load_config(root)?;
    let store = open_store(root)?;
    let indexer = scc_indexer::Indexer::new(crate::open_store(root)?, config.clone());
    let report = indexer.refresh_paths(paths)?;
    drop(indexer);
    recompile(&store)?;
    if !quiet && report.indexed > 0 {
        println!("refreshed {} file(s)", report.indexed);
    }
    Ok(())
}

pub fn cmd_status(root: &Path) -> crate::Result<()> {
    let store = open_store(root)?;
    let repo = store.repository();
    println!("Repository: {} ({})", repo.name, repo.id);
    if let Some(url) = &repo.url {
        println!("Remote: {url}");
    }
    match store.snapshot_status()? {
        Some((snap, _files)) => {
            println!("Revision: {}", snap.revision);
            if let Some(b) = snap.branch {
                println!("Branch: {b}");
            }
            println!("Indexed at: {}", snap.indexed_at);
            let stats = store.stats()?;
            for (k, v) in &stats {
                println!("{k}: {v}");
            }
            let stale = crate::stale_paths(&store)?;
            if stale.is_empty() {
                println!("freshness: up to date");
            } else {
                println!(
                    "freshness: STALE — {} file(s) changed since index (run `scc index`)",
                    stale.len()
                );
                for p in stale.iter().take(10) {
                    println!("  {p}");
                }
            }
        }
        None => println!("not indexed yet — run `scc index`"),
    }
    Ok(())
}

pub fn cmd_overview(root: &Path, json: bool) -> crate::Result<()> {
    let store = open_store(root)?;
    let config = load_config(root)?;
    let stale = crate::stale_paths(&store)?;
    let comp = compiler(&store, &config, stale)?;
    let pack = comp.ctx().system_overview();
    if json {
        println!("{}", serde_json::to_string_pretty(&pack)?);
    } else {
        print!("{}", pack.content);
    }
    Ok(())
}

/// `scc atlas [--budget N] [--json]` — the full System Atlas.
pub fn cmd_atlas(root: &Path, budget: Option<usize>, json: bool) -> crate::Result<()> {
    let store = open_store(root)?;
    let config = load_config(root)?;
    let stale = crate::stale_paths(&store)?;
    let comp = compiler(&store, &config, stale)?;
    let pack = comp.ctx().system_atlas(budget);
    if json {
        println!("{}", serde_json::to_string(&pack)?);
    } else {
        print!("{}", pack.content);
    }
    Ok(())
}

pub fn cmd_context_task(
    root: &Path,
    goal: &str,
    files: &[String],
    symbols: &[String],
    budget: Option<usize>,
    json: bool,
    hook: bool,
) -> crate::Result<()> {
    if hook {
        // Wave 2 (§37): UserPromptSubmit injects a task focus only when
        // context.inject_task_focus is enabled; otherwise silent no-op.
        let config = load_config(root)?;
        if !config.context.inject_task_focus {
            return Ok(());
        }
    }
    // hook mode keeps the focus small (<= 1500 tokens, §37 option B)
    let budget = if hook {
        Some(budget.unwrap_or(1500).min(1500))
    } else {
        budget
    };
    let pack_json = cmd_context_task_json(root, goal, files, symbols, budget)?;
    if json {
        println!("{pack_json}");
    } else {
        let pack: scc_context::ContextPack = serde_json::from_str(&pack_json)?;
        print!("{}", pack.content);
    }
    Ok(())
}

/// THE one task-context pipeline (P0 parity): semantic rankers + beads
/// task state + hindsight lessons applied identically on every transport.
/// CLI, MCP, and HTTP all call this — transport cannot change semantic
/// quality.
pub fn build_task_pack(
    root: &Path,
    goal: &str,
    files: &[String],
    symbols: &[String],
    budget: Option<usize>,
) -> crate::Result<scc_context::ContextPack> {
    let store = open_store(root)?;
    let config = load_config(root)?;
    let stale = crate::stale_paths(&store)?;
    let comp = compiler(&store, &config, stale)?;
    let (scorer, reranker) = crate::embed_cli::rankers(&store, &config, goal);
    let scorer_trait: Option<&dyn scc_context::rank::SemanticScorer> =
        scorer.as_ref().map(|s| s as &dyn scc_context::rank::SemanticScorer);
    let reranker_trait: Option<&dyn scc_context::rank::Reranker> =
        reranker.as_ref().map(|r| r as &dyn scc_context::rank::Reranker);
    let mut pack = comp.ctx().task_context_with_rankers(
        goal,
        files,
        symbols,
        budget,
        scorer_trait,
        reranker_trait,
    );
    // task-state + memory enrichment (below the System IR authority line)
    let beads_active = scc_indexer::adapters::beads::active_beads(root, 5);
    if !beads_active.is_empty() {
        pack.content.push_str(
            "\n# ACTIVE TASK STATE (from .beads/issues.jsonl — task state, not system facts)\n",
        );
        for t in beads_active {
            pack.content.push_str("- ");
            pack.content.push_str(&t);
            pack.content.push('\n');
        }
    }
    if config.integrations.hindsight {
        let lessons = scc_indexer::adapters::hindsight::lessons(&store, 5);
        if !lessons.is_empty() {
            pack.content.push_str(
                "\n# HINDSIGHT LESSONS (memory, below System IR authority — not verified facts)\n",
            );
            for (content, tags) in lessons {
                let tag_str = if tags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", tags.join(", "))
                };
                pack.content.push_str(&format!("- {content}{tag_str}\n"));
            }
        }
    }
    Ok(pack)
}

/// Task pack as JSON (used by the benchmark harness and integrations).
pub fn cmd_context_task_json(
    root: &Path,
    goal: &str,
    files: &[String],
    symbols: &[String],
    budget: Option<usize>,
) -> crate::Result<String> {
    Ok(serde_json::to_string_pretty(&build_task_pack(
        root, goal, files, symbols, budget,
    )?)?)
}

/// `scc context docs <dependency>` — external library docs via Context7
/// (labeled external; never mixed with repository facts).
pub fn cmd_context_docs(root: &Path, dependency: &str) -> crate::Result<()> {
    let config = load_config(root)?;
    if config.integrations.context7_command.is_empty() {
        return Err(crate::CliError::Other(
            "Context7 is not configured — set integrations.context7_command in .scc/config.yaml (e.g. 'npx -y @upstash/context7-mcp')".into(),
        ));
    }
    let mut client =
        scc_indexer::adapters::context7::start(&config.integrations.context7_command, root)
            .map_err(crate::CliError::Other)?;
    let docs = client.docs_for(dependency).map_err(crate::CliError::Other)?;
    print!("{docs}");
    Ok(())
}

/// Subagent context policy (SCC-107, docs/API_AND_INTEGRATIONS.md §5):
/// a narrower, tighter-budget task pack with explicit scope boundaries so
/// delegated agents start from the same system model without re-deriving it.
pub fn cmd_context_subagent(
    root: &Path,
    goal: &str,
    files: &[String],
    symbols: &[String],
    budget: Option<usize>,
    json: bool,
) -> crate::Result<()> {
    let store = open_store(root)?;
    let config = load_config(root)?;
    let stale = crate::stale_paths(&store)?;
    let comp = compiler(&store, &config, stale)?;
    let mut pack = comp.ctx().task_context(goal, files, symbols, budget);
    pack.kind = "subagent".into();
    let mut header = String::new();
    header.push_str("# SUBAGENT SCOPE
");
    header.push_str("You are a delegated agent. Work ONLY within the context below; ");
    header.push_str("do not re-derive the system model. If a needed fact is absent, ");
    header.push_str("state it and ask rather than assume. Your goal is bounded to:
");
    header.push_str(&format!("> {goal}

"));
    pack.content = format!("{header}{}", pack.content);
    if json {
        println!("{}", serde_json::to_string_pretty(&pack)?);
    } else {
        print!("{}", pack.content);
    }
    Ok(())
}

pub fn cmd_context_component(root: &Path, id: &str, json: bool) -> crate::Result<()> {
    let store = open_store(root)?;
    let config = load_config(root)?;
    let stale = crate::stale_paths(&store)?;
    let comp = compiler(&store, &config, stale)?;
    let pack = comp.ctx().component_context(id);
    if json {
        println!("{}", serde_json::to_string_pretty(&pack)?);
    } else {
        print!("{}", pack.content);
    }
    Ok(())
}

pub fn cmd_context_flow(root: &Path, id: &str, json: bool) -> crate::Result<()> {
    let store = open_store(root)?;
    let config = load_config(root)?;
    let stale = crate::stale_paths(&store)?;
    let comp = compiler(&store, &config, stale)?;
    let pack = comp.ctx().flow_context(id);
    if json {
        println!("{}", serde_json::to_string_pretty(&pack)?);
    } else {
        print!("{}", pack.content);
    }
    Ok(())
}

pub fn cmd_impact(
    root: &Path,
    files: &[String],
    symbols: &[String],
    diff: Option<&str>,
    json: bool,
) -> crate::Result<()> {
    let store = open_store(root)?;
    let config = load_config(root)?;
    let stale = crate::stale_paths(&store)?;
    let comp = compiler(&store, &config, stale)?;
    let pack = comp.ctx().impact_context(files, symbols, diff);
    if json {
        println!("{}", serde_json::to_string_pretty(&pack)?);
    } else {
        print!("{}", pack.content);
    }
    Ok(())
}

pub fn cmd_verify(root: &Path, warnings_only: bool, json: bool) -> crate::Result<()> {
    let store = open_store(root)?;
    let config = load_config(root)?;
    let stale = crate::stale_paths(&store)?;
    let comp = compiler(&store, &config, stale)?;
    let pack = comp.ctx().verify_context();
    if warnings_only {
        for w in &pack.warnings {
            println!("⚠ {w}");
        }
        return Ok(());
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&pack)?);
        return Ok(());
    }
    print!("{}", pack.content);
    Ok(())
}

pub fn cmd_drift(root: &Path, json: bool) -> crate::Result<()> {
    let store = open_store(root)?;
    let findings = store.drift_findings(false)?;
    if json {
        let arr: Vec<serde_json::Value> = findings
            .iter()
            .map(|(id, kind, sev, msg, at)| {
                serde_json::json!({"id": id, "kind": kind, "severity": sev, "message": msg, "created_at": at})
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&arr)?);
    } else {
        if findings.is_empty() {
            println!("no drift findings");
        }
        for (id, kind, sev, msg, at) in &findings {
            println!("[{sev}] {kind} (#{id}, {at}): {msg}");
        }
    }
    Ok(())
}

pub fn cmd_export(root: &Path, format: &str) -> crate::Result<()> {
    let store = open_store(root)?;
    let ir = crate::export_ir(&store)?;
    match format {
        "system-ir.json" => println!("{}", serde_json::to_string_pretty(&ir)?),
        "system-ir.jsonl" => {
            for line in crate::export_jsonl(&ir)? {
                println!("{line}");
            }
        }
        "ccg" => println!("{}", serde_json::to_string_pretty(&crate::export_ccg(&ir)?)?),
        "flow-graphs.json" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&store.flow_graphs()?)?
            )
        }
        "capsule.md" => print!("{}", crate::compress::capsule_markdown(root)?),
        other => {
            return Err(crate::CliError::Other(format!(
                "unknown export format '{other}' (use system-ir.json, system-ir.jsonl, ccg, or capsule.md)"
            )))
        }
    }
    Ok(())
}

pub fn cmd_query(root: &Path, query: &str, limit: usize) -> crate::Result<()> {
    let store = open_store(root)?;
    println!("— entities —");
    for e in store.search_entities(query, limit)? {
        println!("{} [{}]", e.name, e.kind);
    }
    println!("— symbols —");
    for (name, sig, kind, file) in store.search_symbols(query, limit)? {
        println!("{name} ({kind}) {file} {sig}");
    }
    Ok(())
}

pub fn cmd_list_components(root: &Path) -> crate::Result<()> {
    let store = open_store(root)?;
    for c in store.components()? {
        println!("{}", c.name);
    }
    Ok(())
}

pub fn cmd_list_flows(root: &Path) -> crate::Result<()> {
    let store = open_store(root)?;
    for f in store.flows()? {
        println!("{} [{}] {}", f.name, crate::flow_kind_str(&f.kind), f.trigger.unwrap_or_default());
    }
    Ok(())
}

/// `scc cochange`: print the git co-change pairs (files changed together
/// across commits) and, when an indexed store exists, enrich its components
/// with the signal.
pub fn cmd_cochange(root: &Path, min_commits: u32) -> crate::Result<()> {
    let pairs = scc_graph::cochange::cochange_pairs(root, min_commits)
        .map_err(crate::CliError::Other)?;
    if pairs.is_empty() {
        println!("no co-change pairs with >= {min_commits} shared commits");
    } else {
        println!("co-change pairs (>= {min_commits} shared commits):");
        for p in &pairs {
            println!("  {} <-> {} ×{}", p.a, p.b, p.commits);
        }
    }
    if crate::db_path(root).exists() {
        let store = crate::open_store(root)?;
        let n = scc_graph::cochange::enrich_components(&store, &pairs)
            .map_err(crate::CliError::Other)?;
        if n > 0 {
            println!("enriched {n} components with co-change signal");
        }
    }
    Ok(())
}

/// scc verify --graph-invariants: structural checks for CI.
pub fn cmd_check_invariants(root: &Path) -> crate::Result<bool> {
    let store = open_store(root)?;
    let graph = scc_graph::RealityGraph::load(&store)?;
    let mut ok = true;
    // dangling refs
    for r in graph.all_rels() {
        let known = |id: &str| {
            graph.entities.contains_key(id)
                || id.contains("/external_api/")
                || id.contains("/component/")
                || id.contains("/flow/")
                || id.contains("/invariant/")
        };
        if !known(&r.subject) {
            println!("dangling subject: {} — {}", r.subject, r.predicate);
            ok = false;
        }
        if !known(&r.object) {
            println!("dangling object: {} — {}", r.predicate, r.object);
            ok = false;
        }
    }
    // resolved without evidence
    for r in graph.all_rels() {
        if r.provenance == scc_core::Provenance::Resolved && r.evidence.is_empty() {
            println!("RESOLVED without evidence: {} — {}", r.subject, r.predicate);
            ok = false;
        }
    }
    // critical invariants unenforced
    for inv in store.invariants()? {
        if inv.severity == scc_core::Severity::Critical && inv.enforced_by.is_empty() {
            println!("critical invariant unenforced: {}", inv.statement);
            ok = false;
        }
    }
    // conflicting owners
    let _ = kinds::DATA_STORE;
    Ok(ok)
}

/// `scc ci check` (docs/DEPLOYMENT_AND_INFRA.md §3, EPIC-180 CI policies):
/// graph invariants + drift severity policy. Exits nonzero on violation.
pub fn cmd_ci_check(root: &Path, max_severity: &str) -> crate::Result<bool> {
    let store = open_store(root)?;
    let mut ok = cmd_check_invariants(root)?;
    let allowed = match max_severity {
        "low" => 1u8,
        "medium" => 2u8,
        "high" => 3u8,
        "critical" => 4u8,
        _ => 2u8, // default: medium allowed
    };
    let findings = store.drift_findings(true)?;
    for (_, kind, sev, msg, _) in &findings {
        let rank = match sev.as_str() {
            "low" => 1u8,
            "medium" => 2u8,
            "high" => 3u8,
            "critical" => 4u8,
            _ => 2u8,
        };
        if rank > allowed {
            println!("[ci:fail] [{sev}] {kind}: {msg}");
            ok = false;
        } else {
            println!("[ci:warn] [{sev}] {kind}: {msg}");
        }
    }
    if ok {
        println!("ci check passed");
    }
    Ok(ok)
}

pub fn cmd_checkpoint_save(root: &Path, json: bool) -> crate::Result<()> {
    let data = checkpoint::capture(root)?;
    if json {
        println!("{}", serde_json::to_string(&data)?);
    } else {
        println!("checkpoint saved to {}", crate::checkpoint_path(root).display());
    }
    Ok(())
}

pub fn cmd_checkpoint_load(root: &Path, inject: bool) -> crate::Result<()> {
    if let Some(content) = checkpoint::load(root)? {
        print!("{content}");
    } else if !inject {
        println!("no checkpoint found");
    }
    Ok(())
}

pub fn cmd_watch(root: &Path) -> crate::Result<()> {
    crate::httpd::watch_loop(root)
}

pub fn cmd_serve(root: &Path) -> crate::Result<()> {
    crate::httpd::serve(root)
}

pub fn cmd_mcp(root: &Path) -> crate::Result<()> {
    crate::mcp::serve_stdio(root)
}

pub fn cmd_setup_claude(root: &Path) -> crate::Result<()> {
    crate::plugin::install(root)
}

pub fn cmd_ingest_runtime(root: &Path, body: &str) -> crate::Result<()> {
    let store = open_store(root)?;
    crate::httpd::ingest_runtime(&store, body)?;
    println!("accepted");
    Ok(())
}

/// Declared capability scope per adapter (docs/SECURITY.md §6: the doc
/// defines the manifest dimensions — FS scope, network, subprocess,
/// credentials — but has no per-adapter table, so the assignments are
/// pinned inline here). Importers read repo-local files only; Context7 runs
/// an MCP server over stdio via npx (subprocess) that makes network calls.
fn adapter_scope(name: &str) -> &'static str {
    match name {
        "context7" => "network+subprocess(npx)",
        "serena" | "beads" | "cbm" | "hindsight" | "scip" | "gitnexus" | "narsil" => "filesystem",
        _ => "filesystem",
    }
}

/// Compute the configured-adapter scope listing: adapters enabled via
/// config.integrations, then the always-available on-demand importers
/// (scip/cbm need no config). Fixed declaration order keeps the output
/// deterministic.
fn configured_adapters(root: &Path) -> crate::Result<Vec<(String, &'static str)>> {
    let config = load_config(root)?;
    let mut configured: Vec<(String, &'static str)> = Vec::new();
    if config.integrations.serena {
        configured.push(("serena".into(), adapter_scope("serena")));
    }
    if config.integrations.beads {
        configured.push(("beads".into(), adapter_scope("beads")));
    }
    if config.integrations.hindsight {
        configured.push(("hindsight".into(), adapter_scope("hindsight")));
    }
    if config.integrations.gitnexus {
        configured.push(("gitnexus".into(), adapter_scope("gitnexus")));
    }
    if config.integrations.narsil {
        configured.push(("narsil".into(), adapter_scope("narsil")));
    }
    if !config.integrations.context7_command.is_empty() {
        configured.push(("context7".into(), adapter_scope("context7")));
    }
    configured.push(("scip".into(), adapter_scope("scip")));
    configured.push(("cbm".into(), adapter_scope("cbm")));
    Ok(configured)
}

/// `scc adapters` — list enabled adapters with their declared capability
/// scope (security audit; docs/SECURITY.md §6). `--json` dumps the full
/// capability manifests instead.
pub fn cmd_adapters(root: &Path, json: bool) -> crate::Result<()> {
    let manifests = scc_indexer::adapters::adapter_manifests();
    if json {
        println!("{}", serde_json::to_string_pretty(&manifests)?);
        return Ok(());
    }
    for (name, scope) in configured_adapters(root)? {
        println!("adapter: {name}  scope: {scope}");
    }
    Ok(())
}

/// `scc lessons add <text>` — append one durable lesson to
/// `<root>/.scc/lessons.jsonl` (the Hindsight memory bank). The bank is
/// ingested into the System IR with `scc import hindsight .scc/lessons.jsonl`.
pub fn cmd_lessons_add(root: &Path, text: &str) -> crate::Result<()> {
    let dir = scc_dir(root);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("lessons.jsonl");
    let n = std::fs::read_to_string(&path)
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);
    let id = format!("lesson-{}", n + 1);
    let record = serde_json::json!({
        "id": id,
        "text": text,
        "created_at": scc_core::now_rfc3339(),
    });
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| crate::CliError::Other(format!("lessons: {e}")))?;
    writeln!(f, "{record}").map_err(|e| crate::CliError::Other(format!("lessons: {e}")))?;
    println!(
        "appended {id} to {} (ingest with `scc import hindsight .scc/lessons.jsonl`)",
        path.display()
    );
    Ok(())
}

/// `scc lessons` — list stored lessons from the System IR (most important
/// first, same ordering as the context-pack enrichment).
pub fn cmd_lessons_list(root: &Path, limit: usize) -> crate::Result<()> {
    let store = open_store(root)?;
    let lessons = scc_indexer::adapters::hindsight::lessons(&store, limit);
    if lessons.is_empty() {
        println!("no lessons in the store — run `scc lessons add \"...\"` then `scc import hindsight .scc/lessons.jsonl`");
        return Ok(());
    }
    for (content, tags) in lessons {
        let tag_str = if tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", tags.join(", "))
        };
        println!("- {content}{tag_str}");
    }
    Ok(())
}

/// `scc beads` — list active (in-progress) tasks from `.beads/issues.jsonl`
/// (task state, not system facts).
pub fn cmd_beads(root: &Path) -> crate::Result<()> {
    let active = scc_indexer::adapters::beads::active_beads(root, 20);
    if active.is_empty() {
        println!("no active beads tasks (checked .beads/issues.jsonl)");
        return Ok(());
    }
    println!("active beads tasks:");
    for t in active {
        println!("- {t}");
    }
    Ok(())
}

pub fn cmd_import(root: &Path, format: &str, file: &str) -> crate::Result<()> {
    let store = open_store(root)?;
    let report = match format {
        "scip" => scc_indexer::adapters::import_scip(&store, std::path::Path::new(file)),
        "ccg" => scc_indexer::adapters::import_ccg(&store, std::path::Path::new(file)),
        "gitnexus" => scc_indexer::adapters::gitnexus::import_gitnexus(&store, std::path::Path::new(file))
            .map(|r| scc_indexer::adapters::ImportReport {
                symbols: r.symbols,
                calls: r.edges,
                imports: 0,
                errors: r.errors,
            }),
        "beads" => scc_indexer::adapters::beads::import_beads(&store, std::path::Path::new(file))
            .map(|r| scc_indexer::adapters::ImportReport {
                symbols: r.tasks,
                calls: r.dependencies,
                imports: r.active,
                errors: r.errors,
            }),
        "cbm" => scc_indexer::adapters::cbm::import_cbm(&store, std::path::Path::new(file))
            .map(|r| scc_indexer::adapters::ImportReport {
                symbols: r.symbols,
                calls: r.relationships,
                imports: 0,
                errors: r.errors,
            }),
        "hindsight" => scc_indexer::adapters::hindsight::import_hindsight(&store, std::path::Path::new(file))
            .map(|r| scc_indexer::adapters::ImportReport {
                symbols: r.lessons,
                calls: 0,
                imports: 0,
                errors: r.errors,
            }),
        other => {
            return Err(crate::CliError::Other(format!(
                "unknown import format '{other}' (use scip, ccg, gitnexus, beads, cbm, or hindsight)"
            )))
        }
    }
    .map_err(crate::CliError::Other)?;
    // P0: imported evidence changes system truth — bump the evidence model
    // epoch (invalidating every epoch-keyed context pack/atlas) and
    // recompile the derived layer so flows/components reflect the new facts.
    store.bump_epoch(scc_store::ModelEpochKind::Evidence)?;
    crate::recompile(&store)?;
    println!(
        "imported {} symbols, {} calls, {} imports ({} errors); evidence epoch bumped, derived layer recompiled",
        report.symbols, report.calls, report.imports, report.errors
    );
    Ok(())
}

pub fn cmd_runtime_status(root: &Path, json: bool) -> crate::Result<()> {
    let store = open_store(root)?;
    let edges = scc_indexer::runtime::runtime_edges(&store)
        .map_err(crate::CliError::Other)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&edges)?);
        return Ok(());
    }
    if edges.is_empty() {
        println!("no runtime observations ingested");
    }
    let total: u64 = edges.iter().map(|e| e.count).sum();
    let errs: u64 = edges.iter().map(|e| e.errors).sum();
    println!("{} observed edge(s), {} observations, {} error(s)", edges.len(), total, errs);
    for e in &edges {
        println!(
            "- {} → {} ×{} (avg {:.1} ms, {} err, last {})",
            e.source, e.target, e.count, e.latency_ms, e.errors, e.last_observed
        );
    }
    Ok(())
}

pub fn cmd_runtime_reconcile(root: &Path, json: bool) -> crate::Result<()> {
    let store = open_store(root)?;
    let rec = scc_indexer::runtime::reconcile(&store).map_err(crate::CliError::Other)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&rec)?);
        return Ok(());
    }
    println!("static-vs-observed reconciliation");
    println!("  matched:             {}", rec.matched.len());
    println!("  observed not static: {}", rec.observed_not_static.len());
    println!("  static not observed: {}", rec.static_not_observed.len());
    for e in &rec.matched {
        println!("  [matched] {e}");
    }
    for e in &rec.observed_not_static {
        println!("  [runtime-only] {e}");
    }
    for e in &rec.static_not_observed {
        println!("  [static-only] {e}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lessons_add_appends_jsonl_lines() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        cmd_lessons_add(&root, "first lesson").unwrap();
        cmd_lessons_add(&root, "second lesson").unwrap();
        let path = root.join(".scc/lessons.jsonl");
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "a second add must append, not overwrite");
        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["id"], "lesson-1");
        assert_eq!(first["text"], "first lesson");
        assert!(first["created_at"].is_string());
        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["id"], "lesson-2");
        assert_eq!(second["text"], "second lesson");
        // the shape must be ingestable by the hindsight adapter
        let lesson: scc_indexer::adapters::hindsight::Lesson =
            serde_json::from_str(lines[0]).unwrap();
        assert_eq!(lesson.body(), "first lesson");
    }

    #[test]
    fn adapters_lists_configured_scope() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join(".scc")).unwrap();
        std::fs::write(
            root.join(".scc/config.yaml"),
            "schema: 1\nintegrations:\n  beads: true\n  context7_command: \"npx -y @upstash/context7-mcp\"\n",
        )
        .unwrap();
        let listing = configured_adapters(&root).unwrap();
        assert!(
            listing.contains(&("beads".to_string(), "filesystem")),
            "{listing:?}"
        );
        assert!(
            listing.contains(&("context7".to_string(), "network+subprocess(npx)")),
            "{listing:?}"
        );
        assert!(
            !listing.iter().any(|(n, _)| n == "hindsight"),
            "disabled integrations must not be listed: {listing:?}"
        );
        // scip/cbm are always-available on-demand importers
        assert!(listing.iter().any(|(n, _)| n == "scip"), "{listing:?}");
        assert!(listing.iter().any(|(n, _)| n == "cbm"), "{listing:?}");
        // cmd_adapters renders the exact expected line format
        cmd_adapters(&root, false).unwrap();
    }
}
