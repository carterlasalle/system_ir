//! Resolution-in-the-bench integration tests (Wave 8 §58): the atlas
//! benchmark resolves call chains through the language backends (pyright +
//! tsserver) before scoring by default, so behavior flows seed from
//! RESOLVED edges and the per-repo report carries `resolved_calls`;
//! `--no-resolve` opts out (native extraction only).

use std::path::Path;
use std::process::Command;

mod golden;

fn write_fixture(root: &Path) {
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(
        root.join("package.json"),
        "{\"name\":\"resolve-fixture\",\"version\":\"1.0.0\",\"dependencies\":{\"express\":\"^4.0.0\"}}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("tsconfig.json"),
        "{\"compilerOptions\":{\"module\":\"commonjs\",\"target\":\"es2020\",\"esModuleInterop\":true}}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("server.ts"),
        r#"import express from "express";
import { db } from "./db";

const app = express();

/** Fetch the user rows through the shared db client. */
export async function handleList(): Promise<Array<{ id: string }>> {
  const rows = await db.users.findMany();
  return rows;
}

app.get("/api/users", handleList);

export default app;
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("db.ts"),
        r#"export const db = {
  users: {
    findMany: async () => [] as Array<{ id: string }>,
  },
};
"#,
    )
    .unwrap();
}

/// Run `scc bench atlas --json` over the hermetic corpus/ground-truth dirs
/// (plus optional extra flags) and return the parsed report.
fn bench_json(root: &Path, corpus: &Path, gt: &Path, extra: &[&str]) -> serde_json::Value {
    let mut args: Vec<String> = vec![
        "bench".into(),
        "atlas".into(),
        "--json".into(),
        "--corpus".into(),
        corpus.to_str().unwrap().into(),
        "--ground-truth".into(),
        gt.to_str().unwrap().into(),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    let out = Command::new(golden::scc())
        .args(&args)
        .current_dir(root)
        .output()
        .expect("scc bench atlas runs");
    assert!(
        out.status.success(),
        "`scc bench atlas` failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("bench JSON output")
}

#[test]
fn resolve_seeds_behavior_flows_and_reports_resolved_calls() {
    // The bench degrades gracefully when a semantic backend is missing
    // (resolved_calls = 0, never fatal — documented contract). The
    // resolve-specific assertions require tsserver; skip them when it is
    // not installed rather than fail (CI installs it in the Test step).
    let tsserver_available = Command::new("typescript-language-server")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !tsserver_available {
        eprintln!("typescript-language-server not installed; skipping resolve assertions");
    }

    let tmp = tempfile::TempDir::new().unwrap();
    let fixture_src = tmp.path().join("fixture-src");
    write_fixture(&fixture_src);

    // two pristine corpus copies: one scored with the default resolve pass,
    // one with --no-resolve
    let gt = tmp.path().join("ground-truth");
    std::fs::create_dir_all(&gt).unwrap();
    std::fs::write(
        gt.join("resolve-fixture.md"),
        "## architecture\n- root\n## entrypoints\n- handleList\n## behavior\n- handleList\n## state_authority\n- s\n## contracts\n- GET /api/users\n## tests\n- t\n",
    )
    .unwrap();

    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    std::fs::create_dir_all(corpus.join("resolve-fixture")).unwrap();
    golden::copy_tree(&fixture_src, &corpus.join("resolve-fixture"));

    let corpus_nor = tmp.path().join("corpus-nor");
    std::fs::create_dir_all(&corpus_nor).unwrap();
    std::fs::create_dir_all(corpus_nor.join("resolve-fixture")).unwrap();
    golden::copy_tree(&fixture_src, &corpus_nor.join("resolve-fixture"));

    // 1. The bench (default: resolve ON) indexes, resolves the call chain
    // through tsserver, and reports resolved_calls > 0; the behavior layer
    // matches the resolved flow step op. (Skipped without tsserver: the
    // bench degrades and reports 0 — an environment property, not a bug.)
    let json = bench_json(tmp.path(), &corpus, &gt, &[]);
    assert_eq!(json["scored"].as_u64(), Some(1), "{json}");
    let repo = json["repos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["repo"] == "resolve-fixture")
        .unwrap();
    if tsserver_available {
        assert!(
            repo["resolved_calls"].as_u64().unwrap() > 0,
            "resolved_calls must be reported: {repo}"
        );
        assert!(
            repo["behavior"].as_f64().unwrap() > 0.0,
            "behavior layer must match the resolved flow step: {repo}"
        );
    } else {
        eprintln!(
            "resolve assertions skipped (no tsserver); resolved_calls={}",
            repo["resolved_calls"]
        );
    }

    // 2. The canonical flow graph for the route carries the resolved
    // chain: handleList -> db with a RESOLVED edge (the flow step ops).
    let fg = Command::new(golden::scc())
        .args(["export", "flow-graphs.json"])
        .current_dir(corpus.join("resolve-fixture"))
        .output()
        .expect("scc export flow-graphs.json runs");
    assert!(
        fg.status.success(),
        "export failed: {}",
        String::from_utf8_lossy(&fg.stderr)
    );
    let graphs: serde_json::Value = serde_json::from_slice(&fg.stdout).expect("flow-graphs JSON");
    let route = graphs
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["name"] == "get-/api/users")
        .unwrap_or_else(|| panic!("route flow missing: {graphs}"));
    let ops: Vec<&str> = route["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["operation"].as_str().unwrap())
        .collect();
    assert!(
        ops.iter().any(|o| o.ends_with("handleList")),
        "flow starts at the handler op: {ops:?}"
    );
    assert!(
        ops.iter().any(|o| o.ends_with("/db") || o.ends_with("db")),
        "flow reaches the resolved callee: {ops:?}"
    );
    let resolved_edge = route["edges"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["provenance"] == "RESOLVED");
    if tsserver_available {
        assert!(resolved_edge, "route flow has a RESOLVED edge: {route}");
    }

    // 3. `--no-resolve` opts out: zero resolved calls (native extraction).
    let json = bench_json(tmp.path(), &corpus_nor, &gt, &["--no-resolve"]);
    let repo = json["repos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["repo"] == "resolve-fixture")
        .unwrap();
    assert_eq!(
        repo["resolved_calls"].as_u64().unwrap(),
        0,
        "--no-resolve must skip the semantic backends: {repo}"
    );
}
