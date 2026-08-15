//! Wave-15 external benchmark suite integration tests: the pinned
//! external-tool lock, the adapters + harness, and the `scc bench external`
//! variant runner (metric row format, artifact generation, and the
//! SKIPPED-UNINSTALLED path for missing external tools).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

// trace:v1 id=test.scc.external-bench work=WORK-SCC-002 verifies=REQ-SCC-IR exercises=impl.scc.bench.variant,impl.scc.cli.main

// trace:exempt reason=unit-test  # external-bench suite test/helper
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

// trace:exempt reason=unit-test  # external-bench suite test/helper
fn benchmarks_dir() -> PathBuf {
    repo_root().join("benchmarks")
}

// trace:exempt reason=unit-test  # external-bench suite test/helper
fn scc() -> &'static str {
    env!("CARGO_BIN_EXE_scc")
}

// trace:exempt reason=unit-test  # external-bench suite test/helper
fn run_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(scc())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("scc binary runs")
}

// trace:exempt reason=unit-test  # external-bench suite test/helper
fn run_in_env(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(scc());
    cmd.args(args).current_dir(dir);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.output().expect("scc binary runs")
}

// trace:exempt reason=unit-test  # external-bench suite test/helper
fn python3() -> String {
    // Resolve the interpreter with the normal PATH first so a restricted
    // PATH below cannot break the adapter invocation itself.
    let out = Command::new("python3")
        .arg("-c")
        .arg("import sys; print(sys.executable)")
        .output()
        .expect("python3 resolves");
    assert!(out.status.success(), "python3 must be installed");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A deterministic fake agent that emits a benchagent JSONL event stream:
/// a plan line naming a ground-truth key, a search, then two file reads.
const FAKE_AGENT: &str = r#"printf '%s\n' '{"type":"item.completed","item":{"type":"agent_message","text":"Plan: cli.rs"}}' '{"type":"item.completed","item":{"type":"command_execution","command":"/bin/zsh -lc \"rg -n paging .\"","exit_code":0}}' '{"type":"item.completed","item":{"type":"mcp_tool_call","tool":"read_file","arguments":{"file_path":"cli.rs"}}}' '{"type":"item.completed","item":{"type":"mcp_tool_call","tool":"read_file","arguments":{"file_path":"main.go"}}}'"#;

// ---------------------------------------------------------------------------
// (a) external-lock.json pins the two external tools
// ---------------------------------------------------------------------------

#[test]
// trace:exempt reason=unit-test  # external-bench suite test/helper
fn external_lock_pins_aider_and_repomix() {
    let text = std::fs::read_to_string(benchmarks_dir().join("external-lock.json"))
        .expect("external-lock.json exists");
    let lock: serde_json::Value = serde_json::from_str(&text).expect("external-lock.json parses");
    let aider = &lock["aider"];
    assert_eq!(aider["repository"], "Aider-AI/aider");
    assert_eq!(
        aider["commit"],
        "5dc9490bb35f9729ef2c95d00a19ccd30c26339c"
    );
    assert_eq!(aider["commit"].as_str().unwrap().len(), 40, "full git sha");
    let repomix = &lock["repomix"];
    assert_eq!(repomix["repository"], "yamadashy/repomix");
    assert_eq!(
        repomix["commit"],
        "e3b15a406ed78d8a463620a032a059ce911bfc0e"
    );
    assert_eq!(repomix["commit"].as_str().unwrap().len(), 40, "full git sha");
}

// ---------------------------------------------------------------------------
// (b) the adapters and harness exist and are executable
// ---------------------------------------------------------------------------

#[test]
// trace:exempt reason=unit-test  # external-bench suite test/helper
fn external_scripts_exist_and_are_executable() {
    for name in [
        "aider_adapter.py",
        "repomix_adapter.py",
        "run_context_bench.py",
        "ground-truth.yaml",
    ] {
        let path = benchmarks_dir().join("external").join(name);
        assert!(path.is_file(), "{name} exists at {}", path.display());
    }
    for name in ["aider_adapter.py", "repomix_adapter.py", "run_context_bench.py"] {
        let path = benchmarks_dir().join("external").join(name);
        let meta = std::fs::metadata(&path).expect("metadata");
        use std::os::unix::fs::PermissionsExt;
        assert!(
            meta.permissions().mode() & 0o111 != 0,
            "{name} is executable"
        );
    }
}

// ---------------------------------------------------------------------------
// (c) the harness --help exits 0
// ---------------------------------------------------------------------------

#[test]
// trace:exempt reason=unit-test  # external-bench suite test/helper
fn harness_help_exits_zero() {
    let py = python3();
    let out = Command::new(&py)
        .arg(benchmarks_dir().join("external").join("run_context_bench.py"))
        .arg("--help")
        .output()
        .expect("harness runs");
    assert!(
        out.status.success(),
        "--help exited {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let help = String::from_utf8_lossy(&out.stdout);
    for variant in ["raw", "aider-repomap", "repomix-compress", "scc-atlas-surface", "scc-full"] {
        assert!(help.contains(variant), "--help lists variant {variant}");
    }
}

// ---------------------------------------------------------------------------
// (d) scc bench external --variant scc-atlas-surface on the cli-service
//     fixture produces the expected metric-row format
// ---------------------------------------------------------------------------

#[test]
// trace:exempt reason=unit-test  # external-bench suite test/helper
fn external_variant_emits_metric_row() {
    let workdir = tempfile::TempDir::new().unwrap();
    let out = run_in(
        workdir.path(),
        &[
            "bench",
            "external",
            "--variant",
            "scc-atlas-surface",
            "--repo",
            "cli-service",
            "--budget",
            "8000",
            "--workdir",
            workdir.path().to_str().unwrap(),
            "--cmd",
            FAKE_AGENT,
            "--json",
        ],
    );
    assert!(
        out.status.success(),
        "scc bench external failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let row: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout is the metric row JSON");

    // §72 columns: variant, success rate, mean exploration, first-plan
    // accuracy, tokens — plus the exploration detail fields.
    assert_eq!(row["variant"], "scc-atlas-surface");
    assert_eq!(row["repo"], "cli-service");
    assert_eq!(row["budget"], 8000);
    assert_eq!(row["tasks"], 3, "cli-service has 3 ground-truth tasks");
    assert_eq!(row["success_rate"], 1.0);
    assert_eq!(row["mean_files_opened"], 2.0, "cli.rs + main.go read");
    assert_eq!(row["mean_search_tool_calls"], 1.0, "one rg search per task");
    assert!(
        row["context_tokens"].as_u64().unwrap() > 0,
        "the startup artifact carries tokens"
    );
    assert!(
        row["mean_exploration"].as_f64().unwrap() >= 3.0,
        "exploration = files + searches + graph queries"
    );
    assert!(
        row["first_plan_accuracy"].as_f64().unwrap() >= 1.0,
        "the fake plan names a ground-truth key for every task"
    );
    assert!(
        row.get("mean_files_opened_before_first_correct").is_some(),
        "wrong-first metric present"
    );

    // The variant artifact landed in the workdir.
    let artifact = workdir.path().join("artifacts").join("cli-service.txt");
    assert!(artifact.is_file(), "artifact written: {}", artifact.display());
    assert!(
        !std::fs::read_to_string(&artifact).unwrap().is_empty(),
        "artifact non-empty"
    );
}

// ---------------------------------------------------------------------------
// (e) the scc-surface variant generates a non-empty surface artifact
// ---------------------------------------------------------------------------

#[test]
// trace:exempt reason=unit-test  # external-bench suite test/helper
fn scc_surface_variant_generates_artifact() {
    let workdir = tempfile::TempDir::new().unwrap();
    let out = run_in(
        workdir.path(),
        &[
            "bench",
            "external",
            "--variant",
            "scc-surface",
            "--repo",
            "cli-service",
            "--budget",
            "8000",
            "--workdir",
            workdir.path().to_str().unwrap(),
            "--cmd",
            FAKE_AGENT,
            "--json",
        ],
    );
    assert!(
        out.status.success(),
        "scc bench external (scc-surface) failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let artifact = workdir.path().join("artifacts").join("cli-service.txt");
    let text = std::fs::read_to_string(&artifact)
        .expect("surface artifact exists")
        .trim()
        .to_string();
    assert!(!text.is_empty(), "surface artifact is non-empty");
    assert!(
        text.contains("cli.rs") || text.contains("cli.py") || text.contains("main.go"),
        "surface artifact names cli-service surfaces: {text}"
    );
}

// ---------------------------------------------------------------------------
// (f) external-tool variants: SKIPPED-UNINSTALLED exits 2 cleanly when the
//     tool is missing (tested under a restricted PATH so the result is
//     deterministic regardless of local installation)
// ---------------------------------------------------------------------------

// trace:exempt reason=unit-test  # external-bench suite test/helper
fn restricted_env() -> Vec<(&'static str, &'static str)> {
    vec![("PATH", "/usr/bin:/bin")]
}

#[test]
// trace:exempt reason=unit-test  # external-bench suite test/helper
fn aider_adapter_skips_cleanly_when_tool_missing() {
    let py = python3();
    let out_dir = tempfile::TempDir::new().unwrap();
    let out = Command::new(&py)
        .arg(benchmarks_dir().join("external").join("aider_adapter.py"))
        .arg(repo_root().join("fixtures").join("cli-service"))
        .arg("8000")
        .arg(out_dir.path())
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("adapter runs");
    assert_eq!(out.status.code(), Some(2), "exit 2 = SKIPPED-UNINSTALLED");
    let payload: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("error JSON on stdout");
    assert_eq!(payload["ok"], false);
    assert!(
        payload["error"]
            .as_str()
            .unwrap()
            .starts_with("SKIPPED-UNINSTALLED"),
        "error names the skip reason: {}",
        payload["error"]
    );
}

#[test]
// trace:exempt reason=unit-test  # external-bench suite test/helper
fn repomix_adapter_skips_cleanly_when_tool_missing() {
    let py = python3();
    let out_dir = tempfile::TempDir::new().unwrap();
    let out = Command::new(&py)
        .arg(benchmarks_dir().join("external").join("repomix_adapter.py"))
        .arg(repo_root().join("fixtures").join("cli-service"))
        .arg("8000")
        .arg(out_dir.path())
        .arg("--compress")
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("adapter runs");
    assert_eq!(out.status.code(), Some(2), "exit 2 = SKIPPED-UNINSTALLED");
    let payload: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("error JSON on stdout");
    assert_eq!(payload["ok"], false);
    assert!(
        payload["error"]
            .as_str()
            .unwrap()
            .starts_with("SKIPPED-UNINSTALLED")
    );
}

#[test]
// trace:exempt reason=unit-test  # external-bench suite test/helper
fn external_variant_delegation_reports_skipped_status() {
    // `scc bench external --variant aider-repomap` delegates to the python
    // harness; with the tool missing it reports SKIPPED-UNINSTALLED and the
    // arm succeeds (exit 0).
    let workdir = tempfile::TempDir::new().unwrap();
    let out = run_in_env(
        workdir.path(),
        &[
            "bench",
            "external",
            "--variant",
            "aider-repomap",
            "--repo",
            "cli-service",
            "--budget",
            "8000",
        ],
        &restricted_env(),
    );
    assert!(
        out.status.success(),
        "delegated run reports the skip without failing: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("SKIPPED-UNINSTALLED"),
        "row names the skip: {stdout}"
    );
}
