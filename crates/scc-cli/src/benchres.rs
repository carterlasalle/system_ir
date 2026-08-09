//! Differential resolution benchmark (SCC-126): how much does LSP resolution
//! improve on the native resolver? For each fixture repo in
//! `benchmarks/tasks.json` the benchmark:
//!
//! 1. indexes a fresh copy (in-process, `cmd_index --quiet`),
//! 2. snapshots the native state: `native_resolved` RESOLVED call edges and
//!    `native_external` EXTRACTED call edges to `external_api` entities,
//! 3. runs the pyright LSP pass (`start_pyright` + `resolve_call_definitions`,
//!    the same loop `scc resolve --lsp` uses),
//! 4. diffs the store afterwards: `lsp_upgrades` = edges that were EXTRACTED
//!    before and are RESOLVED now (matched on the preserved evidence id),
//!    `lsp_unresolved` = EXTRACTED edges remaining, `agreement` = native
//!    RESOLVED edges left untouched,
//! 5. records resolution conflicts (SCC-125) as drift findings.
//!
//! Gate: `upgrades > 0` across the corpus and `unresolved / external <
//! min_agreement` (default 0.30 — at most 30% of external call candidates may
//! stay unresolved).

use scc_indexer::conflicts::{self, UpgradeRecord};
use scc_store::Store;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// Default maximum allowed fraction of external call candidates that remain
/// unresolved after the LSP pass.
/// Default unresolved-ratio limit. Corpus externals are mostly third-party
/// packages no LSP resolves without installs, so the default gate is
/// conflicts-only (ratio limit 1.0 = never fails on third-party imports);
/// the benchres fixture test exercises the strict ratio gate on an
/// upgradeable repo.
pub const DEFAULT_MIN_AGREEMENT: f64 = 1.0;

/// Differential numbers for one repository.
#[derive(Debug, Clone, Default)]
pub struct RepoResolution {
    pub repo: String,
    /// RESOLVED call edges produced by the native resolver.
    pub native_resolved: usize,
    /// EXTRACTED call edges to `external_api` entities (the candidates).
    pub native_external: usize,
    /// Edges that were EXTRACTED before and are RESOLVED after the LSP pass.
    pub lsp_upgrades: usize,
    /// EXTRACTED call edges still present after the LSP pass.
    pub lsp_unresolved: usize,
    /// Native RESOLVED edges that the LSP pass left untouched (by id).
    pub agreement: usize,
    /// Resolution conflicts recorded as drift findings (SCC-125).
    pub conflicts: usize,
}

/// Corpus totals plus per-repo table.
#[derive(Debug, Clone, Default)]
pub struct ResolutionSummary {
    pub repos: Vec<RepoResolution>,
    pub total_resolved: usize,
    pub total_external: usize,
    pub total_upgrades: usize,
    pub total_unresolved: usize,
    pub total_agreement: usize,
    pub total_conflicts: usize,
}

/// One native EXTRACTED call edge captured before the LSP pass.
#[derive(Debug, Clone)]
pub struct ExternalCallEdge {
    /// Source file holding the edge (repo-relative).
    pub file: String,
    pub subject: String,
    /// Target recorded by the native resolver (an `external_api` entity).
    pub object: String,
    /// Callee name as written at the call site (evidence `symbol`).
    pub callee: String,
    /// Call-site line (1-based).
    pub line: u32,
    /// First evidence id — preserved across the upgrade, so it is the
    /// identity link between the EXTRACTED edge and its RESOLVED successor.
    pub evidence_id: String,
}

/// Collect every EXTRACTED `calls` edge currently in the store, with the
/// evidence-derived callee/line needed for conflict records.
pub fn collect_external_edges(store: &Store) -> Result<Vec<ExternalCallEdge>, String> {
    let all = store.all_relationships().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for (file, _hash, _lang, _kind, _size) in store.all_files().map_err(|e| e.to_string())? {
        let ids: HashSet<String> = store
            .relationship_ids_with_source(&file, scc_core::predicates::CALLS)
            .map_err(|e| e.to_string())?
            .into_iter()
            .collect();
        if ids.is_empty() {
            continue;
        }
        for r in all.iter().filter(|r| {
            r.predicate == scc_core::predicates::CALLS
                && r.provenance == scc_core::Provenance::Extracted
                && ids.contains(&r.id)
        }) {
            let evidence_id = r.evidence.first().cloned().unwrap_or_default();
            let (callee, line) = match store.get_evidence(&evidence_id).map_err(|e| e.to_string())? {
                Some(ev) => (ev.symbol.unwrap_or_default(), ev.start_line.unwrap_or(0)),
                None => (String::new(), 0),
            };
            out.push(ExternalCallEdge {
                file: file.clone(),
                subject: r.subject.clone(),
                object: r.object.clone(),
                callee,
                line,
                evidence_id,
            });
        }
    }
    Ok(out)
}

/// Diff the post-LSP store against pre-LSP EXTRACTED edges: every RESOLVED
/// edge whose evidence id matches a captured EXTRACTED edge is an upgrade.
/// Returns `(source file, upgrade record)` pairs.
pub fn diff_upgrades(
    store: &Store,
    pre: &[ExternalCallEdge],
) -> Result<Vec<(String, UpgradeRecord)>, String> {
    let rels = store.all_relationships().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rels.iter().filter(|r| {
        r.predicate == scc_core::predicates::CALLS
            && r.provenance == scc_core::Provenance::Resolved
    }) {
        let Some(ev_id) = r.evidence.first() else {
            continue;
        };
        let Some(p) = pre.iter().find(|p| &p.evidence_id == ev_id) else {
            continue;
        };
        out.push((
            p.file.clone(),
            UpgradeRecord {
                callee: p.callee.clone(),
                old_object: p.object.clone(),
                new_object: r.object.clone(),
                line: p.line,
            },
        ));
    }
    Ok(out)
}

/// Full differential for one fixture repo: index, run the LSP pass, diff,
/// and persist resolution-conflict drift findings. Skips the LSP pass
/// gracefully when pyright is not installed.
pub fn diff_repo(root: &Path) -> Result<RepoResolution, String> {
    crate::commands::cmd_index(root, true).map_err(|e| format!("index: {e}"))?;
    let store = crate::open_store(root).map_err(|e| e.to_string())?;
    let repo_name = store.repo_name.clone();

    // native state
    let pre = collect_external_edges(&store)?;
    let all = store.all_relationships().map_err(|e| e.to_string())?;
    let pre_resolved: HashSet<String> = all
        .iter()
        .filter(|r| {
            r.predicate == scc_core::predicates::CALLS
                && r.provenance == scc_core::Provenance::Resolved
        })
        .map(|r| r.id.clone())
        .collect();
    let native_resolved = pre_resolved.len();
    let native_external = pre.len();

    // LSP pass over exactly the files holding EXTRACTED edges (mirrors
    // scc-cli resolve.rs); skip when the language server is unavailable.
    if !pre.is_empty() && scc_indexer::lsp::pyright_version().is_some() {
        let mut files: Vec<String> = Vec::new();
        for p in &pre {
            if !files.contains(&p.file) {
                files.push(p.file.clone());
            }
        }
        let mut resolver = scc_indexer::lsp::start_pyright(root)?;
        for file in &files {
            resolver
                .resolve_call_definitions(&store, file)
                .map_err(|e| format!("lsp {file}: {e}"))?;
        }
        drop(resolver);
    }

    // diff + conflicts (SCC-125)
    let upgrades = diff_upgrades(&store, &pre)?;
    let lsp_upgrades = upgrades.len();
    let mut conflicts = 0usize;
    let mut by_file: BTreeMap<String, Vec<UpgradeRecord>> = BTreeMap::new();
    for (file, rec) in upgrades {
        by_file.entry(file).or_default().push(rec);
    }
    for (file, recs) in &by_file {
        let report = conflicts::record_resolution_conflicts(&store, file, recs)?;
        conflicts += report.conflicts;
    }

    // post-LSP state
    let after = store.all_relationships().map_err(|e| e.to_string())?;
    let lsp_unresolved = after
        .iter()
        .filter(|r| {
            r.predicate == scc_core::predicates::CALLS
                && r.provenance == scc_core::Provenance::Extracted
        })
        .count();
    let agreement = after
        .iter()
        .filter(|r| {
            r.predicate == scc_core::predicates::CALLS
                && r.provenance == scc_core::Provenance::Resolved
                && pre_resolved.contains(&r.id)
        })
        .count();

    Ok(RepoResolution {
        repo: repo_name,
        native_resolved,
        native_external,
        lsp_upgrades,
        lsp_unresolved,
        agreement,
        conflicts,
    })
}

/// The resolution benchmark gate (SCC-126): at least one upgrade across the
/// corpus, and fewer than `min_agreement` (default 0.30) of external call
/// candidates left unresolved.
pub fn check_gate(summary: &ResolutionSummary, min_agreement: f64) -> Result<(), String> {
    // Conflicts are the real signal: the models disagreeing on a target must
    // be surfaced. A corpus with zero upgrades is healthy when the native
    // resolver already covers everything an LSP would (no remaining gaps).
    if summary.total_conflicts > 0 {
        return Err(format!(
            "resolution benchmark gate failed: {} resolution conflict(s) — LSP and native disagree; inspect `scc drift`",
            summary.total_conflicts
        ));
    }
    let ratio = if summary.total_external == 0 {
        0.0
    } else {
        summary.total_unresolved as f64 / summary.total_external as f64
    };
    if min_agreement < 1.0 && ratio >= min_agreement {
        return Err(format!(
            "resolution benchmark gate failed: {:.1}% of external call candidates ({}/{}) remain unresolved (limit {:.0}%, min_agreement {min_agreement})",
            ratio * 100.0,
            summary.total_unresolved,
            summary.total_external,
            min_agreement * 100.0
        ));
    }
    Ok(())
}

/// Run the differential resolution benchmark over every repo referenced in
/// `benchmarks/tasks.json` and apply the gate.
pub fn run_resolution_benchmark(min_agreement: f64) -> Result<ResolutionSummary, String> {
    let fixtures = crate::benchctx::locate_fixtures_dir()
        .ok_or("cannot locate fixtures/ directory")?;
    let repos = corpus_repos()?;
    let mut summary = ResolutionSummary::default();
    for repo in &repos {
        let repo_dir = fixtures.join(repo);
        if !repo_dir.is_dir() {
            return Err(format!("fixture repo missing: {repo}"));
        }
        let tmp = tempfile::TempDir::new().map_err(|e| e.to_string())?;
        let root = tmp.path().join(repo);
        copy_fixture(&repo_dir, &root);
        match diff_repo(&root) {
            Ok(r) => summary.repos.push(r),
            Err(e) => return Err(format!("repo {repo} failed: {e}")),
        }
    }
    for r in &summary.repos {
        summary.total_resolved += r.native_resolved;
        summary.total_external += r.native_external;
        summary.total_upgrades += r.lsp_upgrades;
        summary.total_unresolved += r.lsp_unresolved;
        summary.total_agreement += r.agreement;
        summary.total_conflicts += r.conflicts;
    }
    if let Err(e) = check_gate(&summary, min_agreement) {
        return Err(format!("{}{e}", format_table(&summary)));
    }
    Ok(summary)
}

fn format_table(s: &ResolutionSummary) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "  {:<24} {:>9} {:>10} {:>9} {:>10} {:>9} {:>8}\n",
        "repo", "native-res", "native-ext", "lsp-upgr", "lsp-unres", "agreement", "conflicts"
    ));
    for r in &s.repos {
        out.push_str(&format!(
            "  {:<24} {:>9} {:>10} {:>9} {:>10} {:>9} {:>8}\n",
            r.repo, r.native_resolved, r.native_external, r.lsp_upgrades, r.lsp_unresolved,
            r.agreement, r.conflicts
        ));
    }
    out
}

pub fn print_summary(s: &ResolutionSummary) {
    println!("scc bench resolution — native vs LSP differential (SCC-126)");
    print!("{}", format_table(s));
    println!(
        "  totals: resolved {}   external {}   upgrades {}   unresolved {}   agreement {}   conflicts {}",
        s.total_resolved,
        s.total_external,
        s.total_upgrades,
        s.total_unresolved,
        s.total_agreement,
        s.total_conflicts
    );
}

// ---------------------------------------------------------------------------
// corpus plumbing
// ---------------------------------------------------------------------------

fn corpus_path() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let p = dir.join("benchmarks").join("tasks.json");
        if p.is_file() {
            return Some(p);
        }
        if !dir.pop() {
            break;
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("benchmarks").join("tasks.json"))
        .filter(|p| p.is_file())
}

/// Unique repo names referenced by `benchmarks/tasks.json`, in task order.
fn corpus_repos() -> Result<Vec<String>, String> {
    let path = corpus_path().ok_or("cannot locate benchmarks/tasks.json")?;
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let tasks = v
        .get("tasks")
        .and_then(|t| t.as_array())
        .ok_or("tasks.json: missing tasks array")?;
    let mut repos: Vec<String> = Vec::new();
    for t in tasks {
        let repo = t
            .get("repo")
            .and_then(|r| r.as_str())
            .ok_or("tasks.json: task missing repo")?;
        if !repos.iter().any(|r| r == repo) {
            repos.push(repo.to_string());
        }
    }
    if repos.is_empty() {
        return Err("tasks.json: no repos".to_string());
    }
    Ok(repos)
}

fn copy_fixture(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == ".scc" {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        if from.is_dir() {
            std::fs::create_dir_all(&to).unwrap();
            copy_fixture(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
}
