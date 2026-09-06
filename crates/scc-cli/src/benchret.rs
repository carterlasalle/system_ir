//! Retrieval Recall@k / MRR over `benchmarks/tasks.json` gold.
//!
//! Compares inspectable ranking arms. Does not change production ranking.

use crate::benchctx::{copy_fixture, locate_fixtures_dir, BenchmarkCorpus, GroundTruth};
use scc_core::{mean_reciprocal_rank, recall_at_k, RankingArm};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
// trace:exempt reason=internal-detail
pub struct ArmScores {
    pub arm: String,
    pub recall_at_1: f64,
    pub recall_at_5: f64,
    pub recall_at_10: f64,
    pub mrr: f64,
    pub tasks: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
// trace:exempt reason=internal-detail
pub struct RetrievalSummary {
    pub arms: Vec<ArmScores>,
    pub per_task: Vec<TaskArmRow>,
}

#[derive(Debug, Clone, Serialize)]
// trace:exempt reason=internal-detail
pub struct TaskArmRow {
    pub task: String,
    pub arm: String,
    pub recall_at_1: f64,
    pub recall_at_5: f64,
    pub recall_at_10: f64,
    pub mrr: f64,
}

// trace:exempt reason=internal-detail
fn gold_keys(gt: &GroundTruth) -> HashSet<String> {
    let mut s = HashSet::new();
    for f in &gt.files {
        s.insert(format!("file:{f}"));
    }
    for n in &gt.symbols {
        s.insert(format!("symbol:{n}"));
    }
    for n in &gt.components {
        s.insert(format!("component:{n}"));
    }
    for n in &gt.routes {
        s.insert(format!("route:{n}"));
    }
    s
}

// trace:exempt reason=internal-detail
fn push_key(out: &mut Vec<String>, seen: &mut HashSet<String>, key: String) {
    if seen.insert(key.clone()) {
        out.push(key);
    }
}

// trace:exempt reason=internal-detail
fn keys_for_hit(kind: &str, name: &str, file: &str) -> Vec<String> {
    let mut v = vec![format!("{kind}:{name}")];
    if kind == "file" {
        if let Some(base) = name.rsplit('/').next() {
            if base != name {
                v.push(format!("file:{base}"));
            }
        }
    }
    if !file.is_empty() {
        v.push(format!("file:{file}"));
        if let Some(base) = file.rsplit('/').next() {
            v.push(format!("file:{base}"));
        }
    }
    if kind == "symbol" {
        if let Some(last) = name.rsplit(['.', ':']).next() {
            if last != name {
                v.push(format!("symbol:{last}"));
            }
        }
    }
    v
}

/// Run retrieval eval. `repo_filter` limits to one fixture repo id.
// trace:v1 id=impl.scc.cli.bench-retrieval work=WORK-ripwire-lessons-phase2 satisfies=REQ-retrieval-eval
pub fn run_retrieval_benchmark(
    k: usize,
    arms: &[RankingArm],
    min_recall_at_10: f64,
    repo_filter: Option<&str>,
) -> Result<RetrievalSummary, String> {
    let fixtures = locate_fixtures_dir().ok_or("cannot locate fixtures/ directory")?;
    let corpus_path = fixtures
        .parent()
        .map(|p| p.join("benchmarks/tasks.json"))
        .ok_or("cannot locate benchmarks/tasks.json")?;
    let text = std::fs::read_to_string(&corpus_path)
        .map_err(|e| format!("read {}: {e}", corpus_path.display()))?;
    let corpus: BenchmarkCorpus =
        serde_json::from_str(&text).map_err(|e| format!("tasks.json: {e}"))?;

    let mut by_repo: BTreeMap<String, Vec<&crate::benchctx::BenchTask>> = BTreeMap::new();
    for t in &corpus.tasks {
        if let Some(f) = repo_filter {
            if t.repo != f {
                continue;
            }
        }
        by_repo.entry(t.repo.clone()).or_default().push(t);
    }
    if by_repo.is_empty() {
        return Err("no retrieval tasks matched".into());
    }

    let mut summary = RetrievalSummary::default();
    let mut acc: BTreeMap<&'static str, (f64, f64, f64, f64, usize)> = BTreeMap::new();

    for (repo, tasks) in &by_repo {
        let src = fixtures.join(repo);
        if !src.is_dir() {
            continue;
        }
        let tmp = tempfile::TempDir::new().map_err(|e| e.to_string())?;
        let root = tmp.path().join("repo");
        copy_fixture(&src, &root);
        crate::commands::cmd_index(&root, true).map_err(|e| format!("index {repo}: {e}"))?;
        let store = crate::open_store(&root).map_err(|e| e.to_string())?;
        let config = crate::load_config(&root).map_err(|e| e.to_string())?;
        let comp = crate::compiler(&store, &config, Vec::new()).map_err(|e| e.to_string())?;
        let ctx = comp.ctx();
        let bm25 = ctx
            .store
            .meta_get("bm25_corpus")
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str::<scc_core::Bm25CorpusStats>(&raw).ok());

        for task in tasks {
            let gold = gold_keys(&task.ground_truth);
            if gold.is_empty() {
                continue;
            }
            for arm in arms {
                let ranked = ranked_keys_for_arm(
                    &ctx,
                    &root,
                    &task.goal,
                    *arm,
                    k.max(10),
                    bm25.as_ref(),
                );
                let r1 = recall_at_k(&ranked, &gold, 1);
                let r5 = recall_at_k(&ranked, &gold, 5);
                let r10 = recall_at_k(&ranked, &gold, 10);
                let mrr = mean_reciprocal_rank(&ranked, &gold);
                summary.per_task.push(TaskArmRow {
                    task: task.id.clone(),
                    arm: arm.as_str().to_string(),
                    recall_at_1: r1,
                    recall_at_5: r5,
                    recall_at_10: r10,
                    mrr,
                });
                let e = acc.entry(arm.as_str()).or_insert((0.0, 0.0, 0.0, 0.0, 0));
                e.0 += r1;
                e.1 += r5;
                e.2 += r10;
                e.3 += mrr;
                e.4 += 1;
            }
        }
    }

    for arm in arms {
        if let Some((r1, r5, r10, mrr, n)) = acc.get(arm.as_str()) {
            let count = *n;
            if count > 0 {
                let n = count as f64;
                summary.arms.push(ArmScores {
                    arm: arm.as_str().to_string(),
                    recall_at_1: r1 / n,
                    recall_at_5: r5 / n,
                    recall_at_10: r10 / n,
                    mrr: mrr / n,
                    tasks: count,
                });
            }
        }
    }

    if min_recall_at_10 > 0.0 {
        for a in &summary.arms {
            if a.recall_at_10 < min_recall_at_10 {
                return Err(format!(
                    "retrieval gate failed: {} recall@10 {:.3} < {min_recall_at_10}",
                    a.arm, a.recall_at_10
                ));
            }
        }
    }
    let _ = k;
    Ok(summary)
}

// trace:exempt reason=internal-detail
fn ranked_keys_for_arm(
    ctx: &scc_context::ContextCompiler<'_>,
    root: &Path,
    goal: &str,
    arm: RankingArm,
    limit: usize,
    corpus: Option<&scc_core::Bm25CorpusStats>,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    if arm == RankingArm::ProductionBlended {
        let budget = 8000usize;
        let request = scc_context::surface::SurfaceRequest {
            mode: scc_context::surface::SurfaceMode::Task {
                goal,
                visible: None,
            },
            budget,
            explain: false,
            policy: scc_context::surface::SurfacePolicy::defaults(budget),
            semantic: None,
        };
        let result = scc_context::build_surface(ctx, request);
        let map = scc_context::surface::compile_surface_map(ctx);
        for id in &result.rendered_ids {
            if let Some(e) = map.entries.iter().find(|ent| &ent.id == id) {
                let qn = e
                    .qualified_name
                    .rsplit(['.', ':'])
                    .next()
                    .unwrap_or(&e.qualified_name);
                for k in keys_for_hit("symbol", qn, &e.path) {
                    push_key(&mut out, &mut seen, k);
                }
                if let Some(c) = &e.component {
                    push_key(&mut out, &mut seen, format!("component:{c}"));
                }
            } else if let Some(ent) = ctx.view.entity(id) {
                let file = ent
                    .attributes
                    .get("file")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                for k in keys_for_hit(&ent.kind, &ent.name, file) {
                    push_key(&mut out, &mut seen, k);
                }
            }
        }
        return out;
    }
    let hits = scc_context::relevance::collect_relevance_candidates(
        &ctx.view, goal, limit, arm, Some(root), corpus,
    );
    for h in hits {
        let file = ctx
            .view
            .entity(&h.id)
            .and_then(|e| e.attributes.get("file"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        for k in keys_for_hit(&h.kind, &h.name, file) {
            push_key(&mut out, &mut seen, k);
        }
    }
    out
}

// trace:exempt reason=internal-detail
pub fn print_summary(s: &RetrievalSummary) {
    println!("scc bench retrieval — Recall@k / MRR (production ranker unchanged)");
    println!(
        "  {:<22} {:>8} {:>8} {:>9} {:>7} {:>6}",
        "arm", "R@1", "R@5", "R@10", "MRR", "tasks"
    );
    for a in &s.arms {
        println!(
            "  {:<22} {:>8.3} {:>8.3} {:>9.3} {:>7.3} {:>6}",
            a.arm, a.recall_at_1, a.recall_at_5, a.recall_at_10, a.mrr, a.tasks
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // trace:v1 id=test.scc.cli.retrieval-eval verifies=REQ-retrieval-eval exercises=impl.scc.cli.bench-retrieval
    fn retrieval_eval_runs_on_http_service_fixture() {
        let summary = run_retrieval_benchmark(
            10,
            &[RankingArm::LexicalThenGraph, RankingArm::QueryRouted],
            0.0,
            Some("http-service-python"),
        )
        .expect("retrieval bench");
        assert!(!summary.arms.is_empty());
        let lex = summary
            .arms
            .iter()
            .find(|a| a.arm == "lexical-then-graph")
            .expect("lexical arm");
        assert!(
            lex.recall_at_10 > 0.0,
            "lexical lens must retrieve some gold on the fixture: {lex:?}"
        );
        assert!(lex.mrr >= 0.0);
    }
}
