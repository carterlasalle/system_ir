//! Git co-change signal (SCC-046): files that are frequently changed in the
//! same commit are a structural coupling signal. `cochange_pairs` extracts
//! the signal from `git log --name-only`; `enrich_components` annotates store
//! components with a `cochange` attribute.
//!
//! Design notes:
//! - Determinism is the contract: pairs are canonicalized to lexicographic
//!   order and the result is sorted by (commits desc, a, b).
//! - `.scc/` state files and lockfiles never participate in pairing.
//! - Co-change is an enrichment signal only: `enrich_components` adds the
//!   `cochange` attribute without ever changing component assignment.
//!   Merging components based on co-change is a future tuning step.

use scc_store::Store;
use serde_json::json;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::process::Command;

/// One co-changed file pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CochangePair {
    /// Lexicographically smaller file (repo-relative).
    pub a: String,
    /// Lexicographically larger file (repo-relative).
    pub b: String,
    /// Number of commits that touched both files.
    pub commits: u32,
}

/// Extract co-change pairs from the git history of `root` via
/// `git log --name-only --pretty=format:%H`. Pairs with fewer than
/// `min_commits` shared commits are filtered out. Not a git repository (or
/// git unavailable) yields `Ok(empty)`. Deterministic: sorted by commits
/// descending, then `a`, then `b`.
pub fn cochange_pairs(root: &Path, min_commits: u32) -> Result<Vec<CochangePair>, String> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let out = Command::new("git")
        .args(["log", "--name-only", "--pretty=format:%H"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("git log failed: {e}"))?;
    if !out.status.success() {
        // not a git repository (or git unavailable) — no signal
        return Ok(Vec::new());
    }
    let text = String::from_utf8_lossy(&out.stdout);

    let mut files: HashSet<String> = HashSet::new();
    let mut counts: BTreeMap<(String, String), u32> = BTreeMap::new();
    let mut in_commit = false;
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() {
            // blank line = commit separator: close the current commit
            if in_commit {
                record_commit(&files, &mut counts);
                files.clear();
                in_commit = false;
            }
            continue;
        }
        if is_commit_hash(line) && !in_commit {
            in_commit = true;
            continue;
        }
        if in_commit && !is_skip_path(line) {
            files.insert(line.to_string());
        }
    }
    if in_commit {
        record_commit(&files, &mut counts);
    }

    let mut pairs: Vec<CochangePair> = counts
        .into_iter()
        .filter(|((_, _), n)| *n >= min_commits)
        .map(|((a, b), n)| CochangePair { a, b, commits: n })
        .collect();
    pairs.sort_by(|x, y| {
        y.commits
            .cmp(&x.commits)
            .then_with(|| x.a.cmp(&y.a))
            .then_with(|| x.b.cmp(&y.b))
    });
    Ok(pairs)
}

/// Annotate each store component whose implementation paths contain BOTH
/// sides of a co-change pair with a `cochange` attribute:
/// `{pairs: ["a <-> b ×N", ...], top: N}` (top 5 pairs by commit count).
/// The synthetic `root` component matches top-level files (no `/`),
/// mirroring `components::component_for_path`.
///
/// Co-change is an enrichment signal only: component assignment (the
/// `implementation` attribute) is never modified; merging components based
/// on co-change is a future tuning step. Returns the number of components
/// annotated.
pub fn enrich_components(store: &Store, pairs: &[CochangePair]) -> Result<usize, String> {
    let mut comps = store.components().map_err(|e| e.to_string())?;
    let mut changed = 0usize;
    for comp in &mut comps {
        let paths: Vec<&str> = comp
            .attributes
            .get("implementation")
            .and_then(|v| v.get("paths"))
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.iter().map(|p| p.as_str()).collect::<Option<Vec<&str>>>())
            .unwrap_or_default();
        let mut matched: Vec<&CochangePair> = pairs
            .iter()
            .filter(|p| file_in_paths(&p.a, &paths) && file_in_paths(&p.b, &paths))
            .collect();
        if matched.is_empty() {
            continue;
        }
        matched.sort_by(|x, y| {
            y.commits
                .cmp(&x.commits)
                .then_with(|| x.a.cmp(&y.a))
                .then_with(|| x.b.cmp(&y.b))
        });
        let top = matched[0].commits;
        let rendered: Vec<serde_json::Value> = matched
            .iter()
            .take(5)
            .map(|p| json!(format!("{} <-> {} ×{}", p.a, p.b, p.commits)))
            .collect();
        comp.attributes
            .insert("cochange".to_string(), json!({"pairs": rendered, "top": top}));
        changed += 1;
    }
    if changed > 0 {
        store.replace_components(&comps).map_err(|e| e.to_string())?;
    }
    Ok(changed)
}

/// Count every unordered pair of distinct files changed together in one
/// commit. Pairs are canonicalized to lexicographic order so (a, b) and
/// (b, a) collapse into a single counter.
fn record_commit(files: &HashSet<String>, counts: &mut BTreeMap<(String, String), u32>) {
    let mut list: Vec<&String> = files.iter().collect();
    list.sort();
    for (i, a) in list.iter().enumerate() {
        for b in list.iter().skip(i + 1) {
            *counts.entry(((*a).clone(), (*b).clone())).or_insert(0) += 1;
        }
    }
}

/// A commit header line from `--pretty=format:%H`: 40 hex chars (SHA-1) or
/// 64 (SHA-256).
fn is_commit_hash(line: &str) -> bool {
    let n = line.len();
    (n == 40 || n == 64) && line.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `.scc/` state files and lockfiles never participate in co-change pairing.
fn is_skip_path(path: &str) -> bool {
    if path.starts_with(".scc/") || path == ".scc" {
        return true;
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    let low = name.to_ascii_lowercase();
    low == "go.sum"
        || low == "npm-shrinkwrap.json"
        || low.ends_with(".lock")
        || low.contains("-lock.")
        || low.contains("_lock.")
}

/// True when `file` lives under one of the component's path prefixes. The
/// synthetic `root` component matches top-level files (no `/`), mirroring
/// `components::component_for_path`.
fn file_in_paths(file: &str, paths: &[&str]) -> bool {
    for p in paths {
        let p = p.trim_end_matches('/');
        if p.is_empty() {
            continue;
        }
        if p == "root" {
            if !file.contains('/') {
                return true;
            }
            continue;
        }
        if file == p || file.starts_with(&format!("{p}/")) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use scc_core::kinds;
    use std::process::Command;

    fn store_for() -> (Store, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&tmp.path().join("scc.db"), &root).unwrap();
        (store, tmp)
    }

    fn git_init(dir: &Path) {
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "SCC Test"],
        ] {
            let out = Command::new("git").args(&args).current_dir(dir).output().unwrap();
            assert!(out.status.success(), "git {args:?} failed");
        }
    }

    fn commit_all(dir: &Path, msg: &str) {
        let out = Command::new("git")
            .args(["add", "-A"])
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(out.status.success());
        let out = Command::new("git")
            .args(["commit", "-q", "-m", msg])
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "commit failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn write(dir: &Path, name: &str, content: &str) {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    #[test]
    fn git_repo_pairs() {
        let tmp = tempfile::TempDir::new().unwrap();
        git_init(tmp.path());
        write(tmp.path(), "src/a.py", "a = 1\n");
        write(tmp.path(), "src/b.py", "b = 2\n");
        commit_all(tmp.path(), "c1");
        write(tmp.path(), "src/a.py", "a = 2\n");
        write(tmp.path(), "src/b.py", "b = 3\n");
        commit_all(tmp.path(), "c2");
        // third commit touches only a.py — pair stays at 2 commits
        write(tmp.path(), "src/a.py", "a = 3\n");
        commit_all(tmp.path(), "c3");

        let pairs = cochange_pairs(tmp.path(), 2).unwrap();
        assert_eq!(pairs.len(), 1, "pairs: {pairs:?}");
        let p = &pairs[0];
        assert_eq!(p.a, "src/a.py");
        assert_eq!(p.b, "src/b.py");
        assert_eq!(p.commits, 2);

        // raising the threshold filters the pair out
        assert!(cochange_pairs(tmp.path(), 3).unwrap().is_empty());
        // min_commits = 1 includes it
        let pairs1 = cochange_pairs(tmp.path(), 1).unwrap();
        assert_eq!(pairs1.len(), 1);
        assert_eq!(pairs1[0].commits, 2);
    }

    #[test]
    fn skip_state_and_lockfiles() {
        let tmp = tempfile::TempDir::new().unwrap();
        git_init(tmp.path());
        write(tmp.path(), "src/x.py", "x = 1\n");
        write(tmp.path(), "Cargo.lock", "lock\n");
        write(tmp.path(), ".scc/config.yaml", "c\n");
        commit_all(tmp.path(), "c1");
        write(tmp.path(), "src/x.py", "x = 2\n");
        write(tmp.path(), "Cargo.lock", "lock2\n");
        write(tmp.path(), ".scc/config.yaml", "c2\n");
        commit_all(tmp.path(), "c2");
        // only x.py participates; no pair forms
        assert!(cochange_pairs(tmp.path(), 1).unwrap().is_empty());
    }

    #[test]
    fn no_git_dir_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(cochange_pairs(tmp.path(), 1).unwrap().is_empty());
        // nonexistent dir
        assert!(cochange_pairs(&tmp.path().join("nope"), 1).unwrap().is_empty());
    }

    #[test]
    fn deterministic_sort() {
        let tmp = tempfile::TempDir::new().unwrap();
        git_init(tmp.path());
        // a+b together in 3 commits; a+c together in 1; b+c together in 1
        for i in 0..3 {
            write(tmp.path(), "src/a.py", &format!("a = {i}\n"));
            write(tmp.path(), "src/b.py", &format!("b = {i}\n"));
            commit_all(tmp.path(), &format!("ab{i}"));
        }
        write(tmp.path(), "src/a.py", "a = 9\n");
        write(tmp.path(), "src/c.py", "c = 9\n");
        commit_all(tmp.path(), "ac");
        write(tmp.path(), "src/b.py", "b = 9\n");
        write(tmp.path(), "src/c.py", "c = 10\n");
        commit_all(tmp.path(), "bc");

        let pairs = cochange_pairs(tmp.path(), 1).unwrap();
        // a<->b first (3 commits), then a<->c and b<->c tied at 1 (a asc)
        assert_eq!(pairs.len(), 3);
        assert_eq!((pairs[0].a.as_str(), pairs[0].b.as_str(), pairs[0].commits), ("src/a.py", "src/b.py", 3));
        assert_eq!((pairs[1].a.as_str(), pairs[1].b.as_str()), ("src/a.py", "src/c.py"));
        assert_eq!((pairs[2].a.as_str(), pairs[2].b.as_str()), ("src/b.py", "src/c.py"));

        // deterministic across repeated calls
        assert_eq!(cochange_pairs(tmp.path(), 1).unwrap(), pairs);
    }

    #[test]
    fn enrich_adds_cochange_attribute() {
        let (store, _t) = store_for();
        let mut api = scc_core::Entity::new(
            scc_core::entity_id(&store.repo_id, kinds::COMPONENT, "api"),
            kinds::COMPONENT,
            "api",
        );
        api.attr("implementation", json!({"paths": ["src/api"], "symbols": []}));
        let mut root = scc_core::Entity::new(
            scc_core::entity_id(&store.repo_id, kinds::COMPONENT, "root"),
            kinds::COMPONENT,
            "root",
        );
        root.attr("implementation", json!({"paths": ["root"], "symbols": []}));
        store.replace_components(&[api, root]).unwrap();

        let pairs = vec![
            CochangePair { a: "src/api/routes.py".into(), b: "src/api/handlers.py".into(), commits: 3 },
            CochangePair { a: "src/api/routes.py".into(), b: "src/web/app.ts".into(), commits: 2 },
            CochangePair { a: "top.py".into(), b: "README.md".into(), commits: 4 },
        ];
        let n = enrich_components(&store, &pairs).unwrap();
        assert_eq!(n, 2, "api + root annotated");

        let comps = store.components().unwrap();
        let api = comps.iter().find(|c| c.name == "api").unwrap();
        let cc = api.attributes["cochange"].clone();
        assert_eq!(cc["top"], 3);
        let rendered: Vec<&str> = cc["pairs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(rendered, ["src/api/routes.py <-> src/api/handlers.py ×3"]);
        // implementation untouched: assignment is never changed
        assert_eq!(api.attributes["implementation"]["paths"][0], "src/api");
        assert_eq!(api.attributes["implementation"]["symbols"].as_array().unwrap().len(), 0);

        let rootc = comps.iter().find(|c| c.name == "root").unwrap();
        assert_eq!(rootc.attributes["cochange"]["top"], 4);
    }

    #[test]
    fn enrich_no_match_no_change() {
        let (store, _t) = store_for();
        let mut web = scc_core::Entity::new(
            scc_core::entity_id(&store.repo_id, kinds::COMPONENT, "web"),
            kinds::COMPONENT,
            "web",
        );
        web.attr("implementation", json!({"paths": ["src/web"], "symbols": []}));
        store.replace_components(&[web]).unwrap();

        let pairs = vec![CochangePair { a: "src/api/r.py".into(), b: "src/api/h.py".into(), commits: 5 }];
        let n = enrich_components(&store, &pairs).unwrap();
        assert_eq!(n, 0);
        let comps = store.components().unwrap();
        assert!(!comps[0].attributes.contains_key("cochange"));
    }
}
