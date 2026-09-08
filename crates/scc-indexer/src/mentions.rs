//! Markdown backtick mentions as DECLARED evidence.
//!
//! Port of Ripwire's *observed* ingest_docs backtick scan: identifier
//! spans of length ≥ 3, outside fenced code, resolved against known
//! entities. Stored as DECLARED_AS (never CALLS, never PageRank).

use crate::write::{evidence_id, rel_id};
use scc_core::kinds;
use scc_core::{entity_id, Evidence, Provenance, Relationship};
use scc_store::Store;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Max matched entities per backtick span (bounded noise).
const MAX_TARGETS_PER_SPAN: usize = 8;

/// Identifier-like backtick spans considered for mention matching.
const MIN_MENTION_LEN: usize = 3;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct MentionStats {
    pub matched: u32,
    pub unmatched: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// trace:exempt reason=internal-detail
pub struct MarkdownMention {
    pub line: u32,
    pub span: String,
}

/// Extract identifier-like backtick spans outside fenced code.
/// Fences are 3+ backticks or tildes at the start of a trimmed line.
// trace:v1 id=impl.scc.index.markdown-backticks work=WORK-ripwire-lessons-phase2 satisfies=REQ-declared-mentions
pub fn extract_markdown_backticks(content: &str) -> Vec<MarkdownMention> {
    let mut out = Vec::new();
    let mut in_fence = false;
    let mut fence_ch = ' ';
    let mut fence_len = 0usize;
    for (i, raw) in content.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        let trimmed = line.trim_start();
        if let Some((ch, n)) = fence_open(trimmed) {
            if !in_fence {
                in_fence = true;
                fence_ch = ch;
                fence_len = n;
                continue;
            } else if ch == fence_ch && n >= fence_len {
                in_fence = false;
                continue;
            }
        }
        if in_fence {
            continue;
        }
        for span in backtick_spans(line) {
            if let Some(name) = normalize_mention_span(&span) {
                out.push(MarkdownMention {
                    line: (i as u32) + 1,
                    span: name,
                });
            }
        }
    }
    out
}

// trace:exempt reason=internal-detail
fn fence_open(trimmed: &str) -> Option<(char, usize)> {
    let ch = trimmed.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let n = trimmed.chars().take_while(|c| *c == ch).count();
    if n >= 3 {
        Some((ch, n))
    } else {
        None
    }
}

// trace:exempt reason=internal-detail
fn backtick_spans(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut b = 0usize;
    while b + 1 < chars.len() {
        if chars[b] != '`' {
            b += 1;
            continue;
        }
        let mut e = b + 1;
        while e < chars.len() && chars[e] != '`' {
            e += 1;
        }
        if e >= chars.len() {
            break;
        }
        let span: String = chars[b + 1..e].iter().collect();
        out.push(span);
        b = e + 1;
    }
    out
}

/// `foo()` → foo, `A::b` → b, keep dotted `Class.method` when clean.
// trace:exempt reason=internal-detail
fn normalize_mention_span(span: &str) -> Option<String> {
    let mut s = span.trim();
    if let Some(p) = s.find('(') {
        s = s[..p].trim();
    }
    if let Some(c) = s.rfind("::") {
        s = &s[c + 2..];
    }
    if s.len() < MIN_MENTION_LEN {
        return None;
    }
    if is_clean_ident(s) {
        return Some(s.to_string());
    }
    if s.contains('.') {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() >= 2 && parts.iter().all(|p| is_clean_ident(p)) {
            return Some(s.to_string());
        }
    }
    if s.contains('/') {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.iter().all(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        }) {
            return Some(s.to_string());
        }
    }
    None
}

// trace:exempt reason=internal-detail
fn is_clean_ident(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

// trace:exempt reason=internal-detail
fn is_mention_target_kind(kind: &str) -> bool {
    matches!(
        kind,
        kinds::SYMBOL
            | kinds::COMPONENT
            | kinds::ROUTE
            | kinds::CONTRACT
            | kinds::STATE
            | kinds::SCHEMA
            | kinds::FILE
            | kinds::DATA_STORE
            | kinds::DATA_ENTITY
            | kinds::TEST
            | kinds::FLOW
    )
}

// trace:exempt reason=internal-detail
fn match_rank(entity_name: &str, kind: &str, path: &str, span: &str) -> Option<u8> {
    if entity_name == span {
        return Some(0);
    }
    if entity_name.eq_ignore_ascii_case(span) {
        return Some(1);
    }
    if entity_name.ends_with(&format!(".{span}")) || entity_name.ends_with(&format!("::{span}")) {
        return Some(2);
    }
    if kind == kinds::FILE {
        let base = entity_name.rsplit('/').next().unwrap_or(entity_name);
        if base == span {
            return Some(3);
        }
        let stem = base.rsplit_once('.').map(|(s, _)| s).unwrap_or(base);
        if stem == span {
            return Some(4);
        }
        if entity_name.ends_with(&format!("/{span}")) {
            return Some(3);
        }
    }
    if kind == kinds::ROUTE {
        let slash = format!("/{span}");
        let tokens: Vec<&str> = entity_name
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '/'))
            .filter(|t| !t.is_empty())
            .collect();
        if tokens.iter().any(|t| *t == span || *t == slash.as_str()) {
            return Some(5);
        }
        if entity_name.contains(span) && span.contains('/') {
            return Some(5);
        }
    }
    if !path.is_empty() {
        let base = path.rsplit('/').next().unwrap_or(path);
        if base == span {
            return Some(4);
        }
        let stem = base.rsplit_once('.').map(|(s, _)| s).unwrap_or(base);
        if stem == span {
            return Some(5);
        }
    }
    None
}

/// Rewrite DECLARED_AS edges from every markdown file in the store.
/// Must run after source facts exist so matching sees current entities.
// trace:v1 id=impl.scc.index.write-mentions work=WORK-ripwire-lessons-phase2 satisfies=REQ-declared-mentions
pub fn write_mentions(store: &Store) -> Result<MentionStats, scc_store::StoreError> {
    let mut files: Vec<(String, String)> = store
        .all_files()?
        .into_iter()
        .filter(|(_p, _h, lang, _k, _s)| lang == "markdown")
        .map(|(p, _, _, _, _)| {
            let id = entity_id(&store.repo_id, kinds::FILE, &p);
            (p, id)
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let entities = store.all_entities()?;
    let mut stats = MentionStats::default();

    for (path, file_id) in &files {
        for id in store.relationship_ids_with_source(path, scc_core::predicates::DECLARED_AS)? {
            store.delete_relationship(&id)?;
        }
        let full = store.root.join(path);
        let Ok(content) = std::fs::read_to_string(&full) else {
            continue;
        };
        let spans = extract_markdown_backticks(&content);
        let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
        for m in spans {
            let mut ranked: Vec<(u8, String)> = Vec::new();
            for e in &entities {
                if e.id == *file_id || !is_mention_target_kind(&e.kind) {
                    continue;
                }
                let path_attr = e
                    .attributes
                    .get("file")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if let Some(rank) = match_rank(&e.name, &e.kind, path_attr, &m.span) {
                    ranked.push((rank, e.id.clone()));
                }
            }
            ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            ranked.dedup_by(|a, b| a.1 == b.1);
            if ranked.is_empty() {
                stats.unmatched += 1;
                continue;
            }
            stats.matched += 1;
            for (_rank, target) in ranked.into_iter().take(MAX_TARGETS_PER_SPAN) {
                if !seen.insert((file_id.clone(), target.clone())) {
                    continue;
                }
                let ev = {
                    let mut e = Evidence::source(
                        evidence_id(path, "mention", &m.span, m.line),
                        path,
                    );
                    e.symbol = Some(m.span.clone());
                    e.start_line = Some(m.line);
                    e.end_line = Some(m.line);
                    e.extractor = Some("scc-mentions".to_string());
                    e.extractor_version = Some(env!("CARGO_PKG_VERSION").to_string());
                    e
                };
                store.insert_evidence(&ev)?;
                let rel = Relationship::new(
                    rel_id(&["declared_as", file_id, &target]),
                    file_id.clone(),
                    scc_core::predicates::DECLARED_AS,
                    target,
                    Provenance::Declared,
                )
                .with_evidence(vec![ev.id]);
                store.insert_relationship(&rel, path)?;
            }
        }
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;
    use crate::Indexer;
    use scc_core::predicates;
    use scc_store::Store;

    #[test]
    // trace:v1 id=test.scc.index.markdown-backticks verifies=REQ-declared-mentions exercises=impl.scc.index.markdown-backticks
    fn fences_and_short_spans_are_ignored() {
        let md = "# Title\n\nSee `ok` and `handleList`.\n\n```\n`fencedIdent`\n```\n\nMore `Class.method` and `NonexistentParser`.\n";
        let spans: Vec<String> = extract_markdown_backticks(md)
            .into_iter()
            .map(|m| m.span)
            .collect();
        assert!(!spans.iter().any(|s| s == "ok"), "len<3 dropped: {spans:?}");
        assert!(spans.contains(&"handleList".to_string()), "{spans:?}");
        assert!(spans.contains(&"Class.method".to_string()), "{spans:?}");
        assert!(spans.contains(&"NonexistentParser".to_string()), "{spans:?}");
        assert!(
            !spans.iter().any(|s| s == "fencedIdent"),
            "fenced code must not emit mentions: {spans:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.declared-mentions verifies=REQ-declared-mentions exercises=impl.scc.index.write-mentions
    fn matched_backticks_become_declared_as_not_calls() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/calc.py"),
            "def handleList(a):\n    return a\n\ndef main():\n    return handleList(1)\n",
        )
        .unwrap();
        std::fs::write(
            root.join("README.md"),
            "# Calc\n\nThe entry is `handleList`. Also `NonexistentParser` and `ok`.\n\n```\n`handleList`\n```\n",
        )
        .unwrap();
        let tmp = tempfile::TempDir::new().unwrap();
        let store = Store::open(&tmp.path().join("scc.db"), root).unwrap();
        let idx = Indexer::new(store, Config::default());
        let report = idx.index().unwrap();
        assert!(report.analysis_quality.matched_doc_mentions >= 1);
        assert!(
            report.analysis_quality.unmatched_doc_mentions >= 1,
            "NonexistentParser must not become a fact: {:?}",
            report.analysis_quality
        );
        let rels = idx.store.all_relationships().unwrap();
        let declared: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == predicates::DECLARED_AS)
            .collect();
        assert!(
            !declared.is_empty(),
            "expected DECLARED_AS from README to handleList"
        );
        assert!(declared
            .iter()
            .all(|r| r.provenance == scc_core::Provenance::Declared));
        assert!(
            !declared
                .iter()
                .any(|r| r.predicate == predicates::CALLS),
            "mentions must not be CALLS"
        );
        let ents = idx.store.all_entities().unwrap();
        let names: Vec<&str> = declared
            .iter()
            .filter_map(|r| ents.iter().find(|e| e.id == r.object).map(|e| e.name.as_str()))
            .collect();
        assert!(
            names.contains(&"handleList"),
            "matched handleList, got {names:?}"
        );
        assert!(
            !names.contains(&"NonexistentParser"),
            "unmatched must not create a fact: {names:?}"
        );
        for e in &ents {
            if e.kind == kinds::FILE {
                assert!(
                    !e.attributes.contains_key("matched_doc_mentions"),
                    "gauges must not live on FILE entities"
                );
            }
        }
    }
}
