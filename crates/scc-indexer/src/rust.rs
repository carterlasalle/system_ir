//! Rust language extractor (tree-sitter based).
//!
//! Pure, deterministic extraction: `(path, content) -> ExtractedFile`.
//! Syntax-level only; cross-file resolution happens in `resolve.rs`.
//!
//! Kind mapping (SymbolKind has no struct/trait/impl variants, so the
//! nearest model kinds are used): `struct` -> Class, `enum` -> Enum,
//! `trait` -> Interface. `impl` blocks are not emitted as symbols
//! themselves (their name would collide with the type they implement);
//! their methods are emitted as Method symbols named `Type.method` so the
//! native resolver's `self`/`this` rule can resolve `self.method()` calls
//! (resolve.rs splits on '.' — `Type::method` names would never resolve).

use crate::facts;
use crate::model::{
    Call, Entrypoint, ExtractedFile, Import, ImportType, LanguageExtractor, Retry, SemanticFact,
    SourceFile, StoreOp, StoreRef, Symbol, SymbolKind, Test, TestKind,
};
use tree_sitter::{Node, Parser};
use std::collections::{BTreeMap, BTreeSet};
// trace:v1 id=impl.scc.extract.rust work=WORK-SCC-004 satisfies=REQ-SCC-IR

/// Rust extractor. Uses the tree-sitter-rust grammar.
pub struct RustExtractor {
    language: tree_sitter::Language,
}

impl Default for RustExtractor {
    fn default() -> Self {
        RustExtractor {
            language: tree_sitter_rust::LANGUAGE.into(),
        }
    }
}

impl LanguageExtractor for RustExtractor {
    fn language(&self) -> &'static str {
        "rust"
    }

    fn extract(&self, file: &SourceFile) -> ExtractedFile {
        let src = file.content.as_bytes();
        let mut parser = Parser::new();
        if parser.set_language(&self.language).is_err() {
            return ExtractedFile::default();
        }
        let Some(tree) = parser.parse(&file.content, None) else {
            return ExtractedFile::default();
        };
        let mut ctx = Ctx {
            module_name: facts::module_stem(&file.path),
            ..Default::default()
        };
        self.walk(tree.root_node(), &mut ctx, src);
        ctx.into_extracted()
        }
}

// ---------------------------------------------------------------------------
// Node helpers
// ---------------------------------------------------------------------------

fn node_text<'a>(node: Option<Node<'a>>, src: &'a [u8]) -> &'a str {
    match node {
        Some(n) => n.utf8_text(src).unwrap_or(""),
        None => "",
    }
}

/// Collapse internal whitespace runs to single spaces; trim ends.
fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_ws = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            pending_ws = !out.is_empty();
        } else {
            if pending_ws {
                out.push(' ');
            }
            pending_ws = false;
            out.push(ch);
        }
    }
    out
}

fn clean(s: &str) -> String {
    collapse(s.trim())
}

fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Case-insensitive byte search; returns index into the original string.
fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let h = hay.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || n.len() > h.len() {
        return None;
    }
    'outer: for i in 0..=(h.len() - n.len()) {
        for j in 0..n.len() {
            if !h[i + j].eq_ignore_ascii_case(&n[j]) {
                continue 'outer;
            }
        }
        return Some(i);
    }
    None
}

/// First dotted identifier (`foo` or `schema.foo`) at/after byte `from`.
fn first_dotted_after(s: &str, from: usize) -> Option<String> {
    let bytes = s.as_bytes();
    let mut i = from.min(bytes.len());
    while i < bytes.len() && !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let start = i;
    let mut end = i;
    while end < bytes.len() {
        let c = bytes[end];
        if c.is_ascii_alphanumeric() || c == b'_' || c == b'.' {
            end += 1;
        } else {
            break;
        }
    }
    while end > start && bytes[end - 1] == b'.' {
        end -= 1;
    }
    if end == start {
        None
    } else {
        Some(s[start..end].to_string())
    }
}

fn words_after(s: &str, from: usize) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = from.min(bytes.len());
    while i < bytes.len() {
        while i < bytes.len() && !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let start = i;
        let mut end = i;
        while end < bytes.len() {
            let c = bytes[end];
            if c.is_ascii_alphanumeric() || c == b'_' || c == b'.' {
                end += 1;
            } else {
                break;
            }
        }
        while end > start && bytes[end - 1] == b'.' {
            end -= 1;
        }
        if end > start {
            out.push(s[start..end].to_string());
        }
        i = end;
    }
    out
}

/// Classify a SQL statement and extract its target table.
fn sql_op_table(sql: &str) -> (StoreOp, Option<String>) {
    const WRITE_KW: &[&str] = &["insert", "update", "delete", "replace", "merge"];
    const MIGRATE_KW: &[&str] = &["create", "alter", "drop"];
    const QUERY_KW: &[&str] = &["select", "show", "describe"];
    let mut best: Option<(usize, &str, StoreOp)> = None;
    for kw in WRITE_KW {
        if let Some(p) = find_ci(sql, kw) {
            if best.map(|(bp, _, _)| p < bp).unwrap_or(true) {
                best = Some((p, kw, StoreOp::Write));
            }
        }
    }
    for kw in MIGRATE_KW {
        if let Some(p) = find_ci(sql, kw) {
            if best.map(|(bp, _, _)| p < bp).unwrap_or(true) {
                best = Some((p, kw, StoreOp::Migrate));
            }
        }
    }
    for kw in QUERY_KW {
        if let Some(p) = find_ci(sql, kw) {
            if best.map(|(bp, _, _)| p < bp).unwrap_or(true) {
                best = Some((p, kw, StoreOp::Query));
            }
        }
    }
    let Some((pos, verb, op)) = best else {
        return (StoreOp::Query, None);
    };
    let table = extract_table(sql, verb, pos);
    (op, table)
}

fn extract_table(sql: &str, verb: &str, verb_pos: usize) -> Option<String> {
    match verb {
        "insert" | "replace" | "merge" => {
            let pos = find_ci(sql, "into")?;
            first_dotted_after(sql, pos + 4)
        }
        "update" => first_dotted_after(sql, verb_pos + 6),
        "delete" => {
            let pos = find_ci(sql, "from")?;
            first_dotted_after(sql, pos + 4)
        }
        "select" => {
            let pos = find_ci(sql, "from")?;
            first_dotted_after(sql, pos + 4)
        }
        "show" | "describe" => first_dotted_after(sql, verb_pos + verb.len()),
        "create" | "alter" | "drop" => {
            let words = words_after(sql, verb_pos + verb.len());
            if words.is_empty() {
                return None;
            }
            // CREATE INDEX [name] ON table
            if words.iter().take(3).any(|w| w.eq_ignore_ascii_case("index")) {
                for (i, w) in words.iter().enumerate() {
                    if w.eq_ignore_ascii_case("on") && i + 1 < words.len() {
                        return Some(words[i + 1].clone());
                    }
                }
                return None;
            }
            let mut i = 0;
            while i < words.len() {
                let lower = words[i].to_ascii_lowercase();
                if matches!(
                    lower.as_str(),
                    "table"
                        | "view"
                        | "sequence"
                        | "unique"
                        | "temporary"
                        | "temp"
                        | "materialized"
                        | "global"
                        | "local"
                        | "or"
                        | "replace"
                        | "if"
                        | "not"
                        | "exists"
                ) {
                    i += 1;
                } else {
                    break;
                }
            }
            words.get(i).cloned()
        }
        _ => None,
    }
}

/// Value of a Rust string literal (or raw string), quotes stripped. The
/// grammar has no `content` field for string literals, so take the text
/// between the first and last `"` (works for `"..."`, `r"..."`, `r#"..."#`).
fn rust_string_value(node: Node, src: &[u8]) -> Option<String> {
    let t = node_text(Some(node), src).trim();
    let b = t.as_bytes();
    let first = b.iter().position(|&c| c == b'"')?;
    let last = b.iter().rposition(|&c| c == b'"')?;
    if last <= first {
        return None;
    }
    Some(t[first + 1..last].to_string())
}

/// First string-literal argument of a call.
fn first_string_arg(call: Node, src: &[u8]) -> Option<String> {
    let args = call.child_by_field_name("arguments")?;
    let mut cursor = args.walk();
    for child in args.named_children(&mut cursor) {
        if matches!(child.kind(), "string_literal" | "raw_string_literal") {
            return rust_string_value(child, src);
        }
    }
    None
}

/// First string-literal value anywhere under `node` (macro invocation
/// args, token trees, …), quotes stripped.
fn first_string_literal(node: Node, src: &[u8]) -> Option<String> {
    if matches!(node.kind(), "string_literal" | "raw_string_literal") {
        return rust_string_value(node, src);
    }
    let mut cursor = node.walk();
    for c in node.named_children(&mut cursor) {
        if let Some(v) = first_string_literal(c, src) {
            return Some(v);
        }
    }
    None
}

/// First paragraph of a docstring (blank-line terminated), like python.rs.
fn first_paragraph(s: &str) -> String {
    let t = s.trim();
    let mut lines: Vec<&str> = Vec::new();
    for line in t.lines() {
        if line.trim().is_empty() {
            if !lines.is_empty() {
                break;
            }
            continue;
        }
        lines.push(line.trim_end());
    }
    let joined = lines.join("\n");
    truncate_chars(joined.trim(), 200)
}

/// Strip a `///` or `/** */` doc marker; `None` for plain comments.
fn strip_doc_marker(raw: &str) -> Option<String> {
    let t = raw.trim_start();
    if let Some(rest) = t.strip_prefix("///") {
        let body = rest.strip_prefix(' ').unwrap_or(rest).trim_end();
        return Some(body.to_string());
    }
    if let Some(rest) = t.strip_prefix("/**") {
        let inner = rest.strip_suffix("*/").unwrap_or(rest);
        let lines: Vec<String> = inner
            .lines()
            .map(|l| l.trim_start().trim_start_matches('*').trim().to_string())
            .collect();
        return Some(lines.join("\n"));
    }
    None
}

// ---------------------------------------------------------------------------
// Tables (same receiver vocabulary as python.rs)
// ---------------------------------------------------------------------------

const STORE_RECEIVERS: &[&str] = &[
    "db", "database", "session", "conn", "cursor", "pool", "engine", "client", "redis", "r",
    "mongo", "collection", "kafka", "producer", "consumer", "queue", "publisher", "broker", "es",
    "elasticsearch", "s3", "bucket", "storage", "supabase", "firestore", "dynamodb", "table",
    "cache",
];

/// Roots for which a string literal argument names the target (key/topic).
const STRING_TARGET_ROOTS: &[&str] = &[
    "redis", "r", "kafka", "producer", "consumer", "broker", "publisher", "queue",
];

fn classify_op(name: &str) -> Option<StoreOp> {
    match name {
        "execute" | "executemany" | "executescript" => Some(StoreOp::Query), // refined by SQL sniff
        "commit" | "save" | "add" | "delete" | "update" | "insert" | "remove" | "set" | "upsert" => {
            Some(StoreOp::Write)
        }
        "get" | "fetch" | "read" | "count" => Some(StoreOp::Read),
        "query" | "select" | "find" => Some(StoreOp::Query),
        "publish" | "send" | "produce" => Some(StoreOp::Publish),
        "subscribe" | "consume" | "on_message" | "on_event" => Some(StoreOp::Subscribe),
        "incr" | "decr" | "push" | "pop" => Some(StoreOp::Write),
        _ => None,
    }
}

fn technology_for(root: &str) -> Option<String> {
    match root {
        "redis" | "r" => Some("redis".to_string()),
        "kafka" | "producer" | "consumer" | "broker" => Some("kafka".to_string()),
        "mongo" | "collection" => Some("mongodb".to_string()),
        "s3" | "bucket" => Some("s3".to_string()),
        "session" | "engine" | "conn" | "cursor" | "pool" | "db" | "database" => {
            Some("sql".to_string())
        }
        "supabase" => Some("postgres".to_string()),
        "es" | "elasticsearch" => Some("elasticsearch".to_string()),
        "firestore" => Some("firestore".to_string()),
        "dynamodb" => Some("dynamodb".to_string()),
        _ => None,
    }
}

/// CFG evidence for a call site: `(conditional, control_block, inside_loop,
/// inside_try)`, walking ancestors up to the nearest function boundary.
/// Rust has no syntactic try/catch (errors flow via `Result`), so the
/// control blocks are if/else/for/while/loop/match. The nearest block
/// wins; loop nesting accumulates (a call inside `if` within a `for` is
/// still `inside_loop`).
fn call_cfg(node: tree_sitter::Node) -> (bool, Option<&'static str>, bool, bool) {
    let mut cur = node.parent();
    let mut inside_loop = false;
    let mut block: Option<&'static str> = None;
    while let Some(anc) = cur {
        match anc.kind() {
            "function_item" | "function_signature_item" | "closure_expression"
            | "impl_item" | "trait_item" | "mod_item" | "source_file" => break,
            "if_expression" => {
                block.get_or_insert("if");
            }
            "else_clause" => {
                block.get_or_insert("else");
            }
            "for_expression" | "while_expression" | "loop_expression" => {
                inside_loop = true;
                let kind = match anc.kind() {
                    "for_expression" => "for",
                    "while_expression" => "while",
                    _ => "loop",
                };
                block.get_or_insert(kind);
            }
            "match_expression" => {
                block.get_or_insert("match");
            }
            _ => {}
        }
        cur = anc.parent();
    }
    (block.is_some(), block, inside_loop, false)
}

/// True when the call is awaited: `expr.await` — the ancestor is an
/// `await_expression` (tree-sitter-rust represents `x.await` as
/// `await_expression` wrapping the expression).
fn call_is_awaited(node: tree_sitter::Node) -> bool {
    let mut cur = node.parent();
    while let Some(anc) = cur {
        match anc.kind() {
            "function_item" | "function_signature_item" | "closure_expression"
            | "impl_item" | "trait_item" | "mod_item" | "source_file" => return false,
            "await_expression" => return true,
            _ => cur = anc.parent(),
        }
    }
    false
}

/// True when the call's result is consumed (assigned/returned/compared/
/// passed) rather than discarded as a bare expression statement.
fn call_returns_value(node: tree_sitter::Node) -> bool {
    let mut cur = node.parent();
    while let Some(anc) = cur {
        match anc.kind() {
            "function_item" | "function_signature_item" | "closure_expression"
            | "impl_item" | "trait_item" | "mod_item" | "source_file"
            | "expression_statement" => return false,
            "await_expression" | "parenthesized_expression" => cur = anc.parent(),
            _ => return true,
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Extraction context
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Scope {
    name: String,
    /// True inside an `impl` block: functions become methods of `name`.
    is_impl: bool,
}

#[derive(Default)]
struct Ctx {
    symbols: Vec<Symbol>,
    imports: Vec<Import>,
    calls: Vec<Call>,
    tests: Vec<Test>,
    store_refs: Vec<StoreRef>,
    retries: Vec<Retry>,
    entrypoints: Vec<Entrypoint>,
    /// CLI flags per owning symbol (clap `#[arg(...)]`), `-`/`--`
    /// prefixed, sorted + deduped.
    cli_flags: BTreeMap<String, BTreeSet<String>>,
    /// Wave 9 semantic facts collected during the walk.
    facts: Vec<SemanticFact>,
    /// axum Router registrations: emitted only when the file imports axum
    /// (verified in `into_extracted`, once imports are complete).
    reg_candidates: Vec<SemanticFact>,
    /// Contract subclass evidence: type → serializer trait names seen on
    /// it (`Serialize`/`Deserialize` from `#[derive(...)]` or `impl`).
    serde_impls: BTreeMap<String, BTreeSet<String>>,
    scopes: Vec<Scope>,
    /// Module-symbol name (file stem) owning crate-level STATE facts
    /// (`static` items).
    module_name: String,
    /// Per-caller call-site counter (source order) — CFG lexical evidence.
    call_seq: BTreeMap<Option<String>, u32>,
}

impl Ctx {
    fn caller(&self) -> Option<String> {
        self.scopes.last().map(|s| s.name.clone())
    }
    fn top_is_impl(&self) -> bool {
        self.scopes.last().map(|s| s.is_impl).unwrap_or(false)
    }
    fn top_name(&self) -> String {
        self.scopes.last().map(|s| s.name.clone()).unwrap_or_default()
    }
    fn into_extracted(self) -> ExtractedFile {
        let cli_flags = self
            .cli_flags
            .into_iter()
            .map(|(k, v)| (k, v.into_iter().collect()))
            .collect();
        // Framework verification: axum registrations are facts only when
        // the file imports axum (imports are complete after the full walk);
        // the Router receiver check already ran during the walk.
        let has_axum = self
            .imports
            .iter()
            .any(|i| i.module == "axum" || i.module.starts_with("axum::"));
        let mut facts = self.facts;
        // Contract subclass evidence (Contract ontology): serializer/
        // deserializer pairs around a type. A type with BOTH `Serialize`
        // and `Deserialize` (from `#[derive(Serialize, Deserialize)]` or
        // `impl Serialize for T` + `impl Deserialize for T`) is a
        // Serialization contract; the surface is the pair string.
        // Deterministic: types sorted.
        for (ty, traits) in &self.serde_impls {
            if traits.contains("Serialize") && traits.contains("Deserialize") {
                facts.push(SemanticFact::Registration {
                    owner: ty.clone(),
                    kind: "serialization".to_string(),
                    target: "Serialize/Deserialize".to_string(),
                });
            }
        }
        facts.extend(self.reg_candidates.into_iter().filter(|_| has_axum));
        facts.sort_by_key(fact_key);
        facts.dedup();
        // Crate-level `static` items are STATE facts owned by the module
        // symbol (file stem). Ensure it exists unless a real same-named
        // symbol is declared in this file (Field facts then attach to it —
        // same file, same component attribution, no id clash).
        let module_owned = facts
            .iter()
            .any(|f| matches!(f, SemanticFact::Field { owner, .. } if owner == &self.module_name));
        let mut symbols = self.symbols;
        if module_owned
            && !self.module_name.is_empty()
            && !symbols.iter().any(|s| s.name == self.module_name)
        {
            symbols.push(Symbol {
                name: self.module_name.clone(),
                kind: SymbolKind::Module,
                signature: None,
                start_line: 1,
                end_line: 1,
                exported: false,
                docstring: None,
                parent: None,
            });
        }
        ExtractedFile {
            symbols,
            imports: self.imports,
            calls: self.calls,
            routes: Vec::new(),
            tests: self.tests,
            store_refs: self.store_refs,
            retries: self.retries,
            entrypoints: self.entrypoints,
            cli_flags,
            facts,
        }
        }
}

// ---------------------------------------------------------------------------
// Walker
// ---------------------------------------------------------------------------

impl RustExtractor {
    fn walk(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        match node.kind() {
            // trait method declarations have no body (function_signature_item)
            "function_item" | "function_signature_item" => self.walk_function(node, ctx, src),
            "struct_item" => self.walk_type_item(node, SymbolKind::Class, ctx, src),
            "enum_item" => self.walk_type_item(node, SymbolKind::Enum, ctx, src),
            "trait_item" => self.walk_type_item(node, SymbolKind::Interface, ctx, src),
            "impl_item" => self.walk_impl(node, ctx, src),
            "mod_item" => self.walk_mod(node, ctx, src),
            "const_item" | "static_item" => self.walk_const(node, ctx, src),
            "use_declaration" => self.record_use(node, ctx, src),
            "call_expression" => self.record_call(node, ctx, src),
            "macro_invocation" => self.record_macro_invocation(node, ctx, src),
            _ => self.walk_children(node, ctx, src),
        }
    }

    fn walk_children(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, ctx, src);
        }
    }

    /// True when the item has a `pub` visibility modifier child.
    fn is_exported(&self, node: Node) -> bool {
        let mut cursor = node.walk();
        let mut exported = false;
        for c in node.children(&mut cursor) {
            if c.kind() == "visibility_modifier" {
                exported = true;
                break;
            }
        }
        exported
    }

    /// Docstring + attribute items directly above `node` (siblings, walking
    /// backward; doc comments may precede attributes). Returns `(docstring,
    /// attribute_items)` — docstring is the contiguous run of `///`/`/**`
    /// comments, first paragraph.
    fn leading_annotations<'a>(
        &self,
        node: Node<'a>,
        src: &'a [u8],
    ) -> (Option<String>, Vec<Node<'a>>) {
        let Some(parent) = node.parent() else {
            return (None, Vec::new());
        };
        let mut cursor = parent.walk();
        let children: Vec<Node> = parent.named_children(&mut cursor).collect();
        let Some(idx) = children.iter().position(|c| c.id() == node.id()) else {
            return (None, Vec::new());
        };
        let mut doc_lines: Vec<String> = Vec::new();
        let mut attrs: Vec<Node> = Vec::new();
        for c in children[..idx].iter().rev() {
            match c.kind() {
                "line_comment" | "block_comment" => {
                    match strip_doc_marker(node_text(Some(*c), src)) {
                        Some(s) => doc_lines.push(s),
                        // a plain comment breaks the doc run
                        None => break,
                    }
                }
                "attribute_item" => attrs.push(*c),
                _ => break,
            }
        }
        doc_lines.reverse();
        let doc = if doc_lines.is_empty() {
            None
        } else {
            Some(first_paragraph(&doc_lines.join("\n")))
        };
        (doc, attrs)
    }

    fn walk_function(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let name = clean(node_text(node.child_by_field_name("name"), src));
        if name.is_empty() {
            self.walk_children(node, ctx, src);
            return;
        }
        let in_impl = ctx.top_is_impl();
        let (sym_name, kind, parent) = if in_impl {
            let ty = ctx.top_name();
            (format!("{ty}.{name}"), SymbolKind::Method, Some(ty))
        } else {
            (name.clone(), SymbolKind::Function, None)
        };
        let exported = self.is_exported(node);
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;
        let (doc, attrs) = self.leading_annotations(node, src);
        let sig = signature(&name, node, src);
        ctx.symbols.push(Symbol {
            name: sym_name.clone(),
            kind,
            signature: Some(sig),
            start_line,
            end_line,
            exported,
            docstring: doc,
            parent,
        });
        for a in &attrs {
            let aname = attr_name(*a, src).to_ascii_lowercase();
            if aname.contains("retry") || aname.contains("backoff") {
                ctx.retries.push(Retry {
                    symbol: sym_name.clone(),
                    policy: attr_policy(*a, src),
                    line: a.start_position().row as u32 + 1,
                });
            }
            if aname == "test" || aname.ends_with("::test") {
                ctx.tests.push(Test {
                    name: name.clone(),
                    symbol: Some(sym_name.clone()),
                    kind: TestKind::Unit,
                    line: a.start_position().row as u32 + 1,
                });
            }
        }
        // `fn main` at crate top level is the program entrypoint.
        if !in_impl && ctx.scopes.is_empty() && name == "main" {
            ctx.entrypoints.push(Entrypoint {
                symbol: "main".to_string(),
                kind: "bin".to_string(),
                line: start_line,
            });
        }
        // Public API surface: `pub fn` at module level, `pub fn` methods.
        if exported {
            let kind = if in_impl { "method" } else { "function" };
            ctx.facts.push(SemanticFact::PublicExport {
                symbol: sym_name.clone(),
                kind: kind.to_string(),
            });
        }
        // Wave 9 builder/factory contracts inside `impl` blocks:
        // `fn new()/builder()/from_*()` constructors make the type a
        // factory; `fn with_x(mut self) -> Self` chains make it a builder.
        if in_impl && !ctx.top_name().is_empty() {
            let ty = ctx.top_name();
            if facts::is_factory_name("rust", &name) {
                ctx.facts.push(SemanticFact::Registration {
                    owner: ty.clone(),
                    kind: "factory".to_string(),
                    target: ty.clone(),
                });
            } else if facts::is_builder_chain_method(&name)
                && fn_returns_self_type(node, src)
            {
                ctx.facts.push(SemanticFact::Registration {
                    owner: ty.clone(),
                    kind: "builder".to_string(),
                    target: ty,
                });
            }
        }
        ctx.scopes.push(Scope {
            name: sym_name,
            is_impl: false,
        });
        self.walk_children(node, ctx, src);
        ctx.scopes.pop();
    }

    fn walk_type_item(&self, node: Node, kind: SymbolKind, ctx: &mut Ctx, src: &[u8]) {
        let name = clean(node_text(node.child_by_field_name("name"), src));
        if name.is_empty() {
            self.walk_children(node, ctx, src);
            return;
        }
        let exported = self.is_exported(node);
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;
        let (doc, attrs) = self.leading_annotations(node, src);
        ctx.symbols.push(Symbol {
            name: name.clone(),
            kind,
            signature: None,
            start_line,
            end_line,
            exported,
            docstring: doc,
            parent: None,
        });
        // clap derive surface: `#[derive(Parser)]` structs own their field
        // `#[arg(...)]` flags; `#[derive(Subcommand)]` enums expose each
        // variant as a CLI subcommand and own the variants' arg flags.
        let derives = derive_names(&attrs, src);
        let is_parser = derives.iter().any(|d| d == "Parser");
        let is_subcommand = derives.iter().any(|d| d == "Subcommand");
        if is_parser || is_subcommand {
            let mut lists: Vec<Node> = Vec::new();
            let mut variants: Vec<Node> = Vec::new();
            let mut cursor = node.walk();
            for c in node.named_children(&mut cursor) {
                match c.kind() {
                    "field_declaration_list" => lists.push(c),
                    "enum_variant_list" => {
                        let mut c2 = c.walk();
                        for inner in c.named_children(&mut c2) {
                            if inner.kind() == "enum_variant" {
                                variants.push(inner);
                                let mut c3 = inner.walk();
                                for f in inner.named_children(&mut c3) {
                                    if f.kind() == "field_declaration_list" {
                                        lists.push(f);
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            let mut flags = clap_arg_flags(&lists, src);
            if !flags.is_empty() {
                // sorted + deduped
                flags.sort();
                flags.dedup();
                ctx.cli_flags.entry(name.clone()).or_default().extend(flags);
            }
            if is_subcommand {
                for v in variants {
                    let vname = clean(node_text(v.child_by_field_name("name"), src));
                    if vname.is_empty() {
                        continue;
                    }
                    ctx.entrypoints.push(Entrypoint {
                        symbol: vname,
                        kind: "cli-subcommand".to_string(),
                        line: v.start_position().row as u32 + 1,
                    });
                }
            }
        }
        // Wave 9 facts: public type surface, derive annotations, fields.
        if exported {
            let ek = match kind {
                SymbolKind::Class => "class",
                SymbolKind::Enum => "enum",
                SymbolKind::Interface => "trait",
                _ => "type",
            };
            ctx.facts.push(SemanticFact::PublicExport {
                symbol: name.clone(),
                kind: ek.to_string(),
            });
        }
        for d in &derives {
            ctx.facts.push(SemanticFact::Annotation {
                name: d.clone(),
                target: name.clone(),
            });
            // Contract subclass evidence: a Serialize/Deserialize derive on
            // the type is one side of a Serialization pair.
            if matches!(d.as_str(), "Serialize" | "Deserialize") {
                ctx.serde_impls.entry(name.clone()).or_default().insert(d.clone());
            }
        }
        if kind == SymbolKind::Class {
            self.record_struct_fields(node, &name, ctx, src);
            // Wave 11: `#[derive(Serialize, Deserialize)]` structs are
            // schema contracts (serde-import-gated).
            let has_serde = ctx
                .imports
                .iter()
                .any(|i| i.module == "serde" || i.module.starts_with("serde::"));
            if has_serde
                && derives.iter().any(|d| d == "Serialize")
                && derives.iter().any(|d| d == "Deserialize")
            {
                ctx.facts.push(SemanticFact::SchemaDefinition {
                    owner: name.clone(),
                    name: name.clone(),
                });
            }
        }
        // Trait bodies contain function declarations (contracts, default
        // impls); struct/enum bodies have none. Walk everything uniformly.
        self.walk_children(node, ctx, src);
    }

    fn walk_impl(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        // Receiver type of the impl (`impl Trait for Type` -> Type). Generic
        // parameters are dropped from the name.
        let type_name = node
            .child_by_field_name("type")
            .map(|t| clean(node_text(Some(t), src)))
            .unwrap_or_default();
        let type_name = type_name.split('<').next().unwrap_or("").trim().to_string();
        if type_name.is_empty() {
            self.walk_children(node, ctx, src);
            return;
        }
        // Contract subclass evidence: `impl Serialize for T` /
        // `impl Deserialize for T` (last path segment, so `serde::Serialize`
        // counts) are Serialization-pair sides around the type.
        if let Some(trait_node) = node.child_by_field_name("trait") {
            let trait_name = clean(node_text(Some(trait_node), src))
                .rsplit("::")
                .next()
                .unwrap_or("")
                .to_string();
            if matches!(trait_name.as_str(), "Serialize" | "Deserialize") {
                ctx.serde_impls
                    .entry(type_name.clone())
                    .or_default()
                    .insert(trait_name);
            }
        }
        ctx.scopes.push(Scope {
            name: type_name,
            is_impl: true,
        });
        self.walk_children(node, ctx, src);
        ctx.scopes.pop();
    }

    fn walk_mod(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let name = clean(node_text(node.child_by_field_name("name"), src));
        if name.is_empty() {
            self.walk_children(node, ctx, src);
            return;
        }
        let exported = self.is_exported(node);
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;
        let (doc, _attrs) = self.leading_annotations(node, src);
        ctx.symbols.push(Symbol {
            name: name.clone(),
            kind: SymbolKind::Module,
            signature: None,
            start_line,
            end_line,
            exported,
            docstring: doc,
            parent: None,
        });
        if exported {
            ctx.facts.push(SemanticFact::PublicExport {
                symbol: name.clone(),
                kind: "module".to_string(),
            });
        }
        // Module bodies are walked WITHOUT a scope push: nested items keep
        // plain names (symbols are per-file, so cross-module collisions in
        // one file are the only risk — rare, and deterministic).
        self.walk_children(node, ctx, src);
    }

    /// `const` / `static` items → Const symbols; `static` items additionally
    /// become mutable crate-level STATE facts.
    fn walk_const(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let name = clean(node_text(node.child_by_field_name("name"), src));
        if name.is_empty() {
            self.walk_children(node, ctx, src);
            return;
        }
        let exported = self.is_exported(node);
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;
        let (doc, _attrs) = self.leading_annotations(node, src);
        ctx.symbols.push(Symbol {
            name: name.clone(),
            kind: SymbolKind::Const,
            signature: None,
            start_line,
            end_line,
            exported,
            docstring: doc,
            parent: None,
        });
        if exported {
            ctx.facts.push(SemanticFact::PublicExport {
                symbol: name.clone(),
                kind: "const".to_string(),
            });
        }
        // `static` items are mutable crate-level state (a `const` is a
        // compile-time constant — not state).
        if node.kind() == "static_item" && !ctx.module_name.is_empty() {
            ctx.facts.push(SemanticFact::Field {
                owner: ctx.module_name.clone(),
                name: name.clone(),
                mutable: true,
            });
        }
        self.walk_children(node, ctx, src);
    }

    // ---- imports (`use` declarations) ----

    fn record_use(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let line = node.start_position().row as u32 + 1;
        let Some(arg) = node.child_by_field_name("argument") else {
            return;
        };
        // `pub use` binds a name into the crate's public API surface.
        let reexport = self.is_exported(node);
        self.emit_use(arg, "", reexport, ctx, src, line);
    }

    /// Emit one `use` binding as an Import. `prefix` accumulates enclosing
    /// `use a::b::{...}` paths. The module string is the full path as
    /// written (`a::b::c`); the bound name is the last path segment.
    /// `reexport` is true for `pub use` — the bound name is also a
    /// PublicExport.
    fn emit_use(&self, node: Node, prefix: &str, reexport: bool, ctx: &mut Ctx, src: &[u8], line: u32) {
        let join = |p: &str, s: &str| {
            if p.is_empty() {
                s.to_string()
            } else {
                format!("{p}::{s}")
            }
        };
        match node.kind() {
            "use_as_clause" => {
                let path = clean(node_text(
                    node.child_by_field_name("path"),
                    src,
                ));
                let alias = clean(node_text(node.child_by_field_name("alias"), src));
                if path.is_empty() || alias.is_empty() {
                    return;
                }
                let module = join(prefix, &path);
                let last = path.rsplit("::").next().unwrap_or(&path).to_string();
                ctx.imports.push(Import {
                    module,
                    names: vec![(alias.clone(), last)],
                    line,
                    r#type: ImportType::Member,
                });
                self.push_reexport(ctx, &alias, reexport);
            }
            "use_wildcard" => {
                let t = clean(node_text(Some(node), src)); // `a::b::*` or `*`
                let module = t.strip_suffix("::*").unwrap_or(&t).to_string();
                let module = if prefix.is_empty() {
                    module
                } else {
                    join(prefix, &module)
                };
                if !module.is_empty() {
                    ctx.imports.push(Import {
                        module,
                        names: Vec::new(),
                        line,
                        r#type: ImportType::Member,
                    });
                }
            }
            "use_list" => {
                let mut cursor = node.walk();
                for c in node.named_children(&mut cursor) {
                    self.emit_use(c, prefix, reexport, ctx, src, line);
                }
            }
            "scoped_use_list" => {
                let path = clean(node_text(node.child_by_field_name("path"), src));
                let Some(list) = node.child_by_field_name("list") else {
                    return;
                };
                let new_prefix = if prefix.is_empty() {
                    path
                } else {
                    join(prefix, &path)
                };
                let mut cursor = list.walk();
                for c in list.named_children(&mut cursor) {
                    self.emit_use(c, &new_prefix, reexport, ctx, src, line);
                }
            }
            "self" | "super" | "crate" => {
                // `use a::{self}` binds the enclosing module `a`, not a
                // path segment `a::self`; `use crate;` binds `crate`.
                let t = clean(node_text(Some(node), src));
                let module = if prefix.is_empty() { t } else { prefix.to_string() };
                let bound = module.rsplit("::").next().unwrap_or(&module).to_string();
                ctx.imports.push(Import {
                    module,
                    names: vec![(bound.clone(), bound.clone())],
                    line,
                    r#type: ImportType::Member,
                });
                self.push_reexport(ctx, &bound, reexport);
            }
            _ => {
                // identifier / scoped_identifier: the full path binds the
                // last segment (`use a::b::c` binds `c`).
                let t = clean(node_text(Some(node), src));
                if t.is_empty() {
                    return;
                }
                let module = join(prefix, &t);
                let bound = t.rsplit("::").next().unwrap_or(&t).to_string();
                ctx.imports.push(Import {
                    module,
                    names: vec![(bound.clone(), bound.clone())],
                    line,
                    r#type: ImportType::Member,
                });
                self.push_reexport(ctx, &bound, reexport);
            }
        }
    }

    /// `pub use` binds `name` into the crate's public API surface; the
    /// re-exported item's kind is unknown syntactically, so it is
    /// recorded as a generic `type`.
    fn push_reexport(&self, ctx: &mut Ctx, name: &str, reexport: bool) {
        if reexport && !name.is_empty() {
            ctx.facts.push(SemanticFact::PublicExport {
                symbol: name.to_string(),
                kind: "type".to_string(),
            });
        }
    }

    // ---- calls + store refs ----

    fn record_call(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        // clap builder API surface: `Command::new("x").arg(Arg::new(..).
        // long(..)).subcommand(Command::new("y"))` chains. Only the
        // outermost call of a chain is processed (each inner call climbs to
        // the same outer node).
        if chain_outer(node).id() == node.id() {
            self.record_clap_builder(node, ctx, src);
            self.record_axum_registration(node, ctx, src);
        }
        if let Some(fn_node) = node.child_by_field_name("function") {
            // Skip chained-call receivers (`a()()`, `a.b()()`): the inner
            // call is recorded on its own; the chain text is noise.
            let chained = fn_node.kind() == "call_expression"
                || (fn_node.kind() == "field_expression"
                    && fn_node
                        .child_by_field_name("value")
                        .map(|v| v.kind() == "call_expression")
                        .unwrap_or(false));
            if !chained {
                let mut callee = collapse(node_text(Some(fn_node), src));
                // `self::method()` is normalized to `self.method` so the
                // native resolver's self/this rule finds the sibling method.
                if fn_node.kind() == "scoped_identifier" {
                    if let Some(p) = fn_node.child_by_field_name("path") {
                        if clean(node_text(Some(p), src)) == "self" {
                            let m = clean(node_text(
                                fn_node.child_by_field_name("name"),
                                src,
                            ));
                            if !m.is_empty() {
                                callee = format!("self.{m}");
                            }
                        }
                    }
                }
                if !callee.is_empty() {
                    // Configuration reads: std::env::var("KEY") /
                    // std::env::var_os("KEY").
                    if matches!(callee.as_str(), "std::env::var" | "std::env::var_os")
                        || callee.ends_with("::env::var")
                        || callee.ends_with("::env::var_os")
                    {
                        if let Some(key) = first_string_arg(node, src) {
                            if let Some(owner) = ctx.caller() {
                                ctx.facts.push(SemanticFact::Configuration {
                                    owner,
                                    key,
                                });
                            }
                        }
                    }
                    // Wave 11: `serde_json::from_str::<T>(...)` validates a
                    // schema — the turbofish type names the schema target
                    // (serde_json-import-gated).
                    let base_callee = callee.split("::<").next().unwrap_or(&callee);
                    if (base_callee == "serde_json::from_str"
                        || base_callee.ends_with("::serde_json::from_str"))
                        && ctx
                            .imports
                            .iter()
                            .any(|i| i.module == "serde_json" || i.module.starts_with("serde_json::"))
                    {
                        if let Some(target) = turbofish_type(fn_node, src) {
                            if let Some(owner) = ctx.caller() {
                                ctx.facts.push(SemanticFact::SchemaValidation {
                                    owner,
                                    target,
                                });
                            }
                        }
                    }
                    let caller = ctx.caller();
                    let seq = ctx.call_seq.entry(caller.clone()).or_insert(0);
                    *seq += 1;
                    let (conditional, control_block, inside_loop, inside_try) = call_cfg(node);
                    ctx.calls.push(Call {
                        caller,
                        callee,
                        line: node.start_position().row as u32 + 1,
                        known_receiver: known_receiver(fn_node, src),
                        conditional,
                        lexical_order: *seq - 1,
                        control_block: control_block.map(str::to_string),
                        inside_loop,
                        inside_try,
                        awaited: call_is_awaited(node),
                        returns_value: call_returns_value(node),
                    });
                    self.record_store_ref(node, fn_node, ctx, src);
                }
            }
        }
        self.walk_children(node, ctx, src);
    }

    /// clap builder API (non-derive): `Command::new("name")` chains with
    /// `.arg(Arg::new("x").long("x").short('x'))` and
    /// `.subcommand(Command::new("sub"))` segments, on a `Command::new`
    /// root or on a variable holding one (`app.subcommand(...)`).
    ///
    /// Flags attach to the enclosing function symbol (the parser owner);
    /// each registered subcommand emits a `cli-subcommand` entrypoint on
    /// the owning function symbol so the atlas renders it.
    fn record_clap_builder(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let Some((root, _root_args, segs)) = clap_chain(node, src) else {
            return;
        };
        let root_cmd = root.ends_with("Command::new");
        if !root_cmd && !segs.iter().any(|(m, _)| m == "arg" || m == "subcommand") {
            return; // not a Command builder chain
        }
        let owner = ctx.caller();
        let line = node.start_position().row as u32 + 1;
        for (m, args) in &segs {
            match m.as_str() {
                "arg" => {
                    let Some(args) = args else { continue };
                    let Some(first) = args.named_child(0) else { continue };
                    let Some((aroot, _aargs, asegs)) = clap_chain(first, src) else {
                        continue;
                    };
                    if !aroot.ends_with("Arg::new") {
                        continue;
                    }
                    let Some(owner) = &owner else { continue };
                    for (am, aargs) in &asegs {
                        let flag = match am.as_str() {
                            "long" => string_arg(*aargs, src).map(|v| format!("--{v}")),
                            "short" => string_arg(*aargs, src).map(|v| format!("-{v}")),
                            _ => None,
                        };
                        if let Some(flag) = flag {
                            ctx.cli_flags.entry(owner.clone()).or_default().insert(flag);
                        }
                    }
                }
                "subcommand" => {
                    let Some(args) = args else { continue };
                    let Some(first) = args.named_child(0) else { continue };
                    let Some((sroot, sroot_args, _ssegs)) = clap_chain(first, src) else {
                        continue;
                    };
                    if !sroot.ends_with("Command::new") {
                        continue;
                    }
                    let Some(name) = string_arg(sroot_args, src) else {
                        continue;
                    };
                    // attach to the owning function symbol when the chain
                    // lives in one; otherwise fall back to the subcommand
                    // name (file-level entity).
                    let symbol = owner.clone().unwrap_or_else(|| name.clone());
                    ctx.entrypoints.push(Entrypoint {
                        symbol,
                        kind: "cli-subcommand".to_string(),
                        line,
                    });
                }
                _ => {}
            }
        }
    }

    fn record_store_ref(&self, call: Node, fn_node: Node, ctx: &mut Ctx, src: &[u8]) {
        if fn_node.kind() != "field_expression" {
            return;
        }
        let mut segs: Vec<String> = Vec::new();
        attribute_segments(fn_node, &mut segs, src);
        if segs.len() < 2 {
            return;
        }
        // `self.conn.execute(...)`: unwrap the instance prefix.
        if segs[0] == "self" && segs.len() >= 3 {
            segs.remove(0);
        }
        let store = segs[0].clone();
        if !STORE_RECEIVERS.contains(&store.as_str()) {
            return;
        }
        let op_name = segs.last().unwrap().clone();
        let Some(op) = classify_op(&op_name) else {
            return;
        };
        // mid-chain segment is the entity (table/model/collection) when present
        let mut target = if segs.len() >= 3 {
            Some(segs[segs.len() - 2].clone())
        } else {
            None
        };
        // string-literal keys/topics for redis/kafka-ish roots
        if STRING_TARGET_ROOTS.contains(&store.as_str()) {
            if let Some(s) = first_string_arg(call, src) {
                target = Some(s);
            }
        }
        // SQL sniffing for execute-family ops overrides op + target
        if matches!(op_name.as_str(), "execute" | "executemany" | "executescript") {
            if let Some(sql) = first_string_arg(call, src) {
                let (sniff_op, sniff_target) = sql_op_table(&sql);
                ctx.store_refs.push(StoreRef {
                    caller: ctx.caller(),
                    technology: technology_for(&store),
                    store,
                    op: sniff_op,
                    target: sniff_target,
                    line: call.start_position().row as u32 + 1,
                });
                return;
            }
        }
        ctx.store_refs.push(StoreRef {
            caller: ctx.caller(),
            technology: technology_for(&store),
            store,
            op,
            target,
            line: call.start_position().row as u32 + 1,
        });
    }

    /// axum route/middleware registrations:
    /// `Router::new().route(path, get(handler)).layer(mw)`. The receiver
    /// must be a `Router` constructor (`Router::new`, `Router::with_state`,
    /// `axum::Router::…`); the axum import gate is applied in
    /// `into_extracted` once imports are complete, so a plain method named
    /// `route` on another receiver is never a fact.
    fn record_axum_registration(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let Some((root, _root_args, segs)) = clap_chain(node, src) else {
            return;
        };
        let router_receiver = root == "Router"
            || root.starts_with("Router::")
            || root.starts_with("axum::Router::");
        if !router_receiver {
            return;
        }
        let Some(owner) = ctx.caller() else {
            return;
        };
        for (m, args) in &segs {
            let kind = match m.as_str() {
                "route" => "route",
                "layer" => "middleware",
                _ => continue,
            };
            let target = if kind == "route" {
                // the path literal names the registered contract
                match string_arg(*args, src) {
                    Some(p) => p,
                    None => continue,
                }
            } else {
                let t = args
                    .and_then(|a| a.named_child(0))
                    .map(|c| clean(node_text(Some(c), src)))
                    .unwrap_or_default();
                if t.is_empty() {
                    continue;
                }
                truncate_chars(&t, 120)
            };
            ctx.reg_candidates.push(SemanticFact::Registration {
                owner: owner.clone(),
                kind: kind.to_string(),
                target,
            });
        }
    }

    /// Struct fields (state surface). A field is mutable when its type
    /// uses an interior-mutability/atomic wrapper; plain fields are
    /// immutable under Rust's ownership rules.
    fn record_struct_fields(&self, node: Node, owner: &str, ctx: &mut Ctx, src: &[u8]) {
        const MUTABLE_TYPES: &[&str] = &[
            "Cell<",
            "RefCell<",
            "Mutex<",
            "RwLock<",
            "Atomic",
            "UnsafeCell<",
        ];
        // Wave 11: `#[serde(flatten)]` fields compose the struct's schema
        // from the flattened field type (serde-import-gated).
        let has_serde = ctx
            .imports
            .iter()
            .any(|i| i.module == "serde" || i.module.starts_with("serde::"));
        let mut cursor = node.walk();
        for c in node.named_children(&mut cursor) {
            if c.kind() != "field_declaration_list" {
                continue;
            }
            let mut pending: Vec<Node> = Vec::new();
            let mut c2 = c.walk();
            for f in c.named_children(&mut c2) {
                match f.kind() {
                    "attribute_item" => pending.push(f),
                    "field_declaration" => {
                        let fname = clean(node_text(f.child_by_field_name("name"), src));
                        if !fname.is_empty() {
                            let ftype = node_text(f.child_by_field_name("type"), src);
                            let mutable = MUTABLE_TYPES.iter().any(|t| ftype.contains(t));
                            ctx.facts.push(SemanticFact::Field {
                                owner: owner.to_string(),
                                name: fname,
                                mutable,
                            });
                        }
                        if has_serde
                            && pending
                                .iter()
                                .any(|a| attr_policy(*a, src).contains("flatten"))
                        {
                            if let Some(parent) = flatten_parent_type(f, src) {
                                ctx.facts.push(SemanticFact::SchemaComposition {
                                    owner: owner.to_string(),
                                    name: owner.to_string(),
                                    parent,
                                });
                            }
                        }
                        pending.clear();
                    }
                    _ => {}
                }
            }
        }
    }

    /// `env!("KEY")` / `option_env!("KEY")` configuration reads (macro
    /// invocation form; the `std::env::var` function form is handled in
    /// `record_call`). Body children are walked like before so calls
    /// inside macro arguments are still recorded.
    fn record_macro_invocation(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let mac = clean(node_text(node.child_by_field_name("macro"), src));
        if mac == "env" || mac == "option_env" {
            if let Some(key) = first_string_literal(node, src) {
                if let Some(owner) = ctx.caller() {
                    ctx.facts.push(SemanticFact::Configuration { owner, key });
                }
            }
        }
        self.walk_children(node, ctx, src);
    }
}

// ---------------------------------------------------------------------------
// Walker helpers
// ---------------------------------------------------------------------------

/// Innermost object of a field-expression chain (`self.conn.execute` ->
/// `self`).
fn callee_root(mut node: Node) -> Node {
    loop {
        match node.kind() {
            "field_expression" => match node.child_by_field_name("value") {
                Some(v) => node = v,
                None => return node,
            },
            "parenthesized_expression" => match node.named_child(0) {
                Some(inner) => node = inner,
                None => return node,
            },
            _ => return node,
        }
    }
}

/// Field-expression chain segments from outermost (root) to method.
fn attribute_segments(mut node: Node, out: &mut Vec<String>, src: &[u8]) {
    let mut stack: Vec<String> = Vec::new();
    loop {
        match node.kind() {
            "field_expression" => {
                let f = node_text(node.child_by_field_name("field"), src);
                if !f.is_empty() {
                    stack.push(f.to_string());
                }
                match node.child_by_field_name("value") {
                    Some(v) => node = v,
                    None => break,
                }
            }
            "identifier" | "self" | "super" | "crate" => {
                let t = node_text(Some(node), src);
                if !t.is_empty() {
                    stack.push(t.to_string());
                }
                break;
            }
            _ => break,
        }
    }
    out.extend(stack.into_iter().rev());
}

/// Outermost call of the method chain containing `node`
/// (`Command::new(x).arg(y)` -> the `.arg(y)` call). Non-chain calls return
/// themselves. tree-sitter-rust nests chains as
/// `call -> field_expression -> call -> ...`, so climbing goes through the
/// field_expression that is the enclosing call's `function`.
fn chain_outer(mut node: Node) -> Node {
    loop {
        let Some(parent) = node.parent() else {
            return node;
        };
        if parent.kind() == "field_expression" {
            let Some(gp) = parent.parent() else {
                return node;
            };
            let is_function = gp.kind() == "call_expression"
                && gp.child_by_field_name("function")
                    .map(|f| f.id() == parent.id())
                    .unwrap_or(false);
            if is_function {
                node = gp;
                continue;
            }
        }
        return node;
    }
}

/// One clap chain segment: the method name and its argument node.
type ChainSegment<'a> = (String, Option<Node<'a>>);
/// `(root_text, root_args, segments)` — root is the innermost function
/// text (`Command::new`, `Arg::new`, or a plain receiver identifier).
type ClapChain<'a> = (String, Option<Node<'a>>, Vec<ChainSegment<'a>>);

/// Method segments of a call chain, outermost call first:
/// `(method, arguments)` per segment.
fn clap_chain<'a>(node: Node<'a>, src: &'a [u8]) -> Option<ClapChain<'a>> {
    let mut segs: Vec<ChainSegment> = Vec::new();
    let mut cur = node;
    loop {
        if cur.kind() != "call_expression" {
            // chain root receiver (`app.subcommand(...)`, `foo().arg(...)`)
            // — its text is the root
            let root = collapse(node_text(Some(cur), src));
            return Some((root, None, segs));
        }
        let f = cur.child_by_field_name("function")?;
        match f.kind() {
            "field_expression" => {
                let m = collapse(node_text(f.child_by_field_name("field"), src));
                let args = cur.child_by_field_name("arguments");
                segs.push((m, args));
                cur = f.child_by_field_name("value")?;
            }
            // `foo().method()` — climb through the call receiver
            "call_expression" => cur = f,
            _ => {
                let root = collapse(node_text(Some(f), src));
                let root_args = cur.child_by_field_name("arguments");
                return Some((root, root_args, segs));
            }
        }
    }
}

/// First argument of an `arguments` node when it is a literal
/// (`"paging"`, `'P'`, `r#"x"#`), unquoted.
fn string_arg(args: Option<Node>, src: &[u8]) -> Option<String> {
    let first = args?.named_child(0)?;
    let t = node_text(Some(first), src).trim();
    let t = match first.kind() {
        "string_literal" => t.strip_prefix('"').and_then(|s| s.strip_suffix('"')),
        "char_literal" => t.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')),
        // raw strings: `r#"paging"#` -> `paging`
        "raw_string_literal" => {
            let t = t.strip_prefix("r#").unwrap_or(t);
            t.strip_suffix('#')
        }
        _ => None,
    }?;
    let t = t.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

/// The callee root is a local/imported binding or a known receiver when it
/// is a plain identifier, `self`, or a `self::`/`crate::`/`super::` path.
fn known_receiver(fn_node: Node, src: &[u8]) -> bool {
    match fn_node.kind() {
        "identifier" | "self" => true,
        "scoped_identifier" => {
            let p = clean(node_text(fn_node.child_by_field_name("path"), src));
            matches!(p.as_str(), "self" | "crate" | "super")
        }
        "generic_function" => fn_node
            .child_by_field_name("function")
            .map(|f| known_receiver(f, src))
            .unwrap_or(false),
        "field_expression" => {
            let root = callee_root(fn_node);
            matches!(root.kind(), "identifier" | "self")
        }
        "parenthesized_expression" => fn_node
            .named_child(0)
            .map(|f| known_receiver(f, src))
            .unwrap_or(false),
        _ => false,
    }
}

/// True when a function's return type mentions `Self` (`-> Self`,
/// `-> &mut Self`) — fluent builder chain evidence.
fn fn_returns_self_type(node: Node, src: &[u8]) -> bool {
    let Some(rt) = node.child_by_field_name("return_type") else {
        return false;
    };
    node_text(Some(rt), src).contains("Self")
}

fn signature(name: &str, fn_node: Node, src: &[u8]) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(params) = fn_node.child_by_field_name("parameters") {
        let mut cursor = params.walk();
        for c in params.named_children(&mut cursor) {
            if c.kind() == "self_parameter" {
                continue; // receivers are dropped, like python's self/cls
            }
            let t = collapse(node_text(Some(c), src));
            if !t.is_empty() {
                parts.push(t);
            }
        }
    }
    let mut sig = format!("fn {name}({})", parts.join(", "));
    if let Some(rt) = fn_node.child_by_field_name("return_type") {
        let t = clean(node_text(Some(rt), src))
            .trim_start_matches("->")
            .trim()
            .to_string();
        if !t.is_empty() {
            sig.push_str(" -> ");
            sig.push_str(&t);
        }
    }
    truncate_chars(&sig, 120)
}

/// Policy text of an attribute item, e.g. `#[retry(attempts = 3)]` ->
/// `retry(attempts = 3)`.
fn attr_policy(a: Node, src: &[u8]) -> String {
    let t = node_text(Some(a), src).trim();
    let t = t.strip_prefix("#[").unwrap_or(t);
    let t = t.strip_suffix(']').unwrap_or(t);
    clean(t)
}

/// Name of an attribute (`#[cfg(test)]` -> `cfg`, `#[tokio::test]` ->
/// `tokio::test`).
fn attr_name(a: Node, src: &[u8]) -> String {
    let p = attr_policy(a, src);
    let end = p
        .find(|c: char| c == '(' || c.is_whitespace() || c == ']')
        .unwrap_or(p.len());
    p[..end].to_string()
}

/// First type name of a turbofish type argument list
/// (`serde_json::from_str::<User>` → `User`; `Foo<T>` → `Foo`).
fn turbofish_type(fn_node: Node, src: &[u8]) -> Option<String> {
    let ta = fn_node.child_by_field_name("type_arguments")?;
    let mut cursor = ta.walk();
    for child in ta.named_children(&mut cursor) {
        let t = clean(node_text(Some(child), src));
        if t.is_empty() {
            continue;
        }
        let base = t.split('<').next().unwrap_or(&t).trim();
        if base.is_empty() {
            continue;
        }
        let base = base.rsplit("::").next().unwrap_or(base).trim();
        if base.is_empty() {
            continue;
        }
        return Some(base.to_string());
    }
    None
}

/// Field type of a `#[serde(flatten)]` field as the composed parent schema
/// name: strips an outer `Option<...>`, rejects qualified/generic types
/// (only plain local type names are resolvable).
fn flatten_parent_type(f: Node, src: &[u8]) -> Option<String> {
    let mut t = clean(node_text(f.child_by_field_name("type"), src));
    if t.starts_with("Option<") && t.ends_with('>') {
        t = t["Option<".len()..t.len() - 1].trim().to_string();
    }
    if t.is_empty() || t.contains("::") || t.contains('<') || t.contains('>') {
        return None;
    }
    Some(t)
}

/// Derive names of a `#[derive(...)]` attribute run, last path segment
/// only (`#[derive(clap::Parser)]` -> `Parser`).
fn derive_names(attrs: &[Node], src: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in attrs {
        if attr_name(*a, src) != "derive" {
            continue;
        }
        let p = attr_policy(*a, src);
        let Some(inner) = p.strip_prefix("derive(").and_then(|s| s.strip_suffix(')')) else {
            continue;
        };
        for part in inner.split(',') {
            let t = clean(part);
            let t = t.rsplit("::").next().unwrap_or(&t).trim().to_string();
            if !t.is_empty() {
                out.push(t);
            }
        }
    }
    out
}

/// True when an `#[arg(...)]` policy mentions `long` (explicit or
/// bare, e.g. `long = "port"` / `short, long`).
fn attr_has_long(policy: &str) -> bool {
    let body = policy
        .trim_start_matches("arg")
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')');
    body.split(',').any(|t| t.trim().starts_with("long"))
}

/// Explicit `long = "port"` value, when present.
fn attr_long_value(policy: &str) -> Option<String> {
    let body = policy
        .trim_start_matches("arg")
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')');
    for t in body.split(',') {
        let t = t.trim();
        if let Some(rest) = t.strip_prefix("long") {
            let rest = rest.trim();
            if let Some(v) = rest.strip_prefix('=') {
                let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
                if !v.is_empty() {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// `--flag` names from `#[arg(...)]` attributes on clap fields. In the
/// tree-sitter-rust grammar the attributes precede their field inside the
/// enclosing `field_declaration_list`; each attribute applies to the next
/// field_declaration.
fn clap_arg_flags(lists: &[Node], src: &[u8]) -> Vec<String> {
    let mut flags: Vec<String> = Vec::new();
    for list in lists {
        let mut pending: Vec<Node> = Vec::new();
        let mut cursor = list.walk();
        for c in list.named_children(&mut cursor) {
            match c.kind() {
                "attribute_item" => pending.push(c),
                "field_declaration" => {
                    let fname = clean(node_text(c.child_by_field_name("name"), src));
                    for a in pending.drain(..) {
                        let an = attr_name(a, src);
                        if an != "arg" && !an.ends_with("::arg") {
                            continue;
                        }
                        let p = attr_policy(a, src);
                        if !attr_has_long(&p) || fname.is_empty() {
                            continue;
                        }
                        match attr_long_value(&p) {
                            Some(v) => flags.push(format!("--{v}")),
                            None => flags.push(format!("--{fname}")),
                        }
                    }
                }
                _ => {}
            }
        }
    }
    flags
}

/// Deterministic fact sort key: (owning symbol, kind/name). PublicExport
/// sorts by (symbol, kind); the other families by their owner-ish field
/// and name, so facts group by owner and are stable across runs.
fn fact_key(f: &SemanticFact) -> (String, String) {
    match f {
        SemanticFact::PublicExport { symbol, kind } => (symbol.clone(), kind.clone()),
        SemanticFact::Annotation { name, target } => (target.clone(), name.clone()),
        SemanticFact::Field { owner, name, .. } => (owner.clone(), name.clone()),
        SemanticFact::Registration { owner, kind, .. } => (owner.clone(), kind.clone()),
        SemanticFact::Configuration { owner, key } => (owner.clone(), key.clone()),
        SemanticFact::Callback { owner, callback } => (owner.clone(), callback.clone()),
        SemanticFact::SchemaDefinition { owner, name } => (owner.clone(), name.clone()),
        SemanticFact::SchemaComposition { owner, name, parent } => {
            (owner.clone(), format!("{name}<:{parent}"))
        }
        SemanticFact::SchemaValidation { owner, target } => (owner.clone(), target.clone()),
        SemanticFact::ReactiveState { owner, name, access } => {
            (owner.clone(), format!("{name}:{access}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ImportType, StoreOp, SymbolKind, TestKind};

    fn extract(src: &str) -> ExtractedFile {
        let f = SourceFile::new("test.rs", src);
        RustExtractor::default().extract(&f)
        }

    fn find_symbol<'a>(ef: &'a ExtractedFile, name: &str) -> &'a Symbol {
        ef.symbols
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("symbol {name} not found"))
    }

    #[test]
    fn symbols_methods_signatures() {
        let ef = extract(
            r#"use std::collections::HashMap;

const MAX: usize = 10;
static FLAG: bool = true;

/// A job service.
pub struct Service {
    name: String,
}

/// Handles jobs.
pub enum JobState {
    Pending,
    Done,
}

/// Persistence contract.
pub trait Store {
    fn save(&self, job: &str);
}

mod internal {
    fn helper() {}
}

impl Service {
    /// Create a service.
    pub fn new(name: &str) -> Service {
        Service { name: name.to_string() }
    }

    fn run(&self) -> Result<(), String> {
        Ok(())
    }
}
"#,
        );
        assert_eq!(ef.symbols.len(), 11);
        // `static FLAG` is module-level state: the file-stem module symbol
        // owns it (kind Module, not exported).
        let module = find_symbol(&ef, "test");
        assert_eq!(module.kind, SymbolKind::Module);

        let max = find_symbol(&ef, "MAX");
        assert_eq!(max.kind, SymbolKind::Const);
        assert!(!max.exported);
        assert_eq!(max.start_line, 3);
        assert_eq!(max.end_line, 3);

        let flag = find_symbol(&ef, "FLAG");
        assert_eq!(flag.kind, SymbolKind::Const);

        let svc = find_symbol(&ef, "Service");
        assert_eq!(svc.kind, SymbolKind::Class);
        assert!(svc.exported);
        assert_eq!(svc.signature, None);
        assert_eq!(svc.docstring.as_deref(), Some("A job service."));

        let st = find_symbol(&ef, "JobState");
        assert_eq!(st.kind, SymbolKind::Enum);
        assert_eq!(st.docstring.as_deref(), Some("Handles jobs."));

        let tr = find_symbol(&ef, "Store");
        assert_eq!(tr.kind, SymbolKind::Interface);
        assert!(tr.exported);

        // trait method declarations (no body) are recorded too; the
        // receiver is dropped from the signature like method symbols
        let save = find_symbol(&ef, "save");
        assert_eq!(save.kind, SymbolKind::Function);
        assert!(!save.exported);
        assert_eq!(save.signature.as_deref(), Some("fn save(job: &str)"));

        let m = find_symbol(&ef, "internal");
        assert_eq!(m.kind, SymbolKind::Module);
        assert!(!m.exported);

        let helper = find_symbol(&ef, "helper");
        assert_eq!(helper.kind, SymbolKind::Function);
        assert!(!helper.exported);

        let new = find_symbol(&ef, "Service.new");
        assert_eq!(new.kind, SymbolKind::Method);
        assert!(new.exported);
        assert_eq!(new.parent.as_deref(), Some("Service"));
        assert_eq!(new.docstring.as_deref(), Some("Create a service."));
        assert_eq!(new.signature.as_deref(), Some("fn new(name: &str) -> Service"));

        let run = find_symbol(&ef, "Service.run");
        assert_eq!(run.kind, SymbolKind::Method);
        assert!(!run.exported);
        assert_eq!(run.parent.as_deref(), Some("Service"));
        // receiver dropped from signature
        assert_eq!(run.signature.as_deref(), Some("fn run() -> Result<(), String>"));
    }

    #[test]
    fn imports_all_forms() {
        let ef = extract(
            "use a::b::c;\nuse x::y as z;\nuse std::{collections::HashMap, io::Write as IoWrite};\nuse serde::*;\nuse super::config;\nuse crate::domain::{self, Service};\n",
        );
        let imps = &ef.imports;
        assert_eq!(imps.len(), 8);

        assert_eq!(imps[0].module, "a::b::c");
        assert_eq!(imps[0].names, vec![("c".into(), "c".into())]);
        assert_eq!(imps[0].r#type, ImportType::Member);
        assert_eq!(imps[0].line, 1);

        assert_eq!(imps[1].module, "x::y");
        assert_eq!(imps[1].names, vec![("z".into(), "y".into())]);

        assert_eq!(imps[2].module, "std::collections::HashMap");
        assert_eq!(imps[2].names, vec![("HashMap".into(), "HashMap".into())]);

        assert_eq!(imps[3].module, "std::io::Write");
        assert_eq!(imps[3].names, vec![("IoWrite".into(), "Write".into())]);

        // wildcard: no bound names
        assert_eq!(imps[4].module, "serde");
        assert!(imps[4].names.is_empty());

        assert_eq!(imps[5].module, "super::config");
        assert_eq!(imps[5].names, vec![("config".into(), "config".into())]);

        // `use crate::domain::{self, Service}` -> two imports
        assert_eq!(imps[6].module, "crate::domain");
        assert_eq!(imps[6].names, vec![("domain".into(), "domain".into())]);
        assert_eq!(imps[7].module, "crate::domain::Service");
        assert_eq!(imps[7].names, vec![("Service".into(), "Service".into())]);
    }

    #[test]
    fn calls_and_receivers() {
        let ef = extract(
            r#"fn helper() -> i32 { 1 }

struct Svc;

impl Svc {
    fn run(&self) {
        helper();
        self.do_it(1);
        self::other();
    }
    fn do_it(&self, x: i32) {}
    fn other() {}
}
"#,
        );
        let calls = &ef.calls;
        assert_eq!(calls.len(), 3);
        // document order
        assert_eq!(calls[0].caller.as_deref(), Some("Svc.run"));
        assert_eq!(calls[0].callee, "helper");
        assert!(calls[0].known_receiver);
        assert_eq!(calls[0].line, 7);

        assert_eq!(calls[1].caller.as_deref(), Some("Svc.run"));
        assert_eq!(calls[1].callee, "self.do_it");
        assert!(calls[1].known_receiver);

        // `self::other()` is normalized to `self.other`
        assert_eq!(calls[2].caller.as_deref(), Some("Svc.run"));
        assert_eq!(calls[2].callee, "self.other");
        assert!(calls[2].known_receiver);
    }

    #[test]
    fn calls_chains_and_unknown_receivers() {
        let ef = extract(
            r#"fn f() {
    obj.method();
    get_client().send();
    let x = v.unwrap();
    Service::new();
    (factory)(1);
}
"#,
        );
        let calls = &ef.calls;
        assert_eq!(calls.len(), 5);
        assert_eq!(calls[0].callee, "obj.method");
        assert!(calls[0].known_receiver);
        assert_eq!(calls[1].callee, "get_client");
        assert!(calls[1].known_receiver);
        // `.send()` on a call receiver is skipped (chained)
        assert_eq!(calls[2].callee, "v.unwrap");
        assert!(calls[2].known_receiver);
        assert_eq!(calls[3].callee, "Service::new");
        assert!(!calls[3].known_receiver);
        assert_eq!(calls[4].callee, "(factory)");
        assert!(calls[4].known_receiver);
    }

    #[test]
    fn store_refs_sql_and_clients() {
        let ef = extract(
            r#"fn worker(conn: &str, redis: &str, client: &str, svc: &str) {
    conn.execute("INSERT INTO jobs (id) VALUES (?)", &["1"]);
    conn.execute("SELECT * FROM users");
    redis.set("k", "v");
    client.fetch_data();
    svc.ingest("x");
}
"#,
        );
        let refs = &ef.store_refs;
        assert_eq!(refs.len(), 3);
        let caller = Some("worker".to_string());

        assert_eq!(refs[0].store, "conn");
        assert_eq!(refs[0].technology.as_deref(), Some("sql"));
        assert_eq!(refs[0].op, StoreOp::Write);
        assert_eq!(refs[0].target.as_deref(), Some("jobs"));
        assert_eq!(refs[0].caller, caller);

        assert_eq!(refs[1].store, "conn");
        assert_eq!(refs[1].op, StoreOp::Query);
        assert_eq!(refs[1].target.as_deref(), Some("users"));

        assert_eq!(refs[2].store, "redis");
        assert_eq!(refs[2].technology.as_deref(), Some("redis"));
        assert_eq!(refs[2].op, StoreOp::Write);
        assert_eq!(refs[2].target.as_deref(), Some("k"));

        // unknown receivers are not flagged
        assert!(!refs.iter().any(|r| r.store == "client"));
        assert!(!refs.iter().any(|r| r.store == "svc"));
    }

    #[test]
    fn retry_attributes() {
        let ef = extract(
            r#"#[retry(attempts = 3)]
fn a() {}

#[backoff(on = "transient")]
fn b() {}

#[derive(Debug)]
struct C;

impl C {
    /// Retrying method.
    #[retry(max = 5)]
    fn go(&self) {}
}
"#,
        );
        let retries = &ef.retries;
        assert_eq!(retries.len(), 3);
        assert_eq!(retries[0].symbol, "a");
        assert_eq!(retries[0].policy, "retry(attempts = 3)");
        assert_eq!(retries[0].line, 1);
        assert_eq!(retries[1].symbol, "b");
        assert_eq!(retries[1].policy, "backoff(on = \"transient\")");
        assert_eq!(retries[2].symbol, "C.go");
        assert_eq!(retries[2].policy, "retry(max = 5)");
    }

    #[test]
    fn tests_detection() {
        let ef = extract(
            r#"#[test]
fn it_works() {}

#[tokio::test]
async fn it_works_async() {}

fn plain() {}
"#,
        );
        let tests = &ef.tests;
        assert_eq!(tests.len(), 2);
        assert_eq!(tests[0].name, "it_works");
        assert_eq!(tests[0].symbol.as_deref(), Some("it_works"));
        assert_eq!(tests[0].kind, TestKind::Unit);
        assert_eq!(tests[0].line, 1);
        assert_eq!(tests[1].name, "it_works_async");
    }

    #[test]
    fn entrypoint_main() {
        let ef = extract("fn main() {\n    run();\n}\nfn run() {}\n");
        let eps = &ef.entrypoints;
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].symbol, "main");
        assert_eq!(eps[0].kind, "bin");
        assert_eq!(eps[0].line, 1);

        // a method named main is not an entrypoint
        let ef2 = extract("struct S;\nimpl S {\n    fn main(&self) {}\n}\n");
        assert!(ef2.entrypoints.is_empty());
    }

    #[test]
    fn clap_subcommands_and_flags() {
        let ef = extract(
            r#"use clap::{Parser, Subcommand};

/// demo CLI
#[derive(Parser)]
struct Cli {
    /// paged output
    #[arg(long)]
    paging: bool,

    /// output format
    #[arg(short, long)]
    format: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// serve requests
    Serve {
        #[arg(long = "port", default_value_t = 8080)]
        port: u16,
    },
    /// deploy the build
    Deploy {
        #[arg(short, long)]
        env: String,
    },
}

fn main() {
    let _cli = Cli::parse();
}
"#,
        );
        let eps = &ef.entrypoints;
        assert_eq!(eps.len(), 3, "eps: {eps:?}");
        // fn main + the two subcommand variants
        assert_eq!(eps[0].symbol, "Serve");
        assert_eq!(eps[0].kind, "cli-subcommand");
        assert_eq!(eps[1].symbol, "Deploy");
        assert_eq!(eps[1].kind, "cli-subcommand");
        assert_eq!(eps[2].symbol, "main");
        assert_eq!(eps[2].kind, "bin");
        // flags: struct fields (explicit long + bare long) on Cli;
        // variant fields on Command; sorted + deduped
        let cli = ef.cli_flags.get("Cli").expect("flags on Cli");
        assert_eq!(cli, &["--format", "--paging"]);
        let cmd = ef.cli_flags.get("Command").expect("flags on Command");
        assert_eq!(cmd, &["--env", "--port"]);
        assert_eq!(ef.cli_flags.len(), 2);
    }

    #[test]
    fn clap_derives_only() {
        // derive without Parser/Subcommand emits nothing
        let ef = extract(
            r#"#[derive(Debug)]
struct Plain {
    #[arg(long)]
    paging: bool,
}
"#,
        );
        assert!(ef.entrypoints.is_empty());
        assert!(ef.cli_flags.is_empty());
    }


    #[test]
    fn clap_builder_chains() {
        let ef = extract(
            r#"use clap::{Arg, Command};

/// Build the demo CLI.
fn build_cli() -> Command {
    Command::new("demo")
        .version("1.0")
        .arg(Arg::new("paging").long("paging"))
        .arg(Arg::new("theme").short('t').long("theme"))
        .arg(Arg::new("FILE"))
        .subcommand(
            Command::new("serve")
                .about("Serve requests")
                .arg(Arg::new("port").short('p').long("port")),
        )
        .subcommand(Command::new("deploy"))
}
"#,
        );
        // flags attach to the owning function symbol (parser owner)
        let flags = ef.cli_flags.get("build_cli").expect("flags on build_cli");
        assert_eq!(flags, &["--paging", "--port", "--theme", "-p", "-t"]);
        // FILE has no long/short -> no flag
        assert!(!flags.iter().any(|f| f.contains("FILE")));
        // each registered subcommand emits a cli-subcommand entrypoint on
        // the owning function symbol
        let eps = &ef.entrypoints;
        assert_eq!(eps.len(), 2, "eps: {eps:?}");
        assert_eq!(eps[0].symbol, "build_cli");
        assert_eq!(eps[0].kind, "cli-subcommand");
        assert_eq!(eps[1].symbol, "build_cli");
        assert_eq!(eps[1].kind, "cli-subcommand");
    }

    #[test]
    fn clap_builder_var_receiver() {
        // `app.subcommand(...)` / `app.arg(...)` on a variable holding the
        // Command; nested subcommand chains contribute their own flags.
        let ef = extract(
            r#"use clap::{Arg, Command};

fn build(mut app: Command) -> Command {
    app.subcommand(Command::new("cache").arg(Arg::new("build").long("build")))
        .arg(Arg::new("paging").long("paging"))
}
"#,
        );
        let flags = ef.cli_flags.get("build").expect("flags on build");
        assert_eq!(flags, &["--build", "--paging"]);
        let eps = &ef.entrypoints;
        assert_eq!(eps.len(), 1, "eps: {eps:?}");
        assert_eq!(eps[0].symbol, "build");
        assert_eq!(eps[0].kind, "cli-subcommand");
    }

    #[test]
    fn clap_builder_non_clap_chains_ignored() {
        // plain method chains without Arg::new/Command::new arguments are
        // not clap surface
        let ef = extract(
            r#"struct S;
impl S {
    fn arg(&self, _x: i32) {}
    fn run(&self) {
        self.arg(42);
    }
}
"#,
        );
        assert!(ef.cli_flags.is_empty());
        assert!(ef.entrypoints.is_empty());
    }

    #[test]
    fn clap_builder_macro_name_no_entrypoint() {
        // `Command::new(crate_name!())` has no literal name: subcommand
        // detection must not panic and emits nothing for the root
        let ef = extract(
            r#"use clap::{Arg, Command};
fn build() -> Command {
    Command::new(crate_name!()).arg(Arg::new("paging").long("paging"))
}
"#,
        );
        let flags = ef.cli_flags.get("build").expect("flags on build");
        assert_eq!(flags, &["--paging"]);
        assert!(ef.entrypoints.is_empty());
    }

    #[test]
    fn docstrings_first_paragraph() {
        let ef = extract(
            r#"/// Sum two.
///
/// More here.
fn f() {}

/** Class doc. */
struct A {}

// not a doc
fn g() {}
"#,
        );
        let f = find_symbol(&ef, "f");
        assert_eq!(f.docstring.as_deref(), Some("Sum two."));
        let a = find_symbol(&ef, "A");
        assert_eq!(a.docstring.as_deref(), Some("Class doc."));
        let g = find_symbol(&ef, "g");
        assert_eq!(g.docstring, None);
    }

    #[test]
    fn malformed_input_does_not_panic() {
        let cases = [
            "fn broken(:",
            "pub fn (\n",
            "impl {}\n",
            "use ;\n",
            "#[retry(]\nfn x() {}\n",
            "\u{0}\u{1}\u{2}\u{ff}",
            "struct S { fn\n",
            "fn main( {\n",
            "use a::{b, c as\n",
            "/// doc\n",
            "fn f() { Router::new().route(; }\n",
            "fn f() { Router::new().route(\"a\", get(h)).layer( }\n",
            "fn f() { env!( }\n",
            "pub use a::{b as\n",
        ];
        for c in cases {
            let _ = extract(c);
        }
    }

    #[test]
    fn facts_public_exports_annotations_fields() {
        let ef = extract(
            r#"use std::time::Duration;

/// Re-exported duration type.
pub use std::time::Duration as StdDuration;

pub const MAX: usize = 10;

/// A job service.
#[derive(Debug, Serialize)]
pub struct Service {
    pub name: String,
    cache: std::sync::RwLock<Vec<String>>,
}

#[derive(Clone)]
pub enum Mode {
    Fast,
}

pub trait Store {
    fn save(&self, x: &str);
}

impl Service {
    pub fn new(name: &str) -> Service {
        Service { name: name.into(), cache: std::sync::RwLock::new(Vec::new()) }
    }
    fn run(&self) {}
}

fn private_fn() {}
"#,
        );
        // public API surface: pub items, pub use re-export, pub method
        let exports: Vec<(&str, &str)> = ef
            .facts
            .iter()
            .filter_map(|f| match f {
                SemanticFact::PublicExport { symbol, kind } => {
                    Some((symbol.as_str(), kind.as_str()))
                }
                _ => None,
            })
            .collect();
        for want in ["StdDuration", "MAX", "Service", "Mode", "Store", "Service.new"] {
            assert!(
                exports.iter().any(|(s, _)| *s == want),
                "missing export {want}: {exports:?}"
            );
        }
        // re-exports are recorded with a generic `type` kind
        assert!(exports.contains(&("StdDuration", "type")));
        assert!(exports.contains(&("Service", "class")));
        assert!(exports.contains(&("Service.new", "method")));
        assert!(!exports.iter().any(|(s, _)| *s == "run"));
        assert!(!exports.iter().any(|(s, _)| *s == "private_fn"));

        // derive macros annotate their target
        let anns: Vec<(&str, &str)> = ef
            .facts
            .iter()
            .filter_map(|f| match f {
                SemanticFact::Annotation { name, target } => {
                    Some((name.as_str(), target.as_str()))
                }
                _ => None,
            })
            .collect();
        assert!(anns.contains(&("Debug", "Service")), "anns: {anns:?}");
        assert!(anns.contains(&("Serialize", "Service")));
        assert!(anns.contains(&("Clone", "Mode")));

        // fields with the interior-mutability flag
        let fields: Vec<(&str, &str, bool)> = ef
            .facts
            .iter()
            .filter_map(|f| match f {
                SemanticFact::Field { owner, name, mutable } => {
                    Some((owner.as_str(), name.as_str(), *mutable))
                }
                _ => None,
            })
            .collect();
        assert!(
            fields.contains(&("Service", "name", false)),
            "fields: {fields:?}"
        );
        assert!(fields.contains(&("Service", "cache", true)));
    }

    #[test]
    fn facts_axum_registrations_require_framework() {
        // axum import + Router::new receiver -> registrations
        let ef = extract(
            r#"use axum::Router;

fn build_router() -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/users", get(list_users))
        .layer(tower_http::trace::TraceLayer::new())
}

fn health() {}
fn list_users() {}
"#,
        );
        let regs: Vec<(&str, &str, &str)> = ef
            .facts
            .iter()
            .filter_map(|f| match f {
                SemanticFact::Registration { owner, kind, target } => {
                    Some((owner.as_str(), kind.as_str(), target.as_str()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(regs.len(), 3, "regs: {regs:?}");
        assert!(regs.contains(&("build_router", "route", "/health")));
        assert!(regs.contains(&("build_router", "route", "/users")));
        assert!(regs.contains(&(
            "build_router",
            "middleware",
            "tower_http::trace::TraceLayer::new()"
        )));

        // no axum import -> registrations are not facts
        let ef2 = extract(
            r#"fn build_router() {
    Router::new().route("/health", get(health));
}
"#,
        );
        assert!(
            !ef2.facts
                .iter()
                .any(|f| matches!(f, SemanticFact::Registration { .. })),
            "no axum import must suppress registrations"
        );

        // a plain receiver that is not a Router constructor -> nothing,
        // even with the axum import
        let ef3 = extract(
            r#"use axum::Router;
fn f(app: Router) {
    app.route("/x", get(handler));
}
"#,
        );
        assert!(
            !ef3.facts
                .iter()
                .any(|f| matches!(f, SemanticFact::Registration { .. })),
            "non-Router receiver must not register"
        );
    }

    #[test]
    fn facts_configuration_env_reads() {
        let ef = extract(
            r#"fn service_port() -> String {
    std::env::var("PORT").unwrap_or_else(|_| "8080".to_string())
}

fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn opt() -> Option<&'static str> {
    option_env!("SCC_FEATURE")
}
"#,
        );
        let cfgs: Vec<(&str, &str)> = ef
            .facts
            .iter()
            .filter_map(|f| match f {
                SemanticFact::Configuration { owner, key } => {
                    Some((owner.as_str(), key.as_str()))
                }
                _ => None,
            })
            .collect();
        assert!(
            cfgs.contains(&("service_port", "PORT")),
            "cfgs: {cfgs:?}"
        );
        assert!(
            cfgs.contains(&("version", "CARGO_PKG_VERSION")),
            "cfgs: {cfgs:?}"
        );
        assert!(cfgs.contains(&("opt", "SCC_FEATURE")), "cfgs: {cfgs:?}");
    }

    #[test]
    fn facts_sorted_and_deduped() {
        let ef = extract(
            r#"use axum::Router;
pub struct A;
pub fn z() {}
pub fn a() {}
#[derive(Debug)]
pub struct B { x: i32 }
fn cfg() { env!("K"); std::env::var("K"); }
fn router() { Router::new().route("/a", get(a)).route("/b", get(b)); }
"#,
        );
        let keys: Vec<(String, String)> = ef.facts.iter().map(fact_key).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "facts must be sorted");

        // identical configuration reads from the same owner dedupe
        let cfgs = ef
            .facts
            .iter()
            .filter(|f| matches!(f, SemanticFact::Configuration { .. }))
            .count();
        assert_eq!(cfgs, 1, "deduped config reads: {cfgs}");
    }

    #[test]
    fn deterministic_output() {
        let src = "use std::collections::HashMap;\n\n#[retry(attempts = 3)]\nfn retry_me() {}\n\nfn main() {\n    retry_me();\n}\n";
        let a = extract(src);
        let b = extract(src);
        let ja = serde_json::to_string(&a).unwrap();
        let jb = serde_json::to_string(&b).unwrap();
        assert_eq!(ja, jb);
    }

    #[test]
    fn facts_impl_factory_and_builder_plus_static_state() {
        // tokio-style `Builder::new_multi_thread()` factory +
        // `with_worker_threads(mut self) -> Self` builder chain; a
        // crate-level `static` is mutable module state.
        let ef = extract(
            r#"static MAX_BLOCKING: usize = 512;

pub struct Builder { threads: usize }

impl Builder {
    pub fn new() -> Builder { Builder { threads: 1 } }
    pub fn new_multi_thread() -> Builder { Builder { threads: 4 } }
    pub fn with_worker_threads(mut self, n: usize) -> Self { self.threads = n; self }
    pub fn set_name(&mut self, _n: &str) -> &mut Self { self }
    fn helper() {}
}
"#,
        );
        let regs: Vec<(String, String, String)> = ef
            .facts
            .iter()
            .filter_map(|f| match f {
                SemanticFact::Registration { owner, kind, target } => {
                    Some((owner.clone(), kind.clone(), target.clone()))
                }
                _ => None,
            })
            .collect();
        assert!(
            regs.contains(&("Builder".into(), "factory".into(), "Builder".into())),
            "impl new factory missing: {regs:?}"
        );
        assert!(
            regs.contains(&("Builder".into(), "builder".into(), "Builder".into())),
            "fluent with_/set_ builder missing: {regs:?}"
        );
        assert_eq!(regs.iter().filter(|(o, k, _)| o == "Builder" && k == "factory").count(), 1, "new + new_multi_thread dedupe to one factory fact: {regs:?}");
        let fields: Vec<(String, String, bool)> = ef
            .facts
            .iter()
            .filter_map(|f| match f {
                SemanticFact::Field { owner, name, mutable } => {
                    Some((owner.clone(), name.clone(), *mutable))
                }
                _ => None,
            })
            .collect();
        assert!(
            fields.contains(&("test".into(), "MAX_BLOCKING".into(), true)),
            "static item state missing: {fields:?}"
        );
        assert!(
            ef.symbols
                .iter()
                .any(|s| s.name == "test" && s.kind == SymbolKind::Module),
            "module symbol missing: {:?}",
            ef.symbols.iter().map(|s| s.name.as_str()).collect::<Vec<_>>()
        );
    }

    // -------------------------------------------------------------------
    // Wave 11: serde schema contracts
    // -------------------------------------------------------------------

    #[test]
    fn wave11_serde_schema_definition_flatten_validation() {
        let ef = extract(
            "use serde::{Deserialize, Serialize};\nuse serde_json;\n\n#[derive(Serialize, Deserialize)]\npub struct Base {\n    pub id: u64,\n}\n\n#[derive(Serialize, Deserialize)]\npub struct User {\n    #[serde(flatten)]\n    pub base: Base,\n    pub name: String,\n}\n\npub fn load(raw: &str) -> Result<User, serde_json::Error> {\n    serde_json::from_str::<User>(raw)\n}\n",
        );
        let schemas: Vec<&SemanticFact> = ef
            .facts
            .iter()
            .filter(|f| matches!(
                f,
                SemanticFact::SchemaDefinition { .. }
                    | SemanticFact::SchemaComposition { .. }
                    | SemanticFact::SchemaValidation { .. }
            ))
            .collect();
        assert!(
            schemas.iter().any(|f| matches!(
                f,
                SemanticFact::SchemaDefinition { owner, name } if owner == "User" && name == "User"
            )),
            "serde struct must be a SchemaDefinition: {schemas:?}"
        );
        assert!(
            schemas.iter().any(|f| matches!(
                f,
                SemanticFact::SchemaDefinition { owner, name } if owner == "Base" && name == "Base"
            )),
            "serde base struct must be a SchemaDefinition: {schemas:?}"
        );
        assert!(
            schemas.iter().any(|f| matches!(
                f,
                SemanticFact::SchemaComposition { owner, name, parent } if owner == "User" && name == "User" && parent == "Base"
            )),
            "serde flatten must be a SchemaComposition: {schemas:?}"
        );
        assert!(
            schemas.iter().any(|f| matches!(
                f,
                SemanticFact::SchemaValidation { owner, target } if owner == "load" && target == "User"
            )),
            "serde_json::from_str::<T> must emit SchemaValidation: {schemas:?}"
        );
        // no serde import → no schema facts
        let ef2 = extract(
            "pub struct Plain {\n    pub id: u64,\n}\n",
        );
        assert!(
            !ef2.facts.iter().any(|f| matches!(
                f,
                SemanticFact::SchemaDefinition { .. }
                    | SemanticFact::SchemaComposition { .. }
                    | SemanticFact::SchemaValidation { .. }
            )),
            "no serde import → no schema facts: {:?}",
            ef2.facts
        );
    }
}
