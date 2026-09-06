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
}

#[derive(Debug, Clone, Serialize, Default)]
// trace:exempt reason=internal-detail
pub struct LoopSummary {
    pub arms: Vec<LoopArmSummary>,
    pub per_task: Vec<LoopTaskRow>,
    pub ripwire_bin: Option<String>,
}

#[derive(Debug, Clone, Default)]
// trace:exempt reason=internal-detail
pub struct LoopOptions {
    pub k: usize,
    pub repo_filter: Option<String>,
    pub ripwire_bin: Option<PathBuf>,
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
            let contaminated = std::env::var("SCC_GOLD").is_ok();
            for arm in arms {
                let row = run_arm(*arm, &root, &task.id, &task.repo, &task.goal, gold, &indexed, k, ripwire.as_deref(), contaminated)?;
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
    indexed: &[String],
    k: usize,
    ripwire: Option<&Path>,
    contaminated: bool,
) -> Result<LoopTaskRow, String> {
    match arm {
        LoopArm::Baseline => {
            let opened = baseline_open(root, goal, k);
            Ok(score_row(task_id, repo, arm, "ran", &opened, gold, 1, 0, 0.0, contaminated))
        }
        LoopArm::Scc => {
            let pack = scc_pack(root, goal)?;
            let opened = files_from_text(&pack, indexed, k);
            Ok(score_row(task_id, repo, arm, "ran", &opened, gold, 0, 1, 1.0, contaminated))
        }
        LoopArm::Ripwire => match ripwire {
            None => Ok(LoopTaskRow {
                task: task_id.to_string(),
                repo: repo.to_string(),
                arm: arm.as_str().to_string(),
                status: "skipped".into(),
                localization: 0.0,
                first_correct_rank: None,
                files_opened: 0,
                search_calls: 0,
                scc_calls: 0,
                substitution_rate: 0.0,
                contaminated,
            }),
            Some(bin) => {
                let out = run_ripwire(bin, root, goal)?;
                let opened = files_from_ripwire(&out, indexed, k);
                Ok(score_row(task_id, repo, arm, "ran", &opened, gold, 0, 0, 0.0, contaminated))
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
    }
}

// trace:exempt reason=internal-detail
fn gold_matches(opened: &str, gold: &BTreeSet<String>) -> bool {
    gold.iter().any(|g| opened == g || opened.ends_with(&format!("/{g}")) || g.ends_with(&format!("/{opened}")))
}

// trace:exempt reason=internal-detail
fn aggregate(rows: &[LoopTaskRow], arms: &[LoopArm]) -> Vec<LoopArmSummary> {
    let mut out = Vec::new();
    for arm in arms {
        let mine: Vec<&LoopTaskRow> = rows.iter().filter(|r| r.arm == arm.as_str()).collect();
        let skipped = mine.iter().filter(|r| r.status == "skipped").count();
        let ran: Vec<&LoopTaskRow> = mine.iter().copied().filter(|r| r.status == "ran").collect();
        let mut by_repo: BTreeMap<String, Vec<f64>> = BTreeMap::new();
        let mut pooled = 0.0;
        let mut subst = 0.0;
        for r in &ran {
            by_repo.entry(r.repo.clone()).or_default().push(r.localization);
            pooled += r.localization;
            subst += r.substitution_rate;
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
    let vendored = PathBuf::from("/tmp/vendor/ripwire/build/ripwire");
    if vendored.is_file() {
        return Some(vendored);
    }
    which("ripwire")
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

// trace:exempt reason=internal-detail
pub fn print_loop_summary(s: &LoopSummary) {
    println!("scc bench loop — locator arms (clustered localization; production ranker unchanged)");
    if let Some(bin) = &s.ripwire_bin {
        println!("  ripwire: {bin}");
    } else {
        println!("  ripwire: skipped (no binary; set RIPWIRE_BIN or --ripwire-bin)");
    }
    println!(
        "  {:<12} {:>8} {:>12} {:>10} {:>10} {:>8}",
        "arm", "status", "clustered", "pooled", "subst", "tasks"
    );
    for a in &s.arms {
        println!(
            "  {:<12} {:>8} {:>12.3} {:>10.3} {:>10.3} {:>8}",
            a.arm,
            a.status,
            a.clustered_localization,
            a.pooled_localization,
            a.mean_substitution,
            a.tasks
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
    // trace:exempt reason=internal-detail
    fn ripwire_p_attr_paths_parse() {
        let xml = r#"<ctx><d p="src/server.py" n="handle"/><d p="src/db.py"/></ctx>"#;
        let indexed = vec!["src/server.py".into(), "src/db.py".into()];
        let got = files_from_ripwire(xml, &indexed, 10);
        assert_eq!(got, vec!["src/server.py", "src/db.py"]);
    }
}
