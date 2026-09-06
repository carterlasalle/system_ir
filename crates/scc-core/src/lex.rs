//! Relevance-first lexical machinery: subtoken tokenizer, BM25, query shape.
//!
//! This module is a *lens*, not the production ranker. PageRank answers
//! structural importance; BM25 answers "does this text match the query".
//! Those scores stay separately inspectable. Do not fuse them here.

use serde::{Deserialize, Serialize};

/// Robertson/Sparck Jones BM25 term saturation.
pub const BM25_K1: f64 = 1.5;
/// BM25 length normalization.
pub const BM25_B: f64 = 0.75;
/// Identifier / symbol-name field weight (Ripwire name=3).
pub const WEIGHT_NAME: u32 = 3;
/// File/path field weight.
pub const WEIGHT_PATH: u32 = 2;
/// Doc-comment field weight (Ripwire doc=2).
pub const WEIGHT_DOC: u32 = 2;
/// Body / signature / callee vocabulary weight (Ripwire body=1).
pub const WEIGHT_BODY: u32 = 1;
/// Subtokens shorter than this are dropped (Ripwire ≥2-byte rule).
pub const MIN_SUBTOKEN_LEN: usize = 2;

/// One tokenizer for query and documents. Port of Ripwire `forEachLexSubtoken`:
/// alphanumeric runs, split on camelCase and ACRONYMWord (`HTTPServer` →
/// `HTTP`+`Server`), keep all-caps acronyms (`MCP` → `mcp`).
// trace:v1 id=impl.scc.core.subtokens work=WORK-ripwire-lessons-phase2 satisfies=REQ-subtoken-bm25
pub fn subtokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for_each_lex_subtoken(text, |start, end| {
        if end.saturating_sub(start) < MIN_SUBTOKEN_LEN {
            return;
        }
        let tok: String = text.as_bytes()[start..end]
            .iter()
            .map(|b| lex_lower_byte(*b) as char)
            .collect();
        out.push(tok);
    });
    out
}

// trace:exempt reason=internal-detail
fn lex_lower_byte(c: u8) -> u8 {
    if c.is_ascii_uppercase() {
        c - b'A' + b'a'
    } else {
        c
    }
}

// trace:exempt reason=internal-detail
fn lex_upper_opens_token(text: &[u8], k: usize, prev_upper: bool) -> bool {
    let next = if k + 1 < text.len() { text[k + 1] } else { 0 };
    !prev_upper || next.is_ascii_lowercase()
}

// trace:exempt reason=internal-detail
fn for_each_lex_subtoken(text: &str, mut emit: impl FnMut(usize, usize)) {
    let bytes = text.as_bytes();
    let mut tok_start: Option<usize> = None;
    let mut prev_upper = false;
    for k in 0..bytes.len() {
        let c = bytes[k];
        let upper = c.is_ascii_uppercase();
        let lower = c.is_ascii_lowercase();
        let digit = c.is_ascii_digit();
        if !upper && !lower && !digit {
            if let Some(s) = tok_start.take() {
                emit(s, k);
            }
            prev_upper = false;
            continue;
        }
        if upper {
            if let Some(s) = tok_start {
                if lex_upper_opens_token(bytes, k, prev_upper) {
                    emit(s, k);
                    tok_start = Some(k);
                }
            }
        }
        if tok_start.is_none() {
            tok_start = Some(k);
        }
        prev_upper = upper;
    }
    if let Some(s) = tok_start {
        emit(s, bytes.len());
    }
}

/// One weighted text field of a BM25 document.
#[derive(Debug, Clone)]
// trace:exempt reason=internal-detail
pub struct LexField {
    pub text: String,
    pub weight: u32,
}

/// A document scored by the BM25 lens (symbol, component, route, …).
#[derive(Debug, Clone)]
// trace:exempt reason=internal-detail
pub struct LexDoc {
    pub id: String,
    pub fields: Vec<LexField>,
}

// trace:exempt reason=internal-detail
impl LexDoc {
    // trace:exempt reason=internal-detail
    pub fn from_parts(
        id: impl Into<String>,
        name: &str,
        path: &str,
        doc: &str,
        body: &str,
    ) -> Self {
        LexDoc {
            id: id.into(),
            fields: vec![
                LexField {
                    text: name.to_string(),
                    weight: WEIGHT_NAME,
                },
                LexField {
                    text: path.to_string(),
                    weight: WEIGHT_PATH,
                },
                LexField {
                    text: doc.to_string(),
                    weight: WEIGHT_DOC,
                },
                LexField {
                    text: body.to_string(),
                    weight: WEIGHT_BODY,
                },
            ],
        }
    }
}

/// BM25 scores, one per document, in input order. Empty query → all zeros.
// trace:v1 id=impl.scc.core.bm25 work=WORK-ripwire-lessons-phase2 satisfies=REQ-subtoken-bm25
pub fn bm25_scores(query: &str, docs: &[LexDoc]) -> Vec<f64> {
    let q_toks = subtokens(query);
    let n = docs.len();
    if q_toks.is_empty() || n == 0 {
        return vec![0.0; n];
    }

    // unique query terms in first-seen order (BTreeMap would scramble)
    let mut unique: Vec<String> = Vec::new();
    let mut q_index: Vec<usize> = Vec::with_capacity(q_toks.len());
    for t in &q_toks {
        if let Some(i) = unique.iter().position(|u| u == t) {
            q_index.push(i);
        } else {
            q_index.push(unique.len());
            unique.push(t.clone());
        }
    }
    let u_count = unique.len();

    let mut dl = vec![0u32; n];
    let mut tf = vec![0u32; n * u_count];
    for (i, doc) in docs.iter().enumerate() {
        for field in &doc.fields {
            if field.weight == 0 {
                continue;
            }
            for tok in subtokens(&field.text) {
                dl[i] = dl[i].saturating_add(field.weight);
                if let Some(u) = unique.iter().position(|t| t == &tok) {
                    tf[i * u_count + u] = tf[i * u_count + u].saturating_add(field.weight);
                }
            }
        }
    }

    let avgdl = if n == 0 {
        1.0
    } else {
        dl.iter().map(|d| *d as f64).sum::<f64>() / n as f64
    };
    let avgdl = if avgdl > 0.0 { avgdl } else { 1.0 };

    let mut df = vec![0u32; u_count];
    for i in 0..n {
        for u in 0..u_count {
            if tf[i * u_count + u] > 0 {
                df[u] += 1;
            }
        }
    }

    let mut scores = vec![0.0; n];
    for i in 0..n {
        let mut sc = 0.0;
        // one contribution per query occurrence (Ripwire: duplicates still add)
        for &u in &q_index {
            let term_tf = tf[i * u_count + u] as f64;
            if term_tf == 0.0 {
                continue;
            }
            let n_df = df[u] as f64;
            let idf = ((n as f64 - n_df + 0.5) / (n_df + 0.5) + 1.0).ln();
            let denom = term_tf + BM25_K1 * (1.0 - BM25_B + BM25_B * (dl[i] as f64) / avgdl);
            sc += idf * (term_tf * (BM25_K1 + 1.0)) / denom;
        }
        scores[i] = sc;
    }
    scores
}

/// Persisted corpus-wide BM25 statistics (Ripwire warm lexindex). IDF and
/// average length come from the full indexed corpus, not the candidate slice.
/// Stored in store meta (`bm25_corpus`), never on FILE entity attributes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
// trace:exempt reason=internal-detail
pub struct Bm25CorpusStats {
    pub n: u32,
    pub avgdl: f64,
    /// Term → document frequency (docs containing the term at least once).
    pub df: std::collections::BTreeMap<String, u32>,
    /// Document id → weighted length.
    pub dl: std::collections::BTreeMap<String, u32>,
}

// trace:exempt reason=internal-detail
impl Bm25CorpusStats {
    /// Build corpus-wide stats. Document order does not affect `df`/`dl`
    /// maps; `n` is `docs.len()`.
    // trace:v1 id=impl.scc.core.bm25-persist work=WORK-ripwire-lessons-phase2 satisfies=REQ-bm25-persist
    pub fn from_docs(docs: &[LexDoc]) -> Self {
        let mut df: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
        let mut dl: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
        for doc in docs {
            let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            let mut len = 0u32;
            for field in &doc.fields {
                if field.weight == 0 {
                    continue;
                }
                for tok in subtokens(&field.text) {
                    len = len.saturating_add(field.weight);
                    seen.insert(tok);
                }
            }
            dl.insert(doc.id.clone(), len);
            for tok in seen {
                *df.entry(tok).or_insert(0) += 1;
            }
        }
        let n = docs.len() as u32;
        let avgdl = if n == 0 {
            1.0
        } else {
            let sum: f64 = dl.values().map(|d| *d as f64).sum();
            let a = sum / n as f64;
            if a > 0.0 {
                a
            } else {
                1.0
            }
        };
        Bm25CorpusStats { n, avgdl, df, dl }
    }
}

/// BM25 using persisted corpus IDF/`avgdl`. Term frequencies still come from
/// `docs`. When `docs` is the same corpus `stats` was built from, scores match
/// [`bm25_scores`].
// trace:exempt reason=internal-detail
pub fn bm25_scores_with_stats(query: &str, docs: &[LexDoc], stats: &Bm25CorpusStats) -> Vec<f64> {
    let q_toks = subtokens(query);
    let n_docs = docs.len();
    if q_toks.is_empty() || n_docs == 0 || stats.n == 0 {
        return vec![0.0; n_docs];
    }
    let mut unique: Vec<String> = Vec::new();
    let mut q_index: Vec<usize> = Vec::with_capacity(q_toks.len());
    for t in &q_toks {
        if let Some(i) = unique.iter().position(|u| u == t) {
            q_index.push(i);
        } else {
            q_index.push(unique.len());
            unique.push(t.clone());
        }
    }
    let u_count = unique.len();
    let mut dl = vec![0u32; n_docs];
    let mut tf = vec![0u32; n_docs * u_count];
    for (i, doc) in docs.iter().enumerate() {
        dl[i] = stats.dl.get(&doc.id).copied().unwrap_or(0);
        for field in &doc.fields {
            if field.weight == 0 {
                continue;
            }
            for tok in subtokens(&field.text) {
                if !stats.dl.contains_key(&doc.id) {
                    dl[i] = dl[i].saturating_add(field.weight);
                }
                if let Some(u) = unique.iter().position(|t| t == &tok) {
                    tf[i * u_count + u] = tf[i * u_count + u].saturating_add(field.weight);
                }
            }
        }
    }
    let avgdl = if stats.avgdl > 0.0 { stats.avgdl } else { 1.0 };
    let n = stats.n as f64;
    let mut scores = vec![0.0; n_docs];
    for i in 0..n_docs {
        let mut sc = 0.0;
        let doc_len = if dl[i] == 0 { 1.0 } else { dl[i] as f64 };
        for &u in &q_index {
            let term_tf = tf[i * u_count + u] as f64;
            if term_tf == 0.0 {
                continue;
            }
            let n_df = stats.df.get(&unique[u]).copied().unwrap_or(0) as f64;
            let idf = ((n - n_df + 0.5) / (n_df + 0.5) + 1.0).ln();
            let denom = term_tf + BM25_K1 * (1.0 - BM25_B + BM25_B * doc_len / avgdl);
            sc += idf * (term_tf * (BM25_K1 + 1.0)) / denom;
        }
        scores[i] = sc;
    }
    scores
}

/// Rank documents by BM25 descending, id ascending. Deterministic.
// trace:exempt reason=internal-detail
pub fn bm25_rank(query: &str, docs: &[LexDoc]) -> Vec<(String, f64)> {
    let scores = bm25_scores(query, docs);
    let mut pairs: Vec<(String, f64)> = docs
        .iter()
        .zip(scores)
        .map(|(d, s)| (d.id.clone(), s))
        .collect();
    pairs.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    pairs
}

/// Query shape from text alone. Conservative: under-fire rather than
/// demote documents for a prose question that merely mentions a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
// trace:exempt reason=internal-detail
pub enum QueryShape {
    Identifier,
    SymbolLike,
    PathLike,
    StackTrace,
    ErrorMessage,
    Conceptual,
    Architecture,
    Flow,
    State,
    Impact,
}

// trace:exempt reason=internal-detail
impl QueryShape {
    // trace:exempt reason=internal-detail
    pub fn as_str(self) -> &'static str {
        match self {
            QueryShape::Identifier => "identifier",
            QueryShape::SymbolLike => "symbol_like",
            QueryShape::PathLike => "path_like",
            QueryShape::StackTrace => "stack_trace",
            QueryShape::ErrorMessage => "error_message",
            QueryShape::Conceptual => "conceptual",
            QueryShape::Architecture => "architecture",
            QueryShape::Flow => "flow",
            QueryShape::State => "state",
            QueryShape::Impact => "impact",
        }
    }
}

/// FILE:LINE (and optional symbol) extracted from a stack/error query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct QueryLocus {
    pub path: String,
    pub line: u32,
    pub symbol: Option<String>,
}

/// How the retrieval lens should spend its budget. Flags, not a fused score.
#[derive(Debug, Clone, PartialEq, Eq)]
// trace:exempt reason=internal-detail
pub struct RetrievalPlan {
    pub shape: QueryShape,
    pub prefer_exact_name: bool,
    pub prefer_bm25: bool,
    pub prefer_atlas: bool,
    pub prefer_locus: bool,
    pub loci: Vec<QueryLocus>,
}

/// Classify query text. Pure; two runs agree by construction.
// trace:v1 id=impl.scc.core.query-shape work=WORK-ripwire-lessons-phase2 satisfies=REQ-query-shape-router
pub fn classify_query(query: &str) -> QueryShape {
    route_query(query).shape
}

/// Route a query to a retrieval plan. Does not score the corpus.
// trace:v1 id=impl.scc.core.route-query work=WORK-ripwire-lessons-phase2 satisfies=REQ-query-shape-router
pub fn route_query(query: &str) -> RetrievalPlan {
    let trimmed = query.trim();
    let loci = extract_loci(trimmed);
    let specific_frames = count_specific_frames(trimmed);
    let generic_frames = loci.len();
    if specific_frames > 0 || generic_frames >= 2 {
        return plan(QueryShape::StackTrace, false, false, false, true, loci);
    }
    if looks_error_message(trimmed) {
        return plan(
            QueryShape::ErrorMessage,
            false,
            generic_frames > 0,
            false,
            generic_frames > 0,
            loci,
        );
    }
    if looks_path_query(trimmed) {
        return plan(QueryShape::PathLike, true, false, false, generic_frames > 0, loci);
    }
    if looks_symbol_like(trimmed) {
        return plan(QueryShape::SymbolLike, true, false, false, false, loci);
    }
    if looks_identifier(trimmed) {
        return plan(QueryShape::Identifier, true, false, false, false, loci);
    }
    let lower = trimmed.to_ascii_lowercase();
    if has_any(&lower, &["architecture", "system atlas", "component", "trust boundary", "deployment"])
    {
        return plan(QueryShape::Architecture, false, true, true, false, loci);
    }
    if has_any(&lower, &["what happens", "call flow", "causal flow"])
        || (lower.contains("flow") && (lower.contains('?') || lower.contains("when")))
    {
        return plan(QueryShape::Flow, false, true, false, false, loci);
    }
    if has_any(
        &lower,
        &[
            "state authority",
            "source of truth",
            "who writes",
            "invalidat",
            "cache owner",
        ],
    ) {
        return plan(QueryShape::State, false, true, false, false, loci);
    }
    if has_any(
        &lower,
        &[
            "blast radius",
            "who calls",
            "what breaks",
            "impact of",
            "if i change",
        ],
    ) {
        return plan(QueryShape::Impact, false, true, false, false, loci);
    }
    plan(QueryShape::Conceptual, false, true, false, false, loci)
}

// trace:exempt reason=internal-detail
fn plan(
    shape: QueryShape,
    prefer_exact_name: bool,
    prefer_bm25: bool,
    prefer_atlas: bool,
    prefer_locus: bool,
    loci: Vec<QueryLocus>,
) -> RetrievalPlan {
    RetrievalPlan {
        shape,
        prefer_exact_name,
        prefer_bm25,
        prefer_atlas,
        prefer_locus,
        loci,
    }
}

// trace:exempt reason=internal-detail
fn has_any(hay: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| hay.contains(n))
}

// trace:exempt reason=internal-detail
fn looks_identifier(q: &str) -> bool {
    let q = q.trim();
    if q.is_empty() || q.contains(char::is_whitespace) || q.contains('/') || q.contains('\\') {
        return false;
    }
    if q.contains("::") || q.contains('.') {
        return false;
    }
    q.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && q.chars().any(|c| c.is_ascii_alphabetic())
}

// trace:exempt reason=internal-detail
fn looks_symbol_like(q: &str) -> bool {
    let q = q.trim();
    if q.is_empty() || q.contains(char::is_whitespace) || q.contains('/') || q.contains('\\') {
        return false;
    }
    (q.contains("::") || q.contains('.'))
        && q.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '.' | '#'))
}

// trace:exempt reason=internal-detail
fn looks_path_query(q: &str) -> bool {
    let q = q.trim();
    if q.contains(char::is_whitespace) {
        return false;
    }
    let slash = q.contains('/') || q.contains('\\');
    let ext = q.rsplit_once('.').map(|(_, e)| {
        !e.is_empty()
            && e.len() <= 5
            && e.chars().all(|c| c.is_ascii_alphanumeric())
    });
    slash && ext.unwrap_or(false) && !q.contains("://")
}

// trace:exempt reason=internal-detail
fn looks_error_message(q: &str) -> bool {
    let lower = q.to_ascii_lowercase();
    has_any(
        &lower,
        &[
            "error:",
            "exception",
            "panic!",
            "fatal:",
            "failed to",
            "undefined is not",
            "cannot find",
            "typeerror",
            "nullpointer",
        ],
    )
}

// trace:exempt reason=internal-detail
fn count_specific_frames(q: &str) -> usize {
    let mut n = 0;
    for line in q.lines() {
        let t = line.trim();
        let lower = t.to_ascii_lowercase();
        if lower.contains("file \"") && lower.contains(", line ") {
            n += 1;
            continue;
        }
        if t.starts_with("at ") && (t.contains('(') || t.contains(".java:")) {
            n += 1;
            continue;
        }
        if t.contains(" --> ") && t.contains(".rs:") {
            n += 1;
            continue;
        }
        if t.starts_with('#') && t.contains("0x") {
            n += 1;
        }
    }
    n
}

/// Extract `path:line` loci. A lone `host:port` is ignored (no file extension).
// trace:exempt reason=internal-detail
pub fn extract_loci(query: &str) -> Vec<QueryLocus> {
    let mut out = Vec::new();
    for line in query.lines() {
        if let Some(loc) = locus_from_line(line) {
            out.push(loc);
        }
    }
    out
}

/// Indexed path matches a stack-frame path: equal, or a `/`-anchored suffix.
/// `extra.py` does not match locus `a.py`.
// trace:v1 id=impl.scc.core.path-matches-locus work=WORK-ripwire-lessons-phase3 satisfies=REQ-stack-locus-ingest
pub fn path_matches_locus(indexed: &str, locus: &str) -> bool {
    fn norm(p: &str) -> String {
        p.trim().trim_start_matches("./").replace('\\', "/")
    }
    let a = norm(indexed);
    let b = norm(locus);
    if a.is_empty() || b.is_empty() {
        return false;
    }
    a == b || a.ends_with(&format!("/{b}")) || b.ends_with(&format!("/{a}"))
}

// trace:exempt reason=internal-detail
fn locus_from_line(line: &str) -> Option<QueryLocus> {
    let t = line.trim();
    // Python: File "src/foo.py", line 12, in handle
    if let Some(rest) = t.find("File \"").or_else(|| t.find("file \"")) {
        let after = &t[rest + 6..];
        if let Some(end) = after.find('"') {
            let path = after[..end].to_string();
            let tail = &after[end + 1..];
            let line_no = parse_after(tail, "line ")?;
            let symbol = tail
                .rsplit("in ")
                .next()
                .map(|s| s.trim().trim_end_matches(',').to_string())
                .filter(|s| {
                    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                });
            return Some(QueryLocus {
                path,
                line: line_no,
                symbol,
            });
        }
    }
    // rustc: --> src/foo.rs:12:3
    if let Some(idx) = t.find("--> ") {
        return path_line_from(&t[idx + 4..]);
    }
    path_line_from(t)
}

// trace:exempt reason=internal-detail
fn path_line_from(t: &str) -> Option<QueryLocus> {
    let token = t
        .split_whitespace()
        .rev()
        .find(|s| s.contains('.') && s.contains(':'))?;
    let token = token.trim_end_matches([')', ',', ']']);
    if token.contains("://") {
        return None;
    }
    let mut parts: Vec<&str> = token.rsplitn(3, ':').collect();
    parts.reverse();
    let (path, line_s) = match parts.as_slice() {
        [path, line] if line.chars().all(|c| c.is_ascii_digit()) => (*path, *line),
        [path, line, col]
            if line.chars().all(|c| c.is_ascii_digit()) && col.chars().all(|c| c.is_ascii_digit()) =>
        {
            (*path, *line)
        }
        _ => return None,
    };
    if !looks_source_path(path) {
        return None;
    }
    let line: u32 = line_s.parse().ok()?;
    Some(QueryLocus {
        path: path.to_string(),
        line,
        symbol: None,
    })
}

// trace:exempt reason=internal-detail
fn looks_source_path(path: &str) -> bool {
    let ext = path.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    !ext.is_empty()
        && ext.len() <= 5
        && ext.chars().all(|c| c.is_ascii_alphanumeric())
        && !path.contains("://")
}

// trace:exempt reason=internal-detail
fn parse_after(hay: &str, key: &str) -> Option<u32> {
    let i = hay.find(key)?;
    let rest = hay[i + key.len()..].trim_start();
    let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    num.parse().ok()
}

/// Experimental ranking arms. Production default is blended importance
/// (today's surface). Other arms exist so evals can compare them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
// trace:exempt reason=internal-detail
pub enum RankingArm {
    /// Current SCC fused ranker. Do not change without evidence.
    #[default]
    ProductionBlended,
    LexicalThenGraph,
    QueryRouted,
    NoGraph,
    NoLexical,
    NoSemantic,
    NoCochange,
    NoTaskPpr,
    NoGlobalPpr,
}

// trace:exempt reason=internal-detail
impl RankingArm {
    // trace:exempt reason=internal-detail
    pub fn as_str(self) -> &'static str {
        match self {
            RankingArm::ProductionBlended => "production-blended",
            RankingArm::LexicalThenGraph => "lexical-then-graph",
            RankingArm::QueryRouted => "query-routed",
            RankingArm::NoGraph => "no-graph",
            RankingArm::NoLexical => "no-lexical",
            RankingArm::NoSemantic => "no-semantic",
            RankingArm::NoCochange => "no-cochange",
            RankingArm::NoTaskPpr => "no-task-ppr",
            RankingArm::NoGlobalPpr => "no-global-ppr",
        }
    }

    // trace:exempt reason=internal-detail
    pub fn all() -> &'static [RankingArm] {
        &[
            RankingArm::ProductionBlended,
            RankingArm::LexicalThenGraph,
            RankingArm::QueryRouted,
            RankingArm::NoGraph,
            RankingArm::NoLexical,
            RankingArm::NoSemantic,
            RankingArm::NoCochange,
            RankingArm::NoTaskPpr,
            RankingArm::NoGlobalPpr,
        ]
    }

    // trace:exempt reason=internal-detail
    pub fn parse(s: &str) -> Option<Self> {
        Self::all().iter().copied().find(|a| a.as_str() == s)
    }
}

/// A relevance hit. BM25 and exact-anchor are separate numbers/flags.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct RelevanceHit {
    pub id: String,
    pub bm25: f64,
    pub exact_anchor: bool,
    pub shape: QueryShape,
}

/// Cap on extracted mention tokens (Ripwire `kMentionMaxRawTokens`).
pub const QUERY_MENTION_MAX_RAW: usize = 16;

/// One explicit mention extracted from query/task text.
#[derive(Debug, Clone, PartialEq, Eq)]
// trace:exempt reason=internal-detail
pub struct QueryMention {
    pub segments: Vec<String>,
    pub is_path: bool,
    pub backticked: bool,
}

// trace:exempt reason=internal-detail
fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

// trace:exempt reason=internal-detail
fn is_mention_token_char(c: char) -> bool {
    is_ident_char(c) || c == '.' || c == '/' || c == '-'
}

/// Extract path / dotted / backtick mentions from task text.
/// Plain prose words never qualify.
// trace:v1 id=impl.scc.core.query-mentions work=WORK-ripwire-lessons-phase2 satisfies=REQ-query-mentions
pub fn extract_query_mentions(task: &str) -> Vec<QueryMention> {
    let bytes: Vec<char> = task.chars().collect();
    let mut raw = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() && raw.len() < QUERY_MENTION_MAX_RAW {
        if !is_mention_token_char(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && is_mention_token_char(bytes[i]) {
            i += 1;
        }
        let mut tok: String = bytes[start..i].iter().collect();
        let backticked = start > 0
            && bytes[start - 1] == '`'
            && i < bytes.len()
            && bytes[i] == '`';
        while tok.ends_with('.') || tok.ends_with('/') || tok.ends_with('-') {
            tok.pop();
        }
        while tok.starts_with('.') || tok.starts_with('/') || tok.starts_with('-') {
            tok.remove(0);
        }
        if tok.len() < 3 || tok.len() > 200 {
            continue;
        }
        let has_slash = tok.contains('/');
        let has_dot = tok.contains('.');
        if !has_slash && !has_dot && !backticked {
            continue;
        }
        let joiner = if has_slash { '/' } else { '.' };
        let mut segments: Vec<String> = Vec::new();
        let mut malformed = false;
        let mut max_seg = 0usize;
        for seg in tok.split(joiner) {
            if seg.is_empty() {
                if has_slash {
                    continue;
                }
                malformed = true;
                break;
            }
            max_seg = max_seg.max(seg.len());
            segments.push(seg.to_string());
        }
        if malformed || segments.is_empty() {
            continue;
        }
        if !has_slash && !backticked {
            if segments.len() < 2 || max_seg < 3 {
                continue;
            }
            if segments
                .iter()
                .all(|s| s.chars().all(|c| c.is_ascii_digit()))
            {
                continue;
            }
        }
        if has_slash && segments.len() > 3 {
            segments = segments.split_off(segments.len() - 3);
        }
        raw.push(QueryMention {
            segments,
            is_path: has_slash,
            backticked,
        });
    }
    raw
}

// trace:exempt reason=internal-detail
fn path_basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

// trace:exempt reason=internal-detail
fn strip_ext(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

/// Path-suffix match: mention segments are a tail of whole path components
/// (extension-agnostic on the last).
// trace:exempt reason=internal-detail
fn path_suffix_matches(path: &str, segments: &[String]) -> bool {
    if segments.is_empty() {
        return false;
    }
    let mut comps: Vec<&str> = path
        .split(['/', '\\'])
        .filter(|s| !s.is_empty())
        .collect();
    if comps.is_empty() {
        return false;
    }
    let last = *comps.last().unwrap();
    let want_last = segments.last().unwrap().as_str();
    if last != want_last && strip_ext(last) != want_last {
        return false;
    }
    comps.pop();
    let earlier = &segments[..segments.len() - 1];
    if earlier.len() > comps.len() {
        return false;
    }
    let tail = &comps[comps.len() - earlier.len()..];
    tail.iter().zip(earlier.iter()).all(|(c, s)| *c == s.as_str())
}

/// True when a mention names this document's entity name or path.
// trace:v1 id=impl.scc.core.mention-matches-doc work=WORK-ripwire-lessons-phase2 satisfies=REQ-query-mentions
pub fn mention_matches_doc(name: &str, path: &str, mention: &QueryMention) -> bool {
    let joined = mention.segments.join(if mention.is_path { "/" } else { "." });
    if !name.is_empty() {
        if name == joined || name.eq_ignore_ascii_case(&joined) {
            return true;
        }
        if let Some(last) = mention.segments.last() {
            if name == last.as_str()
                || name.ends_with(&format!(".{last}"))
                || name.ends_with(&format!("::{last}"))
            {
                return true;
            }
        }
    }
    if path.is_empty() {
        return false;
    }
    if path_suffix_matches(path, &mention.segments) {
        return true;
    }
    for suffix_len in 1..mention.segments.len() {
        let suffix = &mention.segments[mention.segments.len() - suffix_len..];
        if path_suffix_matches(path, suffix) {
            return true;
        }
    }
    if let Some(last) = mention.segments.last() {
        let base = path_basename(path);
        if base == last.as_str() || strip_ext(base) == last.as_str() {
            return true;
        }
    }
    false
}

/// True when `name` is an exact anchor for `query` after identifier
/// normalization (case-insensitive, subtoken-set equal, or last path segment).
// trace:v1 id=impl.scc.core.exact-anchor work=WORK-ripwire-lessons-phase2 satisfies=REQ-exact-anchors
pub fn is_exact_anchor(query: &str, name: &str) -> bool {
    let q = query.trim();
    if q.is_empty() || name.is_empty() {
        return false;
    }
    if q.eq_ignore_ascii_case(name) {
        return true;
    }
    let q_last = q.rsplit(['/', '\\', '.', ':']).next().unwrap_or(q);
    if q_last.eq_ignore_ascii_case(name) {
        return true;
    }
    let q_toks = subtokens(q);
    let n_toks = subtokens(name);
    !q_toks.is_empty() && q_toks == n_toks
}

/// Score documents with BM25 and mark exact anchors. Does not blend PPR.
/// Uses candidate-set IDF (cold). Prefer [`relevance_hits_with_stats`] when
/// corpus-wide stats are persisted.
// trace:exempt reason=internal-detail
pub fn relevance_hits(query: &str, docs: &[LexDoc]) -> Vec<RelevanceHit> {
    relevance_hits_from_scores(query, docs, bm25_scores(query, docs))
}

/// Same as [`relevance_hits`] but BM25 uses persisted corpus IDF/`avgdl`.
/// Experimental lens only — production `build_surface` must not call this.
// trace:v1 id=impl.scc.core.bm25-warm-hits work=WORK-ripwire-lessons-phase2 satisfies=REQ-bm25-persist
pub fn relevance_hits_with_stats(
    query: &str,
    docs: &[LexDoc],
    stats: &Bm25CorpusStats,
) -> Vec<RelevanceHit> {
    relevance_hits_from_scores(query, docs, bm25_scores_with_stats(query, docs, stats))
}

// trace:exempt reason=internal-detail
fn relevance_hits_from_scores(
    query: &str,
    docs: &[LexDoc],
    scores: Vec<f64>,
) -> Vec<RelevanceHit> {
    let shape = classify_query(query);
    let mentions = extract_query_mentions(query);
    let mut hits: Vec<RelevanceHit> = docs
        .iter()
        .zip(scores)
        .map(|(d, bm25)| {
            let name = anchor_name(d);
            let path = path_field(d);
            let named = is_exact_anchor(query, &name)
                || mentions
                    .iter()
                    .any(|m| mention_matches_doc(&name, &path, m));
            RelevanceHit {
                id: d.id.clone(),
                bm25,
                exact_anchor: named,
                shape,
            }
        })
        .collect();
    hits.sort_by(|a, b| {
        b.exact_anchor
            .cmp(&a.exact_anchor)
            .then_with(|| {
                b.bm25
                    .partial_cmp(&a.bm25)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.id.cmp(&b.id))
        });
    hits
}

// trace:exempt reason=internal-detail
fn anchor_name(doc: &LexDoc) -> String {
    doc.fields
        .iter()
        .find(|f| f.weight == WEIGHT_NAME)
        .map(|f| f.text.clone())
        .unwrap_or_default()
}

// trace:exempt reason=internal-detail
fn path_field(doc: &LexDoc) -> String {
    doc.fields
        .iter()
        .find(|f| f.weight == WEIGHT_PATH)
        .map(|f| f.text.clone())
        .unwrap_or_default()
}

/// All ranking-arm ids. Tests fail if an arm is added to the enum and
/// omitted here — same SoT pattern as language/predicate registries.
// trace:exempt reason=internal-detail
pub fn ranking_arm_ids() -> Vec<&'static str> {
    RankingArm::all().iter().map(|a| a.as_str()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // trace:v1 id=test.scc.core.subtokens-acronym verifies=REQ-subtoken-bm25 exercises=impl.scc.core.subtokens
    fn acronym_and_camel_splits_match_ripwire_rule() {
        assert_eq!(subtokens("MCP"), vec!["mcp"]);
        assert_eq!(subtokens("HTTPServer"), vec!["http", "server"]);
        assert_eq!(
            subtokens("updateCollisionPositionVelocity"),
            vec!["update", "collision", "position", "velocity"]
        );
        assert_eq!(subtokens("_max_speed"), vec!["max", "speed"]);
        assert_eq!(subtokens("IOError"), vec!["io", "error"]);
        assert_eq!(subtokens("XMLHttpRequest"), vec!["xml", "http", "request"]);
        assert_eq!(subtokens("a"), Vec::<String>::new()); // dropped < 2
        assert_eq!(subtokens("handleList"), vec!["handle", "list"]);
    }

    #[test]
    // trace:v1 id=test.scc.core.bm25-deterministic verifies=REQ-subtoken-bm25 exercises=impl.scc.core.bm25
    fn bm25_is_deterministic_and_name_weighted() {
        let docs = vec![
            LexDoc::from_parts("b", "unrelated", "src/other.py", "", "noise"),
            LexDoc::from_parts(
                "a",
                "handleList",
                "src/server.ts",
                "Fetch the user rows",
                "db.users.findMany",
            ),
        ];
        let s1 = bm25_scores("handleList", &docs);
        let s2 = bm25_scores("handleList", &docs);
        assert_eq!(s1, s2);
        assert!(s1[1] > s1[0], "name match must outrank unrelated: {s1:?}");
        let ranked = bm25_rank("handleList", &docs);
        assert_eq!(ranked[0].0, "a");
        // empty query is zeros, not NaN
        assert_eq!(bm25_scores("", &docs), vec![0.0, 0.0]);
    }

    #[test]
    // trace:v1 id=test.scc.core.query-shape-conservative verifies=REQ-query-shape-router exercises=impl.scc.core.query-shape
    fn query_shape_is_conservative() {
        assert_eq!(classify_query("handleList"), QueryShape::Identifier);
        assert_eq!(classify_query("Foo::bar"), QueryShape::SymbolLike);
        assert_eq!(classify_query("src/server.ts"), QueryShape::PathLike);
        assert_eq!(
            classify_query("https://example.com:8080/docs"),
            QueryShape::Conceptual
        );
        assert_eq!(
            classify_query("see Type.py:12 in the docs"),
            QueryShape::Conceptual
        ); // one generic path:line is not a trace
        let py = r#"Traceback (most recent call last):
  File "src/server.ts", line 12, in handleList
    db.users.findMany()
"#;
        assert_eq!(classify_query(py), QueryShape::StackTrace);
        let plan = route_query(py);
        assert!(plan.prefer_locus);
        assert!(!plan.loci.is_empty());
        assert_eq!(
            classify_query("how does authentication work?"),
            QueryShape::Conceptual
        );
        assert_eq!(
            classify_query("what is the system architecture of billing?"),
            QueryShape::Architecture
        );
        assert_eq!(
            classify_query("what happens when login fails?"),
            QueryShape::Flow
        );
        assert_eq!(
            classify_query("who writes SessionStore?"),
            QueryShape::State
        );
        assert_eq!(
            classify_query("blast radius if I change parseToken"),
            QueryShape::Impact
        );
        assert_eq!(
            classify_query("TypeError: cannot read property of undefined"),
            QueryShape::ErrorMessage
        );
        assert_eq!(RankingArm::default(), RankingArm::ProductionBlended);
        assert_eq!(ranking_arm_ids().len(), RankingArm::all().len());
    }

    #[test]
    // trace:v1 id=test.scc.core.exact-anchor verifies=REQ-exact-anchors exercises=impl.scc.core.exact-anchor
    fn exact_anchor_is_not_mixed_into_bm25() {
        assert!(is_exact_anchor("handleList", "handleList"));
        assert!(is_exact_anchor("HTTPServer", "http_server"));
        assert!(!is_exact_anchor("how does login work", "handleList"));
        let docs = vec![
            LexDoc::from_parts("noise", "loginHelper", "src/a.ts", "handles login flow", ""),
            LexDoc::from_parts("hit", "handleList", "src/b.ts", "", ""),
        ];
        let hits = relevance_hits("handleList", &docs);
        let hit = hits.iter().find(|h| h.id == "hit").unwrap();
        assert!(hit.exact_anchor);
        assert!(!hits.iter().find(|h| h.id == "noise").unwrap().exact_anchor);
        // anchors sort first even if BM25 could argue otherwise
        assert_eq!(hits[0].id, "hit");
        assert_eq!(hits[0].shape, QueryShape::Identifier);
    }

    #[test]
    // trace:exempt reason=internal-detail
    fn ranking_arms_are_the_single_list() {
        let ids = ranking_arm_ids();
        assert!(ids.contains(&"production-blended"));
        assert!(ids.contains(&"lexical-then-graph"));
        assert!(ids.contains(&"query-routed"));
        let mut seen = std::collections::BTreeSet::new();
        for id in &ids {
            assert!(seen.insert(*id), "duplicate arm {id}");
        }
    }

    #[test]
    // trace:v1 id=test.scc.core.query-mentions verifies=REQ-query-mentions exercises=impl.scc.core.query-mentions
    fn query_mentions_are_precision_first() {
        assert!(extract_query_mentions("how does login work").is_empty());
        let path = extract_query_mentions("fix sklearn/ensemble/_iforest.py please");
        assert_eq!(path.len(), 1);
        assert!(path[0].is_path);
        assert_eq!(
            path[0].segments,
            vec!["sklearn", "ensemble", "_iforest.py"]
        );
        let dotted = extract_query_mentions("see transformers.optimization");
        assert_eq!(dotted.len(), 1);
        assert!(!dotted[0].is_path);
        let tick = extract_query_mentions("look at `handleList` next");
        assert_eq!(tick.len(), 1);
        assert!(tick[0].backticked);
        assert_eq!(tick[0].segments, vec!["handleList"]);
        assert!(extract_query_mentions("version 3.10 of python").is_empty());
        assert!(extract_query_mentions("e.g. this").is_empty());
    }

    #[test]
    // trace:v1 id=test.scc.core.query-mention-anchor verifies=REQ-query-mentions exercises=impl.scc.core.mention-matches-doc
    fn query_mentions_mark_anchors_without_changing_bm25() {
        let docs = vec![
            LexDoc::from_parts("noise", "loginHelper", "src/login.ts", "handles the list", ""),
            LexDoc::from_parts("hit", "handleList", "src/server.ts", "", ""),
            LexDoc::from_parts("file", "iforest", "sklearn/ensemble/_iforest.py", "", ""),
        ];
        let hits = relevance_hits("please inspect `handleList` in the service", &docs);
        let hit = hits.iter().find(|h| h.id == "hit").unwrap();
        assert!(hit.exact_anchor);
        let noise = hits.iter().find(|h| h.id == "noise").unwrap();
        assert!(!noise.exact_anchor);
        let prose = relevance_hits("how does login work", &docs);
        assert!(!prose.iter().any(|h| h.exact_anchor));
        let path_hits = relevance_hits("bug in sklearn/ensemble/_iforest.py", &docs);
        assert!(
            path_hits.iter().find(|h| h.id == "file").unwrap().exact_anchor,
            "path mention must anchor the named file"
        );
    }

    #[test]
    // trace:v1 id=test.scc.core.bm25-persist verifies=REQ-bm25-persist exercises=impl.scc.core.bm25-persist
    fn persisted_corpus_stats_match_cold_bm25_on_same_docs() {
        let docs = vec![
            LexDoc::from_parts("a", "handleList", "src/a.py", "list handler", "def handle_list"),
            LexDoc::from_parts("b", "other", "src/b.py", "unrelated", "def other"),
            LexDoc::from_parts("c", "handleThing", "src/c.py", "", "def handle_thing"),
        ];
        let stats = Bm25CorpusStats::from_docs(&docs);
        assert_eq!(stats.n, 3);
        assert!(stats.df.contains_key("handle"));
        let cold = bm25_scores("handleList", &docs);
        let warm = bm25_scores_with_stats("handleList", &docs, &stats);
        assert_eq!(cold.len(), warm.len());
        for (c, w) in cold.iter().zip(warm.iter()) {
            assert!((c - w).abs() < 1e-9, "cold={c} warm={w}");
        }
        let cold_hits = relevance_hits("handleList", &docs);
        let warm_hits = relevance_hits_with_stats("handleList", &docs, &stats);
        assert_eq!(cold_hits.len(), warm_hits.len());
        for (c, w) in cold_hits.iter().zip(warm_hits.iter()) {
            assert_eq!(c.id, w.id);
            assert_eq!(c.exact_anchor, w.exact_anchor);
            assert!((c.bm25 - w.bm25).abs() < 1e-9);
        }
        let subset = vec![docs[0].clone()];
        let subset_cold = bm25_scores("handleList", &subset);
        let subset_warm = bm25_scores_with_stats("handleList", &subset, &stats);
        assert!(
            (subset_cold[0] - subset_warm[0]).abs() > 1e-12,
            "full-corpus IDF must differ from 1-document slice IDF"
        );
    }

    #[test]
    // trace:v1 id=test.scc.core.path-matches-locus verifies=REQ-stack-locus-ingest exercises=impl.scc.core.path-matches-locus
    fn path_suffix_match_does_not_hit_extra_py() {
        assert!(path_matches_locus("src/app.py", "app.py"));
        assert!(path_matches_locus("src/app.py", "src/app.py"));
        assert!(!path_matches_locus("src/extra.py", "a.py"));
        assert!(!path_matches_locus("src/ba.py", "a.py"));
    }
}
