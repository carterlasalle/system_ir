//! Oh My Pi (OMP) integration (docs/API_AND_INTEGRATIONS.md §5, P0).
//!
//! `scc setup omp` installs a native project-scoped integration with NO
//! manual post-config:
//!   1. `.omp/extensions/scc/index.ts` + `package.json` — ONE native
//!      extension module that registers ALL ordering-dependent SCC
//!      lifecycle behavior (`session_start`, `before_agent_start`,
//!      `tool_result`, `session_before_compact`, `session.compacting`).
//!   2. `.omp/mcp.json` — merged (preserving existing servers) wiring the
//!      SCC MCP server (`scc mcp`) for manual semantic drill-down. The
//!      command honors `SCC_BIN` when set at setup time.
//!   3. `.omp/skills/scc-system-context/SKILL.md` — on-demand workflow
//!      guidance (no hard rules live only in the skill).
//!   4. AGENTS.md — SCC DURABLE RULES ONLY (never generated Atlas/Surface
//!      facts). Native `.omp/AGENTS.md` has higher provider priority than
//!      the standalone root `AGENTS.md`. The installer patches the
//!      winning existing file and never silently creates a shadowing
//!      `.omp/AGENTS.md` when only the root file exists.
//!
//! The extension uses `pi.exec(process.env.SCC_BIN ?? "scc", [...])`
//! argument arrays (never shell interpolation), preserves arg boundaries,
//! captures exit/stdout/stderr, catches errors, and never crashes the OMP
//! session.

use std::path::Path;

use crate::agents_md::{
    replace_scc_section_in, resolve_omp_agents_path, AgentsTarget,
    SCC_OMP_SECTION_CLOSE, SCC_OMP_SECTION_OPEN,
};

// The native extension entry: ONE module registers every ordering-dependent
// SCC lifecycle behavior (OMP does not promise filename/module ordering).
const EXTENSION_TS: &str = include_str!("../../../plugins/omp/scc/index.ts");
const EXTENSION_PACKAGE: &str = include_str!("../../../plugins/omp/scc/package.json");
const SKILL_MD: &str = include_str!("../../../plugins/omp/scc/skills/scc-system-context/SKILL.md");

// trace:exempt reason=const-data (behavior boundary is write_agents_rules)
const AGENTS_RULES: &str = "<!-- SCC-OMP-SECTION -->\n\
# SCC (System Context Compiler)\n\
This repository is indexed by SCC. Durable rules:\n\
- The native SCC extension injects the startup architecture once per session\n\
  and a task-specific context pack per prompt. Work within the injected task\n\
  context; it is the authoritative system slice for the current goal.\n\
- For the fused startup architecture (Atlas + Surface + coverage),\n\
  `scc context startup` is the canonical command; `scc atlas` is only its\n\
  Level-0 Atlas component.\n\
- Authority ordering: source/runtime > SCC System IR > checkpoint > Hindsight\n\
  > model assumption.\n\
- Drift and invariants: `scc drift`, `scc ci check`, and `scc impact <files>`\n\
  before cross-layer edits.\n\
<!-- /SCC-OMP-SECTION -->\n";

/// `scc setup omp` — install the native OMP integration.
// trace:v1 id=impl.crates-scc-cli-src-plugin-omp.cmd-setup-omp work=WORK-SCC-001 satisfies=REQ-SCC-API
pub fn cmd_setup_omp(root: &Path) -> crate::Result<()> {
    let omp_dir = root.join(".omp");
    // 1. Native extension package.
    let ext_dir = omp_dir.join("extensions/scc");
    std::fs::create_dir_all(&ext_dir)?;
    std::fs::write(ext_dir.join("index.ts"), installable_extension_ts())?;
    std::fs::write(ext_dir.join("package.json"), extension_package())?;
    println!("wrote {}", ext_dir.join("index.ts").display());

    // 2. Merge `.omp/mcp.json`, preserving existing servers.
    merge_mcp_json(&omp_dir)?;

    // 3. Skill (on-demand workflow guidance).
    let skill_dir = omp_dir.join("skills/scc-system-context");
    std::fs::create_dir_all(&skill_dir)?;
    std::fs::write(skill_dir.join("SKILL.md"), skill_md())?;
    println!("wrote {}", skill_dir.join("SKILL.md").display());

    // 4. Durable rules: patch the OMP-winning instruction file; never
    // silently introduce a shadowing `.omp/AGENTS.md` when only the
    // root AGENTS.md exists.
    let agents_path = write_agents_rules(root)?;

    println!();
    println!("OMP integration installed:");
    println!("  extension  -> {}", ext_dir.join("index.ts").display());
    println!("  mcp.json   -> {}", omp_dir.join("mcp.json").display());
    println!("  skill      -> {}", skill_dir.join("SKILL.md").display());
    println!("  AGENTS.md  -> {}", agents_path.display());
    println!();
    println!("Restart OMP, then run `/extensions` to verify the SCC extension");
    println!("is loaded (`/extensions` is an inspector, not a reload).");
    println!("Verify MCP with `/mcp reload` then `/mcp test scc`.");
    println!(
        "The `scc` binary must be on PATH, or set SCC_BIN (the extension reads process.env.SCC_BIN)."
    );
    Ok(())
}

/// The extension source with system_ir's authoring markers rewritten to
/// exempt comments: the installed artifact runs in a user's repository,
/// where this repo's work/requirement nodes do not exist (dangling TL002
/// edges under TraceLayer otherwise).
// trace:exempt reason=internal-helper
fn installable_extension_ts() -> String {
    EXTENSION_TS.replace(
        "// trace:v1 id=impl.omp.scc work=WORK-SCC-001 satisfies=REQ-SCC-API",
        "// trace:exempt reason=scc-installed-tooling (authoring marker from the SCC source repo removed at install)",
    )
}

// trace:exempt reason=internal-helper
fn extension_package() -> &'static str {
    EXTENSION_PACKAGE
}

// trace:exempt reason=internal-helper
fn skill_md() -> &'static str {
    SKILL_MD
}

// trace:exempt reason=internal-helper
fn scc_bin() -> String {
    if let Ok(v) = std::env::var("SCC_BIN") {
        let v = v.trim().to_string();
        if !v.is_empty() {
            return v;
        }
    }
    // Absolute path only when the current exe IS the scc binary (exact
    // file-stem match — a test-harness or build-tool exe path must never
    // leak into generated configs).
    if let Ok(exe) = std::env::current_exe() {
        if exe.file_stem().map(|n| n == "scc").unwrap_or(false) {
            return exe.to_string_lossy().into_owned();
        }
    }
    "scc".to_string()
}

/// Merge the SCC MCP server into `.omp/mcp.json`, preserving any existing
/// servers. The format is the standard MCP config: `{ "mcpServers": {
/// "<name>": { "command": ..., "args": [...] } } }`. Idempotent: if the
/// `scc` server is already present, it is left unchanged unless SCC_BIN
/// is set (then the command is updated to the known static path).
// trace:v1 id=impl.crates-scc-cli-src-plugin-omp.merge-mcp-json work=WORK-SCC-001 satisfies=REQ-SCC-API
fn merge_mcp_json(omp_dir: &Path) -> crate::Result<()> {
    let path = omp_dir.join("mcp.json");
    let mut v: serde_json::Value = if path.exists() {
        let text = std::fs::read_to_string(&path)?;
        serde_json::from_str(&text).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    let servers = v
        .get_mut("mcpServers")
        .and_then(|m| m.as_object_mut())
        .cloned()
        .unwrap_or_default();
    let mut servers = servers;
    let cmd = scc_bin();
    match servers.get_mut("scc") {
        Some(existing) => {
            if std::env::var("SCC_BIN").map(|s| !s.trim().is_empty()).unwrap_or(false) {
                existing["command"] = serde_json::Value::String(cmd);
            }
        }
        None => {
            servers.insert(
                "scc".to_string(),
                serde_json::json!({"command": cmd, "args": ["mcp"]}),
            );
        }
    }
    v["mcpServers"] = serde_json::Value::Object(servers);
    std::fs::write(&path, serde_json::to_string_pretty(&v)?)?;
    Ok(())
}

/// Write the durable SCC rules into the OMP-winning AGENTS.md, preserving
/// user content on BOTH sides of the managed section (idempotent).
// trace:v1 id=impl.crates-scc-cli-src-plugin-omp.write-agents-rules work=WORK-SCC-001 satisfies=REQ-SCC-API
fn write_agents_rules(root: &Path) -> crate::Result<std::path::PathBuf> {
    let resolution = resolve_omp_agents_path(root);
    if resolution.both_exist {
        eprintln!(
            "warning: both .omp/AGENTS.md and AGENTS.md exist; OMP prefers .omp/AGENTS.md \
             (higher-priority same-scope context) and will shadow the root file. \
             Patching .omp/AGENTS.md. Remove one of the files if that precedence is unintended."
        );
    }
    let path = match resolution.target {
        AgentsTarget::Omp => root.join(".omp").join("AGENTS.md"),
        AgentsTarget::Root => root.join("AGENTS.md"),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let out = replace_scc_section_in(
        &existing,
        AGENTS_RULES,
        SCC_OMP_SECTION_OPEN,
        SCC_OMP_SECTION_CLOSE,
    );
    std::fs::write(&path, out)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Holds the process-wide SCC_BIN lock for the lifetime of setup/merge
    /// tests. `mcp_json_honors_scc_bin_at_setup` mutates the env; any parallel
    /// test that reads SCC_BIN without this guard will flake (CI PR job:
    /// `mcp_merge_preserves_existing_servers` saw `/opt/custom/scc`).
    // trace:exempt reason=internal-helper
    struct SccBinGuard {
        _lock: MutexGuard<'static, ()>,
        prev: Option<String>,
    }

    // trace:exempt reason=internal-helper
    impl Drop for SccBinGuard {
        // trace:exempt reason=internal-helper
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var("SCC_BIN", v),
                None => std::env::remove_var("SCC_BIN"),
            }
        }
    }

    // trace:exempt reason=internal-helper
    fn lock_scc_bin(value: Option<&str>) -> SccBinGuard {
        let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("SCC_BIN").ok();
        match value {
            Some(v) => std::env::set_var("SCC_BIN", v),
            None => std::env::remove_var("SCC_BIN"),
        }
        SccBinGuard { _lock: lock, prev }
    }

    // trace:exempt reason=internal-helper
    fn installed_extension(root: &Path) -> String {
        std::fs::read_to_string(root.join(".omp/extensions/scc/index.ts")).unwrap()
    }

    #[test]
    // trace:v1 id=test.scc-cli-plugin-omp.installs-extension-and-mcp work=WORK-SCC-001 verifies=REQ-SCC-API
    fn installs_extension_and_mcp() {
        let _env = lock_scc_bin(None);
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        cmd_setup_omp(&root).unwrap();

        // Extension files are present.
        assert!(root.join(".omp/extensions/scc/index.ts").exists());
        assert!(root.join(".omp/extensions/scc/package.json").exists());
        // The extension imports the resolvable package.
        let ts = installed_extension(&root);
        assert!(ts.contains("@oh-my-pi/pi-coding-agent"), "extension must import the canonical resolvable package");
        assert!(ts.contains("before_agent_start"), "extension must wire before_agent_start");
        assert!(ts.contains("session_start"), "extension must wire session_start");

        // mcp.json has the scc server.
        let mcp: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(".omp/mcp.json")).unwrap()).unwrap();
        assert_eq!(mcp["mcpServers"]["scc"]["command"], "scc");
        assert_eq!(mcp["mcpServers"]["scc"]["args"][0], "mcp");

        // Neither file existed: canonical root AGENTS.md, no shadowing .omp/AGENTS.md.
        assert!(root.join("AGENTS.md").exists(), "canonical instruction file is root AGENTS.md");
        assert!(
            !root.join(".omp/AGENTS.md").exists(),
            "must not create shadowing .omp/AGENTS.md when root did not exist"
        );
        let agents = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
        assert!(agents.contains("SCC (System Context Compiler)"), "AGENTS.md must carry SCC durable rules");
        let skill = std::fs::read_to_string(root.join(".omp/skills/scc-system-context/SKILL.md")).unwrap();
        assert!(skill.contains("`system_context`"), "skill must teach system_context");
        assert!(skill.contains("`surface_map`"), "skill must teach surface_map");
        assert!(skill.contains("`structural_source`"), "skill must teach structural_source");
        assert!(!skill.contains("| `system_overview` |"), "skill table must not lead with the retired startup tool");
    }

    #[test]
    // trace:v1 id=test.scc-cli-plugin-omp.mcp-merge-preserves-existing work=WORK-SCC-001 verifies=REQ-SCC-API
    fn mcp_merge_preserves_existing_servers() {
        let _env = lock_scc_bin(None);
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let omp = root.join(".omp");
        std::fs::create_dir_all(&omp).unwrap();
        std::fs::write(
            omp.join("mcp.json"),
            r#"{"mcpServers":{"existing":{"command":"foo","args":["bar"]}}}"#,
        )
        .unwrap();
        merge_mcp_json(&omp).unwrap();
        let mcp: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(omp.join("mcp.json")).unwrap()).unwrap();
        assert_eq!(mcp["mcpServers"]["existing"]["command"], "foo", "existing server must be preserved");
        assert_eq!(mcp["mcpServers"]["scc"]["command"], "scc", "scc server must be added");
    }

    #[test]
    // trace:v1 id=test.scc-cli-plugin-omp.agents-idempotent-preserves-user work=WORK-SCC-001 verifies=REQ-SCC-API
    fn agents_idempotent_preserves_user_content() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let omp = root.join(".omp");
        std::fs::create_dir_all(&omp).unwrap();
        std::fs::write(omp.join("AGENTS.md"), "USER CONTENT\n").unwrap();
        write_agents_rules(&root).unwrap();
        let first = std::fs::read_to_string(omp.join("AGENTS.md")).unwrap();
        assert!(first.starts_with("USER CONTENT"), "user content must be preserved");
        assert!(first.contains("SCC (System Context Compiler)"), "SCC rules must be appended");
        // Idempotent: a second run does not duplicate the SCC section.
        write_agents_rules(&root).unwrap();
        let second = std::fs::read_to_string(omp.join("AGENTS.md")).unwrap();
        assert_eq!(
            first.matches("SCC (System Context Compiler)").count(),
            second.matches("SCC (System Context Compiler)").count(),
            "SCC rules must not duplicate on reinstall"
        );
    }

    #[test]
    // trace:v1 id=test.scc-cli-plugin-omp.agents-preserves-after-marker work=WORK-SCC-001 verifies=REQ-SCC-API,REQ-implement-p0-omp-integration-correctness-and-writable-benchmark-scient
    fn agents_rewrite_preserves_text_after_closing_marker() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join(".omp")).unwrap();
        std::fs::write(
            root.join(".omp/AGENTS.md"),
            "BEFORE\n<!-- SCC-OMP-SECTION -->\nold managed\n<!-- /SCC-OMP-SECTION -->\nKEEP AFTER\n",
        )
        .unwrap();
        write_agents_rules(&root).unwrap();
        let text = std::fs::read_to_string(root.join(".omp/AGENTS.md")).unwrap();
        assert!(text.contains("BEFORE"), "{text}");
        assert!(text.contains("KEEP AFTER"), "text after the closing marker must survive: {text}");
        assert!(!text.contains("old managed"), "{text}");
    }

    #[test]
    // trace:v1 id=test.scc-cli-plugin-omp.agents-patches-root-not-shadow work=WORK-SCC-001 verifies=REQ-SCC-API,REQ-implement-p0-omp-integration-correctness-and-writable-benchmark-scient
    fn agents_patches_existing_root_and_does_not_create_shadow() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("AGENTS.md"), "# project rules\nkeep me\n").unwrap();
        let path = write_agents_rules(&root).unwrap();
        assert_eq!(path, root.join("AGENTS.md"));
        assert!(!root.join(".omp/AGENTS.md").exists(), "must not create shadowing .omp/AGENTS.md");
        let text = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
        assert!(text.contains("keep me"), "{text}");
        assert!(text.contains("SCC (System Context Compiler)"), "{text}");
    }

    #[test]
    // trace:v1 id=test.scc-cli-plugin-omp.omp-codex-sections-coexist work=WORK-SCC-001 verifies=REQ-SCC-API
    fn omp_and_codex_sections_coexist_in_one_file() {
        // The OMP and Codex installers share instruction files; each owns
        // a DISTINCT marker pair so neither installer deletes the other's
        // section (last-writer-wins data loss). Order-independent.
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("AGENTS.md"),
            "# user\n<!-- SCC-SECTION -->\ncodex capsule\n<!-- /SCC-SECTION -->\n",
        )
        .unwrap();
        write_agents_rules(&root).unwrap();
        let text = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
        assert!(text.contains("codex capsule"), "codex section must survive OMP setup: {text}");
        assert!(text.contains("SCC-OMP-SECTION"), "OMP section must be installed: {text}");
        // Reinstall: both sections still present exactly once.
        write_agents_rules(&root).unwrap();
        let again = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
        assert_eq!(again.matches("SCC-OMP-SECTION").count(), 2, "one OMP section: {again}");
        assert!(again.contains("codex capsule"), "codex section survives reinstall: {again}");
    }

    #[test]
    // trace:v1 id=test.scc-cli-plugin-omp.extension-index-uses-paths work=WORK-SCC-001 verifies=REQ-SCC-API,REQ-implement-p0-omp-integration-correctness-and-writable-benchmark-scient
    fn generated_extension_invokes_index_paths_and_does_not_treat_failure_as_success() {
        let _env = lock_scc_bin(None);
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        cmd_setup_omp(&root).unwrap();
        let ts = installed_extension(&root);
        assert!(
            ts.contains("\"--paths\"") || ts.contains("[\"index\", \"--paths\""),
            "generated extension must invoke `scc index --paths` (plural): {ts}"
        );
        assert!(
            !ts.contains("\"--path\""),
            "must not pass the rejected singular --path flag: {ts}"
        );
        assert!(
            ts.contains("index --paths failed") || ts.contains("did not succeed"),
            "failed index must be reported, not treated as success: {ts}"
        );
        assert!(
            ts.contains("session.compacting") && ts.contains("session_before_compact"),
            "compaction must save on session_before_compact and inject on session.compacting: {ts}"
        );
        assert!(
            ts.contains("checkpoint") && ts.contains("load") && ts.contains("--inject"),
            "compacting must load the checkpoint with --inject: {ts}"
        );
        assert!(
            ts.contains("process.env.SCC_BIN") || ts.contains("SCC_BIN"),
            "extension must honor SCC_BIN: {ts}"
        );
        assert!(
            ts.contains("session_switch") && ts.contains("session_branch") && ts.contains("session_tree"),
            "injection marker must reset on switch/branch/tree: {ts}"
        );
        assert!(
            ts.contains("hash-object") || ts.contains("porcelain"),
            "opaque mutations must snapshot dirty files: {ts}"
        );
        assert!(
            ts.contains("--porcelain=v1") && ts.contains("-z"),
            "porcelain must be NUL-delimited (-z): {ts}"
        );
        assert!(
            ts.contains("\"--name-only\"") || ts.contains("diff\", \"--name-only\""),
            "HEAD revision diffs must use --name-only: {ts}"
        );
        assert!(
            ts.contains("split(\"\\0\")") || ts.contains("split('\\0')") || ts.contains(".split(\"\\0\")"),
            "revision diffs must split on NUL, not newlines: {ts}"
        );
        assert!(
            ts.contains("rev-parse") && ts.contains("HEAD"),
            "snapshots must record HEAD so clean-to-clean mutations refresh: {ts}"
        );
        assert!(
            !ts.contains("toolName}:${Date.now()") && !ts.contains("toolName}:${Date.now()}"),
            "dirtySnapshots must not use a Date.now() fallback id: {ts}"
        );
        assert!(
            ts.contains("if (!id) return"),
            "snapshots without toolCallId must be skipped: {ts}"
        );
        assert!(
            ts.contains("startupOk"),
            "compaction must track startup injection separately from checkpoint: {ts}"
        );
    }

    #[test]
    // trace:v1 id=test.scc-cli-plugin-omp.scc-bin-honored-in-mcp work=WORK-SCC-001 verifies=REQ-SCC-API,REQ-implement-p0-omp-integration-correctness-and-writable-benchmark-scient
    fn mcp_json_honors_scc_bin_at_setup() {
        let _env = lock_scc_bin(Some("/opt/custom/scc"));
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        cmd_setup_omp(&root).unwrap();
        let mcp: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(".omp/mcp.json")).unwrap()).unwrap();
        assert_eq!(mcp["mcpServers"]["scc"]["command"], "/opt/custom/scc");
        let ts = installed_extension(&root);
        assert!(ts.contains("process.env.SCC_BIN"), "{ts}");
    }

    #[test]
    // trace:v1 id=test.scc-cli-plugin-omp.setup-says-restart-then-verify work=WORK-SCC-001 verifies=REQ-SCC-API
    fn setup_instructions_do_not_claim_extensions_reloads() {
        // The installer prints to stdout; capture by checking the source
        // contract the user sees after setup (the function's println!s).
        let src = include_str!("plugin_omp.rs");
        assert!(
            src.contains("Restart OMP, then run `/extensions` to verify"),
            "instructions must say restart, then /extensions to verify"
        );
        assert!(
            src.contains("`/extensions` is an inspector, not a reload"),
            "must say /extensions is an inspector"
        );
        // The user-facing installer must not tell operators that /extensions reloads.
        let installer = src.split("#[cfg(test)]").next().unwrap();
        assert!(
            !installer.contains("or run `/extensions` to reload"),
            "/extensions is an inspector, not a reload"
        );
    }
}
