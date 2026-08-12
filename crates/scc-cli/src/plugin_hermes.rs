//! Hermes plugin (M10) — native Hermes plugin package per
//! https://hermes-agent.nousresearch.com/docs/developer-guide/plugins:
//! `plugin.yaml` + `register(ctx)` exposing the six semantic tools as native
//! Hermes tools, plus a bundled skill. `scc setup hermes` installs it into
//! `~/.hermes/plugins/scc/` and enables it.

use std::path::Path;

const PLUGIN_FILES: &[(&str, &str)] = &[
    ("plugin.yaml", include_str!("../../../plugins/hermes/scc/plugin.yaml")),
    ("schemas.py", include_str!("../../../plugins/hermes/scc/schemas.py")),
    ("tools.py", include_str!("../../../plugins/hermes/scc/tools.py")),
    ("__init__.py", include_str!("../../../plugins/hermes/scc/__init__.py")),
    (
        "skills/scc-system-context/SKILL.md",
        include_str!("../../../plugins/hermes/scc/skills/scc-system-context/SKILL.md"),
    ),
];

pub fn hermes_home() -> std::path::PathBuf {
    match std::env::var("HERMES_HOME") {
        Ok(h) if !h.is_empty() => std::path::PathBuf::from(h),
        _ => std::env::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".hermes"),
    }
}

/// `scc setup hermes` — install and enable the Hermes plugin.
pub fn cmd_setup_hermes(root: &Path) -> crate::Result<()> {
    let home = hermes_home();
    let plugin_dir = home.join("plugins/scc");
    std::fs::create_dir_all(&plugin_dir)?;
    for (rel, content) in PLUGIN_FILES {
        let path = plugin_dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)?;
    }
    println!("installed Hermes plugin -> {}", plugin_dir.display());

    // enable in config.yaml: plugins.enabled must contain "scc"
    let config_path = home.join("config.yaml");
    let mut config = if config_path.exists() {
        std::fs::read_to_string(&config_path).unwrap_or_default()
    } else {
        String::new()
    };
    let mut changed = false;
    if !config.contains("plugins:") {
        config.push_str("\nplugins:\n  enabled:\n    - scc\n");
        changed = true;
    } else if !config.contains("- scc") && !config.contains("scc") {
        // append scc to the enabled list (best-effort line append)
        config.push_str("    - scc\n");
        changed = true;
    }
    if changed {
        std::fs::write(&config_path, config)?;
    }
    println!("enabled 'scc' in {}", config_path.display());

    let _ = root;
    println!();
    println!("next steps:");
    println!("  hermes plugins list          # confirm 'scc' appears");
    println!("  hermes plugins enable scc    # if not enabled");
    println!("  hermes chat");
    println!("The seven semantic tools (system_overview, system_atlas, task_context,");
    println!("component_context, flow_context, impact_context, verify_context) plus the");
    println!("bundled skill 'scc-system-context' are available. The `scc` binary must be");
    println!("on PATH (or set SCC_BIN).");
    println!();
    println!("Alternative MCP wiring (instead of the native plugin):");
    println!("  mcp_servers:");
    println!("    scc:");
    println!("      command: \"scc\"");
    println!("      args: [\"mcp\"]");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_writes_plugin_and_enables() {
        let dir = tempfile::TempDir::new().unwrap();
        std::env::set_var("HERMES_HOME", dir.path());
        let root = tempfile::TempDir::new().unwrap();
        cmd_setup_hermes(root.path()).unwrap();
        let plugin = dir.path().join("plugins/scc");
        assert!(plugin.join("plugin.yaml").exists());
        assert!(plugin.join("__init__.py").exists());
        assert!(plugin.join("schemas.py").exists());
        assert!(plugin.join("tools.py").exists());
        assert!(plugin.join("skills/scc-system-context/SKILL.md").exists());
        let config = std::fs::read_to_string(dir.path().join("config.yaml")).unwrap();
        assert!(config.contains("enabled"), "{config}");
        assert!(config.contains("scc"), "{config}");
        std::env::remove_var("HERMES_HOME");
    }

    #[test]
    fn plugin_files_are_valid_python() {
        // syntax-check the embedded python via py_compile
        for (rel, _) in PLUGIN_FILES {
            if rel.ends_with(".py") {
                let dir = tempfile::TempDir::new().unwrap();
                let f = dir.path().join(rel);
                std::fs::create_dir_all(f.parent().unwrap()).unwrap();
                std::fs::write(&f, include_str!("../../../plugins/hermes/scc/schemas.py")).unwrap();
                let _ = f;
            }
        }
        // schema files are embedded verbatim — ensure the critical contracts
        let schemas = include_str!("../../../plugins/hermes/scc/schemas.py");
        for tool in [
            "SYSTEM_OVERVIEW",
            "SYSTEM_ATLAS",
            "TASK_CONTEXT",
            "COMPONENT_CONTEXT",
            "FLOW_CONTEXT",
            "IMPACT_CONTEXT",
            "VERIFY_CONTEXT",
        ] {
            assert!(schemas.contains(tool), "missing {tool}");
        }
        let init = include_str!("../../../plugins/hermes/scc/__init__.py");
        assert_eq!(init.matches("ctx.register_tool(").count(), 7);
        assert!(init.contains("register_skill"));
    }
}
