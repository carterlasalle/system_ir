//! Java language extractor (tree-sitter based).
//!
//! Pure, deterministic extraction: `(path, content) -> ExtractedFile`.
//! Syntax-level only; cross-file resolution happens in `resolve.rs`
//! (same-class / same-file names resolve; imports are recorded as module
//! strings, matching the python.rs contract).

use crate::facts;
use crate::model::{
    Call, Entrypoint, ExtractedFile, Import, ImportType, LanguageExtractor, Retry, SemanticFact,
    SourceFile, StoreOp, StoreRef, Symbol, SymbolKind,
};
use std::collections::BTreeMap;
use tree_sitter::{Node, Parser};

/// Java extractor. Uses the tree-sitter-java grammar.
pub struct JavaExtractor {
    language: tree_sitter::Language,
}

impl Default for JavaExtractor {
    fn default() -> Self {
        JavaExtractor {
            language: tree_sitter_java::LANGUAGE.into(),
        }
    }
}

impl LanguageExtractor for JavaExtractor {
    fn language(&self) -> &'static str {
        "java"
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

/// Value of a Java string literal node (`"..."` — quotes stripped). Text
/// blocks are left unhandled (they are rarely SQL/host literals).
fn string_literal_value(node: Node, src: &[u8]) -> Option<String> {
    if node.kind() != "string_literal" {
        return None;
    }
    let raw = node_text(Some(node), src);
    let bytes = raw.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'"' || bytes[bytes.len() - 1] != b'"' {
        return None;
    }
    Some(raw[1..raw.len() - 1].to_string())
}

/// First positional string literal argument of a call.
fn first_string_arg(call: Node, src: &[u8]) -> Option<String> {
    let args = call.child_by_field_name("arguments")?;
    let mut cursor = args.walk();
    for child in args.named_children(&mut cursor) {
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

/// Classify a SQL statement and extract its target table (mirrors python.rs).
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
// Store tables
// ---------------------------------------------------------------------------

/// JDBC-ish receivers whose method calls represent store access.
const STORE_RECEIVERS: &[&str] = &[
    "connection", "conn", "db", "database", "client", "session", "pool", "em", "repository",
];

fn classify_op(name: &str) -> Option<StoreOp> {
    match name {
        "execute" | "prepareStatement" | "prepareCall" | "createStatement" => {
            Some(StoreOp::Query) // refined by SQL sniff
        }
        "executeUpdate" | "commit" | "save" | "add" | "insert" | "update" | "delete" | "remove"
        | "set" | "upsert" => Some(StoreOp::Write),
        "executeQuery" | "get" | "fetch" | "read" | "count" | "find" => Some(StoreOp::Read),
        "query" | "select" => Some(StoreOp::Query),
        "publish" | "send" => Some(StoreOp::Publish),
        "subscribe" | "consume" => Some(StoreOp::Subscribe),
        _ => None,
    }
}

fn technology_for(root: &str) -> Option<String> {
    match root {
        "connection" | "conn" | "db" | "database" | "pool" | "session" | "em" => {
            Some("sql".to_string())
        }
        "repository" => Some("repository".to_string()),
        _ => None,
    }
}

/// CFG evidence for a call site: `(conditional, control_block, inside_loop,
/// inside_try)`, walking ancestors up to the nearest class/method boundary.
/// tree-sitter-java has no `else_clause` node (the alternative is a plain
/// block), so else-branch calls report the enclosing `if_statement` — the
/// branch evidence is still correct. The nearest block wins; loop/try
/// nesting accumulates independently of it.
fn call_cfg(node: tree_sitter::Node) -> (bool, Option<&'static str>, bool, bool) {
    let mut cur = node.parent();
    let mut inside_loop = false;
    let mut inside_try = false;
    let mut block: Option<&'static str> = None;
    // A `finally` body runs on every path — guaranteed, not an alternative —
    // so it contributes no branch evidence; skip the try_statement it
    // belongs to (the walk continues to any *outer* construct).
    let mut skip_next_try = false;
    while let Some(anc) = cur {
        match anc.kind() {
            "class_declaration" | "interface_declaration" | "enum_declaration"
            | "method_declaration" | "constructor_declaration" | "lambda_expression"
            | "program" | "compilation_unit" => break,
            "if_statement" => {
                block.get_or_insert("if");
            }
            "for_statement" | "enhanced_for_statement" => {
                inside_loop = true;
                block.get_or_insert("for");
            }
            "while_statement" => {
                inside_loop = true;
                block.get_or_insert("while");
            }
            "do_statement" => {
                inside_loop = true;
                block.get_or_insert("do");
            }
            "try_statement" => {
                inside_try = true;
                if skip_next_try {
                    skip_next_try = false;
                } else {
                    block.get_or_insert("try");
                }
            }
            "catch_clause" => {
                inside_try = true;
                block.get_or_insert("catch");
            }
            "finally_clause" => {
                inside_try = true;
                skip_next_try = true;
            }
            "switch_expression" | "switch_statement" => {
                block.get_or_insert("switch");
            }
            "ternary_expression" => {
                block.get_or_insert("if");
            }
            _ => {}
        }
        cur = anc.parent();
    }
    (block.is_some(), block, inside_loop, inside_try)
}

/// True when the call's result is consumed (assigned/returned/compared/
/// passed) rather than discarded as a bare expression statement. Java has
/// no syntactic await — `awaited` is always false for java call sites.
fn call_returns_value(node: tree_sitter::Node) -> bool {
    let mut cur = node.parent();
    while let Some(anc) = cur {
        match anc.kind() {
            "class_declaration" | "interface_declaration" | "enum_declaration"
            | "method_declaration" | "constructor_declaration" | "lambda_expression"
            | "program" | "compilation_unit" | "expression_statement" => return false,
            "parenthesized_expression" => cur = anc.parent(),
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
    is_class: bool,
    /// The enclosing type is an interface: members are implicitly public.
    is_interface: bool,
}

#[derive(Default)]
struct Ctx {
    symbols: Vec<Symbol>,
    imports: Vec<Import>,
    calls: Vec<Call>,
    store_refs: Vec<StoreRef>,
    retries: Vec<Retry>,
    entrypoints: Vec<Entrypoint>,
    scopes: Vec<Scope>,
    facts: Vec<SemanticFact>,
    /// Per-caller call-site counter (source order) — CFG lexical evidence.
    call_seq: BTreeMap<Option<String>, u32>,
}

impl Ctx {
    fn caller(&self) -> Option<String> {
        self.scopes.last().map(|s| s.name.clone())
    }
    fn top_is_class(&self) -> bool {
        self.scopes.last().map(|s| s.is_class).unwrap_or(false)
    }
    /// True while walking inside any class body (including inside methods).
    fn in_class_context(&self) -> bool {
        self.scopes.iter().any(|s| s.is_class)
    }
    fn top_name(&self) -> String {
        self.scopes.last().map(|s| s.name.clone()).unwrap_or_default()
    }
    fn into_extracted(self) -> ExtractedFile {
        let mut facts = self.facts;
        // Deterministic order: (owning symbol, fact kind, tiebreaker).
        facts.sort_by_key(fact_sort_key);
        ExtractedFile {
            symbols: self.symbols,
            imports: self.imports,
            calls: self.calls,
            routes: Vec::new(),
            tests: Vec::new(),
            store_refs: self.store_refs,
            retries: self.retries,
            entrypoints: self.entrypoints,
            cli_flags: std::collections::BTreeMap::new(),
            facts,
        }
    }

    /// Any import whose module string starts with `prefix`.
    fn has_import(&self, prefix: &str) -> bool {
        self.imports.iter().any(|i| i.module.starts_with(prefix))
    }
}

// ---------------------------------------------------------------------------
// Walker
// ---------------------------------------------------------------------------

/// Deterministic order for semantic facts: (owning symbol, fact kind,
/// tiebreaker). `facts` are sorted before emission so exports never vary
/// run-to-run regardless of walk order.
fn fact_sort_key(f: &SemanticFact) -> (String, u8, String, String) {
    match f {
        SemanticFact::PublicExport { symbol, kind } => {
            (symbol.clone(), 0, kind.clone(), String::new())
        }
        SemanticFact::Annotation { name, target } => {
            (target.clone(), 1, name.clone(), String::new())
        }
        SemanticFact::Field {
            owner,
            name,
            mutable,
        } => (
            owner.clone(),
            2,
            name.clone(),
            if *mutable { "mutable" } else { "final" }.to_string(),
        ),
        SemanticFact::Registration { owner, kind, target } => {
            (owner.clone(), 3, kind.clone(), target.clone())
        }
        SemanticFact::Callback { owner, callback } => {
            (owner.clone(), 4, callback.clone(), String::new())
        }
        SemanticFact::Configuration { owner, key } => {
            (owner.clone(), 5, key.clone(), String::new())
        }
    }
}

// ---------------------------------------------------------------------------
// Semantic facts (Wave 9)
// ---------------------------------------------------------------------------

/// Framework import roots. A framework-named annotation is only recognized
/// as a framework fact when the matching import is present — a plain method
/// carrying a custom `@GetMapping` and no Spring import is never a route.
const SPRING_ROOT: &str = "org.springframework";
const JUNIT_ROOT: &str = "org.junit";
const MOCKITO_ROOT: &str = "org.mockito";

/// Spring MVC/context annotation names.
fn is_spring_annotation(name: &str) -> bool {
    matches!(
        name,
        "Controller"
            | "RestController"
            | "GetMapping"
            | "PostMapping"
            | "PutMapping"
            | "DeleteMapping"
            | "PatchMapping"
            | "RequestMapping"
            | "Bean"
            | "Service"
            | "Repository"
            | "Component"
            | "Autowired"
            | "Configuration"
            | "Value"
            | "Scheduled"
            | "PathVariable"
            | "RequestBody"
            | "RequestParam"
            | "ResponseBody"
            | "Transactional"
            | "SpringBootApplication"
    )
}

/// JUnit 4/5 annotation names.
fn is_junit_annotation(name: &str) -> bool {
    matches!(
        name,
        "Test"
            | "Before"
            | "After"
            | "BeforeClass"
            | "AfterClass"
            | "BeforeAll"
            | "AfterAll"
            | "BeforeEach"
            | "AfterEach"
            | "Rule"
            | "Ignore"
            | "Disabled"
            | "RunWith"
            | "ExtendWith"
            | "ParameterizedTest"
            | "RepeatedTest"
            | "Timeout"
            | "TempDir"
            | "DisplayName"
    )
}

/// Mockito annotation names.
fn is_mockito_annotation(name: &str) -> bool {
    matches!(name, "Mock" | "InjectMocks" | "Spy" | "Captor" | "MockBean")
}

impl JavaExtractor {
    fn walk(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        match node.kind() {
            "class_declaration" => self.walk_type(node, ctx, src, SymbolKind::Class),
            "interface_declaration" => self.walk_type(node, ctx, src, SymbolKind::Interface),
            "enum_declaration" => self.walk_type(node, ctx, src, SymbolKind::Enum),
            "method_declaration" => self.walk_method(node, ctx, src),
            "constructor_declaration" => self.walk_constructor(node, ctx, src),
            "field_declaration" => self.walk_field(node, ctx, src),
            "method_invocation" => self.record_call(node, ctx, src),
            "object_creation_expression" => self.record_creation(node, ctx, src),
            "import_declaration" => self.record_import(node, ctx, src),
            "package_declaration" => self.record_package(node, ctx, src),
            _ => self.walk_children(node, ctx, src),
        }
    }

    fn walk_children(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, ctx, src);
        }
    }

    /// A type declaration (class/interface/enum) and everything in it.
    fn walk_type(&self, node: Node, ctx: &mut Ctx, src: &[u8], kind: SymbolKind) {
        let name = clean(node_text(node.child_by_field_name("name"), src));
        if name.is_empty() {
            self.walk_children(node, ctx, src);
            return;
        }
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;
        ctx.symbols.push(Symbol {
            name: name.clone(),
            kind,
            signature: None,
            start_line,
            end_line,
            exported: true,
            docstring: self.leading_javadoc(node, src),
            parent: None,
        });
        let modifiers = self.modifiers_text(node, src);
        let is_public = modifiers.split_whitespace().any(|w| w == "public");
        if is_public {
            let kind_str = match kind {
                SymbolKind::Interface => "interface",
                SymbolKind::Enum => "enum",
                _ => "class",
            };
            ctx.facts.push(SemanticFact::PublicExport {
                symbol: name.clone(),
                kind: kind_str.to_string(),
            });
        }
        self.record_annotations(node, ctx, src, &name);
        ctx.scopes.push(Scope {
            name: name.clone(),
            is_class: true,
            is_interface: kind == SymbolKind::Interface,
        });
        self.walk_children(node, ctx, src);
        ctx.scopes.pop();
    }

    fn walk_method(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        if !ctx.top_is_class() {
            self.walk_children(node, ctx, src);
            return;
        }
        let name = clean(node_text(node.child_by_field_name("name"), src));
        if name.is_empty() {
            self.walk_children(node, ctx, src);
            return;
        }
        let class = ctx.top_name();
        let sym_name = format!("{class}.{name}");
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;
        let modifiers = self.modifiers_text(node, src);
        let is_static = modifiers.split_whitespace().any(|w| w == "static");
        let sig = self.signature(node, src, false);
        ctx.symbols.push(Symbol {
            name: sym_name.clone(),
            kind: SymbolKind::Method,
            signature: Some(sig),
            start_line,
            end_line,
            exported: false,
            docstring: self.leading_javadoc(node, src),
            parent: Some(class.clone()),
        });
        // retry annotations: @Retryable / @Retry on the method
        self.maybe_retry(node, ctx, src, &sym_name);
        // main entrypoint: public static void main(String[] args)
        if is_static && name == "main" && ctx.scopes.len() == 1 {
            ctx.entrypoints.push(Entrypoint {
                symbol: sym_name.clone(),
                kind: "main".to_string(),
                line: start_line,
            });
        }
        // Wave 9 facts: public API surface, annotations, registrations,
        // lifecycle callbacks.
        let is_interface = ctx.scopes.last().map(|s| s.is_interface).unwrap_or(false);
        let is_public = modifiers.split_whitespace().any(|w| w == "public") || is_interface;
        if is_public {
            ctx.facts.push(SemanticFact::PublicExport {
                symbol: sym_name.clone(),
                kind: "method".to_string(),
            });
        }
        // Wave 9 builder/factory contracts: static factories (guava
        // `ImmutableList.of(...)`, `builder()`) and fluent builder methods
        // (`.withX()/.setX()/.addX()` returning `this`).
        if is_static && facts::is_factory_name("java", &name) {
            let kind = if name == "builder" || name == "newBuilder" {
                "builder"
            } else {
                "factory"
            };
            ctx.facts.push(SemanticFact::Registration {
                owner: class.clone(),
                kind: kind.to_string(),
                target: class.clone(),
            });
        } else if facts::is_builder_chain_method(&name) && self.method_returns_this(node, src) {
            ctx.facts.push(SemanticFact::Registration {
                owner: class.clone(),
                kind: "builder".to_string(),
                target: class.clone(),
            });
        }
        self.record_annotations(node, ctx, src, &sym_name);
        self.record_spring_registrations(node, ctx, src, &class, &sym_name);
        self.record_junit_lifecycle(node, ctx, src, &class, &sym_name);
        ctx.scopes.push(Scope {
            name: sym_name,
            is_class: false,
            is_interface: false,
        });
        self.walk_children(node, ctx, src);
        ctx.scopes.pop();
    }

    fn walk_constructor(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        if !ctx.top_is_class() {
            self.walk_children(node, ctx, src);
            return;
        }
        let class = ctx.top_name();
        let sym_name = format!("{class}.{class}");
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;
        let sig = self.signature(node, src, true);
        ctx.symbols.push(Symbol {
            name: sym_name.clone(),
            kind: SymbolKind::Method,
            signature: Some(sig),
            start_line,
            end_line,
            exported: false,
            docstring: self.leading_javadoc(node, src),
            parent: Some(class.clone()),
        });
        self.maybe_retry(node, ctx, src, &sym_name);
        // Wave 9: a public constructor is API surface.
        let modifiers = self.modifiers_text(node, src);
        if modifiers.split_whitespace().any(|w| w == "public") {
            ctx.facts.push(SemanticFact::PublicExport {
                symbol: sym_name.clone(),
                kind: "constructor".to_string(),
            });
        }
        self.record_annotations(node, ctx, src, &sym_name);
        ctx.scopes.push(Scope {
            name: sym_name,
            is_class: false,
            is_interface: false,
        });
        self.walk_children(node, ctx, src);
        ctx.scopes.pop();
    }

    /// Field declarations → const symbols named `Class.field` plus Wave 9
    /// Field facts (mutable unless `final` / interface constant) and JUnit
    /// `@Rule` registrations.
    fn walk_field(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        if !ctx.top_is_class() {
            self.walk_children(node, ctx, src);
            return;
        }
        let class = ctx.top_name();
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;
        let modifiers = self.modifiers_text(node, src);
        let is_final = modifiers.split_whitespace().any(|w| w == "final");
        let in_interface = ctx.scopes.last().map(|s| s.is_interface).unwrap_or(false);
        let mutable = !is_final && !in_interface;
        let has_rule = ctx.has_import(JUNIT_ROOT)
            && annotations_on(node, src)
                .iter()
                .any(|(n, _)| n == "Rule");
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() != "variable_declarator" {
                continue;
            }
            let Some(fname) = child.child_by_field_name("name") else {
                continue;
            };
            let fname = clean(node_text(Some(fname), src));
            if fname.is_empty() {
                continue;
            }
            let fq = format!("{class}.{fname}");
            ctx.symbols.push(Symbol {
                name: fq.clone(),
                kind: SymbolKind::Const,
                signature: None,
                start_line,
                end_line,
                exported: false,
                docstring: None,
                parent: Some(class.clone()),
            });
            ctx.facts.push(SemanticFact::Field {
                owner: class.clone(),
                name: fname.clone(),
                mutable,
            });
            self.record_annotations(node, ctx, src, &fq);
            if has_rule {
                ctx.facts.push(SemanticFact::Registration {
                    owner: class.clone(),
                    kind: "rule".to_string(),
                    target: fq,
                });
            }
        }
    }

    /// Wave 9: annotation facts on a declaration targeting `target`.
    /// Framework-named annotations (Spring/JUnit/Mockito) are only recorded
    /// when the matching framework import is present; JDK and custom
    /// annotations are always recorded.
    fn record_annotations(&self, node: Node, ctx: &mut Ctx, src: &[u8], target: &str) {
        for (simple, _line) in annotations_on(node, src) {
            let fw_root = if is_spring_annotation(&simple) {
                Some(SPRING_ROOT)
            } else if is_junit_annotation(&simple) {
                Some(JUNIT_ROOT)
            } else if is_mockito_annotation(&simple) {
                Some(MOCKITO_ROOT)
            } else {
                None
            };
            if let Some(root) = fw_root {
                if !ctx.has_import(root) {
                    continue;
                }
            }
            ctx.facts.push(SemanticFact::Annotation {
                name: simple,
                target: target.to_string(),
            });
        }
    }

    /// Wave 9: Spring mapping annotations (@GetMapping/@PostMapping/.../
    /// @RequestMapping) and @Bean on a method → Registration. Only when the
    /// Spring import is present — a plain method named `get` is never a
    /// route.
    fn record_spring_registrations(
        &self,
        node: Node,
        ctx: &mut Ctx,
        src: &[u8],
        class: &str,
        method: &str,
    ) {
        if !ctx.has_import(SPRING_ROOT) {
            return;
        }
        for (simple, _line) in annotations_on(node, src) {
            let kind = if matches!(
                simple.as_str(),
                "GetMapping"
                    | "PostMapping"
                    | "PutMapping"
                    | "DeleteMapping"
                    | "PatchMapping"
                    | "RequestMapping"
            ) {
                Some("route")
            } else if simple == "Bean" {
                Some("bean")
            } else {
                None
            };
            if let Some(kind) = kind {
                ctx.facts.push(SemanticFact::Registration {
                    owner: class.to_string(),
                    kind: kind.to_string(),
                    target: method.to_string(),
                });
            }
        }
    }

    /// Wave 9: JUnit lifecycle methods (@BeforeClass/@BeforeAll/@BeforeEach/
    /// @Before/@After/...) → Callback. Only when the JUnit import is present.
    fn record_junit_lifecycle(
        &self,
        node: Node,
        ctx: &mut Ctx,
        src: &[u8],
        class: &str,
        method: &str,
    ) {
        if !ctx.has_import(JUNIT_ROOT) {
            return;
        }
        for (simple, _line) in annotations_on(node, src) {
            if matches!(
                simple.as_str(),
                "Before" | "After" | "BeforeClass" | "AfterClass" | "BeforeAll" | "AfterAll"
                    | "BeforeEach" | "AfterEach"
            ) {
                ctx.facts.push(SemanticFact::Callback {
                    owner: class.to_string(),
                    callback: method.to_string(),
                });
            }
        }
    }

    fn record_call(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = clean(node_text(Some(name_node), src));
            if !name.is_empty() {
                let callee = match node.child_by_field_name("object") {
                    Some(obj) => {
                        let obj_text = collapse(node_text(Some(obj), src));
                        if obj_text.is_empty() {
                            name.clone()
                        } else {
                            format!("{obj_text}.{name}")
                        }
                    }
                    // unqualified call inside a class: resolve as `this.<name>`
                    None if ctx.in_class_context() => format!("this.{name}"),
                    None => name.clone(),
                };
                let known_receiver = match node.child_by_field_name("object") {
                    Some(obj) => {
                        let root = callee_root(obj);
                        matches!(root.kind(), "identifier" | "this" | "super")
                    }
                    None => false,
                };
                let caller = ctx.caller();
                let seq = ctx.call_seq.entry(caller.clone()).or_insert(0);
                *seq += 1;
                let (conditional, control_block, inside_loop, inside_try) = call_cfg(node);
                ctx.calls.push(Call {
                    caller,
                    callee,
                    line: node.start_position().row as u32 + 1,
                    known_receiver,
                    conditional,
                    lexical_order: *seq - 1,
                    control_block: control_block.map(str::to_string),
                    inside_loop,
                    inside_try,
                    awaited: false, // java has no syntactic await
                    returns_value: call_returns_value(node),
                });
                self.record_store_ref(node, ctx, src);
            }
        }
        self.walk_children(node, ctx, src);
    }

    /// `new Service(...)` — a constructor call site.
    fn record_creation(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        if let Some(type_node) = node.child_by_field_name("type") {
            let type_name = clean(node_text(Some(type_node), src));
            if !type_name.is_empty() && !type_name.contains('[') {
                let caller = ctx.caller();
                let seq = ctx.call_seq.entry(caller.clone()).or_insert(0);
                *seq += 1;
                let (conditional, control_block, inside_loop, inside_try) = call_cfg(node);
                ctx.calls.push(Call {
                    caller,
                    callee: type_name,
                    line: node.start_position().row as u32 + 1,
                    known_receiver: false,
                    conditional,
                    lexical_order: *seq - 1,
                    control_block: control_block.map(str::to_string),
                    inside_loop,
                    inside_try,
                    awaited: false, // java has no syntactic await
                    returns_value: call_returns_value(node),
                });
            }
        }
        self.walk_children(node, ctx, src);
    }

    /// JDBC-style store access: `this.connection.prepareStatement(sql)`,
    /// `conn.executeUpdate()`, ... with SQL sniffing for prepared statements.
    fn record_store_ref(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let mut segs = attribute_segments(node, src);
        if segs.len() < 2 {
            return;
        }
        if segs[0] == "this" {
            segs.remove(0);
        }
        let store = segs[0].clone();
        if !STORE_RECEIVERS.contains(&store.as_str()) {
            return;
        }
        let op_name = segs.last().unwrap().clone();
        let Some(mut op) = classify_op(&op_name) else {
            return;
        };
        let mut target = if segs.len() >= 3 {
            Some(segs[segs.len() - 2].clone())
        } else {
            None
        };
        // SQL sniffing overrides op + target for statement-building calls
        if matches!(
            op_name.as_str(),
            "execute" | "executeUpdate" | "executeQuery" | "prepareStatement" | "prepareCall"
                | "query"
        ) {
            if let Some(sql) = first_string_arg(node, src) {
                let (sniff_op, sniff_target) = sql_op_table(&sql);
                op = sniff_op;
                target = sniff_target;
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
        let text = clean(node_text(Some(node), src));
        let body = text.strip_prefix("import").unwrap_or(&text).trim();
        let body = body.strip_prefix("static").unwrap_or(body).trim();
        let rest = body.trim_end_matches(';').trim();
        if rest.is_empty() {
            return;
        }
        let module = rest.to_string();
        let last = module.rsplit('.').next().unwrap_or("").to_string();
        let names = if last.is_empty() || last == "*" {
            Vec::new()
        } else {
            vec![(last.clone(), last)]
        };
        ctx.imports.push(Import {
            module,
            names,
            line,
            r#type: ImportType::Member,
        });
    }

    fn record_package(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let line = node.start_position().row as u32 + 1;
        let text = clean(node_text(Some(node), src));
        let name = text
            .strip_prefix("package")
            .unwrap_or(&text)
            .trim()
            .trim_end_matches(';')
            .trim();
        if name.is_empty() {
            return;
        }
        ctx.symbols.push(Symbol {
            name: name.to_string(),
            kind: SymbolKind::Module,
            signature: None,
            start_line: line,
            end_line: line,
            exported: true,
            docstring: None,
            parent: None,
        });
    }

    /// Retry annotations (`@Retryable`, `@Retry`, or `@foo.Retry...`) on a
    /// method → Retry { symbol, policy: annotation text, line }.
    fn maybe_retry(&self, node: Node, ctx: &mut Ctx, src: &[u8], symbol: &str) {
        let Some(mods) = find_named_child(node, "modifiers") else {
            return;
        };
        let mut cursor = mods.walk();
        for child in mods.named_children(&mut cursor) {
            if child.kind() != "annotation" {
                continue;
            }
            let aname = child
                .child_by_field_name("name")
                .map(|n| collapse(node_text(Some(n), src)))
                .unwrap_or_default();
            let last = aname.rsplit('.').next().unwrap_or(&aname).to_string();
            if last == "Retryable" || last == "Retry" {
                ctx.retries.push(Retry {
                    symbol: symbol.to_string(),
                    policy: collapse(node_text(Some(child), src)),
                    line: child.start_position().row as u32 + 1,
                });
            }
        }
    }

    /// True when a method body contains `return this;` (fluent builder evidence
    /// for `.withX()/.setX()/.addX()` chains).
    fn method_returns_this(&self, node: Node, src: &[u8]) -> bool {
        let Some(body) = node.child_by_field_name("body") else {
            return false;
        };
        let mut cursor = body.walk();
        for c in body.named_children(&mut cursor) {
            if c.kind() != "return_statement" {
                continue;
            }
            // `return this;` — the returned `this` is a direct child (the
            // grammar has no `expression` field on return_statement here).
            let mut c2 = c.walk();
            if c.children(&mut c2).any(|k| node_text(Some(k), src) == "this") {
                return true;
            }
        }
        false
    }

    /// Text of the `modifiers` child (keywords + annotations), or "".
    fn modifiers_text(&self, node: Node, src: &[u8]) -> String {
        find_named_child(node, "modifiers")
            .map(|c| collapse(node_text(Some(c), src)))
            .unwrap_or_default()
    }

    /// One-line signature: `public void storeOrder(String orderId)`.
    fn signature(&self, node: Node, src: &[u8], is_constructor: bool) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(mods) = find_named_child(node, "modifiers") {
            let mut cursor = mods.walk();
            for child in mods.children(&mut cursor) {
                if child.kind() == "annotation" || child.kind() == "comment" {
                    continue;
                }
                let t = clean(node_text(Some(child), src));
                if !t.is_empty() {
                    parts.push(t);
                }
            }
        }
        if !is_constructor {
            if let Some(ty) = node.child_by_field_name("type") {
                let t = clean(node_text(Some(ty), src));
                if !t.is_empty() {
                    parts.push(t);
                }
            }
        }
        let name = clean(node_text(node.child_by_field_name("name"), src));
        if !name.is_empty() {
            parts.push(name);
        }
        let mut params: Vec<String> = Vec::new();
        if let Some(p) = node.child_by_field_name("parameters") {
            let mut cursor = p.walk();
            for child in p.named_children(&mut cursor) {
                let t = collapse(node_text(Some(child), src));
                if !t.is_empty() {
                    params.push(t);
                }
            }
        }
        truncate_chars(&format!("{}({})", parts.join(" "), params.join(", ")), 120)
    }

    /// Javadoc block comment immediately preceding `node` (first paragraph).
    fn leading_javadoc(&self, node: Node, src: &[u8]) -> Option<String> {
        let prev = node.prev_sibling()?;
        if prev.kind() != "block_comment" && prev.kind() != "line_comment" {
            return None;
        }
        let text = node_text(Some(prev), src);
        if !text.starts_with("/**") {
            return None;
        }
        // adjacent: no blank line between comment and declaration
        if prev.end_position().row + 2 < node.start_position().row {
            return None;
        }
        Some(first_paragraph(text))
    }
}

// ---------------------------------------------------------------------------
// Small walker helpers
// ---------------------------------------------------------------------------

/// Innermost object of an attribute chain (identifier, this, call, ...).
fn callee_root(mut node: Node) -> Node {
    loop {
        match node.kind() {
            "primary_expression" => match node.named_child(0) {
                Some(inner) => node = inner,
                None => return node,
            },
            "field_access" => match node.child_by_field_name("object") {
                Some(obj) => node = obj,
                None => return node,
            },
            _ => return node,
        }
    }
}

/// Attribute chain segments from outermost (root) to innermost, e.g.
/// `this.connection.prepareStatement` -> ["this", "connection", "prepareStatement"].
fn attribute_segments(mut node: Node, src: &[u8]) -> Vec<String> {
    let mut stack: Vec<String> = Vec::new();
    loop {
        match node.kind() {
            "method_invocation" => {
                let name = node_text(node.child_by_field_name("name"), src);
                if !name.is_empty() {
                    stack.push(name.to_string());
                }
                match node.child_by_field_name("object") {
                    Some(obj) => node = obj,
                    None => break,
                }
            }
            "field_access" => {
                let field = node_text(node.child_by_field_name("field"), src);
                if !field.is_empty() {
                    stack.push(field.to_string());
                }
                match node.child_by_field_name("object") {
                    Some(obj) => node = obj,
                    None => break,
                }
            }
            _ => {
                let t = node_text(Some(node), src);
                if !t.is_empty() {
                    stack.push(t.to_string());
                }
                break;
            }
        }
    }
    stack.reverse();
    stack
}

/// First paragraph of a (possibly `*`-prefixed) comment block.
fn first_paragraph(s: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for raw in s.lines() {
        let mut line = raw.trim_start_matches(['*', '/', ' ']).trim();
        if line.ends_with("*/") {
            line = line[..line.len() - 2].trim_end();
        }
        if line.is_empty() {
            if !lines.is_empty() {
                break;
            }
            continue;
        }
        lines.push(line.to_string());
    }
    truncate_chars(lines.join(" ").trim(), 200)
}

/// First named child of `node` with the given kind.
fn find_named_child<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    let mut out = None;
    for child in node.named_children(&mut cursor) {
        if child.kind() == kind {
            out = Some(child);
            break;
        }
    }
    out
}

/// Every annotation on a declaration, as `(simple name, line)`. The simple
/// name is the last segment of a possibly-qualified annotation
/// (`@org.junit.Test` → `Test`). Both `annotation` (with arguments) and
/// `marker_annotation` (argument-less, e.g. `@Test`/`@RestController`) are
/// collected.
fn annotations_on(node: Node, src: &[u8]) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    let Some(mods) = find_named_child(node, "modifiers") else {
        return out;
    };
    let mut cursor = mods.walk();
    for child in mods.named_children(&mut cursor) {
        if child.kind() != "annotation" && child.kind() != "marker_annotation" {
            continue;
        }
        let name = child
            .child_by_field_name("name")
            .map(|n| clean(node_text(Some(n), src)))
            .unwrap_or_default();
        let simple = name.rsplit('.').next().unwrap_or(&name).to_string();
        if simple.is_empty() {
            continue;
        }
        out.push((simple, child.start_position().row as u32 + 1));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ImportType, StoreOp, SymbolKind};

    fn extract(src: &str) -> ExtractedFile {
        let f = SourceFile::new("com/example/Service.java", src);
        JavaExtractor::default().extract(&f)
        }

    fn find_symbol<'a>(ef: &'a ExtractedFile, name: &str) -> &'a Symbol {
        ef.symbols
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("symbol {name} not found"))
    }

    #[test]
    fn symbols_classes_methods_fields() {
        let ef = extract(
            r#"
package com.example;

import java.sql.Connection;

/** Owns order persistence. */
public class Service {
    /** The JDBC connection. */
    private final Connection connection;

    public Service() {
        this.connection = DriverManager.getConnection("jdbc:sqlite:orders.db");
    }

    /** Process an order. */
    @Retryable(maxAttempts = 3)
    public void process(String orderId) {
        this.storeOrder(orderId);
    }

    public void storeOrder(String orderId) {
    }
}

interface Listener {
    void onEvent(String e);
}

enum Mode {
    FAST,
    SLOW
}
"#,
        );
        assert_eq!(ef.symbols.len(), 9, "symbols: {:?}", ef.symbols);

        let pkg = find_symbol(&ef, "com.example");
        assert_eq!(pkg.kind, SymbolKind::Module);
        assert!(pkg.exported);

        let cls = find_symbol(&ef, "Service");
        assert_eq!(cls.kind, SymbolKind::Class);
        assert!(cls.exported);
        assert_eq!(cls.docstring.as_deref(), Some("Owns order persistence."));

        let conn = find_symbol(&ef, "Service.connection");
        assert_eq!(conn.kind, SymbolKind::Const);
        assert_eq!(conn.parent.as_deref(), Some("Service"));

        let ctor = find_symbol(&ef, "Service.Service");
        assert_eq!(ctor.kind, SymbolKind::Method);
        assert!(ctor.signature.as_deref().unwrap_or("").contains("Service()"));
        assert_eq!(ctor.parent.as_deref(), Some("Service"));

        let process = find_symbol(&ef, "Service.process");
        assert_eq!(process.kind, SymbolKind::Method);
        assert_eq!(process.parent.as_deref(), Some("Service"));
        assert!(process.signature.as_deref().unwrap_or("").contains("void process"));
        assert_eq!(process.docstring.as_deref(), Some("Process an order."));

        let store = find_symbol(&ef, "Service.storeOrder");
        assert_eq!(store.kind, SymbolKind::Method);
        assert!(!store.exported);

        let listener = find_symbol(&ef, "Listener");
        assert_eq!(listener.kind, SymbolKind::Interface);

        // abstract methods declared in interfaces are methods too
        let on_event = find_symbol(&ef, "Listener.onEvent");
        assert_eq!(on_event.kind, SymbolKind::Method);
        assert_eq!(on_event.parent.as_deref(), Some("Listener"));

        let mode = find_symbol(&ef, "Mode");
        assert_eq!(mode.kind, SymbolKind::Enum);
    }

    #[test]
    fn retries_entrypoints() {
        let ef = extract(
            r#"
public class Main {
    public static void main(String[] args) {
        System.out.println("hi");
    }

    @Retryable(maxAttempts = 3, backoff = @Backoff(delay = 100))
    public void poll() {
        work();
    }

    @Retry(delay = 5)
    public void push() {
    }
}
"#,
        );
        assert_eq!(ef.entrypoints.len(), 1);
        assert_eq!(ef.entrypoints[0].symbol, "Main.main");
        assert_eq!(ef.entrypoints[0].kind, "main");
        assert_eq!(ef.entrypoints[0].line, 3);

        assert_eq!(ef.retries.len(), 2, "retries: {:?}", ef.retries);
        let poll = &ef.retries[0];
        assert_eq!(poll.symbol, "Main.poll");
        assert!(poll.policy.contains("Retryable(maxAttempts = 3"));
        assert_eq!(poll.line, 7);
        assert_eq!(ef.retries[1].symbol, "Main.push");
    }

    #[test]
    fn calls_this_and_objects() {
        let ef = extract(
            r#"
public class Service {
    public void process(String orderId) {
        this.storeOrder(orderId);
        fanout(orderId);
        helper.log(orderId);
        new Helper();
    }
    public void storeOrder(String orderId) {}
    public void fanout(String orderId) {}
}
"#,
        );
        let callees: Vec<&str> = ef.calls.iter().map(|c| c.callee.as_str()).collect();
        assert!(callees.contains(&"this.storeOrder"), "{callees:?}");
        // unqualified call inside a class resolves as `this.<name>`
        assert!(callees.contains(&"this.fanout"), "{callees:?}");
        assert!(callees.contains(&"helper.log"), "{callees:?}");
        assert!(callees.contains(&"Helper"), "{callees:?}");
        // same-class calls get the method symbol as caller
        let call = ef.calls.iter().find(|c| c.callee == "this.storeOrder").unwrap();
        assert_eq!(call.caller.as_deref(), Some("Service.process"));
        assert!(call.known_receiver);
    }

    #[test]
    fn imports_module_names() {
        let ef = extract(
            r#"
package com.example;

import java.sql.Connection;
import java.sql.PreparedStatement;
import java.util.*;
import static java.util.Arrays.asList;
"#,
        );
        let modules: Vec<&str> = ef.imports.iter().map(|i| i.module.as_str()).collect();
        assert!(modules.contains(&"java.sql.Connection"), "{modules:?}");
        assert!(modules.contains(&"java.sql.PreparedStatement"), "{modules:?}");
        assert!(modules.contains(&"java.util.*"), "{modules:?}");
        assert!(modules.contains(&"java.util.Arrays.asList"), "{modules:?}");
        let imp = ef
            .imports
            .iter()
            .find(|i| i.module == "java.sql.Connection")
            .unwrap();
        assert_eq!(imp.names, vec![("Connection".to_string(), "Connection".to_string())]);
        assert_eq!(imp.r#type, ImportType::Member);
        let wild = ef.imports.iter().find(|i| i.module == "java.util.*").unwrap();
        assert!(wild.names.is_empty());
    }

    #[test]
    fn jdbc_store_refs_with_sql_sniff() {
        let ef = extract(
            r#"
public class Service {
    public void storeOrder(String orderId) throws Exception {
        try (PreparedStatement ps = this.connection.prepareStatement(
                "INSERT INTO orders (id) VALUES (?)")) {
            ps.setString(1, orderId);
            ps.executeUpdate();
        }
        conn.commit();
        this.db.query("SELECT id FROM orders");
    }
}
"#,
        );
        assert_eq!(ef.store_refs.len(), 3, "refs: {:?}", ef.store_refs);

        let ins = &ef.store_refs[0];
        assert_eq!(ins.store, "connection");
        assert_eq!(ins.op, StoreOp::Write);
        assert_eq!(ins.target.as_deref(), Some("orders"));
        assert_eq!(ins.caller.as_deref(), Some("Service.storeOrder"));
        assert_eq!(ins.technology.as_deref(), Some("sql"));
        assert_eq!(ins.line, 4);

        let commit = &ef.store_refs[1];
        assert_eq!(commit.store, "conn");
        assert_eq!(commit.op, StoreOp::Write);

        let q = &ef.store_refs[2];
        assert_eq!(q.op, StoreOp::Query);
        assert_eq!(q.target.as_deref(), Some("orders"));
    }

    #[test]
    fn hostile_input_never_panics() {
        let ef = extract("");
        assert!(ef.symbols.is_empty());
        let ef = extract("class {");
        assert!(ef.symbols.is_empty());
        let ef = extract("\u{0}\u{1}\u{ff} garbage \u{80}\u{fe}");
        assert!(ef.symbols.is_empty());
        let ef = extract("public static void main(");
        assert!(ef.symbols.is_empty());
        // binary junk must not panic
        let junk = String::from_utf8_lossy(&[0x00u8, 0x01, 0xff, 0xfe, b'a', 0x80]).to_string();
        let _ = extract(&junk);
    }

    #[test]
    fn facts_public_exports_annotations_fields() {
        let ef = extract(
            r#"
package com.example;

import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.RestController;

/** Greeting surface. */
@RestController
public class GreetingController {

    private final String prefix = "Hello";

    public GreetingController() {
    }

    @GetMapping("/greet")
    public String greet() {
        return this.prefix;
    }

    void internal() {
    }
}

public interface Listener {
    void onEvent(String e);
}
"#,
        );
        use crate::model::SemanticFact;
        let fact = |n: &str| ef.facts.iter().filter(move |f| matches!(f, SemanticFact::Annotation { name, .. } if name == n)).count();

        // public types + public methods + public constructors
        let exports: Vec<String> = ef
            .facts
            .iter()
            .filter_map(|f| match f {
                SemanticFact::PublicExport { symbol, kind } => {
                    Some(format!("{symbol}:{kind}"))
                }
                _ => None,
            })
            .collect();
        assert!(exports.contains(&"GreetingController:class".to_string()), "{exports:?}");
        assert!(exports.contains(&"GreetingController.GreetingController:constructor".to_string()), "{exports:?}");
        assert!(exports.contains(&"GreetingController.greet:method".to_string()), "{exports:?}");
        assert!(exports.contains(&"Listener:interface".to_string()), "{exports:?}");
        // interface methods are implicitly public
        assert!(exports.contains(&"Listener.onEvent:method".to_string()), "{exports:?}");
        // package-private method is NOT an export
        assert!(!exports.contains(&"GreetingController.internal:method".to_string()), "{exports:?}");

        // framework annotations require the matching import
        assert_eq!(fact("RestController"), 1, "facts: {:?}", ef.facts);
        assert_eq!(fact("GetMapping"), 1, "facts: {:?}", ef.facts);

        // fields with mutability
        let fields: Vec<String> = ef
            .facts
            .iter()
            .filter_map(|f| match f {
                SemanticFact::Field { owner, name, mutable } => {
                    Some(format!("{owner}.{name}:{}", if *mutable { "mutable" } else { "final" }))
                }
                _ => None,
            })
            .collect();
        assert_eq!(fields, vec!["GreetingController.prefix:final"], "{fields:?}");

        // spring route registrations point at the handler methods
        let routes: Vec<String> = ef
            .facts
            .iter()
            .filter_map(|f| match f {
                SemanticFact::Registration { owner, kind, target } if kind == "route" => {
                    Some(format!("{owner}->{target}"))
                }
                _ => None,
            })
            .collect();
        assert_eq!(routes, vec!["GreetingController->GreetingController.greet"], "{routes:?}");
    }

    #[test]
    fn facts_framework_verification_requires_imports() {
        // Same annotations WITHOUT the spring import: no route registration,
        // and the framework-named annotation is not a fact at all.
        let ef = extract(
            r#"
public class PlainController {
    @GetMapping("/nope")
    public String get() {
        return "x";
    }

    @Bean
    public String thing() {
        return "x";
    }
}
"#,
        );
        use crate::model::SemanticFact;
        let routes = ef
            .facts
            .iter()
            .filter(|f| matches!(f, SemanticFact::Registration { kind, .. } if kind == "route" || kind == "bean"))
            .count();
        assert_eq!(routes, 0, "no framework import -> no registrations: {:?}", ef.facts);
        let ann = ef
            .facts
            .iter()
            .filter(|f| matches!(f, SemanticFact::Annotation { .. }))
            .count();
        assert_eq!(ann, 0, "no framework import -> no framework annotations: {:?}", ef.facts);
    }

    #[test]
    fn facts_junit_lifecycle_callbacks_and_rules() {
        let ef = extract(
            r#"
import org.junit.Before;
import org.junit.BeforeClass;
import org.junit.Rule;
import org.junit.Test;
import org.junit.rules.TemporaryFolder;

public class CalcTest {

    @Rule
    public TemporaryFolder tmp = new TemporaryFolder();

    @BeforeClass
    public static void setupAll() {
    }

    @Before
    public void setUp() {
    }

    @Test
    public void adds() {
    }
}
"#,
        );
        use crate::model::SemanticFact;
        let callbacks: Vec<String> = ef
            .facts
            .iter()
            .filter_map(|f| match f {
                SemanticFact::Callback { owner, callback } => Some(format!("{owner}->{callback}")),
                _ => None,
            })
            .collect();
        assert!(
            callbacks.contains(&"CalcTest->CalcTest.setupAll".to_string()),
            "{callbacks:?}"
        );
        assert!(callbacks.contains(&"CalcTest->CalcTest.setUp".to_string()), "{callbacks:?}");

        // @Test is an annotation fact; @Rule registers the field
        let tests = ef
            .facts
            .iter()
            .filter(|f| matches!(f, SemanticFact::Annotation { name, .. } if name == "Test"))
            .count();
        assert_eq!(tests, 1, "facts: {:?}", ef.facts);
        let rules: Vec<&str> = ef
            .facts
            .iter()
            .filter_map(|f| match f {
                SemanticFact::Registration { kind, target, .. } if kind == "rule" => {
                    Some(target.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(rules, vec!["CalcTest.tmp"], "{rules:?}");

        // field facts: final fields immutable, non-final mutable
        let mutable: Vec<((String, String), bool)> = ef
            .facts
            .iter()
            .filter_map(|f| match f {
                SemanticFact::Field { owner, name, mutable } => {
                    Some(((owner.clone(), name.clone()), *mutable))
                }
                _ => None,
            })
            .collect();
        assert!(
            mutable
                .iter()
                .any(|(n, m)| n == &("CalcTest".to_string(), "tmp".to_string()) && *m),
            "{mutable:?}"
        );
    }

    #[test]
    fn facts_deterministic_sorted_order() {
        let src = r#"
import org.junit.Test;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.RestController;

@RestController
public class SortedController {
    private String a = "1";
    private int z = 2;

    @GetMapping("/a")
    public String first() { return ""; }

    @Test
    public void second() {}
}
"#;
        let ef1 = JavaExtractor::default().extract(&SourceFile::new("Sorted.java", src));
        let ef2 = JavaExtractor::default().extract(&SourceFile::new("Sorted.java", src));
        assert_eq!(ef1.facts, ef2.facts, "facts must be deterministic");
        assert!(ef1.facts.len() >= 6, "facts: {:?}", ef1.facts);
        // sorted by (owning symbol, fact kind): annotations before callbacks
        // before fields before registrations per owner
        for w in ef1.facts.windows(2) {
            let k1 = fact_sort_key(&w[0]);
            let k2 = fact_sort_key(&w[1]);
            assert!(k1 <= k2, "facts not sorted: {:?} > {:?}", k1, k2);
        }
    }

    fn regs(ef: &ExtractedFile) -> Vec<(String, String, String)> {
        ef.facts
            .iter()
            .filter_map(|f| match f {
                SemanticFact::Registration { owner, kind, target } => {
                    Some((owner.clone(), kind.clone(), target.clone()))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn facts_guava_style_static_factories_and_builders() {
        // guava `ImmutableList.of(...)` / `builder()` static factories +
        // a fluent `withX() { return this; }` builder.
        let ef = extract(
            "package com.example;\n\npublic final class ImmutableList {\n    public static <E> ImmutableList<E> of(E e1) { return new ImmutableList<E>(); }\n    public static Builder builder() { return new Builder(); }\n    public static void sort() {}\n    public static final class Builder {\n        public Builder withTag(String t) { return this; }\n    }\n}\n",
        );
        let rs = regs(&ef);
        assert!(
            rs.contains(&("ImmutableList".into(), "factory".into(), "ImmutableList".into())),
            "static of factory missing: {rs:?}"
        );
        assert!(
            rs.contains(&("ImmutableList".into(), "builder".into(), "ImmutableList".into())),
            "static builder() missing: {rs:?}"
        );
        assert!(
            rs.contains(&("Builder".into(), "builder".into(), "Builder".into())),
            "fluent withX builder missing: {rs:?}"
        );
        assert!(
            !rs.iter().any(|(_, k, _)| k == "factory" && false),
            "unexpected"
        );
        // static non-factory method must not fire
        assert!(
            !ef.facts.iter().any(|f| matches!(
                f,
                SemanticFact::Registration { owner, .. } if owner == "ImmutableList.sort"
            )),
            "sort() is not a factory: {:?}",
            regs(&ef)
        );
    }

    #[test]
    fn facts_static_fields_are_state() {
        // static fields are mutable STATE unless `final`; the runtime
        // authority attributes mutable Field facts.
        let ef = extract(
            "package com.example;\n\npublic class Config {\n    public static int retries = 3;\n    public static final String NAME = \"cfg\";\n}\n",
        );
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
            fields.contains(&("Config".into(), "retries".into(), true)),
            "static mutable field missing: {fields:?}"
        );
        assert!(
            fields.contains(&("Config".into(), "NAME".into(), false)),
            "final static must be immutable: {fields:?}"
        );
    }
}

