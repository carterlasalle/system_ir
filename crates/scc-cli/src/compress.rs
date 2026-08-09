//! Pluggable constrained compression (docs/CONTEXT_COMPILER.md §7, SCC-072)
//! and cross-harness capsule export (docs/G7).

use std::io::Read;
use std::io::Write;
use std::path::Path;

/// `scc context compress <goal> [--cmd <cmd>] [--budget N] [--json]`
///
/// The pack builder already applies the structural compression ladder
/// (dedup, component collapse, evidence compression, priority-preserving
/// truncation). `--cmd` additionally pipes the markdown through an external
/// summarizer; its output is treated as UNTRUSTED inference and labeled.
pub fn cmd_context_compress_json(
    root: &Path,
    goal: &str,
    cmd: Option<String>,
    budget: Option<usize>,
) -> crate::Result<String> {
    cmd_context_compress_json_claims(root, goal, cmd, budget, false)
}

/// Constrained generation seam (SCC-072, docs §71): with `claims=true` the
/// external summarizer receives structured pack JSON (goal, content,
/// entity_ids, evidence_summary) and MUST emit typed claims referencing
/// known evidence; claims without valid evidence references are rejected —
/// the LLM may label evidence, it may not invent it.
pub fn cmd_context_compress_json_claims(
    root: &Path,
    goal: &str,
    cmd: Option<String>,
    budget: Option<usize>,
    claims: bool,
) -> crate::Result<String> {
    let pack_json = crate::commands::cmd_context_task_json(root, goal, &[], &[], budget)?;
    let mut pack: scc_context::ContextPack = serde_json::from_str(&pack_json)?;

    if let Some(command) = cmd {
        if claims {
            return compress_with_claims(&command, &mut pack);
        }
        let input = pack.content.clone();
        let out = run_external(&command, &input)?;
        let max = input.len().saturating_mul(2).max(4096);
        if out.len() > max {
            return Err(crate::CliError::Other(format!(
                "external summarizer produced {} bytes (limit {max})",
                out.len()
            )));
        }
        let cmd_hint: String = command.chars().take(80).collect();
        pack.content = format!(
            "<!-- compressed by external summarizer: {cmd_hint} — treat as INFERRED, not verified facts -->\n\n{out}"
        );
        pack.kind = "task-compressed".into();
        pack.compression_policy = Some(serde_json::json!({
            "external": cmd_hint,
            "note": "output is untrusted inference; repository facts remain authoritative"
        }));
    }

    Ok(serde_json::to_string_pretty(&pack)?)
}

fn compress_with_claims(command: &str, pack: &mut scc_context::ContextPack) -> crate::Result<String> {
    let input = serde_json::json!({
        "goal": pack.content.split('\n').next().unwrap_or("").to_string(),
        "content": pack.content,
        "entity_ids": pack.entity_ids,
        "evidence_summary": pack.evidence_summary,
    });
    let input_str = input.to_string();
    let out = run_external(command, &input_str)?;
    let max = input_str.len().saturating_mul(2).max(4096);
    if out.len() > max {
        return Err(crate::CliError::Other(format!(
            "external summarizer produced {} bytes (limit {max})",
            out.len()
        )));
    }
    let parsed: serde_json::Value = serde_json::from_str(&out)
        .map_err(|e| crate::CliError::Other(format!("summarizer output is not JSON: {e}")))?;
    let claims_arr = parsed
        .get("claims")
        .or_else(|| parsed.as_array().map(|_| &parsed))
        .and_then(|v| v.as_array())
        .ok_or_else(|| crate::CliError::Other("summarizer must emit {\"claims\": [...]} or an array".into()))?;

    let known: std::collections::BTreeSet<String> = pack
        .entity_ids
        .iter()
        .cloned()
        .chain(pack.evidence_summary.keys().cloned())
        .collect();
    let mut rendered = String::new();
    let mut rejected: Vec<String> = Vec::new();
    for (i, c) in claims_arr.iter().enumerate() {
        let claim = c.get("claim").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let ev: Vec<String> = c
            .get("evidence")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        if claim.is_empty() {
            rejected.push(format!("claim #{i}: empty claim text"));
            continue;
        }
        let bad: Vec<&String> = ev.iter().filter(|e| !known.contains(*e)).collect();
        if !bad.is_empty() {
            rejected.push(format!("claim #{i} ({claim:?}): unknown evidence {bad:?}"));
            continue;
        }
        let ev_disp = if ev.is_empty() {
            "none (unverified)".to_string()
        } else {
            ev.join(", ")
        };
        rendered.push_str(&format!("- {claim} [evidence: {ev_disp}]\n"));
    }
    if !rejected.is_empty() {
        return Err(crate::CliError::Other(format!(
            "{} claim(s) rejected — the summarizer may label evidence but not invent it:\n{}",
            rejected.len(),
            rejected.join("\n")
        )));
    }
    pack.content = format!(
        "<!-- claims compressed by external summarizer: {} — INFERRED, not verified facts -->\n\n{}\n## EXTERNAL CLAIMS (INFERRED)\n{}",
        command.chars().take(80).collect::<String>(),
        pack.content,
        rendered
    );
    pack.kind = "task-claims".into();
    Ok(serde_json::to_string_pretty(pack)?)
}

pub fn cmd_context_compress(
    root: &Path,
    goal: &str,
    cmd: Option<String>,
    budget: Option<usize>,
    json: bool,
) -> crate::Result<()> {
    let out = cmd_context_compress_json(root, goal, cmd, budget)?;
    if json {
        println!("{out}");
    } else {
        let pack: scc_context::ContextPack = serde_json::from_str(&out)?;
        print!("{}", pack.content);
    }
    Ok(())
}

fn run_external(command: &str, input: &str) -> crate::Result<String> {
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| crate::CliError::Other(format!("cannot spawn summarizer: {e}")))?;
    child
        .stdin
        .take()
        .ok_or_else(|| crate::CliError::Other("summarizer stdin unavailable".into()))?
        .write_all(input.as_bytes())
        .map_err(|e| crate::CliError::Other(format!("write to summarizer: {e}")))?;
    let mut out = String::new();
    child
        .stdout
        .take()
        .ok_or_else(|| crate::CliError::Other("summarizer stdout unavailable".into()))?
        .read_to_string(&mut out)
        .map_err(|e| crate::CliError::Other(format!("read from summarizer: {e}")))?;
    let status = child
        .wait()
        .map_err(|e| crate::CliError::Other(format!("summarizer wait: {e}")))?;
    if !status.success() {
        return Err(crate::CliError::Other(format!(
            "summarizer exited with {status}"
        )));
    }
    Ok(out)
}

/// `scc export capsule.md` — portable startup capsule for any harness
/// (Claude Code, Codex, Hermes, OpenCode...).
pub fn capsule_markdown(root: &Path) -> crate::Result<String> {
    let store = crate::open_store(root)?;
    let config = crate::load_config(root)?;
    let stale = crate::stale_paths(&store)?;
    let comp = crate::compiler(&store, &config, stale)?;
    let overview = comp.ctx().system_overview();
    let revision = overview.repository_revision.clone();
    let repo = store.repository();
    Ok(format!(
        "<!-- SCC-CAPSULE v1 repo={} revision={} generated={} -->\n# SYSTEM CAPSULE\n\n{}\n",
        repo.id,
        revision,
        scc_core::now_rfc3339(),
        overview.content
    ))
}

/// `scc setup codex` — write AGENTS.md with the capsule + usage rules
/// (docs/API_AND_INTEGRATIONS.md §5 for the Codex harness).
pub fn cmd_setup_codex(root: &Path) -> crate::Result<()> {
    let capsule = capsule_markdown(root)?;
    let path = root.join("AGENTS.md");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let user_part = if existing.is_empty() {
        String::new()
    } else {
        // strip a previous SCC section, keep everything BEFORE it (user content)
        match existing.find("<!-- SCC-SECTION") {
            Some(idx) => existing[..idx].trim_end().to_string(),
            None => existing.clone(),
        }
    };

    let section = format!(
        "<!-- SCC-SECTION -->\n{capsule}\n## SCC usage rules\n\
         - The repository is indexed by SCC. For a task, run: `scc context task \"<goal>\"` and work within it.\n\
         - `scc verify` reports freshness and drift — do not trust stale facts; re-index with `scc index` first.\n\
         - Authority ordering: repository/runtime > System IR > task state > memory > model assumption.\n\
         - Drift and invariants: `scc drift`, `scc ci check`, and `scc impact <files>` before cross-layer edits.\n\
         <!-- /SCC-SECTION -->\n"
    );

    let mut out = String::new();
    if !user_part.trim().is_empty() {
        out.push_str(user_part.trim_start());
        out.push_str("\n\n");
    }
    out.push_str(&section);
    std::fs::write(&path, out)?;
    println!("wrote {}", path.display());
    println!("AGENTS.md now carries the system capsule; normal Codex sessions start with system understanding.");
    Ok(())
}

/// `scc setup opencode` (M10): AGENTS.md already carries the capsule
/// (Codex/OpenCode/Hermes all read AGENTS.md); additionally write
/// .opencode/opencode.json wiring the SCC MCP server so OpenCode sessions
/// get the six semantic tools.
pub fn cmd_setup_opencode(root: &Path) -> crate::Result<()> {
    if !root.join("AGENTS.md").exists() {
        cmd_setup_codex(root)?;
    }
    let dir = root.join(".opencode");
    std::fs::create_dir_all(&dir)?;
    let config = dir.join("opencode.json");
    let existing = std::fs::read_to_string(&config).unwrap_or_else(|_| "{}".to_string());
    let mut v: serde_json::Value = serde_json::from_str(&existing).unwrap_or(serde_json::json!({}));
    let mcp = v.get_mut("mcp").and_then(|m| m.as_object_mut()).cloned().unwrap_or_default();
    let mut mcp = mcp;
    mcp.insert(
        "scc".to_string(),
        serde_json::json!({"type": "stdio", "command": "scc", "args": ["mcp"]}),
    );
    v["mcp"] = serde_json::Value::Object(mcp);
    std::fs::write(&config, serde_json::to_string_pretty(&v)?)?;
    println!("wrote {}", config.display());
    println!("OpenCode sessions will auto-connect the SCC MCP server (six semantic tools).");
    println!("Hermes and other harnesses read AGENTS.md (system capsule) — no per-harness config needed.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_summarizer_runs_and_is_labeled() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.py"), "def helper():\n    pass\n").unwrap();
        crate::commands::cmd_index(&root, true).unwrap();
        let out = cmd_context_compress_json(&root, "helper", Some("tr -d '#'".into()), None).unwrap();
        assert!(out.contains("compressed by external summarizer"), "{out}");
        assert!(out.contains("INFERRED"), "{out}");
        assert!(out.contains("TASK"), "{out}");
    }

    #[test]
    fn claims_mode_rejects_invented_evidence() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.py"), "def helper():\n    return 1\n").unwrap();
        crate::commands::cmd_index(&root, true).unwrap();
        let fake = "python3 -c 'import json,sys; print(json.dumps({\"claims\":[{\"claim\":\"x\",\"evidence\":[\"evidence:nope\"]}]}))'";
        let err = cmd_context_compress_json_claims(&root, "helper", Some(fake.into()), None, true)
            .err()
            .expect("invented evidence must be rejected");
        assert!(err.to_string().contains("rejected"), "{err}");
        assert!(err.to_string().contains("evidence:nope"), "{err}");
    }

    #[test]
    fn capsule_export_has_header() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.py"), "def helper():\n    pass\n").unwrap();
        crate::commands::cmd_index(&root, true).unwrap();
        let md = capsule_markdown(&root).unwrap();
        assert!(md.starts_with("<!-- SCC-CAPSULE v1"), "{md}");
        assert!(md.contains("# SYSTEM CAPSULE"), "{md}");
        assert!(md.contains("IDENTITY"), "{md}");
    }

    #[test]
    fn codex_setup_idempotent_with_user_content() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.py"), "def helper():\n    pass\n").unwrap();
        crate::commands::cmd_index(&root, true).unwrap();
        std::fs::write(root.join("AGENTS.md"), "# user content\nkeep me\n").unwrap();
        cmd_setup_codex(&root).unwrap();
        cmd_setup_codex(&root).unwrap();
        let text = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
        assert!(text.contains("keep me"), "user content preserved");
        assert!(text.contains("SCC-SECTION"), "{text}");
        assert_eq!(text.matches("SCC-SECTION").count(), 2, "one SCC section only");
        assert!(text.contains("scc context task"), "{text}");
    }
}
