//! Python language extractor (tree-sitter based).
//!
//! Pure, deterministic extraction: `(path, content) -> ExtractedFile`.
//! Syntax-level only; cross-file resolution happens in `resolve.rs`.

use crate::model::{
    Call, Entrypoint, ExtractedFile, Import, ImportType, LanguageExtractor, Retry, Route,
    SourceFile, StoreOp, StoreRef, Symbol, SymbolKind, Test, TestKind,
};
use tree_sitter::{Node, Parser};

/// Python extractor. Uses the tree-sitter-python grammar.
pub struct PythonExtractor {
    language: tree_sitter::Language,
}

impl Default for PythonExtractor {
    fn default() -> Self {
        PythonExtractor {
            language: tree_sitter_python::LANGUAGE.into(),
        }
    }
}

impl LanguageExtractor for PythonExtractor {
    fn language(&self) -> &'static str {
        "python"
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
        let mut ctx = Ctx::default();
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

/// Strip Python string literal quoting (and r/f/b/u prefixes).
fn strip_quotes(raw: &str) -> Option<String> {
    let s = raw.trim();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && matches!(bytes[i], b'r' | b'f' | b'b' | b'u') {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let q = bytes[i];
    if q != b'\'' && q != b'"' {
        return None;
    }
    let mut count = 0;
    while i + count < bytes.len() && bytes[i + count] == q {
        count += 1;
    }
    let start = i + count;
    let mut j = bytes.len();
    while j > start && bytes[j - 1] == q {
        j -= 1;
    }
    if j <= start {
        return None;
    }
    Some(s[start..j].to_string())
}

/// Value of a string literal node (or concatenated string).
fn string_literal_value(node: Node, src: &[u8]) -> Option<String> {
    match node.kind() {
        "string" => strip_quotes(node_text(Some(node), src)),
        "concatenated_string" => {
            let mut out = String::new();
            let mut cursor = node.walk();
            for c in node.named_children(&mut cursor) {
                if c.kind() == "string" {
                    if let Some(p) = strip_quotes(node_text(Some(c), src)) {
                        out.push_str(&p);
                    }
                }
            }
            Some(out)
        }
        _ => None,
    }
}

/// First positional string argument of a call.
fn first_string_arg(call: Node, src: &[u8]) -> Option<String> {
    let args = call.child_by_field_name("arguments")?;
    let mut cursor = args.walk();
    for child in args.named_children(&mut cursor) {
        if child.kind() == "keyword_argument" {
            continue;
        }
        if let Some(s) = string_literal_value(child, src) {
            return Some(s);
        }
    }
    None
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

// ---------------------------------------------------------------------------
// Tables
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

const ROUTE_VERBS: &[&str] = &["get", "post", "put", "delete", "patch", "options", "head", "route"];

/// True when the call sits inside a conditional/loop/try body within its
/// enclosing function — walking ancestors from the call node up to (but
/// excluding) the nearest function/class/module boundary. A call inside a
/// nested function definition is NOT conditional (it is conditionally
/// *defined*, not conditionally *called*).
fn call_is_conditional(node: tree_sitter::Node) -> bool {
    let mut cur = node.parent();
    while let Some(anc) = cur {
        match anc.kind() {
            "function_definition" | "class_definition" | "module" => return false,
            "if_statement" | "for_statement" | "while_statement" | "try_statement"
            | "with_statement" | "match_statement" => return true,
            _ => cur = anc.parent(),
        }
    }
    false
}

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

// ---------------------------------------------------------------------------
// Extraction context
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Scope {
    name: String,
    is_class: bool,
}

#[derive(Default)]
struct Ctx {
    symbols: Vec<Symbol>,
    imports: Vec<Import>,
    calls: Vec<Call>,
    routes: Vec<Route>,
    tests: Vec<Test>,
    store_refs: Vec<StoreRef>,
    retries: Vec<Retry>,
    entrypoints: Vec<Entrypoint>,
    scopes: Vec<Scope>,
}

impl Ctx {
    fn caller(&self) -> Option<String> {
        self.scopes.last().map(|s| s.name.clone())
    }
    fn top_is_class(&self) -> bool {
        self.scopes.last().map(|s| s.is_class).unwrap_or(false)
    }
    fn top_name(&self) -> String {
        self.scopes.last().map(|s| s.name.clone()).unwrap_or_default()
    }
    fn into_extracted(self) -> ExtractedFile {
        ExtractedFile {
            symbols: self.symbols,
            imports: self.imports,
            calls: self.calls,
            routes: self.routes,
            tests: self.tests,
            store_refs: self.store_refs,
            retries: self.retries,
            entrypoints: self.entrypoints,
        }
    }
}

// ---------------------------------------------------------------------------
// Walker
// ---------------------------------------------------------------------------

impl PythonExtractor {
    fn walk(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        match node.kind() {
            "function_definition" => self.walk_function(node, ctx, src),
            "class_definition" => self.walk_class(node, ctx, src),
            "decorated_definition" => self.walk_decorated(node, ctx, src),
            "call" => self.record_call(node, ctx, src),
            "import_statement" => self.record_import(node, ctx, src),
            "import_from_statement" => self.record_from_import(node, ctx, src),
            "if_statement" => {
                self.maybe_entrypoint(node, ctx, src);
                self.walk_children(node, ctx, src);
            }
            _ => self.walk_children(node, ctx, src),
        }
    }

    fn walk_children(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, ctx, src);
        }
    }

    /// Symbol full name for a definition node given the current scope.
    fn def_symbol_name(&self, def: Node, ctx: &Ctx, src: &[u8]) -> String {
        let name = clean(node_text(def.child_by_field_name("name"), src));
        if def.kind() == "function_definition" && ctx.top_is_class() {
            format!("{}.{}", ctx.top_name(), name)
        } else {
            name
        }
    }

    fn walk_function(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let name = clean(node_text(node.child_by_field_name("name"), src));
        let in_class = ctx.top_is_class();
        let exported = ctx.scopes.is_empty();
        let (sym_name, kind, parent) = if in_class {
            let class = ctx.top_name();
            (format!("{class}.{name}"), SymbolKind::Method, Some(class))
        } else {
            (name.clone(), SymbolKind::Function, None)
        };
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;
        let doc = first_docstring(node.child_by_field_name("body"), src);
        let sig = signature(&name, node, src, in_class);
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
        // module-level pytest-style tests
        if !in_class && exported && name.starts_with("test_") {
            ctx.tests.push(Test {
                name: name.clone(),
                symbol: Some(name.clone()),
                kind: TestKind::Unit,
                line: start_line,
            });
        }
        ctx.scopes.push(Scope {
            name: sym_name,
            is_class: false,
        });
        self.walk_children(node, ctx, src);
        ctx.scopes.pop();
    }

    fn walk_class(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let name = clean(node_text(node.child_by_field_name("name"), src));
        let exported = ctx.scopes.is_empty();
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;
        let doc = first_docstring(node.child_by_field_name("body"), src);
        ctx.symbols.push(Symbol {
            name: name.clone(),
            kind: SymbolKind::Class,
            signature: None,
            start_line,
            end_line,
            exported,
            docstring: doc,
            parent: None,
        });
        if is_test_class(node, &name, src) {
            ctx.tests.push(Test {
                name: name.clone(),
                symbol: Some(name.clone()),
                kind: TestKind::Unit,
                line: start_line,
            });
        }
        ctx.scopes.push(Scope {
            name: name.clone(),
            is_class: true,
        });
        self.walk_children(node, ctx, src);
        ctx.scopes.pop();
    }

    fn walk_decorated(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let mut cursor = node.walk();
        let defs: Vec<Node> = node
            .named_children(&mut cursor)
            .filter(|c| matches!(c.kind(), "function_definition" | "class_definition"))
            .collect();
        let def = defs.first().copied();
        let def_symbol = def.map(|d| self.def_symbol_name(d, ctx, src));
        let def_plain = def.map(|d| clean(node_text(d.child_by_field_name("name"), src)));
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "decorator" {
                self.process_decorator(child, ctx, src, def_plain.as_deref(), def_symbol.as_deref());
            }
            self.walk(child, ctx, src);
        }
    }

    fn process_decorator(
        &self,
        dec: Node,
        ctx: &mut Ctx,
        src: &[u8],
        def_plain: Option<&str>,
        def_symbol: Option<&str>,
    ) {
        let line = dec.start_position().row as u32 + 1;
        let policy = node_text(Some(dec), src)
            .trim()
            .trim_start_matches('@')
            .trim()
            .to_string();
        let mut cursor = dec.walk();
        let Some(expr) = dec.named_children(&mut cursor).next() else {
            return;
        };
        // dotted name of the decorated expression (call -> function part)
        let dotted = match expr.kind() {
            "call" => expr
                .child_by_field_name("function")
                .map(|f| collapse(node_text(Some(f), src)))
                .unwrap_or_default(),
            _ => collapse(node_text(Some(expr), src)),
        };
        // retry/backoff decoration
        let lower = dotted.to_ascii_lowercase();
        if !dotted.is_empty() && (lower.contains("retry") || lower.contains("backoff")) {
            if let Some(sym) = def_symbol {
                ctx.retries.push(Retry {
                    symbol: sym.to_string(),
                    policy,
                    line,
                });
            }
        }
        // route decoration
        if expr.kind() != "call" {
            return;
        }
        let Some(fn_node) = expr.child_by_field_name("function") else {
            return;
        };
        let fname = collapse(node_text(Some(fn_node), src));
        let Some(verb) = fname.rsplit('.').next() else {
            return;
        };
        if !ROUTE_VERBS.contains(&verb) {
            return;
        }
        let Some(path) = first_string_arg(expr, src) else {
            return;
        };
        let methods: Vec<String> = if verb == "route" {
            let mut m = methods_kwarg(expr, src);
            if m.is_empty() {
                m.push("GET".to_string());
            }
            m
        } else {
            vec![verb.to_uppercase()]
        };
        for method in methods {
            ctx.routes.push(Route {
                method,
                path: path.clone(),
                handler: def_plain.map(str::to_string),
                line,
                framework: "http".to_string(),
            });
        }
    }

    fn record_call(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        if let Some(fn_node) = node.child_by_field_name("function") {
            let callee = collapse(node_text(Some(fn_node), src));
            if !callee.is_empty() {
                let root = callee_root(fn_node);
                let known_receiver = root.kind() == "identifier";
                ctx.calls.push(Call {
                    caller: ctx.caller(),
                    callee,
                    line: node.start_position().row as u32 + 1,
                    known_receiver,
                    conditional: call_is_conditional(node),
                });
                self.record_store_ref(node, fn_node, &root, ctx, src);
            }
        }
        self.walk_children(node, ctx, src);
    }

    fn record_store_ref(&self, node: Node, fn_node: Node, root: &Node, ctx: &mut Ctx, src: &[u8]) {
        if root.kind() != "identifier" {
            return;
        }
        let mut segs: Vec<String> = Vec::new();
        attribute_segments(fn_node, &mut segs, src);
        if segs.len() < 2 {
            return;
        }
        // `self.db.execute(...)` / `cls.redis.set(...)` / `this.db.query(...)`:
        // unwrap the instance prefix and treat the next segment as the store.
        if matches!(segs[0].as_str(), "self" | "cls" | "this") && segs.len() >= 3 {
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
            if let Some(s) = first_string_arg(node, src) {
                target = Some(s);
            }
        }
        // SQL sniffing for execute-family ops overrides op + target
        if matches!(op_name.as_str(), "execute" | "executemany" | "executescript") {
            if let Some(sql) = first_string_arg(node, src) {
                let (sniff_op, sniff_target) = sql_op_table(&sql);
                ctx.store_refs.push(StoreRef {
                    caller: ctx.caller(),
                    technology: technology_for(&store),
                    store,
                    op: sniff_op,
                    target: sniff_target,
                    line: node.start_position().row as u32 + 1,
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
            line: node.start_position().row as u32 + 1,
        });
    }

    fn record_import(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let line = node.start_position().row as u32 + 1;
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "import_list" => {
                    let mut c2 = child.walk();
                    for c in child.named_children(&mut c2) {
                        self.push_import_group(c, ctx, src, line);
                    }
                }
                "dotted_name" | "aliased_import" => {
                    self.push_import_group(child, ctx, src, line);
                }
                _ => {}
            }
        }
    }

    fn push_import_group(&self, node: Node, ctx: &mut Ctx, src: &[u8], line: u32) {
        let (dotted, alias) = match node.kind() {
            "aliased_import" => self.aliased_import(node, src),
            _ => (collapse(node_text(Some(node), src)), None),
        };
        if dotted.is_empty() {
            return;
        }
        let names = match alias {
            Some(a) => vec![(a, dotted.clone())],
            None => {
                let root = dotted.split('.').next().unwrap_or("").to_string();
                if root.is_empty() {
                    return;
                }
                vec![(root.clone(), root)]
            }
        };
        ctx.imports.push(Import {
            module: dotted,
            names,
            line,
            r#type: ImportType::Module,
        });
    }

    /// Returns `(imported_name, alias)`. Handles `a.b as c` (dotted) and
    /// `x as y` (two identifiers, from-imports).
    fn aliased_import(&self, node: Node, src: &[u8]) -> (String, Option<String>) {
        let mut dotted = String::new();
        let mut idents: Vec<String> = Vec::new();
        let mut cursor = node.walk();
        for c in node.named_children(&mut cursor) {
            match c.kind() {
                "dotted_name" => dotted = collapse(node_text(Some(c), src)),
                "identifier" => {
                    let n = clean(node_text(Some(c), src));
                    if !n.is_empty() {
                        idents.push(n);
                    }
                }
                _ => {}
            }
        }
        if !dotted.is_empty() {
            (dotted, idents.first().cloned())
        } else {
            match idents.len() {
                2 => (idents[0].clone(), Some(idents[1].clone())),
                1 => (idents[0].clone(), None),
                _ => (String::new(), None),
            }
        }
    }

    fn record_from_import(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let line = node.start_position().row as u32 + 1;
        let module_node = node.child_by_field_name("module_name");
        let module = module_node
            .map(|m| collapse(node_text(Some(m), src)))
            .unwrap_or_default();
        let mut names: Vec<(String, String)> = Vec::new();
        let mut wildcard = false;
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            // the module specifier itself is a dotted_name child too
            if let Some(mn) = module_node {
                if child.id() == mn.id() {
                    continue;
                }
            }
            match child.kind() {
                "dotted_name" => {
                    // `from m import a` / `from m import a.b` (binds root)
                    let t = clean(node_text(Some(child), src));
                    if !t.is_empty() {
                        let root = t.split('.').next().unwrap_or("").to_string();
                        if !root.is_empty() {
                            names.push((root.clone(), root));
                        }
                    }
                }
                "aliased_import" => {
                    let (imported, alias) = self.aliased_import(child, src);
                    if let Some(a) = alias {
                        if !imported.is_empty() {
                            names.push((a, imported));
                        }
                    }
                }
                "wildcard_import" | "_" => wildcard = true,
                _ => {
                    if collapse(node_text(Some(child), src)) == "*" {
                        wildcard = true;
                    }
                }
            }
        }
        if wildcard {
            names.clear();
        }
        ctx.imports.push(Import {
            module,
            names,
            line,
            r#type: ImportType::Member,
        });
    }

    fn maybe_entrypoint(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        if !ctx.scopes.is_empty() {
            return;
        }
        let cond = node_text(node.child_by_field_name("condition"), src);
        if !cond.contains("__name__") || !cond.contains("__main__") || !cond.contains("==") {
            return;
        }
        let line = node.start_position().row as u32 + 1;
        if let Some(cons) = node.child_by_field_name("consequence") {
            if let Some(sym) = find_main_call(cons, src) {
                ctx.entrypoints.push(Entrypoint {
                    symbol: sym,
                    kind: "main-guard".to_string(),
                    line,
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Small walker helpers
// ---------------------------------------------------------------------------

/// Innermost object of an attribute chain (identifier, literal, call, ...).
fn callee_root(mut node: Node) -> Node {
    loop {
        match node.kind() {
            "attribute" => match node.child_by_field_name("object") {
                Some(obj) => node = obj,
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

/// Attribute chain segments from outermost (root) to method.
fn attribute_segments(mut node: Node, out: &mut Vec<String>, src: &[u8]) {
    let mut stack: Vec<String> = Vec::new();
    loop {
        match node.kind() {
            "attribute" => {
                let attr = node_text(node.child_by_field_name("attribute"), src);
                if !attr.is_empty() {
                    stack.push(attr.to_string());
                }
                match node.child_by_field_name("object") {
                    Some(obj) => node = obj,
                    None => break,
                }
            }
            "identifier" => {
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

fn signature(name: &str, fn_node: Node, src: &[u8], is_method: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(params) = fn_node.child_by_field_name("parameters") {
        let mut cursor = params.walk();
        for child in params.named_children(&mut cursor) {
            if child.kind() == "type" {
                continue; // return annotation inside parameters (older grammar)
            }
            let t = collapse(node_text(Some(child), src));
            if t.is_empty() {
                continue;
            }
            if is_method && is_self_param(&t) {
                continue;
            }
            parts.push(t);
        }
    }
    let mut sig = format!("def {name}({})", parts.join(", "));
    if let Some(rt) = fn_node.child_by_field_name("return_type") {
        let t = collapse(node_text(Some(rt), src));
        if !t.is_empty() {
            sig.push_str(" -> ");
            sig.push_str(&t);
        }
    }
    truncate_chars(&sig, 120)
}

fn is_self_param(p: &str) -> bool {
    p == "self"
        || p == "cls"
        || p.starts_with("self:")
        || p.starts_with("cls:")
        || p.starts_with("self=")
        || p.starts_with("cls=")
}

/// First statement of a body, if it is a string literal.
fn first_docstring(body: Option<Node>, src: &[u8]) -> Option<String> {
    let block = body?;
    let mut cursor = block.walk();
    let first = block.named_children(&mut cursor).next()?;
    if first.kind() != "expression_statement" {
        return None;
    }
    let mut c2 = first.walk();
    for child in first.named_children(&mut c2) {
        if matches!(child.kind(), "string" | "concatenated_string") {
            return docstring_value(child, src);
        }
    }
    None
}

fn docstring_value(node: Node, src: &[u8]) -> Option<String> {
    let content = string_literal_value(node, src)?;
    Some(first_paragraph(&content))
}

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

/// True when a class looks like a test suite: inherits (unittest.)TestCase,
/// name starts with `Test`, or defines `test_*` methods.
fn is_test_class(class: Node, class_name: &str, src: &[u8]) -> bool {
    if class_name.starts_with("Test") {
        return true;
    }
    if let Some(supers) = class.child_by_field_name("superclasses") {
        let mut cursor = supers.walk();
        for s in supers.named_children(&mut cursor) {
            let t = node_text(Some(s), src);
            if t.trim_end().ends_with("TestCase") {
                return true;
            }
        }
    }
    if let Some(body) = class.child_by_field_name("body") {
        let mut cursor = body.walk();
        for stmt in body.named_children(&mut cursor) {
            if stmt.kind() == "function_definition" {
                let n = node_text(stmt.child_by_field_name("name"), src);
                if n.starts_with("test_") {
                    return true;
                }
            }
        }
    }
    false
}

/// First bare-identifier call in document order (for `__main__` guards).
fn find_main_call(node: Node, src: &[u8]) -> Option<String> {
    if node.kind() == "call" {
        if let Some(f) = node.child_by_field_name("function") {
            if f.kind() == "identifier" {
                let n = clean(node_text(Some(f), src));
                if !n.is_empty() {
                    return Some(n);
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(n) = find_main_call(child, src) {
            return Some(n);
        }
    }
    None
}

/// `methods=[...]` keyword argument value of a call.
fn methods_kwarg(call: Node, src: &[u8]) -> Vec<String> {
    let Some(args) = call.child_by_field_name("arguments") else {
        return Vec::new();
    };
    let mut cursor = args.walk();
    for child in args.named_children(&mut cursor) {
        if child.kind() != "keyword_argument" {
            continue;
        }
        if node_text(child.child_by_field_name("name"), src) != "methods" {
            continue;
        }
        let Some(val) = child.child_by_field_name("value") else {
            continue;
        };
        let mut out = Vec::new();
        if val.kind() == "list" {
            let mut c2 = val.walk();
            for s in val.named_children(&mut c2) {
                if let Some(v) = string_literal_value(s, src) {
                    out.push(v.to_uppercase());
                }
            }
        } else if let Some(v) = string_literal_value(val, src) {
            out.push(v.to_uppercase());
        }
        return out;
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ImportType, StoreOp, SymbolKind, TestKind};

    fn extract(src: &str) -> ExtractedFile {
        let f = SourceFile::new("test.py", src);
        PythonExtractor::default().extract(&f)
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
            "def add(a, b):\n    return a + b\n\nclass Calculator:\n    \"\"\"Calc docs.\"\"\"\n\n    def helper(self, x: int) -> int:\n        return x\n\n    @staticmethod\n    def make(cls, y):\n        return y\n\n    async def go(self, *args, **kw):\n        pass\n",
        );
        assert_eq!(ef.symbols.len(), 5);

        let add = find_symbol(&ef, "add");
        assert_eq!(add.kind, SymbolKind::Function);
        assert!(add.exported);
        assert_eq!(add.signature.as_deref(), Some("def add(a, b)"));
        assert_eq!(add.start_line, 1);
        assert_eq!(add.end_line, 2);
        assert_eq!(add.parent, None);
        assert_eq!(add.docstring, None);

        let calc = find_symbol(&ef, "Calculator");
        assert_eq!(calc.kind, SymbolKind::Class);
        assert!(calc.exported);
        assert_eq!(calc.signature, None);
        assert_eq!(calc.docstring.as_deref(), Some("Calc docs."));

        let helper = find_symbol(&ef, "Calculator.helper");
        assert_eq!(helper.kind, SymbolKind::Method);
        assert!(!helper.exported);
        assert_eq!(helper.parent.as_deref(), Some("Calculator"));
        // self dropped from signature
        assert_eq!(helper.signature.as_deref(), Some("def helper(x: int) -> int"));

        let make = find_symbol(&ef, "Calculator.make");
        assert_eq!(make.kind, SymbolKind::Method);
        assert_eq!(make.parent.as_deref(), Some("Calculator"));
        // cls dropped from signature
        assert_eq!(make.signature.as_deref(), Some("def make(y)"));

        let go = find_symbol(&ef, "Calculator.go");
        assert_eq!(go.signature.as_deref(), Some("def go(*args, **kw)"));
        assert_eq!(go.start_line, 14);
        assert_eq!(go.end_line, 15);
    }

    #[test]
    fn imports_all_forms() {
        let ef = extract(
            "import os\nimport json.decoder\nimport a.b as c\nimport x, y.z as w\nfrom flask import Flask, request as req\nfrom . import local\nfrom .models import User\nfrom .utils import *\nfrom typing import List\n",
        );
        let imps = &ef.imports;
        assert_eq!(imps.len(), 10);

        assert_eq!(imps[0].module, "os");
        assert_eq!(imps[0].names, vec![("os".into(), "os".into())]);
        assert_eq!(imps[0].r#type, ImportType::Module);
        assert_eq!(imps[0].line, 1);

        assert_eq!(imps[1].module, "json.decoder");
        assert_eq!(imps[1].names, vec![("json".into(), "json".into())]);

        assert_eq!(imps[2].module, "a.b");
        assert_eq!(imps[2].names, vec![("c".into(), "a.b".into())]);

        // `import x, y.z as w` splits into two module imports
        assert_eq!(imps[3].module, "x");
        assert_eq!(imps[3].names, vec![("x".into(), "x".into())]);
        assert_eq!(imps[4].module, "y.z");
        assert_eq!(imps[4].names, vec![("w".into(), "y.z".into())]);

        let flask = &imps[5];
        assert_eq!(flask.module, "flask");
        assert_eq!(
            flask.names,
            vec![("Flask".into(), "Flask".into()), ("req".into(), "request".into())]
        );
        assert_eq!(flask.r#type, ImportType::Member);

        assert_eq!(imps[6].module, ".");
        assert_eq!(imps[6].names, vec![("local".into(), "local".into())]);

        assert_eq!(imps[7].module, ".models");
        assert_eq!(imps[7].names, vec![("User".into(), "User".into())]);

        // wildcard: no bound names
        assert_eq!(imps[8].module, ".utils");
        assert!(imps[8].names.is_empty());
        assert_eq!(imps[8].r#type, ImportType::Member);

        assert_eq!(imps[9].module, "typing");
        assert_eq!(imps[9].names, vec![("List".into(), "List".into())]);
    }

    #[test]
    fn calls_and_receivers() {
        let ef = extract(
            "import client\n\ndef top():\n    client.execute(\"SELECT 1\")\n    return helper()\n\ndef helper():\n    return 1\n\nx = factory()\n\nclass Svc:\n    def run(self):\n        self.do(1)\n        return done()\n\n    def do(self, x):\n        return x\n",
        );
        let calls = &ef.calls;
        assert_eq!(calls.len(), 5);
        // document order
        assert_eq!(calls[0].caller.as_deref(), Some("top"));
        assert_eq!(calls[0].callee, "client.execute");
        assert!(calls[0].known_receiver);
        assert_eq!(calls[0].line, 4);

        assert_eq!(calls[1].caller.as_deref(), Some("top"));
        assert_eq!(calls[1].callee, "helper");
        assert!(calls[1].known_receiver);

        // module-level call: no caller
        assert_eq!(calls[2].caller, None);
        assert_eq!(calls[2].callee, "factory");

        assert_eq!(calls[3].caller.as_deref(), Some("Svc.run"));
        assert_eq!(calls[3].callee, "self.do");
        assert!(calls[3].known_receiver);
        assert_eq!(calls[3].line, 14);

        assert_eq!(calls[4].caller.as_deref(), Some("Svc.run"));
        assert_eq!(calls[4].callee, "done");
    }

    #[test]
    fn calls_unknown_receiver() {
        let ef = extract(
            "def f():\n    dct[\"k\"]()\n    \"abc\".join(\"x\")\n    obj.method()\n    (factory)(1)\n",
        );
        let calls = &ef.calls;
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[0].callee, "dct[\"k\"]");
        assert!(!calls[0].known_receiver);
        assert_eq!(calls[1].callee, "\"abc\".join");
        assert!(!calls[1].known_receiver);
        assert_eq!(calls[2].callee, "obj.method");
        assert!(calls[2].known_receiver);
        assert_eq!(calls[3].callee, "(factory)");
        assert!(calls[3].known_receiver);
    }

    #[test]
    fn routes_decorators() {
        let ef = extract(
            "from flask import Flask\n\napp = Flask(__name__)\n\n@app.get(\"/ping\")\ndef ping():\n    return \"pong\"\n\n@app.post(\"/items\")\ndef create():\n    pass\n\n@blueprint.route(\"/p\", methods=[\"GET\", \"POST\"])\ndef page():\n    pass\n\n@app.route(\"/x\")\ndef x():\n    pass\n\n@router.put(\"/y\")\ndef y():\n    pass\n",
        );
        let routes = &ef.routes;
        assert_eq!(routes.len(), 6);
        assert_eq!(routes[0].method, "GET");
        assert_eq!(routes[0].path, "/ping");
        assert_eq!(routes[0].handler.as_deref(), Some("ping"));
        assert_eq!(routes[0].framework, "http");
        assert_eq!(routes[0].line, 5);

        assert_eq!(routes[1].method, "POST");
        assert_eq!(routes[1].path, "/items");
        assert_eq!(routes[1].handler.as_deref(), Some("create"));

        // methods= kwarg expands to one route per verb
        assert_eq!(routes[2].method, "GET");
        assert_eq!(routes[2].path, "/p");
        assert_eq!(routes[2].handler.as_deref(), Some("page"));
        assert_eq!(routes[3].method, "POST");
        assert_eq!(routes[3].path, "/p");
        assert_eq!(routes[3].handler.as_deref(), Some("page"));
        assert_eq!(routes[3].line, 13);

        // .route without methods= defaults to GET
        assert_eq!(routes[4].method, "GET");
        assert_eq!(routes[4].path, "/x");

        assert_eq!(routes[5].method, "PUT");
        assert_eq!(routes[5].path, "/y");
        assert_eq!(routes[5].handler.as_deref(), Some("y"));
    }

    #[test]
    fn tests_detection() {
        let ef = extract(
            "import unittest\n\ndef test_add():\n    pass\n\nasync def test_async():\n    pass\n\nclass TestCalculator(unittest.TestCase):\n    def test_add(self):\n        pass\n\nclass MyTests:\n    def test_x(self):\n        pass\n\nclass NotATest:\n    def helper(self):\n        pass\n\ndef normal():\n    pass\n",
        );
        let tests = &ef.tests;
        assert_eq!(tests.len(), 4);
        assert_eq!(tests[0].name, "test_add");
        assert_eq!(tests[0].symbol.as_deref(), Some("test_add"));
        assert_eq!(tests[0].kind, TestKind::Unit);
        assert_eq!(tests[0].line, 3);

        assert_eq!(tests[1].name, "test_async");

        assert_eq!(tests[2].name, "TestCalculator");
        assert_eq!(tests[2].symbol.as_deref(), Some("TestCalculator"));

        assert_eq!(tests[3].name, "MyTests");
        assert_eq!(tests[3].kind, TestKind::Unit);
    }

    #[test]
    fn store_refs_sql_and_clients() {
        let ef = extract(
            "def worker(db, conn, redis, r, producer, consumer, collection, cursor, session, mongo):\n    db.execute(\"INSERT INTO users (name) VALUES (?)\", (\"x\",))\n    conn.execute(\"SELECT * FROM orders WHERE id = 1\")\n    cursor.executemany(\"UPDATE accounts SET bal = 0\")\n    session.execute(\"CREATE TABLE logs (id int)\")\n    db.execute(\"DROP TABLE old_events\")\n    redis.get(\"user:1\")\n    r.set(\"k\", \"v\")\n    producer.send(\"events\", {\"a\": 1})\n    consumer.subscribe(\"orders\")\n    collection.find({\"x\": 1})\n    mongo.db.users.count({\"active\": True})\n    db.commit()\n",
        );
        let refs = &ef.store_refs;
        assert_eq!(refs.len(), 12);
        let caller = Some("worker".to_string());

        assert_eq!(refs[0].store, "db");
        assert_eq!(refs[0].technology.as_deref(), Some("sql"));
        assert_eq!(refs[0].op, StoreOp::Write);
        assert_eq!(refs[0].target.as_deref(), Some("users"));
        assert_eq!(refs[0].caller, caller);

        assert_eq!(refs[1].store, "conn");
        assert_eq!(refs[1].op, StoreOp::Query);
        assert_eq!(refs[1].target.as_deref(), Some("orders"));

        assert_eq!(refs[2].store, "cursor");
        assert_eq!(refs[2].op, StoreOp::Write);
        assert_eq!(refs[2].target.as_deref(), Some("accounts"));

        assert_eq!(refs[3].store, "session");
        assert_eq!(refs[3].op, StoreOp::Migrate);
        assert_eq!(refs[3].target.as_deref(), Some("logs"));

        assert_eq!(refs[4].store, "db");
        assert_eq!(refs[4].op, StoreOp::Migrate);
        assert_eq!(refs[4].target.as_deref(), Some("old_events"));

        assert_eq!(refs[5].store, "redis");
        assert_eq!(refs[5].technology.as_deref(), Some("redis"));
        assert_eq!(refs[5].op, StoreOp::Read);
        assert_eq!(refs[5].target.as_deref(), Some("user:1"));

        assert_eq!(refs[6].store, "r");
        assert_eq!(refs[6].technology.as_deref(), Some("redis"));
        assert_eq!(refs[6].op, StoreOp::Write);
        assert_eq!(refs[6].target.as_deref(), Some("k"));

        assert_eq!(refs[7].store, "producer");
        assert_eq!(refs[7].technology.as_deref(), Some("kafka"));
        assert_eq!(refs[7].op, StoreOp::Publish);
        assert_eq!(refs[7].target.as_deref(), Some("events"));

        assert_eq!(refs[8].store, "consumer");
        assert_eq!(refs[8].technology.as_deref(), Some("kafka"));
        assert_eq!(refs[8].op, StoreOp::Subscribe);
        assert_eq!(refs[8].target.as_deref(), Some("orders"));

        assert_eq!(refs[9].store, "collection");
        assert_eq!(refs[9].technology.as_deref(), Some("mongodb"));
        assert_eq!(refs[9].op, StoreOp::Query);
        assert_eq!(refs[9].target, None);

        // member chain: root.mid.op -> target = mid
        assert_eq!(refs[10].store, "mongo");
        assert_eq!(refs[10].technology.as_deref(), Some("mongodb"));
        assert_eq!(refs[10].op, StoreOp::Read);
        assert_eq!(refs[10].target.as_deref(), Some("users"));

        assert_eq!(refs[11].store, "db");
        assert_eq!(refs[11].op, StoreOp::Write);
        assert_eq!(refs[11].target, None);
    }

    #[test]
    fn store_refs_not_flagged_for_unknown_receivers() {
        let ef = extract(
            "def f(client, http):\n    http.get(\"/x\")\n    client.fetch_data()\n    other.execute(\"INSERT INTO nope\")\n    client.get(\"foo\")\n",
        );
        // client.get IS flagged (client is in the receiver list) but
        // fetch_data / http.get / other.execute are not.
        let refs = &ef.store_refs;
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].store, "client");
        assert_eq!(refs[0].op, StoreOp::Read);
    }

    #[test]
    fn retry_decorators() {
        let ef = extract(
            "@retry\ndef a():\n    pass\n\n@tenacity.retry(wait=1)\ndef b():\n    pass\n\n@backoff.on_exception(backoff.expo)\ndef c():\n    pass\n\n@other\ndef d():\n    pass\n",
        );
        let retries = &ef.retries;
        assert_eq!(retries.len(), 3);
        assert_eq!(retries[0].symbol, "a");
        assert_eq!(retries[0].policy, "retry");
        assert_eq!(retries[0].line, 1);
        assert_eq!(retries[1].symbol, "b");
        assert_eq!(retries[1].policy, "tenacity.retry(wait=1)");
        assert_eq!(retries[2].symbol, "c");
        assert_eq!(retries[2].policy, "backoff.on_exception(backoff.expo)");
    }

    #[test]
    fn entrypoint_main_guard() {
        let ef = extract(
            "def main():\n    pass\n\nif __name__ == \"__main__\":\n    main()\n",
        );
        let eps = &ef.entrypoints;
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].symbol, "main");
        assert_eq!(eps[0].kind, "main-guard");
        assert_eq!(eps[0].line, 4);
    }

    #[test]
    fn docstrings_first_paragraph() {
        let ef = extract(
            "def f():\n    \"\"\"Sum two.\n\n    More here.\n    \"\"\"\n    return 1\n\nclass A:\n    \"\"\"Class doc.\"\"\"\n    pass\n\ndef g():\n    x = 1\n    return x\n\ndef h():\n    'single line'\n    pass\n",
        );
        let f = find_symbol(&ef, "f");
        assert_eq!(f.docstring.as_deref(), Some("Sum two."));
        let a = find_symbol(&ef, "A");
        assert_eq!(a.docstring.as_deref(), Some("Class doc."));
        let g = find_symbol(&ef, "g");
        assert_eq!(g.docstring, None);
        let h = find_symbol(&ef, "h");
        assert_eq!(h.docstring.as_deref(), Some("single line"));
    }

    #[test]
    fn malformed_input_does_not_panic() {
        let cases = [
            "def broken(:",
            "def broken(:\n    return",
            "\u{0}\u{1}\u{2}\u{ff}\u{fe}",
            "class :\n def ",
            "@\n@@@\n@",
            "import",
            "from import",
            "def xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx(\n",
            "x = ((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((\n",
        ];
        for c in cases {
            let _ = extract(c);
        }
    }

    #[test]
    fn deterministic_output() {
        let src = "import os\n\ndef add(a, b):\n    return a + b\n\n@app.get(\"/x\")\ndef x():\n    pass\n\nif __name__ == \"__main__\":\n    add(1, 2)\n";
        let a = extract(src);
        let b = extract(src);
        let ja = serde_json::to_string(&a).unwrap();
        let jb = serde_json::to_string(&b).unwrap();
        assert_eq!(ja, jb);
    }
}
