//! Wave 14C/E/F integration tests: the deterministic startup artifact
//! (`scc context startup`), the System Surface Map (`scc surface`, global
//! and task-personalized), and the task delta appended to `scc context
//! task`. Uses the cli-service fixture (python argparse + rust clap + go
//! cobra + package.json surfaces).

mod golden;

#[test]
// trace:exempt reason=unit-test
// trace:v1 id=test.scc.surface-startup work=WORK-SCC-014 verifies=REQ-SCC-IR exercises=impl.scc.surface,impl.scc.context.startup
fn startup_artifact_has_all_sections_and_is_deterministic() {
    let repo = golden::copy_fixture("cli-service");
    let dir = golden::workdir(repo.path());
    golden::run_ok(&dir, &["index", "--quiet"]);

    let first = golden::run_ok(&dir, &["context", "startup"]);
    for header in [
        "# SCC SYSTEM CONTEXT",
        "## SYSTEM ATLAS",
        "## SYSTEM SURFACE MAP",
        "## MODEL COVERAGE",
        "## OMISSIONS",
    ] {
        assert!(first.contains(header), "missing {header:?} in startup output");
    }
    assert!(
        first.contains("sha256:"),
        "startup must carry the artifact hash: {first}"
    );

    // prompt-cache stability: a second run is byte-identical
    let second = golden::run_ok(&dir, &["context", "startup"]);
    assert_eq!(first, second, "startup must be byte-identical across runs");
}

#[test]
// trace:exempt reason=unit-test
fn surface_shows_component_grouped_api_map() {
    let repo = golden::copy_fixture("cli-service");
    let dir = golden::workdir(repo.path());
    golden::run_ok(&dir, &["index", "--quiet"]);

    let out = golden::run_ok(&dir, &["surface"]);
    // known fixture callable APIs surface in the map
    assert!(out.contains("serve"), "surface must mention serve: {out}");
    assert!(out.contains("deploy"), "surface must mention deploy: {out}");
    assert!(!out.is_empty(), "surface output must not be empty");
}

#[test]
// trace:exempt reason=unit-test
fn surface_task_personalizes_the_map() {
    let repo = golden::copy_fixture("cli-service");
    let dir = golden::workdir(repo.path());
    golden::run_ok(&dir, &["index", "--quiet"]);

    let out = golden::run_ok(&dir, &["surface", "--task", "serve"]);
    assert!(
        out.contains("task-personalized"),
        "task surface must carry the personalized header: {out}"
    );
    assert!(out.contains("serve"), "task surface must mention serve: {out}");
}

#[test]
// trace:exempt reason=unit-test
fn surface_explain_appends_rank_reasons() {
    // Wave 15.1/15.2: `--explain` flows through build_surface
    // (request.explain) and renders the FULL per-entry score
    // decomposition — all eight components + total + reasons, never a
    // bare `importance:`.
    let repo = golden::copy_fixture("cli-service");
    let dir = golden::workdir(repo.path());
    golden::run_ok(&dir, &["index", "--quiet"]);

    let plain = golden::run_ok(&dir, &["surface"]);
    let explained = golden::run_ok(&dir, &["surface", "--explain"]);
    assert!(
        explained.contains("importance:"),
        "--explain must append per-entry importance: {explained}"
    );
    for key in [
        "task_ppr:",
        "global_ppr:",
        "lexical:",
        "semantic:",
        "confidence:",
        "criticality:",
        "change_risk:",
        "novelty:",
        "because:",
    ] {
        assert!(
            explained.contains(key),
            "--explain must render the {key:?} component: {explained}"
        );
    }
    assert_ne!(
        plain, explained,
        "--explain output must differ from the plain render"
    );
}

#[test]
// trace:exempt reason=unit-test
fn context_task_appends_task_delta_when_goal_matches() {
    let repo = golden::copy_fixture("cli-service");
    let dir = golden::workdir(repo.path());
    golden::run_ok(&dir, &["index", "--quiet"]);

    let out = golden::run_ok(&dir, &["context", "task", "serve"]);
    assert!(out.contains("# SCC TASK DELTA"), "missing TASK DELTA: {out}");
    assert!(out.contains("TASK-FOCUS: serve"), "missing TASK-FOCUS: {out}");
}

/// Replicate the CLI's compiler exactly (default config → rank salt
/// `false::`), so the store cache key matches what the CLI process wrote.
/// Scoped: the store outlives the compiler borrow inside the closure.
// trace:exempt reason=internal-detail  # test helper; the traced tests below exercise it
fn with_cli_compiler<T>(
    dir: &std::path::Path,
    f: impl FnOnce(&scc_store::Store, &scc_cli::Compiler) -> T,
) -> T {
    let store = scc_store::Store::open(&dir.join(".scc").join("scc.db"), dir).unwrap();
    let comp = scc_cli::compiler(&store, &scc_indexer::Config::default(), Vec::new()).unwrap();
    f(&store, &comp)
}

#[test]
// trace:exempt reason=unit-test
// trace:v1 id=test.scc.surface-startup.rank-cache verifies=REQ-global-rank-cached-per-model-epoch exercises=impl.scc.startup.rank-cache-load
fn startup_rank_cache_persists_and_is_reused_across_runs() {
    // Wave 15.2: the per-ModelEpoch global rank cache. Two consecutive
    // `scc context startup` runs: the first stores the entry (miss), the
    // second reuses it (the deterministic `hits` marker increments).
    let repo = golden::copy_fixture("cli-service");
    let dir = golden::workdir(repo.path());
    golden::run_ok(&dir, &["index", "--quiet"]);

    golden::run_ok(&dir, &["context", "startup"]);
    golden::run_ok(&dir, &["context", "startup"]);

    with_cli_compiler(&dir, |store, comp| {
        let ctx = comp.ctx();
        let cache = scc_context::startup::load_global_rank_cache(&ctx)
            .expect("rank cache entry must exist after the first startup");
        assert!(
            cache.hits >= 1,
            "the second startup must reuse the cache (hits marker {} >= 1)",
            cache.hits
        );
        let epoch = store.cache_epoch().unwrap();
        assert_eq!(cache.epoch, epoch, "cache epoch pins the model epoch");
        assert_eq!(cache.candidates_epoch, epoch, "candidates are epoch-stable");
        assert!(
            !cache.candidate_ids.is_empty(),
            "epoch-stable candidate id list cached"
        );
        assert!(
            !cache.node_symbol_map.is_empty(),
            "global projection map cached"
        );
    });
}

#[test]
// trace:exempt reason=unit-test
// trace:v1 id=test.scc.surface-startup.ledger-same-render verifies=REQ-global-rank-cached-per-model-epoch exercises=impl.scc.context.startup
fn startup_ledger_records_the_same_render_it_printed() {
    // Wave 15.2 single-pass startup: the ledger's visible ids MUST come
    // from the SAME render the artifact printed — the surface is built
    // once, never rebuilt for recording. The CLI's startup output is
    // byte-identical to a library rebuild (deterministic), and the
    // recorded ledger equals the visible ids derived from that render.
    let repo = golden::copy_fixture("cli-service");
    let dir = golden::workdir(repo.path());
    golden::run_ok(&dir, &["index", "--quiet"]);

    let out = golden::run_ok(&dir, &["context", "startup"]);
    with_cli_compiler(&dir, |store, comp| {
        let ctx = comp.ctx();

        let budget = scc_core::ContextBudget::default();
        let sc = scc_context::startup::build_startup(
            &ctx,
            &budget,
            scc_context::startup::RENDERER_VERSION,
        );
        // The artifact the CLI printed is exactly this render (same epoch,
        // same default budget, deterministic pipeline).
        assert_eq!(
            out,
            scc_context::startup::render_startup(&sc),
            "CLI startup output must match the deterministic rebuild"
        );

        // Every rendered entry appears in the printed text (the render
        // block carries the entry's simple name).
        for id in &sc.surface_render.rendered_ids {
            let symbol = id
                .rsplit_once("#overload")
                .map(|(l, _)| l)
                .unwrap_or(id);
            let name = symbol.rsplit('/').next().unwrap_or(symbol);
            assert!(
                out.contains(name),
                "printed startup must contain rendered entry {id}"
            );
        }

        // ...and the ledger records exactly the ids derived from that same
        // render (never a rebuild, never the candidate pool).
        let (syms, files, comps, flows) =
            scc_context::startup::visible_ids_from_startup(&ctx, &sc);
        let ledger_store = scc_context::context_ledger::ContextLedgerStore::new(store);
        let led = ledger_store.load();
        assert_eq!(
            led.visible_symbols, syms,
            "ledger symbols must match the render-derived visible set"
        );
        assert_eq!(
            led.visible_files, files,
            "ledger files must match the render-derived visible set"
        );
        assert_eq!(
            led.visible_components, comps,
            "ledger components must match the render-derived visible set"
        );
        assert_eq!(
            led.visible_flows, flows,
            "ledger flows must match the render-derived visible set"
        );
    });
}

#[test]
// trace:exempt reason=unit-test
// trace:v1 id=test.scc.surface-startup.adaptive-budgets verifies=REQ-adaptive-startup-budgets exercises=impl.scc.core.budget-adaptive
fn startup_budget_adapts_to_repo_complexity() {
    // Wave 15.2 adaptive budgets: the fixed 13:7 split is replaced by
    // complexity tiers. The tiny cli-service fixture with `--budget
    // 20000` must get the tiny 55/45 split — a 9000-token surface slice
    // (vs the old 7000-token fixed share), visible in the coverage line.
    let repo = golden::copy_fixture("cli-service");
    let dir = golden::workdir(repo.path());
    golden::run_ok(&dir, &["index", "--quiet"]);

    let out = golden::run_ok(&dir, &["context", "startup", "--budget", "20000"]);
    assert!(
        out.contains("(budget 9000)"),
        "tiny repo must get the 45% surface share (budget 9000): {out}"
    );

    // A large repo (entity_count > 5000) must produce a different split
    // from the tiny one — the same total, different atlas/surface cut.
    let tiny = scc_core::ContextBudget::adaptive(20_000, 50, 2, 1, 10);
    let large = scc_core::ContextBudget::adaptive(20_000, 6_000, 5, 1, 10);
    assert_ne!(tiny.surface, large.surface, "tiny vs large splits differ");
    assert!(
        (tiny.surface as f64 / tiny.total as f64 - 0.45).abs() <= 0.01,
        "tiny surface share is 45%"
    );
    assert!(
        (large.surface as f64 / large.total as f64 - 0.35).abs() <= 0.01,
        "large surface share is 35%"
    );
}
