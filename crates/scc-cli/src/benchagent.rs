//! Agent-run recorder and baseline harness (SCC-002, docs/TEST_PLAN.md §9).
//!
//! Runs each ground-truth task through an external agent command and records
//! outcome metrics. Tool-level instrumentation (files opened, search
//! commands) requires the harness's own tracing; this recorder captures the
//! portable layer: wall time, exit status, output size, and per-task
//! pass/fail when the agent's output contains the task's ground-truth files.

use serde::Deserialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::benchctx::{locate_fixtures_dir, BenchTask, BenchmarkCorpus, GroundTruth};

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct AgentTaskResult {
    pub id: String,
    pub exit_ok: bool,
    pub duration_ms: u64,
    pub output_bytes: usize,
    /// ground-truth files mentioned in the agent's output (localization)
    pub files_surfaced: usize,
    pub files_total: usize,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct AgentBenchSummary {
    pub tasks: usize,
    pub passed: usize,
    pub mean_duration_ms: f64,
    pub mean_localization: f64,
    pub results: Vec<AgentTaskResult>,
}

/// `scc bench agent --cmd "<command>"` — the command receives the task goal
/// via the `SCC_GOAL` env var and the repo path as its working directory
/// (like `claude -p "$SCC_GOAL"` or `codex exec -- "$SCC_GOAL"`).
pub fn run_agent_benchmark(cmd: &str, min_files: f64) -> Result<AgentBenchSummary, String> {
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

    let mut summary = AgentBenchSummary {
        tasks: corpus.tasks.len(),
        ..Default::default()
    };
    for task in &corpus.tasks {
        let repo_dir = fixtures.join(&task.repo);
        let tmp = tempfile::TempDir::new().map_err(|e| e.to_string())?;
        let root = tmp.path().join("repo");
        copy_fixture_tree(&repo_dir, &root);
        // index first so the agent starts warm (matches the SCC baseline flow)
        crate::commands::cmd_index(&root, true).map_err(|e| e.to_string())?;

        let started = Instant::now();
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .env("SCC_GOAL", &task.goal)
            .current_dir(&root)
            .output()
            .map_err(|e| format!("spawn agent command: {e}"))?;
        let duration_ms = started.elapsed().as_millis() as u64;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let output = format!("{stdout}\n{stderr}");

        let files_surfaced = task
            .ground_truth
            .files
            .iter()
            .filter(|f| output.contains(f.as_str()))
            .count();
        summary.mean_localization +=
            files_surfaced as f64 / task.ground_truth.files.len().max(1) as f64;
        summary.mean_duration_ms += duration_ms as f64;
        if out.status.success() {
            summary.passed += 1;
        }
        summary.results.push(AgentTaskResult {
            id: task.id.clone(),
            exit_ok: out.status.success(),
            duration_ms,
            output_bytes: output.len(),
            files_surfaced,
            files_total: task.ground_truth.files.len(),
        });
    }
    let n = corpus.tasks.len() as f64;
    summary.mean_duration_ms /= n;
    summary.mean_localization /= n;

    if summary.mean_localization < min_files {
        return Err(format!(
            "agent benchmark gate failed: mean localization {:.3} < {min_files}",
            summary.mean_localization
        ));
    }
    Ok(summary)
}

fn copy_fixture_tree(src: &Path, dst: &Path) {
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
            copy_fixture_tree(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
}

pub fn print_agent_summary(s: &AgentBenchSummary) {
    println!("scc bench agent — ground-truth corpus through an external agent command");
    println!(
        "  tasks: {}   exit-ok: {}/{}   mean duration: {:.0} ms   mean localization: {:.3}",
        s.tasks, s.passed, s.tasks, s.mean_duration_ms, s.mean_localization
    );
    for r in &s.results {
        println!(
            "  {:<42} {} {:>8} ms {:>8} B  files {}/{}",
            r.id,
            if r.exit_ok { "ok  " } else { "FAIL" },
            r.duration_ms,
            r.output_bytes,
            r.files_surfaced,
            r.files_total
        );
    }
    println!("  (tool-level metrics — files opened, search commands — require harness tracing; this is the portable layer)");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn fake_agent_records_metrics() {
        // the fake agent echoes the goal and lists the repo (shows files)
        let summary = run_agent_benchmark("echo \"$SCC_GOAL\" && ls -R .", 0.0).unwrap();
        assert_eq!(summary.tasks, 21);
        assert!(summary.results.iter().all(|r| r.exit_ok));
        assert!(summary.mean_duration_ms > 0.0);
        let _ = PathBuf::new();
    }

    #[test]
    fn corpus_ground_truth_parses() {
        let fixtures = locate_fixtures_dir().unwrap();
        let path = fixtures.parent().unwrap().join("benchmarks/tasks.json");
        let corpus: BenchmarkCorpus =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert!(corpus.tasks.iter().all(|t| !t.goal.is_empty()));
        // GroundTruth fields must deserialize
        let gt: GroundTruth = serde_json::from_str(
            r#"{"files":["a.py"],"symbols":["f"],"components":["c"],"tests":["t"]}"#,
        )
        .unwrap();
        assert_eq!(gt.files, vec!["a.py"]);
    }
}
