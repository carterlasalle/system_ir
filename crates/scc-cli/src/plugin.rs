//! Claude Code integration (EPIC-100, docs/API_AND_INTEGRATIONS.md §5).
//!
//! `scc setup claude` writes `.claude/settings.json` hooks that call the
//! embedded shell scripts. Hooks:
//! - SessionStart: inject startup capsule + verify warnings + checkpoint
//! - UserPromptSubmit: inject a task pack for repository-changing prompts
//! - PostToolUse (Edit|Write|MultiEdit|NotebookEdit): incremental refresh
//! - PreCompact: persist a task checkpoint (docs §126)
//!
//! Normal usage requires no slash command.

use std::path::Path;

const SESSION_START: &str = r#"#!/usr/bin/env bash
# SCC SessionStart: inject system capsule + freshness warnings + checkpoint.
SCC_BIN="${SCC_BIN:-scc}"
command -v "$SCC_BIN" >/dev/null 2>&1 || exit 0
if [ -f .scc/scc.db ]; then
  "$SCC_BIN" verify --warnings 2>/dev/null
  "$SCC_BIN" checkpoint load --inject 2>/dev/null
  echo ""
  "$SCC_BIN" overview 2>/dev/null
fi
"#;

const USER_PROMPT_SUBMIT: &str = r#"#!/usr/bin/env bash
# SCC UserPromptSubmit: inject a bounded task pack for coding prompts.
SCC_BIN="${SCC_BIN:-scc}"
command -v "$SCC_BIN" >/dev/null 2>&1 || exit 0
command -v python3 >/dev/null 2>&1 || exit 0
prompt=$(python3 -c 'import json,sys
try:
    print(json.load(sys.stdin).get("prompt",""))
except Exception:
    print("")' 2>/dev/null)
[ -z "$prompt" ] && exit 0
# Skip conversational, git-only, or very short prompts.
case "$prompt" in
  git*|help|hello*|hi*|thanks*|thank*|yes|no|ok|okay|"") exit 0 ;;
esac
len=${#prompt}
[ "$len" -lt 40 ] && exit 0
if [ -f .scc/scc.db ]; then
  echo "<!-- SCC TASK CONTEXT (auto-injected) -->"
  "$SCC_BIN" context task "$prompt" 2>/dev/null
fi
"#;

const POST_TOOL_USE: &str = r#"#!/usr/bin/env bash
# SCC PostToolUse: incremental refresh of changed files.
SCC_BIN="${SCC_BIN:-scc}"
command -v "$SCC_BIN" >/dev/null 2>&1 || exit 0
command -v python3 >/dev/null 2>&1 || exit 0
files=$(python3 -c 'import json,sys
try:
    d=json.load(sys.stdin)
    ti=d.get("tool_input",{}) or {}
    out=[]
    for k in ("file_path","filePaths","files"):
        v=ti.get(k)
        if isinstance(v,str): out.append(v)
        elif isinstance(v,list): out.extend(x for x in v if isinstance(x,str))
    print("\n".join(out))
except Exception:
    print("")' 2>/dev/null)
[ -z "$files" ] && exit 0
[ -f .scc/scc.db ] || exit 0
args=()
while IFS= read -r f; do
  [ -n "$f" ] && args+=("$f")
done <<< "$files"
if [ "${#args[@]}" -gt 0 ]; then
  timeout 15 "$SCC_BIN" index --paths "${args[@]}" --quiet >/dev/null 2>&1
fi
"#;

const PRE_COMPACT: &str = r#"#!/usr/bin/env bash
# SCC PreCompact: persist a checkpoint so rehydration is transparent.
SCC_BIN="${SCC_BIN:-scc}"
command -v "$SCC_BIN" >/dev/null 2>&1 || exit 0
[ -f .scc/scc.db ] || exit 0
python3 - "$SCC_BIN" <<'PYEOF'
import json, subprocess, sys
scc = sys.argv[1]
try:
    save = subprocess.run([scc, "checkpoint", "save", "--json"], capture_output=True, text=True, timeout=10)
    load = subprocess.run([scc, "checkpoint", "load", "--inject"], capture_output=True, text=True, timeout=10)
    content = load.stdout.strip() if load.returncode == 0 and load.stdout.strip() else "SCC checkpoint unavailable at compaction time."
    if not content:
        content = json.dumps(json.loads(save.stdout)) if save.returncode == 0 and save.stdout.strip() else content
except Exception:
    content = "SCC checkpoint unavailable at compaction time."
print(json.dumps({"files": {"scc-checkpoint.md": content}}))
PYEOF
"#;

pub fn install(root: &Path) -> crate::Result<()> {
    let claude_dir = root.join(".claude");
    let hook_dir = claude_dir.join("hooks/scc");
    std::fs::create_dir_all(&hook_dir)?;

    let scripts: [(&str, &str, &str); 4] = [
        ("session_start.sh", "SessionStart", SESSION_START),
        ("user_prompt_submit.sh", "UserPromptSubmit", USER_PROMPT_SUBMIT),
        ("post_tool_use.sh", "PostToolUse", POST_TOOL_USE),
        ("pre_compact.sh", "PreCompact", PRE_COMPACT),
    ];
    let mut hooks: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    for (file, event, content) in scripts {
        let script_path = hook_dir.join(file);
        std::fs::write(&script_path, content)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))?;
        }
        let command = format!("{}", script_path.display());
        let entry = serde_json::json!([{
            "matcher": if event == "PostToolUse" { "Edit|Write|MultiEdit|NotebookEdit" } else { "*" },
            "hooks": [{"type": "command", "command": command}]
        }]);
        hooks.insert(event.to_string(), entry);
    }

    // merge with existing settings.json if present
    let settings_path = claude_dir.join("settings.json");
    let mut settings: serde_json::Value = if settings_path.exists() {
        let text = std::fs::read_to_string(&settings_path)?;
        serde_json::from_str(&text).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    let existing_hooks = settings
        .get_mut("hooks")
        .and_then(|h| h.as_object_mut())
        .cloned()
        .unwrap_or_default();
    let mut merged = existing_hooks;
    for (event, entry) in hooks {
        merged.insert(event, entry);
    }
    settings["hooks"] = serde_json::Value::Object(merged);
    std::fs::write(&settings_path, serde_json::to_string_pretty(&settings)?)?;

    println!("Claude Code plugin installed:");
    println!("  hooks -> {}", settings_path.display());
    println!("  scripts -> {}", hook_dir.display());
    println!();
    println!("Make sure `scc` is on PATH (or set SCC_BIN).");
    println!("Restart Claude Code for the hooks to take effect.");
    println!("No slash command needed: startup capsule, task packs, and");
    println!("checkpoints are automatic.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_are_valid_bash() {
        for (_, _, content) in [
            ("", "", SESSION_START),
            ("", "", USER_PROMPT_SUBMIT),
            ("", "", POST_TOOL_USE),
            ("", "", PRE_COMPACT),
        ] {
            assert!(content.starts_with("#!/usr/bin/env bash"));
            assert!(content.contains("scc"));
        }
    }

    #[test]
    fn install_writes_hooks() {
        let dir = tempfile::TempDir::new().unwrap();
        install(dir.path()).unwrap();
        let settings: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join(".claude/settings.json")).unwrap(),
        )
        .unwrap();
        let hooks = settings["hooks"].as_object().unwrap();
        for event in ["SessionStart", "UserPromptSubmit", "PostToolUse", "PreCompact"] {
            assert!(hooks.contains_key(event), "missing {event}");
        }
        assert!(dir.path().join(".claude/hooks/scc/session_start.sh").exists());
    }
}
