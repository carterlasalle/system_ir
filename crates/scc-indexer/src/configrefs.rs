//! Config-reference detection (SCC-147): syntactic, line-based scan for
//! configuration-key reads (`os.environ[...]`, `os.environ.get(...)`,
//! `os.getenv(...)`, bare `getenv(...)`, `process.env.X`, `process.env["X"]`)
//! that links calling code to `configuration` entities through
//! `configured_by` edges.
//!
//! Design notes:
//! - No tree-sitter, no execution, no import tracking: purely lexical and
//!   deliberately permissive (a comment containing `os.getenv("K")` still
//!   yields a hit). Determinism is the contract — same content always
//!   produces the same hits and the same content-derived ids.
//! - Only single- and double-quoted string literals are read. Values are
//!   never stored — only the key, so secrets referenced through env reads
//!   never leak into the store.
//! - Ids reuse the blake3 schemes from `crate::write` (rel_id / evidence_id),
//!   which keeps re-indexing idempotent: the same hit maps to the same rel.

use crate::write::{evidence_id, rel_id};
use scc_core::kinds;
use scc_core::{entity_id, symbol_id, Evidence, EvidenceType, Provenance, Relationship};
use scc_store::Store;

/// One detected configuration-key read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigRefHit {
    /// Repo-relative path of the referencing file. Filled by the caller:
    /// [`scan_config_refs`] cannot know the path and leaves it empty.
    pub caller_file: String,
    /// Name of the enclosing function/class when determinable from the
    /// content (nearest preceding `def`/`class`/`function` boundary);
    /// methods get the bare name.
    pub caller_symbol: Option<String>,
    /// The configuration key that was read.
    pub key: String,
    /// 1-based line number of the read.
    pub line: u32,
}

/// Scan `content` for configuration-key reads in the given `language`
/// (`python`, `typescript`, or `javascript`; anything else yields nothing).
/// Pure and deterministic: identical input produces identical output.
pub fn scan_config_refs(content: &str, language: &str) -> Vec<ConfigRefHit> {
    let mut hits: Vec<ConfigRefHit> = Vec::new();
    match language {
        "python" => scan_env_refs(content, language, python_keys, &mut hits),
        "typescript" | "javascript" => scan_env_refs(content, language, ts_keys, &mut hits),
        _ => {}
    }
    hits.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.key.cmp(&b.key)));
    hits.dedup();
    hits
}

/// Apply `hits` for one file into the store: ensure each referenced
/// configuration entity exists (name-only; never values), then create or
/// refresh a `configured_by` relationship from the calling symbol (or the
/// file when no symbol is known) to the configuration entity, backed by a
/// `config` evidence row. Idempotent: the same hit always maps to the same
/// relationship and evidence ids. Returns the number of hits applied.
pub fn apply_config_refs(
    store: &Store,
    file: &str,
    language: &str,
    content: &str,
    hits: Vec<ConfigRefHit>,
) -> Result<usize, String> {
    let repo = &store.repo_id;
    let file_id = entity_id(repo, kinds::FILE, file);

    // Configuration entities seen so far, so a key is only inserted once.
    let mut known_configs: std::collections::HashSet<String> = store
        .entities_by_kind(kinds::CONFIGURATION)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|e| e.id)
        .collect();

    let mut hits = hits;
    hits.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.key.cmp(&b.key)));

    let revision = store.meta_get("revision").ok().flatten();

    let mut applied = 0usize;
    for hit in hits {
        let key = hit.key.trim().to_string();
        if key.is_empty() {
            continue;
        }
        let config_id = entity_id(repo, kinds::CONFIGURATION, &key);
        if !known_configs.contains(&config_id) {
            let e = scc_core::Entity::new(config_id.clone(), kinds::CONFIGURATION, key.clone());
            store
                .insert_entity(&e, &[file.to_string()])
                .map_err(|e| e.to_string())?;
            known_configs.insert(config_id.clone());
        }

        // Subject: the enclosing symbol when the hit has one (either filled
        // by the caller or recovered from the content), else the file.
        let caller = hit
            .caller_symbol
            .clone()
            .or_else(|| enclosing_symbol(content, language, hit.line));
        let subject = match &caller {
            Some(c) => symbol_id(repo, file, c),
            None => file_id.clone(),
        };

        let mut ev = Evidence::source(evidence_id(file, "config", &key, hit.line), file);
        ev.r#type = EvidenceType::Config;
        ev.symbol = Some(key.clone());
        ev.start_line = Some(hit.line);
        ev.extractor = Some("scc-configrefs".to_string());
        if let Some(rev) = &revision {
            ev.revision = Some(rev.clone());
        }
        store.insert_evidence(&ev).map_err(|e| e.to_string())?;

        let rel = Relationship::new(
            rel_id(&["configured_by", &subject, &config_id]),
            subject,
            scc_core::predicates::CONFIGURED_BY,
            config_id,
            Provenance::Extracted,
        )
        .with_evidence(vec![ev.id.clone()]);
        store
            .insert_relationship(&rel, file)
            .map_err(|e| e.to_string())?;
        applied += 1;
    }
    Ok(applied)
}

// ---------------------------------------------------------------------------
// scanning
// ---------------------------------------------------------------------------

/// Walk `content` line by line; for each line collect keys via
/// `keys_for_line`, then remember the nearest preceding function/class
/// boundary as the caller symbol for any hits on that line.
fn scan_env_refs(
    content: &str,
    language: &str,
    keys_for_line: impl Fn(&str, &mut Vec<String>),
    hits: &mut Vec<ConfigRefHit>,
) {
    let mut boundary: Option<String> = None;
    for (idx, raw) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let mut keys: Vec<String> = Vec::new();
        keys_for_line(raw.trim_start(), &mut keys);
        for key in keys {
            if key.is_empty() {
                continue;
            }
            hits.push(ConfigRefHit {
                caller_file: String::new(),
                caller_symbol: boundary.clone(),
                key,
                line: line_no,
            });
        }
        // A boundary on this line applies to *later* lines ("nearest
        // preceding line" per SCC-147).
        if let Some(name) = boundary_name(raw, language) {
            boundary = Some(name);
        }
    }
}

/// Python reads: `os.environ["K"]` / `os.environ.get("K")` / `os.getenv("K")`
/// / bare `getenv("K")` / `environ.get("K")`. Tokens are located without
/// regex; the key is the first quoted literal after the token.
fn python_keys(line: &str, out: &mut Vec<String>) {
    for pos in token_positions(line, "os.environ") {
        if let Some(key) = next_quoted(line, pos + "os.environ".len()) {
            out.push(key);
        }
    }
    for token in ["os.getenv(", "getenv(", "environ.get("] {
        for pos in token_positions(line, token) {
            if let Some(key) = next_quoted(line, pos + token.len()) {
                out.push(key);
            }
        }
    }
}

/// TypeScript/JavaScript reads: `process.env.KEY` and `process.env["KEY"]`.
fn ts_keys(line: &str, out: &mut Vec<String>) {
    for pos in token_positions(line, "process.env") {
        let rest = line[pos + "process.env".len()..].trim_start();
        if let Some(after) = rest.strip_prefix('.') {
            let ident = take_ident(after.trim_start());
            if !ident.is_empty() {
                out.push(ident.to_string());
            }
        } else if rest.starts_with('[') {
            if let Some(key) = next_quoted(rest, 0) {
                out.push(key);
            }
        }
    }
}

/// All (non-overlapping) byte positions of `needle` in `haystack`.
fn token_positions(haystack: &str, needle: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = haystack[from..].find(needle) {
        let pos = from + rel;
        out.push(pos);
        from = pos + needle.len();
    }
    out
}

/// First single- or double-quoted literal at or after byte `from`; returns
/// the contents without quotes. Escapes are not interpreted (kept simple and
/// deterministic).
fn next_quoted(s: &str, from: usize) -> Option<String> {
    let tail = &s[from..];
    let quote = tail.bytes().position(|b| b == b'\'' || b == b'"')?;
    let q = tail.as_bytes()[quote];
    let inner = &tail[quote + 1..];
    let end = inner.bytes().position(|b| b == q)?;
    Some(inner[..end].to_string())
}

/// Leading identifier characters ([A-Za-z0-9_$], never starting with a digit).
fn take_ident(s: &str) -> &str {
    let mut end = 0;
    for (i, c) in s.char_indices() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
            if end == 0 && c.is_ascii_digit() {
                break;
            }
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    &s[..end]
}

/// Name of the function/class declared on `line`, if any. Python matches
/// `def`/`async def`/`class`; TS/JS additionally matches `function` (with
/// optional `export`/`async` prefixes). Methods get the bare name.
fn boundary_name(line: &str, language: &str) -> Option<String> {
    let t = line.trim_start();
    let rest = match language {
        "python" => {
            if let Some(r) = t.strip_prefix("async def ") {
                r
            } else if let Some(r) = t.strip_prefix("def ") {
                r
            } else if let Some(r) = t.strip_prefix("class ") {
                r
            } else {
                return None;
            }
        }
        _ => {
            let mut r = t;
            if let Some(x) = r.strip_prefix("export ") {
                r = x;
            }
            if let Some(x) = r.strip_prefix("async function ") {
                x
            } else if let Some(x) = r.strip_prefix("function ") {
                x
            } else if let Some(x) = r.strip_prefix("class ") {
                x
            } else {
                return None;
            }
        }
    };
    let name: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
        .collect();
    let valid = !name.is_empty() && !name.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false);
    valid.then_some(name)
}

/// Name of the nearest function/class declared on a line strictly above
/// 1-based `line` in `content`, if any.
fn enclosing_symbol(content: &str, language: &str, line: u32) -> Option<String> {
    if line < 2 {
        return None;
    }
    let lines: Vec<&str> = content.lines().collect();
    let mut i = (line - 1) as usize;
    while i > 0 {
        i -= 1;
        if let Some(name) = boundary_name(lines[i], language) {
            return Some(name);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use scc_core::kinds;

    fn store_for() -> (Store, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&tmp.path().join("scc.db"), &root).unwrap();
        (store, tmp)
    }

    #[test]
    fn python_env_patterns() {
        let content = r#"import os

def main():
    db = os.environ["DATABASE_URL"]
    port = os.environ.get("PORT", "8080")
    host = os.getenv("HOST")
    user = getenv("USER")
    secret = environ.get('SECRET')
    return db
"#;
        let hits = scan_config_refs(content, "python");
        let keys: Vec<&str> = hits.iter().map(|h| h.key.as_str()).collect();
        assert_eq!(keys, ["DATABASE_URL", "PORT", "HOST", "USER", "SECRET"]);
        // sorted by line; all inside main()
        let lines: Vec<u32> = hits.iter().map(|h| h.line).collect();
        assert_eq!(lines, [4, 5, 6, 7, 8]);
        assert!(hits.iter().all(|h| h.caller_symbol.as_deref() == Some("main")));
    }

    #[test]
    fn python_multiple_keys_one_line() {
        let content = "a = os.getenv(\"X\") or os.environ.get(\"Y\")\n";
        let hits = scan_config_refs(content, "python");
        let keys: Vec<&str> = hits.iter().map(|h| h.key.as_str()).collect();
        // same line: sorted by key; duplicates collapsed
        assert_eq!(keys, ["X", "Y"]);
        assert!(hits.iter().all(|h| h.line == 1));
    }

    #[test]
    fn ts_env_patterns() {
        let content = r#"export function load() {
    const db = process.env.DATABASE_URL;
    const port = process.env["PORT"];
    const queue = process.env['QUEUE'];
    return db;
}
"#;
        let hits = scan_config_refs(content, "typescript");
        let keys: Vec<&str> = hits.iter().map(|h| h.key.as_str()).collect();
        assert_eq!(keys, ["DATABASE_URL", "PORT", "QUEUE"]);
        assert!(hits.iter().all(|h| h.caller_symbol.as_deref() == Some("load")));

        let js = scan_config_refs(content, "javascript");
        assert_eq!(js.len(), hits.len());
    }

    #[test]
    fn negative_no_match() {
        assert!(scan_config_refs("print('hello')\nconfig = load_config()\n", "python").is_empty());
        // a variable, not a literal, is not a config ref
        assert!(scan_config_refs("k = os.environ.get(varname)\n", "python").is_empty());
        // plain identifier access on process.env without a key
        assert!(scan_config_refs("const x = process.env;\n", "typescript").is_empty());
        // unsupported languages are skipped entirely
        assert!(scan_config_refs("x = os.getenv(\"K\")\n", "rust").is_empty());
        assert!(scan_config_refs("x = process.env.K\n", "go").is_empty());
    }

    #[test]
    fn enclosing_symbol_extraction() {
        let content = r#"import os

class Api:
    def connect(self):
        return os.getenv("API_KEY")

def helper():
    return os.environ.get("EXTRA")
"#;
        let hits = scan_config_refs(content, "python");
        let by_key: std::collections::BTreeMap<_, _> = hits
            .into_iter()
            .map(|h| (h.key.clone(), h.caller_symbol))
            .collect();
        // methods get the bare name; the nearest boundary (def) beats class
        assert_eq!(by_key["API_KEY"].as_deref(), Some("connect"));
        assert_eq!(by_key["EXTRA"].as_deref(), Some("helper"));
    }

    #[test]
    fn no_symbol_means_file_subject() {
        let content = "import os\nx = os.getenv(\"TOP_LEVEL\")\n";
        let hits = scan_config_refs(content, "python");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].caller_symbol.is_none());
    }

    #[test]
    fn apply_creates_entities_and_rels() {
        let (store, _t) = store_for();
        let file = "src/app.py";
        let content = "import os\n\ndef main():\n    return os.getenv(\"API_KEY\")\n";
        let hits = scan_config_refs(content, "python");
        assert_eq!(hits.len(), 1);

        // the indexer would create the symbol entity + symbols row
        let sym_id = scc_core::symbol_id(&store.repo_id, file, "main");
        store
            .insert_entity(
                &scc_core::Entity::new(sym_id.clone(), kinds::SYMBOL, "main"),
                &[file.to_string()],
            )
            .unwrap();
        store
            .insert_symbol(file, "main", "function", None, 3, 4, false, None)
            .unwrap();

        let n = apply_config_refs(&store, file, "python", content, hits).unwrap();
        assert_eq!(n, 1);

        // configuration entity created, name-only (no values, no attributes)
        let cfgs = store.entities_by_kind(kinds::CONFIGURATION).unwrap();
        assert_eq!(cfgs.len(), 1);
        assert_eq!(cfgs[0].name, "API_KEY");
        assert!(cfgs[0].attributes.is_empty());

        // configured_by rel: symbol -> configuration, EXTRACTED, confidence 1.0
        let cfg_rels: Vec<_> = store
            .all_relationships()
            .unwrap()
            .into_iter()
            .filter(|r| r.predicate == scc_core::predicates::CONFIGURED_BY)
            .collect();
        assert_eq!(cfg_rels.len(), 1);
        let rel = &cfg_rels[0];
        assert_eq!(rel.subject, sym_id);
        assert_eq!(
            rel.object,
            scc_core::entity_id(&store.repo_id, kinds::CONFIGURATION, "API_KEY")
        );
        assert_eq!(rel.provenance, scc_core::Provenance::Extracted);
        assert_eq!(rel.confidence, 1.0);
        assert_eq!(rel.evidence.len(), 1);

        // evidence row: type config, path, symbol = key, line, extractor
        let ev = store
            .evidence_for_path(file)
            .unwrap()
            .into_iter()
            .find(|e| e.extractor.as_deref() == Some("scc-configrefs"))
            .expect("configrefs evidence row");
        assert_eq!(ev.r#type, scc_core::EvidenceType::Config);
        assert_eq!(ev.symbol.as_deref(), Some("API_KEY"));
        assert_eq!(ev.start_line, Some(4));
        assert_eq!(rel.evidence[0], ev.id);

        // idempotent: re-applying the same scan changes nothing
        let hits2 = scan_config_refs(content, "python");
        let n2 = apply_config_refs(&store, file, "python", content, hits2).unwrap();
        assert_eq!(n2, 1);
        let cfg_rels2: Vec<_> = store
            .all_relationships()
            .unwrap()
            .into_iter()
            .filter(|r| r.predicate == scc_core::predicates::CONFIGURED_BY)
            .collect();
        assert_eq!(cfg_rels2.len(), 1);
        assert_eq!(cfg_rels2[0].id, rel.id);
        assert_eq!(store.entities_by_kind(kinds::CONFIGURATION).unwrap().len(), 1);
    }

    #[test]
    fn apply_file_subject_without_symbol() {
        let (store, _t) = store_for();
        let file = "top.py";
        let content = "import os\nx = os.getenv(\"TOP_LEVEL\")\n";
        let hits = scan_config_refs(content, "python");
        apply_config_refs(&store, file, "python", content, hits).unwrap();
        let rel = store
            .all_relationships()
            .unwrap()
            .into_iter()
            .find(|r| r.predicate == scc_core::predicates::CONFIGURED_BY)
            .expect("configured_by rel");
        assert_eq!(rel.subject, scc_core::entity_id(&store.repo_id, kinds::FILE, file));
    }

    #[test]
    fn shared_key_shared_entity() {
        let (store, _t) = store_for();
        for (file, content) in [
            ("a.py", "import os\nx = os.getenv(\"SHARED\")\n"),
            ("b.py", "import os\ny = os.environ.get(\"SHARED\")\n"),
        ] {
            let hits = scan_config_refs(content, "python");
            apply_config_refs(&store, file, "python", content, hits).unwrap();
        }
        let cfgs = store.entities_by_kind(kinds::CONFIGURATION).unwrap();
        assert_eq!(cfgs.len(), 1, "one shared configuration entity");
        let rels = store
            .all_relationships()
            .unwrap()
            .into_iter()
            .filter(|r| r.predicate == scc_core::predicates::CONFIGURED_BY)
            .count();
        assert_eq!(rels, 2, "one configured_by per referencing file");
    }
}
