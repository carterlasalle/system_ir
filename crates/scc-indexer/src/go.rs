//! Go language extractor (tree-sitter based).
//!
//! Pure, deterministic extraction: `(path, content) -> ExtractedFile`.
//! Syntax-level only; cross-file resolution happens in `resolve.rs`.
//! Mirrors `python.rs` structurally: document-order walk, defensive
//! unwraps_or everywhere — hostile input never panics.

use crate::model::{
    Call, Entrypoint, ExtractedFile, Import, ImportType, LanguageExtractor, SourceFile, StoreOp,
    StoreRef, Symbol, SymbolKind,
};
use tree_sitter::{Node, Parser};
use std::collections::{BTreeMap, BTreeSet};

/// Go extractor. Uses the tree-sitter-go grammar.
pub struct GoExtractor {
    language: tree_sitter::Language,
}

impl Default for GoExtractor {
    fn default() -> Self {
        GoExtractor {
            language: tree_sitter_go::LANGUAGE.into(),
        }
    }
}

impl LanguageExtractor for GoExtractor {
    fn language(&self) -> &'static str {
        "go"
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
        // prepass: `var cmd = ...cobra.Command{Use: "x"}` bindings, used to
        // name `rootCmd.AddCommand(cmd)` subcommands.
        scan_cobra_commands(tree.root_node(), src, &mut ctx.command_uses);
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

/// Strip Go string-literal quoting: `'x'`, `"x"`, or `` `x` `` (raw).
fn strip_quotes(raw: &str) -> Option<String> {
    let s = raw.trim();
    let bytes = s.as_bytes();
    if bytes.len() < 2 {
        return None;
    }
    let q = bytes[0];
    if q != b'\'' && q != b'"' && q != b'`' {
        return None;
    }
    if bytes[bytes.len() - 1] != q {
        return None;
    }
    Some(s[1..s.len() - 1].to_string())
}

/// Value of a Go string-literal node (interpreted or raw).
fn string_literal_value(node: Node, src: &[u8]) -> Option<String> {
    match node.kind() {
        "interpreted_string_literal" | "raw_string_literal" => {
            strip_quotes(node_text(Some(node), src))
        }
        _ => None,
    }
}

/// First string-literal argument of a call.
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

/// Client variable roots that identify a store access. `sql` is
/// deliberately absent: `sql.Open(...)` is a package call, not a store ref.
const STORE_RECEIVERS: &[&str] = &[
    "db", "database", "conn", "client", "pool", "session", "engine", "redis", "r", "kafka",
    "producer", "consumer", "queue", "mongo", "collection", "s3", "bucket", "cache", "es",
    "elasticsearch", "supabase", "firestore", "dynamodb", "table",
];

/// Roots for which a string literal argument names the target (key/topic).
const STRING_TARGET_ROOTS: &[&str] = &[
    "redis", "r", "kafka", "producer", "consumer", "broker", "queue",
];

fn classify_op(name: &str) -> Option<StoreOp> {
    match name {
        "Exec" | "Query" | "QueryRow" => Some(StoreOp::Query), // refined by SQL sniff
        "Insert" | "Update" | "Delete" | "Create" | "Set" | "Save" | "Add" | "Remove" | "Upsert"
        | "Commit" => Some(StoreOp::Write),
        "Get" | "Fetch" | "Read" | "Find" | "Count" | "Scan" => Some(StoreOp::Read),
        "Publish" | "Send" | "Produce" => Some(StoreOp::Publish),
        "Subscribe" | "Consume" => Some(StoreOp::Subscribe),
        _ => None,
    }
}

fn technology_for(root: &str) -> Option<String> {
    match root {
        "redis" | "r" => Some("redis".to_string()),
        "kafka" | "producer" | "consumer" | "broker" => Some("kafka".to_string()),
        "mongo" | "collection" => Some("mongodb".to_string()),
        "s3" | "bucket" => Some("s3".to_string()),
        "session" | "engine" | "conn" | "pool" | "db" | "database" => Some("sql".to_string()),
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

/// Enclosing symbol names; the last entry is the current caller.
#[derive(Default)]
struct Ctx {
    symbols: Vec<Symbol>,
    imports: Vec<Import>,
    calls: Vec<Call>,
    store_refs: Vec<StoreRef>,
    entrypoints: Vec<Entrypoint>,
    scopes: Vec<String>,
    /// `var name = ...cobra.Command{Use: "x"}` map (prepass).
    command_uses: BTreeMap<String, String>,
    /// CLI flags per owning symbol (cobra `Flags().StringP(...)`),
    /// `--` prefixed, sorted + deduped.
    cli_flags: BTreeMap<String, BTreeSet<String>>,
    /// Subcommand names already emitted as entrypoints.
    seen_subcommands: BTreeSet<String>,
}

impl Ctx {
    fn caller(&self) -> Option<String> {
        self.scopes.last().cloned()
    }
    fn into_extracted(self) -> ExtractedFile {
        let cli_flags = self
            .cli_flags
            .into_iter()
            .map(|(k, v)| (k, v.into_iter().collect()))
            .collect();
        ExtractedFile {
            symbols: self.symbols,
            imports: self.imports,
            calls: self.calls,
            store_refs: self.store_refs,
            entrypoints: self.entrypoints,
            cli_flags,
            ..ExtractedFile::default()
        }
        }
}

// ---------------------------------------------------------------------------
// Walker
// ---------------------------------------------------------------------------

impl GoExtractor {
    fn walk(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        match node.kind() {
            "function_declaration" => self.walk_function(node, ctx, src),
            "method_declaration" => self.walk_method(node, ctx, src),
            "type_declaration" => {
                self.walk_type_decl(node, ctx, src);
                self.walk_children(node, ctx, src);
            }
            "const_declaration" => {
                self.walk_value_decl(node, ctx, src, SymbolKind::Const);
                self.walk_children(node, ctx, src);
            }
            "var_declaration" => {
                self.walk_value_decl(node, ctx, src, SymbolKind::Const);
                self.walk_children(node, ctx, src);
            }
            "import_declaration" => self.record_import(node, ctx, src),
            "call_expression" => self.record_call(node, ctx, src),
            _ => self.walk_children(node, ctx, src),
        }
    }

    fn walk_children(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, ctx, src);
        }
    }

    fn walk_function(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let name = clean(node_text(node.child_by_field_name("name"), src));
        if name.is_empty() {
            self.walk_children(node, ctx, src);
            return;
        }
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;
        ctx.symbols.push(Symbol {
            name: name.clone(),
            kind: SymbolKind::Function,
            signature: Some(signature(&name, node, src)),
            start_line,
            end_line,
            exported: is_exported(&name),
            docstring: leading_doc(node, src),
            parent: None,
        });
        if name == "main" && ctx.scopes.is_empty() {
            ctx.entrypoints.push(Entrypoint {
                symbol: "main".to_string(),
                kind: "bin".to_string(),
                line: start_line,
            });
        }
        ctx.scopes.push(name);
        self.walk_children(node, ctx, src);
        ctx.scopes.pop();
    }

    fn walk_method(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let name = clean(node_text(node.child_by_field_name("name"), src));
        if name.is_empty() {
            self.walk_children(node, ctx, src);
            return;
        }
        let parent = receiver_type(node.child_by_field_name("receiver"), src);
        let sym_name = match &parent {
            Some(t) if !t.is_empty() => format!("{t}.{name}"),
            _ => name.clone(),
        };
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;
        ctx.symbols.push(Symbol {
            name: sym_name.clone(),
            kind: SymbolKind::Method,
            signature: Some(signature(&name, node, src)),
            start_line,
            end_line,
            exported: is_exported(&name),
            docstring: leading_doc(node, src),
            parent,
        });
        ctx.scopes.push(sym_name);
        self.walk_children(node, ctx, src);
        ctx.scopes.pop();
    }

    /// `type_declaration` → one `type` symbol per `type_spec`.
    fn walk_type_decl(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let mut specs: Vec<Node> = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "type_spec" => specs.push(child),
                "type_spec_list" => {
                    let mut c2 = child.walk();
                    for s in child.named_children(&mut c2) {
                        if s.kind() == "type_spec" {
                            specs.push(s);
                        }
                    }
                }
                _ => {}
            }
        }
        for spec in specs {
            let name = clean(node_text(spec.child_by_field_name("name"), src));
            if name.is_empty() {
                continue;
            }
            ctx.symbols.push(Symbol {
                name: name.clone(),
                kind: SymbolKind::Type,
                signature: None,
                start_line: spec.start_position().row as u32 + 1,
                end_line: spec.end_position().row as u32 + 1,
                exported: is_exported(&name),
                docstring: leading_doc(spec, src),
                parent: None,
            });
        }
    }

    /// `const_declaration` / `var_declaration` → one symbol per spec.
    fn walk_value_decl(
        &self,
        node: Node,
        ctx: &mut Ctx,
        src: &[u8],
        kind: SymbolKind,
    ) {
        let mut specs: Vec<Node> = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "const_spec" | "var_spec" => specs.push(child),
                "const_spec_list" | "var_spec_list" => {
                    let mut c2 = child.walk();
                    for s in child.named_children(&mut c2) {
                        if matches!(s.kind(), "const_spec" | "var_spec") {
                            specs.push(s);
                        }
                    }
                }
                _ => {}
            }
        }
        for spec in specs {
            // `a, b = 1, 2` binds several names; collect them all.
            let mut names: Vec<String> = Vec::new();
            let mut c2 = spec.walk();
            for n in spec.children_by_field_name("name", &mut c2) {
                let t = clean(node_text(Some(n), src));
                if !t.is_empty() {
                    names.push(t);
                }
            }
            for name in names {
                ctx.symbols.push(Symbol {
                    name: name.clone(),
                    kind,
                    signature: None,
                    start_line: spec.start_position().row as u32 + 1,
                    end_line: spec.end_position().row as u32 + 1,
                    exported: is_exported(&name),
                    docstring: None,
                    parent: None,
                });
            }
        }
    }

    fn record_import(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let line = node.start_position().row as u32 + 1;
        let mut specs: Vec<Node> = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "import_spec" => specs.push(child),
                "import_spec_list" => {
                    let mut c2 = child.walk();
                    for s in child.named_children(&mut c2) {
                        if s.kind() == "import_spec" {
                            specs.push(s);
                        }
                    }
                }
                _ => {}
            }
        }
        for spec in specs {
            let path = spec
                .child_by_field_name("path")
                .and_then(|p| string_literal_value(p, src))
                .unwrap_or_default();
            if path.is_empty() {
                continue;
            }
            let alias = clean(node_text(spec.child_by_field_name("name"), src));
            // `.` (dot import) and `_` (blank import) bind nothing locally.
            if alias == "." || alias == "_" {
                continue;
            }
            let local = if alias.is_empty() {
                path.rsplit('/').next().unwrap_or("").to_string()
            } else {
                alias.clone()
            };
            if local.is_empty() || local == "_" {
                continue;
            }
            let names = if alias.is_empty() {
                vec![(local.clone(), local.clone())]
            } else {
                // like python `import a.b as c`: (local, module specifier)
                vec![(local.clone(), path.clone())]
            };
            ctx.imports.push(Import {
                module: path,
                names,
                line,
                r#type: ImportType::Module,
            });
        }
    }

    fn record_call(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        if let Some(fn_node) = node.child_by_field_name("function") {
            let callee = collapse(node_text(Some(fn_node), src));
            if !callee.is_empty() {
                let root = callee_root(fn_node);
                let known_receiver =
                    matches!(root.kind(), "identifier" | "type_identifier" | "field_identifier");
                self.record_cli_surface(node, &callee, ctx, src);
                ctx.calls.push(Call {
                    caller: ctx.caller(),
                    callee,
                    line: node.start_position().row as u32 + 1,
                    known_receiver,
                    conditional: false,
                });
                self.record_store_ref(node, &fn_node, &root, ctx, src);
            }
        }
        self.walk_children(node, ctx, src);
    }

    /// cobra CLI surface: `rootCmd.AddCommand(serveCmd)` registers
    /// subcommand entrypoints (named via the prepass `Use` map or an inline
    /// `cobra.Command{Use: ...}` literal); `cmd.Flags().StringP("paging",
    /// ...)` contributes `--` flags to the enclosing function (the parser
    /// owner).
    fn record_cli_surface(&self, node: Node, callee: &str, ctx: &mut Ctx, src: &[u8]) {
        let line = node.start_position().row as u32 + 1;
        if callee.ends_with(".AddCommand") {
            let Some(args) = node.child_by_field_name("arguments") else {
                return;
            };
            let mut cursor = args.walk();
            for arg in args.named_children(&mut cursor) {
                let name = match arg.kind() {
                    "identifier" => ctx
                        .command_uses
                        .get(&clean(node_text(Some(arg), src)))
                        .cloned(),
                    // inline `&cobra.Command{Use: "serve"}` literal
                    _ => cobra_command_use(arg, src),
                };
                let Some(name) = name else { continue };
                if !ctx.seen_subcommands.insert(name.clone()) {
                    continue;
                }
                ctx.entrypoints.push(Entrypoint {
                    symbol: name,
                    kind: "cli-subcommand".to_string(),
                    line,
                });
            }
            return;
        }
        if !(callee.contains(".Flags().") || callee.contains(".PersistentFlags().")) {
            return;
        }
        let method = if callee.contains(".Flags().") {
            callee.rsplit(".Flags().").next().unwrap_or("")
        } else if callee.contains(".PersistentFlags().") {
            callee.rsplit(".PersistentFlags().").next().unwrap_or("")
        } else {
            return;
        };
        if !PFLAG_METHODS.contains(&method) {
            return;
        }
        let Some(caller) = ctx.caller() else {
            return;
        };
        let Some(name) = first_string_arg(node, src) else {
            return;
        };
        ctx.cli_flags
            .entry(caller)
            .or_default()
            .insert(format!("--{name}"));
    }

    fn record_store_ref(
        &self,
        node: Node,
        fn_node: &Node,
        root: &Node,
        ctx: &mut Ctx,
        src: &[u8],
    ) {
        if !matches!(root.kind(), "identifier" | "type_identifier") {
            return;
        }
        let mut segs: Vec<String> = Vec::new();
        attribute_segments(*fn_node, &mut segs, src);
        if segs.len() < 2 {
            return;
        }
        // `db.Exec(...)` (store = first segment) or receiver-field pattern
        // `s.db.Exec(...)` / `s.redis.Get(...)` (store = second segment).
        let start = if STORE_RECEIVERS.contains(&segs[0].as_str()) {
            0
        } else if segs.len() >= 3 && STORE_RECEIVERS.contains(&segs[1].as_str()) {
            1
        } else {
            return;
        };
        let store = segs[start].clone();
        let op_name = segs.last().unwrap().clone();
        let Some(op) = classify_op(&op_name) else {
            return;
        };
        let mut target = if segs.len() >= start + 3 {
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
        // SQL sniffing for Exec/Query-family ops overrides op + target
        if matches!(op_name.as_str(), "Exec" | "Query" | "QueryRow") {
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
}

// ---------------------------------------------------------------------------
// cobra CLI surface helpers
// ---------------------------------------------------------------------------

/// pflag `FlagSet` registration methods whose first string argument is the
/// flag name (`StringP("paging", "p", ...)`, `BoolVar(&x, "theme", ...)`).
const PFLAG_METHODS: &[&str] = &[
    "Bool", "BoolP", "BoolVar", "BoolVarP", "Count", "CountP", "Duration", "DurationP",
    "DurationVar", "DurationVarP", "Float64", "Float64P", "Float64Var", "Float64VarP", "IP",
    "IPP", "IPVar", "IPVarP", "IPSlice", "IPSliceP", "Int", "IntP", "IntVar", "IntVarP", "Int64",
    "Int64P", "Int64Var", "Int64VarP", "IntSlice", "IntSliceP", "String", "StringP", "StringVar",
    "StringVarP", "StringArray", "StringArrayP", "StringSlice", "StringSliceP", "StringToInt",
    "StringToIntP", "StringToInt64", "StringToInt64P", "StringToString", "StringToStringP",
    "StringToUint", "StringToUintP", "Uint", "UintP", "UintVar", "UintVarP", "Uint64", "Uint64P",
    "Uint64Var", "Uint64VarP", "UintSlice", "UintSliceP", "Var", "VarP",
];

/// The `Use` value of a `cobra.Command{Use: "serve", ...}` literal value
/// (unwrapping `&`/parens), or `None` for anything else.
fn cobra_command_use(expr: Node, src: &[u8]) -> Option<String> {
    let mut n = expr;
    while matches!(n.kind(), "unary_expression" | "parenthesized_expression") {
        let inner = n.named_child(0)?;
        n = inner;
    }
    if n.kind() != "composite_literal" {
        return None;
    }
    // the type is a plain child (`cobra.Command` qualified_type); some
    // grammar versions expose it as a `type` field instead.
    let ty = match n.child_by_field_name("type") {
        Some(t) => t,
        None => {
            let mut cursor = n.walk();
            let mut found: Option<Node> = None;
            for c in n.named_children(&mut cursor) {
                if matches!(c.kind(), "qualified_type" | "type_identifier") {
                    found = Some(c);
                    break;
                }
            }
            found?
        }
    };
    let ty = clean(node_text(Some(ty), src));
    if ty != "cobra.Command" && !ty.ends_with(".cobra.Command") {
        return None;
    }
    let mut cursor = n.walk();
    for c in n.named_children(&mut cursor) {
        if c.kind() != "literal_value" {
            continue;
        }
        let mut c2 = c.walk();
        for el in c.named_children(&mut c2) {
            if el.kind() != "keyed_element" {
                continue;
            }
            // key/value are wrapped in `literal_element` nodes.
            let key = el
                .child_by_field_name("key")
                .map(|k| literal_inner(k))
                .map(|k| clean(node_text(Some(k), src)))
                .unwrap_or_default();
            if key != "Use" {
                continue;
            }
            let v = el.child_by_field_name("value")?;
            return string_literal_value(literal_inner(v), src);
        }
    }
    None
}

/// Descend through `literal_element` wrappers to the underlying value node.
fn literal_inner(mut n: Node) -> Node {
    while n.kind() == "literal_element" {
        match n.named_child(0) {
            Some(inner) => n = inner,
            None => break,
        }
    }
    n
}

/// Prepass: map `var serveCmd = &cobra.Command{Use: "serve"}` (and
/// `serveCmd := ...`) bindings to their `Use` names.
fn scan_cobra_commands(node: Node, src: &[u8], out: &mut BTreeMap<String, String>) {
    let mut cursor = node.walk();
    for c in node.named_children(&mut cursor) {
        match c.kind() {
            "var_declaration" => {
                let mut c2 = c.walk();
                for spec in c.named_children(&mut c2) {
                    if spec.kind() != "var_spec" {
                        continue;
                    }
                    let name = clean(node_text(spec.child_by_field_name("name"), src));
                    let Some(value) = spec.child_by_field_name("value") else {
                        continue;
                    };
                    let mut c3 = value.walk();
                    for expr in value.named_children(&mut c3) {
                        if let Some(u) = cobra_command_use(expr, src) {
                            out.insert(name.clone(), u);
                        }
                    }
                }
            }
            "short_var_declaration" => {
                let mut lefts: Vec<String> = Vec::new();
                let mut lc = c.walk();
                for n in c.children_by_field_name("left", &mut lc) {
                    let t = clean(node_text(Some(n), src));
                    if !t.is_empty() {
                        lefts.push(t);
                    }
                }
                let mut rights: Vec<Node> = Vec::new();
                let mut rc = c.walk();
                for n in c.children_by_field_name("right", &mut rc) {
                    rights.push(n);
                }
                for (name, expr_list) in lefts.iter().zip(rights.iter()) {
                    let mut ec = expr_list.walk();
                    for expr in expr_list.named_children(&mut ec) {
                        if let Some(u) = cobra_command_use(expr, src) {
                            out.insert(name.clone(), u);
                        }
                    }
                }
            }
            _ => scan_cobra_commands(c, src, out),
        }
    }
}

// ---------------------------------------------------------------------------
// Small walker helpers
// ---------------------------------------------------------------------------

/// Innermost object of a selector chain (identifier, literal, call, ...).
fn callee_root(mut node: Node) -> Node {
    loop {
        match node.kind() {
            "selector_expression" => match node.child_by_field_name("operand") {
                Some(op) => node = op,
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

/// Selector chain segments from outermost (root) to method, e.g.
/// `s.db.Exec` → `["s", "db", "Exec"]`.
fn attribute_segments(mut node: Node, out: &mut Vec<String>, src: &[u8]) {
    let mut stack: Vec<String> = Vec::new();
    loop {
        match node.kind() {
            "selector_expression" => {
                let field = node_text(node.child_by_field_name("field"), src);
                if !field.is_empty() {
                    stack.push(field.to_string());
                }
                match node.child_by_field_name("operand") {
                    Some(op) => node = op,
                    None => break,
                }
            }
            "identifier" | "type_identifier" => {
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

/// One-line signature: `func name(args) result`.
fn signature(name: &str, fn_node: Node, src: &[u8]) -> String {
    let mut sig = format!("func {name}(");
    let mut parts: Vec<String> = Vec::new();
    if let Some(params) = fn_node.child_by_field_name("parameters") {
        let mut cursor = params.walk();
        for child in params.named_children(&mut cursor) {
            let t = collapse(node_text(Some(child), src));
            if !t.is_empty() {
                parts.push(t);
            }
        }
    }
    sig.push_str(&parts.join(", "));
    sig.push(')');
    if let Some(result) = fn_node.child_by_field_name("result") {
        let t = collapse(node_text(Some(result), src));
        if !t.is_empty() {
            sig.push(' ');
            sig.push_str(&t);
        }
    }
    truncate_chars(&sig, 120)
}

/// Receiver type of a method: `(s *Store)` / `(s Store)` / `(s *pkg.Store)`
/// → `Store`. Drops pointer/package prefixes and generic instantiation.
fn receiver_type(receiver: Option<Node>, src: &[u8]) -> Option<String> {
    let plist = receiver?;
    let mut cursor = plist.walk();
    let pd = plist.named_children(&mut cursor).next()?;
    let t = pd.child_by_field_name("type")?;
    let mut n = t;
    loop {
        match n.kind() {
            "pointer_type" | "parenthesized_type" => {
                let Some(inner) = n.named_child(0) else {
                    break;
                };
                n = inner;
            }
            "qualified_type" => {
                let Some(name) = n.child_by_field_name("name") else {
                    break;
                };
                n = name;
            }
            _ => break,
        }
    }
    let text = clean(node_text(Some(n), src));
    let text = text.split('[').next().unwrap_or("").trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Go export convention: the first rune is an uppercase ASCII letter.
fn is_exported(name: &str) -> bool {
    name.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false)
}

/// Immediately preceding `//` or `/* */` comment; first paragraph only.
fn leading_doc(node: Node, src: &[u8]) -> Option<String> {
    let mut prev = node.prev_named_sibling();
    if prev.is_none() {
        prev = node.parent().and_then(|p| p.prev_named_sibling());
    }
    let c = prev?;
    if c.kind() != "comment" {
        return None;
    }
    let raw = node_text(Some(c), src);
    let text = raw
        .strip_prefix("//")
        .map(str::to_string)
        .or_else(|| {
            raw.strip_prefix("/*")
                .map(|rest| rest.strip_suffix("*/").unwrap_or(rest).to_string())
        })?;
    Some(first_paragraph(&text))
}

fn first_paragraph(s: &str) -> String {
    let t = s.trim();
    let mut lines: Vec<&str> = Vec::new();
    for line in t.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            if !lines.is_empty() {
                break;
            }
            continue;
        }
        let stripped = trimmed.strip_prefix("//").unwrap_or(trimmed);
        let stripped = stripped.strip_prefix('*').unwrap_or(stripped);
        lines.push(stripped.trim_end());
    }
    let joined = lines.join(" ");
    truncate_chars(joined.trim(), 200)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SymbolKind;

    fn extract(src: &str) -> ExtractedFile {
        let f = SourceFile::new("main.go", src);
        GoExtractor::default().extract(&f)
        }

    fn find_symbol<'a>(ef: &'a ExtractedFile, name: &str) -> &'a Symbol {
        ef.symbols
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("symbol {name} not found"))
    }

    #[test]
    fn symbols_functions_methods_types_consts() {
        let ef = extract(
            "package app\n\n// Greet says hello.\nfunc Greet(name string) string {\n    return \"hi\" + name\n}\n\nconst MaxRetries = 3\nconst (\n    A = 1\n    B = 2\n)\n\nvar Version = \"1.0\"\n\ntype Store struct {\n    db *sql.DB\n}\n\nfunc (s *Store) Save(order string) error {\n    return nil\n}\n\nfunc main() {\n    Greet(\"x\")\n}\n",
        );
        assert_eq!(ef.symbols.len(), 8);

        let greet = find_symbol(&ef, "Greet");
        assert_eq!(greet.kind, SymbolKind::Function);
        assert!(greet.exported);
        assert_eq!(greet.signature.as_deref(), Some("func Greet(name string) string"));
        assert_eq!(greet.start_line, 4);
        assert_eq!(greet.end_line, 6);
        assert_eq!(greet.parent, None);
        assert_eq!(greet.docstring.as_deref(), Some("Greet says hello."));

        let max = find_symbol(&ef, "MaxRetries");
        assert_eq!(max.kind, SymbolKind::Const);
        assert!(max.exported);
        assert_eq!(max.start_line, 8);

        assert_eq!(find_symbol(&ef, "A").kind, SymbolKind::Const);
        assert_eq!(find_symbol(&ef, "B").kind, SymbolKind::Const);
        assert_eq!(find_symbol(&ef, "B").start_line, 11);

        let version = find_symbol(&ef, "Version");
        assert_eq!(version.kind, SymbolKind::Const); // var modeled as const symbol
        assert!(version.exported); // capitalized = exported in Go

        let store = find_symbol(&ef, "Store");
        assert_eq!(store.kind, SymbolKind::Type);
        assert!(store.exported);
        assert_eq!(store.signature, None);

        let save = find_symbol(&ef, "Store.Save");
        assert_eq!(save.kind, SymbolKind::Method);
        assert!(save.exported);
        assert_eq!(save.parent.as_deref(), Some("Store"));
        assert_eq!(save.signature.as_deref(), Some("func Save(order string) error"));

        let main = find_symbol(&ef, "main");
        assert_eq!(main.kind, SymbolKind::Function);
        assert!(!main.exported);
    }

    #[test]
    fn imports_plain_grouped_aliased() {
        let ef = extract(
            "package app\n\nimport \"database/sql\"\n\nimport (\n    \"fmt\"\n    svc \"go-service/internal/service\"\n    _ \"embed\"\n)\n",
        );
        let imps = &ef.imports;
        assert_eq!(imps.len(), 3);

        assert_eq!(imps[0].module, "database/sql");
        assert_eq!(imps[0].names, vec![("sql".into(), "sql".into())]);
        assert_eq!(imps[0].r#type, ImportType::Module);
        assert_eq!(imps[0].line, 3);

        assert_eq!(imps[1].module, "fmt");
        assert_eq!(imps[1].names, vec![("fmt".into(), "fmt".into())]);

        // aliased import binds the alias; module keeps the full path
        assert_eq!(imps[2].module, "go-service/internal/service");
        assert_eq!(
            imps[2].names,
            vec![("svc".into(), "go-service/internal/service".into())]
        );
    }

    #[test]
    fn calls_and_receivers() {
        let ef = extract(
            "package app\n\nfunc top() {\n    helper()\n    fmt.Println(\"x\")\n    srv.serve(1)\n    panic(\"boom\")\n}\n\nfunc helper() {\n    return\n}\n",
        );
        let calls = &ef.calls;
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[0].caller.as_deref(), Some("top"));
        assert_eq!(calls[0].callee, "helper");
        assert!(calls[0].known_receiver);
        assert_eq!(calls[0].line, 4);

        assert_eq!(calls[1].caller.as_deref(), Some("top"));
        assert_eq!(calls[1].callee, "fmt.Println");
        assert!(calls[1].known_receiver);

        assert_eq!(calls[2].caller.as_deref(), Some("top"));
        assert_eq!(calls[2].callee, "srv.serve");
        assert!(calls[2].known_receiver);

        assert_eq!(calls[3].caller.as_deref(), Some("top"));
        assert_eq!(calls[3].callee, "panic");
        assert!(calls[3].known_receiver);
    }

    #[test]
    fn methods_scope_calls_to_receiver_name() {
        let ef = extract(
            "package app\n\ntype Svc struct{}\n\nfunc (s *Svc) Run() {\n    s.Help(1)\n    done()\n}\n\nfunc (s *Svc) Help(x int) {}\n\nfunc done() {}\n",
        );
        let calls = &ef.calls;
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].caller.as_deref(), Some("Svc.Run"));
        assert_eq!(calls[0].callee, "s.Help");
        assert!(calls[0].known_receiver);
        assert_eq!(calls[1].caller.as_deref(), Some("Svc.Run"));
        assert_eq!(calls[1].callee, "done");
    }

    #[test]
    fn store_refs_sql_and_clients() {
        let ef = extract(
            "package app\n\nfunc worker(db *sql.DB, redis *client.Redis) {\n    db.Exec(\"INSERT INTO users (name) VALUES (?)\", \"x\")\n    db.Query(\"SELECT * FROM orders WHERE id = 1\")\n    redis.Get(\"user:1\")\n    s := &svc{}\n    s.db.Exec(\"UPDATE accounts SET bal = 0\")\n    sql.Open(\"sqlite3\", \"x.db\")\n}\n",
        );
        let refs = &ef.store_refs;
        assert_eq!(refs.len(), 4, "refs: {refs:?}");
        let caller = Some("worker".to_string());

        assert_eq!(refs[0].store, "db");
        assert_eq!(refs[0].technology.as_deref(), Some("sql"));
        assert_eq!(refs[0].op, StoreOp::Write);
        assert_eq!(refs[0].target.as_deref(), Some("users"));
        assert_eq!(refs[0].caller, caller);

        assert_eq!(refs[1].store, "db");
        assert_eq!(refs[1].op, StoreOp::Query);
        assert_eq!(refs[1].target.as_deref(), Some("orders"));

        // redis string-literal target
        assert_eq!(refs[2].store, "redis");
        assert_eq!(refs[2].technology.as_deref(), Some("redis"));
        assert_eq!(refs[2].op, StoreOp::Read);
        assert_eq!(refs[2].target.as_deref(), Some("user:1"));

        // receiver-field pattern: `s.db.Exec(...)`
        assert_eq!(refs[3].store, "db");
        assert_eq!(refs[3].op, StoreOp::Write);
        assert_eq!(refs[3].target.as_deref(), Some("accounts"));

        // `sql.Open` is a package call, never a store ref
        assert!(
            !refs.iter().any(|r| r.store == "sql"),
            "sql.Open must not be a store ref"
        );
    }

    #[test]
    fn entrypoint_main() {
        let ef = extract(
            "package main\n\nfunc helper() {}\n\nfunc main() {\n    helper()\n}\n",
        );
        let eps = &ef.entrypoints;
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].symbol, "main");
        assert_eq!(eps[0].kind, "bin");
        assert_eq!(eps[0].line, 5);
    }

    #[test]
    fn cobra_subcommands_and_flags() {
        let ef = extract(
            "package main\n\nimport \"github.com/spf13/cobra\"\n\nvar rootCmd = &cobra.Command{Use: \"cli-service\"}\n\nvar serveCmd = &cobra.Command{\n    Use:   \"serve\",\n    Short: \"serve requests\",\n    Run:   func(cmd *cobra.Command, args []string) {},\n}\n\nvar deployCmd = &cobra.Command{Use: \"deploy\"}\n\nfunc init() {\n    rootCmd.AddCommand(serveCmd, deployCmd)\n    rootCmd.AddCommand(&cobra.Command{Use: \"inline\"})\n    serveCmd.Flags().IntP(\"port\", \"p\", 8080, \"port to listen on\")\n    serveCmd.Flags().Bool(\"paging\", false, \"paged output\")\n    deployCmd.PersistentFlags().StringVar(&env, \"env\", \"dev\", \"target env\")\n}\n\nfunc main() {\n    rootCmd.Execute()\n}\n",
        );
        let eps = &ef.entrypoints;
        assert_eq!(eps.len(), 4, "eps: {eps:?}");
        assert_eq!(eps[0].symbol, "serve");
        assert_eq!(eps[0].kind, "cli-subcommand");
        assert_eq!(eps[0].line, 16);
        assert_eq!(eps[1].symbol, "deploy");
        assert_eq!(eps[1].kind, "cli-subcommand");
        // inline `&cobra.Command{Use: "inline"}` literal arg
        assert_eq!(eps[2].symbol, "inline");
        assert_eq!(eps[2].kind, "cli-subcommand");
        assert_eq!(eps[3].symbol, "main");
        assert_eq!(eps[3].kind, "bin");
        // flags attach to the function that owns the parser (init),
        // `--` prefixed, sorted + deduped
        let flags = ef.cli_flags.get("init").expect("flags on init");
        assert_eq!(flags, &["--env", "--paging", "--port"]);
        assert_eq!(ef.cli_flags.len(), 1);
    }

    #[test]
    fn cobra_use_map_short_decl() {
        let ef = extract(
            "package main\n\nfunc main() {\n    serveCmd := &cobra.Command{Use: \"serve\"}\n    root.AddCommand(serveCmd)\n}\n",
        );
        let eps = &ef.entrypoints;
        assert_eq!(eps.len(), 2, "eps: {eps:?}");
        assert_eq!(eps[0].symbol, "main");
        assert_eq!(eps[0].kind, "bin");
        assert_eq!(eps[1].symbol, "serve");
        assert_eq!(eps[1].kind, "cli-subcommand");
    }

    #[test]
    fn receiver_types() {
        let ef = extract(
            "package app\n\ntype Store struct{}\n\nfunc (s Store) A() {}\nfunc (s *Store) B() {}\nfunc (s *pkg.Store) C() {}\nfunc (s *Store[T]) D() {}\n",
        );
        assert_eq!(find_symbol(&ef, "Store.A").parent.as_deref(), Some("Store"));
        assert_eq!(find_symbol(&ef, "Store.B").parent.as_deref(), Some("Store"));
        assert_eq!(find_symbol(&ef, "Store.C").parent.as_deref(), Some("Store"));
        assert_eq!(find_symbol(&ef, "Store.D").parent.as_deref(), Some("Store"));
    }

    #[test]
    fn malformed_input_does_not_panic() {
        let cases = [
            "func broken(",
            "func broken(\n    return",
            "package \n\nfunc {\n}",
            "\u{0}\u{1}\u{2}\u{ff}\u{fe}",
            "import (",
            "type (",
            "func x() {\n    ((\n}",
            "x := ((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((\n",
        ];
        for c in cases {
            let _ = extract(c);
        }
    }

    #[test]
    fn deterministic_output() {
        let src = "package main\n\nimport \"fmt\"\n\nfunc main() {\n    fmt.Println(\"hi\")\n}\n";
        let a = extract(src);
        let b = extract(src);
        let ja = serde_json::to_string(&a).unwrap();
        let jb = serde_json::to_string(&b).unwrap();
        assert_eq!(ja, jb);
    }
}
