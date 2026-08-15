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
fn context_task_appends_task_delta_when_goal_matches() {
    let repo = golden::copy_fixture("cli-service");
    let dir = golden::workdir(repo.path());
    golden::run_ok(&dir, &["index", "--quiet"]);

    let out = golden::run_ok(&dir, &["context", "task", "serve"]);
    assert!(out.contains("# SCC TASK DELTA"), "missing TASK DELTA: {out}");
    assert!(out.contains("TASK-FOCUS: serve"), "missing TASK-FOCUS: {out}");
}
