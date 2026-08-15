//! `scc` — System Context Compiler CLI (docs/API_AND_INTEGRATIONS.md §4).

use clap::{Parser, Subcommand};
use scc_cli::commands;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "scc",
    version,
    about = "System Context Compiler — compile repositories into evidence-backed system context for coding agents",
    long_about = "Continuously compiles code, configuration, infrastructure, runtime evidence, and architectural intent \
                   into an evidence-backed machine model of a software system, then emits small task-specific context packs \
                   for coding agents. Give agents more repository understanding per token."
)]
// trace:v1 id=impl.scc.cli.main work=WORK-SCC-014 satisfies=REQ-SCC-IR
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Repository root (default: nearest directory containing .git or .scc)
    #[arg(long, global = true)]
    root: Option<PathBuf>,
}

#[derive(Subcommand)]
// trace:exempt reason=internal-detail
enum Commands {
    /// Initialize the SCC workspace (.scc/config.yaml + database)
    Init,

    /// Index the repository (cold on first run, incremental afterwards)
    Index {
        /// Only refresh these paths (watch/post-edit)
        #[arg(long, value_delimiter = ' ')]
        paths: Vec<String>,
        #[arg(long)]
        quiet: bool,
        /// Run the language-aware semantic backends before recompiling
        /// (Wave 4 lazy resolution: pyright + typescript-language-server)
        #[arg(long)]
        resolve: bool,
    },

    /// Show index status, stats, and freshness
    Status,

    /// Watch the filesystem and re-index changed files
    Watch,

    /// Print the startup capsule / system overview
    Overview {
        #[arg(long)]
        json: bool,
    },

    /// Compile context packs
    Context {
        #[command(subcommand)]
        sub: ContextSub,
    },

    /// The System Surface Map: the actual callable API layer (Wave 14)
    Surface {
        /// Personalize the map for a task goal (task PPR re-ranking)
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        budget: Option<usize>,
        /// Append per-entry rank reasons
        #[arg(long)]
        explain: bool,
    },

    /// Impact analysis for a change
    Impact {
        /// Git base for the diff (e.g. origin/main, HEAD~1)
        #[arg(long)]
        diff: Option<String>,
        /// Affected files
        #[arg(value_delimiter = ' ')]
        files: Vec<String>,
        #[arg(long)]
        symbols: Vec<String>,
        #[arg(long)]
        json: bool,
    },

    /// Full System Atlas: complete architecture for agent session startup
    Atlas {
        /// Token budget (default: context.atlas_tokens, 15000)
        #[arg(long)]
        budget: Option<usize>,
        #[arg(long)]
        json: bool,
        /// Resolve unresolved call edges through the language backends first
        #[arg(long)]
        resolve: bool,
    },

    /// Verify freshness, evidence integrity, and drift
    Verify {
        /// Only print warnings (for hooks)
        #[arg(long)]
        warnings: bool,
        #[arg(long)]
        json: bool,
    },

    /// Show architectural drift findings
    Drift {
        #[arg(long)]
        json: bool,
    },

    /// Export the System IR
    Export {
        /// system-ir.json | system-ir.jsonl | ccg | flow-graphs.json
        #[arg(default_value = "system-ir.json")]
        format: String,
    },

    /// Lexical search over entities and symbols
    Query {
        query: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },

    /// List components
    Components,

    /// List flows
    Flows,

    /// Show git co-change pairs (files changed together across commits)
    Cochange {
        /// Minimum number of shared commits
        #[arg(long, default_value_t = 2)]
        min_commits: u32,
    },

    /// Save or restore the task checkpoint (compaction recovery)
    Checkpoint {
        #[command(subcommand)]
        sub: CheckpointSub,
    },

    /// Run graph invariant checks; exit nonzero on violation (CI)
    CheckInvariants,

    /// Install the Claude Code plugin (hooks)
    Setup {
        #[command(subcommand)]
        sub: SetupSub,
    },

    /// Start the local daemon (HTTP API + watcher)
    Serve,

    /// Run the MCP server on stdio
    Mcp,

    /// Ingest runtime observations (POST /v1/runtime/traces body)
    Ingest {
        body: String,
    },

    /// Semantic resolution: upgrade EXTRACTED facts via a language server
    Resolve {
        /// Use the LSP adapter (pyright) for definition resolution
        #[arg(long)]
        lsp: bool,
    },

    /// Compute and store entity embeddings (optional semantic ranker)
    Embed,

    /// List enabled adapters with their declared capability scope (security audit)
    Adapters {
        /// Dump the full capability manifests instead of the scope listing
        #[arg(long)]
        json: bool,
    },

    /// Manage the Hindsight lesson bank (.scc/lessons.jsonl)
    Lessons {
        /// Bare `scc lessons` lists stored lessons (limit: --limit)
        #[command(subcommand)]
        sub: Option<LessonsSub>,
    },

    /// List active bead tasks from .beads/issues.jsonl
    Beads,

    /// Import external evidence (SCIP index, Narsil CCG)
    Import {
        /// scip | ccg | gitnexus
        format: String,
        /// Path to the evidence file
        file: String,
    },

    /// Runtime observations and static-vs-observed reconciliation
    Runtime {
        #[command(subcommand)]
        sub: RuntimeSub,
    },

    /// CI gate: graph invariants + drift severity policy
    Ci {
        #[command(subcommand)]
        sub: CiSub,
    },

    /// Run performance benchmarks on a synthetic repository
    Bench {
        #[command(subcommand)]
        sub: BenchSub,
    },

    /// Print the SCC state directory (honors SCC_STATE_DIR; used by agent
    /// integrations so a read-only repository with external state works)
    StatePath,
}

#[derive(Subcommand)]
enum RuntimeSub {
    /// List ingested runtime edges (aggregates)
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Static-vs-observed reconciliation report
    Reconcile {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum CiSub {
    /// Check invariants and drift; exit nonzero on violation
    Check {
        /// Highest drift severity that passes (low|medium|high|critical)
        #[arg(long, default_value = "medium")]
        max_severity: String,
    },
}

#[derive(Subcommand)]
// trace:exempt reason=internal-detail  # CLI subcommand enum (impl.scc.cli.main)
enum BenchSub {
    /// Cold/incremental index + task-pack latency on a generated repo
    Index {
        /// Number of generated files
        #[arg(long, default_value_t = 200)]
        files: usize,
        /// Lines per file
        #[arg(long, default_value_t = 250)]
        lines: usize,
    },
    /// Ground-truth context benchmark (docs/TEST_PLAN.md §84): recall,
    /// precision, localization, hallucinations, budgets
    Context {
        /// Minimum mean recall gate
        #[arg(long, default_value_t = 0.6)]
        min_recall: f64,
    },
    /// Agent-run recorder (SCC-002): run the corpus through an external
    /// agent command and record outcome metrics
    Agent {
        /// Agent command; receives the goal via $SCC_GOAL, runs in the repo
        #[arg(long)]
        cmd: String,
        /// Minimum mean localization gate
        #[arg(long, default_value_t = 0.0)]
        min_files: f64,
        /// Agent-behavior release gate: run the baseline (--baseline-cmd)
        /// AND the atlas variant (--cmd), then FAIL when the atlas variant
        /// does not reduce exploration (E.search_tool_calls < A.search,
        /// E.files_opened <= A.files_opened + 1, E.first_correct_ms <=
        /// A.first_correct_ms; means)
        #[arg(long)]
        gate: bool,
        /// Baseline (A) agent command for --gate; receives the goal via
        /// $SCC_GOAL, runs in the repo
        #[arg(long)]
        baseline_cmd: Option<String>,
    },
    /// Differential resolution benchmark (SCC-126): native vs LSP upgrades
    /// over the fixture corpus, gated on upgrades and unresolved externals
    Resolution {
        /// Maximum allowed fraction of external call candidates left
        /// unresolved after the LSP pass
        #[arg(long, default_value_t = scc_cli::benchres::DEFAULT_MIN_AGREEMENT)]
        min_agreement: f64,
    },
    /// Atlas recall benchmark (Wave 8 §57): recall of independently
    /// documented ground truth against the startup System Atlas on real
    /// repos (defaults: <repo-root>/benchmarks/corpus +
    /// <repo-root>/benchmarks/ground-truth; falls back to the fixtures when
    /// the corpus dir is absent)
    Atlas {
        /// Corpus directory (default: <repo-root>/benchmarks/corpus)
        #[arg(long)]
        corpus: Option<PathBuf>,
        /// Ground-truth docs directory (default: <repo-root>/benchmarks/ground-truth)
        #[arg(long)]
        ground_truth: Option<PathBuf>,
        /// Emit the full report as JSON (per-repo recall + all missed keys)
        #[arg(long)]
        json: bool,
        /// Classify every missed ground-truth item by gap kind
        /// (PARSER/EXTRACTOR/RESOLUTION/COMPILER/PROJECTION/ALIAS) and print
        /// a per-kind histogram plus per-repo gap lines (the regeneration
        /// source for benchmarks/results/ground-truth-gaps.md)
        #[arg(long)]
        diagnose: bool,
        /// Blind holdout protocol: score the dev corpus AND the holdout
        /// corpus (benchmarks/holdout + benchmarks/holdout-ground-truth),
        /// compare per-layer recall, write benchmarks/results/holdout-v3.txt
        /// and print the overfit verdict
        #[arg(long, conflicts_with = "blind")]
        holdout: bool,
        /// Blind-test protocol: score the validation corpus
        /// (benchmarks/holdout) AND the blind-test corpus
        /// (benchmarks/blind-test + benchmarks/blind-test-ground-truth),
        /// print ONLY aggregates (overall, per-section means, the
        /// validation-vs-blind generalization gap, precision, density — no
        /// per-repo rows, no missed keys, no filenames), and write
        /// benchmarks/results/blind-v1.txt. blind-test failures are never
        /// shown to tuning agents; --diagnose is refused on the blind corpus
        #[arg(long, conflicts_with = "holdout")]
        blind: bool,
        /// Skip the semantic resolution pass (pyright + tsserver) before
        /// scoring: the atlas then runs on native extraction only, and
        /// `resolved_calls` reports 0. Default is ON (resolve before
        /// scoring, so behavior flows seed from resolved call chains)
        /// for both the dev and holdout corpora.
        #[arg(long)]
        no_resolve: bool,
        /// Wave-11 generalization gates over two saved holdout result files
        /// (JSON `HoldoutComparison`s, e.g. `scc bench atlas --holdout
        /// --json` output): loads OLD (the earlier run) and NEW (the current
        /// run), prints the per-section deltas, the generalization
        /// efficiency GE = validation_delta / development_delta, and the
        /// per-section regression guard, then exits nonzero when a gate
        /// fails (semantic waves must generalize)
        #[arg(long, num_args = 2, value_names = ["OLD", "NEW"], conflicts_with_all = ["blind", "holdout"])]
        compare: Option<Vec<PathBuf>>,
        /// GE gate floor for `--compare`: fail when
        /// GE = validation_delta / development_delta <= MIN (default 0.0 —
        /// semantic waves must generalize to validation)
        #[arg(long, default_value_t = 0.0)]
        gate_ge: f64,
        /// Per-section regression guard for `--compare`: fail when ANY
        /// startup-required section (architecture/entrypoints/behavior/
        /// state_authority/contracts) drops by more than MAX between the
        /// two runs, in development or validation (default 0.05)
        #[arg(long, default_value_t = 0.05)]
        guard_section_delta: f64,
    },
    /// Wave-15 external benchmark suite: run one A-H context variant over
    /// a repo's ground-truth tasks through the benchagent protocol and
    /// print the spec §72 metric row (variant, success rate, mean
    /// exploration, first-plan accuracy, context tokens). Native variants
    /// (raw, scc-*) run in-process; aider-repomap / repomix-compress
    /// delegate to benchmarks/external/run_context_bench.py, which owns
    /// those pinned tools and reports SKIPPED-UNINSTALLED when they are
    /// not installed.
    External {
        /// Variant: raw | aider-repomap | repomix-compress | scc-atlas |
        /// scc-surface | scc-atlas-surface | scc-full
        #[arg(long)]
        variant: String,
        /// Restrict to one fixture repo (default: every ground-truth repo)
        #[arg(long)]
        repo: Option<String>,
        /// Context-artifact token budget (equal-token mode)
        #[arg(long, default_value_t = 8000)]
        budget: usize,
        /// Agent command; the prompt (context artifact + task goal) is
        /// piped to it on stdin and $SCC_GOAL carries the goal (the
        /// benchagent protocol, see benchmarks/run_agent_bench.sh)
        #[arg(long)]
        cmd: Option<String>,
        /// Workdir for generated artifacts and result JSON
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Emit the metric row as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
// trace:exempt reason=internal-detail
enum ContextSub {
    /// Wave 14C startup artifact: Atlas + Surface fusion, deterministic per
    /// epoch (prompt-cache stable)
    Startup {
        /// Token budget (default: the full startup split, 20000)
        #[arg(long)]
        budget: Option<usize>,
    },
    /// Task context pack for a goal
    Task {
        goal: String,
        #[arg(long, value_delimiter = ' ')]
        files: Vec<String>,
        #[arg(long)]
        symbols: Vec<String>,
        #[arg(long)]
        budget: Option<usize>,
        #[arg(long)]
        json: bool,
        /// Hook mode (UserPromptSubmit): prints nothing unless
        /// context.inject_task_focus is enabled; caps the focus budget.
        #[arg(long, hide = true)]
        hook: bool,
        /// Resolve unresolved call edges through the language backends first
        #[arg(long)]
        resolve: bool,
    },
    /// Component context pack
    Component {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Flow context pack
    Flow {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Compress a task pack (structural; optional external summarizer)
    Compress {
        goal: String,
        /// External summarizer command (stdin -> stdout); output is labeled INFERRED
        #[arg(long)]
        cmd: Option<String>,
        /// Constrained mode: summarizer must emit typed claims with known evidence
        #[arg(long)]
        claims: bool,
        #[arg(long)]
        budget: Option<usize>,
        #[arg(long)]
        json: bool,
    },
    /// External library docs via Context7 (labeled external)
    Docs {
        /// dependency in owner/name form (e.g. fastapi/fastapi)
        dependency: String,
    },
    /// Subagent-scoped task pack (tight budget + scope boundaries)
    Subagent {
        goal: String,
        #[arg(long, value_delimiter = ' ')]
        files: Vec<String>,
        #[arg(long)]
        symbols: Vec<String>,
        #[arg(long)]
        budget: Option<usize>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum CheckpointSub {
    /// Capture the current task state
    Save {
        /// Emit the checkpoint JSON on stdout
        #[arg(long)]
        json: bool,
    },
    /// Load and print the checkpoint (rehydration)
    Load {
        /// Silent when no checkpoint exists (hook use)
        #[arg(long)]
        inject: bool,
    },
}

#[derive(Subcommand)]
enum LessonsSub {
    /// Append a lesson to .scc/lessons.jsonl (ingest with `scc import hindsight .scc/lessons.jsonl`)
    Add {
        text: String,
    },
    /// List stored lessons from the store (after `scc import hindsight`)
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
}

#[derive(Subcommand)]
enum SetupSub {
    /// Install Claude Code hooks
    Claude,
    /// Write AGENTS.md with the system capsule (Codex and other harnesses)
    Codex,
    /// Write AGENTS.md + .opencode/opencode.json (SCC MCP server)
    Opencode,
    /// Install the Hermes plugin (native tools + skill)
    Hermes,
}

// trace:exempt reason=internal-detail
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Default SIGPIPE behavior: dying silently on `scc ... | head` is the
    // standard Unix contract; ignoring it makes println! panic on EPIPE.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let cli = Cli::parse();
    let root = match cli.root.clone() {
        // explicit --root is exact: no upward walk
        Some(r) if r.is_absolute() => r,
        Some(r) => std::env::current_dir().unwrap().join(r),
        None => scc_cli::find_root(&std::env::current_dir().unwrap()),
    };

    let result = match cli.command {
        Commands::Init => commands::cmd_init(&root),
        Commands::Index { paths, quiet, resolve } => {
            if resolve {
                commands::cmd_index(&root, quiet)?;
                let rep = scc_cli::resolve_and_recompile(&root)?;
                if !quiet {
                    println!(
                        "resolved: {} upgraded, {} unresolved, {} errors",
                        rep.upgraded, rep.unresolved, rep.errors
                    );
                }
                Ok(())
            } else if paths.is_empty() {
                commands::cmd_index(&root, quiet)
            } else {
                commands::cmd_index_paths(&root, &paths, quiet)
            }
        }
        Commands::Status => commands::cmd_status(&root),
        Commands::Watch => commands::cmd_watch(&root),
        Commands::Overview { json } => commands::cmd_overview(&root, json),
        Commands::Context { sub } => match sub {
            ContextSub::Startup { budget } => commands::cmd_context_startup(&root, budget),
            ContextSub::Task { goal, files, symbols, budget, json, hook, resolve } => {
                if resolve {
                    let rep = scc_cli::resolve_and_recompile(&root)?;
                    eprintln!(
                        "resolved: {} upgraded, {} unresolved, {} errors",
                        rep.upgraded, rep.unresolved, rep.errors
                    );
                }
                commands::cmd_context_task(&root, &goal, &files, &symbols, budget, json, hook)
            }
            ContextSub::Component { id, json } => commands::cmd_context_component(&root, &id, json),
            ContextSub::Flow { id, json } => commands::cmd_context_flow(&root, &id, json),
            ContextSub::Docs { dependency } => commands::cmd_context_docs(&root, &dependency),
            ContextSub::Subagent { goal, files, symbols, budget, json } => {
                commands::cmd_context_subagent(&root, &goal, &files, &symbols, budget, json)
            }
            ContextSub::Compress { goal, cmd, claims, budget, json } => {
                if claims {
                    let out = scc_cli::compress::cmd_context_compress_json_claims(
                        &root, &goal, cmd, budget, true,
                    )?;
                    if json {
                        println!("{out}");
                    } else {
                        let pack: scc_context::ContextPack = serde_json::from_str(&out)?;
                        print!("{}", pack.content);
                    }
                    Ok(())
                } else {
                    scc_cli::compress::cmd_context_compress(&root, &goal, cmd, budget, json)
                }
            }
        },
        Commands::Impact { diff, files, symbols, json } => {
            commands::cmd_impact(&root, &files, &symbols, diff.as_deref(), json)
        }
        Commands::Surface { task, budget, explain } => {
            commands::cmd_surface(&root, task.as_deref(), budget, explain)
        }
        Commands::Atlas { budget, json, resolve } => {
            if resolve {
                let rep = scc_cli::resolve_and_recompile(&root)?;
                eprintln!(
                    "resolved: {} upgraded, {} unresolved, {} errors",
                    rep.upgraded, rep.unresolved, rep.errors
                );
            }
            commands::cmd_atlas(&root, budget, json)
        }
        Commands::Verify { warnings, json } => commands::cmd_verify(&root, warnings, json),
        Commands::Drift { json } => commands::cmd_drift(&root, json),
        Commands::Export { format } => commands::cmd_export(&root, &format),
        Commands::Query { query, limit } => commands::cmd_query(&root, &query, limit),
        Commands::Components => commands::cmd_list_components(&root),
        Commands::Flows => commands::cmd_list_flows(&root),
        Commands::Cochange { min_commits } => commands::cmd_cochange(&root, min_commits),
        Commands::Checkpoint { sub } => match sub {
            CheckpointSub::Save { json } => commands::cmd_checkpoint_save(&root, json),
            CheckpointSub::Load { inject } => commands::cmd_checkpoint_load(&root, inject),
        },
        Commands::CheckInvariants => match commands::cmd_check_invariants(&root) {
            Ok(true) => Ok(()),
            Ok(false) => Err(scc_cli::CliError::Other(
                "graph invariants violated".into(),
            )),
            Err(e) => Err(e),
        },
        Commands::Setup { sub } => match sub {
            SetupSub::Claude => commands::cmd_setup_claude(&root),
            SetupSub::Codex => scc_cli::compress::cmd_setup_codex(&root),
            SetupSub::Opencode => scc_cli::compress::cmd_setup_opencode(&root),
            SetupSub::Hermes => scc_cli::plugin_hermes::cmd_setup_hermes(&root),
        },
        Commands::Serve => commands::cmd_serve(&root),
        Commands::Mcp => commands::cmd_mcp(&root),
        Commands::Ingest { body } => commands::cmd_ingest_runtime(&root, &body),
        Commands::Embed => scc_cli::embed_cli::cmd_embed(&root),
        Commands::Adapters { json } => commands::cmd_adapters(&root, json),
        Commands::Lessons { sub } => match sub.unwrap_or(LessonsSub::List { limit: 20 }) {
            LessonsSub::Add { text } => commands::cmd_lessons_add(&root, &text),
            LessonsSub::List { limit } => commands::cmd_lessons_list(&root, limit),
        },
        Commands::Beads => commands::cmd_beads(&root),
        Commands::Resolve { lsp: true } => {
            // SCC-125: capture native EXTRACTED edges before the LSP pass so
            // target changes can be recorded as resolution_conflict drift
            // findings (the upgrade preserves the evidence id, which lets the
            // diff link each EXTRACTED edge to its RESOLVED successor).
            let pre = {
                let store = scc_cli::open_store(&root)?;
                scc_cli::benchres::collect_external_edges(&store)?
            };
            scc_cli::resolve::cmd_resolve_lsp(&root)?;
            let store = scc_cli::open_store(&root)?;
            // upgrades changed the reality graph — rebuild the derived layer
            // first (recompile regenerates drift findings, so the resolution
            // conflicts are recorded after it)
            scc_cli::recompile(&store)?;
            let mut by_file: std::collections::BTreeMap<
                String,
                Vec<scc_indexer::conflicts::UpgradeRecord>,
            > = std::collections::BTreeMap::new();
            for (file, rec) in scc_cli::benchres::diff_upgrades(&store, &pre)? {
                by_file.entry(file).or_default().push(rec);
            }
            for (file, recs) in &by_file {
                let report =
                    scc_indexer::conflicts::record_resolution_conflicts(&store, file, recs)?;
                if report.conflicts > 0 {
                    println!(
                        "scc resolve: {} resolution conflict(s) in {file}",
                        report.conflicts
                    );
                }
            }
            Ok(())
        }
        Commands::Resolve { lsp: false } => Err(scc_cli::CliError::Other(
            "no resolver selected (use --lsp)".into(),
        )),
        Commands::Import { format, file } => commands::cmd_import(&root, &format, &file),
        Commands::Runtime { sub } => match sub {
            RuntimeSub::Status { json } => commands::cmd_runtime_status(&root, json),
            RuntimeSub::Reconcile { json } => commands::cmd_runtime_reconcile(&root, json),
        },
        Commands::Ci { sub } => match sub {
            CiSub::Check { max_severity } => match commands::cmd_ci_check(&root, &max_severity) {
                Ok(true) => Ok(()),
                Ok(false) => Err(scc_cli::CliError::Other(
                    "CI check failed: invariants or drift policy violated".into(),
                )),
                Err(e) => Err(e),
            },
        },
        Commands::Bench { sub } => match sub {
            BenchSub::Agent { cmd, min_files, gate, baseline_cmd } => {
                if gate {
                    let bc = baseline_cmd.ok_or(scc_cli::CliError::Other(
                        "--gate requires --baseline-cmd (the A-variant command)".into(),
                    ))?;
                    match scc_cli::benchagent::run_agent_gate(&bc, &cmd, min_files) {
                        Ok(g) => {
                            scc_cli::benchagent::print_agent_gate(&g);
                            if g.passed {
                                Ok(())
                            } else {
                                Err(scc_cli::CliError::Other(
                                    "agent-behavior gate FAILED: the atlas variant does not reduce exploration (search calls / files opened / first-correct time)".into(),
                                ))
                            }
                        }
                        Err(e) => Err(scc_cli::CliError::Other(e)),
                    }
                } else {
                    match scc_cli::benchagent::run_agent_benchmark(&cmd, min_files) {
                        Ok(summary) => {
                            scc_cli::benchagent::print_agent_summary(&summary);
                            Ok(())
                        }
                        Err(e) => Err(scc_cli::CliError::Other(e)),
                    }
                }
            },
            BenchSub::Context { min_recall } => match scc_cli::benchctx::run_context_benchmark(min_recall)
            {
                Ok(summary) => {
                    scc_cli::benchctx::print_summary(&summary);
                    Ok(())
                }
                Err(e) => Err(scc_cli::CliError::Other(e)),
            },
            BenchSub::Index { files, lines } => {
                let dir = std::env::temp_dir().join(format!("scc-bench-{}", std::process::id()));
                let _ = std::fs::remove_dir_all(&dir);
                std::fs::create_dir_all(&dir)?;
                match scc_cli::bench::bench_index(&dir, files, lines) {
                    Ok(r) => {
                        scc_cli::bench::print_report(&r);
                        let _ = std::fs::remove_dir_all(&dir);
                        Ok(())
                    }
                    Err(e) => {
                        let _ = std::fs::remove_dir_all(&dir);
                        Err(e)
                    }
                }
            }
            BenchSub::Resolution { min_agreement } => {
                match scc_cli::benchres::run_resolution_benchmark(min_agreement) {
                    Ok(summary) => {
                        scc_cli::benchres::print_summary(&summary);
                        Ok(())
                    }
                    Err(e) => Err(scc_cli::CliError::Other(e)),
                }
            }
            BenchSub::External { variant, repo, budget, cmd, workdir, json } => {
                let v = variant.as_str();
                if EXTERNAL_VARIANTS.contains(&v) {
                    run_external_python_delegation(v, repo.as_deref(), budget, cmd.as_deref(), json)
                } else if NATIVE_VARIANTS.contains(&v) {
                    run_external_variant(v, repo.as_deref(), budget, cmd.as_deref(), workdir.as_deref(), json)
                } else {
                    Err(scc_cli::CliError::Other(format!(
                        "unknown external variant {v:?} (expected one of: {})",
                        EXTERNAL_VARIANTS
                            .iter()
                            .chain(NATIVE_VARIANTS.iter())
                            .copied()
                            .collect::<Vec<_>>()
                            .join(", ")
                    )))
                }
            }
            BenchSub::Atlas {
                corpus,
                ground_truth,
                json,
                diagnose,
                holdout,
                blind,
                no_resolve,
                compare,
                gate_ge,
                guard_section_delta,
            } => {
                let resolve = !no_resolve;
                if let Some(pair) = compare {
                    if pair.len() != 2 {
                        return Err(scc_cli::CliError::Other(
                            "--compare requires exactly two result files: OLD NEW".into(),
                        )
                        .into());
                    }
                    let old = scc_cli::benchatlas::load_holdout_result(&pair[0])?;
                    let new = scc_cli::benchatlas::load_holdout_result(&pair[1])?;
                    let mut report = scc_cli::benchatlas::compare_runs(
                        &old,
                        &new,
                        gate_ge,
                        guard_section_delta,
                    );
                    report.old_file = pair[0].display().to_string();
                    report.new_file = pair[1].display().to_string();
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&report)
                                .map_err(|e| scc_cli::CliError::Other(e.to_string()))?
                        );
                    } else {
                        scc_cli::benchatlas::print_compare_report(&report);
                    }
                    if report.passed() {
                        Ok(())
                    } else {
                        Err(scc_cli::CliError::Other(
                            "Wave-11 generalization gates FAILED: the semantic wave does not generalize (see --compare report)".into(),
                        ))
                    }
                } else if blind {
                    match scc_cli::benchatlas::run_atlas_blind(diagnose, resolve) {
                        Ok(comparison) => {
                            if json {
                                println!(
                                    "{}",
                                    serde_json::to_string_pretty(&comparison)
                                        .map_err(|e| scc_cli::CliError::Other(e.to_string()))?
                                );
                            } else {
                                scc_cli::benchatlas::print_blind_report(&comparison);
                            }
                            Ok(())
                        }
                        Err(e) => Err(scc_cli::CliError::Other(e)),
                    }
                } else if holdout {
                    match scc_cli::benchatlas::run_atlas_holdout(
                        corpus.as_deref(),
                        ground_truth.as_deref(),
                        diagnose,
                        resolve,
                    ) {
                        Ok(comparison) => {
                            if json {
                                println!(
                                    "{}",
                                    serde_json::to_string_pretty(&comparison)
                                        .map_err(|e| scc_cli::CliError::Other(e.to_string()))?
                                );
                            } else {
                                scc_cli::benchatlas::print_holdout_report(&comparison, diagnose);
                            }
                            Ok(())
                        }
                        Err(e) => Err(scc_cli::CliError::Other(e)),
                    }
                } else {
                    match scc_cli::benchatlas::run_atlas_bench(
                        corpus.as_deref(),
                        ground_truth.as_deref(),
                        diagnose,
                        resolve,
                    ) {
                        Ok(report) => {
                            if json {
                                println!(
                                    "{}",
                                    serde_json::to_string_pretty(&report)
                                        .map_err(|e| scc_cli::CliError::Other(e.to_string()))?
                                );
                            } else {
                                scc_cli::benchatlas::print_report(&report, diagnose);
                            }
                            Ok(())
                        }
                        Err(e) => Err(scc_cli::CliError::Other(e)),
                    }
                }
            },
        },
        Commands::StatePath => {
            println!("{}", scc_cli::state_dir(&root).display());
            Ok(())
        }
    };

    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// `scc bench external` — Wave-15 external benchmark variants (A–H).
// Native variants generate their context artifact in-process (or via the
// `scc` binary itself) and run the benchagent protocol; the aider/repomix
// variants delegate to benchmarks/external/run_context_bench.py.
// ---------------------------------------------------------------------------

const EXTERNAL_VARIANTS: [&str; 2] = ["aider-repomap", "repomix-compress"];
const NATIVE_VARIANTS: [&str; 5] = [
    "raw",
    "scc-atlas",
    "scc-surface",
    "scc-atlas-surface",
    "scc-full",
];
const DEFAULT_AGENT_CMD: &str = "codex exec --json --sandbox read-only --skip-git-repo-check --ephemeral --color never -C . -";

// trace:exempt reason=internal-detail  # external-bench arm helpers (impl.scc.cli.main)
fn benchmarks_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("benchmarks"))
        .unwrap_or_else(|| PathBuf::from("benchmarks"))
}

// --- ground truth: benchmarks/tasks.json + external/ground-truth.yaml -----

#[derive(Debug, Clone, serde::Deserialize)]
// trace:exempt reason=internal-detail  # external-bench ground-truth schema (impl.scc.cli.main)
struct ExternalGroundTruth {
    #[serde(default)]
    repos: BTreeMap<String, ExternalRepoTasks>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
// trace:exempt reason=internal-detail  # external-bench ground-truth schema (impl.scc.cli.main)
struct ExternalRepoTasks {
    #[serde(default)]
    tasks: Vec<ExternalTaskDef>,
}

#[derive(Debug, Clone, serde::Deserialize)]
// trace:exempt reason=internal-detail  # external-bench ground-truth schema (impl.scc.cli.main)
struct ExternalTaskDef {
    id: String,
    goal: String,
    #[serde(default)]
    public_surfaces: Vec<String>,
    #[serde(default)]
    important_types: Vec<String>,
    #[serde(default)]
    implementation_landmarks: Vec<String>,
    #[serde(default)]
    symbols: Vec<String>,
}

/// Load the variant ground truth: the canonical `benchmarks/tasks.json`
/// corpus merged with `benchmarks/external/ground-truth.yaml` (extra repos
/// such as cli-service; Wave-14 §75 vocabulary). Optionally restricted to
/// one fixture repo. Localization ground truth = files; first-plan keys =
/// files + symbols.
// trace:exempt reason=internal-detail  # external-bench ground-truth loader (impl.scc.cli.main)
fn load_external_tasks(repo_filter: Option<&str>) -> Result<Vec<scc_cli::benchagent::VariantTask>, String> {
    let mut by_id: BTreeMap<String, scc_cli::benchagent::VariantTask> = BTreeMap::new();

    let tasks_json = benchmarks_dir().join("tasks.json");
    if tasks_json.is_file() {
        let text = std::fs::read_to_string(&tasks_json).map_err(|e| e.to_string())?;
        let corpus: scc_cli::benchctx::BenchmarkCorpus =
            serde_json::from_str(&text).map_err(|e| e.to_string())?;
        for t in &corpus.tasks {
            let mut plan_keys = t.ground_truth.files.clone();
            plan_keys.extend(t.ground_truth.symbols.iter().cloned());
            by_id.insert(
                t.id.clone(),
                scc_cli::benchagent::VariantTask {
                    id: t.id.clone(),
                    repo: t.repo.clone(),
                    goal: t.goal.clone(),
                    files: t.ground_truth.files.clone(),
                    plan_keys,
                },
            );
        }
    }

    let yaml_path = benchmarks_dir().join("external").join("ground-truth.yaml");
    if yaml_path.is_file() {
        let text = std::fs::read_to_string(&yaml_path).map_err(|e| e.to_string())?;
        let gt: ExternalGroundTruth = serde_yaml::from_str(&text).map_err(|e| e.to_string())?;
        for (repo, spec) in gt.repos {
            for t in spec.tasks {
                let mut plan_keys = t.implementation_landmarks.clone();
                plan_keys.extend(t.public_surfaces.iter().cloned());
                plan_keys.extend(t.important_types.iter().cloned());
                plan_keys.extend(t.symbols.iter().cloned());
                by_id.insert(
                    t.id.clone(),
                    scc_cli::benchagent::VariantTask {
                        id: t.id.clone(),
                        repo: repo.clone(),
                        goal: t.goal.clone(),
                        files: t.implementation_landmarks.clone(),
                        plan_keys,
                    },
                );
            }
        }
    }

    let mut tasks: Vec<scc_cli::benchagent::VariantTask> = by_id.into_values().collect();
    if let Some(r) = repo_filter {
        tasks.retain(|t| t.repo == r);
    }
    tasks.sort_by(|a, b| a.repo.cmp(&b.repo).then(a.id.cmp(&b.id)));
    Ok(tasks)
}

// --- artifact generation ----------------------------------------------------

/// Deterministic chars/4 token estimate — the same rule the python harness
/// and the adapters use, so context_tokens are comparable across variants.
// trace:exempt reason=internal-detail  # external-bench token heuristic (impl.scc.cli.main)
fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        (text.len() / 4).max(1)
    }
}

/// Run the `scc` binary itself (the current executable) and capture stdout.
/// The variant artifacts are generated through the exact production CLI so
/// benchmark output never diverges from what an agent would receive.
// trace:exempt reason=internal-detail  # external-bench artifact generator (impl.scc.cli.main)
fn run_scc_capture(root: &Path, args: &[&str]) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current exe: {e}"))?;
    let out = std::process::Command::new(&exe)
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("spawn {}: {e}", exe.display()))?;
    if !out.status.success() {
        return Err(format!(
            "`scc {}` exited {}: {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Structural source render for the task's matched files (scc-full): the
/// per-file signature/structural representation from the Wave-14C module.
// trace:exempt reason=internal-detail  # external-bench scc-full component (impl.scc.cli.main)
fn structural_source_text(root: &Path, files: &[String]) -> Result<String, String> {
    let store = scc_cli::open_store(root).map_err(|e| e.to_string())?;
    let config = scc_cli::load_config(root).map_err(|e| e.to_string())?;
    let stale = scc_cli::stale_paths(&store).map_err(|e| e.to_string())?;
    let comp = scc_cli::compiler(&store, &config, stale).map_err(|e| e.to_string())?;
    let ctx = comp.ctx();
    let units = scc_context::structural_source::structural_source(&ctx, files, 8);
    Ok(scc_context::structural_source::render_structural(&units))
}

/// Generate the variant's context artifact for one task (after the runner
/// has copied + indexed the repo). Writes `<artifacts_dir>/<repo>.txt` and
/// returns its path and token estimate.
// trace:exempt reason=internal-detail  # external-bench artifact generator (impl.scc.cli.main)
fn generate_variant_artifact(
    variant: &str,
    task: &scc_cli::benchagent::VariantTask,
    root: &Path,
    budget: usize,
    artifacts_dir: &Path,
) -> Result<(PathBuf, usize), String> {
    std::fs::create_dir_all(artifacts_dir).map_err(|e| e.to_string())?;
    let artifact_path = artifacts_dir.join(format!("{}.txt", task.repo));
    let budget_s = budget.to_string();

    let text: String = match variant {
        "raw" => String::new(),
        "scc-atlas" => {
            // startup, atlas section only: everything before the surface map
            let full =
                run_scc_capture(root, &["context", "startup", "--budget", &budget_s])?;
            let cut = full.find("\n## SYSTEM SURFACE MAP").unwrap_or(full.len());
            full[..cut].to_string()
        }
        "scc-surface" => run_scc_capture(root, &["surface", "--budget", &budget_s])?,
        "scc-atlas-surface" => run_scc_capture(root, &["context", "startup", "--budget", &budget_s])?,
        "scc-full" => {
            let startup = run_scc_capture(root, &["context", "startup", "--budget", &budget_s])?;
            let task_pack =
                run_scc_capture(root, &["context", "task", &task.goal, "--budget", &budget_s])?;
            let structural = structural_source_text(root, &task.files)?;
            format!("{startup}\n\n{task_pack}\n\n{structural}")
        }
        other => return Err(format!("unsupported native variant: {other}")),
    };

    std::fs::write(&artifact_path, &text).map_err(|e| e.to_string())?;
    Ok((artifact_path, estimate_tokens(&text)))
}

/// Shell single-quote escaping for embedding the agent command inside the
/// variant command.
// trace:exempt reason=internal-detail  # external-bench shell quoting (impl.scc.cli.main)
fn sh_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The benchagent variant command: the context artifact is catted into
/// $CTX, then the prompt `SCC CONTEXT: <artifact> \n TASK: <goal>` is piped
/// to the agent command on stdin ($SCC_GOAL carries the goal).
// trace:exempt reason=internal-detail  # external-bench agent command builder (impl.scc.cli.main)
fn build_variant_command(agent_cmd: &str, artifact_path: &Path) -> String {
    let art = sh_single_quote(&artifact_path.display().to_string());
    let agent = sh_single_quote(agent_cmd);
    format!(
        "CTX=$(cat {art} 2>/dev/null || true); printf 'SCC CONTEXT:\\n%s\\n\\nTASK: %s\\n' \"$CTX\" \"$SCC_GOAL\" | sh -c {agent}"
    )
}

/// Run one native variant through the benchagent runner and print the §72
/// metric row (variant, success rate, mean exploration, first-plan
/// accuracy, context tokens).
// trace:exempt reason=internal-detail  # external-bench native arm (impl.scc.cli.main)
fn run_external_variant(
    variant: &str,
    repo: Option<&str>,
    budget: usize,
    agent_cmd: Option<&str>,
    workdir: Option<&Path>,
    json: bool,
) -> Result<(), scc_cli::CliError> {
    let agent = agent_cmd.unwrap_or(DEFAULT_AGENT_CMD);
    let tasks = load_external_tasks(repo).map_err(scc_cli::CliError::Other)?;
    if tasks.is_empty() {
        return Err(scc_cli::CliError::Other(format!(
            "no ground-truth tasks for repo {}",
            repo.unwrap_or("(all)")
        )));
    }
    let workdir = workdir.map(Path::to_path_buf).unwrap_or_else(|| {
        std::env::temp_dir().join(format!("scc-external-{}", std::process::id()))
    });
    let artifacts_dir = workdir.join("artifacts");
    std::fs::create_dir_all(&artifacts_dir).map_err(|e| scc_cli::CliError::Other(e.to_string()))?;

    // context-token mean over tasks (scc-full artifacts are goal-dependent)
    let mut tokens_seen: Vec<usize> = Vec::new();
    let summary = scc_cli::benchagent::run_variant_tasks(variant, &tasks, |task, root| {
        let (path, tokens) = generate_variant_artifact(variant, task, root, budget, &artifacts_dir)?;
        tokens_seen.push(tokens);
        Ok(build_variant_command(agent, &path))
    }, 0.0)
    .map_err(scc_cli::CliError::Other)?;

    let mean_tokens = if tokens_seen.is_empty() {
        0
    } else {
        tokens_seen.iter().sum::<usize>() / tokens_seen.len()
    };
    let n = summary.tasks.max(1) as f64;
    let row = serde_json::json!({
        "variant": variant,
        "budget": budget,
        "repo": repo.unwrap_or("all"),
        "tasks": summary.tasks,
        "success_rate": summary.passed as f64 / n,
        "mean_exploration": summary.mean_files_opened
            + summary.mean_search_tool_calls
            + summary.mean_graph_tool_calls,
        "first_plan_accuracy": summary.mean_first_plan_correct,
        "context_tokens": mean_tokens,
        "mean_files_opened": summary.mean_files_opened,
        "mean_search_tool_calls": summary.mean_search_tool_calls,
        "mean_graph_tool_calls": summary.mean_graph_tool_calls,
        "mean_files_opened_before_first_correct": summary.mean_wrong_first_locations,
    });

    // persist the row + full summary for later aggregation
    let results_dir = workdir.join("results");
    std::fs::create_dir_all(&results_dir).map_err(|e| scc_cli::CliError::Other(e.to_string()))?;
    let result_path = results_dir.join(format!(
        "{variant}_{}_{budget}.json",
        repo.unwrap_or("all")
    ));
    let result_json = serde_json::json!({ "row": row, "summary": summary });
    let _ = std::fs::write(
        &result_path,
        serde_json::to_string_pretty(&result_json)
            .map_err(|e| scc_cli::CliError::Other(e.to_string()))?,
    );

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&row).map_err(|e| scc_cli::CliError::Other(e.to_string()))?
        );
    } else {
        println!(
            "{variant:<18} success {:>6.3}   mean exploration {:>7.2}   first-plan accuracy {:>6.3}   context tokens {:>6}",
            summary.passed as f64 / n,
            summary.mean_files_opened + summary.mean_search_tool_calls + summary.mean_graph_tool_calls,
            summary.mean_first_plan_correct,
            mean_tokens
        );
        scc_cli::benchagent::print_agent_summary(&summary);
    }
    Ok(())
}

/// Delegate the external-tool variants (aider-repomap, repomix-compress) to
/// benchmarks/external/run_context_bench.py, which owns the pinned tools.
/// Exit 2 from the harness = SKIPPED-UNINSTALLED: the variant row is
/// reported with that status and the arm succeeds (the caller decides
/// whether missing tools are acceptable).
// trace:exempt reason=internal-detail  # external-bench delegation arm (impl.scc.cli.main)
fn run_external_python_delegation(
    variant: &str,
    repo: Option<&str>,
    budget: usize,
    agent_cmd: Option<&str>,
    json: bool,
) -> Result<(), scc_cli::CliError> {
    let script = benchmarks_dir().join("external").join("run_context_bench.py");
    if !script.is_file() {
        return Err(scc_cli::CliError::Other(format!(
            "missing harness: {}",
            script.display()
        )));
    }
    let mut cmd = std::process::Command::new("python3");
    cmd.arg(&script)
        .arg("--variant")
        .arg(variant)
        .arg("--budget")
        .arg(budget.to_string())
        .arg("--single");
    if let Some(r) = repo {
        cmd.arg("--repo").arg(r);
    }
    if let Some(a) = agent_cmd {
        cmd.arg("--agent-cmd").arg(a);
    }
    if json {
        cmd.arg("--json");
    }
    let out = cmd
        .output()
        .map_err(|e| scc_cli::CliError::Other(format!("spawn python harness: {e}")))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if out.status.code() == Some(2) {
        // SKIPPED-UNINSTALLED: report the variant row with that status
        if json {
            print!("{stdout}");
        } else {
            println!("{variant}  SKIPPED-UNINSTALLED");
            eprintln!("{stdout}");
        }
        return Ok(());
    }
    if !out.status.success() {
        return Err(scc_cli::CliError::Other(format!(
            "run_context_bench.py exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    print!("{stdout}");
    Ok(())
}
