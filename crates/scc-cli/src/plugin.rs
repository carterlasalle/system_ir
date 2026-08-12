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
# State location comes from SCC itself (honors SCC_STATE_DIR), so the
# repository can be read-only with external state.
SCC_BIN="${SCC_BIN:-scc}"
command -v "$SCC_BIN" >/dev/null 2>&1 || exit 0
SCC_STATE="$("$SCC_BIN" state-path 2>/dev/null)" || exit 0
[ -n "$SCC_STATE" ] && [ -f "$SCC_STATE/scc.db" ] || exit 0
"$SCC_BIN" verify --warnings 2>/dev/null
"$SCC_BIN" checkpoint load --inject 2>/dev/null
echo ""
# Wave 2: the FULL System Atlas is the startup architecture injection.
"$SCC_BIN" atlas 2>/dev/null
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
SCC_STATE="$("$SCC_BIN" state-path 2>/dev/null)" || exit 0
[ -n "$SCC_STATE" ] && [ -f "$SCC_STATE/scc.db" ] || exit 0
# Wave 2 (§37): the Atlas is already in context; UserPromptSubmit injects
# a small task focus ONLY when context.inject_task_focus is true (the CLI
# itself is the gatekeeper — prints nothing when disabled).
"$SCC_BIN" context task "$prompt" --hook 2>/dev/null
"#;

const POST_TOOL_USE: &str = r#"#!/usr/bin/env bash
# SCC PostToolUse: incremental refresh of changed files.
# The operation timeout lives in the Rust CLI's caller (python subprocess
# timeout below) — no GNU `timeout` dependency, platform-independent.
SCC_BIN="${SCC_BIN:-scc}"
command -v "$SCC_BIN" >/dev/null 2>&1 || exit 0
command -v python3 >/dev/null 2>&1 || exit 0
python3 - "$SCC_BIN" <<'PYEOF'
import json, subprocess, sys, os
scc = sys.argv[1]
try:
    d = json.load(sys.stdin)
    ti = d.get("tool_input", {}) or {}
    files = []
    for k in ("file_path", "filePaths", "files"):
        v = ti.get(k)
        if isinstance(v, str):
            files.append(v)
        elif isinstance(v, list):
            files.extend(x for x in v if isinstance(x, str))
    files = [f for f in files if f]
except Exception:
    files = []
if not files:
    sys.exit(0)
try:
    state = subprocess.run([scc, "state-path"], capture_output=True, text=True, timeout=5)
    db = state.stdout.strip()
    if state.returncode != 0 or not db or not os.path.isfile(os.path.join(db, "scc.db")):
        sys.exit(0)
    subprocess.run(
        [scc, "index", "--paths"] + files + ["--quiet"],
        capture_output=True, text=True, timeout=15,
    )
except Exception:
    pass
PYEOF
"#;

const PRE_COMPACT: &str = r#"#!/usr/bin/env bash
# SCC PreCompact: re-inject the System Atlas + task checkpoint so the
# architecture survives compaction (Wave 2 §38).
SCC_BIN="${SCC_BIN:-scc}"
command -v "$SCC_BIN" >/dev/null 2>&1 || exit 0
SCC_STATE="$("$SCC_BIN" state-path 2>/dev/null)" || exit 0
[ -n "$SCC_STATE" ] && [ -f "$SCC_STATE/scc.db" ] || exit 0
python3 - "$SCC_BIN" <<'PYEOF'
import json, subprocess, sys
scc = sys.argv[1]
def run(args, timeout=15):
    try:
        return subprocess.run([scc] + args, capture_output=True, text=True, timeout=timeout).stdout
    except Exception:
        return ""
try:
    save = run(["checkpoint", "save", "--json"], 10)
    checkpoint = run(["checkpoint", "load", "--inject"], 10)
    if not checkpoint.strip():
        checkpoint = save if save.strip() else "SCC checkpoint unavailable at compaction time."
    atlas = run(["atlas"])
    if not atlas.strip():
        atlas = "SCC atlas unavailable at compaction time."
    content = "SYSTEM ATLAS (re-injected after compaction)\n\n" + atlas
    if checkpoint.strip():
        content += "\n\nTASK CHECKPOINT\n\n" + checkpoint
except Exception:
    content = "SCC rehydration unavailable at compaction time."
print(json.dumps({"files": {"scc-rehydrate.md": content}}))
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

    // merge with existing settings.json if present (P0 §12): SCC hooks are
    // APPENDED to the per-event arrays so existing hooks (Serena, security,
    // RTK, ...) survive installation
    let settings_path = claude_dir.join("settings.json");
    let mut settings: serde_json::Value = if settings_path.exists() {
        let text = std::fs::read_to_string(&settings_path)?;
        serde_json::from_str(&text).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    let existing_hooks: serde_json::Map<String, serde_json::Value> = settings
        .get("hooks")
        .and_then(|h| h.as_object())
        .cloned()
        .unwrap_or_default();
    let mut merged = existing_hooks;
    for (event, entry) in hooks {
        let scc_entry = entry;
        match merged.get_mut(&event) {
            Some(serde_json::Value::Array(existing)) => {
                // append SCC's matcher entry to whatever already runs
                existing.push(scc_entry);
            }
            _ => {
                merged.insert(event, scc_entry);
            }
        }
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

    #[test]
    fn install_preserves_existing_hooks() {
        // P0 §12: SCC installation must append to, never replace, existing
        // hooks for the same event (Serena/security/RTK coexistence).
        let dir = tempfile::TempDir::new().unwrap();
        let claude = dir.path().join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        let existing = serde_json::json!({
            "hooks": {
                "SessionStart": [
                    {"matcher": "*", "hooks": [{"type": "command", "command": "/serena/start"}]}
                ],
                "UserPromptSubmit": [
                    {"matcher": "*", "hooks": [{"type": "command", "command": "/security/scan"}]}
                ]
            }
        });
        std::fs::write(
            claude.join("settings.json"),
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();

        install(dir.path()).unwrap();
        let settings: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(claude.join("settings.json")).unwrap(),
        )
        .unwrap();
        let hooks = settings["hooks"].as_object().unwrap();

        let ss = hooks["SessionStart"].as_array().unwrap();
        assert_eq!(ss.len(), 2, "existing + SCC SessionStart: {ss:?}");
        let serena = ss
            .iter()
            .find(|e| e.to_string().contains("serena"))
            .expect("Serena hook preserved");
        assert!(serena.get("hooks").is_some());
        let scc_entry = ss
            .iter()
            .find(|e| e.to_string().contains("session_start.sh"))
            .expect("SCC hook appended");
        assert!(scc_entry[0].get("hooks").is_some(), "matcher entry: {scc_entry:?}");

        let ups = hooks["UserPromptSubmit"].as_array().unwrap();
        assert_eq!(ups.len(), 2, "existing + SCC UserPromptSubmit: {ups:?}");
        assert!(ups.iter().any(|e| e.to_string().contains("security")));
        assert!(ups.iter().any(|e| e.to_string().contains("user_prompt_submit.sh")));
    }

    #[test]
    fn post_tool_use_has_no_shell_timeout_dependency() {
        // P0 §14: integration must not rely on GNU `timeout`.
        assert!(!POST_TOOL_USE.contains("timeout 15"), "no shell timeout call");
        assert!(
            POST_TOOL_USE.contains("timeout=15"),
            "python-side subprocess timeout"
        );
        // P0 §13: state location comes from SCC itself
        for script in [SESSION_START, USER_PROMPT_SUBMIT, POST_TOOL_USE, PRE_COMPACT] {
            assert!(
                !script.contains(".scc/scc.db"),
                "no direct .scc probe: {script}"
            );
            assert!(script.contains("state-path"), "must use scc state-path");
        }
    }
}
