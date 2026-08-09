//! CBM adapter (SCC-202): import evidence from codebase-memory-mcp's
//! exported knowledge graph — `.codebase-memory/graph.db.zst` (a
//! zstd-compressed, index-stripped SQLite property graph).
//!
//! The importer decompresses, opens the SQLite snapshot read-only,
//! introspects its schema, and maps symbol-ish and relationship-ish rows
//! into SCC entities/relationships with RESOLVED provenance (compiler-grade
//! extraction). Defensive: unknown table shapes are counted, never fatal.

use scc_store::Store;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CbmReport {
    pub symbols: usize,
    pub relationships: usize,
    pub tables: usize,
    pub skipped_tables: usize,
    pub errors: usize,
}

/// Decompress a zstd stream into memory (bounded to 512 MiB).
pub fn zstd_decode(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let decoder = ruzstd::StreamingDecoder::new(bytes)
        .map_err(|e| format!("zstd: {e}"))?;
    let mut out = Vec::new();
    decoder
        .take(512 * 1024 * 1024)
        .read_to_end(&mut out)
        .map_err(|e| format!("zstd read: {e}"))?;
    Ok(out)
}

/// Heuristic column classification for unknown schemas.
enum Col {
    Name,
    Kind,
    File,
    Subject,
    Object,
    RelKind,
    Id,
    Other,
}

fn table_columns(conn: &scc_store::rusqlite::Connection, table: &str) -> Result<Vec<String>, String> {
    let mut s = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| format!("pragma {table}: {e}"))?;
    let rows = s
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

fn classify_column(name: &str) -> Col {
    let n = name.to_ascii_lowercase();
    if n.contains("name") {
        Col::Name
    } else if n.contains("kind") || n.contains("label") || n.contains("type") {
        Col::Kind
    } else if n.contains("file") || n.contains("path") {
        Col::File
    } else if n == "subject" || n.contains("source") || n.contains("from") || n.contains("caller") {
        Col::Subject
    } else if n == "object" || n.contains("target") || n.contains("to_") || n.contains("callee") {
        Col::Object
    } else if n.contains("relation") || n.contains("edge") || n == "predicate" || n == "rel" {
        Col::RelKind
    } else if n == "id" || n.ends_with("_id") {
        Col::Id
    } else {
        Col::Other
    }
}

/// Import a CBM graph snapshot (plain SQLite or `.zst`).
pub fn import_cbm(store: &Store, path: &Path) -> Result<CbmReport, String> {
    let raw = std::fs::read(path).map_err(|e| format!("cbm: {e}"))?;
    let sqlite_bytes = if raw.len() >= 4 && &raw[..4] == b"(\xb5/\xfd" {
        zstd_decode(&raw)?
    } else {
        raw
    };

    let tmp = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    std::fs::write(tmp.path(), &sqlite_bytes).map_err(|e| e.to_string())?;
    let conn = scc_store::rusqlite::Connection::open(tmp.path()).map_err(|e| format!("cbm sqlite: {e}"))?;
    let mut report = CbmReport::default();

    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .map_err(|e| format!("cbm schema: {e}"))?;
    let tables: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);

    for table in tables {
        if table.starts_with("sqlite_") || table.starts_with("fts") {
            continue;
        }
        report.tables += 1;
        // introspect columns
        let cols = table_columns(&conn, &table)?;
        let classes: Vec<(String, Col)> = cols
            .iter()
            .map(|c| (c.clone(), classify_column(c)))
            .collect();
        let has_name = classes.iter().any(|(_, c)| matches!(c, Col::Name));
        let has_subject = classes.iter().any(|(_, c)| matches!(c, Col::Subject));
        let has_object = classes.iter().any(|(_, c)| matches!(c, Col::Object));

        if has_subject && has_object {
            // relationship-ish table
            let (subj, _) = classes.iter().find(|(_, c)| matches!(c, Col::Subject)).unwrap();
            let (obj, _) = classes.iter().find(|(_, c)| matches!(c, Col::Object)).unwrap();
            let rel = classes
                .iter()
                .find(|(_, c)| matches!(c, Col::RelKind))
                .map(|(n, _)| n.clone());
            let sql = match &rel {
                Some(r) => format!(
                    "SELECT {subj}, {r}, {obj} FROM {table} LIMIT 100000"
                ),
                None => format!("SELECT {subj}, {obj} FROM {table} LIMIT 100000"),
            };
            if let Ok(mut s) = conn.prepare(&sql) {
                let rows = s
                    .query_map([], |r| {
                        let a = r.get::<_, String>(0).unwrap_or_default();
                        let c = match &rel {
                            Some(_) => r.get::<_, String>(1).unwrap_or_else(|_| "calls".into()),
                            None => "calls".to_string(),
                        };
                        let b = match &rel {
                            Some(_) => r.get::<_, String>(2).unwrap_or_default(),
                            None => r.get::<_, String>(1).unwrap_or_default(),
                        };
                        Ok((a, c, b))
                    })
                    .map_err(|e| e.to_string())?;
                for row in rows {
                    if let Ok((a, c, b)) = row {
                        if a.is_empty() || b.is_empty() {
                            continue;
                        }
                        let rel = scc_core::Relationship::new(
                            crate::write::rel_id(&["cbm", &a, &c, &b]),
                            scc_core::entity_id(&store.repo_id, "symbol", &a),
                            c.to_ascii_lowercase(),
                            scc_core::entity_id(&store.repo_id, "symbol", &b),
                            scc_core::Provenance::Resolved,
                        );
                        if store
                            .insert_relationship(&rel, "cbm:graph.db")
                            .is_ok()
                        {
                            report.relationships += 1;
                        }
                    }
                }
            }
        } else if has_name {
            // symbol-ish table
            let (name, _) = classes.iter().find(|(_, c)| matches!(c, Col::Name)).unwrap();
            let kind = classes
                .iter()
                .find(|(_, c)| matches!(c, Col::Kind))
                .map(|(n, _)| n.clone());
            let file = classes
                .iter()
                .find(|(_, c)| matches!(c, Col::File))
                .map(|(n, _)| n.clone());
            let sql = match (&kind, &file) {
                (Some(k), Some(f)) => {
                    format!("SELECT {name}, {k}, {f} FROM {table} LIMIT 100000")
                }
                (Some(k), None) => format!("SELECT {name}, {k} FROM {table} LIMIT 100000"),
                _ => format!("SELECT {name} FROM {table} LIMIT 100000"),
            };
            if let Ok(mut s) = conn.prepare(&sql) {
                let rows = s
                    .query_map([], |r| {
                        let n = r.get::<_, String>(0).unwrap_or_default();
                        let k = match &kind {
                            Some(_) => r.get::<_, String>(1).unwrap_or_else(|_| "symbol".into()),
                            None => "symbol".into(),
                        };
                        let f = match (&kind, &file) {
                            (Some(_), Some(_)) => r.get::<_, String>(2).unwrap_or_default(),
                            (None, Some(_)) => r.get::<_, String>(1).unwrap_or_default(),
                            _ => String::new(),
                        };
                        Ok((n, k, f))
                    })
                    .map_err(|e| e.to_string())?;
                for row in rows {
                    if let Ok((n, k, f)) = row {
                        if n.is_empty() {
                            continue;
                        }
                        let file = if f.is_empty() { "cbm".to_string() } else { f };
                        let id = scc_core::symbol_id(&store.repo_id, &file, &n);
                        let mut e = scc_core::Entity::new(id, "symbol", n);
                        e.attr("kind", serde_json::json!(k.to_ascii_lowercase()));
                        e.attr("file", serde_json::json!(file));
                        e.attr("extractor", serde_json::json!("cbm"));
                        if store.insert_entity(&e, &["cbm:graph.db".into()]).is_ok() {
                            report.symbols += 1;
                        }
                    }
                }
            }
        } else {
            report.skipped_tables += 1;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::process::Command;

    fn tmp_store() -> (Store, TempDir) {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        (store, dir)
    }

    fn build_graph_db() -> TempDir {
        // build a small sqlite graph: symbols + relationships
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("graph.db");
        let conn = scc_store::rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE symbols(id INTEGER PRIMARY KEY, name TEXT, kind TEXT, file TEXT);
             INSERT INTO symbols(name, kind, file) VALUES ('helper','function','a.py'), ('main','function','b.py');
             CREATE TABLE relationships(id INTEGER PRIMARY KEY, source TEXT, predicate TEXT, target TEXT);
             INSERT INTO relationships(source, predicate, target) VALUES ('main','calls','helper');
             CREATE TABLE irrelevant(id INTEGER PRIMARY KEY, blob BLOB);",
        )
        .unwrap();
        drop(conn);
        dir
    }

    #[test]
    fn zstd_roundtrip_with_cli() {
        let dir = TempDir::new().unwrap();
        let plain = dir.path().join("plain");
        std::fs::write(&plain, b"hello zstd world").unwrap();
        let zst = dir.path().join("plain.zst");
        let out = Command::new("zstd")
            .args(["-q", "-f"])
            .arg(&plain)
            .arg("-o")
            .arg(&zst)
            .output();
        if out.is_err() || !out.unwrap().status.success() {
            eprintln!("zstd CLI unavailable — skipping");
            return;
        }
        let bytes = std::fs::read(&zst).unwrap();
        let decoded = zstd_decode(&bytes).unwrap();
        assert_eq!(decoded, b"hello zstd world");
    }

    #[test]
    fn imports_plain_sqlite_graph() {
        let (store, _d) = tmp_store();
        let dir = build_graph_db();
        let report = import_cbm(&store, &dir.path().join("graph.db")).unwrap();
        assert!(report.symbols >= 2, "{report:?}");
        assert!(report.relationships >= 1, "{report:?}");
        let syms = store.entities_by_kind("symbol").unwrap();
        assert!(syms.iter().any(|e| e.name == "helper"));
        let rels = store.all_relationships().unwrap();
        assert!(
            rels.iter().any(|r| r.predicate == "calls"
                && r.provenance == scc_core::Provenance::Resolved),
            "{rels:?}"
        );
    }

    #[test]
    fn imports_zst_snapshot() {
        let (store, _d) = tmp_store();
        let dir = build_graph_db();
        let db = dir.path().join("graph.db");
        let zst = dir.path().join("graph.db.zst");
        let out = Command::new("zstd")
            .args(["-q", "-f"])
            .arg(&db)
            .arg("-o")
            .arg(&zst)
            .output();
        if out.is_err() || !out.unwrap().status.success() {
            eprintln!("zstd CLI unavailable — skipping");
            return;
        }
        let report = import_cbm(&store, &zst).unwrap();
        assert!(report.symbols >= 2, "{report:?}");
    }

    #[test]
    fn missing_file_errors() {
        let (store, _d) = tmp_store();
        assert!(import_cbm(&store, Path::new("/nonexistent")).is_err());
    }
}
