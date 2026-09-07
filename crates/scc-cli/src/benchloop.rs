//! Deterministic agent-loop comparison: baseline vs SCC vs Ripwire.
//!
//! This is a locator loop (retrieve context, open named files), not an LLM
//! run. Ripwire is a black-box CLI. A missing binary is `skipped`, never a
//! claimed win. Production ranking is unchanged.

use crate::benchctx::{copy_fixture, locate_fixtures_dir, BenchmarkCorpus};
use scc_core::subtokens;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// trace:exempt reason=internal-detail
pub enum LoopArm {
    Baseline,
    Scc,
    Ripwire,
}

// trace:exempt reason=internal-detail
impl LoopArm {
    // trace:exempt reason=internal-detail
    pub fn as_str(self) -> &'static str {
        match self {
            LoopArm::Baseline => "baseline",
            LoopArm::Scc => "scc",
            LoopArm::Ripwire => "ripwire",
        }
    }

    // trace:exempt reason=internal-detail
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "baseline" => Some(LoopArm::Baseline),
            "scc" => Some(LoopArm::Scc),
            "ripwire" => Some(LoopArm::Ripwire),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
// trace:exempt reason=internal-detail
pub struct LoopTaskRow {
    pub task: String,
    pub repo: String,
    pub arm: String,
    pub status: String,
    pub localization: f64,
    pub first_correct_rank: Option<usize>,
    pub files_opened: usize,
    pub search_calls: usize,
    pub scc_calls: usize,
    pub substitution_rate: f64,
    pub contaminated: bool,
    #[serde(default)]
    pub protocol: String,
    #[serde(default)]
    pub read_calls: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_correct_ms: Option<u64>,
    #[serde(default)]
    pub wrong_first: usize,
    #[serde(default)]
    pub jsonl_events: usize,
    /// Recall of gold test names/paths against pack tests_to_run. `None`
    /// when the task has no gold tests (omitted from clustered mean).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tests_localization: Option<f64>,
    #[serde(default)]
    pub tests_hit: usize,
    #[serde(default)]
    pub tests_gold: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
// trace:exempt reason=internal-detail
pub struct LoopArmSummary {
    pub arm: String,
    pub status: String,
    pub tasks: usize,
    pub clustered_localization: f64,
    pub pooled_localization: f64,
    pub mean_substitution: f64,
    pub skipped: usize,
    #[serde(default)]
    pub mean_search: f64,
    #[serde(default)]
    pub mean_read: f64,
    #[serde(default)]
    pub mean_files_opened: f64,
    #[serde(default)]
    pub clustered_tests: f64,
    #[serde(default)]
    pub pooled_tests: f64,
    #[serde(default)]
    pub tests_tasks: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
// trace:exempt reason=internal-detail
pub struct LoopSummary {
    pub arms: Vec<LoopArmSummary>,
    pub per_task: Vec<LoopTaskRow>,
    pub ripwire_bin: Option<String>,
    #[serde(default)]
    pub protocol: String,
}

#[derive(Debug, Clone, Default)]
// trace:exempt reason=internal-detail
pub struct LoopOptions {
    pub k: usize,
    pub repo_filter: Option<String>,
    pub ripwire_bin: Option<PathBuf>,
    pub explore: bool,
    pub agent_cmd: Option<String>,
}

/// Run the three-way locator loop. `min_delta` is unused here (measure-only
/// in tests); the CLI applies it after printing.
// trace:v1 id=impl.scc.cli.bench-loop work=WORK-ripwire-lessons-phase5 satisfies=REQ-agent-loop-three-way
pub fn run_agent_loop(arms: &[LoopArm], opts: &LoopOptions) -> Result<LoopSummary, String> {
    let k = opts.k.max(1);
    let fixtures = locate_fixtures_dir().ok_or("cannot locate fixtures/ directory")?;
    let corpus_path = fixtures
        .parent()
        .map(|p| p.join("benchmarks/tasks.json"))
        .ok_or("cannot locate benchmarks/tasks.json")?;
    let text = std::fs::read_to_string(&corpus_path)
        .map_err(|e| format!("read {}: {e}", corpus_path.display()))?;
    let corpus: BenchmarkCorpus =
        serde_json::from_str(&text).map_err(|e| format!("tasks.json: {e}"))?;

    let ripwire = resolve_ripwire(opts.ripwire_bin.as_deref());
    let mut summary = LoopSummary {
        ripwire_bin: ripwire.as_ref().map(|p| p.display().to_string()),
        protocol: if opts.explore {
            "explore".into()
        } else {
            "locator".into()
        },
        ..Default::default()
    };

    let mut by_repo: BTreeMap<String, Vec<&crate::benchctx::BenchTask>> = BTreeMap::new();
    for t in &corpus.tasks {
        if let Some(f) = &opts.repo_filter {
            if t.repo != *f {
                continue;
            }
        }
        by_repo.entry(t.repo.clone()).or_default().push(t);
    }
    if by_repo.is_empty() {
        return Err("no agent-loop tasks matched".into());
    }

    for (repo, tasks) in &by_repo {
        let src = fixtures.join(repo);
        if !src.is_dir() {
            continue;
        }
        let tmp = tempfile::TempDir::new().map_err(|e| e.to_string())?;
        let root = tmp.path().join("repo");
        copy_fixture(&src, &root);
        crate::commands::cmd_index(&root, true).map_err(|e| format!("index {repo}: {e}"))?;

        let indexed = indexed_paths(&root)?;
        for task in tasks {
            let gold = &task.ground_truth.files;
            if gold.is_empty() {
                continue;
            }
            let gold_tests = &task.ground_truth.tests;
            let contaminated = std::env::var("SCC_GOLD").is_ok();
            for arm in arms {
                let row = if opts.explore {
                    run_explore_arm(
                        *arm,
                        &root,
                        &task.id,
                        &task.repo,
                        &task.goal,
                        gold,
                        gold_tests,
                        &indexed,
                        k,
                        ripwire.as_deref(),
                        contaminated,
                        opts.agent_cmd.as_deref(),
                    )?
                } else {
                    run_arm(
                        *arm,
                        &root,
                        &task.id,
                        &task.repo,
                        &task.goal,
                        gold,
                        gold_tests,
                        &indexed,
                        k,
                        ripwire.as_deref(),
                        contaminated,
                    )?
                };
                summary.per_task.push(row);
            }
        }
    }

    summary.arms = aggregate(&summary.per_task, arms);
    Ok(summary)
}

#[allow(clippy::too_many_arguments)]
// trace:exempt reason=internal-detail
fn run_arm(
    arm: LoopArm,
    root: &Path,
    task_id: &str,
    repo: &str,
    goal: &str,
    gold: &[String],
    gold_tests: &[String],
    indexed: &[String],
    k: usize,
    ripwire: Option<&Path>,
    contaminated: bool,
) -> Result<LoopTaskRow, String> {
    match arm {
        LoopArm::Baseline => {
            let opened = baseline_open(root, goal, k);
            Ok(score_row(
                task_id,
                repo,
                arm,
                "ran",
                &opened,
                gold,
                gold_tests,
                "",
                1,
                0,
                0.0,
                contaminated,
            ))
        }
        LoopArm::Scc => {
            let pack = scc_pack(root, goal)?;
            let opened = files_from_text(&pack, indexed, k);
            Ok(score_row(
                task_id,
                repo,
                arm,
                "ran",
                &opened,
                gold,
                gold_tests,
                &pack,
                0,
                1,
                1.0,
                contaminated,
            ))
        }
        LoopArm::Ripwire => match ripwire {
            None => Ok(skipped_row(task_id, repo, "locator", contaminated)),
            Some(bin) => {
                let out = run_ripwire(bin, root, goal)?;
                let opened = files_from_ripwire(&out, indexed, k);
                Ok(score_row(
                    task_id,
                    repo,
                    arm,
                    "ran",
                    &opened,
                    gold,
                    gold_tests,
                    &out,
                    0,
                    0,
                    0.0,
                    contaminated,
                ))
            }
        },
    }
}

#[allow(clippy::too_many_arguments)]
// trace:exempt reason=internal-detail
fn score_row(
    task: &str,
    repo: &str,
    arm: LoopArm,
    status: &str,
    opened: &[String],
    gold: &[String],
    gold_tests: &[String],
    pack: &str,
    search_calls: usize,
    scc_calls: usize,
    substitution_rate: f64,
    contaminated: bool,
) -> LoopTaskRow {
    let gold_set: BTreeSet<String> = gold.iter().cloned().collect();
    let hits = opened
        .iter()
        .filter(|f| gold_matches(f, &gold_set))
        .count();
    let localization = if gold.is_empty() {
        1.0
    } else {
        hits as f64 / gold.len() as f64
    };
    let first_correct_rank = opened.iter().position(|f| gold_matches(f, &gold_set)).map(|i| i + 1);
    let (tests_localization, tests_hit, tests_gold) = score_tests(pack, gold_tests);
    LoopTaskRow {
        task: task.to_string(),
        repo: repo.to_string(),
        arm: arm.as_str().to_string(),
        status: status.to_string(),
        localization,
        first_correct_rank,
        files_opened: opened.len(),
        search_calls,
        scc_calls,
        substitution_rate,
        contaminated,
        protocol: "locator".into(),
        read_calls: 0,
        first_correct_ms: None,
        wrong_first: 0,
        jsonl_events: 0,
        tests_localization,
        tests_hit,
        tests_gold,
    }
}

// trace:exempt reason=internal-detail
fn skipped_row(task_id: &str, repo: &str, protocol: &str, contaminated: bool) -> LoopTaskRow {
    LoopTaskRow {
        task: task_id.to_string(),
        repo: repo.to_string(),
        arm: LoopArm::Ripwire.as_str().to_string(),
        status: "skipped".into(),
        localization: 0.0,
        first_correct_rank: None,
        files_opened: 0,
        search_calls: 0,
        scc_calls: 0,
        substitution_rate: 0.0,
        contaminated,
        protocol: protocol.into(),
        read_calls: 0,
        first_correct_ms: None,
        wrong_first: 0,
        jsonl_events: 0,
        tests_localization: None,
        tests_hit: 0,
        tests_gold: 0,
    }
}

// trace:exempt reason=internal-detail
fn gold_matches(opened: &str, gold: &BTreeSet<String>) -> bool {
    gold.iter().any(|g| opened == g || opened.ends_with(&format!("/{g}")) || g.ends_with(&format!("/{opened}")))
}

/// Names and paths from SCC TESTS rows and Ripwire `<test p="...">` pack rows.
// trace:v1 id=impl.scc.cli.loop-tests work=WORK-phase-18-of-scc-x-ripwire-lessons-score-tests-to-run-in-the-agent-loop satisfies=REQ-implement-phase-18-of-scc-x-ripwire-lessons-score-tests-to-run-in-the implements=PLAN-phase-18-of-scc-x-ripwire-lessons-score-tests-to-run-in-the-agent-loop
fn tests_from_pack(pack: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in pack.lines() {
        let t = line.trim();
        if let Some(p) = xml_test_path(t) {
            out.push(p);
            continue;
        }
        if !(t.starts_with("- ") && (t.contains(" — ") || t.contains(" -- "))) {
            continue;
        }
        let rest = t.trim_start_matches("- ");
        let left = rest
            .split(" — ")
            .next()
            .unwrap_or(rest)
            .split(" -- ")
            .next()
            .unwrap_or(rest)
            .trim();
        if let Some((name, after)) = left.split_once(" (") {
            let name = name.trim();
            if !name.is_empty() {
                out.push(name.to_string());
            }
            if let Some(file) = after.strip_suffix(')') {
                let file = file.trim();
                if !file.is_empty() {
                    out.push(file.to_string());
                }
            }
        } else if !left.is_empty() {
            out.push(left.to_string());
        }
    }
    out.sort();
    out.dedup();
    out
}

// trace:exempt reason=internal-detail
fn xml_test_path(t: &str) -> Option<String> {
    let idx = t.find("<test ")?;
    let rest = &t[idx..];
    let p = rest.find("p=\"")?;
    let start = p + 3;
    let end = rest[start..].find('"')?;
    let path = rest[start..start + end].trim();
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

/// Gold kebab-case `it()` ids match SCC titles; file paths do not match function names.
// trace:v1 id=impl.scc.cli.loop-test-match work=WORK-phase-20-of-scc-x-ripwire-lessons-raise-tests-to-run-recall-for-type-scr satisfies=REQ-implement-phase-20-of-scc-x-ripwire-lessons-raise-tests-to-run-recall implements=PLAN-phase-20-of-scc-x-ripwire-lessons-raise-tests-to-run-recall-for-type-scr
fn test_name_matches(proposed: &str, gold: &str) -> bool {
    if proposed.eq_ignore_ascii_case(gold) {
        return true;
    }
    if gold.contains('/') && (proposed.ends_with(gold) || gold.ends_with(proposed)) {
        return true;
    }
    let p: Vec<String> = ident_tokens(proposed).collect();
    let g: Vec<String> = ident_tokens(gold).collect();
    let gold_l = gold.to_ascii_lowercase();
    p.contains(&gold_l) || (!g.is_empty() && p == g)
}

// trace:exempt reason=internal-detail
fn ident_tokens(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
}

/// `None` when the task has no gold tests — omit from clustered mean.
// trace:exempt reason=internal-detail
fn score_tests(pack: &str, gold: &[String]) -> (Option<f64>, usize, usize) {
    if gold.is_empty() {
        return (None, 0, 0);
    }
    let proposed = tests_from_pack(pack);
    let hits = gold
        .iter()
        .filter(|g| proposed.iter().any(|p| test_name_matches(p, g)))
        .count();
    (Some(hits as f64 / gold.len() as f64), hits, gold.len())
}

// trace:exempt reason=internal-detail
fn aggregate(rows: &[LoopTaskRow], arms: &[LoopArm]) -> Vec<LoopArmSummary> {
    let mut out = Vec::new();
    for arm in arms {
        let mine: Vec<&LoopTaskRow> = rows.iter().filter(|r| r.arm == arm.as_str()).collect();
        let skipped = mine.iter().filter(|r| r.status == "skipped").count();
        let ran: Vec<&LoopTaskRow> = mine.iter().copied().filter(|r| r.status == "ran").collect();
        let mut by_repo: BTreeMap<String, Vec<f64>> = BTreeMap::new();
        let mut by_repo_tests: BTreeMap<String, Vec<f64>> = BTreeMap::new();
        let mut pooled = 0.0;
        let mut subst = 0.0;
        let mut search = 0.0;
        let mut read = 0.0;
        let mut files = 0.0;
        let mut pooled_tests = 0.0;
        let mut tests_n = 0usize;
        for r in &ran {
            by_repo.entry(r.repo.clone()).or_default().push(r.localization);
            pooled += r.localization;
            subst += r.substitution_rate;
            search += r.search_calls as f64;
            read += r.read_calls as f64;
            files += r.files_opened as f64;
            if let Some(t) = r.tests_localization {
                by_repo_tests.entry(r.repo.clone()).or_default().push(t);
                pooled_tests += t;
                tests_n += 1;
            }
        }
        let clustered = clustered_mean(&by_repo);
        let n = ran.len();
        out.push(LoopArmSummary {
            arm: arm.as_str().to_string(),
            status: if n == 0 && skipped > 0 {
                "skipped".into()
            } else {
                "ran".into()
            },
            tasks: n,
            clustered_localization: clustered,
            pooled_localization: if n == 0 { 0.0 } else { pooled / n as f64 },
            mean_substitution: if n == 0 { 0.0 } else { subst / n as f64 },
            skipped,
            mean_search: if n == 0 { 0.0 } else { search / n as f64 },
            mean_read: if n == 0 { 0.0 } else { read / n as f64 },
            mean_files_opened: if n == 0 { 0.0 } else { files / n as f64 },
            clustered_tests: clustered_mean(&by_repo_tests),
            pooled_tests: if tests_n == 0 { 0.0 } else { pooled_tests / tests_n as f64 },
            tests_tasks: tests_n,
        });
    }
    out
}

// trace:exempt reason=internal-detail
fn clustered_mean(per_repo: &BTreeMap<String, Vec<f64>>) -> f64 {
    if per_repo.is_empty() {
        return 0.0;
    }
    let mut acc = 0.0;
    let mut n = 0usize;
    for vals in per_repo.values() {
        if vals.is_empty() {
            continue;
        }
        acc += vals.iter().sum::<f64>() / vals.len() as f64;
        n += 1;
    }
    if n == 0 {
        0.0
    } else {
        acc / n as f64
    }
}

// trace:exempt reason=internal-detail
fn indexed_paths(root: &Path) -> Result<Vec<String>, String> {
    let store = crate::open_store(root).map_err(|e| e.to_string())?;
    let mut paths: Vec<String> = store
        .all_files()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(p, _, _, _, _)| p)
        .collect();
    paths.sort();
    Ok(paths)
}

// trace:exempt reason=internal-detail
fn scc_pack(root: &Path, goal: &str) -> Result<String, String> {
    let store = crate::open_store(root).map_err(|e| e.to_string())?;
    let config = crate::load_config(root).map_err(|e| e.to_string())?;
    let comp = crate::compiler(&store, &config, Vec::new()).map_err(|e| e.to_string())?;
    let ctx = comp.ctx();
    let pack = ctx.task_context(goal, &[], &[], Some(4000));
    Ok(pack.content)
}

// trace:exempt reason=internal-detail
fn baseline_open(root: &Path, goal: &str, k: usize) -> Vec<String> {
    let q: BTreeSet<String> = subtokens(goal).into_iter().collect();
    if q.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(i32, String)> = Vec::new();
    walk_files(root, root, &mut |rel, full| {
        let mut hay = rel.to_ascii_lowercase();
        if let Ok(text) = std::fs::read_to_string(full) {
            let take = text.chars().take(8000).collect::<String>();
            hay.push(' ');
            hay.push_str(&take);
        }
        let toks = subtokens(&hay);
        let score = toks.iter().filter(|t| q.contains(*t)).count() as i32;
        if score > 0 {
            scored.push((score, rel.to_string()));
        }
    });
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().take(k).map(|(_, p)| p).collect()
}

// trace:exempt reason=internal-detail
fn walk_files(root: &Path, dir: &Path, visit: &mut impl FnMut(&str, &Path)) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let name = entry.file_name();
        if name == ".scc" || name == ".git" {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            walk_files(root, &path, visit);
        } else if path.is_file() {
            if let Ok(rel) = path.strip_prefix(root) {
                let rel = rel.to_string_lossy().replace('\\', "/");
                visit(&rel, &path);
            }
        }
    }
}

/// Prefer FETCH `handle=scc://...` keys. Stale handles are refused (not
/// guessed as a path). Packs without handles fall back to path matching.
/// Path mentions may fill leftover slots only for files that were not
/// refused via a stale handle.
// trace:v1 id=impl.scc.cli.explore-handles work=WORK-phase-13-of-scc-x-ripwire-lessons-1-go-extract-time-receiver-field-as satisfies=REQ-implement-phase-13-of-scc-x-ripwire-lessons-1-go-extract-time-recei implements=PLAN-phase-13-of-scc-x-ripwire-lessons-1-go-extract-time-receiver-field-as
fn files_from_pack(text: &str, root: &Path, indexed: &[String], k: usize) -> Vec<String> {
    let (saw_handle, mut opened, refused) = files_from_handles(text, root, indexed, k);
    if !saw_handle {
        return files_from_text(text, indexed, k);
    }
    if opened.len() >= k {
        return opened;
    }
    for p in files_from_text(text, indexed, k) {
        if refused.contains(&p) || opened.iter().any(|x| x == &p) {
            continue;
        }
        opened.push(p);
        if opened.len() >= k {
            break;
        }
    }
    opened
}

// trace:exempt reason=internal-detail
fn files_from_handles(
    text: &str,
    root: &Path,
    indexed: &[String],
    k: usize,
) -> (bool, Vec<String>, BTreeSet<String>) {
    let mut saw = false;
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut refused = BTreeSet::new();
    for raw in text.split_whitespace() {
        let token = raw
            .strip_prefix("handle=")
            .unwrap_or(raw)
            .trim_end_matches([',', ';', ')', ']']);
        if !token.starts_with("scc://") {
            continue;
        }
        saw = true;
        match scc_context::structural_source::resolve_handle_to_path(root, token) {
            Ok(p) => {
                let hit = indexed
                    .iter()
                    .any(|i| i == &p || i.ends_with(&format!("/{p}")));
                if hit && seen.insert(p.clone()) && out.len() < k {
                    out.push(p);
                }
            }
            Err(_) => {
                if let Some(p) = path_from_handle(token) {
                    refused.insert(p);
                }
            }
        }
    }
    (saw, out, refused)
}

// trace:exempt reason=internal-detail
fn path_from_handle(token: &str) -> Option<String> {
    let h = scc_core::ContentHandle::parse(token).ok()?;
    match h.kind {
        scc_core::HandleKind::File => Some(h.key),
        scc_core::HandleKind::Symbol => h.key.split_once("::").map(|(p, _)| p.to_string()),
        scc_core::HandleKind::Span => h.key.split_once(":L").map(|(p, _)| p.to_string()),
        _ => None,
    }
}

// trace:exempt reason=internal-detail
fn files_from_text(text: &str, indexed: &[String], k: usize) -> Vec<String> {
    let mut hits: Vec<(usize, String)> = Vec::new();
    for p in indexed {
        if let Some(idx) = text.find(p) {
            hits.push((idx, p.clone()));
            continue;
        }
        if let Some(base) = p.rsplit('/').next() {
            if base.len() >= 3 {
                if let Some(idx) = text.find(base) {
                    hits.push((idx, p.clone()));
                }
            }
        }
    }
    hits.sort_by_key(|(i, p)| (*i, p.clone()));
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for (_, p) in hits {
        if seen.insert(p.clone()) {
            out.push(p);
        }
        if out.len() >= k {
            break;
        }
    }
    out
}

// trace:exempt reason=internal-detail
fn files_from_ripwire(text: &str, indexed: &[String], k: usize) -> Vec<String> {
    let mut ordered: Vec<String> = Vec::new();
    let mut seen = BTreeSet::new();
    for (prefix, quote) in [("p=\"", '"'), ("p='", '\'')] {
        let mut rest = text;
        while let Some(i) = rest.find(prefix) {
            let after = &rest[i + prefix.len()..];
            if let Some(end) = after.find(quote) {
                let p = after[..end].replace('\\', "/");
                if !p.is_empty() && seen.insert(p.clone()) {
                    ordered.push(p);
                }
                rest = &after[end + 1..];
            } else {
                break;
            }
        }
    }
    if ordered.is_empty() {
        return files_from_text(text, indexed, k);
    }
    let mut out = Vec::new();
    for p in ordered {
        if let Some(hit) = indexed.iter().find(|idx| {
            *idx == &p || idx.ends_with(&format!("/{p}")) || p.ends_with(&format!("/{idx}"))
        }) {
            if !out.iter().any(|x| x == hit) {
                out.push(hit.clone());
            }
        } else if !out.contains(&p) {
            out.push(p);
        }
        if out.len() >= k {
            break;
        }
    }
    out
}

// trace:exempt reason=internal-detail
fn resolve_ripwire(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        if p.is_file() {
            return Some(p.to_path_buf());
        }
    }
    if let Ok(p) = std::env::var("RIPWIRE_BIN") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    // Unit tests must not depend on a machine-local vendor tree. CLI and
    // `cargo run` still search known build dirs so a present binary is
    // measured instead of silently skipped.
    if !cfg!(test) {
        for cand in [
            "/tmp/vendor/ripwire/build/ripwire",
            "/tmp/vendor/ripwire/build-gxx/ripwire",
        ] {
            let p = PathBuf::from(cand);
            if p.is_file() {
                return Some(p);
            }
        }
        if let Some(p) = scan_ripwire_builds() {
            return Some(p);
        }
    }
    which("ripwire")
}

// trace:exempt reason=internal-detail
fn scan_ripwire_builds() -> Option<PathBuf> {
    let root = Path::new("/tmp/vendor/ripwire");
    let rd = std::fs::read_dir(root).ok()?;
    let mut bins: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("build"))
        })
        .map(|p| p.join("ripwire"))
        .filter(|p| p.is_file())
        .collect();
    bins.sort();
    bins.into_iter().next()
}

// trace:exempt reason=internal-detail
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var("PATH").ok()?;
    for dir in path.split(':') {
        let p = Path::new(dir).join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

// trace:exempt reason=internal-detail
fn run_ripwire(bin: &Path, root: &Path, goal: &str) -> Result<String, String> {
    let out = Command::new(bin)
        .arg(root)
        .arg(format!("--pack-task={goal}"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();
    let out = match out {
        Ok(o) => o,
        Err(e) => return Err(format!("ripwire spawn: {e}")),
    };
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if stdout.trim().is_empty() {
        return Err(format!(
            "ripwire empty stdout (status {}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(stdout)
}

#[allow(clippy::too_many_arguments)]
// trace:v1 id=impl.scc.cli.bench-explore work=WORK-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing-from-unique satisfies=REQ-implement-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing
fn run_explore_arm(
    arm: LoopArm,
    root: &Path,
    task_id: &str,
    repo: &str,
    goal: &str,
    gold: &[String],
    gold_tests: &[String],
    indexed: &[String],
    k: usize,
    ripwire: Option<&Path>,
    contaminated: bool,
    agent_cmd: Option<&str>,
) -> Result<LoopTaskRow, String> {
    if let Some(cmd) = agent_cmd {
        return run_explore_agent_cmd(
            arm,
            root,
            task_id,
            repo,
            goal,
            gold,
            gold_tests,
            indexed,
            k,
            ripwire,
            contaminated,
            cmd,
        );
    }
    match arm {
        LoopArm::Baseline => {
            let jsonl = explore_baseline_jsonl(root, goal, k);
            Ok(score_explore(
                task_id,
                repo,
                arm,
                "ran",
                &jsonl,
                root,
                gold,
                gold_tests,
                "",
                0,
                0.0,
                contaminated,
            ))
        }
        LoopArm::Scc => {
            let pack = scc_pack(root, goal)?;
            let opened = files_from_pack(&pack, root, indexed, k);
            let jsonl = explore_pack_jsonl("task_context", goal, &opened);
            Ok(score_explore(
                task_id,
                repo,
                arm,
                "ran",
                &jsonl,
                root,
                gold,
                gold_tests,
                &pack,
                1,
                1.0,
                contaminated,
            ))
        }
        LoopArm::Ripwire => match ripwire {
            None => Ok(skipped_row(task_id, repo, "explore", contaminated)),
            Some(bin) => {
                let out = run_ripwire(bin, root, goal)?;
                let opened = files_from_ripwire(&out, indexed, k);
                let jsonl = explore_ripwire_jsonl(goal, &opened);
                Ok(score_explore(
                    task_id,
                    repo,
                    arm,
                    "ran",
                    &jsonl,
                    root,
                    gold,
                    gold_tests,
                    &out,
                    0,
                    0.0,
                    contaminated,
                ))
            }
        },
    }
}

#[allow(clippy::too_many_arguments)]
// trace:exempt reason=internal-detail
fn run_explore_agent_cmd(
    arm: LoopArm,
    root: &Path,
    task_id: &str,
    repo: &str,
    goal: &str,
    gold: &[String],
    gold_tests: &[String],
    indexed: &[String],
    k: usize,
    ripwire: Option<&Path>,
    contaminated: bool,
    cmd: &str,
) -> Result<LoopTaskRow, String> {
    if arm == LoopArm::Ripwire && ripwire.is_none() {
        return Ok(skipped_row(task_id, repo, "explore", contaminated));
    }
    let pack = match arm {
        LoopArm::Scc => scc_pack(root, goal)?,
        LoopArm::Ripwire => run_ripwire(ripwire.unwrap(), root, goal)?,
        LoopArm::Baseline => String::new(),
    };
    let pack_path = root.join(".scc").join("explore-pack.txt");
    let _ = std::fs::create_dir_all(root.join(".scc"));
    std::fs::write(&pack_path, &pack).map_err(|e| e.to_string())?;
    let out = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(root)
        .env("SCC_GOAL", goal)
        .env("SCC_ARM", arm.as_str())
        .env("SCC_PACK", pack_path.display().to_string())
        .output()
        .map_err(|e| format!("explore agent: {e}"))?;
    let jsonl = String::from_utf8_lossy(&out.stdout).to_string();
    let scc_calls = if arm == LoopArm::Scc { 1 } else { 0 };
    let subst = if arm == LoopArm::Scc { 1.0 } else { 0.0 };
    let _ = (indexed, k);
    Ok(score_explore(
        task_id,
        repo,
        arm,
        "ran",
        &jsonl,
        root,
        gold,
        gold_tests,
        &pack,
        scc_calls,
        subst,
        contaminated,
    ))
}

// trace:exempt reason=internal-detail
fn explore_baseline_jsonl(root: &Path, goal: &str, k: usize) -> String {
    let mut lines = Vec::new();
    lines.push(
        serde_json::json!({
            "type": "tool_use",
            "name": "grep",
            "input": {"query": goal}
        })
        .to_string(),
    );
    for p in baseline_open(root, goal, k) {
        lines.push(
            serde_json::json!({
                "type": "tool_use",
                "name": "read",
                "input": {"file_path": p}
            })
            .to_string(),
        );
    }
    lines.join("\n")
}

// trace:exempt reason=internal-detail
fn explore_pack_jsonl(tool: &str, goal: &str, files: &[String]) -> String {
    let mut lines = Vec::new();
    lines.push(
        serde_json::json!({
            "type": "item.completed",
            "item": {
                "type": "mcp_tool_call",
                "tool": tool,
                "arguments": {"goal": goal}
            }
        })
        .to_string(),
    );
    for p in files {
        lines.push(
            serde_json::json!({
                "type": "tool_use",
                "name": "read",
                "input": {"file_path": p}
            })
            .to_string(),
        );
    }
    lines.join("\n")
}

// trace:exempt reason=internal-detail
fn explore_ripwire_jsonl(goal: &str, files: &[String]) -> String {
    let mut lines = Vec::new();
    lines.push(
        serde_json::json!({
            "type": "item.completed",
            "item": {
                "type": "command_execution",
                "command": format!("ripwire . --pack-task={goal}")
            }
        })
        .to_string(),
    );
    for p in files {
        lines.push(
            serde_json::json!({
                "type": "tool_use",
                "name": "read",
                "input": {"file_path": p}
            })
            .to_string(),
        );
    }
    lines.join("\n")
}

#[allow(clippy::too_many_arguments)]
// trace:exempt reason=internal-detail
fn score_explore(
    task: &str,
    repo: &str,
    arm: LoopArm,
    status: &str,
    jsonl: &str,
    root: &Path,
    gold: &[String],
    gold_tests: &[String],
    pack: &str,
    scc_calls: usize,
    substitution_rate: f64,
    contaminated: bool,
) -> LoopTaskRow {
    let m = crate::benchagent::metrics_from_jsonl(jsonl, root, task, gold);
    let gold_set: BTreeSet<String> = gold.iter().cloned().collect();
    let opened: Vec<String> = jsonl
        .lines()
        .filter_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).ok()?;
            let input = v.get("input").or_else(|| {
                v.get("item")
                    .and_then(|i| i.get("arguments"))
            })?;
            input.get("file_path").and_then(|p| p.as_str()).map(|s| s.to_string())
        })
        .collect();
    let hits = opened
        .iter()
        .filter(|f| gold_matches(f, &gold_set))
        .count();
    let localization = if gold.is_empty() {
        1.0
    } else {
        hits as f64 / gold.len() as f64
    };
    let first_correct_rank = opened.iter().position(|f| gold_matches(f, &gold_set)).map(|i| i + 1);
    let search_calls = m.search_tool_calls;
    let subst = if scc_calls + search_calls == 0 {
        substitution_rate
    } else {
        scc_calls as f64 / (scc_calls + search_calls) as f64
    };
    let (tests_localization, tests_hit, tests_gold) = score_tests(pack, gold_tests);
    LoopTaskRow {
        task: task.to_string(),
        repo: repo.to_string(),
        arm: arm.as_str().to_string(),
        status: status.to_string(),
        localization,
        first_correct_rank,
        files_opened: m.files_opened,
        search_calls,
        scc_calls,
        substitution_rate: subst,
        contaminated,
        protocol: "explore".into(),
        read_calls: m.read_tool_calls,
        first_correct_ms: m.first_correct_ms,
        wrong_first: m.wrong_first_locations,
        jsonl_events: m.total_tool_calls,
        tests_localization,
        tests_hit,
        tests_gold,
    }
}

// trace:exempt reason=internal-detail
pub fn print_loop_summary(s: &LoopSummary) {
    let proto = if s.protocol == "explore" {
        "explore JSONL"
    } else {
        "locator"
    };
    println!("scc bench loop — {proto} arms (clustered localization; production ranker unchanged)");
    if let Some(bin) = &s.ripwire_bin {
        println!("  ripwire: {bin}");
    } else {
        println!("  ripwire: skipped (no binary; set RIPWIRE_BIN or --ripwire-bin)");
    }
    println!(
        "  {:<12} {:>8} {:>12} {:>10} {:>10} {:>8} {:>8} {:>8} {:>10} {:>8}",
        "arm", "status", "clustered", "pooled", "subst", "search", "read", "tasks", "tests_cl", "tests_n"
    );
    for a in &s.arms {
        println!(
            "  {:<12} {:>8} {:>12.3} {:>10.3} {:>10.3} {:>8.2} {:>8.2} {:>8} {:>10.3} {:>8}",
            a.arm,
            a.status,
            a.clustered_localization,
            a.pooled_localization,
            a.mean_substitution,
            a.mean_search,
            a.mean_read,
            a.tasks,
            a.clustered_tests,
            a.tests_tasks
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // trace:v1 id=test.scc.cli.bench-loop verifies=REQ-agent-loop-three-way exercises=impl.scc.cli.bench-loop
    fn agent_loop_scc_beats_or_matches_baseline_on_fixture() {
        let opts = LoopOptions {
            k: 10,
            repo_filter: Some("http-service-python".into()),
            ripwire_bin: None,
            explore: false,
            agent_cmd: None,
        };
        let summary = run_agent_loop(&[LoopArm::Baseline, LoopArm::Scc, LoopArm::Ripwire], &opts)
            .expect("loop");
        let base = summary
            .arms
            .iter()
            .find(|a| a.arm == "baseline")
            .expect("baseline");
        let scc = summary.arms.iter().find(|a| a.arm == "scc").expect("scc");
        assert!(base.tasks > 0, "baseline must run tasks");
        assert!(scc.tasks > 0, "scc must run tasks");
        assert_eq!(scc.mean_substitution, 1.0);
        assert_eq!(base.mean_substitution, 0.0);
        assert!(
            scc.clustered_localization + 1e-9 >= base.clustered_localization,
            "SCC pack locator must not lose to lexical baseline on this fixture: scc={} base={}",
            scc.clustered_localization,
            base.clustered_localization
        );
        let rw = summary.arms.iter().find(|a| a.arm == "ripwire").expect("ripwire arm");
        if summary.ripwire_bin.is_none() {
            assert_eq!(rw.status, "skipped");
        }
        assert!(
            summary.per_task.iter().all(|r| !r.contaminated),
            "harness must not inject SCC_GOLD"
        );
    }

    #[test]
    // trace:exempt reason=internal-detail
    fn clustered_mean_is_not_pooled() {
        let mut m = BTreeMap::new();
        m.insert("a".into(), vec![1.0, 1.0, 1.0, 1.0]);
        m.insert("b".into(), vec![0.0]);
        let c = clustered_mean(&m);
        let pooled = 4.0 / 5.0;
        assert!((c - 0.5f64).abs() < 1e-9, "clustered={c}");
        assert!((pooled - 0.8f64).abs() < 1e-9);
    }

    #[test]
    // trace:v1 id=test.scc.cli.bench-loop-ripwire verifies=REQ-agent-loop-three-way exercises=impl.scc.cli.bench-loop
    fn agent_loop_ripwire_runs_when_binary_is_configured() {
        let gxx = PathBuf::from("/tmp/vendor/ripwire/build-gxx/ripwire");
        let cmake = PathBuf::from("/tmp/vendor/ripwire/build/ripwire");
        let bin = if gxx.is_file() {
            Some(gxx)
        } else if cmake.is_file() {
            Some(cmake)
        } else {
            None
        };
        let opts = LoopOptions {
            k: 10,
            repo_filter: Some("http-service-python".into()),
            ripwire_bin: bin,
            explore: false,
            agent_cmd: None,
        };
        let summary = run_agent_loop(&[LoopArm::Ripwire], &opts).expect("loop");
        let rw = summary
            .arms
            .iter()
            .find(|a| a.arm == "ripwire")
            .expect("ripwire arm");
        if summary.ripwire_bin.is_some() {
            assert_eq!(
                rw.status, "ran",
                "a present Ripwire binary must be scored, never skipped as an SCC win"
            );
            assert!(rw.tasks > 0);
        } else {
            assert_eq!(rw.status, "skipped");
        }
    }

    #[test]
    // trace:exempt reason=internal-detail
    fn ripwire_p_attr_paths_parse() {
        let xml = r#"<ctx><d p="src/server.py" n="handle"/><d p="src/db.py"/></ctx>"#;
        let indexed = vec!["src/server.py".into(), "src/db.py".into()];
        let got = files_from_ripwire(xml, &indexed, 10);
        assert_eq!(got, vec!["src/server.py", "src/db.py"]);
    }

    #[test]
    // trace:v1 id=test.scc.cli.explore-handles verifies=REQ-implement-phase-13-of-scc-x-ripwire-lessons-1-go-extract-time-recei exercises=impl.scc.cli.explore-handles
    fn explore_prefers_fetch_handles_and_refuses_stale() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.py"), "print(1)\n").unwrap();
        let hash = scc_core::fnv1a64_hex(b"print(1)\n");
        let fresh = scc_core::ContentHandle::for_file("r", "e", "a.py", &hash).to_string();
        let indexed = vec!["a.py".into()];
        let pack = format!("FETCH\n- a.py handle={fresh}\n# a.py mentioned");
        let got = files_from_pack(&pack, root, &indexed, 10);
        assert_eq!(got, vec!["a.py"]);
        let stale = scc_core::ContentHandle::for_file("r", "e", "a.py", "aaaaaaaaaaaaaaaa").to_string();
        let refused = files_from_pack(
            &format!("FETCH\n- a.py handle={stale}\n"),
            root,
            &indexed,
            10,
        );
        assert!(
            refused.is_empty(),
            "stale handle must not fall back to guessing a.py: {refused:?}"
        );
        let fallback = files_from_pack("IMPLEMENTATION\na.py\n", root, &indexed, 10);
        assert_eq!(fallback, vec!["a.py"]);
        std::fs::write(root.join("b.py"), "print(2)\n").unwrap();
        let indexed2 = vec!["a.py".into(), "b.py".into()];
        let mixed = files_from_pack(
            &format!("FETCH\n- a.py handle={stale}\nIMPLEMENTATION\nb.py\n"),
            root,
            &indexed2,
            10,
        );
        assert_eq!(
            mixed,
            vec!["b.py"],
            "stale a.py must stay refused while other paths can still fill: {mixed:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.cli.bench-explore verifies=REQ-implement-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing exercises=impl.scc.cli.bench-explore
    fn explore_protocol_emits_jsonl_metrics_and_scc_does_not_grep() {
        let opts = LoopOptions {
            k: 10,
            repo_filter: Some("http-service-python".into()),
            ripwire_bin: None,
            explore: true,
            agent_cmd: None,
        };
        let summary = run_agent_loop(&[LoopArm::Baseline, LoopArm::Scc, LoopArm::Ripwire], &opts)
            .expect("explore");
        assert_eq!(summary.protocol, "explore");
        let base = summary
            .arms
            .iter()
            .find(|a| a.arm == "baseline")
            .expect("baseline");
        let scc = summary.arms.iter().find(|a| a.arm == "scc").expect("scc");
        assert!(base.tasks > 0);
        assert!(scc.tasks > 0);
        assert!(
            base.mean_search >= 1.0,
            "baseline pack-consumer must grep: search={}",
            base.mean_search
        );
        assert_eq!(
            scc.mean_search, 0.0,
            "SCC pack-consumer must not grep after task_context"
        );
        assert!(scc.mean_read > 0.0, "SCC must read pack files");
        assert_eq!(scc.mean_substitution, 1.0);
        assert_eq!(base.mean_substitution, 0.0);
        assert!(
            summary
                .per_task
                .iter()
                .filter(|r| r.arm == "scc")
                .all(|r| r.protocol == "explore" && r.jsonl_events > 0 && r.first_correct_ms.is_some()),
            "SCC explore rows must carry JSONL first-correct: {:?}",
            summary.per_task
        );
        let rw = summary.arms.iter().find(|a| a.arm == "ripwire").expect("ripwire");
        if summary.ripwire_bin.is_none() {
            assert_eq!(rw.status, "skipped");
        }
        assert!(summary.per_task.iter().all(|r| !r.contaminated));
        let scc_tests: Vec<_> = summary
            .per_task
            .iter()
            .filter(|r| r.arm == "scc" && r.tests_localization.is_some())
            .collect();
        assert!(
            !scc_tests.is_empty(),
            "http-service-python has gold tests; SCC must report tests_localization"
        );
        assert!(
            summary
                .per_task
                .iter()
                .filter(|r| r.arm == "baseline" && r.tests_localization.is_some())
                .all(|r| r.tests_localization == Some(0.0)),
            "baseline has no tests_to_run list and must score 0 when gold tests exist"
        );
        assert!(
            summary
                .per_task
                .iter()
                .filter(|r| r.arm == "scc" && r.tests_gold == 0)
                .all(|r| r.tests_localization.is_none()),
            "empty gold tests must be omitted, not scored as 1.0"
        );
    }

    #[test]
    // trace:v1 id=test.scc.cli.loop-tests verifies=REQ-implement-phase-18-of-scc-x-ripwire-lessons-score-tests-to-run-in-the exercises=impl.scc.cli.loop-tests
    fn tests_from_pack_parses_scc_rows_and_ripwire_xml() {
        let scc = "TESTS\n- test_normalization_preserves_raw (tests/test_transcripts.py) — direct\nFETCH\n- src/server.py handle=scc://abc\n";
        let got = tests_from_pack(scc);
        assert!(
            got.iter()
                .any(|s| s == "test_normalization_preserves_raw"),
            "SCC name: {got:?}"
        );
        assert!(
            got.iter().any(|s| s == "tests/test_transcripts.py"),
            "SCC file: {got:?}"
        );
        assert!(
            !got.iter().any(|s| s.contains("handle=")),
            "FETCH rows are not tests: {got:?}"
        );

        let rw = r#"<ctx><test p="tests/test_transcripts.py">test_transcripts</test></ctx>"#;
        let rw_got = tests_from_pack(rw);
        assert_eq!(rw_got, vec!["tests/test_transcripts.py"]);
        assert!(
            !test_name_matches("tests/test_transcripts.py", "test_normalization_preserves_raw"),
            "Ripwire file-only rows must not match a gold function name"
        );
    }

    #[test]
    // trace:v1 id=test.scc.cli.score-tests verifies=REQ-implement-phase-18-of-scc-x-ripwire-lessons-score-tests-to-run-in-the exercises=impl.scc.cli.loop-tests
    fn score_tests_omits_empty_gold_and_zeros_baseline() {
        assert_eq!(score_tests("whatever", &[]).0, None);
        let gold = vec!["test_foo".into()];
        assert_eq!(score_tests("", &gold), (Some(0.0), 0, 1));
        let pack = "- test_foo (tests/t.py) — direct\n";
        assert_eq!(score_tests(pack, &gold), (Some(1.0), 1, 1));
        let ascii = "- test_foo (tests/t.py) -- import\n";
        assert_eq!(score_tests(ascii, &gold), (Some(1.0), 1, 1));
    }

    #[test]
    // trace:v1 id=test.scc.cli.loop-test-match verifies=REQ-implement-phase-20-of-scc-x-ripwire-lessons-raise-tests-to-run-recall exercises=impl.scc.cli.loop-test-match
    fn score_tests_matches_kebab_it_titles_not_ripwire_files() {
        let gold = vec!["computes-order-totals-from-line-items".into()];
        let pack = "- computes order totals from line items (tests/orders.test.ts) — import\n";
        assert_eq!(score_tests(pack, &gold), (Some(1.0), 1, 1));
        assert_eq!(
            score_tests(r#"<test p="tests/orders.test.ts"/>"#, &gold),
            (Some(0.0), 0, 1)
        );
        let gold_api = vec!["joins-user-names-from-the-api-response".into()];
        let pack_api =
            "- joins user names from the API response (web/view.test.ts) — import\n";
        assert_eq!(score_tests(pack_api, &gold_api), (Some(1.0), 1, 1));
    }

    #[test]
    // trace:v1 id=test.scc.cli.loop-test-ts verifies=REQ-implement-phase-20-of-scc-x-ripwire-lessons-raise-tests-to-run-recall exercises=impl.scc.cli.loop-test-match
    fn tests_to_run_hits_typescript_it_titles() {
        let opts = LoopOptions {
            k: 10,
            repo_filter: Some("large-ts".into()),
            ripwire_bin: None,
            explore: false,
            agent_cmd: None,
        };
        let summary = run_agent_loop(&[LoopArm::Scc], &opts).expect("loop");
        let hits: Vec<_> = summary
            .per_task
            .iter()
            .filter(|r| r.arm == "scc" && r.tests_localization == Some(1.0))
            .map(|r| r.task.as_str())
            .collect();
        assert!(
            hits.contains(&"large-ts.order-total-invariant"),
            "SCC TESTS must list the it() title that gold kebab-case names: {hits:?} rows={:?}",
            summary.per_task
        );
        let qw = LoopOptions {
            k: 10,
            repo_filter: Some("queue-worker-ts".into()),
            ripwire_bin: None,
            explore: false,
            agent_cmd: None,
        };
        let qw_summary = run_agent_loop(&[LoopArm::Scc], &qw).expect("queue-worker");
        let qw_hits: Vec<_> = qw_summary
            .per_task
            .iter()
            .filter(|r| r.arm == "scc" && r.tests_localization == Some(1.0))
            .map(|r| r.task.as_str())
            .collect();
        assert!(
            qw_hits.contains(&"queue-worker.street-vocabulary"),
            "fixed relative imports must let import-reason tests_to_run fire: {qw_hits:?} rows={:?}",
            qw_summary.per_task
        );
        let api = LoopOptions {
            k: 10,
            repo_filter: Some("ts-api-web".into()),
            ripwire_bin: None,
            explore: false,
            agent_cmd: None,
        };
        let api_summary = run_agent_loop(&[LoopArm::Scc], &api).expect("ts-api-web");
        let api_hits: Vec<_> = api_summary
            .per_task
            .iter()
            .filter(|r| r.arm == "scc" && r.tests_localization == Some(1.0))
            .map(|r| r.task.as_str())
            .collect();
        assert!(
            api_hits.contains(&"ts-api-web.contract-field")
                && api_hits.contains(&"ts-api-web.creation-test"),
            "kebab gold must match it() titles that contain acronyms: {api_hits:?} rows={:?}",
            api_summary.per_task
        );
    }
}
