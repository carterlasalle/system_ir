//! Atlas recall benchmark (Wave 8 §57): deterministic recall of
//! independently documented ground truth against the startup System Atlas.
//!
//! For each repo in the corpus: index in place, compile the atlas pack, then
//! score every ground-truth key string (from `benchmarks/ground-truth/<name>.md`)
//! as a case-insensitive substring of the atlas content. Per-section recall
//! (components/entrypoints/flows/ownership/contracts, tests informational),
//! overall = equal-weighted mean of the five scored sections, and a quality
//! gate on the overall mean (floor 0.5 — real repos are messy; raise later).
//!
//! When `benchmarks/corpus/` is absent (or empty), the harness falls back to
//! the golden `fixtures/`: ground truth is synthesized from
//! `benchmarks/tasks.json`, fixture copies are indexed in a temp dir (the
//! golden fixtures are never written into), and the same recall pipeline runs.

use crate::benchctx::{BenchmarkCorpus, BenchTask};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Quality gate: overall mean recall must be >= this floor (Wave 8 §57).
pub const ATLAS_GATE: f64 = 0.5;

/// The five sections that count toward the overall score (tests are
/// informational only).
const SCORED_SECTIONS: [&str; 5] = [
    "components",
    "entrypoints",
    "flows",
    "ownership",
    "contracts",
];

/// Ground-truth sections parsed from `benchmarks/ground-truth/<name>.md`
/// (one `- <key string>` bullet per item).
#[derive(Debug, Clone, Default)]
pub struct GroundTruthDoc {
    pub components: Vec<String>,
    pub entrypoints: Vec<String>,
    pub flows: Vec<String>,
    pub ownership: Vec<String>,
    pub contracts: Vec<String>,
    pub tests: Vec<String>,
}

impl GroundTruthDoc {
    pub fn section(&self, name: &str) -> &Vec<String> {
        match name {
            "components" => &self.components,
            "entrypoints" => &self.entrypoints,
            "flows" => &self.flows,
            "ownership" => &self.ownership,
            "contracts" => &self.contracts,
            "tests" => &self.tests,
            _ => unreachable!("unknown section {name}"),
        }
    }

    fn section_mut(&mut self, name: &str) -> &mut Vec<String> {
        match name {
            "components" => &mut self.components,
            "entrypoints" => &mut self.entrypoints,
            "flows" => &mut self.flows,
            "ownership" => &mut self.ownership,
            "contracts" => &mut self.contracts,
            "tests" => &mut self.tests,
            _ => unreachable!("unknown section {name}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RepoRecall {
    pub repo: String,
    pub components: f64,
    pub entrypoints: f64,
    pub flows: f64,
    pub ownership: f64,
    pub contracts: f64,
    pub tests: f64,
    pub overall: f64,
    /// When set, the repo was not scored (missing dir / missing ground
    /// truth / index failure) and the recall fields are meaningless.
    pub skipped_reason: Option<String>,
    /// Ground-truth key strings absent from the atlas content
    /// (`section:key`), for diagnosing misses.
    pub missed: Vec<String>,
}

impl RepoRecall {
    fn skipped(repo: &str, reason: impl Into<String>) -> Self {
        RepoRecall {
            repo: repo.to_string(),
            skipped_reason: Some(reason.into()),
            ..Default::default()
        }
    }
}

impl Default for RepoRecall {
    fn default() -> Self {
        RepoRecall {
            repo: String::new(),
            components: 0.0,
            entrypoints: 0.0,
            flows: 0.0,
            ownership: 0.0,
            contracts: 0.0,
            tests: 0.0,
            overall: 0.0,
            skipped_reason: None,
            missed: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AtlasRecallReport {
    /// One row per requested repo, in sorted order; skipped repos carry
    /// `skipped_reason`.
    pub repos: Vec<RepoRecall>,
    /// Where the run came from: "benchmarks/corpus" or "fixtures fallback".
    pub mode: String,
    pub mean_components: f64,
    pub mean_entrypoints: f64,
    pub mean_flows: f64,
    pub mean_ownership: f64,
    pub mean_contracts: f64,
    /// Equal-weighted mean of the five scored sections over scored repos.
    pub mean_overall: f64,
    pub scored: usize,
    pub skipped: usize,
    pub gate_passed: bool,
}

/// Parse a ground-truth markdown doc into per-section key strings.
///
/// Accepts the Wave 8 corpus format (`## section` heading + `- item` bullets).
/// A bullet is either the bare key string or `<key string> — explanation`;
/// the explanation is not expected in atlas output, so only the key string
/// (before ` — `) is scored. Inline-code backticks are stripped.
pub fn parse_ground_truth(md: &str) -> GroundTruthDoc {
    let mut doc = GroundTruthDoc::default();
    let mut current: Option<&'static str> = None;
    for raw in md.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("## ") {
            current = match rest.trim().to_ascii_lowercase().as_str() {
                "components" => Some("components"),
                "entrypoints" => Some("entrypoints"),
                "flows" => Some("flows"),
                "ownership" => Some("ownership"),
                "contracts" => Some("contracts"),
                "tests" => Some("tests"),
                _ => None,
            };
            continue;
        }
        let Some(section) = current else { continue };
        let Some(item) = line
            .strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
        else {
            continue;
        };
        let item = item.trim().trim_matches('`').trim();
        if item.is_empty() {
            continue;
        }
        let key = match item.split_once(" — ") {
            Some((k, _)) => k.trim().trim_matches('`').trim(),
            None => item,
        };
        if key.is_empty() {
            continue;
        }
        doc.section_mut(section).push(key.to_string());
    }
    doc
}

/// Recall for one section: fraction of ground-truth key strings found as
/// case-insensitive substrings of the atlas content. An empty ground truth
/// scores 1.0 (nothing to miss).
fn section_recall(items: &[String], content_lc: &str) -> (f64, usize, usize) {
    if items.is_empty() {
        return (1.0, 0, 0);
    }
    let mut hit = 0usize;
    for item in items {
        if content_lc.contains(&item.to_ascii_lowercase()) {
            hit += 1;
        }
    }
    (hit as f64 / items.len() as f64, hit, items.len())
}

/// Index one repo in place and score its ground truth against the atlas.
pub fn score_repo(repo_dir: &Path, gt: &GroundTruthDoc) -> Result<RepoRecall, String> {
    crate::commands::cmd_index(repo_dir, true).map_err(|e| format!("index failed: {e}"))?;
    let store = crate::open_store(repo_dir).map_err(|e| format!("store: {e}"))?;
    let config = crate::load_config(repo_dir).map_err(|e| format!("config: {e}"))?;
    let stale = crate::stale_paths(&store).map_err(|e| format!("stale: {e}"))?;
    let comp = crate::compiler(&store, &config, stale).map_err(|e| format!("compiler: {e}"))?;
    let pack = comp.ctx().system_atlas(None);
    let content_lc = pack.content.to_ascii_lowercase();

    let (components, _, _) = section_recall(&gt.components, &content_lc);
    let (entrypoints, _, _) = section_recall(&gt.entrypoints, &content_lc);
    let (flows, _, _) = section_recall(&gt.flows, &content_lc);
    let (ownership, _, _) = section_recall(&gt.ownership, &content_lc);
    let (contracts, _, _) = section_recall(&gt.contracts, &content_lc);
    let (tests, _, _) = section_recall(&gt.tests, &content_lc);
    let overall = (components + entrypoints + flows + ownership + contracts) / 5.0;

    let mut missed: Vec<String> = Vec::new();
    for section in SCORED_SECTIONS.iter().chain(std::iter::once(&"tests")) {
        for item in gt.section(section) {
            if !content_lc.contains(&item.to_ascii_lowercase()) {
                missed.push(format!("{section}:{item}"));
            }
        }
    }

    let repo = repo_dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| repo_dir.display().to_string());
    Ok(RepoRecall {
        repo,
        components,
        entrypoints,
        flows,
        ownership,
        contracts,
        tests,
        overall,
        skipped_reason: None,
        missed,
    })
}

/// Run the recall benchmark over `repo_names` (sorted for deterministic
/// output). Repos whose corpus dir is missing, whose ground-truth doc is
/// missing, or whose index/atlas fails are recorded with `skipped_reason` —
/// this function never panics on missing dirs.
pub fn run_atlas_recall(
    corpus_dir: &Path,
    ground_truth_dir: &Path,
    repo_names: &[String],
) -> Result<AtlasRecallReport, String> {
    let mut names: Vec<&String> = repo_names.iter().collect();
    names.sort();

    let mut report = AtlasRecallReport {
        mode: format!("corpus: {}", corpus_dir.display()),
        ..Default::default()
    };
    for name in names {
        let repo_dir = corpus_dir.join(name);
        if !repo_dir.is_dir() {
            report.skipped += 1;
            report
                .repos
                .push(RepoRecall::skipped(name, "corpus dir missing"));
            continue;
        }
        let gt_path = ground_truth_dir.join(format!("{name}.md"));
        let gt = match std::fs::read_to_string(&gt_path) {
            Ok(text) => parse_ground_truth(&text),
            Err(_) => {
                report.skipped += 1;
                report.repos.push(RepoRecall::skipped(
                    name,
                    format!("ground truth missing: {}", gt_path.display()),
                ));
                continue;
            }
        };
        match score_repo(&repo_dir, &gt) {
            Ok(r) => {
                report.mean_components += r.components;
                report.mean_entrypoints += r.entrypoints;
                report.mean_flows += r.flows;
                report.mean_ownership += r.ownership;
                report.mean_contracts += r.contracts;
                report.scored += 1;
                report.repos.push(r);
            }
            Err(e) => {
                report.skipped += 1;
                report.repos.push(RepoRecall::skipped(name, e));
            }
        }
    }

    if report.scored > 0 {
        let n = report.scored as f64;
        report.mean_components /= n;
        report.mean_entrypoints /= n;
        report.mean_flows /= n;
        report.mean_ownership /= n;
        report.mean_contracts /= n;
        report.mean_overall = (report.mean_components
            + report.mean_entrypoints
            + report.mean_flows
            + report.mean_ownership
            + report.mean_contracts)
            / 5.0;
    }
    report.gate_passed = report.mean_overall >= ATLAS_GATE;
    Ok(report)
}

/// Top-level entry for `scc bench atlas`: locate the workspace, resolve the
/// corpus/ground-truth directories (or the fixtures fallback), and run.
pub fn run_atlas_bench(
    corpus: Option<&Path>,
    ground_truth: Option<&Path>,
) -> Result<AtlasRecallReport, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let root = crate::find_root(&cwd);
    let default_corpus = root.join("benchmarks").join("corpus");

    // Fixtures fallback: no corpus dir (or an empty one) -> run over the
    // golden fixtures with ground truth synthesized from tasks.json.
    if corpus.is_none() && repo_dirs(&default_corpus).is_empty() {
        return fixtures_fallback(&root);
    }
    if let Some(p) = corpus {
        if !p.is_dir() {
            return Err(format!("corpus dir not found: {}", p.display()));
        }
    }

    let corpus_dir = corpus.map(Path::to_path_buf).unwrap_or(default_corpus);
    let gt_dir = match ground_truth {
        Some(p) => p.to_path_buf(),
        None => root.join("benchmarks").join("ground-truth"),
    };
    let names = repo_dirs(&corpus_dir);
    run_atlas_recall(&corpus_dir, &gt_dir, &names)
}

/// Sorted non-hidden subdirectory names of `dir` (the corpus listing).
fn repo_dirs(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if entry.path().is_dir() {
            out.push(name);
        }
    }
    out.sort();
    out
}

/// Hermetic fixtures run: copy the golden fixtures into a temp corpus (the
/// real fixtures are never indexed into) and synthesize ground-truth docs
/// from `benchmarks/tasks.json`.
fn fixtures_fallback(root: &Path) -> Result<AtlasRecallReport, String> {
    let fixtures = locate_fixtures_dir().ok_or("cannot locate fixtures/ directory")?;
    let tasks_path = root.join("benchmarks").join("tasks.json");
    let text = std::fs::read_to_string(&tasks_path)
        .map_err(|e| format!("cannot read {}: {e}", tasks_path.display()))?;
    let corpus: BenchmarkCorpus =
        serde_json::from_str(&text).map_err(|e| format!("tasks.json parse: {e}"))?;

    let names: Vec<String> = corpus
        .tasks
        .iter()
        .map(|t| t.repo.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let tmp = tempfile::TempDir::new().map_err(|e| e.to_string())?;
    let tmp_corpus = tmp.path().join("corpus");
    let tmp_gt = tmp.path().join("ground-truth");
    std::fs::create_dir_all(&tmp_corpus).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&tmp_gt).map_err(|e| e.to_string())?;
    for name in &names {
        let src = fixtures.join(name);
        if src.is_dir() {
            copy_tree_skip_scc(&src, &tmp_corpus.join(name));
        }
        let doc = ground_truth_from_tasks(&corpus.tasks, name);
        std::fs::write(tmp_gt.join(format!("{name}.md")), doc.to_markdown())
            .map_err(|e| e.to_string())?;
    }

    let mut report = run_atlas_recall(&tmp_corpus, &tmp_gt, &names)?;
    report.mode = "fixtures fallback (ground truth from benchmarks/tasks.json)".to_string();
    Ok(report)
}

/// Fixtures-fallback ground truth per repo, synthesized from the task
/// corpus. Mapping: components -> components; routes -> entrypoints AND
/// contracts (HTTP routes are both); symbols proxy flow steps; stores + data
/// -> ownership (who owns the store/DB); tests -> tests (informational).
fn ground_truth_from_tasks(tasks: &[BenchTask], repo: &str) -> GroundTruthDoc {
    let mut doc = GroundTruthDoc::default();
    for t in tasks {
        if t.repo != repo {
            continue;
        }
        doc.components.extend(t.ground_truth.components.iter().cloned());
        for r in &t.ground_truth.routes {
            doc.entrypoints.push(r.clone());
            doc.contracts.push(r.clone());
        }
        doc.flows.extend(t.ground_truth.symbols.iter().cloned());
        doc.ownership.extend(t.ground_truth.stores.iter().cloned());
        doc.ownership.extend(t.ground_truth.data.iter().cloned());
        doc.tests.extend(t.ground_truth.tests.iter().cloned());
    }
    doc.dedupe();
    doc
}

impl GroundTruthDoc {
    /// Remove duplicates, preserving first-seen order.
    fn dedupe(&mut self) {
        for name in [
            "components",
            "entrypoints",
            "flows",
            "ownership",
            "contracts",
            "tests",
        ] {
            let mut seen: BTreeSet<String> = BTreeSet::new();
            self.section_mut(name).retain(|item| seen.insert(item.clone()));
        }
    }

    fn to_markdown(&self) -> String {
        let mut out = String::from("# fixtures fallback (synthesized from benchmarks/tasks.json)\n");
        for name in [
            "components",
            "entrypoints",
            "flows",
            "ownership",
            "contracts",
            "tests",
        ] {
            out.push_str(&format!("## {name}\n"));
            for item in self.section(name) {
                out.push_str(&format!("- {item}\n"));
            }
        }
        out
    }
}

/// Locate the fixtures directory: walk up from cwd; fall back to the
/// workspace-relative path (dev tooling).
pub fn locate_fixtures_dir() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join("fixtures").join("http-service-python").is_dir() {
            return Some(dir.join("fixtures"));
        }
        if !dir.pop() {
            break;
        }
    }
    let manifest: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidate = manifest
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("fixtures"))
        .filter(|p| p.join("http-service-python").is_dir());
    candidate
}

/// Copy a tree, skipping `.scc` state dirs (mirrors golden::copy_tree).
fn copy_tree_skip_scc(src: &Path, dst: &Path) {
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
            copy_tree_skip_scc(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
}

pub fn print_report(r: &AtlasRecallReport) {
    println!("scc bench atlas — startup-atlas recall vs independent ground truth (Wave 8 §57)");
    println!("  mode: {}", r.mode);
    println!("  gate: overall mean recall >= {ATLAS_GATE}");
    println!(
        "  {:<24} {:>10} {:>11} {:>7} {:>10} {:>10} {:>7} {:>8}  note",
        "repo",
        "components",
        "entrypoints",
        "flows",
        "ownership",
        "contracts",
        "tests",
        "overall"
    );
    for repo in &r.repos {
        match &repo.skipped_reason {
            Some(reason) => println!(
                "  {:<24} {:>10} {:>11} {:>7} {:>10} {:>10} {:>7} {:>8}  skipped: {reason}",
                repo.repo, "-", "-", "-", "-", "-", "-", "-"
            ),
            None => {
                println!(
                    "  {:<24} {:>10.3} {:>11.3} {:>7.3} {:>10.3} {:>10.3} {:>7.3} {:>8.3}",
                    repo.repo,
                    repo.components,
                    repo.entrypoints,
                    repo.flows,
                    repo.ownership,
                    repo.contracts,
                    repo.tests,
                    repo.overall
                );
                for m in &repo.missed {
                    println!("      missed: {m}");
                }
            }
        }
    }
    println!(
        "  {:<24} {:>10.3} {:>11.3} {:>7.3} {:>10.3} {:>10.3} {:>7} {:>8.3}",
        "mean",
        r.mean_components,
        r.mean_entrypoints,
        r.mean_flows,
        r.mean_ownership,
        r.mean_contracts,
        "-",
        r.mean_overall
    );
    println!("  scored: {}   skipped: {}", r.scored, r.skipped);
    println!(
        "  gate: {} (overall mean recall {:.3} >= {ATLAS_GATE})",
        if r.gate_passed { "PASS" } else { "FAIL" },
        r.mean_overall
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const GT_MD: &str = r#"# synth repo
> synthetic | python | service

## components
- root — the app root component
- services — business logic

## entrypoints
- GET /api/items — fetch items

## flows
- `handle_items` — entry handler

## ownership
- db.items — owned by services

## contracts
- POST /api/items

## tests
- test_create_item — creation test
"#;

    #[test]
    fn parses_ground_truth_sections() {
        let doc = parse_ground_truth(GT_MD);
        assert_eq!(doc.components, ["root", "services"]);
        assert_eq!(doc.entrypoints, ["GET /api/items"]);
        assert_eq!(doc.flows, ["handle_items"], "inline-code backticks stripped");
        assert_eq!(doc.ownership, ["db.items"]);
        assert_eq!(doc.contracts, ["POST /api/items"]);
        assert_eq!(doc.tests, ["test_create_item"]);
    }

    #[test]
    fn recall_counts_all_hit_partial_and_zero() {
        let content = "\
SYSTEM PURPOSE
get /api/items
handle_items
ARCHITECTURE
ROOT
SERVICES
DATA OWNERSHIP
services owns db.items
CONTRACTS
POST /api/items
test_create_item
";
        let lc = content.to_ascii_lowercase();
        let doc = parse_ground_truth(GT_MD);
        let (c, _, _) = section_recall(&doc.components, &lc);
        assert_eq!(c, 1.0);
        let (e, _, _) = section_recall(&doc.entrypoints, &lc);
        assert_eq!(e, 1.0);
        let (f, _, _) = section_recall(&doc.flows, &lc);
        assert_eq!(f, 1.0);
        let (o, _, _) = section_recall(&doc.ownership, &lc);
        assert_eq!(o, 1.0);
        let (ct, _, _) = section_recall(&doc.contracts, &lc);
        assert_eq!(ct, 1.0);
        let (t, _, _) = section_recall(&doc.tests, &lc);
        assert_eq!(t, 1.0);

        // zero: no ground-truth item is a substring of empty content
        let (z, _, _) = section_recall(&doc.components, "");
        assert_eq!(z, 0.0);

        // partial: "root" hits, "services" does not
        let (p, hit, total) = section_recall(&doc.components, "root only");
        assert_eq!((p, hit, total), (0.5, 1, 2));

        // empty ground truth scores 1.0 (nothing to miss)
        let empty = GroundTruthDoc::default();
        let (x, hit0, total0) = section_recall(&empty.components, "anything");
        assert_eq!((x, hit0, total0), (1.0, 0, 0));
    }

    #[test]
    fn run_atlas_recall_skips_missing_dirs_without_panicking() {
        let tmp = tempfile::TempDir::new().unwrap();
        let corpus = tmp.path().join("corpus");
        let gt = tmp.path().join("ground-truth");
        std::fs::create_dir_all(&corpus).unwrap();
        std::fs::create_dir_all(&gt).unwrap();
        std::fs::create_dir_all(corpus.join("repo-a")).unwrap();

        let names = ["repo-b".to_string(), "repo-a".to_string()];
        let report = run_atlas_recall(&corpus, &gt, &names).unwrap();
        assert_eq!(report.scored, 0);
        assert_eq!(report.skipped, 2);
        assert!(!report.gate_passed);
        assert_eq!(report.mean_overall, 0.0);
        // deterministic order regardless of input order
        assert_eq!(report.repos[0].repo, "repo-a");
        assert!(report.repos[0]
            .skipped_reason
            .as_deref()
            .unwrap()
            .contains("ground truth missing"));
        assert_eq!(report.repos[1].repo, "repo-b");
        assert!(report.repos[1]
            .skipped_reason
            .as_deref()
            .unwrap()
            .contains("corpus dir missing"));
    }

    #[test]
    fn run_atlas_recall_scores_synthetic_repo() {
        let tmp = tempfile::TempDir::new().unwrap();
        let corpus = tmp.path().join("corpus");
        let gt = tmp.path().join("ground-truth");
        std::fs::create_dir_all(&corpus).unwrap();
        std::fs::create_dir_all(&gt).unwrap();
        let repo_dir = corpus.join("synth");
        std::fs::create_dir_all(repo_dir.join("services")).unwrap();
        std::fs::write(
            repo_dir.join("main.py"),
            "from fastapi import FastAPI\nfrom services.items import ItemRepository\n\napp = FastAPI()\n\n\n@app.get(\"/api/items\")\ndef handle_items() -> list:\n    \"\"\"List all items.\"\"\"\n    repo = ItemRepository()\n    return repo.find_all()\n",
        )
        .unwrap();
        std::fs::write(
            repo_dir.join("services/items.py"),
            "class ItemRepository:\n    def find_all(self):\n        return []\n",
        )
        .unwrap();
        std::fs::write(
            gt.join("synth.md"),
            "## components\n- root\n- services\n## entrypoints\n- handle_items\n## flows\n- ItemRepository\n## ownership\n- zzz_nonexistent_store\n## contracts\n- GET /api/items\n- GET /api/zzz_nonexistent\n## tests\n- test_nonexistent\n",
        )
        .unwrap();

        let report =
            run_atlas_recall(&corpus, &gt, &["synth".to_string()]).unwrap();
        assert_eq!(report.scored, 1);
        assert_eq!(report.skipped, 0);
        assert!(report.gate_passed == (report.mean_overall >= ATLAS_GATE));
        let r = &report.repos[0];
        assert!(r.skipped_reason.is_none());
        // overall is the equal-weighted mean of the five scored sections
        let expect =
            (r.components + r.entrypoints + r.flows + r.ownership + r.contracts) / 5.0;
        assert!((r.overall - expect).abs() < 1e-9);
        // components / entrypoints / flows surface in the atlas
        assert_eq!(r.components, 1.0, "root + services components");
        assert_eq!(r.entrypoints, 1.0, "handle_items in flow steps");
        assert_eq!(r.flows, 1.0, "ItemRepository in flow steps");
        // the deliberately nonexistent items must be missed
        assert_eq!(r.ownership, 0.0);
        assert_eq!(r.contracts, 0.5, "GET /api/items hits, zzz misses");
        assert!(r.missed.contains(&"ownership:zzz_nonexistent_store".to_string()));
        assert!(r.missed.contains(&"contracts:GET /api/zzz_nonexistent".to_string()));
        assert!(!r.missed.contains(&"entrypoints:handle_items".to_string()));
    }

    #[test]
    fn fallback_ground_truth_maps_tasks_and_dedupes() {
        let task = serde_json::from_str::<BenchTask>(
            r#"{"id":"t1","repo":"r","goal":"g",
               "ground_truth":{"files":["a.py"],"symbols":["s1","s2"],"components":["root"],
                               "data":["db.x"],"routes":["GET /api/x"],"stores":["redis"],
                               "tests":["test_a"]},
               "hallucinations":[]}"#,
        )
        .unwrap();
        let doc = ground_truth_from_tasks(&[task.clone(), task], "r");
        assert_eq!(doc.components, ["root"]);
        assert_eq!(doc.entrypoints, ["GET /api/x"]);
        assert_eq!(doc.contracts, ["GET /api/x"]);
        assert_eq!(doc.flows, ["s1", "s2"]);
        assert_eq!(doc.ownership, ["redis", "db.x"]);
        assert_eq!(doc.tests, ["test_a"]);

        // re-parse the synthesized markdown through the same parser
        let doc2 = parse_ground_truth(&doc.to_markdown());
        assert_eq!(doc2.flows, doc.flows);
        assert_eq!(doc2.ownership, doc.ownership);
    }
}
