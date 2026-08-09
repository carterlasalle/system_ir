//! `scc` — System Context Compiler CLI (docs/API_AND_INTEGRATIONS.md §4).

use clap::{Parser, Subcommand};
use scc_cli::commands;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "scc",
    version,
    about = "System Context Compiler — compile repositories into evidence-backed system context for coding agents",
    long_about = "Continuously compiles code, configuration, infrastructure, runtime evidence, and architectural intent \
                   into an evidence-backed machine model of a software system, then emits small task-specific context packs \
                   for coding agents. Give agents more repository understanding per token."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Repository root (default: nearest directory containing .git or .scc)
    #[arg(long, global = true)]
    root: Option<PathBuf>,
}

#[derive(Subcommand)]
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
        /// system-ir.json | system-ir.jsonl | ccg
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

    /// List adapter capability manifests (security audit)
    Adapters {
        #[arg(long)]
        json: bool,
    },

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
    },
    /// Differential resolution benchmark (SCC-126): native vs LSP upgrades
    /// over the fixture corpus, gated on upgrades and unresolved externals
    Resolution {
        /// Maximum allowed fraction of external call candidates left
        /// unresolved after the LSP pass
        #[arg(long, default_value_t = scc_cli::benchres::DEFAULT_MIN_AGREEMENT)]
        min_agreement: f64,
    },
}

#[derive(Subcommand)]
enum ContextSub {
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
        Commands::Index { paths, quiet } => {
            if paths.is_empty() {
                commands::cmd_index(&root, quiet)
            } else {
                commands::cmd_index_paths(&root, &paths, quiet)
            }
        }
        Commands::Status => commands::cmd_status(&root),
        Commands::Watch => commands::cmd_watch(&root),
        Commands::Overview { json } => commands::cmd_overview(&root, json),
        Commands::Context { sub } => match sub {
            ContextSub::Task { goal, files, symbols, budget, json } => {
                commands::cmd_context_task(&root, &goal, &files, &symbols, budget, json)
            }
            ContextSub::Component { id, json } => commands::cmd_context_component(&root, &id, json),
            ContextSub::Flow { id, json } => commands::cmd_context_flow(&root, &id, json),
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
        Commands::Adapters { json } => commands::cmd_adapters(json),
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
            BenchSub::Agent { cmd, min_files } => match scc_cli::benchagent::run_agent_benchmark(&cmd, min_files)
            {
                Ok(summary) => {
                    scc_cli::benchagent::print_agent_summary(&summary);
                    Ok(())
                }
                Err(e) => Err(scc_cli::CliError::Other(e)),
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
        },
    };

    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
