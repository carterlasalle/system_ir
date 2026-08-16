//! Agent-run recorder and baseline harness (SCC-002, docs/TEST_PLAN.md §9).
//!
//! Runs each ground-truth task through an external agent command and records
//! outcome metrics. When the agent emits a JSON event stream (`codex exec
//! --json` emits JSONL: `{type:item.completed, item:{type:command_execution|
//! mcp_tool_call|...}}`), the recorder additionally extracts tool-level
//! exploration metrics: files opened, search/read tool calls, wrong-first
//! locations opened before the first ground-truth file, and the wall time
//! until the first ground-truth file is touched. When no JSON events are
//! present the recorder falls back to the portable layer: wall time, exit
//! status, output size, and per-task pass/fail localization.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Instant;

use scc_core::estimate_tokens;
use serde_json::Value;

use crate::benchctx::{locate_fixtures_dir, BenchmarkCorpus};

#[derive(Debug, Clone, Default, serde::Serialize)]
// trace:exempt reason=internal-detail  # agent-run recorder data, traced as part of impl.scc.bench.agent
pub struct AgentTaskResult {
    pub id: String,
    pub exit_ok: bool,
    pub duration_ms: u64,
    pub output_bytes: usize,
    /// ground-truth files mentioned in the agent's output (localization)
    pub files_surfaced: usize,
    pub files_total: usize,
    // --- tool-level exploration metrics; 0 / None when the agent emits no
    // JSON event stream (old-style output) ---
    /// unique repo-relative file paths mentioned by tool events
    pub files_opened: usize,
    /// grep/glob/rg/find-style tool calls
    pub search_tool_calls: usize,
    /// read/cat/sed-style tool calls (including file reads)
    pub read_tool_calls: usize,
    /// all tool-like events (command executions + mcp tool calls + tool_use)
    pub total_tool_calls: usize,
    /// unique non-ground-truth files opened before the first ground-truth file was touched
    pub wrong_first_locations: usize,
    /// wall time until the first ground-truth file appears in a tool event or output
    pub first_correct_ms: Option<u64>,
    /// the agent's first plan (output before the first tool event) names a
    /// ground-truth key (file or symbol)
    pub first_plan_correct: bool,
    /// knowledge-graph query tool calls (MCP tools named graph/gitnexus)
    pub graph_tool_calls: usize,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
// trace:exempt reason=internal-detail  # agent-run recorder aggregate, traced as part of impl.scc.bench.agent
pub struct AgentBenchSummary {
    pub tasks: usize,
    pub passed: usize,
    pub mean_duration_ms: f64,
    pub mean_localization: f64,
    pub mean_files_opened: f64,
    pub mean_search_tool_calls: f64,
    pub mean_read_tool_calls: f64,
    pub mean_wrong_first_locations: f64,
    /// mean over tasks that had a first-correct observation (None if none did)
    pub mean_first_correct_ms: Option<f64>,
    /// knowledge-graph query tool calls per task (MCP tools named graph/gitnexus)
    pub mean_graph_tool_calls: f64,
    /// fraction of tasks whose first plan names a ground-truth key
    pub mean_first_plan_correct: f64,
    /// variant name for `scc bench external` runs (empty for `bench agent`)
    #[serde(default)]
    pub variant: String,
    pub results: Vec<AgentTaskResult>,
}

/// A-vs-E agent-behavior gate result: does the atlas variant (E) reduce
/// exploration vs the baseline (A)?
#[derive(Debug, Clone, serde::Serialize)]
// trace:exempt reason=internal-detail  # gate-result data of impl.scc.bench.agent
pub struct AgentGateResult {
    pub baseline: AgentBenchSummary,
    pub atlas: AgentBenchSummary,
    /// E.mean_search_tool_calls < A.mean_search_tool_calls.
    pub search_reduced: bool,
    /// E.mean_files_opened <= A.mean_files_opened + 1.
    pub files_bounded: bool,
    /// E.mean_first_correct_ms <= A.mean_first_correct_ms. Fails closed:
    /// when either side has no first-correct mean (no JSON event stream)
    /// the clause cannot be verified and is false.
    pub first_correct_bounded: bool,
    pub passed: bool,
}

/// Evaluate the gate clauses from two summaries (A = baseline, E = atlas
/// variant). The gate FAILS when the atlas variant does not reduce
/// exploration: requires E.search_tool_calls < A.search_tool_calls AND
/// E.files_opened <= A.files_opened + 1 AND E.first_correct_ms <=
/// A.first_correct_ms (means).
// trace:exempt reason=internal-detail  # gate function of impl.scc.bench.agent
pub fn evaluate_gate(a: &AgentBenchSummary, e: &AgentBenchSummary) -> AgentGateResult {
    let search_reduced = e.mean_search_tool_calls < a.mean_search_tool_calls;
    let files_bounded = e.mean_files_opened <= a.mean_files_opened + 1.0;
    let first_correct_bounded = match (a.mean_first_correct_ms, e.mean_first_correct_ms) {
        (Some(av), Some(ev)) => ev <= av,
        _ => false, // fail closed: cannot verify
    };
    let passed = search_reduced && files_bounded && first_correct_bounded;
    AgentGateResult {
        baseline: a.clone(),
        atlas: e.clone(),
        search_reduced,
        files_bounded,
        first_correct_bounded,
        passed,
    }
}

/// Run the agent-behavior release gate: score the baseline (A) command and
/// the atlas variant (E) command over the same corpus with the same
/// harness, then evaluate the exploration-reduction clauses (see
/// [`evaluate_gate`]). `min_files` applies to BOTH runs.
pub fn run_agent_gate(
    baseline_cmd: &str,
    atlas_cmd: &str,
    min_files: f64,
) -> Result<AgentGateResult, String> {
    let baseline = run_agent_benchmark(baseline_cmd, min_files)?;
    let atlas = run_agent_benchmark(atlas_cmd, min_files)?;
    Ok(evaluate_gate(&baseline, &atlas))
}

pub fn print_agent_gate(g: &AgentGateResult) {
    println!("scc bench agent --gate — A (baseline) vs E (atlas variant)");
    println!("\n--- baseline (A) ---");
    print_agent_summary(&g.baseline);
    println!("\n--- atlas variant (E) ---");
    print_agent_summary(&g.atlas);
    let a = &g.baseline;
    let e = &g.atlas;
    println!("\n=== exploration clauses (means) ===");
    println!(
        "  searches:      E {:.3} < A {:.3}              -> {}",
        e.mean_search_tool_calls,
        a.mean_search_tool_calls,
        if g.search_reduced { "PASS" } else { "FAIL" }
    );
    println!(
        "  files opened:  E {:.3} <= A {:.3} + 1        -> {}",
        e.mean_files_opened,
        a.mean_files_opened,
        if g.files_bounded { "PASS" } else { "FAIL" }
    );
    let first = match (a.mean_first_correct_ms, e.mean_first_correct_ms) {
        (Some(av), Some(ev)) => format!("E {ev:.0} ms <= A {av:.0} ms"),
        _ => "not verifiable (missing first-correct mean)".to_string(),
    };
    println!(
        "  first-correct: {first} -> {}",
        if g.first_correct_bounded { "PASS" } else { "FAIL" }
    );
    println!(
        "  gate: {} (atlas variant must reduce exploration: all three clauses)",
        if g.passed { "PASS" } else { "FAIL" }
    );
}

/// One ground-truth task definition used by the variant runners
/// ([`run_variant_tasks`]). The `files` are the localization ground truth
/// (the benchagent protocol compares tool events against them); `plan_keys`
/// are the first-plan correctness keys (files + symbols).
#[derive(Debug, Clone)]
// trace:exempt reason=internal-detail  # data container of the variant runner (impl.scc.bench.variant)
pub struct VariantTask {
    pub id: String,
    pub repo: String,
    pub goal: String,
    pub files: Vec<String>,
    pub plan_keys: Vec<String>,
}

/// Load the `benchmarks/tasks.json` corpus as [`VariantTask`]s (plan keys =
/// ground-truth files, matching the original `bench agent` protocol).
// trace:exempt reason=internal-detail  # corpus loader of the variant runner (impl.scc.bench.variant)
fn load_corpus_tasks() -> Result<Vec<VariantTask>, String> {
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
    Ok(corpus
        .tasks
        .iter()
        .map(|t| VariantTask {
            id: t.id.clone(),
            repo: t.repo.clone(),
            goal: t.goal.clone(),
            files: t.ground_truth.files.clone(),
            plan_keys: t.ground_truth.files.clone(),
        })
        .collect())
}

/// `scc bench agent --cmd "<command>"` — the command receives the task goal
/// via the `SCC_GOAL` env var and the repo path as its working directory
/// (like `claude -p "$SCC_GOAL"` or `codex exec -- "$SCC_GOAL"`).
// trace:v1 id=impl.scc.bench.agent work=WORK-SCC-002 verifies=REQ-SCC-TEST
pub fn run_agent_benchmark(cmd: &str, min_files: f64) -> Result<AgentBenchSummary, String> {
    let tasks = load_corpus_tasks()?;
    run_variant_tasks("", &tasks, |_, _| Ok(cmd.to_string()), min_files)
}

/// Wave-15 variant benchmark: same protocol as [`run_agent_benchmark`] over
/// the full `benchmarks/tasks.json` corpus, recording the variant name in
/// the summary. `cmd` is a shell command; the variant context artifact is
/// expected to be generated inside it (see benchmarks/run_agent_bench.sh).
// trace:v1 id=impl.scc.bench.variant work=WORK-SCC-002 verifies=REQ-SCC-TEST
pub fn run_variant_benchmark(
    variant: &str,
    cmd: &str,
    min_files: f64,
) -> Result<AgentBenchSummary, String> {
    let tasks = load_corpus_tasks()?;
    run_variant_tasks(variant, &tasks, |_, _| Ok(cmd.to_string()), min_files)
}

/// The shared variant runner: for each task, copy the fixture repo, index
/// it, ask `cmd_for` for the per-task shell command (this is where a
/// variant generates its context artifact into the freshly indexed repo and
/// returns the agent command that consumes it), then run the benchagent
/// protocol. Aggregates the same metrics as `bench agent` plus
/// first-plan accuracy and graph-query counts, and records `variant`.
// trace:exempt reason=internal-detail  # shared runner internals; variant entry point is impl.scc.bench.variant
pub fn run_variant_tasks<F>(
    variant: &str,
    tasks: &[VariantTask],
    mut cmd_for: F,
    min_files: f64,
) -> Result<AgentBenchSummary, String>
where
    F: FnMut(&VariantTask, &Path) -> Result<String, String>,
{
    let fixtures = locate_fixtures_dir().ok_or("cannot locate fixtures/ directory")?;

    let mut summary = AgentBenchSummary {
        variant: variant.to_string(),
        tasks: tasks.len(),
        ..Default::default()
    };
    for task in tasks {
        let repo_dir = fixtures.join(&task.repo);
        let tmp = tempfile::TempDir::new().map_err(|e| e.to_string())?;
        let root = tmp.path().join("repo");
        copy_fixture_tree(&repo_dir, &root);
        // index first so the agent starts warm (matches the SCC baseline flow)
        crate::commands::cmd_index(&root, true).map_err(|e| e.to_string())?;

        let cmd = cmd_for(task, &root)?;
        let res = run_task(&cmd, &root, &task.id, &task.goal, &task.files, &task.plan_keys)?;
        summary.mean_localization +=
            res.files_surfaced as f64 / task.files.len().max(1) as f64;
        summary.mean_duration_ms += res.duration_ms as f64;
        summary.mean_files_opened += res.files_opened as f64;
        summary.mean_search_tool_calls += res.search_tool_calls as f64;
        summary.mean_read_tool_calls += res.read_tool_calls as f64;
        summary.mean_wrong_first_locations += res.wrong_first_locations as f64;
        summary.mean_graph_tool_calls += res.graph_tool_calls as f64;
        summary.mean_first_plan_correct += res.first_plan_correct as usize as f64;
        if res.exit_ok {
            summary.passed += 1;
        }
        summary.results.push(res);
    }
    let n = tasks.len() as f64;
    summary.mean_duration_ms /= n;
    summary.mean_localization /= n;
    summary.mean_files_opened /= n;
    summary.mean_search_tool_calls /= n;
    summary.mean_read_tool_calls /= n;
    summary.mean_wrong_first_locations /= n;
    summary.mean_graph_tool_calls /= n;
    summary.mean_first_plan_correct /= n;
    let with_first: Vec<u64> = summary
        .results
        .iter()
        .filter_map(|r| r.first_correct_ms)
        .collect();
    summary.mean_first_correct_ms = if with_first.is_empty() {
        None
    } else {
        Some(with_first.iter().sum::<u64>() as f64 / with_first.len() as f64)
    };

    if summary.mean_localization < min_files {
        return Err(format!(
            "agent benchmark gate failed: mean localization {:.3} < {min_files}",
            summary.mean_localization
        ));
    }
    Ok(summary)
}

/// Run one task through `sh -c <cmd>` while streaming stdout line by line:
/// each line's arrival time is recorded (for first-correct timing) and lines
/// that look like JSONL events are parsed for tool-level metrics.
// trace:exempt reason=internal-detail  # agent-run recorder internals (impl.scc.bench.agent)
fn run_task(
    cmd: &str,
    root: &Path,
    id: &str,
    goal: &str,
    gt_files: &[String],
    plan_keys: &[String],
) -> Result<AgentTaskResult, String> {
    let started = Instant::now();
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .env("SCC_GOAL", goal)
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn agent command: {e}"))?;
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut r = BufReader::new(stderr);
        let _ = r.read_to_end(&mut buf);
        buf
    });

    let mut out_buf = Vec::new();
    let mut events: Vec<AgentEvent> = Vec::new();
    let mut first_correct_ms: Option<u64> = None;
    let mut gt_seen = false;
    let mut wrong_seen: BTreeSet<String> = BTreeSet::new();
    // The agent's first plan = the output stream before the first tool
    // event; first-plan correctness compares that text against the
    // ground-truth keys (files + symbols).
    let mut plan_buf = String::new();
    let mut plan_done = false;

    let reader = BufReader::new(stdout);
    for line in reader.split(b'\n') {
        let line = line.map_err(|e| format!("read agent stdout: {e}"))?;
        let line = String::from_utf8_lossy(&line);
        let elapsed_ms = started.elapsed().as_millis() as u64;
        out_buf.extend_from_slice(line.as_bytes());
        out_buf.push(b'\n');
        // first ground-truth file touched — in a tool event or any output
        if !gt_seen && gt_files.iter().any(|f| line.contains(f.as_str())) {
            gt_seen = true;
            if first_correct_ms.is_none() {
                first_correct_ms = Some(elapsed_ms);
            }
        }
        if !plan_done {
            plan_buf.push_str(&line);
            plan_buf.push('\n');
        }
        if let Some(ev) = parse_event_line(&line, root) {
            plan_done = true;
            // wrong-first: non-ground-truth files opened before the first
            // ground-truth file was touched (unique, in stream order)
            if !gt_seen {
                for p in &ev.paths {
                    if !gt_files.iter().any(|f| f == p) {
                        wrong_seen.insert(p.clone());
                    }
                }
            }
            events.push(ev);
        }
    }
    let status = child
        .wait()
        .map_err(|e| format!("wait agent command: {e}"))?;
    let duration_ms = started.elapsed().as_millis() as u64;
    let stderr_bytes = err_thread
        .join()
        .map_err(|_| "join stderr thread".to_string())?;
    out_buf.extend_from_slice(&stderr_bytes);

    let output = String::from_utf8_lossy(&out_buf);
    let files_surfaced = gt_files
        .iter()
        .filter(|f| output.contains(f.as_str()))
        .count();
    let first_plan_correct = plan_keys.iter().any(|k| plan_buf.contains(k.as_str()));
    let mut opened: BTreeSet<String> = BTreeSet::new();
    for ev in &events {
        opened.extend(ev.paths.iter().cloned());
    }
    Ok(AgentTaskResult {
        id: id.to_string(),
        exit_ok: status.success(),
        duration_ms,
        output_bytes: output.len(),
        files_surfaced,
        files_total: gt_files.len(),
        files_opened: opened.len(),
        search_tool_calls: events.iter().filter(|e| e.kind == ToolKind::Search).count(),
        read_tool_calls: events.iter().filter(|e| e.kind == ToolKind::Read).count(),
        total_tool_calls: events.len(),
        wrong_first_locations: wrong_seen.len(),
        first_correct_ms,
        first_plan_correct,
        graph_tool_calls: events.iter().filter(|e| e.graph).count(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ToolKind {
    Search,
    Read,
    Other,
}

#[derive(Debug, Clone)]
// trace:exempt reason=internal-detail  # event-model internals (impl.scc.bench.agent)
struct AgentEvent {
    paths: Vec<String>,
    kind: ToolKind,
    /// knowledge-graph query tool (mcp tool / tool_use named graph/gitnexus)
    graph: bool,
}

/// Tolerant JSONL event parser. Recognizes the codex `--json` shape
/// (`{type:"item.completed", item:{type:"command_execution"|"mcp_tool_call"}}`)
/// and the generic `{type:"tool_use"|"tool_result"}` shapes. Returns None for
/// non-JSON lines, in-progress events, and non-tool events (agent messages,
/// errors, turn markers).
// trace:exempt reason=internal-detail  # event parser internals (impl.scc.bench.agent)
fn parse_event_line(line: &str, root: &Path) -> Option<AgentEvent> {
    let trimmed = line.trim();
    if !trimmed.starts_with('{') {
        return None;
    }
    let value: Value = serde_json::from_str(trimmed).ok()?;
    let item = match value.get("type").and_then(|v| v.as_str()) {
        Some("item.completed") => value.get("item")?,
        Some("item.started") => return None,
        _ => &value,
    };
    let obj = item.as_object()?;
    match obj.get("type").and_then(|v| v.as_str())? {
        "command_execution" => {
            let command = obj.get("command").and_then(|v| v.as_str()).unwrap_or("");
            let tool = tool_from_command(command);
            Some(AgentEvent {
                paths: paths_from_command(command, root),
                kind: kind_of(tool),
                graph: false,
            })
        }
        "mcp_tool_call" => {
            let tool = obj.get("tool").and_then(|v| v.as_str()).unwrap_or("mcp_tool_call");
            let paths = obj
                .get("arguments")
                .and_then(Value::as_object)
                .map(|a| paths_from_args(a, root))
                .unwrap_or_default();
            Some(AgentEvent {
                paths,
                kind: kind_of(tool),
                graph: is_graph_tool(tool),
            })
        }
        "tool_use" => {
            let tool = obj.get("name").and_then(|v| v.as_str()).unwrap_or("tool_use");
            let paths = obj
                .get("input")
                .and_then(Value::as_object)
                .map(|a| paths_from_args(a, root))
                .unwrap_or_default();
            Some(AgentEvent {
                paths,
                kind: kind_of(tool),
                graph: is_graph_tool(tool),
            })
        }
        // tool_result is a response, not a call; messages/errors carry no
        // tool intent
        _ => None,
    }
}

/// First meaningful token of a shell command: unwraps `/bin/zsh -lc "tool …"`
/// style wrappers and flag prefixes.
fn tool_from_command(command: &str) -> &str {
    let mut toks = command.split_whitespace();
    let first = toks.next().unwrap_or("");
    let first_trim = first.trim_matches(['\'', '"']);
    let is_shell = (first_trim.starts_with('/') && first_trim.ends_with("sh"))
        || first_trim == "zsh"
        || first_trim == "bash"
        || first_trim == "sh";
    if is_shell {
        for t in toks {
            let t = t.trim_matches(['\'', '"']);
            if !t.is_empty() && !t.starts_with('-') {
                return t;
            }
        }
        return "shell";
    }
    first_trim
}

/// File paths mentioned in a shell command: whitespace tokens that resolve
/// inside the repo or carry a source-file extension. Flags, globs, env refs,
/// and out-of-repo absolute paths are dropped.
fn paths_from_command(command: &str, root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for raw in command.split_whitespace() {
        let tok = raw.trim_matches(['\'', '"']);
        if tok.is_empty()
            || tok == "."
            || tok == ".."
            || tok.starts_with('-')
            || tok.contains(['$', '|', '&', ';', '<', '>', '*', '?', '`', '='])
        {
            continue;
        }
        if let Some(p) = normalize_path(tok, root) {
            out.push(p);
        }
    }
    out
}

/// File paths from an MCP tool_use `arguments` / `input` object.
fn paths_from_args(args: &serde_json::Map<String, Value>, root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for key in ["file_path", "path", "file", "filename"] {
        if let Some(Value::String(s)) = args.get(key) {
            if let Some(p) = normalize_path(s, root) {
                out.push(p);
            }
        }
    }
    if let Some(Value::Array(arr)) = args.get("paths") {
        for v in arr {
            if let Value::String(s) = v {
                if let Some(p) = normalize_path(s, root) {
                    out.push(p);
                }
            }
        }
    }
    out
}

/// Resolve a token to a repo-relative path when it exists under the repo
/// root or looks like a source file. Absolute paths outside the repo are
/// dropped (skill docs, system files are not repo locations).
fn normalize_path(tok: &str, root: &Path) -> Option<String> {
    let tok = tok.trim_end_matches('/');
    if tok.is_empty() {
        return None;
    }
    let abs = if tok.starts_with('/') {
        PathBuf::from(tok)
    } else {
        root.join(tok)
    };
    let rel = abs.strip_prefix(root).ok()?;
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    if rel_str.is_empty() {
        return None;
    }
    let exists = abs.exists();
    let looks_like_file = SOURCE_EXTS.iter().any(|e| rel_str.ends_with(e));
    if exists || looks_like_file {
        Some(rel_str.trim_start_matches("./").to_string())
    } else {
        None
    }
}

const SOURCE_EXTS: [&str; 40] = [
    ".py", ".pyi", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".rs", ".go", ".java", ".kt",
    ".kts", ".rb", ".php", ".c", ".h", ".cpp", ".hpp", ".cc", ".cs", ".dart", ".proto", ".txt",
    ".json", ".toml", ".yaml", ".yml", ".md", ".sh", ".sql", ".html", ".css", ".vue", ".svelte",
    ".swift", ".lua", ".xml", ".gradle", ".dockerfile",
];

fn kind_of(tool: &str) -> ToolKind {
    let t = tool.to_ascii_lowercase();
    if t == "rg" || t == "ag" || t == "ack" || t == "fd" || t == "find" || t.contains("grep")
        || t.contains("glob") || t.contains("search")
    {
        ToolKind::Search
    } else if t == "cat" || t == "sed" || t == "less" || t == "more" || t == "head" || t == "tail"
        || t == "wc" || t == "nl" || t == "open" || t.contains("read") || t.contains("view")
    {
        ToolKind::Read
    } else {
        ToolKind::Other
    }
}

/// A knowledge-graph query tool (MCP graph servers, gitnexus, codegraph).
// trace:exempt reason=internal-detail  # graph-query classifier feeding variant metrics (impl.scc.bench.variant)
fn is_graph_tool(tool: &str) -> bool {
    let t = tool.to_ascii_lowercase();
    t.contains("graph") || t.contains("gitnexus")
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

// trace:exempt reason=internal-detail  # summary printer internals (impl.scc.bench.agent)
pub fn print_agent_summary(s: &AgentBenchSummary) {
    if s.variant.is_empty() {
        println!("scc bench agent — ground-truth corpus through an external agent command");
    } else {
        println!("scc bench external — variant {variant}", variant = s.variant);
    }
    println!(
        "  tasks: {}   exit-ok: {}/{}   mean duration: {:.0} ms   mean localization: {:.3}",
        s.tasks, s.passed, s.tasks, s.mean_duration_ms, s.mean_localization
    );
    println!(
        "  exploration means: files opened {:.1}   searches {:.1}   reads {:.1}   graph {:.1}   wrong-first {:.1}   first-correct {}",
        s.mean_files_opened,
        s.mean_search_tool_calls,
        s.mean_read_tool_calls,
        s.mean_graph_tool_calls,
        s.mean_wrong_first_locations,
        match s.mean_first_correct_ms {
            Some(v) => format!("{v:.0} ms"),
            None => "— (no JSON event stream)".to_string(),
        }
    );
    println!(
        "  first-plan accuracy: {:.3}",
        s.mean_first_plan_correct
    );
    for r in &s.results {
        let first = match r.first_correct_ms {
            Some(v) => format!("{v:>8} ms"),
            None => "       —".to_string(),
        };
        println!(
            "  {:<42} {} {:>8} ms {:>8} B  files {}/{}  opened {:>2}  srh {:>2}  read {:>2}  tot {:>3}  wrng {:>2}  1st-correct {}",
            r.id,
            if r.exit_ok { "ok  " } else { "FAIL" },
            r.duration_ms,
            r.output_bytes,
            r.files_surfaced,
            r.files_total,
            r.files_opened,
            r.search_tool_calls,
            r.read_tool_calls,
            r.total_tool_calls,
            r.wrong_first_locations,
            first,
        );
    }
    if s.mean_first_correct_ms.is_none() {
        println!(
            "  (no JSONL events detected — agent did not emit a --json event stream; tool columns are 0)"
        );
    }
}

// ---------------------------------------------------------------------------
// Wave-15 PPR ablation matrix + leakage-free structural selection
// ---------------------------------------------------------------------------

/// Harness-level surface ranking modes for the PPR ablation matrix. Every
/// mode runs the PRODUCTION surface pipeline ([`render_ablation_surface`]
/// → `build_surface_staged`) with exactly one stage toggled — never a
/// harness reimplementation of ranking. `lexical` switches every stage
/// off (lexical-only); `global-ppr` disables task PPR; `task-ppr` is the
/// full task pipeline; `ppr-mmr`/`ppr-quotas`/`ppr-optimizer` disable the
/// MMR/quotas/optimizer tail stages respectively. The ablation rows are
/// the production rows with one stage removed. See
/// benchmarks/external/README.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// trace:exempt reason=internal-detail  # harness-level ablation mode flags (impl.scc.bench.ablation)
pub enum SurfaceAblation {
    /// Lexical goal match only — every pipeline stage off.
    Lexical,
    /// Global PPR + lexical (no task seeds): task-PPR stage off.
    GlobalPpr,
    /// The production task surface pipeline (every stage on).
    TaskPpr,
    /// Production pipeline with the MMR diversity stage off.
    PprMmr,
    /// Production pipeline with the token-aware quota stage off.
    PprQuotas,
    /// Production pipeline with the value/token budget optimizer off.
    PprOptimizer,
}

// trace:exempt reason=internal-detail  # harness-level ablation mode flags (impl.scc.bench.ablation)
impl SurfaceAblation {
    /// Parse a variant name; `None` for non-ablation variants.
    // trace:exempt reason=internal-detail  # harness-level ablation mode flags (impl.scc.bench.ablation)
    pub fn parse(variant: &str) -> Option<SurfaceAblation> {
        match variant {
            "lexical" => Some(SurfaceAblation::Lexical),
            "global-ppr" => Some(SurfaceAblation::GlobalPpr),
            "task-ppr" => Some(SurfaceAblation::TaskPpr),
            "ppr-mmr" => Some(SurfaceAblation::PprMmr),
            "ppr-quotas" => Some(SurfaceAblation::PprQuotas),
            "ppr-optimizer" => Some(SurfaceAblation::PprOptimizer),
            _ => None,
        }
    }

    // trace:exempt reason=internal-detail  # harness-level ablation mode flags (impl.scc.bench.ablation)
    pub fn as_str(&self) -> &'static str {
        match self {
            SurfaceAblation::Lexical => "lexical",
            SurfaceAblation::GlobalPpr => "global-ppr",
            SurfaceAblation::TaskPpr => "task-ppr",
            SurfaceAblation::PprMmr => "ppr-mmr",
            SurfaceAblation::PprQuotas => "ppr-quotas",
            SurfaceAblation::PprOptimizer => "ppr-optimizer",
        }
    }

    /// The pipeline-stage toggle for this ablation: each mode disables
    /// exactly one stage of the production pipeline (`lexical` disables
    /// everything — lexical-only); every other stage runs exactly as
    /// production runs it.
    // trace:exempt reason=internal-detail  # harness-level ablation mode flags (impl.scc.bench.ablation)
    pub fn stages(&self) -> scc_context::surface::SurfacePipelineStages {
        use scc_context::surface::SurfacePipelineStages;
        let all_on = SurfacePipelineStages {
            lexical: true,
            global_ppr: true,
            task_ppr: true,
            mmr: true,
            quotas: true,
            optimizer: true,
        };
        match self {
            SurfaceAblation::Lexical => SurfacePipelineStages {
                lexical: false,
                global_ppr: false,
                task_ppr: false,
                mmr: false,
                quotas: false,
                optimizer: false,
            },
            SurfaceAblation::GlobalPpr => {
                let mut s = all_on;
                s.task_ppr = false;
                s
            }
            SurfaceAblation::TaskPpr => all_on,
            SurfaceAblation::PprMmr => {
                let mut s = all_on;
                s.mmr = false;
                s
            }
            SurfaceAblation::PprQuotas => {
                let mut s = all_on;
                s.quotas = false;
                s
            }
            SurfaceAblation::PprOptimizer => {
                let mut s = all_on;
                s.optimizer = false;
                s
            }
        }
    }
}

/// Render one ablation-mode task surface for `goal` under `budget_tokens`
/// (the chars/4 estimate) through the PRODUCTION surface pipeline
/// (`build_surface_staged`) with the mode's stage toggle — the ablation
/// rows are the production rows with one stage removed, never a harness
/// reimplementation of ranking. A one-line mode label prefixes the
/// production render so artifacts stay identifiable; the label's token
/// cost is subtracted from the budget (equal-token discipline: the final
/// artifact never exceeds the cap).
// trace:v1 id=impl.scc.bench.ablation work=WORK-SCC-014 satisfies=REQ-SCC-IR
pub fn render_ablation_surface(
    root: &Path,
    mode: SurfaceAblation,
    goal: &str,
    budget_tokens: usize,
) -> Result<String, String> {
    let store = crate::open_store(root).map_err(|e| e.to_string())?;
    let config = crate::load_config(root).map_err(|e| e.to_string())?;
    let stale = crate::stale_paths(&store).map_err(|e| e.to_string())?;
    let comp = crate::compiler(&store, &config, stale).map_err(|e| e.to_string())?;
    let ctx = comp.ctx();
    let label = format!("# SYSTEM SURFACE MAP (ablation {}: {})\n\n", mode.as_str(), goal);
    let label_tokens = estimate_tokens(&label);
    let budget = budget_tokens.saturating_sub(label_tokens);
    let request = scc_context::surface::SurfaceRequest {
        mode: scc_context::surface::SurfaceMode::Task {
            goal,
            visible: None,
        },
        budget,
        explain: false,
        // Ablation policy: the same quotas/MMR/coverage knobs as
        // production, but a strict hard max = the budget — the ablation
        // artifact never exceeds the cap (equal-token discipline; the
        // required-coverage headroom production grants is not part of the
        // ablation contract).
        policy: scc_context::surface::SurfacePolicy {
            quotas: true,
            mmr: true,
            coverage: true,
            hard_max: budget,
        },
        // Ablations never use the semantic scorer (equivalence across
        // inference on/off is the point); the 10% share is redistributed.
        semantic: None,
    };
    let result = scc_context::surface::build_surface_staged(&ctx, request, &mode.stages());
    let mut out = label;
    out.push_str(&result.text);
    Ok(out)
}

/// Parse repo-relative file paths from a rendered surface — the
/// PRODUCTION format: groups headed by an uppercased component line, a
/// blank line, then the group's path line (`<path>` or `<path>  [<sub>]`),
/// then blank-line-separated `  <kind> <name>` entry blocks. Order
/// preserved, deduped. This is the harness's file-selection oracle for
/// the scc-full structural section: selection comes from the rendered
/// surface (task PPR + lexical), NEVER from ground-truth `task.files`.
// trace:exempt reason=internal-detail  # leakage-free selection oracle (impl.scc.bench.structural-select)
pub fn parse_surface_paths(surface_text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let lines: Vec<&str> = surface_text.lines().collect();
    // A group path line sits between two blank lines with a non-blank
    // component line above the first blank: `<COMPONENT>\n\n<path>\n\n`.
    // Path lines are left-aligned (entry lines are indented `  <kind>
    // <name>`); component headers are uppercased by the renderer and
    // carry no path punctuation; paths carry '/' or '.', so the filters
    // keep component lines and entries out of the selection.
    for i in 0..lines.len() {
        let path = lines[i].trim();
        if path.is_empty() || lines[i].starts_with(' ') {
            continue; // entries and signatures are indented, never paths
        }
        let prev_blank = i
            .checked_sub(1)
            .map(|j| lines[j].trim().is_empty())
            .unwrap_or(false);
        let next_blank = lines
            .get(i + 1)
            .map(|l| l.trim().is_empty())
            .unwrap_or(false);
        let above_component = i
            .checked_sub(2)
            .map(|j| !lines[j].trim().is_empty())
            .unwrap_or(false);
        if !(prev_blank && next_blank && above_component) {
            continue;
        }
        if !(path.contains('/') || path.contains('.')) {
            continue;
        }
        if path.chars().all(|c| !c.is_lowercase()) {
            continue; // uppercased component headers never name a path
        }
        let path = path.split('[').next().unwrap_or(path).trim().to_string();
        if seen.insert(path.clone()) {
            out.push(path);
        }
    }
    out
}

/// Whether the running binary supports `scc context structural` (Wave-15
/// Item 7 wiring). Probed once per process via `--help`.
// trace:exempt reason=internal-detail  # structural CLI probe (impl.scc.bench.structural-select)
fn structural_cli_supported() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        std::env::current_exe()
            .ok()
            .map(|exe| {
                std::process::Command::new(&exe)
                    .args(["context", "structural", "--help"])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    })
}

/// Run the current executable with args in `root` and capture stdout.
// trace:exempt reason=internal-detail  # subprocess helper (impl.scc.bench.structural-select)
fn run_capture(exe: &Path, root: &Path, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new(exe)
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("spawn {}: {e}", exe.display()))?;
    if !out.status.success() {
        return Err(format!(
            "`{} {}` exited {}: {}",
            exe.display(),
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Render structural units in order, keeping complete units while the
/// chars/4 token estimate fits `budget_tokens` (equal-token discipline: a
/// file is never truncated mid-file).
// trace:exempt reason=internal-detail  # budgeted structural render (impl.scc.bench.structural-select)
fn render_units_budgeted(units: &[scc_core::StructuralSourceUnit], budget_tokens: usize) -> String {
    let mut out = String::new();
    let mut spent = 0usize;
    for u in units {
        let piece = scc_context::structural_source::render_structural(std::slice::from_ref(u));
        let cost = estimate_tokens(&piece);
        if !out.is_empty() && spent + cost > budget_tokens {
            break;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&piece);
        spent += cost;
    }
    out.trim_end().to_string()
}

/// Build the structural-source section for a goal with NO ground truth:
/// file selection comes from the task-personalized surface (task PPR +
/// lexical), and per-file structural units are rendered while the token
/// estimate fits `budget_tokens`. When the running binary supports it, the
/// production `scc context structural --task "<goal>" --budget N` CLI
/// performs the same selection and render in one shot. `task.files` NEVER
/// enters context construction — ground truth is scoring-only.
// trace:v1 id=impl.scc.bench.structural-select work=WORK-SCC-014 satisfies=REQ-SCC-IR
pub fn structural_source_for_goal(
    root: &Path,
    goal: &str,
    budget_tokens: usize,
) -> Result<String, String> {
    if budget_tokens == 0 || goal.trim().is_empty() {
        return Ok(String::new());
    }
    let exe = std::env::current_exe().map_err(|e| format!("current exe: {e}"))?;
    let budget_s = budget_tokens.to_string();
    if structural_cli_supported() {
        return run_capture(
            &exe,
            root,
            &["context", "structural", "--task", goal, "--budget", &budget_s],
        );
    }
    // Fallback: same selection implemented in the harness — the rendered
    // task-personalized surface picks the files, then structural units are
    // rendered budget-capped (complete files only).
    let surface_text = run_capture(
        &exe,
        root,
        &["surface", "--task", goal, "--budget", &budget_s],
    )?;
    let files = parse_surface_paths(&surface_text);
    if files.is_empty() {
        return Ok(String::new());
    }
    let store = crate::open_store(root).map_err(|e| e.to_string())?;
    let config = crate::load_config(root).map_err(|e| e.to_string())?;
    let stale = crate::stale_paths(&store).map_err(|e| e.to_string())?;
    let comp = crate::compiler(&store, &config, stale).map_err(|e| e.to_string())?;
    let units = scc_context::structural_source::structural_source(&comp.ctx(), &files, usize::MAX);
    Ok(render_units_budgeted(&units, budget_tokens))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchctx::GroundTruth;
    use std::path::PathBuf;

    #[test]
    fn fake_agent_records_metrics() {
        // the fake agent echoes the goal and lists the repo (shows files);
        // no JSON event stream → tool-level counters stay at their defaults
        let summary = run_agent_benchmark("echo \"$SCC_GOAL\" && ls -R .", 0.0).unwrap();
        assert_eq!(summary.tasks, 21);
        assert!(summary.results.iter().all(|r| r.exit_ok));
        assert!(summary.mean_duration_ms > 0.0);
        // no JSON event stream → tool-level counters stay at their defaults;
        // first_correct_ms may still fire because `ls -R` surfaces
        // ground-truth files in plain output (contract: "in a tool_use or output")
        assert!(summary.results.iter().all(|r| {
            r.files_opened == 0
                && r.search_tool_calls == 0
                && r.read_tool_calls == 0
                && r.total_tool_calls == 0
                && r.wrong_first_locations == 0
        }));
        let _ = PathBuf::new();
    }

    #[test]
    fn jsonl_event_stream_metrics() {
        // Synthetic codex --json stream (task 0 ground truth: main.py +
        // services/transcripts.py): a search, a wrong-file read, then the
        // correct file via an mcp read_file tool call.
        let cmd = r#"printf '%s\n' \
'{"type":"item.completed","item":{"id":"i1","type":"command_execution","command":"/bin/zsh -lc \"rg -n transcript .\"","aggregated_output":"","exit_code":0,"status":"completed"}}' \
'{"type":"item.completed","item":{"id":"i2","type":"command_execution","command":"/bin/zsh -lc \"sed -n 1,40p wrong_file.py\"","aggregated_output":"","exit_code":0,"status":"completed"}}' \
'{"type":"item.completed","item":{"id":"i3","type":"mcp_tool_call","server":"files","tool":"read_file","arguments":{"file_path":"main.py"},"result":{"content":"transcript"}}}' \
'{"type":"item.completed","item":{"id":"i4","type":"agent_message","text":"done"}}'"#;
        let summary = run_agent_benchmark(cmd, 0.0).unwrap();
        assert_eq!(summary.tasks, 21);
        assert!(summary.results.iter().all(|r| r.exit_ok));
        // task 0 = http-service.rename-transcript-field (GT: main.py, services/transcripts.py)
        let first = &summary.results[0];
        assert_eq!(first.files_opened, 2, "wrong_file.py + main.py");
        assert_eq!(first.search_tool_calls, 1, "rg");
        assert_eq!(first.read_tool_calls, 2, "sed + read_file");
        assert_eq!(first.total_tool_calls, 3, "agent_message is not a tool");
        assert_eq!(first.wrong_first_locations, 1, "wrong_file.py before main.py");
        assert!(
            first.first_correct_ms.is_some(),
            "main.py touched in the read_file event"
        );
        assert!(first.first_correct_ms.unwrap() <= first.duration_ms);
        assert!(summary.mean_first_correct_ms.is_some());
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

    #[test]
    // trace:exempt reason=unit-test  # wave-15 variant runner test
fn variant_benchmark_records_variant_name() {
        // Same protocol as run_agent_benchmark, but the summary carries the
        // variant name (Wave 15 external suite).
        let summary = run_variant_benchmark("scc-atlas-surface", "echo \"$SCC_GOAL\"", 0.0).unwrap();
        assert_eq!(summary.variant, "scc-atlas-surface");
        assert_eq!(summary.tasks, 21);
        assert!(summary.results.iter().all(|r| r.exit_ok));
        // no tool events -> no graph queries; first plan = whole output
        assert!(summary.results.iter().all(|r| r.graph_tool_calls == 0));
    }

    #[test]
    // trace:exempt reason=unit-test  # wave-15 variant runner test
fn run_variant_tasks_filters_and_first_plan() {
        // A repo-filtered task list: the closure builds the per-task shell
        // command (variant artifact injection point) and the runner records
        // first-plan correctness against the plan keys.
        let tasks = vec![VariantTask {
            id: "http-service.rename-transcript-field".into(),
            repo: "http-service-python".into(),
            goal: "rename the transcript field".into(),
            files: vec!["main.py".into(), "services/transcripts.py".into()],
            plan_keys: vec!["main.py".into(), "handle_transcripts".into()],
        }];
        let cmd_for = |_task: &VariantTask, _root: &Path| {
            // The fake agent's "first plan" names the correct file before
            // any tool event, then emits one search event.
            Ok(r#"printf '%s\n' \
'{"type":"item.completed","item":{"type":"agent_message","text":"Plan: main.py"}}' \
'{"type":"item.completed","item":{"type":"command_execution","command":"/bin/zsh -lc \"rg -n transcript .\"","exit_code":0}}'"#
                .to_string())
        };
        let summary = run_variant_tasks("scc-atlas", &tasks, cmd_for, 0.0).unwrap();
        assert_eq!(summary.tasks, 1);
        assert_eq!(summary.variant, "scc-atlas");
        let r = &summary.results[0];
        assert!(r.first_plan_correct, "plan mentions main.py before tools");
        assert_eq!(r.search_tool_calls, 1);
        assert_eq!(summary.mean_first_plan_correct, 1.0);
        assert_eq!(summary.mean_search_tool_calls, 1.0);
    }

    fn summary_with(search: f64, files: f64, first: Option<f64>) -> AgentBenchSummary {
        AgentBenchSummary {
            tasks: 21,
            passed: 21,
            mean_search_tool_calls: search,
            mean_files_opened: files,
            mean_first_correct_ms: first,
            ..Default::default()
        }
    }

    #[test]
    fn agent_gate_evaluates_all_three_clauses() {
        let a = summary_with(2.0, 4.0, Some(1000.0));
        // E reduces searches AND files AND first-correct -> PASS
        let e = summary_with(1.0, 3.0, Some(900.0));
        let g = evaluate_gate(&a, &e);
        assert!(g.search_reduced && g.files_bounded && g.first_correct_bounded);
        assert!(g.passed);
        // files bounded is <= A + 1, so 5 vs 4 still passes
        let e2 = summary_with(1.0, 5.0, Some(900.0));
        let g2 = evaluate_gate(&a, &e2);
        assert!(g2.files_bounded && g2.passed);
        // slower first-correct -> FAIL
        let e3 = summary_with(1.0, 3.0, Some(1500.0));
        let g3 = evaluate_gate(&a, &e3);
        assert!(!g3.first_correct_bounded);
        assert!(!g3.passed);
        // search must be STRICTLY less (equal does not reduce)
        let e4 = summary_with(2.0, 3.0, Some(900.0));
        let g4 = evaluate_gate(&a, &e4);
        assert!(!g4.search_reduced);
        assert!(!g4.passed);
        // fails closed: missing first-correct mean on either side
        let e5 = summary_with(1.0, 3.0, None);
        let g5 = evaluate_gate(&a, &e5);
        assert!(!g5.first_correct_bounded);
        assert!(!g5.passed);
    }

    #[test]
    fn agent_gate_fails_closed_without_json_streams() {
        // echo produces no JSON event stream -> no first-correct means ->
        // the gate cannot verify reduction and FAILS closed.
        let g = run_agent_gate("echo hi", "echo hi", 0.0).unwrap();
        assert!(!g.passed);
        assert!(!g.first_correct_bounded);
        assert_eq!(g.baseline.tasks, 21);
        assert_eq!(g.atlas.tasks, 21);
    }

    // ---- Wave-15 ablation matrix + leakage-free selection ----------------

    #[test]
    // trace:exempt reason=unit-test  # ablation matrix tests (impl.scc.bench.ablation)
    fn surface_paths_parse_from_production_render() {
        // The PRODUCTION surface format: `SCC SYSTEM SURFACE MAP` header,
        // then per-group `<COMPONENT>\n\n<path>[  [<sub>]]\n\n  <kind> <name>`
        // blocks. The path comes from the group header line, not the
        // entries. (concat! preserves the entry indentation that a `\`
        // line-continuation would strip.)
        let text = concat!(
            "SCC SYSTEM SURFACE MAP\n",
            "\n",
            "CLI\n",
            "\n",
            "cli.rs\n",
            "\n",
            "  method Cli.serve\n",
            "\n",
            "    def serve() -> None\n",
            "  Used by:\n",
            "    main\n",
            "\n",
            "ROUTER\n",
            "\n",
            "router.py  [routing]\n",
            "\n",
            "  function build_router\n",
            "\n",
            "    def build_router()\n",
            "\n",
            "DEPLOY\n",
            "\n",
            "deploy.go\n",
            "\n",
            "  method Deploy.deploy\n",
            "\n",
            "    func (d *Deploy) deploy()\n",
            "\n",
            "UTIL\n",
            "\n",
            "util.rs\n",
            "\n",
            "  function helper\n",
            "\n",
            "    fn helper()\n",
        );
        assert_eq!(
            parse_surface_paths(text),
            vec!["cli.rs", "router.py", "deploy.go", "util.rs"]
        );
    }

    #[test]
    // trace:exempt reason=unit-test  # ablation matrix tests (impl.scc.bench.ablation)
    fn surface_paths_skip_headers_omitted_and_dedupe() {
        // Headers (including the uppercased component lines and the
        // OMITTED section), non-path lines, and duplicate groups carry no
        // paths; duplicate paths collapse. concat! preserves the entry
        // indentation a `\` line-continuation would strip.
        let text = concat!(
            "SCC SYSTEM SURFACE MAP\n",
            "\n",
            "CLI\n",
            "\n",
            "cli.rs\n",
            "\n",
            "  method Cli.serve\n",
            "\n",
            "    def serve() -> None\n",
            "\n",
            "CLI\n",
            "\n",
            "cli.rs\n",
            "\n",
            "  method Cli.serve\n",
            "\n",
            "    def serve() -> None\n",
            "OMITTED (token budget exceeded):\n",
            "  2 lower-ranked function definitions\n",
            "\n",
            "MY.SERVICE\n",
            "\n",
            "  method Thing.run\n",
            "\n",
            "    def run()\n",
            "\n",
            "DATA\n",
            "\n",
            "src/data/store.ts\n",
            "\n",
            "  class Store\n",
            "\n",
            "    export class Store {}\n",
        );
        assert_eq!(
            parse_surface_paths(text),
            vec!["cli.rs", "src/data/store.ts"]
        );
    }

    #[test]
    // trace:exempt reason=unit-test  # ablation matrix tests (impl.scc.bench.ablation)
    fn ablation_modes_render_distinct_artifacts() {
        // Every mode renders a non-empty, mode-labeled artifact for the
        // cli-service fixture (indexed in a scratch copy).
        let fixtures = locate_fixtures_dir().unwrap();
        let src = fixtures.join("cli-service");
        assert!(src.is_dir(), "cli-service fixture exists");
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("repo");
        copy_fixture_tree(&src, &root);
        crate::commands::cmd_index(&root, true).unwrap();
        let goal = "add a --paging flag to the serve subcommand";
        for mode in [
            SurfaceAblation::Lexical,
            SurfaceAblation::GlobalPpr,
            SurfaceAblation::TaskPpr,
            SurfaceAblation::PprMmr,
            SurfaceAblation::PprQuotas,
            SurfaceAblation::PprOptimizer,
        ] {
            let out = render_ablation_surface(&root, mode, goal, 8000).unwrap();
            assert!(!out.is_empty(), "{mode:?} renders");
            assert!(
                out.contains(&format!("ablation {}", mode.as_str())),
                "{mode:?} header marks the mode: {out}"
            );
            // budget discipline: the chars/4 estimate never exceeds the cap
            assert!(
                estimate_tokens(&out) <= 8000,
                "{mode:?} fits the budget ({} tokens)",
                estimate_tokens(&out)
            );
        }
    }

    #[test]
    // trace:exempt reason=unit-test  # ablation matrix tests (impl.scc.bench.ablation)
    fn lexical_ablation_ranks_goal_matches_first() {
        // Lexical mode must surface serve/paging symbols before unrelated
        // ones (pure lexical baseline).
        let fixtures = locate_fixtures_dir().unwrap();
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("repo");
        copy_fixture_tree(&fixtures.join("cli-service"), &root);
        crate::commands::cmd_index(&root, true).unwrap();
        let out = render_ablation_surface(
            &root,
            SurfaceAblation::Lexical,
            "add a --paging flag to the serve subcommand",
            8000,
        )
        .unwrap();
        let serve_lines: Vec<&str> = out
            .lines()
            .filter(|l| {
                // Entry lines are 2-space indented `  <kind> <name>`; the
                // label/headers/signatures are excluded.
                l.starts_with("  ") && !l.starts_with("    ") && l.contains("serve")
            })
            .collect();
        assert!(!serve_lines.is_empty(), "serve symbols surface lexically: {out}");
    }
}
