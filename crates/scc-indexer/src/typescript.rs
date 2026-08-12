//! TypeScript/JavaScript extractor (SCC-026).
//!
//! Tree-sitter based, pure, deterministic: `(path, content) -> ExtractedFile`.
//! Emits symbols, imports, calls, routes (express-style + Next.js route
//! handlers), tests, store refs (prisma/knex/redis/kafka/sql/s3), retry
//! decorators, entrypoints, and JSDoc docstrings. Never panics on malformed
//! input: error/missing nodes are skipped and the traversal is iterative
//! (no recursion depth limits).

use crate::model::{
    Call, Entrypoint, ExtractedFile, Import, ImportType, LanguageExtractor, Retry, Route,
    SourceFile, StoreOp, StoreRef, Symbol, SymbolKind, Test, TestKind,
};
use tree_sitter::{Language, Node, Parser};

/// Tree-sitter based extractor for TypeScript and JavaScript.
///
/// `.ts`/`.mts`/`.cts`/`.js`/`.mjs`/`.cjs` use the TypeScript grammar;
/// `.tsx`/`.jsx` use the TSX grammar.
pub struct TypeScriptExtractor {
    language: Language,
    tsx_language: Language,
}

impl Default for TypeScriptExtractor {
    fn default() -> Self {
        TypeScriptExtractor {
            language: tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            tsx_language: tree_sitter_typescript::LANGUAGE_TSX.into(),
        }
    }
}

impl LanguageExtractor for TypeScriptExtractor {
    fn language(&self) -> &'static str {
        "typescript"
    }

    fn extract(&self, file: &SourceFile) -> ExtractedFile {
        let mut parser = Parser::new();
        let is_tsx = file.path.ends_with(".tsx") || file.path.ends_with(".jsx");
        let lang = if is_tsx {
            &self.tsx_language
        } else {
            &self.language
        };
        if parser.set_language(lang).is_err() {
            return ExtractedFile::default();
        }
        let Some(tree) = parser.parse(file.content.as_bytes(), None) else {
            return ExtractedFile::default();
        };
        let root = tree.root_node();
        let src = file.content.as_bytes();

        let is_test_file = is_test_file(&file.path);
        let is_entry_file = is_entry_file(&file.path);
        let is_next_route = is_next_route_file(&file.path);

        let mut out = ExtractedFile::default();

        // Iterative document-order traversal. Each frame carries the context
        // (caller symbol, enclosing class, describe stack) inherited by its
        // subtree. Children are pushed in reverse so pops happen in document
        // order; state is never mutated across sibling subtrees.
        let mut frames: Vec<(Node, Ctx)> = vec![(root, Ctx::default())];
        while let Some((node, ctx)) = frames.pop() {
            if node.is_error() || node.is_missing() {
                continue;
            }
            let inline_handler_node: Option<(String, Node)> = None;
            match node.kind() {
                "import_statement" => {
                    if let Some(imp) = import_from_statement(&node, src) {
                        out.imports.push(imp);
                    }
                }
                "export_statement" => {
                    if let Some(imp) = import_from_export(&node, src) {
                        out.imports.push(imp);
                    }
                }
                "function_declaration" | "generator_function_declaration" => {
                    if ctx.caller.is_none() && ctx.class.is_none() {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            let name = node_text(&name_node, src).to_string();
                            if !name.is_empty() {
                                let start = line_of(&node);
                                let exported = is_exported(&node);
                                out.symbols.push(Symbol {
                                    name: name.clone(),
                                    kind: SymbolKind::Function,
                                    signature: signature_of(&node, src),
                                    start_line: start,
                                    end_line: end_line_of(&node),
                                    exported,
                                    docstring: leading_jsdoc(&node, src),
                                    parent: None,
                                });
                                if is_next_route && exported && is_next_http_method(&name) {
                                    out.routes.push(Route {
                                        method: name.to_uppercase(),
                                        path: next_route_path(&file.path),
                                        handler: Some(name.clone()),
                                        line: start,
                                        framework: "next".into(),
                                    });
                                }
                                if is_test_file && exported && name.starts_with("test") {
                                    out.tests.push(Test {
                                        name: name.clone(),
                                        symbol: Some(name.clone()),
                                        kind: TestKind::Unit,
                                        line: start,
                                    });
                                }
                                for (policy, pline) in collect_retries(&node, src) {
                                    out.retries.push(Retry {
                                        symbol: name.clone(),
                                        policy,
                                        line: pline,
                                    });
                                }
                            }
                        }
                    }
                }
                "class_declaration" | "abstract_class_declaration" => {
                    if ctx.caller.is_none() && ctx.class.is_none() {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            let name = node_text(&name_node, src).to_string();
                            if !name.is_empty() {
                                out.symbols.push(Symbol {
                                    name: name.clone(),
                                    kind: SymbolKind::Class,
                                    signature: None,
                                    start_line: line_of(&node),
                                    end_line: end_line_of(&node),
                                    exported: is_exported(&node),
                                    docstring: leading_jsdoc(&node, src),
                                    parent: None,
                                });
                                for (policy, pline) in collect_retries(&node, src) {
                                    out.retries.push(Retry {
                                        symbol: name.clone(),
                                        policy,
                                        line: pline,
                                    });
                                }
                            }
                        }
                    }
                }
                "method_definition" => {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        let name = node_text(&name_node, src).to_string();
                        if !name.is_empty() {
                            let full = match &ctx.class {
                                Some(c) => format!("{c}.{name}"),
                                None => name.clone(),
                            };
                            out.symbols.push(Symbol {
                                name: full.clone(),
                                kind: SymbolKind::Method,
                                signature: signature_of(&node, src),
                                start_line: line_of(&node),
                                end_line: end_line_of(&node),
                                exported: false,
                                docstring: leading_jsdoc(&node, src),
                                parent: ctx.class.clone(),
                            });
                            for (policy, pline) in collect_retries(&node, src) {
                                out.retries.push(Retry {
                                    symbol: full.clone(),
                                    policy,
                                    line: pline,
                                });
                            }
                        }
                    }
                }
                "interface_declaration" => {
                    if ctx.caller.is_none() && ctx.class.is_none() {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            let name = node_text(&name_node, src).to_string();
                            if !name.is_empty() {
                                out.symbols.push(Symbol {
                                    name,
                                    kind: SymbolKind::Interface,
                                    signature: None,
                                    start_line: line_of(&node),
                                    end_line: end_line_of(&node),
                                    exported: is_exported(&node),
                                    docstring: leading_jsdoc(&node, src),
                                    parent: None,
                                });
                            }
                        }
                    }
                }
                "type_alias_declaration" => {
                    if ctx.caller.is_none() && ctx.class.is_none() {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            let name = node_text(&name_node, src).to_string();
                            if !name.is_empty() {
                                out.symbols.push(Symbol {
                                    name,
                                    kind: SymbolKind::Type,
                                    signature: None,
                                    start_line: line_of(&node),
                                    end_line: end_line_of(&node),
                                    exported: is_exported(&node),
                                    docstring: leading_jsdoc(&node, src),
                                    parent: None,
                                });
                            }
                        }
                    }
                }
                "enum_declaration" => {
                    if ctx.caller.is_none() && ctx.class.is_none() {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            let name = node_text(&name_node, src).to_string();
                            if !name.is_empty() {
                                out.symbols.push(Symbol {
                                    name,
                                    kind: SymbolKind::Enum,
                                    signature: None,
                                    start_line: line_of(&node),
                                    end_line: end_line_of(&node),
                                    exported: is_exported(&node),
                                    docstring: leading_jsdoc(&node, src),
                                    parent: None,
                                });
                            }
                        }
                    }
                }
                "lexical_declaration" => {
                    if ctx.caller.is_none() && ctx.class.is_none() {
                        let exported = is_exported(&node);
                        let doc = leading_jsdoc(&node, src);
                        let mut cur = node.walk();
                        for d in node.named_children(&mut cur) {
                            if d.kind() != "variable_declarator" {
                                continue;
                            }
                            let Some(name_node) = d.child_by_field_name("name") else {
                                continue;
                            };
                            if name_node.kind() != "identifier" {
                                continue; // destructuring patterns are not symbols
                            }
                            let name = node_text(&name_node, src).to_string();
                            if name.is_empty() {
                                continue;
                            }
                            // `const x = require("m")` binds the module root.
                            if let Some(v) = d.child_by_field_name("value") {
                                if v.kind() == "call_expression" {
                                    if let Some(f) = v.child_by_field_name("function") {
                                        if f.kind() == "identifier"
                                            && node_text(&f, src) == "require"
                                        {
                                            if let Some(module) = first_string_arg(&v, src) {
                                                out.imports.push(Import {
                                                    module,
                                                    names: vec![(name.clone(), "default".into())],
                                                    line: line_of(&d),
                                                    r#type: ImportType::Module,
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                            let start = line_of(&d);
                            if is_next_route && exported && is_next_http_method(&name) {
                                out.routes.push(Route {
                                    method: name.to_uppercase(),
                                    path: next_route_path(&file.path),
                                    handler: Some(name.clone()),
                                    line: start,
                                    framework: "next".into(),
                                });
                            }
                            out.symbols.push(Symbol {
                                name: name.clone(),
                                kind: SymbolKind::Const,
                                signature: const_signature(&d, src),
                                start_line: start,
                                end_line: end_line_of(&d),
                                exported,
                                docstring: doc.clone(),
                                parent: None,
                            });
                        }
                    }
                }
            "call_expression" => {
                let Some(function) = node.child_by_field_name("function") else {
                    push_children(&mut frames, &node, &ctx, src);
                    continue;
                };
                let line = line_of(&node);
                let mut inline_handler_node: Option<(String, Node)> = None;
                // Entrypoints: module-level calls to main/bootstrap/run in
                // main.ts / index.ts / cli.ts.
                if ctx.caller.is_none()
                    && ctx.class.is_none()
                    && is_entry_file
                    && function.kind() == "identifier"
                    && matches!(node_text(&function, src), "main" | "bootstrap" | "run")
                {
                    out.entrypoints.push(Entrypoint {
                        symbol: node_text(&function, src).to_string(),
                        kind: "module-entry".into(),
                        line,
                    });
                }
                // Express-style routes (inline handlers get a synthetic
                // symbol so their bodies contribute calls to the flow).
                if let Some((route, inline)) = express_route_full(&function, &node, src, line) {
                    out.routes.push(route);
                    inline_handler_node = inline;
                }
                if let Some((hname, hnode)) = &inline_handler_node {
                    out.symbols.push(Symbol {
                        name: hname.clone(),
                        kind: SymbolKind::Function,
                        signature: None,
                        start_line: line_of(hnode),
                        end_line: end_line_of(hnode),
                        exported: false,
                        docstring: None,
                        parent: None,
                    });
                    let mut hctx = ctx.clone();
                    hctx.caller = Some(hname.clone());
                    push_children_excluding(&mut frames, hnode, &hctx, src, &[]);
                }
                // Store accesses.
                if let Some(sr) = store_ref(&function, &node, src, &ctx) {
                    out.store_refs.push(sr);
                }
                // describe/it/test in test files.
                if is_test_file {
                    if let Some(t) = test_from_call(&function, &node, src, &ctx) {
                        out.tests.push(t);
                    }
                }
                out.calls.push(Call {
                    caller: ctx.caller.clone(),
                    callee: normalize_callee(node_text(&function, src)),
                    line,
                    known_receiver: known_receiver(&function),
                });
            }
            _ => {}
        }
        // Common child push. Inline route handlers were already pushed with
        // handler context inside the call_expression arm; exclude them here
        // to avoid double-processing.
        match &inline_handler_node {
            Some((_, hnode)) => {
                push_children_excluding(&mut frames, &node, &ctx, src, &[hnode.id()]);
            }
            None => push_children(&mut frames, &node, &ctx, src),
        }
        }

        // Integration tests: any import of supertest / @playwright/test /
        // cypress marks every test in the file as integration.
        let integration_file = out
            .imports
            .iter()
            .any(|i| is_integration_module(&i.module));
        if integration_file {
            for t in out.tests.iter_mut() {
                if t.kind == TestKind::Unit {
                    t.kind = TestKind::Integration;
                }
            }
        }

        out
    }
}

/// Traversal context inherited by a subtree.
#[derive(Clone, Default)]
struct Ctx {
    /// Enclosing callable symbol ("ClassName.method" for methods).
    caller: Option<String>,
    /// Enclosing class name (for method naming).
    class: Option<String>,
    /// Stack of enclosing `describe` titles (test suites).
    describes: Vec<String>,
}

/// Push one frame onto the stack with its inherited context.
fn push_frame<'a>(frames: &mut Vec<(Node<'a>, Ctx)>, c: Node<'a>, c2: &Ctx) {
    frames.push((c, c2.clone()));
}

/// Push a node's named children onto the frame stack (reverse order so pops
/// are in document order). Decorator subtrees are skipped: they are consumed
/// at declaration level for retry detection, and their inner calls are not
/// extracted. Caller/class/describe context is adjusted per child kind.
fn push_children<'a>(
    frames: &mut Vec<(Node<'a>, Ctx)>,
    node: &Node<'a>,
    ctx: &Ctx,
    src: &[u8],
) {
    push_children_excluding(frames, node, ctx, src, &[]);
}

/// `push_children` with a set of node ids to skip (used to avoid re-processing
/// an inline route handler whose body was already pushed with handler
/// context).
fn push_children_excluding<'a>(
    frames: &mut Vec<(Node<'a>, Ctx)>,
    node: &Node<'a>,
    ctx: &Ctx,
    src: &[u8],
    exclude: &[usize],
) {
    let mut children: Vec<Node> = Vec::new();
    {
        let mut cur = node.walk();
        for c in node.named_children(&mut cur) {
            if c.kind() == "decorator" {
                continue;
            }
            if exclude.contains(&c.id()) {
                continue;
            }
            children.push(c);
        }
    }
    match node.kind() {
        "function_declaration" | "generator_function_declaration" => {
            // Only module-level functions create caller context (nested
            // functions are not symbols).
            if ctx.caller.is_none() && ctx.class.is_none() {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = node_text(&name_node, src);
                    if !name.is_empty() {
                        let mut c2 = ctx.clone();
                        c2.caller = Some(name.to_string());
                        for c in children.iter().rev() {
                            push_frame(frames, *c, &c2);
                        }
                        return;
                    }
                }
            }
            for c in children.iter().rev() {
                push_frame(frames, *c, ctx);
            }
        }
        "class_declaration" | "abstract_class_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(&name_node, src);
                if !name.is_empty() {
                    let mut c2 = ctx.clone();
                    c2.class = Some(name.to_string());
                    c2.caller = None;
                    for c in children.iter().rev() {
                        push_frame(frames, *c, &c2);
                    }
                    return;
                }
            }
            for c in children.iter().rev() {
                push_frame(frames, *c, ctx);
            }
        }
        "method_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(&name_node, src);
                if !name.is_empty() {
                    let full = match &ctx.class {
                        Some(c) => format!("{c}.{name}"),
                        None => name.to_string(),
                    };
                    let mut c2 = ctx.clone();
                    c2.caller = Some(full);
                    c2.class = None;
                    for c in children.iter().rev() {
                        push_frame(frames, *c, &c2);
                    }
                    return;
                }
            }
            for c in children.iter().rev() {
                push_frame(frames, *c, ctx);
            }
        }
        "lexical_declaration" => {
            let module_level = ctx.caller.is_none() && ctx.class.is_none();
            for c in children.iter().rev() {
                let mut c2 = ctx.clone();
                if module_level && c.kind() == "variable_declarator" {
                    if let Some(name_node) = c.child_by_field_name("name") {
                        if name_node.kind() == "identifier" {
                            if let Some(v) = c.child_by_field_name("value") {
                                if matches!(
                                    v.kind(),
                                    "arrow_function" | "function_expression" | "generator_function"
                                ) {
                                    let name = node_text(&name_node, src);
                                    if !name.is_empty() {
                                        c2.caller = Some(name.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
                push_frame(frames, *c, &c2);
            }
        }
        "call_expression" => {
            // describe("title", cb): children see the suite on the stack.
            let is_describe = node
                .child_by_field_name("function")
                .map(|f| f.kind() == "identifier" && node_text(&f, src) == "describe")
                .unwrap_or(false);
            if is_describe {
                if let Some(title) = first_string_arg(node, src) {
                    let mut c2 = ctx.clone();
                    c2.describes.push(title);
                    for c in children.iter().rev() {
                        push_frame(frames, *c, &c2);
                    }
                    return;
                }
            }
            for c in children.iter().rev() {
                push_frame(frames, *c, ctx);
            }
        }
        _ => {
            for c in children.iter().rev() {
                push_frame(frames, *c, ctx);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text<'a>(node: &Node, src: &'a [u8]) -> &'a str {
    node.utf8_text(src).unwrap_or("")
}

fn line_of(node: &Node) -> u32 {
    node.start_position().row as u32 + 1
}

fn end_line_of(node: &Node) -> u32 {
    node.end_position().row as u32 + 1
}

/// Collapse whitespace runs to a single space (keeps signatures single-line).
fn collapse_ws(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut ws = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            ws = true;
        } else {
            if ws && !out.is_empty() {
                out.push(' ');
            }
            ws = false;
            out.push(ch);
        }
    }
    out
}

/// Normalize a callee expression to single-line dotted text: whitespace is
/// dropped around `.` (multi-line chains) and collapsed elsewhere.
fn normalize_callee(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut ws = false;
    let mut prev: Option<char> = None;
    for ch in text.chars() {
        if ch.is_whitespace() {
            ws = true;
            continue;
        }
        if ws {
            let next = ch;
            let drop = prev == Some('.') || next == '.';
            if !drop {
                out.push(' ');
            }
            ws = false;
        }
        out.push(ch);
        prev = Some(ch);
    }
    out
}

/// Truncate at a char boundary.
fn truncate(s: &mut String, max: usize) {
    if s.len() > max {
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
}

fn unquote(s: &str) -> String {
    let b = s.as_bytes();
    if b.len() >= 2
        && ((b[0] == b'"' && b[b.len() - 1] == b'"')
            || (b[0] == b'\'' && b[b.len() - 1] == b'\'')
            || (b[0] == b'`' && b[b.len() - 1] == b'`'))
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

fn is_exported(node: &Node) -> bool {
    node.parent()
        .map(|p| p.kind() == "export_statement")
        .unwrap_or(false)
}

fn is_test_file(path: &str) -> bool {
    path.ends_with(".test.ts")
        || path.ends_with(".spec.ts")
        || path.ends_with(".test.tsx")
        || path.ends_with(".spec.tsx")
        || path.ends_with(".test.js")
        || path.ends_with(".spec.js")
        || path.ends_with(".test.mjs")
        || path.ends_with(".spec.mjs")
        || path.ends_with(".test.cjs")
        || path.ends_with(".spec.cjs")
        || path.split('/').any(|seg| seg == "__tests__")
}

fn is_entry_file(path: &str) -> bool {
    let base = path.rsplit('/').next().unwrap_or(path);
    matches!(base, "main.ts" | "index.ts" | "cli.ts")
}

fn is_next_route_file(path: &str) -> bool {
    let base = path.rsplit('/').next().unwrap_or(path);
    if path.starts_with("app/") && (base == "route.ts" || base == "route.tsx") {
        return true;
    }
    if path.starts_with("pages/api/") && (path.ends_with(".ts") || path.ends_with(".tsx")) {
        return true;
    }
    false
}

fn is_next_http_method(name: &str) -> bool {
    matches!(
        name,
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS" | "HEAD"
    )
}

/// Route path for a Next.js route handler file:
/// `app/api/users/route.ts` -> `/api/users`, `app/api/users/[id]/route.ts` ->
/// `/api/users/:id`, `pages/api/health.ts` -> `/api/health`.
fn next_route_path(path: &str) -> String {
    let dir = if let Some(rest) = path.strip_prefix("pages/api/") {
        let rest = rest
            .strip_suffix(".tsx")
            .or_else(|| rest.strip_suffix(".ts"))
            .unwrap_or(rest);
        format!("api/{rest}")
    } else {
        let rest = &path["app/".len()..];
        let rest = rest
            .strip_suffix("route.tsx")
            .or_else(|| rest.strip_suffix("route.ts"))
            .unwrap_or(rest);
        rest.strip_suffix('/').unwrap_or(rest).to_string()
    };
    if dir.is_empty() {
        return "/".to_string();
    }
    let segs: Vec<String> = dir
        .split('/')
        .map(|seg| {
            match seg.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                Some(inner) => format!(":{}", inner.trim_start_matches('.')), // [id] -> :id, [...slug] -> :slug
                None => seg.to_string(),
            }
        })
        .collect();
    format!("/{}", segs.join("/"))
}

fn is_integration_module(module: &str) -> bool {
    module.contains("supertest")
        || module.contains("@playwright/test")
        || module.contains("cypress")
}

fn name_says_integration(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("e2e") || lower.contains("integration")
}

// ---------------------------------------------------------------------------
// Imports
// ---------------------------------------------------------------------------

/// `import ... from "m"` forms, including `import x = require("m")` and
/// side-effect imports.
fn import_from_statement(node: &Node, src: &[u8]) -> Option<Import> {
    let source = node.child_by_field_name("source")?;
    let module = unquote(node_text(&source, src));
    let line = line_of(node);
    // `import_clause` / `import_require_clause` are children, not fields.
    let mut clause: Option<Node> = None;
    let mut req: Option<Node> = None;
    {
        let mut cur = node.walk();
        for c in node.named_children(&mut cur) {
            match c.kind() {
                "import_clause" => clause = Some(c),
                "import_require_clause" => req = Some(c),
                _ => {}
            }
        }
    }
    if let Some(clause) = clause {
        let mut names: Vec<(String, String)> = Vec::new();
        let mut is_ns = false;
        let mut cur = clause.walk();
        for c in clause.named_children(&mut cur) {
            match c.kind() {
                "identifier" => {
                    // default import: `import x from "m"`
                    let local = node_text(&c, src).to_string();
                    if !local.is_empty() {
                        names.push((local, "default".into()));
                    }
                }
                "namespace_import" => {
                    // `import * as ns from "m"`
                    is_ns = true;
                    let mut cur2 = c.walk();
                    for id in c.named_children(&mut cur2) {
                        if id.kind() == "identifier" {
                            names.push((node_text(&id, src).to_string(), "*".into()));
                        }
                    }
                }
                "named_imports" => {
                    // `import { a, b as c } from "m"`
                    let mut cur2 = c.walk();
                    for spec in c.named_children(&mut cur2) {
                        if spec.kind() != "import_specifier" {
                            continue;
                        }
                        let Some(imported) = field_text(&spec, "name", src) else {
                            continue;
                        };
                        let local = field_text(&spec, "alias", src).unwrap_or(imported.clone());
                        names.push((local, imported));
                    }
                }
                _ => {}
            }
        }
        return Some(Import {
            module,
            names,
            line,
            r#type: if is_ns {
                ImportType::Module
            } else {
                ImportType::Member
            },
        });
    }
    if let Some(req) = req {
        // `import x = require("m")`
        let mut names = Vec::new();
        let mut cur = req.walk();
        for c in req.named_children(&mut cur) {
            if c.kind() == "identifier" {
                names.push((node_text(&c, src).to_string(), "default".into()));
            }
        }
        return Some(Import {
            module,
            names,
            line,
            r#type: ImportType::Module,
        });
    }
    // Side-effect import: `import "m"`
    Some(Import {
        module,
        names: Vec::new(),
        line,
        r#type: ImportType::Module,
    })
}

/// Re-export forms: `export { a } from "m"`, `export * from "m"`,
/// `export * as ns from "m"`.
fn import_from_export(node: &Node, src: &[u8]) -> Option<Import> {
    let source = node.child_by_field_name("source")?;
    let module = unquote(node_text(&source, src));
    let line = line_of(node);
    let mut names: Vec<(String, String)> = Vec::new();
    let mut has_clause = false;
    let mut is_ns = false;
    let mut cur = node.walk();
    for c in node.named_children(&mut cur) {
        match c.kind() {
            "export_clause" => {
                has_clause = true;
                let mut cur2 = c.walk();
                for spec in c.named_children(&mut cur2) {
                    if spec.kind() != "export_specifier" {
                        continue;
                    }
                    let Some(imported) = field_text(&spec, "name", src) else {
                        continue;
                    };
                    let local = field_text(&spec, "alias", src).unwrap_or(imported.clone());
                    names.push((local, imported));
                }
            }
            "namespace_export" => {
                is_ns = true;
                has_clause = true;
                let mut cur2 = c.walk();
                for id in c.named_children(&mut cur2) {
                    if id.kind() == "identifier" {
                        names.push((node_text(&id, src).to_string(), "*".into()));
                    }
                }
            }
            _ => {}
        }
    }
    if names.is_empty() && (is_ns || !has_clause) {
        // `export * from "m"` (the star is an anonymous token): binds
        // everything under no local name.
        return Some(Import {
            module,
            names: Vec::new(),
            line,
            r#type: ImportType::Module,
        });
    }
    if names.is_empty() {
        return None; // plain `export { a }` (no source) is not an import
    }
    Some(Import {
        module,
        names,
        line,
        r#type: if is_ns { ImportType::Module } else { ImportType::Member },
    })
}

fn field_text(node: &Node, field: &str, src: &[u8]) -> Option<String> {
    node.child_by_field_name(field)
        .map(|n| node_text(&n, src).to_string())
}

// ---------------------------------------------------------------------------
// Signatures & docstrings
// ---------------------------------------------------------------------------

fn signature_of(decl: &Node, src: &[u8]) -> Option<String> {
    let name = field_text(decl, "name", src).unwrap_or_default();
    let params = decl.child_by_field_name("parameters");
    let ptext: Vec<String> = match params {
        Some(p) => {
            let mut cur = p.walk();
            p.named_children(&mut cur)
                .map(|c| collapse_ws(node_text(&c, src)))
                .collect()
        }
        None => Vec::new(),
    };
    let mut sig = format!("{}({})", name, ptext.join(", "));
    if let Some(rt) = decl.child_by_field_name("return_type") {
        let t = node_text(&rt, src);
        let t = t.trim().trim_start_matches(':').trim();
        if !t.is_empty() {
            sig.push_str(": ");
            sig.push_str(&collapse_ws(t));
        }
    }
    sig = collapse_ws(&sig);
    truncate(&mut sig, 120);
    Some(sig)
}

/// Signature for a const with a function/arrow value.
fn const_signature(declarator: &Node, src: &[u8]) -> Option<String> {
    let name = field_text(declarator, "name", src).unwrap_or_default();
    let value = declarator.child_by_field_name("value")?;
    if !matches!(
        value.kind(),
        "arrow_function" | "function_expression" | "generator_function"
    ) {
        return None;
    }
    let params = value.child_by_field_name("parameters");
    let ptext: Vec<String> = match params {
        Some(p) => {
            let mut cur = p.walk();
            p.named_children(&mut cur)
                .map(|c| collapse_ws(node_text(&c, src)))
                .collect()
        }
        None => {
            // single-parameter arrow: `x => ...`
            match value.child_by_field_name("parameter") {
                Some(p) => vec![collapse_ws(node_text(&p, src))],
                None => Vec::new(),
            }
        }
    };
    let mut sig = format!("{}({})", name, ptext.join(", "));
    if let Some(rt) = value.child_by_field_name("return_type") {
        let t = node_text(&rt, src);
        let t = t.trim().trim_start_matches(':').trim();
        if !t.is_empty() {
            sig.push_str(": ");
            sig.push_str(&collapse_ws(t));
        }
    }
    sig = collapse_ws(&sig);
    truncate(&mut sig, 120);
    Some(sig)
}

/// Leading `/** ... */` comment immediately above the declaration. First
/// paragraph only, trimmed, truncated to 200 chars.
fn leading_jsdoc(node: &Node, src: &[u8]) -> Option<String> {
    // For exported declarations the comment precedes the export_statement.
    let container = match node.parent() {
        Some(p) if p.kind() == "export_statement" => p,
        _ => *node,
    };
    let parent = container.parent()?;
    let mut cur = parent.walk();
    let mut prev: Option<Node> = None;
    for c in parent.named_children(&mut cur) {
        if c.id() == container.id() {
            break;
        }
        prev = Some(c);
    }
    let comment = prev?;
    if comment.kind() != "comment" {
        return None;
    }
    let text = node_text(&comment, src);
    if !text.trim_start().starts_with("/**") {
        return None;
    }
    if comment.end_position().row + 1 != container.start_position().row {
        return None;
    }
    let body = text.trim_start_matches("/**").trim_end_matches("*/");
    let mut para: Vec<&str> = Vec::new();
    for raw in body.lines() {
        let line = raw.trim_start().trim_start_matches('*').trim();
        if line.is_empty() {
            if para.is_empty() {
                continue; // skip leading blank lines (`/**` newline)
            }
            break;
        }
        para.push(line);
    }
    let mut out = para.join(" ").trim().to_string();
    truncate(&mut out, 200);
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Decorator texts (with `@`) whose name contains "retry"/"backoff",
/// as (policy, line). Looks at the node's own decorator children and at
/// decorator siblings immediately preceding it (method decorators live in
/// the class body; `export @dec class` puts them on the export statement).
fn collect_retries(node: &Node, src: &[u8]) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    {
        let mut cur = node.walk();
        for c in node.named_children(&mut cur) {
            if c.kind() == "decorator" {
                push_retry(&c, src, &mut out);
            }
        }
    }
    if let Some(p) = node.parent() {
        let mut cur = p.walk();
        let mut pending: Vec<Node> = Vec::new();
        for c in p.named_children(&mut cur) {
            if c.id() == node.id() {
                break;
            }
            if c.kind() == "decorator" {
                pending.push(c);
            } else {
                pending.clear();
            }
        }
        for c in pending {
            push_retry(&c, src, &mut out);
        }
    }
    out
}

fn push_retry(dec: &Node, src: &[u8], out: &mut Vec<(String, u32)>) {
    let text = node_text(dec, src);
    let lower = text.to_ascii_lowercase();
    if lower.contains("retry") || lower.contains("backoff") {
        let mut policy = collapse_ws(text.trim());
        truncate(&mut policy, 200);
        out.push((policy, line_of(dec)));
    }
}

// ---------------------------------------------------------------------------
// Calls
// ---------------------------------------------------------------------------

/// Whether the callee root is an identifier (or member chain rooted in one).
fn known_receiver(function: &Node) -> bool {
    let mut n = *function;
    loop {
        match n.kind() {
            "identifier" | "property_identifier" | "type_identifier" | "statement_identifier"
            | "this" | "super" => return true,
            "member_expression" => match n.child_by_field_name("object") {
                Some(o) => n = o,
                None => return false,
            },
            _ => return false,
        }
    }
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

/// Like `express_route`, but when the handler is an inline function/arrow it
/// also returns a synthetic handler name plus the handler node, so the caller
/// can attribute the handler body's calls to a named symbol.
fn express_route_full<'a>(
    function: &Node<'a>,
    call: &Node<'a>,
    src: &[u8],
    line: u32,
) -> Option<(Route, Option<(String, Node<'a>)>)> {
    let (root, segments) = chain_parts(function, src)?;
    let ChainRoot::Ident(receiver) = root else {
        return None;
    };
    if !matches!(
        receiver.as_str(),
        "app" | "router" | "server" | "api" | "route" | "express" | "fastify"
    ) {
        return None;
    }
    let method = segments.first()?;
    if !matches!(
        method.as_str(),
        "get" | "post" | "put" | "delete" | "patch" | "all" | "use"
    ) {
        return None;
    }
    let path = first_string_arg(call, src)?;
    let mut handler = last_arg_handler(call, src);
    let mut inline: Option<(String, Node)> = None;
    if handler.is_none() {
        if let Some((hname, hnode)) = inline_handler(call, src) {
            handler = Some(hname.clone());
            inline = Some((hname, hnode));
        }
    }
    let framework = if receiver == "fastify" {
        "fastify"
    } else {
        "express"
    };
    Some((
        Route {
            method: method.to_uppercase(),
            path: path.clone(),
            handler,
            line,
            framework: framework.into(),
        },
        inline,
    ))
}

/// The last argument of the call when it is an inline arrow/function
/// expression: returns a synthetic handler name derived from the route.
fn inline_handler<'a>(call: &Node<'a>, src: &[u8]) -> Option<(String, Node<'a>)> {
    let args = call.child_by_field_name("arguments")?;
    let mut cur = args.walk();
    let children: Vec<Node> = args.named_children(&mut cur).collect();
    let last = *children.last()?;
    let kind = last.kind();
    if !matches!(kind, "arrow_function" | "function_expression") {
        return None;
    }
    let method = call
        .child_by_field_name("function")
        .and_then(|f| f.child_by_field_name("property"))
        .map(|p| node_text(&p, src).to_uppercase())
        .unwrap_or_else(|| "HANDLER".to_string());
    let path = first_string_arg(call, src).unwrap_or_default();
    let name = format!("{method} {path} handler");
    Some((name, last))
}

/// Handler = last argument when it is an identifier or a member expression
/// of identifiers (e.g. `controller.method`); inline functions yield None.
fn last_arg_handler(call: &Node, src: &[u8]) -> Option<String> {
    let args = call.child_by_field_name("arguments")?;
    let mut cur = args.walk();
    let children: Vec<Node> = args.named_children(&mut cur).collect();
    let last = children.last()?;
    match last.kind() {
        "identifier" => Some(node_text(last, src).to_string()),
        "member_expression" => {
            if last
                .child_by_field_name("object")
                .map(|o| matches!(o.kind(), "identifier" | "member_expression"))
                .unwrap_or(false)
                && last
                    .child_by_field_name("property")
                    .is_some()
            {
                Some(normalize_callee(node_text(last, src)))
            } else {
                None
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

fn test_from_call(function: &Node, call: &Node, src: &[u8], ctx: &Ctx) -> Option<Test> {
    if function.kind() != "identifier" {
        return None;
    }
    let callee = node_text(function, src);
    if !matches!(callee, "describe" | "it" | "test") {
        return None;
    }
    let title = first_string_arg(call, src)?;
    let symbol = match ctx.describes.last() {
        Some(suite) => format!("{suite} {title}"),
        None => title.clone(),
    };
    let kind = if name_says_integration(&title) || name_says_integration(&symbol) {
        TestKind::Integration
    } else {
        TestKind::Unit
    };
    Some(Test {
        name: title,
        symbol: Some(symbol),
        kind,
        line: line_of(call),
    })
}

// ---------------------------------------------------------------------------
// Store refs
// ---------------------------------------------------------------------------

enum ChainRoot {
    Ident(String),
    /// Root is itself a call, e.g. `knex("users")`: (client name, first
    /// string argument).
    Call { name: String, first_string_arg: Option<String> },
}

/// Decompose a call's function expression into (root, member segments).
/// Segments are collected outermost-first, so the called method is the
/// first segment and the member nearest the root is the last.
/// `prisma.user.findMany` -> (Ident("prisma"), ["findMany", "user"]);
/// `knex("users").where(x).update(y)` -> (Call{name:"knex", arg:"users"},
/// ["update", "where"]). Computed members or other base expressions yield
/// None.
fn chain_parts(function: &Node, src: &[u8]) -> Option<(ChainRoot, Vec<String>)> {
    let mut segments: Vec<String> = Vec::new();
    let mut n = *function;
    loop {
        match n.kind() {
            "identifier" | "property_identifier" | "type_identifier" | "statement_identifier"
            | "this" | "super" => {
                return Some((ChainRoot::Ident(node_text(&n, src).to_string()), segments));
            }
            "member_expression" => {
                let prop = n.child_by_field_name("property")?;
                segments.push(node_text(&prop, src).to_string());
                n = n.child_by_field_name("object")?;
            }
            "call_expression" => {
                // Base call like knex("users"): keep walking through member
                // chains on its function (e.g. knex("users").where(...)).
                let f = n.child_by_field_name("function")?;
                match f.kind() {
                    "identifier" | "property_identifier" => {
                        let name = node_text(&f, src).to_string();
                        let arg = first_string_arg(&n, src);
                        return Some((ChainRoot::Call { name, first_string_arg: arg }, segments));
                    }
                    "member_expression" => {
                        let prop = f.child_by_field_name("property")?;
                        segments.push(node_text(&prop, src).to_string());
                        n = f.child_by_field_name("object")?;
                    }
                    _ => return None,
                }
            }
            _ => return None,
        }
    }
}

fn store_ref(function: &Node, call: &Node, src: &[u8], ctx: &Ctx) -> Option<StoreRef> {
    let (root, mut segments) = chain_parts(function, src)?;
    // `this.db.query(...)`: unwrap the instance prefix; the store is the
    // segment nearest the root and the method stays outermost.
    if let ChainRoot::Ident(r) = &root {
        if matches!(r.as_str(), "this" | "super") {
            let store = segments.pop()?;
            return store_ref_impl(&store, &segments, call, src, ctx);
        }
    }
    match &root {
        ChainRoot::Call { name, first_string_arg } => {
            // `knex("users").where(...).update(...)`
            if name != "knex" {
                return None;
            }
            let mut target_segments = segments.clone();
            if target_segments.is_empty() {
                return None;
            }
            let method = target_segments.remove(0);
            let op = orm_op(&method)?;
            Some(StoreRef {
                caller: ctx.caller.clone(),
                store: name.clone(),
                technology: Some("sql".into()),
                op,
                target: first_string_arg.clone(),
                line: line_of(call),
            })
        }
        ChainRoot::Ident(_) => store_ref_impl(&root_text(&root), &segments, call, src, ctx),
    }
}

fn root_text(root: &ChainRoot) -> String {
    match root {
        ChainRoot::Ident(s) => s.clone(),
        ChainRoot::Call { name, .. } => name.clone(),
    }
}

fn store_ref_impl(
    r: &str,
    segments: &[String],
    call: &Node,
    src: &[u8],
    ctx: &Ctx,
) -> Option<StoreRef> {
    let method = segments.first()?; // outermost segment is the called method
    let caller = ctx.caller.clone();
    let line = line_of(call);
    match r {
            "prisma" | "sequelize" | "typeorm" | "mongoose" | "knex" => {
                if segments.len() < 2 {
                    return None;
                }
                let op = orm_op(method)?;
                let target = segments.last().cloned(); // model, nearest the root
                let technology = if r == "mongoose" { "mongodb" } else { "sql" };
                Some(StoreRef {
                    caller,
                    store: r.to_string(),
                    technology: Some(technology.into()),
                    op,
                    target,
                    line,
                })
            }
            "db" | "database" | "pool" | "client" | "sql" => {
                let (op, target) = sql_op(method, call, src)?;
                Some(StoreRef {
                    caller,
                    store: r.to_string(),
                    technology: Some("sql".into()),
                    op,
                    target,
                    line,
                })
            }
            "redis" => {
                let op = redis_op(method)?;
                let target = if matches!(method.as_str(), "publish" | "subscribe") {
                    topic_arg(call, src)
                } else {
                    None
                };
                Some(StoreRef {
                    caller,
                    store: r.to_string(),
                    technology: Some("redis".into()),
                    op,
                    target,
                    line,
                })
            }
            "kafka" | "producer" | "consumer" => {
                let op = kafka_op(method)?;
                Some(StoreRef {
                    caller,
                    store: r.to_string(),
                    technology: Some("kafka".into()),
                    op,
                    target: topic_arg(call, src),
                    line,
                })
            }
            "s3" | "bucket" => {
                let op = s3_op(method)?;
                Some(StoreRef {
                    caller,
                    store: r.to_string(),
                    technology: Some("s3".into()),
                    op,
                    target: first_string_arg(call, src),
                    line,
                })
            }
            _ => None,
    }
}

fn orm_op(method: &str) -> Option<StoreOp> {
    match method {
        "create" | "createMany" | "update" | "updateMany" | "delete" | "deleteMany" | "upsert"
        | "insert" | "save" | "remove" => Some(StoreOp::Write),
        "findOne" | "get" => Some(StoreOp::Read),
        "findAll" | "findMany" | "findFirst" | "findUnique" | "findFirstOrThrow"
        | "findUniqueOrThrow" | "count" | "aggregate" | "select" => Some(StoreOp::Query),
        _ if method.starts_with("find") => Some(StoreOp::Query),
        _ => None,
    }
}

fn redis_op(method: &str) -> Option<StoreOp> {
    match method {
        "get" => Some(StoreOp::Read),
        "set" | "del" | "incr" | "decr" | "hset" | "hgetall" | "sadd" | "smembers" => {
            Some(StoreOp::Write)
        }
        "publish" => Some(StoreOp::Publish),
        "subscribe" => Some(StoreOp::Subscribe),
        _ => None,
    }
}

fn kafka_op(method: &str) -> Option<StoreOp> {
    match method {
        "send" | "produce" => Some(StoreOp::Publish),
        "subscribe" | "consume" => Some(StoreOp::Subscribe),
        _ => None,
    }
}

fn s3_op(method: &str) -> Option<StoreOp> {
    match method {
        "getObject" | "headObject" => Some(StoreOp::Read),
        "putObject" | "deleteObject" | "upload" => Some(StoreOp::Write),
        _ => None,
    }
}

/// (op, target) for raw SQL clients: `db.query("SELECT ...")` sniffs the SQL;
/// `db.insert(...)` etc. are direct writes.
fn sql_op(method: &str, call: &Node, src: &[u8]) -> Option<(StoreOp, Option<String>)> {
    match method {
        "query" | "execute" => {
            let sql = sql_text_arg(call, src)?;
            Some((sniff_sql_op(&sql), sniff_sql_target(&sql)))
        }
        "insert" | "update" | "delete" => Some((StoreOp::Write, None)),
        _ => None,
    }
}

/// First string literal argument of a call.
fn first_string_arg(call: &Node, src: &[u8]) -> Option<String> {
    let args = call.child_by_field_name("arguments")?;
    let mut cur = args.walk();
    for a in args.named_children(&mut cur) {
        if a.kind() == "string" {
            return Some(unquote(node_text(&a, src)));
        }
    }
    None
}

/// Channel/topic for publish/subscribe: a string argument or an object
/// literal with a `topic:`/`channel:` string property.
fn topic_arg(call: &Node, src: &[u8]) -> Option<String> {
    if let Some(s) = first_string_arg(call, src) {
        return Some(s);
    }
    let args = call.child_by_field_name("arguments")?;
    let mut cur = args.walk();
    for a in args.named_children(&mut cur) {
        if a.kind() != "object" {
            continue;
        }
        let mut cur2 = a.walk();
        for p in a.named_children(&mut cur2) {
            if p.kind() != "pair" {
                continue;
            }
            if let (Some(k), Some(v)) = (
                p.child_by_field_name("key"),
                p.child_by_field_name("value"),
            ) {
                if v.kind() == "string" && matches!(node_text(&k, src), "topic" | "channel") {
                    return Some(unquote(node_text(&v, src)));
                }
            }
        }
    }
    None
}

/// SQL text argument: a string, a template string, or a pg-style
/// `{ text: "..." }` object.
fn sql_text_arg(call: &Node, src: &[u8]) -> Option<String> {
    let args = call.child_by_field_name("arguments")?;
    let mut cur = args.walk();
    for a in args.named_children(&mut cur) {
        match a.kind() {
            "string" => return Some(unquote(node_text(&a, src))),
            "template_string" => {
                let t = node_text(&a, src);
                return Some(t.trim_start_matches('`').trim_end_matches('`').to_string());
            }
            "object" => {
                let mut cur2 = a.walk();
                for p in a.named_children(&mut cur2) {
                    if p.kind() == "pair" {
                        if let (Some(k), Some(v)) = (
                            p.child_by_field_name("key"),
                            p.child_by_field_name("value"),
                        ) {
                            if node_text(&k, src) == "text" && v.kind() == "string" {
                                return Some(unquote(node_text(&v, src)));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// Word-boundary keyword presence (case-insensitive).
fn has_sql_keyword(sql: &str, keyword: &str) -> bool {
    let upper = sql.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    let kw = keyword.as_bytes();
    let mut i = 0;
    while i + kw.len() <= bytes.len() {
        if &bytes[i..i + kw.len()] == kw {
            let before_ok = i == 0 || !is_ident_byte(bytes[i - 1]);
            let after = i + kw.len();
            let after_ok = after == bytes.len() || !is_ident_byte(bytes[after]);
            if before_ok && after_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn sniff_sql_op(sql: &str) -> StoreOp {
    let upper = sql.to_ascii_uppercase();
    for kw in ["INSERT", "UPDATE", "DELETE", "REPLACE", "MERGE"] {
        if has_sql_keyword(&upper, kw) {
            return StoreOp::Write;
        }
    }
    for kw in ["SELECT", "SHOW", "DESCRIBE"] {
        if has_sql_keyword(&upper, kw) {
            return StoreOp::Query;
        }
    }
    for kw in ["CREATE", "ALTER", "DROP"] {
        if has_sql_keyword(&upper, kw) {
            return StoreOp::Migrate;
        }
    }
    StoreOp::Query
}

/// First identifier after the earliest table keyword
/// (INSERT INTO / REPLACE INTO / MERGE INTO / DELETE FROM / UPDATE / FROM /
/// INTO).
fn sniff_sql_target(sql: &str) -> Option<String> {
    const TABLE_KWS: [&str; 9] = [
        "CREATE TABLE",
        "ALTER TABLE",
        "INSERT INTO",
        "REPLACE INTO",
        "MERGE INTO",
        "DELETE FROM",
        "UPDATE",
        "FROM",
        "INTO",
    ];
    let upper = sql.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    let mut best: Option<(usize, usize)> = None; // (pos, kw_len)
    for kw in TABLE_KWS {
        let kb = kw.as_bytes();
        let mut i = 0;
        while i + kb.len() <= bytes.len() {
            if &bytes[i..i + kb.len()] == kb {
                let before_ok = i == 0 || !is_ident_byte(bytes[i - 1]);
                let after = i + kb.len();
                let after_ok = after == bytes.len() || !is_ident_byte(bytes[after]);
                if before_ok && after_ok {
                    match best {
                        Some((bp, _)) if bp <= i => {}
                        _ => best = Some((i, kb.len())),
                    }
                    break;
                }
            }
            i += 1;
        }
    }
    let (pos, klen) = best?;
    let bytes = sql.as_bytes();
    let mut i = pos + klen;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    // skip opening quotes/brackets
    while i < bytes.len() && matches!(bytes[i], b'"' | b'\'' | b'`' | b'[') {
        i += 1;
    }
    let start = i;
    while i < bytes.len() && is_ident_byte(bytes[i]) {
        i += 1;
    }
    if i == start {
        None
    } else {
        Some(sql[start..i].to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract(path: &str, content: &str) -> ExtractedFile {
        TypeScriptExtractor::default().extract(&SourceFile::new(path, content))
    }

    fn find<'a>(syms: &'a [Symbol], name: &str) -> &'a Symbol {
        syms.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("symbol {name} not found"))
    }

    fn find_call<'a>(calls: &'a [Call], callee: &str) -> Vec<&'a Call> {
        calls.iter().filter(|c| c.callee == callee).collect()
    }

    #[test]
    fn symbols_and_methods() {
        let ef = extract(
            "src/lib.ts",
            r#"export function add(a: number, b: number): number { return a + b; }
function helper() {}
export class User {
  name: string;
  constructor(name: string) { this.name = name; }
  greet(prefix: string): string { return prefix + this.name; }
  static create(n: string): User { return new User(n); }
}
export interface Named { name: string; }
type Alias = string | number;
export enum Color { Red, Green }
export const LIMIT = 10;
const format = (s: string) => s.trim();
"#,
        );
        assert_eq!(ef.symbols.len(), 11);
        let add = find(&ef.symbols, "add");
        assert_eq!(add.kind, SymbolKind::Function);
        assert_eq!(add.signature.as_deref(), Some("add(a: number, b: number): number"));
        assert!(add.exported);
        assert_eq!(add.start_line, 1);
        assert_eq!(add.end_line, 1);
        let helper = find(&ef.symbols, "helper");
        assert!(!helper.exported);
        let user = find(&ef.symbols, "User");
        assert_eq!(user.kind, SymbolKind::Class);
        assert!(user.exported);
        let ctor = find(&ef.symbols, "User.constructor");
        assert_eq!(ctor.kind, SymbolKind::Method);
        assert_eq!(ctor.parent.as_deref(), Some("User"));
        assert!(!ctor.exported);
        assert_eq!(ctor.signature.as_deref(), Some("constructor(name: string)"));
        let greet = find(&ef.symbols, "User.greet");
        assert_eq!(greet.parent.as_deref(), Some("User"));
        assert_eq!(greet.signature.as_deref(), Some("greet(prefix: string): string"));
        let create = find(&ef.symbols, "User.create");
        assert_eq!(create.parent.as_deref(), Some("User"));
        assert_eq!(find(&ef.symbols, "Named").kind, SymbolKind::Interface);
        assert!(find(&ef.symbols, "Named").exported);
        assert_eq!(find(&ef.symbols, "Alias").kind, SymbolKind::Type);
        assert!(!find(&ef.symbols, "Alias").exported);
        assert_eq!(find(&ef.symbols, "Color").kind, SymbolKind::Enum);
        let limit = find(&ef.symbols, "LIMIT");
        assert_eq!(limit.kind, SymbolKind::Const);
        assert!(limit.exported);
        assert_eq!(limit.signature, None);
        let format = find(&ef.symbols, "format");
        assert_eq!(format.kind, SymbolKind::Const);
        assert!(!format.exported);
        assert_eq!(format.signature.as_deref(), Some("format(s: string)"));
    }

    #[test]
    fn imports_all_forms() {
        let ef = extract(
            "src/app.ts",
            r#"import fs from "fs";
import { readFile, writeFile as wf } from "fs/promises";
import * as path from "path";
import "reflect-metadata";
import express, { Router } from "express";
import { resolve } from "./util";
const os = require("os");
export { run } from "./runner";
export * from "./types";
import type { Config } from "./config";
"#,
        );
        assert_eq!(ef.imports.len(), 10);
        let by_module: Vec<&Import> = ef.imports.iter().collect();
        let m = |s: &str| by_module.iter().find(|i| i.module == s).unwrap();
        assert_eq!(m("fs").names, vec![("fs".into(), "default".into())]);
        assert_eq!(m("fs").r#type, ImportType::Member);
        assert_eq!(
            m("fs/promises").names,
            vec![("readFile".into(), "readFile".into()), ("wf".into(), "writeFile".into())]
        );
        assert_eq!(m("path").names, vec![("path".into(), "*".into())]);
        assert_eq!(m("path").r#type, ImportType::Module);
        assert_eq!(m("reflect-metadata").names, Vec::<(String, String)>::new());
        assert_eq!(m("reflect-metadata").r#type, ImportType::Module);
        assert_eq!(
            m("express").names,
            vec![("express".into(), "default".into()), ("Router".into(), "Router".into())]
        );
        assert_eq!(m("./util").names, vec![("resolve".into(), "resolve".into())]);
        assert_eq!(m("os").names, vec![("os".into(), "default".into())]);
        assert_eq!(m("os").r#type, ImportType::Module);
        assert_eq!(m("./runner").names, vec![("run".into(), "run".into())]);
        assert_eq!(m("./types").names, Vec::<(String, String)>::new());
        assert_eq!(m("./types").r#type, ImportType::Module);
        assert_eq!(m("./config").names, vec![("Config".into(), "Config".into())]);
        assert_eq!(ef.imports[0].line, 1);
    }

    #[test]
    fn calls_and_callers() {
        let ef = extract(
            "src/app.ts",
            r#"import { helper } from "./util";
const client = getClient();
function top() { return helper(1) + client.get(); }
export class Service {
  run() { return this.helper("x") + helper(2); }
  helper(v: string) { return v; }
}
top();
client.query("SELECT * FROM users");
const up = "abc".toUpperCase();
"#,
        );
        assert_eq!(ef.calls.len(), 8);
        let c = |callee: &str| {
            let v: Vec<&Call> = find_call(&ef.calls, callee);
            assert!(!v.is_empty(), "no call {callee}");
            v
        };
        let h1 = c("helper");
        assert!(h1.iter().any(|x| x.caller.as_deref() == Some("top")));
        assert!(h1.iter().any(|x| x.caller.as_deref() == Some("Service.run")));
        let cg = c("client.get");
        assert_eq!(cg[0].caller.as_deref(), Some("top"));
        assert!(cg[0].known_receiver);
        let th = c("this.helper");
        assert_eq!(th[0].caller.as_deref(), Some("Service.run"));
        assert!(th[0].known_receiver);
        let top = c("top");
        assert_eq!(top[0].caller, None);
        let cq = c("client.query");
        assert_eq!(cq[0].caller, None);
        assert!(cq[0].known_receiver);
        let up = c("\"abc\".toUpperCase");
        assert!(!up[0].known_receiver);
        assert_eq!(up[0].caller, None);
        let gc = c("getClient");
        assert_eq!(gc[0].caller, None);
    }

    #[test]
    fn express_routes() {
        let ef = extract(
            "src/server.ts",
            r#"import express from "express";
const app = express();
app.get("/health", health);
app.post("/users", auth, createUser);
router.put("/items/:id", updateItem);
server.use("/static", staticHandler);
fastify.get("/ping", (req, res) => res.send("pong"));
app.get();
app.use(middleware);
express();
"#,
        );
        assert_eq!(ef.routes.len(), 5);
        let r = |m: &str, p: &str| {
            ef.routes
                .iter()
                .find(|r| r.method == m && r.path == p)
                .unwrap_or_else(|| panic!("route {m} {p}"))
        };
        assert_eq!(r("GET", "/health").handler.as_deref(), Some("health"));
        assert_eq!(r("GET", "/health").framework, "express");
        assert_eq!(r("POST", "/users").handler.as_deref(), Some("createUser"));
        assert_eq!(r("PUT", "/items/:id").handler.as_deref(), Some("updateItem"));
        assert_eq!(r("USE", "/static").handler.as_deref(), Some("staticHandler"));
        let ping = r("GET", "/ping");
        assert_eq!(ping.handler.as_deref(), Some("GET /ping handler"));
        assert_eq!(ping.framework, "fastify");
    }

    #[test]
    fn next_route_handlers() {
        let ef = extract(
            "app/api/users/route.ts",
            r#"export async function GET(req: Request) { return Response.json({}); }
export function POST() {}
function notExported() {}
export const DELETE = async () => {};
"#,
        );
        assert_eq!(ef.routes.len(), 3);
        let r = |m: &str| ef.routes.iter().find(|r| r.method == m).unwrap();
        assert_eq!(r("GET").path, "/api/users");
        assert_eq!(r("GET").handler.as_deref(), Some("GET"));
        assert_eq!(r("GET").framework, "next");
        assert_eq!(r("POST").path, "/api/users");
        assert_eq!(r("DELETE").path, "/api/users");

        let ef2 = extract(
            "app/api/users/[id]/route.ts",
            "export function GET() {}\nexport function POST() {}\n",
        );
        assert_eq!(ef2.routes.len(), 2);
        assert!(ef2.routes.iter().all(|r| r.path == "/api/users/:id"));

        let ef3 = extract("pages/api/health.ts", "export function POST() {}\n");
        assert_eq!(ef3.routes.len(), 1);
        assert_eq!(ef3.routes[0].path, "/api/health");
        assert_eq!(ef3.routes[0].method, "POST");

        // not a route file
        let ef4 = extract("app/api/users/helper.ts", "export function GET() {}\n");
        assert!(ef4.routes.is_empty());
    }

    #[test]
    fn tests_describe_it() {
        let ef = extract(
            "src/auth.test.ts",
            r#"import { describe, it, test } from "vitest";
describe("AuthService.login", () => {
  it("rejects bad token", () => { expect(1).toBe(1); });
  describe("inner", () => {
    it("deep case", () => {});
  });
});
it("top level", () => {});
test("plain test", () => {});
export function testHelper() { return 1; }
"#,
        );
        assert_eq!(ef.tests.len(), 7);
        let t = |name: &str| {
            ef.tests
                .iter()
                .find(|t| t.name == name)
                .unwrap_or_else(|| panic!("test {name}"))
        };
        assert_eq!(t("AuthService.login").symbol.as_deref(), Some("AuthService.login"));
        assert_eq!(t("AuthService.login").kind, TestKind::Unit);
        assert_eq!(
            t("rejects bad token").symbol.as_deref(),
            Some("AuthService.login rejects bad token")
        );
        assert_eq!(t("deep case").symbol.as_deref(), Some("inner deep case"));
        assert_eq!(t("top level").symbol.as_deref(), Some("top level"));
        assert_eq!(t("plain test").symbol.as_deref(), Some("plain test"));
        let th = t("testHelper");
        assert_eq!(th.symbol.as_deref(), Some("testHelper"));
        assert_eq!(th.kind, TestKind::Unit);
        assert!(ef.symbols.iter().any(|s| s.name == "testHelper" && s.kind == SymbolKind::Function));
    }

    #[test]
    fn tests_integration() {
        let ef = extract(
            "src/api.spec.ts",
            r#"import request from "supertest";
import app from "./app";
describe("api", () => {
  it("returns 200", async () => { await request(app).get("/"); });
});
"#,
        );
        assert_eq!(ef.tests.len(), 2);
        assert!(ef.tests.iter().all(|t| t.kind == TestKind::Integration));

        // name-based: "integration" in title, no special imports
        let ef2 = extract(
            "src/sync.test.ts",
            r#"it("integration syncs data", () => {});
it("plain", () => {});
"#,
        );
        assert!(ef2.tests.iter().any(|t| t.name == "integration syncs data" && t.kind == TestKind::Integration));
        assert!(ef2.tests.iter().any(|t| t.name == "plain" && t.kind == TestKind::Unit));

        // not a test file
        let ef3 = extract("src/util.ts", "it('x', () => {});\n");
        assert!(ef3.tests.is_empty());
    }

    #[test]
    fn store_refs() {
        let ef = extract(
            "src/data.ts",
            r#"const prisma = new PrismaClient();
async function q() {
  const users = await prisma.user.findMany({});
  const u = await prisma.user.findUnique({ where: { id: 1 } });
  await prisma.user.create({ data: {} });
  await prisma.post.deleteMany({});
}
const knex = require("knex");
async function k() {
  await knex("users").where({ id: 1 }).update({ name: "x" });
  await knex("users").select("*");
}
async function r() {
  const v = await redis.get("k1");
  await redis.set("k2", v);
  await redis.publish("chan", "msg");
  await redis.subscribe("chan", cb);
}
async function kk() {
  await kafka.send({ topic: "events", messages: [] });
  await producer.produce("orders", {});
  await consumer.consume("orders");
}
async function s() {
  await db.query("SELECT * FROM users WHERE id = 1");
  await db.execute("INSERT INTO logs (msg) VALUES ('hi')");
  await pool.query("CREATE TABLE t (id int)");
  await db.query({ text: "UPDATE accounts SET bal = 0" });
  await db.delete("users");
}
async function s3f() {
  await s3.getObject({ Key: "a/b" });
  await bucket.putObject({ Key: "c" });
}
async function neg() {
  await client.get("https://example.com");
  await prisma.user;
}
"#,
        );
        assert_eq!(ef.store_refs.len(), 20);
        let sr = |store: &str, op: StoreOp, target: &str, caller: &str| {
            ef.store_refs
                .iter()
                .any(|s| {
                    s.store == store
                        && s.op == op
                        && s.target.as_deref() == Some(target)
                        && s.caller.as_deref() == Some(caller)
                })
        };
        let sr_none = |store: &str, op: StoreOp, caller: &str| {
            ef.store_refs
                .iter()
                .any(|s| s.store == store && s.op == op && s.target.is_none() && s.caller.as_deref() == Some(caller))
        };
        assert!(sr("prisma", StoreOp::Query, "user", "q"));
        assert!(sr("prisma", StoreOp::Query, "user", "q")); // findUnique
        assert!(sr("prisma", StoreOp::Write, "user", "q"));
        assert!(sr("prisma", StoreOp::Write, "post", "q"));
        assert!(sr("knex", StoreOp::Write, "users", "k"));
        assert!(sr("knex", StoreOp::Query, "users", "k"));
        assert!(sr_none("redis", StoreOp::Read, "r"));
        assert!(sr_none("redis", StoreOp::Write, "r"));
        assert!(sr("redis", StoreOp::Publish, "chan", "r"));
        assert!(sr("redis", StoreOp::Subscribe, "chan", "r"));
        assert!(sr("kafka", StoreOp::Publish, "events", "kk"));
        assert!(sr("producer", StoreOp::Publish, "orders", "kk"));
        assert!(sr("consumer", StoreOp::Subscribe, "orders", "kk"));
        assert!(sr("db", StoreOp::Query, "users", "s"));
        assert!(sr("db", StoreOp::Write, "logs", "s"));
        assert!(sr("pool", StoreOp::Migrate, "t", "s"));
        assert!(sr("db", StoreOp::Write, "accounts", "s"));
        assert!(sr_none("db", StoreOp::Write, "s"));
        assert!(sr_none("s3", StoreOp::Read, "s3f"));
        assert!(sr_none("bucket", StoreOp::Write, "s3f"));
        // generic http client.get is not a store op
        assert!(!ef.store_refs.iter().any(|s| s.caller.as_deref() == Some("neg")));
        let knex_import = ef.imports.iter().find(|i| i.module == "knex").unwrap();
        assert_eq!(knex_import.names, vec![("knex".into(), "default".into())]);
    }

    #[test]
    fn retry_decorators() {
        let ef = extract(
            "src/api.ts",
            r#"class Api {
  @retry({ retries: 3 })
  async fetch() {}
  @backoff.on_exception()
  retryMe() {}
}
@tenacity.retry
export class Worker {}
function normal() {}
"#,
        );
        assert_eq!(ef.retries.len(), 3);
        let ret = |sym: &str| ef.retries.iter().find(|r| r.symbol == sym).unwrap();
        assert!(ret("Api.fetch").policy.contains("@retry"));
        assert_eq!(ret("Api.fetch").line, 2);
        assert!(ret("Api.retryMe").policy.contains("@backoff.on_exception"));
        assert!(ret("Worker").policy.contains("@tenacity.retry"));
        // decorator calls are not extracted as calls
        assert!(ef.calls.iter().all(|c| !c.callee.contains("retry") && !c.callee.contains("backoff")));
    }

    #[test]
    fn docstrings() {
        let ef = extract(
            "src/lib.ts",
            r#"/**
 * Adds two numbers.
 *
 * Detailed second paragraph.
 */
export function add(a: number, b: number): number { return a + b; }

function noDoc() {}
"#,
        );
        assert_eq!(find(&ef.symbols, "add").docstring.as_deref(), Some("Adds two numbers."));
        assert_eq!(find(&ef.symbols, "noDoc").docstring, None);

        // JSDoc on methods
        let ef2 = extract(
            "src/lib2.ts",
            "class A {\n  /** Greets the user. */\n  greet() { return 'hi'; }\n}\n",
        );
        assert_eq!(find(&ef2.symbols, "A.greet").docstring.as_deref(), Some("Greets the user."));
    }

    #[test]
    fn entrypoints() {
        let ef = extract(
            "src/main.ts",
            r#"import { bootstrap } from "./boot";
function main() {
  bootstrap();
}
main();
bootstrap();
run();
"#,
        );
        assert_eq!(ef.entrypoints.len(), 3);
        assert_eq!(ef.entrypoints[0].symbol, "main");
        assert_eq!(ef.entrypoints[0].kind, "module-entry");
        assert_eq!(ef.entrypoints[0].line, 5);
        assert_eq!(ef.entrypoints[1].symbol, "bootstrap");
        assert_eq!(ef.entrypoints[2].symbol, "run");

        let ef2 = extract("src/util.ts", "main();\n");
        assert!(ef2.entrypoints.is_empty());
    }

    #[test]
    fn tsx_file() {
        let ef = extract(
            "app/component.tsx",
            r#"import React from "react";
export function Badge(props: { label: string }) {
  const items = [1, 2, 3];
  return <div className="badge">{props.label}{items.map((n) => <span key={n}>{n}</span>)}</div>;
}
"#,
        );
        let badge = find(&ef.symbols, "Badge");
        assert_eq!(badge.kind, SymbolKind::Function);
        assert!(badge.exported);
        assert_eq!(badge.end_line, 5);
        let m = find_call(&ef.calls, "items.map");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].caller.as_deref(), Some("Badge"));
        assert!(ef.imports.iter().any(|i| i.module == "react"));
    }

    #[test]
    fn malformed_input_no_panic() {
        // truncated snippets
        let cases = [
            "function foo(",
            "import { a from 'x'",
            "class A { method( ) }",
            "export default ",
            "const x = require(",
            "describe(",
            "@retry(",
            "}}}",
            "",
            "app.get(\"/p\"",
            "prisma.user.findMany(",
            "((((((((((((((((((((((((",
            "\u{0}\u{1}\u{2}def \u{1F600}\u{FFFD}\u{10FFFF}",
            "const s = \"unterminated",
            "/* unterminated comment",
        ];
        for c in cases {
            let _ = extract("src/x.ts", c);
        }
        // deep nesting: iterative traversal must not blow the stack
        let deep = format!("{}1{}", "(".repeat(100_000), ")".repeat(100_000));
        let _ = extract("src/deep.ts", &deep);
        let _ = extract("src/deep.tsx", &deep);
        // binary-ish garbage
        let garbage: String = (0u8..255).map(char::from).collect();
        let _ = extract("src/garbage.ts", &garbage);
        let _ = extract("src/garbage.ts", "import x from './y'\u{0}\u{1}function z( { ");
    }

    #[test]
    fn deterministic_output() {
        let src = r#"import express from "express";
/** Docs. */
export function add(a: number, b: number) { return a + b; }
class Svc {
  run() { return this.helper(); }
  helper() { return db.query("SELECT * FROM users"); }
}
describe("suite", () => { it("works", () => {}); });
"#;
        let a = extract("src/d.ts", src);
        let b = extract("src/d.ts", src);
        let va = serde_json::to_value(&a).unwrap();
        let vb = serde_json::to_value(&b).unwrap();
        assert_eq!(va, vb);
    }

    #[test]
    fn multiline_callee_normalized() {
        let ef = extract(
            "src/x.ts",
            "function f() {\n  return prisma\n    .user\n    .findMany({});\n}\n",
        );
        let c = find_call(&ef.calls, "prisma.user.findMany");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].caller.as_deref(), Some("f"));
        assert!(c[0].known_receiver);
    }

    #[test]
    fn sniffer_units() {
        assert_eq!(sniff_sql_op("SELECT * FROM users"), StoreOp::Query);
        assert_eq!(sniff_sql_op("  select 1"), StoreOp::Query);
        assert_eq!(sniff_sql_op("INSERT INTO logs VALUES (1)"), StoreOp::Write);
        assert_eq!(sniff_sql_op("UPDATE users SET x = 1"), StoreOp::Write);
        assert_eq!(sniff_sql_op("DELETE FROM t"), StoreOp::Write);
        assert_eq!(sniff_sql_op("CREATE TABLE t (id int)"), StoreOp::Migrate);
        assert_eq!(sniff_sql_op("DROP TABLE t"), StoreOp::Migrate);
        assert_eq!(sniff_sql_op("ALTER TABLE t ADD c int"), StoreOp::Migrate);
        assert_eq!(sniff_sql_op("SHOW TABLES"), StoreOp::Query);
        assert_eq!(sniff_sql_op("PRAGMA table_info(t)"), StoreOp::Query);
        assert_eq!(sniff_sql_op("INSERTED nowhere"), StoreOp::Query); // word boundary
        assert_eq!(sniff_sql_target("SELECT * FROM users WHERE id = 1").as_deref(), Some("users"));
        assert_eq!(sniff_sql_target("INSERT INTO logs VALUES (1)").as_deref(), Some("logs"));
        assert_eq!(sniff_sql_target("UPDATE accounts SET bal = 0").as_deref(), Some("accounts"));
        assert_eq!(sniff_sql_target("DELETE FROM t").as_deref(), Some("t"));
        assert_eq!(sniff_sql_target("SELECT * FROM \"weird table\" x").as_deref(), Some("weird"));
        assert_eq!(sniff_sql_target("SELECT 1"), None);
        assert_eq!(sniff_sql_target("SELECT * FROM db.users").as_deref(), Some("db"));
    }
}
