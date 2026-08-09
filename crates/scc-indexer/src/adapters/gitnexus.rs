//! GitNexus evidence importer (docs §43, SCC-200): imports a documented
//! GitNexus-style export of symbols and edges into the SCC-native evidence
//! layer.
//!
//! Input contract (single-repo shape):
//! ```json
//! {
//!   "producer": "gitnexus",
//!   "version": "0.1",
//!   "symbols": [
//!     {"id": "sym:1", "name": "helper", "file": "a.py", "kind": "function",
//!      "signature": "helper(x: int) -> int"}
//!   ],
//!   "edges": [
//!     {"subject": "sym:1", "predicate": "calls", "object": "sym:2",
//!      "provenance": "RESOLVED"}
//!   ]
//! }
//! ```
//! An alternate multi-repo shape wraps the same fields under a top-level
//! `repositories` array: `[{"repository": "name", "symbols": [...],
//! "edges": [...]}]`; each block is imported against its own repository id,
//! and a repository name appearing twice keeps the first block (later
//! duplicates are skipped silently).
//!
//! The importer is defensive: malformed entries are skipped and counted in
//! [`GitnexusReport::errors`]; only unreadable files or top-level JSON that
//! is not a JSON object return `Err`. All ids are content-derived and
//! deterministic (same input, same ids) using the same blake3 schemes as
//! `crate::write`.

use super::{get_str, make_evidence};
use crate::write::{evidence_id, rel_id};
use scc_core::kinds;
use scc_core::{
    symbol_id, Entity, Evidence, EvidenceType, Provenance, Relationship,
};
use scc_store::Store;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Aggregate result of one GitNexus import run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitnexusReport {
    pub symbols: usize,
    pub edges: usize,
    pub errors: usize,
}

/// Predicates accepted on edges, restricted to the documented ontology.
const ONTOLOGY: &[&str] = &[
    scc_core::predicates::CALLS,
    scc_core::predicates::IMPORTS,
    scc_core::predicates::IMPLEMENTS,
    scc_core::predicates::INHERITS,
    scc_core::predicates::READS,
    scc_core::predicates::WRITES,
    scc_core::predicates::DEPENDS_ON,
    scc_core::predicates::CONTAINS,
];

/// Map an input symbol kind to the strings the native writer uses
/// (`crate::write::core_symbol_kind`); anything outside the documented set
/// falls back to `symbol`.
fn map_kind(kind: Option<&str>) -> &'static str {
    match kind {
        Some("function") => "function",
        Some("method") => "method",
        Some("class") => "class",
        Some("const") => "const",
        Some("interface") => "interface",
        Some("type") => "type",
        Some("enum") => "enum",
        _ => "symbol",
    }
}

/// Source evidence for one imported symbol/edge, tagged `gitnexus`.
fn gitnexus_evidence(path: &str, kind: &str, symbol: &str, version: Option<&str>) -> Evidence {
    make_evidence(
        evidence_id(path, kind, symbol, 0),
        EvidenceType::Source,
        path,
        Some(symbol),
        "gitnexus",
        version,
    )
}

/// Resolve one edge endpoint: first by `gitnexus_id` (the `<id_key>`
/// field) against symbols imported this run, then by the `<file_key>` /
/// `<name_key>` fallback fields into the stable `symbol_id` scheme. Returns
/// `None` when neither resolution applies.
fn resolve_ref(
    entry: &Value,
    repo: &str,
    id_key: &str,
    file_key: &str,
    name_key: &str,
    by_id: &HashMap<String, String>,
) -> Option<String> {
    if let Some(gid) = get_str(entry, id_key) {
        if let Some(id) = by_id.get(gid) {
            return Some(id.clone());
        }
    }
    let file = get_str(entry, file_key)?;
    let name = get_str(entry, name_key)?;
    Some(symbol_id(repo, file, name))
}

/// Import one symbol/edge block (a single repository) into `store`.
fn import_block(
    store: &Store,
    repo: &str,
    source: &str,
    symbols: &[Value],
    edges: &[Value],
    version: Option<&str>,
    report: &mut GitnexusReport,
) -> Result<(), String> {
    // gitnexus_id -> mapped entity id, populated in input order so edges
    // resolve regardless of array order.
    let mut by_id: HashMap<String, String> = HashMap::new();
    // (subject, predicate, object) dedupe within this run.
    let mut edge_seen: HashSet<(String, String, String)> = HashSet::new();
    let sources = [source.to_string()];

    for entry in symbols {
        if entry.as_object().is_none() {
            report.errors += 1;
            continue;
        }
        let Some(name) = get_str(entry, "name") else {
            report.errors += 1;
            continue;
        };
        let Some(file) = get_str(entry, "file") else {
            report.errors += 1;
            continue;
        };
        let gid = get_str(entry, "id").map(str::to_string);
        let kind = map_kind(get_str(entry, "kind"));
        let signature = get_str(entry, "signature");
        let id = symbol_id(repo, file, name);

        let mut e = Entity::new(id.clone(), kinds::SYMBOL, name.to_string());
        e.attr("kind", serde_json::json!(kind));
        e.attr("file", serde_json::json!(file));
        if let Some(sig) = signature {
            e.attr("signature", serde_json::json!(sig));
        }
        if let Some(g) = &gid {
            e.attr("gitnexus_id", serde_json::json!(g));
        }
        let ev = gitnexus_evidence(file, kind, name, version);
        store
            .insert_evidence(&ev)
            .map_err(|e| format!("gitnexus: evidence: {e}"))?;
        e.evidence.push(ev.id.clone());
        store
            .insert_entity(&e, &sources)
            .map_err(|e| format!("gitnexus: entity: {e}"))?;
        store
            .insert_symbol(file, name, kind, signature, 0, 0, true, None)
            .map_err(|e| format!("gitnexus: symbol row: {e}"))?;
        if let Some(g) = &gid {
            by_id.insert(g.clone(), id);
        }
        report.symbols += 1;
    }

    for entry in edges {
        if entry.as_object().is_none() {
            report.errors += 1;
            continue;
        }
        let Some(predicate) = get_str(entry, "predicate") else {
            report.errors += 1;
            continue;
        };
        if !ONTOLOGY.contains(&predicate) {
            report.errors += 1;
            continue;
        }
        let Some(subject) = resolve_ref(entry, repo, "subject", "subject_file", "subject_name", &by_id)
        else {
            report.errors += 1;
            continue;
        };
        let Some(object) = resolve_ref(entry, repo, "object", "object_file", "object_name", &by_id)
        else {
            report.errors += 1;
            continue;
        };
        if !edge_seen.insert((subject.clone(), predicate.to_string(), object.clone())) {
            continue;
        }
        let provenance = match get_str(entry, "provenance") {
            Some("RESOLVED") => Provenance::Resolved,
            Some("EXTRACTED") => Provenance::Extracted,
            _ => Provenance::Inferred,
        };
        let ev = gitnexus_evidence(&subject, "edge", &object, version);
        store
            .insert_evidence(&ev)
            .map_err(|e| format!("gitnexus: evidence: {e}"))?;
        let mut rel = Relationship::new(
            rel_id(&[&subject, predicate, &object]),
            subject,
            predicate,
            object,
            provenance,
        )
        .with_evidence(vec![ev.id.clone()]);
        if rel.provenance == Provenance::Resolved {
            rel = rel.with_confidence(0.99);
        }
        store
            .insert_relationship(&rel, source)
            .map_err(|e| format!("gitnexus: relationship: {e}"))?;
        report.edges += 1;
    }

    Ok(())
}

/// Import a GitNexus-style export file. See module docs for the accepted
/// shapes and the defensive error rules.
pub fn import_gitnexus(store: &Store, path: &Path) -> Result<GitnexusReport, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("gitnexus: cannot read {}: {e}", path.display()))?;
    let root: Value = serde_json::from_str(&text)
        .map_err(|e| format!("gitnexus: invalid JSON in {}: {e}", path.display()))?;
    let root = root
        .as_object()
        .ok_or("gitnexus: top-level JSON must be an object")?;

    let version = root.get("version").and_then(Value::as_str);
    let source = path.to_string_lossy().to_string();
    let mut report = GitnexusReport::default();

    if let Some(repos) = root.get("repositories").and_then(Value::as_array) {
        let mut seen: HashSet<String> = HashSet::new();
        for repo in repos {
            if repo.as_object().is_none() {
                report.errors += 1;
                continue;
            }
            let Some(name) = get_str(repo, "repository") else {
                report.errors += 1;
                continue;
            };
            // First block wins for a repository id collision: later
            // duplicates are skipped silently (no log, no counts).
            if !seen.insert(name.to_string()) {
                continue;
            }
            let symbols = repo
                .get("symbols")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let edges = repo
                .get("edges")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            import_block(
                store,
                name,
                &source,
                &symbols,
                &edges,
                version,
                &mut report,
            )?;
        }
    } else {
        let symbols = root
            .get("symbols")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let edges = root
            .get("edges")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        import_block(
            store,
            &store.repo_id,
            &source,
            &symbols,
            &edges,
            version,
            &mut report,
        )?;
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scc_core::predicates;
    use tempfile::TempDir;

    fn store() -> (Store, TempDir) {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&root.join("scc.db"), &root).unwrap();
        (store, dir)
    }

    fn write(dir: &TempDir, name: &str, text: &str) -> std::path::PathBuf {
        let p = dir.path().join(name);
        std::fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn single_repo_imports_symbols_and_resolved_edges() {
        let (store, dir) = store();
        let p = write(
            &dir,
            "graph.json",
            r#"{
              "producer": "gitnexus",
              "version": "0.1",
              "symbols": [
                {"id": "sym:1", "name": "helper", "file": "a.py", "kind": "function", "signature": "helper(x: int) -> int"},
                {"id": "sym:2", "name": "other", "file": "a.py", "kind": "function", "signature": "other() -> None"}
              ],
              "edges": [
                {"subject": "sym:1", "predicate": "calls", "object": "sym:2", "provenance": "RESOLVED"}
              ]
            }"#,
        );

        let rep = import_gitnexus(&store, &p).unwrap();
        assert_eq!(
            rep,
            GitnexusReport {
                symbols: 2,
                edges: 1,
                errors: 0
            }
        );

        let helper = symbol_id("repo", "a.py", "helper");
        let other = symbol_id("repo", "a.py", "other");

        // symbol entity: kind/file/signature/gitnexus_id attributes
        let se = store.get_entity(&helper).unwrap().unwrap();
        assert_eq!(se.kind, kinds::SYMBOL);
        assert_eq!(se.attributes.get("kind").and_then(Value::as_str), Some("function"));
        assert_eq!(se.attributes.get("file").and_then(Value::as_str), Some("a.py"));
        assert_eq!(
            se.attributes.get("signature").and_then(Value::as_str),
            Some("helper(x: int) -> int")
        );
        assert_eq!(
            se.attributes.get("gitnexus_id").and_then(Value::as_str),
            Some("sym:1")
        );

        // symbols table row with native writer kind + signature
        let rows = store.symbols_in_file("a.py").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].1, "helper");
        assert_eq!(rows[0].2, "function");
        assert_eq!(rows[0].3.as_deref(), Some("helper(x: int) -> int"));

        // symbol evidence tagged gitnexus with the top-level version
        let ev = store.get_evidence(&se.evidence[0]).unwrap().unwrap();
        assert_eq!(ev.r#type, EvidenceType::Source);
        assert_eq!(ev.extractor.as_deref(), Some("gitnexus"));
        assert_eq!(ev.extractor_version.as_deref(), Some("0.1"));

        // RESOLVED call edge at 0.99 confidence
        let calls = store
            .relationships_between(&helper, predicates::CALLS, &other)
            .unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].provenance, Provenance::Resolved);
        assert!((calls[0].confidence - 0.99).abs() < 1e-9);
        let ev = store.get_evidence(&calls[0].evidence[0]).unwrap().unwrap();
        assert_eq!(ev.extractor.as_deref(), Some("gitnexus"));
    }

    #[test]
    fn unknown_id_edges_resolve_via_fallback_fields() {
        let (store, dir) = store();
        let p = write(
            &dir,
            "fallback.json",
            r#"{
              "producer": "gitnexus",
              "version": "0.2",
              "symbols": [
                {"id": "sym:1", "name": "helper", "file": "a.py", "kind": "function"},
                {"id": "sym:2", "name": "other", "file": "b.py", "kind": "function"}
              ],
              "edges": [
                {"subject": "gone:1", "subject_file": "a.py", "subject_name": "helper",
                 "predicate": "calls", "object": "sym:2", "provenance": "EXTRACTED"},
                {"subject": "sym:1", "predicate": "reads",
                 "object": "gone:2", "object_file": "b.py", "object_name": "other",
                 "provenance": "EXTRACTED"},
                {"subject": "gone:3", "predicate": "calls", "object": "sym:2", "provenance": "RESOLVED"}
              ]
            }"#,
        );

        let rep = import_gitnexus(&store, &p).unwrap();
        // both fallback edges land; the unresolvable one is skipped
        assert_eq!(
            rep,
            GitnexusReport {
                symbols: 2,
                edges: 2,
                errors: 1
            }
        );

        let helper = symbol_id("repo", "a.py", "helper");
        let other = symbol_id("repo", "b.py", "other");
        let calls = store
            .relationships_between(&helper, predicates::CALLS, &other)
            .unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].provenance, Provenance::Extracted);
        // object fallback resolves the second edge (helper reads other)
        let reads = store
            .relationships_between(&helper, predicates::READS, &other)
            .unwrap();
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].provenance, Provenance::Extracted);
    }

    #[test]
    fn unknown_predicate_is_skipped() {
        let (store, dir) = store();
        let p = write(
            &dir,
            "pred.json",
            r#"{
              "producer": "gitnexus",
              "symbols": [
                {"id": "sym:1", "name": "helper", "file": "a.py", "kind": "function"},
                {"id": "sym:2", "name": "other", "file": "a.py", "kind": "function"}
              ],
              "edges": [
                {"subject": "sym:1", "predicate": "teleports", "object": "sym:2", "provenance": "RESOLVED"},
                {"subject": "sym:1", "predicate": "calls", "object": "sym:2", "provenance": "INFERRED"}
              ]
            }"#,
        );

        let rep = import_gitnexus(&store, &p).unwrap();
        assert_eq!(
            rep,
            GitnexusReport {
                symbols: 2,
                edges: 1,
                errors: 1
            }
        );
        let helper = symbol_id("repo", "a.py", "helper");
        let other = symbol_id("repo", "a.py", "other");
        let calls = store
            .relationships_between(&helper, predicates::CALLS, &other)
            .unwrap();
        assert_eq!(calls.len(), 1);
        // anything but RESOLVED/EXTRACTED becomes INFERRED at default confidence
        assert_eq!(calls[0].provenance, Provenance::Inferred);
        assert!((calls[0].confidence - 0.7).abs() < 1e-9);
    }

    #[test]
    fn malformed_json_and_entries_never_panic() {
        let (store, dir) = store();
        let bad = write(&dir, "bad.json", "{not json");
        assert!(import_gitnexus(&store, &bad).is_err());
        std::fs::write(&bad, "[]").unwrap();
        assert!(import_gitnexus(&store, &bad).is_err());

        let p = write(
            &dir,
            "malformed.json",
            r#"{
              "producer": "gitnexus",
              "symbols": [
                {"id": "sym:1", "name": "helper", "file": "a.py", "kind": "function"},
                "junk",
                {"id": "sym:2", "file": "a.py", "kind": "function"},
                {"id": "sym:3", "name": "nofile", "kind": "function"}
              ],
              "edges": [
                42,
                {"subject": "sym:1", "object": "sym:1"},
                {"predicate": "calls", "object": "sym:1"},
                {"subject": "nowhere", "predicate": "calls", "object": "sym:1"}
              ]
            }"#,
        );

        let rep = import_gitnexus(&store, &p).unwrap();
        // 1 non-object symbol + 1 missing name + 1 missing file
        // + 1 non-object edge + 1 missing predicate + 1 missing subject
        // + 1 unresolvable subject
        assert_eq!(
            rep,
            GitnexusReport {
                symbols: 1,
                edges: 0,
                errors: 7
            }
        );
    }

    #[test]
    fn multi_repo_shape_imports_each_repository() {
        let (store, dir) = store();
        let p = write(
            &dir,
            "repos.json",
            r#"{
              "producer": "gitnexus",
              "version": "1.0",
              "repositories": [
                {"repository": "alpha",
                 "symbols": [{"id": "a:1", "name": "run", "file": "main.py", "kind": "function"}],
                 "edges": []},
                {"repository": "beta",
                 "symbols": [{"id": "b:1", "name": "serve", "file": "main.py", "kind": "function"}],
                 "edges": [{"subject": "b:1", "predicate": "calls", "object": "b:1", "provenance": "RESOLVED"}]}
              ]
            }"#,
        );

        let rep = import_gitnexus(&store, &p).unwrap();
        assert_eq!(
            rep,
            GitnexusReport {
                symbols: 2,
                edges: 1,
                errors: 0
            }
        );

        // ids derive from each block's repository name
        let alpha = symbol_id("alpha", "main.py", "run");
        let beta = symbol_id("beta", "main.py", "serve");
        assert!(store.get_entity(&alpha).unwrap().is_some());
        assert!(store.get_entity(&beta).unwrap().is_some());
        let calls = store
            .relationships_between(&beta, predicates::CALLS, &beta)
            .unwrap();
        assert_eq!(calls.len(), 1);

        // duplicate repository name: first block wins, no extra rows
        let p2 = write(
            &dir,
            "dup.json",
            r#"{
              "producer": "gitnexus",
              "repositories": [
                {"repository": "alpha",
                 "symbols": [{"id": "a:2", "name": "dup", "file": "x.py", "kind": "function"}],
                 "edges": []},
                {"repository": "alpha",
                 "symbols": [{"id": "a:3", "name": "dup2", "file": "y.py", "kind": "function"}],
                 "edges": []}
              ]
            }"#,
        );
        let rep = import_gitnexus(&store, &p2).unwrap();
        assert_eq!(
            rep,
            GitnexusReport {
                symbols: 1,
                edges: 0,
                errors: 0
            }
        );
        assert!(store.get_entity(&symbol_id("alpha", "x.py", "dup")).unwrap().is_some());
        assert!(store.get_entity(&symbol_id("alpha", "y.py", "dup2")).unwrap().is_none());
    }

    #[test]
    fn reimport_is_idempotent_for_relationships() {
        let (store, dir) = store();
        let p = write(
            &dir,
            "idem.json",
            r#"{
              "producer": "gitnexus",
              "version": "0.1",
              "symbols": [
                {"id": "sym:1", "name": "helper", "file": "a.py", "kind": "function"},
                {"id": "sym:2", "name": "other", "file": "a.py", "kind": "function"}
              ],
              "edges": [
                {"subject": "sym:1", "predicate": "calls", "object": "sym:2", "provenance": "RESOLVED"},
                {"subject": "sym:1", "predicate": "calls", "object": "sym:2", "provenance": "EXTRACTED"}
              ]
            }"#,
        );

        let rep = import_gitnexus(&store, &p).unwrap();
        // the duplicate (subject,predicate,object) edge is deduped in-run
        assert_eq!(
            rep,
            GitnexusReport {
                symbols: 2,
                edges: 1,
                errors: 0
            }
        );
        let before = store.all_relationships().unwrap().len();

        // re-import: content-derived ids overwrite the same rows
        import_gitnexus(&store, &p).unwrap();
        assert_eq!(store.all_relationships().unwrap().len(), before);
    }
}
