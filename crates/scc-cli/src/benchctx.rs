//! Context benchmark (docs/TEST_PLAN.md §84–87, M0/M4): score task packs
//! against the ground-truth corpus in `benchmarks/tasks.json`.
//!
//! Metrics:
//! - recall: ground-truth entities surfaced in the pack / total ground truth
//! - precision: ground-truth entities / pack entities of ground-truth kinds
//! - localization: ground-truth files mentioned in the pack content
//! - hallucination: nonexistent entities that must NOT surface
//! - budget: pack under the configured token budget

use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
// trace:v1 id=impl.crates-scc-cli-src-benchctx.benchmark-corpus
pub struct BenchmarkCorpus {
    pub version: u32,
    #[serde(default)]
    pub description: Option<String>,
    pub tasks: Vec<BenchTask>,
}

#[derive(Debug, Clone, Deserialize)]
// trace:v1 id=impl.crates-scc-cli-src-benchctx.bench-task
pub struct BenchTask {
    pub id: String,
    pub repo: String,
    pub goal: String,
    pub ground_truth: GroundTruth,
    #[serde(default)]
    pub hallucinations: Vec<Hallucination>,
}

#[derive(Debug, Clone, Default, Deserialize)]
// trace:v1 id=impl.crates-scc-cli-src-benchctx.ground-truth
pub struct GroundTruth {
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub symbols: Vec<String>,
    #[serde(default)]
    pub components: Vec<String>,
    #[serde(default)]
    pub data: Vec<String>,
    #[serde(default)]
    pub tests: Vec<String>,
    #[serde(default)]
    pub routes: Vec<String>,
    #[serde(default)]
    pub stores: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
// trace:v1 id=impl.crates-scc-cli-src-benchctx.hallucination
pub struct Hallucination {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, Default)]
// trace:v1 id=impl.crates-scc-cli-src-benchctx.task-result
pub struct TaskResult {
    pub id: String,
    pub recall: f64,
    pub precision: f64,
    pub localization: f64,
    pub tokens: usize,
    pub budget_ok: bool,
    pub hallucinations_hit: Vec<String>,
    pub missed: Vec<String>,
}

#[derive(Debug, Clone, Default)]
// trace:v1 id=impl.crates-scc-cli-src-benchctx.bench-summary
pub struct BenchSummary {
    pub tasks: usize,
    pub mean_recall: f64,
    pub mean_precision: f64,
    pub mean_localization: f64,
    pub budget_ok: usize,
    pub hallucination_violations: usize,
    pub results: Vec<TaskResult>,
}

/// Locate the fixtures directory: walk up from cwd; fall back to the
/// build-time manifest path (dev tooling).
// trace:v1 id=impl.crates-scc-cli-src-benchctx.locate-fixtures-dir
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

// trace:v1 id=impl.crates-scc-cli-src-benchctx.copy-fixture
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

/// Normalize a pack entity id to a `(kind, name)` pair.
// trace:v1 id=impl.crates-scc-cli-src-benchctx.normalize-id
fn normalize_id(id: &str) -> Option<(String, String)> {
    // repo://<repo>/<kind>/<rest...>
    let rest = id.strip_prefix("repo://")?;
    let mut parts = rest.splitn(3, '/');
    let _repo = parts.next()?;
    let kind = parts.next()?.to_string();
    let name_rest = parts.next()?;
    match kind.as_str() {
        "symbol" | "test" => {
            // file/name — the LAST segment is the encoded name
            let idx = name_rest.rfind('/')?;
            let name = scc_core::decode_component(&name_rest[idx + 1..]);
            if kind == "symbol" {
                Some((kind, name))
            } else {
                Some((kind, scc_core::sanitize_key(&name)))
            }
        }
        "file" | "component" | "data" | "route" | "store" | "topic" | "external_api" => {
            // these kinds use sanitize_key ids (lowercase, _ -> -)
            let name = scc_core::sanitize_key(&scc_core::decode_component(name_rest));
            Some((kind, name))
        }
        _ => None,
    }
}

// trace:v1 id=impl.crates-scc-cli-src-benchctx.score-task-public
pub fn score_task_public(
    repo_dir: &Path,
    task: &BenchTask,
) -> Result<TaskResult, String> {
    score_task(repo_dir, task)
}

// trace:v1 id=impl.crates-scc-cli-src-benchctx.score-task
fn score_task(
    repo_dir: &Path,
    task: &BenchTask,
) -> Result<TaskResult, String> {
    let tmp = tempfile::TempDir::new().map_err(|e| e.to_string())?;
    let root = tmp.path().join("repo");
    copy_fixture(repo_dir, &root);

    crate::commands::cmd_index(&root, true).map_err(|e| format!("index: {e}"))?;
    let artifact_json = crate::commands::cmd_context_task_json(&root, &task.goal, &[], &[], None)
        .map_err(|e| format!("task: {e}"))?;
    // The artifact is {pack, delta, delta_ids}; recall scores the PACK.
    let artifact: serde_json::Value =
        serde_json::from_str(&artifact_json).map_err(|e| format!("artifact json: {e}"))?;
    let pack = artifact["pack"].clone();

    let ids: Vec<String> = pack["entity_ids"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let content = pack["content"].as_str().unwrap_or("");
    let tokens = pack["tokens"].as_u64().unwrap_or(0) as usize;
    let budget = pack["budget"].as_u64().unwrap_or(u64::MAX) as usize;
    let budget_ok = tokens <= budget;

    // normalized pack entities by kind
    let mut pack_by_kind: std::collections::BTreeMap<String, BTreeSet<String>> =
        Default::default();
    for id in &ids {
        if let Some((kind, name)) = normalize_id(id) {
            pack_by_kind.entry(kind).or_default().insert(name);
        }
    }

    let gt = &task.ground_truth;
    let mut gt_total = 0usize;
    let mut gt_hit = 0usize;
    let mut missed: Vec<String> = Vec::new();
    let check = |kind: &str, names: &[String],
                     pack_by_kind: &std::collections::BTreeMap<String, BTreeSet<String>>,
                     gt_total: &mut usize, gt_hit: &mut usize, missed: &mut Vec<String>| {
        for name in names {
            *gt_total += 1;
            let norm = if kind == "symbol" {
                name.clone()
            } else {
                scc_core::sanitize_key(name)
            };
            let hit = pack_by_kind
                .get(kind)
                .map(|set| set.contains(&norm))
                .unwrap_or(false);
            if hit {
                *gt_hit += 1;
            } else {
                missed.push(format!("{kind}:{name}"));
            }
        }
    };
    check("file", &gt.files, &pack_by_kind, &mut gt_total, &mut gt_hit, &mut missed);
    check("symbol", &gt.symbols, &pack_by_kind, &mut gt_total, &mut gt_hit, &mut missed);
    check("component", &gt.components, &pack_by_kind, &mut gt_total, &mut gt_hit, &mut missed);
    check("data", &gt.data, &pack_by_kind, &mut gt_total, &mut gt_hit, &mut missed);
    check("test", &gt.tests, &pack_by_kind, &mut gt_total, &mut gt_hit, &mut missed);
    check("route", &gt.routes, &pack_by_kind, &mut gt_total, &mut gt_hit, &mut missed);
    check("store", &gt.stores, &pack_by_kind, &mut gt_total, &mut gt_hit, &mut missed);

    let recall = if gt_total == 0 { 1.0 } else { gt_hit as f64 / gt_total as f64 };

    // precision over pack entities of ground-truth kinds
    let gt_kinds: BTreeSet<&str> = ["file", "symbol", "component", "data", "test", "route", "store"]
        .iter()
        .copied()
        .collect();
    let gt_names: BTreeSet<String> = gt
        .files
        .iter()
        .chain(&gt.symbols)
        .chain(&gt.components)
        .chain(&gt.data)
        .chain(&gt.tests)
        .chain(&gt.routes)
        .chain(&gt.stores)
        .cloned()
        .collect();
    let mut pack_total = 0usize;
    let mut pack_hit = 0usize;
    for (kind, names) in &pack_by_kind {
        if !gt_kinds.contains(kind.as_str()) {
            continue;
        }
        for name in names {
            pack_total += 1;
            if gt_names.contains(name) {
                pack_hit += 1;
            }
        }
    }
    let precision = if pack_total == 0 { 0.0 } else { pack_hit as f64 / pack_total as f64 };

    // localization: gt files mentioned anywhere in the pack content
    let mut file_hits = 0usize;
    for f in &gt.files {
        if content.contains(f) {
            file_hits += 1;
        }
    }
    let localization = if gt.files.is_empty() {
        1.0
    } else {
        file_hits as f64 / gt.files.len() as f64
    };

    // hallucinations must NOT surface
    let mut hallucinations_hit: Vec<String> = Vec::new();
    for h in &task.hallucinations {
        let surfaced = pack_by_kind
            .get(h.kind.as_str())
            .map(|set| set.contains(&h.name))
            .unwrap_or(false)
            || content.contains(&h.name);
        if surfaced {
            hallucinations_hit.push(format!("{}:{}", h.kind, h.name));
        }
    }

    Ok(TaskResult {
        id: task.id.clone(),
        recall,
        precision,
        localization,
        tokens,
        budget_ok,
        hallucinations_hit,
        missed,
    })
}
// trace:v1 id=impl.scc.bench.context work=WORK-SCC-001 verifies=REQ-SCC-TEST

/// Run the full context benchmark against `benchmarks/tasks.json`.
// trace:v1 id=impl.crates-scc-cli-src-benchctx.run-context-benchmark
pub fn run_context_benchmark(min_recall: f64) -> Result<BenchSummary, String> {
    let fixtures = locate_fixtures_dir().ok_or("cannot locate fixtures/ directory")?;
    let corpus_path = fixtures
        .parent()
        .map(|p| p.join("benchmarks/tasks.json"))
        .or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(|p| p.parent())
                .map(|p| p.join("benchmarks/tasks.json"))
        })
        .ok_or("cannot locate benchmarks/tasks.json")?;
    let text = std::fs::read_to_string(&corpus_path).map_err(|e| e.to_string())?;
    let corpus: BenchmarkCorpus = serde_json::from_str(&text).map_err(|e| e.to_string())?;

    let mut summary = BenchSummary {
        tasks: corpus.tasks.len(),
        ..Default::default()
    };
    for task in &corpus.tasks {
        let repo_dir = fixtures.join(&task.repo);
        if !repo_dir.is_dir() {
            return Err(format!("fixture repo missing: {}", task.repo));
        }
        match score_task(&repo_dir, task) {
            Ok(r) => {
                summary.mean_recall += r.recall;
                summary.mean_precision += r.precision;
                summary.mean_localization += r.localization;
                if r.budget_ok {
                    summary.budget_ok += 1;
                }
                summary.hallucination_violations += r.hallucinations_hit.len();
                summary.results.push(r);
            }
            Err(e) => return Err(format!("task {} failed: {e}", task.id)),
        }
    }
    let n = corpus.tasks.len() as f64;
    summary.mean_recall /= n;
    summary.mean_precision /= n;
    summary.mean_localization /= n;

    // fail when the quality gate is not met
    if summary.mean_recall < min_recall {
        return Err(format!(
            "benchmark gate failed: mean recall {:.3} < {min_recall}",
            summary.mean_recall
        ));
    }
    if summary.hallucination_violations > 0 {
        return Err(format!(
            "benchmark gate failed: {} hallucination violation(s)",
            summary.hallucination_violations
        ));
    }
    Ok(summary)
}

// trace:v1 id=impl.crates-scc-cli-src-benchctx.print-summary
pub fn print_summary(s: &BenchSummary) {
    println!("scc bench context — ground-truth corpus");
    println!(
        "  tasks: {}   mean recall: {:.3}   mean precision: {:.3}   mean localization: {:.3}   budget-ok: {}/{}   hallucination violations: {}",
        s.tasks, s.mean_recall, s.mean_precision, s.mean_localization, s.budget_ok, s.tasks,
        s.hallucination_violations
    );
    println!("  {:<42} {:>7} {:>9} {:>12} {:>7} {:>6}", "task", "recall", "precision", "localization", "tokens", "budget");
    for r in &s.results {
        println!(
            "  {:<42} {:>7.3} {:>9.3} {:>12.3} {:>7} {:>6}",
            r.id, r.recall, r.precision, r.localization, r.tokens, if r.budget_ok { "ok" } else { "OVER" }
        );
        for m in &r.missed {
            println!("      missed: {m}");
        }
        for h in &r.hallucinations_hit {
            println!("      HALLUCINATION: {h}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
// trace:v1 id=impl.crates-scc-cli-src-benchctx.normalize-id-handles-encoding
    fn normalize_id_handles_encoding() {
        let id = "repo://repo/symbol/src/asr/client.ts/transcribe";
        let (kind, name) = normalize_id(id).unwrap();
        assert_eq!(kind, "symbol");
        assert_eq!(name, "transcribe");
        let id2 = "repo://repo/symbol/api/routes.ts/renderTranscript";
        let (_, name2) = normalize_id(id2).unwrap();
        assert_eq!(name2, "renderTranscript");
        let (k3, n3) = normalize_id("repo://repo/component/services").unwrap();
        assert_eq!(k3, "component");
        assert_eq!(n3, "services");
    }

    #[test]
// trace:v1 id=impl.crates-scc-cli-src-benchctx.corpus-is-valid-json
    fn corpus_is_valid_json() {
        let fixtures = locate_fixtures_dir().expect("fixtures dir");
        let path = fixtures
            .parent()
            .unwrap()
            .join("benchmarks/tasks.json");
        let text = std::fs::read_to_string(path).unwrap();
        let corpus: BenchmarkCorpus = serde_json::from_str(&text).unwrap();
        assert!(corpus.tasks.len() >= 14, "corpus needs >= 14 tasks");
        let repos: BTreeSet<&str> = corpus.tasks.iter().map(|t| t.repo.as_str()).collect();
        assert!(repos.len() >= 4, "corpus must span >= 4 fixture repos");
    }
}

#[cfg(test)]
mod match_tests {
    use super::*;

    #[test]
// trace:v1 id=impl.crates-scc-cli-src-benchctx.test-entity-matches-sanitized-gt
    fn test_entity_matches_sanitized_gt() {
        // pack id for a test entity
        let id = "repo://repo/test/tests/test-transcripts.py/test-normalization-preserves-raw";
        let (kind, name) = normalize_id(id).unwrap();
        assert_eq!(kind, "test");
        let gt_norm = scc_core::sanitize_key("test_normalization_preserves_raw");
        assert_eq!(name, gt_norm, "sanitized gt must equal normalized id name");
    }

    #[test]
// trace:v1 id=impl.crates-scc-cli-src-benchctx.file-ids-sanitize-underscores
    fn file_ids_sanitize_underscores() {
        let id = "repo://repo/file/tests/test-transcripts.py";
        let (kind, name) = normalize_id(id).unwrap();
        assert_eq!(kind, "file");
        assert_eq!(name, scc_core::sanitize_key("tests/test_transcripts.py"));
    }

    #[test]
// trace:v1 id=impl.crates-scc-cli-src-benchctx.route-ids-match
    fn route_ids_match() {
        let id = "repo://repo/route/get-/api/transcripts/-id";
        let (kind, name) = normalize_id(id).unwrap();
        assert_eq!(kind, "route");
        assert_eq!(name, scc_core::sanitize_key("GET /api/transcripts/:id"));
    }
}
