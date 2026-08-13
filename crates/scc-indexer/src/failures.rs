//! Failure-pattern detection (SCC-058): syntactic, line-based scan for
//! failure branches in source code — `except`/`catch` fallback blocks,
//! circuit-breaker mentions, and dead-letter-queue (DLQ) string literals —
//! that annotates the enclosing symbol and, for DLQs, links the symbol to a
//! `topic` entity.
//!
//! Design notes:
//! - No tree-sitter, no execution, no import tracking: purely lexical and
//!   deliberately permissive (a comment containing `except` or a
//!   `circuit_breaker` mention still yields a hit). Determinism is the
//!   contract — same content always produces the same hits and the same
//!   content-derived ids.
//! - `except`/`catch` are matched as keywords (case-sensitive, word-bounded)
//!   so `raise Exception(...)` / `exceptional()` are not false positives.
//! - DLQ string literals are redacted to their first 40 chars before being
//!   stored; the redacted literal doubles as the topic name, keeping ids
//!   content-derived and idempotent.
//! - `@retry`/`@tenacity.retry` decorators are covered by the retry
//!   extraction elsewhere and are intentionally not re-scanned here.
//! - Ids reuse the blake3 schemes from `crate::write` (rel_id / evidence_id).

use crate::write::{evidence_id, rel_id};
use scc_core::kinds;
use scc_core::{entity_id, symbol_id, Entity, Evidence, EvidenceType, Provenance, Relationship};
use scc_store::Store;
use serde_json::json;

/// One detected failure pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureHit {
    /// Repo-relative path of the file. Filled by the caller:
    /// [`scan_failures`] cannot know the path and leaves it empty.
    pub file: String,
    /// Name of the enclosing function when determinable from the content
    /// (nearest preceding `def`/`function`/`const name =` boundary);
    /// `None` for module-level code.
    pub symbol: Option<String>,
    /// `except-fallback` | `circuit-breaker` | `dlq`.
    pub kind: String,
    /// Human-readable detail. For `dlq` hits this is the quoted string
    /// literal, redacted to its first 40 chars.
    pub detail: String,
    /// 1-based line number of the pattern.
    pub line: u32,
}

/// Scan `content` for failure patterns in the given `language` (`python`,
/// `typescript`, `javascript`, `go`, or `rust`; anything else yields
/// nothing). Pure and deterministic: identical input produces identical
/// output.
// trace:v1 id=impl.scc.failures work=WORK-SCC-001 satisfies=REQ-SCC-FLOW
pub fn scan_failures(content: &str, language: &str) -> Vec<FailureHit> {
    match language {
        "go" => return scan_go(content),
        "rust" => return scan_rust(content),
        "python" | "typescript" | "javascript" | "java" => {}
        _ => return Vec::new(),
    }
    let mut hits: Vec<FailureHit> = Vec::new();
    let keyword = if language == "python" { "except" } else { "catch" };
    let mut boundary: Option<String> = None;
    for (idx, raw) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        // except / catch fallback block
        if has_keyword(raw, keyword) {
            hits.push(FailureHit {
                file: String::new(),
                symbol: boundary.clone(),
                kind: "except-fallback".to_string(),
                detail: format!("{keyword} block"),
                line: line_no,
            });
        }
        // circuit breaker identifier mention
        if let Some(ident) = circuit_identifier(raw) {
            hits.push(FailureHit {
                file: String::new(),
                symbol: boundary.clone(),
                kind: "circuit-breaker".to_string(),
                detail: ident,
                line: line_no,
            });
        }
        // DLQ string literals
        for lit in dlq_literals(raw) {
            hits.push(FailureHit {
                file: String::new(),
                symbol: boundary.clone(),
                kind: "dlq".to_string(),
                detail: lit,
                line: line_no,
            });
        }
        // A boundary on this line applies to *later* lines.
        if let Some(name) = boundary_name(raw, language) {
            boundary = Some(name);
        }
    }
    hits.sort_by(|a, b| {
        a.line
            .cmp(&b.line)
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.detail.cmp(&b.detail))
    });
    hits.dedup();
    hits
}

/// Go failure scanning: `panic(...)` calls (Go has no exception/catch
/// fallback; `defer`/`recover` handling is out of scope). Line-based like
/// the other languages; the enclosing function comes from `boundary_name`
/// (`func main`, `func (s *Store) Save`). `panic` is word-bounded so
/// `panicking` / `panicked` never match, and the `(` guard keeps bare
/// mentions in comments from firing.
fn scan_go(content: &str) -> Vec<FailureHit> {
    let mut hits: Vec<FailureHit> = Vec::new();
    let mut boundary: Option<String> = None;
    for (idx, raw) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        if has_keyword(raw, "panic") && raw.contains('(') {
            hits.push(FailureHit {
                file: String::new(),
                symbol: boundary.clone(),
                kind: "panic".to_string(),
                detail: "panic call".to_string(),
                line: line_no,
            });
        }
        // A boundary on this line applies to *later* lines.
        if let Some(name) = boundary_name(raw, "go") {
            boundary = Some(name);
        }
    }
    hits.sort_by(|a, b| {
        a.line
            .cmp(&b.line)
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.detail.cmp(&b.detail))
    });
    hits.dedup();
    hits
}
/// Rust failure scanning: `panic!`/`panic(` calls and unconditional
/// `unwrap()`/`expect(...)` calls (Rust has no `except`/`catch` fallback;
/// `?` propagation is the idiomatic error path and deliberately not
/// flagged). Line-based like the other languages; the enclosing function
/// comes from `boundary_name` (`fn name(`). `panic`/`unwrap`/`expect` are
/// word-bounded so `panicking`, `unwrap_or`, and `expected(` never match;
/// the `(`/`!` guards keep bare mentions in comments from firing.
fn scan_rust(content: &str) -> Vec<FailureHit> {
    const PANIC_PATTERNS: &[&str] = &["panic!()", "panic!(", "panic(", "unwrap()", "expect("];
    let mut hits: Vec<FailureHit> = Vec::new();
    let mut boundary: Option<String> = None;
    for (idx, raw) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        if let Some(detail) = PANIC_PATTERNS.iter().find(|p| has_keyword(raw, p)) {
            hits.push(FailureHit {
                file: String::new(),
                symbol: boundary.clone(),
                kind: "panic".to_string(),
                detail: (*detail).to_string(),
                line: line_no,
            });
        }
        // circuit breaker identifier mention
        if let Some(ident) = circuit_identifier(raw) {
            hits.push(FailureHit {
                file: String::new(),
                symbol: boundary.clone(),
                kind: "circuit-breaker".to_string(),
                detail: ident,
                line: line_no,
            });
        }
        // DLQ string literals
        for lit in dlq_literals(raw) {
            hits.push(FailureHit {
                file: String::new(),
                symbol: boundary.clone(),
                kind: "dlq".to_string(),
                detail: lit,
                line: line_no,
            });
        }
        // A boundary on this line applies to *later* lines.
        if let Some(name) = boundary_name(raw, "rust") {
            boundary = Some(name);
        }
    }
    hits.sort_by(|a, b| {
        a.line
            .cmp(&b.line)
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.detail.cmp(&b.detail))
    });
    hits.dedup();
    hits
}

pub fn apply_failures(
    store: &Store,
    file: &str,
    language: &str,
    hits: Vec<FailureHit>,
) -> Result<usize, String> {
    let _ = language;
    let repo = &store.repo_id;
    let file_id = entity_id(repo, kinds::FILE, file);
    let revision = store.meta_get("revision").ok().flatten();

    let mut hits = hits;
    hits.sort_by(|a, b| {
        a.line
            .cmp(&b.line)
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.detail.cmp(&b.detail))
    });

    let mut applied = 0usize;
    for hit in hits {
        let symbol = hit.symbol.filter(|s| !s.is_empty());
        let (subject_id, subject_kind, subject_name, subject_is_symbol) = match &symbol {
            Some(name) => (symbol_id(repo, file, name), kinds::SYMBOL, name.clone(), true),
            None => (file_id.clone(), kinds::FILE, file.to_string(), false),
        };

        // ---- evidence row (type source, extractor scc-failures) ----
        let ev_symbol = symbol.as_deref().unwrap_or(file);
        let mut ev = Evidence::source(evidence_id(file, "failure", ev_symbol, hit.line), file);
        ev.r#type = EvidenceType::Source;
        ev.symbol = Some(ev_symbol.to_string());
        ev.start_line = Some(hit.line);
        ev.extractor = Some("scc-failures".to_string());
        if let Some(rev) = &revision {
            ev.revision = Some(rev.clone());
        }
        store.insert_evidence(&ev).map_err(|e| e.to_string())?;

        // ---- subject entity: append to `failures` attribute (dedupe) ----
        let entry = json!({"kind": hit.kind, "detail": hit.detail, "line": hit.line});
        let mut entity = store.get_entity(&subject_id).map_err(|e| e.to_string())?;
        match &mut entity {
            Some(e) => {
                let mut list: Vec<serde_json::Value> = e
                    .attributes
                    .get("failures")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                if !list.contains(&entry) {
                    list.push(entry);
                }
                e.attributes
                    .insert("failures".to_string(), serde_json::Value::Array(list));
                if !e.evidence.contains(&ev.id) {
                    e.evidence.push(ev.id.clone());
                }
            }
            None => {
                let mut e = Entity::new(subject_id.clone(), subject_kind, subject_name);
                if subject_is_symbol {
                    e.attr("kind", json!("function"));
                    e.attr("file", json!(file));
                }
                e.attr("failures", json!([entry]));
                e.evidence.push(ev.id.clone());
                entity = Some(e);
            }
        }
        let entity = entity.as_ref().expect("entity set above");
        store
            .insert_entity(entity, std::slice::from_ref(&file.to_string()))
            .map_err(|e| e.to_string())?;

        // ---- DLQ: topic entity + subscribes relationship ----
        if hit.kind == "dlq" && !hit.detail.is_empty() {
            let topic_id = entity_id(repo, kinds::TOPIC, &hit.detail);
            match store.get_entity(&topic_id).map_err(|e| e.to_string())? {
                Some(mut t) => {
                    t.attributes
                        .insert("dlq".to_string(), serde_json::Value::Bool(true));
                    store
                        .insert_entity(&t, std::slice::from_ref(&file.to_string()))
                        .map_err(|e| e.to_string())?;
                }
                None => {
                    let mut t = Entity::new(topic_id.clone(), kinds::TOPIC, hit.detail.clone());
                    t.attr("dlq", json!(true));
                    store
                        .insert_entity(&t, std::slice::from_ref(&file.to_string()))
                        .map_err(|e| e.to_string())?;
                }
            }
            let rel = Relationship::new(
                rel_id(&["subscribes", &subject_id, &topic_id]),
                subject_id,
                scc_core::predicates::SUBSCRIBES,
                topic_id,
                Provenance::Extracted,
            )
            .with_evidence(vec![ev.id.clone()]);
            store
                .insert_relationship(&rel, file)
                .map_err(|e| e.to_string())?;
        }

        applied += 1;
    }
    Ok(applied)
}

// ---------------------------------------------------------------------------
// scanning
// ---------------------------------------------------------------------------

/// True when `kw` appears in `line` as a word: preceded and followed by a
/// non-identifier char (or the line edge). Case-sensitive, so `Exception`
/// never matches `except`.
fn has_keyword(line: &str, kw: &str) -> bool {
    let mut from = 0;
    while let Some(rel) = line[from..].find(kw) {
        let pos = from + rel;
        let before = pos == 0 || !is_ident_char(line.as_bytes()[pos - 1] as char);
        let after_pos = pos + kw.len();
        let after = after_pos >= line.len() || !is_ident_char(line.as_bytes()[after_pos] as char);
        if before && after {
            return true;
        }
        from = after_pos;
    }
    false
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// First identifier mentioning `circuit` (case-insensitive) on the line,
/// e.g. `circuit_breaker`, `CircuitBreaker`, `circuitbreaker`.
fn circuit_identifier(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let mut from = 0;
    while let Some(rel) = lower[from..].find("circuit") {
        let pos = from + rel;
        let bytes = line.as_bytes();
        let mut start = pos;
        while start > 0 && is_ident_char(bytes[start - 1] as char) {
            start -= 1;
        }
        let mut end = pos + "circuit".len();
        while end < bytes.len() && is_ident_char(bytes[end] as char) {
            end += 1;
        }
        if start < end {
            return Some(line[start..end].to_string());
        }
        from = pos + "circuit".len();
    }
    None
}

/// Quoted string literals on `line` whose content mentions `dlq`,
/// `dead-letter`, or `dead_letter` (case-insensitive). Returns the literal
/// contents, each redacted to its first 40 chars. Escapes are not
/// interpreted (kept simple and deterministic); an unterminated quote ends
/// the scan of the line.
fn dlq_literals(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut it = line.char_indices().peekable();
    while let Some((_i, c)) = it.next() {
        if c != '\'' && c != '"' {
            continue;
        }
        let quote = c;
        let mut content = String::new();
        let mut closed = false;
        for (_j, c2) in it.by_ref() {
            if c2 == quote {
                closed = true;
                break;
            }
            content.push(c2);
        }
        if !closed {
            break; // unterminated quote: no further literals on this line
        }
        let low = content.to_ascii_lowercase();
        if low.contains("dlq") || low.contains("dead-letter") || low.contains("dead_letter") {
            out.push(redact(&content));
        }
    }
    out
}

/// First 40 chars of `s`; longer strings are truncated (deterministic).
fn redact(s: &str) -> String {
    s.chars().take(40).collect()
}

/// Name of the function declared on `line`, if any. Python matches
/// `def`/`async def`; TS/JS matches `function` (with optional `export`/
/// `async` prefixes) and `const name =`; Go matches `func name(` and
/// `func (r Receiver) name(` (method symbol `Receiver.name`, matching the
/// extractor's naming so failures land on the real symbol entity).
fn boundary_name(line: &str, language: &str) -> Option<String> {
    if language == "go" {
        return go_boundary_name(line);
    }
    if language == "java" {
        return java_boundary_name(line);
    }
    if language == "rust" {
        return rust_boundary_name(line);
    }
    let t = line.trim_start();
    let rest: &str = if language == "python" {
        if let Some(r) = t.strip_prefix("async def ") {
            r
        } else { t.strip_prefix("def ")? }
    } else {
        let mut r = t;
        if let Some(x) = r.strip_prefix("export ") {
            r = x;
        }
        if let Some(x) = r.strip_prefix("async ") {
            r = x;
        }
        if let Some(x) = r.strip_prefix("function ") {
            x
        } else {
            let x = r.strip_prefix("const ")?;
            let name = take_ident(x);
            if name.is_empty() {
                return None;
            }
            let after = &x[name.len()..].trim_start();
            if after.starts_with('=') {
                return Some(name.to_string());
            }
            return None;
        }
    };
    let name = take_ident(rest);
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

/// Go function boundary: `func main(` → `main`; `func (s *Store) Save(` →
/// `Store.Save` (receiver type's last dotted segment, matching the Go
/// extractor's method naming).
fn go_boundary_name(line: &str) -> Option<String> {
    let t = line.trim_start();
    let rest = t.strip_prefix("func ")?;
    let rest = rest.trim_start();
    if let Some(inner) = rest.strip_prefix('(') {
        let close = inner.find(')')?;
        let after = inner[close + 1..].trim_start();
        let name = take_ident(after);
        if name.is_empty() {
            return None;
        }
        match receiver_type(&inner[..close]) {
            Some(rt) if !rt.is_empty() => Some(format!("{rt}.{name}")),
            _ => Some(name.to_string()),
        }
    } else {
        let name = take_ident(rest);
        if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        }
    }
}

/// Rust function boundary: `fn name(` → `name`. Strips visibility
/// (`pub`, `pub(crate)`), `async`, `const`, `unsafe`, and `extern`
/// modifiers.
fn rust_boundary_name(line: &str) -> Option<String> {
    let t = line.trim_start();
    let mut r = t;
    loop {
        if let Some(x) = r.strip_prefix("pub ") {
            r = x;
            continue;
        }
        if r.starts_with("pub(") {
            let Some(paren) = r.find(')') else { break };
            r = r[paren + 1..].trim_start();
            continue;
        }
        if let Some(x) = r.strip_prefix("async ") {
            r = x;
            continue;
        }
        if let Some(x) = r.strip_prefix("const ") {
            r = x;
            continue;
        }
        if let Some(x) = r.strip_prefix("unsafe ") {
            r = x;
            continue;
        }
        if let Some(x) = r.strip_prefix("extern ") {
            r = x;
            continue;
        }
        // `extern "C" fn` — skip the ABI string literal
        if r.starts_with('"') {
            if let Some(end) = r[1..].find('"') {
                r = r[end + 2..].trim_start();
                continue;
            }
        }
        break;
    }
    let rest = r.strip_prefix("fn ")?;
    let name = take_ident(rest);
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Receiver type from inside `(s *Service)`: strips the name, `*`/`&`, and
/// any package qualifier, keeping the last dotted segment.
fn receiver_type(recv: &str) -> Option<String> {
    let t = recv.trim();
    let last = t.split_whitespace().last()?;
    let last = last.trim_start_matches(['*', '&']);
    let seg = last.rsplit('.').next().unwrap_or(last);
    let seg = seg.split('[').next().unwrap_or(seg).trim();
    if seg.is_empty() {
        None
    } else {
        Some(seg.to_string())
    }
}

/// Java method declarations: `public void storeOrder(String id) {`,
/// `Order findById(String id) {`, `public Service() {`. The method name is
/// the identifier immediately before the first `(` on a line whose
/// (modifier-stripped) remainder opens a block.
fn java_boundary_name(line: &str) -> Option<String> {
    let mut r = line.trim_start();
    loop {
        let stripped = strip_java_modifier(r);
        if stripped == r {
            break;
        }
        r = stripped;
    }
    let r = r.trim_start();
    if !r.ends_with('{') {
        return None;
    }
    let paren = r.find('(')?;
    if paren == 0 {
        return None;
    }
    let before = &r[..paren];
    let name = take_last_ident(before);
    if name.is_empty() {
        return None;
    }
    // control-flow openers with parenthesized conditions are not methods
    if matches!(
        name,
        "if" | "for" | "while" | "switch" | "catch" | "return" | "new" | "else" | "synchronized"
    ) {
        return None;
    }
    Some(name.to_string())
}

/// Strip one leading Java modifier keyword (if followed by whitespace).
fn strip_java_modifier(s: &str) -> &str {
    const MODIFIERS: &[&str] = &[
        "public", "private", "protected", "static", "final", "abstract", "synchronized",
        "native", "strictfp", "default", "transient", "volatile",
    ];
    let t = s.trim_start();
    for m in MODIFIERS {
        if let Some(r) = t.strip_prefix(m) {
            if r.starts_with(char::is_whitespace) {
                return r.trim_start();
            }
        }
    }
    t
}

/// Last identifier in `s` (e.g. `public void storeOrder` -> `storeOrder`).
fn take_last_ident(s: &str) -> &str {
    s.rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
        .find(|w| !w.is_empty() && !w.starts_with(|c: char| c.is_ascii_digit()))
        .unwrap_or("")
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
    fn python_except_circuit_dlq_with_enclosing_symbol() {
        let content = r#"
import tenacity

@tenacity.retry(stop=tenacity.stop_after_attempt(3))
def fetch(url):
    try:
        return http_get(url)
    except TimeoutError:
        return None

def publish(msg):
    if circuit_breaker.is_open:
        return
    queue.send(msg, dlq="orders-dlq")
"#;
        let hits = scan_failures(content, "python");
        // except + circuit + dlq; the retry decorator lines yield nothing
        assert_eq!(hits.len(), 3, "hits: {hits:?}");
        let by_line: std::collections::BTreeMap<_, _> = hits
            .into_iter()
            .map(|h| (h.line, h))
            .collect();

        let exc = &by_line[&8];
        assert_eq!(exc.kind, "except-fallback");
        assert_eq!(exc.detail, "except block");
        assert_eq!(exc.symbol.as_deref(), Some("fetch"));

        let cb = &by_line[&12];
        assert_eq!(cb.kind, "circuit-breaker");
        assert_eq!(cb.detail, "circuit_breaker");
        assert_eq!(cb.symbol.as_deref(), Some("publish"));

        let dlq = &by_line[&14];
        assert_eq!(dlq.kind, "dlq");
        assert_eq!(dlq.detail, "orders-dlq");
        assert_eq!(dlq.symbol.as_deref(), Some("publish"));
    }

    #[test]
    fn python_top_level_except_no_symbol() {
        let content = "try:\n    setup()\nexcept Exception:\n    pass\n";
        let hits = scan_failures(content, "python");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 3);
        assert!(hits[0].symbol.is_none());
    }

    #[test]
    fn ts_catch_circuit_dlq() {
        let content = r#"
export async function handler(event: any) {
  try {
    await consume(event);
  } catch (err) {
    return { error: err };
  }
}

const producer = () => {
  if (circuit.isOpen()) return;
  queue.publish({ dlq: "events-dead-letter" });
};
"#;
        let hits = scan_failures(content, "typescript");
        assert_eq!(hits.len(), 3, "hits: {hits:?}");
        let by_line: std::collections::BTreeMap<_, _> = hits
            .into_iter()
            .map(|h| (h.line, h))
            .collect();

        let exc = &by_line[&5];
        assert_eq!(exc.kind, "except-fallback");
        assert_eq!(exc.detail, "catch block");
        assert_eq!(exc.symbol.as_deref(), Some("handler"));

        let cb = &by_line[&11];
        assert_eq!(cb.kind, "circuit-breaker");
        assert_eq!(cb.detail, "circuit");
        assert_eq!(cb.symbol.as_deref(), Some("producer"));

        let dlq = &by_line[&12];
        assert_eq!(dlq.kind, "dlq");
        assert_eq!(dlq.detail, "events-dead-letter");
        assert_eq!(dlq.symbol.as_deref(), Some("producer"));

        let js = scan_failures(content, "javascript");
        assert_eq!(js.len(), 3);
    }

    #[test]
    fn ts_plain_function_boundary() {
        let content = "function retry() {\n  try {\n    work();\n  } catch (e) {\n    log(e);\n  }\n}\n";
        let hits = scan_failures(content, "typescript");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].symbol.as_deref(), Some("retry"));
    }

    #[test]
    fn go_panic_with_enclosing_symbol() {
        let content = r#"
package main

func main() {
    if err := run(); err != nil {
        panic("run failed")
    }
}

func (s *Store) Save(order string) error {
    if order == "" {
        panic("empty order")
    }
    return nil
}
"#;
        let hits = scan_failures(content, "go");
        assert_eq!(hits.len(), 2, "hits: {hits:?}");
        assert_eq!(hits[0].kind, "panic");
        assert_eq!(hits[0].detail, "panic call");
        assert_eq!(hits[0].symbol.as_deref(), Some("main"));
        assert_eq!(hits[0].line, 6);

        assert_eq!(hits[1].symbol.as_deref(), Some("Store.Save"));
        assert_eq!(hits[1].line, 12);

        // `panicking` is not `panic`; a bare comment mention with `(` still
        // yields a hit (permissive by design), so test the word boundary
        // separately.
        assert!(scan_failures("func f() {\n    panicking()\n}\n", "go").is_empty());
        // non-go languages never scan for panic (rust scans its own panic!(
        // pattern; use a language without any panic scan here)
        assert!(scan_failures("func main() { panic(\"x\") }\n", "python").is_empty());
    }

    #[test]
    fn rust_panic_unwrap_expect_with_enclosing_symbol() {
        let content = r#"
pub fn main() {
    let conn = "jobs.db";
    conn.execute("INSERT INTO jobs (id) VALUES (?)", &["1"]).expect("store write failed");
}

fn fallback() {
    panic!("queue dead");
}

async fn poll() {
    let v = Option::<i32>::None.unwrap();
}
"#;
        let hits = scan_failures(content, "rust");
        assert_eq!(hits.len(), 3, "hits: {hits:?}");
        assert_eq!(hits[0].kind, "panic");
        assert_eq!(hits[0].detail, "expect(");
        assert_eq!(hits[0].symbol.as_deref(), Some("main"));
        assert_eq!(hits[0].line, 4);

        assert_eq!(hits[1].kind, "panic");
        assert_eq!(hits[1].detail, "panic!(");
        assert_eq!(hits[1].symbol.as_deref(), Some("fallback"));
        assert_eq!(hits[1].line, 8);

        assert_eq!(hits[2].kind, "panic");
        assert_eq!(hits[2].detail, "unwrap()");
        assert_eq!(hits[2].symbol.as_deref(), Some("poll"));
        assert_eq!(hits[2].line, 12);

        // plain `?` propagation is idiomatic and never flagged
        assert!(
            scan_failures("fn load() -> Result<(), String> {\n    read()?;\n    Ok(())\n}\n", "rust")
                .is_empty()
        );
        // `unwrap_or`, `expected(`, and `panicking` are not failure calls
        let neg = "fn f() {\n    let a = v.unwrap_or(0);\n    expected(1);\n    panicking();\n}\n";
        assert!(scan_failures(neg, "rust").is_empty());
    }

    #[test]
    fn rust_boundary_visibility_forms() {
        let content = "pub(crate) async fn send() {\n    panic!(\"x\")\n}\n\npub fn recv() {\n    panic!(\"y\")\n}\n\nunsafe extern \"C\" fn raw() {\n    panic!(\"z\")\n}\n";
        let hits = scan_failures(content, "rust");
        assert_eq!(hits.len(), 3, "hits: {hits:?}");
        assert_eq!(hits[0].symbol.as_deref(), Some("send"));
        assert_eq!(hits[1].symbol.as_deref(), Some("recv"));
        assert_eq!(hits[2].symbol.as_deref(), Some("raw"));
    }

    #[test]
    fn java_catch_with_enclosing_method() {
        let content = r#"
public class Service {
    public void process(String orderId) {
        try {
            this.storeOrder(orderId);
        } catch (Exception e) {
            this.fallback(orderId);
        }
    }
    public void storeOrder(String orderId) {}
    public void fallback(String orderId) {}
}
"#;
        let hits = scan_failures(content, "java");
        assert_eq!(hits.len(), 1, "hits: {hits:?}");
        assert_eq!(hits[0].kind, "except-fallback");
        assert_eq!(hits[0].detail, "catch block");
        assert_eq!(hits[0].symbol.as_deref(), Some("process"));
        assert_eq!(hits[0].line, 6);
        // constructor boundary resolves to the class name, not a method
        let ctor = r#"
public class Service {
    public Service() {
        try {
            setup();
        } catch (Exception e) {
            throw new RuntimeException(e);
        }
    }
}
"#;
        let hits = scan_failures(ctor, "java");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].symbol.as_deref(), Some("Service"));
        // control-flow openers are never method boundaries
        let flow = r#"
public void loop() {
    for (int i = 0; i < 3; i++) {
        try {
            work();
        } catch (Exception e) {
            retry();
        }
    }
}
"#;
        let hits = scan_failures(flow, "java");
        assert_eq!(hits.len(), 1, "hits: {hits:?}");
        assert_eq!(hits[0].symbol.as_deref(), Some("loop"));
    }

    #[test]
    fn negative_cases() {
        // `Exception` (capitalized) and `exceptional` are not the `except` keyword
        assert!(scan_failures("raise Exception('boom')\n", "python").is_empty());
        assert!(scan_failures("x = exceptional_value\n", "python").is_empty());
        // a comment mentioning except without the keyword
        assert!(scan_failures("# retry on failure\n", "python").is_empty());
        // no dlq markers in plain strings
        assert!(scan_failures("queue.send(msg)\n", "python").is_empty());
        assert!(scan_failures("send('payload')\n", "python").is_empty());
        // `catches` is not the `catch` keyword
        assert!(scan_failures("function f() { catches(e); }\n", "typescript").is_empty());
        // unsupported languages are skipped entirely
        assert!(scan_failures("try:\n    pass\nexcept:\n    pass\n", "ruby").is_empty());
        assert!(scan_failures("try {} catch (e) {}", "csharp").is_empty());
    }

    #[test]
    fn dlq_redaction_and_markers() {
        // long literal redacted to 40 chars
        let long = format!("\"{}\"", "x".repeat(100));
        let content = format!("queue.send({long})\n");
        let hits = scan_failures(&content, "python");
        assert!(hits.is_empty(), "no dlq marker -> no hit");
        // with a marker, detail is truncated to 40 chars
        let content = format!("queue.send(\"dead-letter-{}\")\n", "y".repeat(60));
        let hits = scan_failures(&content, "python");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].detail.len(), 40);
        assert_eq!(hits[0].detail, format!("dead-letter-{}", "y".repeat(28)));
        // dead_letter and dlq markers
        assert_eq!(
            scan_failures("q = send('my_dead_letter')\n", "python")[0].kind,
            "dlq"
        );
        assert_eq!(
            scan_failures("q = send(\"DLQ-retry\")\n", "python")[0].kind,
            "dlq"
        );
    }

    #[test]
    fn malformed_input_no_panic() {
        for lang in ["python", "typescript", "javascript", "java", "rust", ""] {
            assert!(scan_failures("", lang).is_empty());
            // binary junk
            let junk = String::from_utf8_lossy(&[0x00u8, 0x01, 0xff, 0xfe, b'a', 0x80]).to_string();
            assert!(scan_failures(&junk, lang).is_empty());
            // unterminated quote with a dlq marker inside
            assert!(scan_failures("x = \"dlq", lang).is_empty());
            // a very long single line
            let long = "a".repeat(100_000);
            assert!(scan_failures(&long, lang).is_empty());
            // bare keywords without any surrounding structure
            let _ = scan_failures("except catch circuit dlq dead-letter\n", lang);
        }
    }

    #[test]
    fn apply_creates_attributes_topic_and_evidence() {
        let (store, _t) = store_for();
        let file = "src/worker.py";
        let content = "def consume():\n    try:\n        pull()\n    except Exception:\n        pass\n    queue.send(dlq=\"orders-dlq\")\n";
        let hits = scan_failures(content, "python");
        assert_eq!(hits.len(), 2, "except + dlq: {hits:?}");
        let n = apply_failures(&store, file, "python", hits).unwrap();
        assert_eq!(n, 2);

        // symbol entity created synthetically with the failures attribute
        let sym_id = scc_core::symbol_id(&store.repo_id, file, "consume");
        let e = store.get_entity(&sym_id).unwrap().expect("synthetic symbol");
        assert_eq!(e.kind, kinds::SYMBOL);
        assert_eq!(e.attributes["kind"], serde_json::json!("function"));
        assert_eq!(e.attributes["file"], serde_json::json!(file));
        let failures = e.attributes["failures"].as_array().unwrap();
        assert_eq!(failures.len(), 2);
        let kinds: Vec<&str> = failures
            .iter()
            .map(|f| f["kind"].as_str().unwrap())
            .collect();
        assert!(kinds.contains(&"except-fallback"));
        assert!(kinds.contains(&"dlq"));
        let exc = failures
            .iter()
            .find(|f| f["kind"] == "except-fallback")
            .unwrap();
        assert_eq!(exc["detail"], "except block");
        assert_eq!(exc["line"], 4);

        // topic entity with {dlq: true}
        let topic_id = scc_core::entity_id(&store.repo_id, kinds::TOPIC, "orders-dlq");
        let t = store.get_entity(&topic_id).unwrap().expect("topic created");
        assert_eq!(t.attributes["dlq"], serde_json::json!(true));

        // subscribes relationship: symbol -> topic, EXTRACTED
        let rel = store
            .all_relationships()
            .unwrap()
            .into_iter()
            .find(|r| r.predicate == scc_core::predicates::SUBSCRIBES)
            .expect("subscribes rel");
        assert_eq!(rel.subject, sym_id);
        assert_eq!(rel.object, topic_id);
        assert_eq!(rel.provenance, scc_core::Provenance::Extracted);
        assert_eq!(rel.confidence, 1.0);
        assert_eq!(rel.evidence.len(), 1);

        // evidence rows: type source, extractor scc-failures
        let evs = store.evidence_for_path(file).unwrap();
        let fail_evs: Vec<_> = evs
            .iter()
            .filter(|e| e.extractor.as_deref() == Some("scc-failures"))
            .collect();
        assert_eq!(fail_evs.len(), 2);
        for ev in &fail_evs {
            assert_eq!(ev.r#type, scc_core::EvidenceType::Source);
            assert_eq!(ev.symbol.as_deref(), Some("consume"));
        }
        assert!(e.evidence.contains(&fail_evs[0].id));
    }

    #[test]
    fn apply_idempotent_and_preserves_existing_attributes() {
        let (store, _t) = store_for();
        let file = "src/worker.py";
        let content = "def consume():\n    try:\n        pull()\n    except Exception:\n        pass\n";
        let hits = scan_failures(content, "python");
        apply_failures(&store, file, "python", hits.clone()).unwrap();
        apply_failures(&store, file, "python", hits).unwrap();

        let sym_id = scc_core::symbol_id(&store.repo_id, file, "consume");
        let e = store.get_entity(&sym_id).unwrap().unwrap();
        let failures = e.attributes["failures"].as_array().unwrap();
        assert_eq!(failures.len(), 1, "deduped across applies");
        let rels = store
            .all_relationships()
            .unwrap()
            .into_iter()
            .filter(|r| r.predicate == scc_core::predicates::SUBSCRIBES)
            .count();
        assert_eq!(rels, 0, "no dlq hits -> no topic rels");

        // pre-existing symbol entity keeps its attributes and evidence
        let (store2, _t2) = store_for();
        let file2 = "src/a.py";
        let mut se = scc_core::Entity::new(
            scc_core::symbol_id(&store2.repo_id, file2, "run"),
            kinds::SYMBOL,
            "run",
        );
        se.attr("kind", serde_json::json!("function"));
        se.attr("exported", serde_json::json!(true));
        store2
            .insert_entity(&se, std::slice::from_ref(&file2.to_string()))
            .unwrap();
        let content2 = "def run():\n    try:\n        x()\n    except:\n        pass\n";
        let hits2 = scan_failures(content2, "python");
        apply_failures(&store2, file2, "python", hits2).unwrap();
        let e2 = store2.get_entity(&se.id).unwrap().unwrap();
        assert_eq!(e2.attributes["exported"], serde_json::json!(true));
        assert_eq!(e2.attributes["failures"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn apply_no_symbol_uses_file_subject() {
        let (store, _t) = store_for();
        let file = "top.py";
        let content = "try:\n    setup()\nexcept Exception:\n    pass\n";
        let hits = scan_failures(content, "python");
        apply_failures(&store, file, "python", hits).unwrap();
        let e = store
            .get_entity(&scc_core::entity_id(&store.repo_id, kinds::FILE, file))
            .unwrap()
            .expect("file entity used as subject");
        assert_eq!(e.attributes["failures"].as_array().unwrap().len(), 1);
    }
}
