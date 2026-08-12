//! MCP server (docs/API_AND_INTEGRATIONS.md §2, EPIC-080).
//!
//! Exposes exactly six intent-level tools to agents — never analyzer-level
//! graph operations:
//!   system_overview, task_context, component_context, flow_context,
//!   impact_context, verify_context
//!
//! Transport: stdio, newline-delimited JSON-RPC 2.0 (MCP stdio framing).
//! Repository read-only by default (docs/SECURITY.md §10).

use std::io::{BufRead, Write};
use std::path::Path;

const PROTOCOL_VERSION: &str = "2025-06-18";

struct Tool {
    name: &'static str,
    description: &'static str,
    input_schema: serde_json::Value,
}

fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "system_overview",
            description: "Compact system overview: purpose, components, boundaries, stores, external systems, flows, invariants, freshness.",
            input_schema: serde_json::json!({"type": "object", "properties": {}}),
        },
        Tool {
            name: "system_atlas",
            description: "Full System Atlas: complete architecture for session startup (purpose, components, flows, ownership, contracts, invariants, failure paths, deployment, trust boundaries). The primary agent startup tool.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"token_budget": {"type": "integer", "description": "Optional token budget (default context.atlas_tokens)"}}
            }),
        },
        Tool {
            name: "task_context",
            description: "Task-specific system context pack for a coding goal. Primary agent operation.",
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["goal"],
                "properties": {
                    "goal": {"type": "string", "description": "The task/goal in natural language"},
                    "files": {"type": "array", "items": {"type": "string"}, "description": "Explicit file paths"},
                    "symbols": {"type": "array", "items": {"type": "string"}, "description": "Explicit symbol names"},
                    "token_budget": {"type": "integer", "minimum": 512, "description": "Hard token budget (default 8000)"}
                }
            }),
        },
        Tool {
            name: "component_context",
            description: "Component detail: responsibility, implementation, dependencies, ownership, flows, contracts, tests, evidence.",
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["component"],
                "properties": {
                    "component": {"type": "string", "description": "Component id or name"}
                }
            }),
        },
        Tool {
            name: "flow_context",
            description: "Flow detail: trigger, steps, branches, data, failures, retries, evidence.",
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["flow"],
                "properties": {
                    "flow": {"type": "string", "description": "Flow id or name"}
                }
            }),
        },
        Tool {
            name: "impact_context",
            description: "Impact of a change: affected components, flows, consumers, contracts, data, invariants, tests, risk.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "files": {"type": "array", "items": {"type": "string"}},
                    "symbols": {"type": "array", "items": {"type": "string"}},
                    "diff": {"type": "string", "description": "Git base revision for diff (e.g. origin/main)"}
                }
            }),
        },
        Tool {
            name: "verify_context",
            description: "Verification report: freshness, stale facts, conflicts, low-confidence dependencies, drift, missing evidence.",
            input_schema: serde_json::json!({"type": "object", "properties": {}}),
        },
    ]
}

fn send(msg: &serde_json::Value) {
    let mut line = serde_json::to_string(msg).unwrap_or_default();
    line.push('\n');
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = lock.write_all(line.as_bytes());
    let _ = lock.flush();
}

fn reply(id: &serde_json::Value, result: serde_json::Value) {
    send(&serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result}));
}

fn error(id: &serde_json::Value, code: i64, message: &str) {
    send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message}
    }));
}

/// Run the MCP server over stdin/stdout for `root`.
pub fn serve_stdio(root: &Path) -> crate::Result<()> {
    let stdin = std::io::stdin();
    let mut line = String::new();
    let mut lock = stdin.lock();
    loop {
        line.clear();
        let n = lock.read_line(&mut line)?;
        if n == 0 {
            break; // EOF
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(method) = msg.get("method").and_then(|m| m.as_str()) else {
            continue; // response or notification
        };
        let id = msg.get("id").cloned().unwrap_or(serde_json::Value::Null);
        let params = msg.get("params").cloned().unwrap_or(serde_json::json!({}));

        match method {
            "initialize" => {
                reply(
                    &id,
                    serde_json::json!({
                        "protocolVersion": PROTOCOL_VERSION,
                        "capabilities": {"tools": {"listChanged": false}},
                        "serverInfo": {"name": "scc", "version": env!("CARGO_PKG_VERSION")}
                    }),
                );
            }
            "notifications/initialized" | "notifications/cancelled" => {}
            "ping" => reply(&id, serde_json::json!({})),
            "tools/list" => {
                let list: Vec<serde_json::Value> = tools()
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "name": t.name,
                            "description": t.description,
                            "inputSchema": t.input_schema,
                        })
                    })
                    .collect();
                reply(&id, serde_json::json!({"tools": list}));
            }
            "tools/call" => {
                let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(serde_json::json!({}));
                match call_tool(root, name, &args) {
                    Ok(text) => reply(
                        &id,
                        serde_json::json!({"content": [{"type": "text", "text": text}]}),
                    ),
                    Err(e) => reply(
                        &id,
                        serde_json::json!({
                            "content": [{"type": "text", "text": format!("error: {e}")}],
                            "isError": true
                        }),
                    ),
                }
            }
            other => error(&id, -32601, &format!("method not found: {other}")),
        }
    }
    Ok(())
}

fn call_tool(root: &Path, name: &str, args: &serde_json::Value) -> crate::Result<String> {
    let store = crate::open_store(root)?;
    if !store.snapshot_status()?.is_some() {
        return Ok("# NOT INDEXED\nRun `scc index` before asking for system context.".to_string());
    }
    let config = crate::load_config(root)?;
    let stale = crate::stale_paths(&store)?;
    let comp = crate::compiler(&store, &config, stale)?;

    let str_arg = |k: &str| -> String {
        args.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string()
    };
    let arr_arg = |k: &str| -> Vec<String> {
        args.get(k)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    };

    match name {
        "system_overview" => Ok(comp.ctx().system_overview().content),
        "system_atlas" => {
            let budget = args.get("token_budget").and_then(|b| b.as_u64()).map(|b| b as usize);
            Ok(comp.ctx().system_atlas(budget).content)
        }
        "task_context" => {
            let goal = str_arg("goal");
            if goal.is_empty() {
                return Ok("task_context requires a `goal` string.".to_string());
            }
            let budget = args
                .get("token_budget")
                .and_then(|v| v.as_u64())
                .map(|b| b as usize);
            Ok(comp
                .ctx()
                .task_context(&goal, &arr_arg("files"), &arr_arg("symbols"), budget)
                .content)
        }
        "component_context" => {
            let id = str_arg("component");
            if id.is_empty() {
                return Ok("component_context requires a `component` id or name.".to_string());
            }
            Ok(comp.ctx().component_context(&id).content)
        }
        "flow_context" => {
            let id = str_arg("flow");
            if id.is_empty() {
                return Ok("flow_context requires a `flow` id or name.".to_string());
            }
            Ok(comp.ctx().flow_context(&id).content)
        }
        "impact_context" => {
            let diff = str_arg("diff");
            Ok(comp
                .ctx()
                .impact_context(&arr_arg("files"), &arr_arg("symbols"), if diff.is_empty() { None } else { Some(&diff) })
                .content)
        }
        "verify_context" => Ok(comp.ctx().verify_context().content),
        other => Err(crate::CliError::Other(format!("unknown tool: {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_schemas_are_valid_json_schema() {
        for t in tools() {
            assert_eq!(t.input_schema["type"], "object");
            assert!(t.input_schema.get("properties").is_some());
        }
        assert_eq!(tools().len(), 7, "the seven semantic tools only");
    }

    #[test]
    fn jsonrpc_shapes() {
        let req = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
        assert_eq!(req["method"], "tools/list");
    }
}
