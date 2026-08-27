//! Oh My Pi (OMP) integration (docs/API_AND_INTEGRATIONS.md §5, P0).
//!
//! `scc setup omp` installs a native project-scoped integration with NO
//! manual post-config:
//!   1. `.omp/extensions/scc/index.ts` + `package.json` — ONE native
//!      extension module that registers ALL ordering-dependent SCC
//!      lifecycle behavior (`session_start`, `before_agent_start`,
//!      `tool_result`, `session_before_compact`, `session_compact`).
//!   2. `.omp/mcp.json` — merged (preserving existing servers) wiring the
//!      SCC MCP server (`scc mcp`) for manual semantic drill-down.
//!   3. `.omp/skills/scc-system-context/SKILL.md` — on-demand workflow
//!      guidance (no hard rules live only in the skill).
//!   4. `.omp/AGENTS.md` — SCC DURABLE RULES ONLY (never generated
//!      Atlas/Surface facts). Native `.omp/AGENTS.md` has higher provider
//!      priority than the standalone root `AGENTS.md`, so SCC writes its
//!      rules there and never touches the root file (which may be the
//!      user's or `scc setup codex`'s).
//!
//! The extension uses `pi.exec("scc", [...])` argument arrays (never shell
//! interpolation), preserves arg boundaries, captures exit/stdout/stderr,
//! catches errors, and never crashes the OMP session.

use std::path::Path;

// The native extension entry: ONE module registers every ordering-dependent
// SCC lifecycle behavior (OMP does not promise filename/module ordering).
const EXTENSION_TS: &str = include_str!("../../../plugins/omp/scc/index.ts");
const EXTENSION_PACKAGE: &str = include_str!("../../../plugins/omp/scc/package.json");
const SKILL_MD: &str = include_str!("../../../plugins/omp/scc/skills/scc-system-context/SKILL.md");

// trace:exempt reason=const-data (behavior boundary is write_agents_rules)
const AGENTS_RULES: &str = "<!-- SCC-SECTION -->\n\
# SCC (System Context Compiler)\n\
This repository is indexed by SCC. Durable rules:\n\
- The native SCC extension injects the startup architecture once per session\n\
  and a task-specific context pack per prompt. Work within the injected task\n\
  context; it is the authoritative system slice for the current goal.\n\
- For the system architecture at session start, `scc atlas` is the\n\
  authoritative startup source.\n\
- `scc verify` reports freshness and drift — do not trust stale facts;\n\
  re-index with `scc index` first.\n\
- Authority ordering: source/runtime > SCC System IR > checkpoint > Hindsight\n\
  > model assumption.\n\
- Drift and invariants: `scc drift`, `scc ci check`, and `scc impact <files>`\n\
  before cross-layer edits.\n\
<!-- /SCC-SECTION -->\n";

/// `scc setup omp` — install the native OMP integration.
// trace:v1 id=impl.crates-scc-cli-src-plugin-omp.cmd-setup-omp work=WORK-SCC-001 satisfies=REQ-SCC-API
pub fn cmd_setup_omp(root: &Path) -> crate::Result<()> {
    let omp_dir = root.join(".omp");
    // 1. Native extension package.
    let ext_dir = omp_dir.join("extensions/scc");
    std::fs::create_dir_all(&ext_dir)?;
    std::fs::write(ext_dir.join("index.ts"), extension_ts())?;
    std::fs::write(ext_dir.join("package.json"), extension_package())?;
    println!("wrote {}", ext_dir.join("index.ts").display());

    // 2. Merge `.omp/mcp.json`, preserving existing servers.
    merge_mcp_json(&omp_dir)?;

    // 3. Skill (on-demand workflow guidance).
    let skill_dir = omp_dir.join("skills/scc-system-context");
    std::fs::create_dir_all(&skill_dir)?;
    std::fs::write(skill_dir.join("SKILL.md"), skill_md())?;
    println!("wrote {}", skill_dir.join("SKILL.md").display());

    // 4. Durable rules in native `.omp/AGENTS.md` (higher priority than the
    // root AGENTS.md; never touch the root file).
    write_agents_rules(&omp_dir)?;

    println!();
    println!("OMP integration installed:");
    println!("  extension  -> {}", ext_dir.join("index.ts").display());
    println!("  mcp.json   -> {}", omp_dir.join("mcp.json").display());
    println!("  skill      -> {}", skill_dir.join("SKILL.md").display());
    println!("  AGENTS.md  -> {}", omp_dir.join("AGENTS.md").display());
    println!();
    println!("Restart OMP (or run `/extensions` to reload) for the SCC extension");
    println!("to take effect. Verify with `/mcp reload` then `/mcp test scc`.");
    println!("The `scc` binary must be on PATH (or set SCC_BIN).");
    Ok(())
}

// trace:exempt reason=internal-helper
fn extension_ts() -> &'static str {
    EXTENSION_TS
}

// trace:exempt reason=internal-helper
fn extension_package() -> &'static str {
    EXTENSION_PACKAGE
}

// trace:exempt reason=internal-helper
fn skill_md() -> &'static str {
    SKILL_MD
}

/// Merge the SCC MCP server into `.omp/mcp.json`, preserving any existing
/// servers. The format is the standard MCP config: `{ "mcpServers": {
/// "<name>": { "command": ..., "args": [...] } } }`. Idempotent: if the
/// `scc` server is already present, it is left unchanged.
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
    servers.entry("scc".to_string()).or_insert_with(|| {
        serde_json::json!({"command": "scc", "args": ["mcp"]})
    });
    v["mcpServers"] = serde_json::Value::Object(servers);
    std::fs::write(&path, serde_json::to_string_pretty(&v)?)?;
    Ok(())
}

/// Write the durable SCC rules into `.omp/AGENTS.md`, preserving any user
/// content and never duplicating the SCC section (idempotent). The native
/// `.omp/AGENTS.md` takes provider priority over the root `AGENTS.md`, so
/// SCC writes its durable rules here and leaves the root file untouched.
// trace:v1 id=impl.crates-scc-cli-src-plugin-omp.write-agents-rules work=WORK-SCC-001 satisfies=REQ-SCC-API
fn write_agents_rules(omp_dir: &Path) -> crate::Result<()> {
    let path = omp_dir.join("AGENTS.md");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    // Strip a previous SCC section (everything between the markers),
    // preserving any user content BEFORE it.
    let user_part = match existing.find("<!-- SCC-SECTION") {
        Some(idx) => existing[..idx].trim_end().to_string(),
        None => existing.trim().to_string(),
    };
    let mut out = String::new();
    if !user_part.is_empty() {
        out.push_str(&user_part);
        out.push_str("\n\n");
    }
    out.push_str(AGENTS_RULES);
    std::fs::write(&path, out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // trace:v1 id=test.scc-cli-plugin-omp.installs-extension-and-mcp work=WORK-SCC-001 verifies=REQ-SCC-API
    fn installs_extension_and_mcp() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        cmd_setup_omp(&root).unwrap();

        // Extension files are present.
        assert!(root.join(".omp/extensions/scc/index.ts").exists());
        assert!(root.join(".omp/extensions/scc/package.json").exists());
        // The extension imports the resolvable package.
        let ts = std::fs::read_to_string(root.join(".omp/extensions/scc/index.ts")).unwrap();
        assert!(ts.contains("@oh-my-pi/pi-coding-agent"), "extension must import the canonical resolvable package");
        assert!(ts.contains("before_agent_start"), "extension must wire before_agent_start");
        assert!(ts.contains("session_start"), "extension must wire session_start");

        // mcp.json has the scc server.
        let mcp: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(".omp/mcp.json")).unwrap()).unwrap();
        assert_eq!(mcp["mcpServers"]["scc"]["command"], "scc");
        assert_eq!(mcp["mcpServers"]["scc"]["args"][0], "mcp");

        // AGENTS.md has the SCC rules.
        let agents = std::fs::read_to_string(root.join(".omp/AGENTS.md")).unwrap();
        assert!(agents.contains("SCC (System Context Compiler)"), "AGENTS.md must carry SCC durable rules");
    }

    #[test]
    // trace:v1 id=test.scc-cli-plugin-omp.mcp-merge-preserves-existing work=WORK-SCC-001 verifies=REQ-SCC-API
    fn mcp_merge_preserves_existing_servers() {
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
        write_agents_rules(&omp).unwrap();
        let first = std::fs::read_to_string(omp.join("AGENTS.md")).unwrap();
        assert!(first.starts_with("USER CONTENT"), "user content must be preserved");
        assert!(first.contains("SCC (System Context Compiler)"), "SCC rules must be appended");
        // Idempotent: a second run does not duplicate the SCC section.
        write_agents_rules(&omp).unwrap();
        let second = std::fs::read_to_string(omp.join("AGENTS.md")).unwrap();
        assert_eq!(
            first.matches("SCC (System Context Compiler)").count(),
            second.matches("SCC (System Context Compiler)").count(),
            "SCC rules must not duplicate on reinstall"
        );
    }
}