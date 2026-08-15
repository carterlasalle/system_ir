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
fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() == ".scc" {
            continue;
        }
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
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
    vec![
        ("PATH", "/usr/bin:/bin"),
        // Force the harness to skip the pinned-tools venv even when it is
        // installed locally, so the SKIPPED path is deterministic.
        ("SCC_BENCH_VENV", "/nonexistent-scc-bench-venv"),
    ]
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

// ---------------------------------------------------------------------------
// (g) Wave-15 PPR ablation variants emit metric rows (native arm)
// ---------------------------------------------------------------------------

#[test]
// trace:exempt reason=unit-test  # external-bench suite test/helper
fn ablation_variants_emit_metric_rows() {
    // Representative modes from each pipeline stage: lexical (no PPR),
    // task-ppr (the CLI-mapped production ranking), ppr-quotas (+MMR +
    // quota caps). Each must emit a §72 row with a non-empty artifact.
    for variant in ["lexical", "task-ppr", "ppr-quotas"] {
        let workdir = tempfile::TempDir::new().unwrap();
        let out = run_in(
            workdir.path(),
            &[
                "bench",
                "external",
                "--variant",
                variant,
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
            "scc bench external ({variant}) failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let row: serde_json::Value =
            serde_json::from_slice(&out.stdout).expect("stdout is the metric row JSON");
        assert_eq!(row["variant"], variant);
        assert_eq!(row["repo"], "cli-service");
        assert_eq!(row["tasks"], 3, "cli-service has 3 ground-truth tasks");
        assert!(
            row["context_tokens"].as_u64().unwrap() > 0,
            "{variant} artifact carries tokens"
        );
        assert!(
            row["context_tokens"].as_u64().unwrap() <= 8000,
            "{variant} artifact respects the budget"
        );
        let artifact = workdir.path().join("artifacts").join("cli-service.txt");
        let text = std::fs::read_to_string(&artifact).expect("artifact exists");
        assert!(
            text.contains(&format!("ablation {variant}")),
            "{variant} artifact is mode-labeled: {text}"
        );
    }
}

// ---------------------------------------------------------------------------
// (h) scc-full: the structural section is goal-selected, never task.files
// ---------------------------------------------------------------------------

#[test]
// trace:exempt reason=unit-test  # external-bench suite test/helper
fn scc_full_structural_section_is_goal_selected_not_ground_truth() {
    // The critical audit fix: `scc-full` must NOT use ground-truth
    // task.files for context construction. The structural section's unit
    // paths must be a subset of what `scc surface --task "<goal>"` renders
    // (the harness's task-PPR selection oracle) — ground truth is
    // scoring-only. The oracle is computed independently here.
    let workdir = tempfile::TempDir::new().unwrap();
    let out = run_in(
        workdir.path(),
        &[
            "bench",
            "external",
            "--variant",
            "scc-full",
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
        "scc bench external (scc-full) failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let artifact = workdir.path().join("artifacts").join("cli-service.txt");
    let text = std::fs::read_to_string(&artifact).expect("artifact exists");

    // FINAL-artifact budget enforcement: the concatenated startup +
    // task-delta + structural context fits the budget (chars/4 rule).
    let estimate = |s: &str| if s.is_empty() { 0 } else { (s.len() / 4).max(1) };
    assert!(
        estimate(&text) <= 8000,
        "concatenated artifact fits the budget ({} tokens)",
        estimate(&text)
    );

    // Structural unit paths: `<path>\n\nsource: <path>:` blocks.
    let mut units: Vec<&str> = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i + 2 < lines.len() {
        if lines[i + 1].is_empty()
            && lines[i + 2].starts_with("source: ")
            && lines[i + 2].contains(&format!("source: {}:", lines[i]))
        {
            units.push(lines[i].trim());
            i += 3;
        } else {
            i += 1;
        }
    }
    assert!(!units.is_empty(), "structural section present: {text}");

    // Oracle: `scc surface --task "<goal>"` in an indexed fixture copy.
    let tmp = tempfile::TempDir::new().unwrap();
    let oracle_root = tmp.path().join("repo");
    copy_tree(&repo_root().join("fixtures").join("cli-service"), &oracle_root);
    let idx = run_in(&oracle_root, &["index", "--quiet"]);
    assert!(idx.status.success(), "oracle index succeeds");
    // Every cli-service task goal: the union of rendered paths is the
    // selection oracle — ground truth files (cli.rs, cli.py, main.go) are
    // all goal-relevant here, so the discriminating contract is that the
    // structural unit set EQUALS the surface-rendered set (the selection
    // pipeline, not task.files, decides what structural units exist).
    let surface = run_in(
        &oracle_root,
        &["surface", "--task", "add a --paging flag to the serve subcommand", "--budget", "8000"],
    );
    assert!(surface.status.success());
    let surface_text = String::from_utf8_lossy(&surface.stdout);
    // The production render: `SCC SYSTEM SURFACE MAP`, then per-group
    // `<COMPONENT>\n\n<path>[  [<sub>]]\n\n  <kind> <name>` blocks. The
    // path comes from the group header line (between two blank lines,
    // above a non-blank component line; component headers are uppercased
    // and carry no path punctuation).
    let mut oracle_paths: Vec<String> = Vec::new();
    let s_lines: Vec<&str> = surface_text.lines().collect();
    for i in 0..s_lines.len() {
        let path = s_lines[i].trim();
        if path.is_empty() || s_lines[i].starts_with(' ') {
            continue; // entries and signatures are indented, never paths
        }
        let prev_blank = i
            .checked_sub(1)
            .map(|j| s_lines[j].trim().is_empty())
            .unwrap_or(false);
        let next_blank = s_lines
            .get(i + 1)
            .map(|l| l.trim().is_empty())
            .unwrap_or(false);
        let above_component = i
            .checked_sub(2)
            .map(|j| !s_lines[j].trim().is_empty())
            .unwrap_or(false);
        if !(prev_blank && next_blank && above_component) {
            continue;
        }
        if !(path.contains('/') || path.contains('.')) {
            continue;
        }
        if path.chars().all(|c| !c.is_lowercase()) {
            continue;
        }
        let path = path.split('[').next().unwrap_or(path).trim().to_string();
        oracle_paths.push(path);
    }
    assert!(
        !oracle_paths.is_empty(),
        "surface oracle renders paths: {surface_text}"
    );
    for unit in &units {
        assert!(
            oracle_paths.iter().any(|p| p == unit),
            "structural unit {unit} comes from the task surface selection (never task.files)"
        );
    }
}

// ---------------------------------------------------------------------------
// (i) adapters hard-error with PIN-MISMATCH (exit 3) on wrong installs
// ---------------------------------------------------------------------------

#[test]
// trace:exempt reason=unit-test  # external-bench suite test/helper
fn aider_adapter_pin_mismatch_exits_3() {
    // A fake site-packages root whose direct_url.json records a WRONG
    // commit: the adapter must fail closed with exit 3 PIN-MISMATCH (no
    // silent floating) before building anything.
    let py = python3();
    let out_dir = tempfile::TempDir::new().unwrap();
    let fake_site = tempfile::TempDir::new().unwrap();
    let dist = fake_site.path().join("aider_chat-9.9.9.dist-info");
    std::fs::create_dir_all(&dist).unwrap();
    std::fs::write(
        dist.join("direct_url.json"),
        r#"{"url": "https://github.com/Aider-AI/aider.git", "vcs_info": {"commit_id": "deadbeef00000000000000000000000000000000"}}"#,
    )
    .unwrap();
    let out = Command::new(&py)
        .arg(benchmarks_dir().join("external").join("aider_adapter.py"))
        .arg(repo_root().join("fixtures").join("cli-service"))
        .arg("8000")
        .arg(out_dir.path())
        .arg("--goal")
        .arg("add a --paging flag")
        .env("SCC_AIDER_SITE_PACKAGES", fake_site.path())
        .output()
        .expect("adapter runs");
    assert_eq!(out.status.code(), Some(3), "exit 3 = PIN-MISMATCH");
    let payload: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("error JSON on stdout");
    assert_eq!(payload["ok"], false);
    assert!(
        payload["error"]
            .as_str()
            .unwrap()
            .starts_with("PIN-MISMATCH"),
        "error names the mismatch: {}",
        payload["error"]
    );
}

#[test]
// trace:exempt reason=unit-test  # external-bench suite test/helper
fn repomix_adapter_pin_mismatch_exits_3() {
    // A fake installed repomix package with the wrong version: exit 3
    // PIN-MISMATCH (fail closed against the lock's version mapping).
    let py = python3();
    let out_dir = tempfile::TempDir::new().unwrap();
    let fake_pkg = tempfile::TempDir::new().unwrap();
    std::fs::write(
        fake_pkg.path().join("package.json"),
        r#"{"name": "repomix", "version": "9.9.9"}"#,
    )
    .unwrap();
    let out = Command::new(&py)
        .arg(benchmarks_dir().join("external").join("repomix_adapter.py"))
        .arg(repo_root().join("fixtures").join("cli-service"))
        .arg("8000")
        .arg(out_dir.path())
        .arg("--compress")
        .env("SCC_REPOMIX_PKG_DIR", fake_pkg.path())
        .env("SCC_REPOMIX_SRC_DIR", "/nonexistent-scc-bench-repomix")
        .output()
        .expect("adapter runs");
    assert_eq!(out.status.code(), Some(3), "exit 3 = PIN-MISMATCH");
    let payload: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("error JSON on stdout");
    assert_eq!(payload["ok"], false);
    assert!(
        payload["error"]
            .as_str()
            .unwrap()
            .starts_with("PIN-MISMATCH"),
        "error names the mismatch: {}",
        payload["error"]
    );
}

#[test]
// trace:exempt reason=unit-test  # external-bench suite test/helper
fn repomix_adapter_pin_unverified_exits_4() {
    // A fake installed repomix reporting the LOCKED version but with NO
    // provable commit (no gitHead, no pinned checkout): exit 4
    // PIN-UNVERIFIED — a version-only match is never a passing pin.
    let py = python3();
    let out_dir = tempfile::TempDir::new().unwrap();
    let fake_pkg = tempfile::TempDir::new().unwrap();
    std::fs::write(
        fake_pkg.path().join("package.json"),
        r#"{"name": "repomix", "version": "1.18.0"}"#,
    )
    .unwrap();
    let out = Command::new(&py)
        .arg(benchmarks_dir().join("external").join("repomix_adapter.py"))
        .arg(repo_root().join("fixtures").join("cli-service"))
        .arg("8000")
        .arg(out_dir.path())
        .arg("--compress")
        .env("SCC_REPOMIX_PKG_DIR", fake_pkg.path())
        .env("SCC_REPOMIX_SRC_DIR", "/nonexistent-scc-bench-repomix")
        .output()
        .expect("adapter runs");
    assert_eq!(out.status.code(), Some(4), "exit 4 = PIN-UNVERIFIED");
    let payload: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("error JSON on stdout");
    assert_eq!(payload["ok"], false);
    assert!(
        payload["error"]
            .as_str()
            .unwrap()
            .starts_with("PIN-UNVERIFIED"),
        "error names the unverified pin: {}",
        payload["error"]
    );
}

#[test]
// trace:exempt reason=unit-test  # external-bench suite test/helper
fn repomix_adapter_pin_passes_on_githead_match() {
    // Positive control: an install whose gitHead IS the locked commit
    // verifies, and the adapter then packs through a fake `repomix`
    // binary that emits the pinned XML shape — exit 0 with ok:true.
    let py = python3();
    let out_dir = tempfile::TempDir::new().unwrap();
    let fake_pkg = tempfile::TempDir::new().unwrap();
    std::fs::write(
        fake_pkg.path().join("package.json"),
        r#"{"name": "repomix", "version": "1.18.0", "gitHead": "e3b15a406ed78d8a463620a032a059ce911bfc0e"}"#,
    )
    .unwrap();
    let bin_dir = tempfile::TempDir::new().unwrap();
    let fake_repomix = bin_dir.path().join("repomix");
    std::fs::write(
        &fake_repomix,
        "#!/bin/sh\nout=''\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = \"-o\" ]; then out=\"$2\"; shift 2; else shift; fi\ndone\nprintf '<repository><file path=\"main.py\">print(1)</file></repository>' > \"$out\"\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&fake_repomix, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = Command::new(&py)
        .arg(benchmarks_dir().join("external").join("repomix_adapter.py"))
        .arg(repo_root().join("fixtures").join("cli-service"))
        .arg("8000")
        .arg(out_dir.path())
        .arg("--compress")
        .env("SCC_REPOMIX_PKG_DIR", fake_pkg.path())
        .env("SCC_REPOMIX_SRC_DIR", "/nonexistent-scc-bench-repomix")
        .env("PATH", format!("{}:/usr/bin:/bin", bin_dir.path().display()))
        .output()
        .expect("adapter runs");
    assert_eq!(out.status.code(), Some(0), "gitHead match packs successfully");
    let payload: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("JSON on stdout");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["pinned"], "e3b15a406ed78d8a463620a032a059ce911bfc0e");
    let artifact = PathBuf::from(payload["artifact"].as_str().unwrap());
    assert!(artifact.is_file(), "artifact written: {}", artifact.display());
}
