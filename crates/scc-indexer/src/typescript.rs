//! TypeScript/JavaScript extractor (SCC-026).
//!
//! Tree-sitter based, pure, deterministic: `(path, content) -> ExtractedFile`.
//! Emits symbols, imports, calls, routes (express-style + Next.js route
//! handlers), tests, store refs (prisma/knex/redis/kafka/sql/s3), retry
//! decorators, entrypoints, and JSDoc docstrings. Never panics on malformed
//! input: error/missing nodes are skipped and the traversal is iterative
//! (no recursion depth limits).

use crate::facts;
use crate::model::{
    Call, Entrypoint, ExtractedFile, Import, ImportType, LanguageExtractor, Retry, Route,
    SemanticFact, SourceFile, StoreOp, StoreRef, Symbol, SymbolKind, Test, TestKind,
};
use std::collections::{BTreeMap, BTreeSet};
use tree_sitter::{Language, Node, Parser};

/// Tree-sitter based extractor for TypeScript and JavaScript.
///
/// `.ts`/`.mts`/`.cts`/`.js`/`.mjs`/`.cjs` use the TypeScript grammar;
/// `.tsx`/`.jsx` use the TSX grammar.
// trace:v1 id=impl.scc.extract.typescript work=WORK-SCC-004 satisfies=REQ-SCC-IR
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
        // Per-caller call-site counter (source order) — CFG lexical
        // evidence; frames are processed in document order, so the counter
        // advances deterministically per enclosing callable.
        let mut call_seq: std::collections::BTreeMap<Option<String>, u32> =
            std::collections::BTreeMap::new();
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
                let callee = normalize_callee(node_text(&function, src));
                let seq = call_seq.entry(ctx.caller.clone()).or_insert(0);
                *seq += 1;
                let (conditional, control_block, inside_loop, inside_try) = ts_call_cfg(node);
                out.calls.push(Call {
                    caller: ctx.caller.clone(),
                    callee: callee.clone(),
                    line,
                    known_receiver: known_receiver(&function),
                    conditional,
                    lexical_order: *seq - 1,
                    control_block: control_block.map(str::to_string),
                    inside_loop,
                    inside_try,
                    awaited: ts_call_is_awaited(node, &callee),
                    returns_value: ts_call_returns_value(node),
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

        // Semantic facts (Wave 9): public exports, annotations, fields,
        // registrations, configuration ownership, callbacks. Framework facts
        // (nest decorators, express registrations, react/svelte callbacks)
        // are gated on the matching import; see `collect_facts`.
        out.facts = collect_facts(&root, src, &file.path, &out.imports, &out.symbols);

        // Wave 11: queue consumers (import-gated): bullmq `new Worker(...)`
        // and amqplib `channel.consume(...)` — store refs with SUBSCRIBES
        // semantics, so the atlas seeds Queue invocation surfaces.
        out.store_refs.extend(queue_consumers(&root, src, &out.imports));

        // Module-level mutable globals (`let x = ...`) are STATE facts owned
        // by the module symbol (file stem). Ensure that symbol exists unless
        // a real same-named symbol is declared in this file (Field facts then
        // attach to it — same file, same component attribution, no id clash).
        let module_name = facts::module_stem(&file.path);
        let needs_module = out.facts.iter().any(|f| {
            matches!(f, SemanticFact::Field { owner, .. } if owner == &module_name)
        });
        if needs_module
            && !module_name.is_empty()
            && !out.symbols.iter().any(|s| s.name == module_name)
        {
            out.symbols.push(Symbol {
                name: module_name.clone(),
                kind: SymbolKind::Module,
                signature: None,
                start_line: 1,
                end_line: 1,
                exported: false,
                docstring: None,
                parent: None,
            });
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
// Semantic facts (Wave 9)
// ---------------------------------------------------------------------------
//
// A second, independent traversal collects `SemanticFact`s on top of the
// classic extraction. Facts are pure syntax: public exports (export
// statements), decorators (nest), class fields, framework registrations
// (nest `@Module` arrays, express route/middleware calls, next.config),
// `process.env` configuration reads, and framework callbacks
// (`useEffect`/`onMount`/`addEventListener`). Framework facts are verified
// against the file's imports so a plain method named `get()` is never a
// route and a random `@Whatever()` is never a nest annotation.

/// Enclosing-symbol context for fact attribution.
#[derive(Clone, Default)]
struct FactCtx {
    /// Enclosing callable ("ClassName.method" for methods, arrow-const names).
    caller: Option<String>,
    /// Enclosing class name.
    class: Option<String>,
    /// Module-level const being initialized (e.g. `const PORT = process.env.PORT`).
    const_owner: Option<String>,
}

fn is_next_config_file(path: &str) -> bool {
    matches!(
        path.rsplit('/').next().unwrap_or(path),
        "next.config.js"
            | "next.config.cjs"
            | "next.config.mjs"
            | "next.config.ts"
            | "next.config.cts"
            | "next.config.mts"
    )
}

/// Serialize-side function/method names (general, cross-repo idioms) for
/// the serializer/deserializer pair rule.
fn is_serialize_side(name: &str) -> bool {
    matches!(name, "toJson" | "toJSON" | "serialize" | "stringify")
}

/// Deserialize-side function/method names for the same pair rule.
fn is_deserialize_side(name: &str) -> bool {
    matches!(name, "fromJson" | "fromJSON" | "deserialize" | "parse")
}

/// Interface names in a class declaration's `implements` clause
/// (`class X implements A, B { ... }`). tree-sitter-typescript puts the
/// clause inside a `class_heritage` node; each entry is a `type` node
/// wrapping a `type_identifier` (or a `generic_type` for `Foo<T>`).
fn implemented_interfaces(node: &Node, src: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = node.walk();
    for child in node.named_children(&mut cur) {
        if child.kind() != "class_heritage" {
            continue;
        }
        let mut cur2 = child.walk();
        for clause in child.named_children(&mut cur2) {
            if clause.kind() != "implements_clause" {
                continue;
            }
            let mut cur3 = clause.walk();
            for t in clause.named_children(&mut cur3) {
                // unwrap `type` -> inner identifier/generic_type
                let inner = if t.kind() == "type" {
                    t.named_children(&mut t.walk()).next()
                } else {
                    Some(t)
                };
                let name = match inner.map(|n| n.kind()) {
                    Some("identifier" | "type_identifier") => inner
                        .map(|n| node_text(&n, src).to_string())
                        .unwrap_or_default(),
                    Some("generic_type") => inner
                        .and_then(|n| n.child_by_field_name("type"))
                        .map(|n| node_text(&n, src).to_string())
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                if !name.is_empty() {
                    out.push(name);
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Deterministic total order over facts: (family, owner/symbol, secondary,
/// tertiary). Identical facts sort adjacent so `dedup` collapses them.
fn fact_sort_key(f: &SemanticFact) -> (u8, String, String, String) {
    match f {
        SemanticFact::PublicExport { symbol, kind } => (0, symbol.clone(), kind.clone(), String::new()),
        SemanticFact::Annotation { name, target } => (1, target.clone(), name.clone(), String::new()),
        SemanticFact::Field { owner, name, mutable } => (
            2,
            owner.clone(),
            name.clone(),
            if *mutable { "mutable" } else { "readonly" }.to_string(),
        ),
        SemanticFact::Registration { owner, kind, target } => {
            (3, owner.clone(), kind.clone(), target.clone())
        }
        SemanticFact::Configuration { owner, key } => (4, owner.clone(), key.clone(), String::new()),
        SemanticFact::Callback { owner, callback } => (5, owner.clone(), callback.clone(), String::new()),
        SemanticFact::SchemaDefinition {   owner, name, .. }=> {
            (6, owner.clone(), name.clone(), String::new())
        }
        SemanticFact::SchemaComposition {   owner, name, parent, .. }=> {
            (6, owner.clone(), name.clone(), parent.clone())
        }
        SemanticFact::SchemaValidation {   owner, target, .. }=> {
            (6, owner.clone(), target.clone(), String::new())
        }
        SemanticFact::ReactiveState {   owner, name, access, .. }=> {
            (7, owner.clone(), name.clone(), access.clone())
        }
    }
}

fn has_import(imports: &[Import], pred: impl Fn(&str) -> bool) -> bool {
    imports.iter().any(|i| pred(&i.module))
}

/// `@Name` / `@Name(...)` / `@ns.Name(...)` -> `Name`.
fn decorator_name(dec: &Node, src: &[u8]) -> Option<String> {
    let first = dec.named_children(&mut dec.walk()).next()?;
    match first.kind() {
        "identifier" => {
            let n = node_text(&first, src).to_string();
            if n.is_empty() { None } else { Some(n) }
        }
        "call_expression" => {
            let f = first.child_by_field_name("function")?;
            match f.kind() {
                "identifier" => Some(node_text(&f, src).to_string()),
                "member_expression" => f
                    .child_by_field_name("property")
                    .map(|p| node_text(&p, src).to_string()),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Name of the declaration a decorator decorates (class name, or
/// `Class.member` for body decorators).
fn decorated_target(dec: &Node, src: &[u8], ctx: &FactCtx) -> Option<String> {
    let parent = dec.parent()?;
    match parent.kind() {
        "class_declaration" | "abstract_class_declaration" => parent
            .child_by_field_name("name")
            .map(|n| node_text(&n, src).to_string()),
        "export_statement" => {
            // `export @Controller(...) class X` — find the class/function.
            let mut cur = parent.walk();
            for c in parent.named_children(&mut cur) {
                if matches!(c.kind(), "class_declaration" | "abstract_class_declaration" | "function_declaration" | "generator_function_declaration") {
                    if let Some(n) = c.child_by_field_name("name") {
                        return Some(node_text(&n, src).to_string());
                    }
                }
            }
            None
        }
        "class_body" => {
            // Member decorator: the decorated member is the next sibling.
            let member = dec.next_named_sibling()?;
            let name = match member.kind() {
                "method_definition" | "public_field_definition" => member
                    .child_by_field_name("name")
                    .map(|n| node_text(&n, src).to_string()),
                _ => None,
            }?;
            match &ctx.class {
                Some(c) if !c.is_empty() => Some(format!("{c}.{name}")),
                _ => Some(name),
            }
        }
        _ => None,
    }
}

/// Identifiers in a nest `@Module({ controllers: [...], ... })` object:
/// `(kind, target)` per array element.
fn nest_module_arrays(dec: &Node, src: &[u8], out: &mut Vec<SemanticFact>, owner: &str) {
    let Some(expr) = dec.named_children(&mut dec.walk()).next() else {
        return;
    };
    let Some(call) = expr.kind().eq("call_expression").then_some(expr) else {
        return;
    };
    let Some(args) = call.child_by_field_name("arguments") else {
        return;
    };
    let Some(obj) = args
        .named_children(&mut args.walk())
        .find(|c| c.kind() == "object")
    else {
        return;
    };
    let mut cur = obj.walk();
    for pair in obj.named_children(&mut cur) {
        if pair.kind() != "pair" {
            continue;
        }
        let Some(key) = pair.child_by_field_name("key") else {
            continue;
        };
        let kind = node_text(&key, src);
        if !matches!(kind, "controllers" | "providers" | "imports" | "exports") {
            continue;
        }
        let Some(value) = pair.child_by_field_name("value") else {
            continue;
        };
        if value.kind() != "array" {
            continue;
        }
        let mut cur2 = value.walk();
        for el in value.named_children(&mut cur2) {
            if el.kind() != "identifier" {
                continue;
            }
            let target = node_text(&el, src);
            if !target.is_empty() {
                out.push(SemanticFact::Registration {
                    owner: owner.to_string(),
                    kind: kind.to_string(),
                    target: target.to_string(),
                });
            }
        }
    }
}

/// `process.env.KEY` / `process.env["KEY"]` -> key string.
fn env_key(node: &Node, src: &[u8]) -> Option<String> {
    let is_env = |obj: &Node| -> bool {
        obj.kind() == "member_expression"
            && obj
                .child_by_field_name("object")
                .map(|o| o.kind() == "identifier" && node_text(&o, src) == "process")
                .unwrap_or(false)
            && obj
                .child_by_field_name("property")
                .map(|p| node_text(&p, src) == "env")
                .unwrap_or(false)
    };
    match node.kind() {
        "member_expression" => {
            let obj = node.child_by_field_name("object")?;
            if !is_env(&obj) {
                return None;
            }
            let key = node.child_by_field_name("property")?;
            let k = node_text(&key, src);
            if k.is_empty() { None } else { Some(k.to_string()) }
        }
        "subscript_expression" => {
            let obj = node.child_by_field_name("object")?;
            if !is_env(&obj) {
                return None;
            }
            let idx = node.child_by_field_name("index")?;
            if idx.kind() != "string" {
                return None;
            }
            let k = unquote(node_text(&idx, src));
            if k.is_empty() { None } else { Some(k) }
        }
        _ => None,
    }
}

/// Method name of a member call whose receiver is an identifier in `allowed`
/// (`app.get` -> "get"). Returns None for other shapes.
fn receiver_ident(function: &Node, src: &[u8], allowed: &[&str]) -> Option<String> {
    if function.kind() != "member_expression" {
        return None;
    }
    let obj = function.child_by_field_name("object")?;
    if obj.kind() != "identifier" {
        return None;
    }
    let receiver = node_text(&obj, src);
    if !allowed.contains(&receiver) {
        return None;
    }
    let prop = function.child_by_field_name("property")?;
    let method = node_text(&prop, src);
    if method.is_empty() {
        None
    } else {
        Some(method.to_string())
    }
}

/// Text of the last identifier/member argument (a named handler/middleware),
/// else None.
fn last_named_arg(call: &Node, src: &[u8]) -> Option<String> {
    let args = call.child_by_field_name("arguments")?;
    let children: Vec<Node> = args.named_children(&mut args.walk()).collect();
    for c in children.iter().rev() {
        if matches!(c.kind(), "identifier" | "member_expression") {
            return Some(normalize_callee(node_text(c, src)));
        }
    }
    None
}

/// Reactive access kind for a declaration value: svelte `$state`/`$derived`/
/// `$props`, vue `ref`/`reactive`/`computed`, react `useState`/`useReducer`/
/// `useContext`, mobx `observable`/`action`/`computed`. Framework-import
/// gates are applied by the caller (`react`/`vue`/`mobx`/`svelte` flags).
fn reactive_access(
    v: &Node,
    src: &[u8],
    react: bool,
    vue: bool,
    mobx: bool,
    svelte: bool,
) -> Option<&'static str> {
    if v.kind() != "call_expression" {
        return None;
    }
    let f = v.child_by_field_name("function")?;
    if f.kind() != "identifier" {
        return None;
    }
    let callee = node_text(&f, src);
    if svelte {
        match callee {
            "$state" => return Some("state"),
            "$derived" => return Some("derive"),
            "$props" => return Some("read"),
            _ => {}
        }
    }
    if vue {
        match callee {
            "ref" | "reactive" => return Some("state"),
            "computed" => return Some("derive"),
            _ => {}
        }
    }
    if react {
        match callee {
            "useState" | "useReducer" => return Some("state"),
            "useContext" => return Some("read"),
            _ => {}
        }
    }
    if mobx {
        match callee {
            "observable" | "makeObservable" | "makeAutoObservable" => return Some("state"),
            "action" => return Some("write"),
            "computed" => return Some("derive"),
            _ => {}
        }
    }
    None
}

/// True when a value expression is `z.object({...})` (zod schema
/// construction; `zod_locals` are the local bindings of the zod import).
fn zod_object_expr(v: &Node, src: &[u8], zod_locals: &BTreeSet<String>) -> bool {
    if v.kind() != "call_expression" {
        return false;
    }
    let Some(f) = v.child_by_field_name("function") else {
        return false;
    };
    if f.kind() != "member_expression" {
        return false;
    }
    let Some(obj) = f.child_by_field_name("object") else {
        return false;
    };
    if obj.kind() != "identifier" || !zod_locals.contains(node_text(&obj, src)) {
        return false;
    }
    f.child_by_field_name("property")
        .map(|p| node_text(&p, src) == "object")
        .unwrap_or(false)
}

/// The defining expression text of a zod `z.object(...)` / `z.<schema>(...)`
/// construction call (the `function` part of a call whose receiver is a zod
/// local): `z.object({ name: z.string() })` → the whole call's source text.
/// Returns None when the call is not a zod schema construction.
fn zod_object_expr_text(f: &Node, src: &[u8], zod_locals: &BTreeSet<String>) -> Option<String> {
    if f.kind() != "member_expression" {
        return None;
    }
    let obj = f.child_by_field_name("object")?;
    if obj.kind() != "identifier" || !zod_locals.contains(node_text(&obj, src)) {
        return None;
    }
    let parent = f.parent()?;
    if parent.kind() != "call_expression" {
        return None;
    }
    Some(bound_expr(&parent, src))
}

/// Parent schema name of a zod composition value: `Base.extend({...})` /
/// `Base.merge(...)` → `Base`. Anonymous bases (`z.object({...}).extend(...)`)
/// are not resolvable → None.
fn zod_compose_parent(v: &Node, src: &[u8]) -> Option<String> {    if v.kind() != "call_expression" {
        return None;
    }
    let f = v.child_by_field_name("function")?;
    if f.kind() != "member_expression" {
        return None;
    }
    let method = node_text(&f.child_by_field_name("property")?, src);
    if method != "extend" && method != "merge" {
        return None;
    }
    let obj = f.child_by_field_name("object")?;
    if obj.kind() != "identifier" {
        return None;
    }
    let t = node_text(&obj, src).to_string();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

/// Target schema name of a zod validation call: `UserSchema.parse(x)` /
/// `UserSchema.safeParse(x)` → `UserSchema` (receiver must be an
/// identifier — chained/member receivers are not resolvable locally).
fn zod_parse_target(f: &Node, src: &[u8]) -> Option<String> {
    if f.kind() != "member_expression" {
        return None;
    }
    let method = node_text(&f.child_by_field_name("property")?, src);
    if method != "parse" && method != "safeParse" {
        return None;
    }
    let obj = f.child_by_field_name("object")?;
    if obj.kind() != "identifier" {
        return None;
    }
    let t = node_text(&obj, src).to_string();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

/// True when an identifier node is a declaration/assignment target or a
/// property key — never a reactive read access.
fn is_reactive_decl_position(node: &Node) -> bool {
    let Some(p) = node.parent() else {
        return false;
    };
    match p.kind() {
        "variable_declarator" => p
            .child_by_field_name("name")
            .map(|n| n.id() == node.id())
            .unwrap_or(false),
        "assignment_expression" | "augmented_assignment_expression" => p
            .child_by_field_name("left")
            .map(|l| l.id() == node.id())
            .unwrap_or(false),
        "pair" => p
            .child_by_field_name("key")
            .map(|k| k.id() == node.id())
            .unwrap_or(false),
        "function_declaration" | "function_expression" | "arrow_function" | "method_definition"
        | "generator_function_declaration" | "generator_function" => p
            .child_by_field_name("name")
            .map(|n| n.id() == node.id())
            .unwrap_or(false),
        "import_specifier" | "update_expression" | "labeled_statement" => true,
        _ => false,
    }
}

/// Collect all semantic facts for one file. Iterative (no recursion), never
/// panics: hostile input just yields fewer facts.
/// panics: hostile input just yields fewer facts.
fn collect_facts(
    root: &Node,
    src: &[u8],
    path: &str,
    imports: &[Import],
    symbols: &[Symbol],
) -> Vec<SemanticFact> {
    // Module-symbol name (file stem): owner of module-level STATE facts.
    let module_name = facts::module_stem(path);
    let nest = has_import(imports, |m| m.starts_with("@nestjs/"));
    let express = has_import(imports, |m| m == "express");
    let react = has_import(imports, |m| m == "react");
    let svelte = has_import(imports, |m| m == "svelte");
    let vue = has_import(imports, |m| m == "vue");
    let mobx = has_import(imports, |m| m == "mobx" || m.starts_with("mobx/"));
    let zod = has_import(imports, |m| m == "zod" || m.starts_with("zod/"));
    let next_config = is_next_config_file(path);

    // Local bindings of the zod import (`import { z } from "zod"` →
    // "z"; `import * as z from "zod"` → "z").
    let mut zod_locals: BTreeSet<String> = BTreeSet::new();
    for i in imports {
        if i.module == "zod" || i.module.starts_with("zod/") {
            for (local, _) in &i.names {
                zod_locals.insert(local.clone());
            }
        }
    }

    let mut facts: Vec<SemanticFact> = Vec::new();
    // Wave 11: reactive declarations per owning symbol (declaration facts
    // carry owner; read/write accesses match only the declaring owner).
    let mut reactive_names: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    // Module-level const symbols (zod composition parents must resolve to a
    // locally declared schema).
    let local_consts: BTreeSet<String> = symbols
        .iter()
        .filter(|s| s.parent.is_none() && s.kind == SymbolKind::Const)
        .map(|s| s.name.clone())
        .collect();
    // Contract subclass evidence (Contract ontology): exported module-level
    // function names and per-class method names for the serializer/
    // deserializer pair rule; declared-symbol names for the extension
    // interface guard (an interface declared in this file is a symbol, so a
    // registration targeting it would never materialize a CONTRACT entity).
    let mut module_fns: BTreeSet<String> = BTreeSet::new();
    let mut class_methods: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut frames: Vec<(Node, FactCtx)> = vec![(*root, FactCtx::default())];

    while let Some((node, ctx)) = frames.pop() {
        if node.is_error() || node.is_missing() {
            continue;
        }
        let module_level = ctx.caller.is_none() && ctx.class.is_none();
        match node.kind() {
            "export_statement" => {
                // Re-exports: `export { a } from "m"`, `export * from "m"`,
                // `export * as ns from "m"`.
                if let Some(source) = node.child_by_field_name("source") {
                    let module = unquote(node_text(&source, src));
                    let mut any = false;
                    let mut cur = node.walk();
                    for c in node.named_children(&mut cur) {
                        match c.kind() {
                            "export_clause" => {
                                let mut cur2 = c.walk();
                                for spec in c.named_children(&mut cur2) {
                                    if spec.kind() != "export_specifier" {
                                        continue;
                                    }
                                    let name = field_text(&spec, "alias", src)
                                        .or_else(|| field_text(&spec, "name", src));
                                    if let Some(n) = name {
                                        if !n.is_empty() {
                                            facts.push(SemanticFact::PublicExport {
                                                symbol: n,
                                                kind: "module".into(),
                                            });
                                            any = true;
                                        }
                                    }
                                }
                            }
                            "namespace_export" => {
                                let mut cur2 = c.walk();
                                for id in c.named_children(&mut cur2) {
                                    if id.kind() == "identifier" {
                                        let n = node_text(&id, src);
                                        if !n.is_empty() {
                                            facts.push(SemanticFact::PublicExport {
                                                symbol: n.to_string(),
                                                kind: "module".into(),
                                            });
                                            any = true;
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    // `export * from "m"`: the whole module is public surface.
                    if !any && !module.is_empty() {
                        facts.push(SemanticFact::PublicExport {
                            symbol: module,
                            kind: "module".into(),
                        });
                    }
                } else {
                    // Local re-export `export { a }` or `export default <id>`.
                    // (`default` is an unnamed keyword token.)
                    let mut cur = node.walk();
                    let has_default = node.children(&mut cur).any(|c| c.kind() == "default");
                    let children: Vec<Node> = node.named_children(&mut cur).collect();
                    if has_default {
                        for id in children.iter().filter(|c| c.kind() == "identifier") {
                            let n = node_text(id, src);
                            if !n.is_empty() {
                                facts.push(SemanticFact::PublicExport {
                                    symbol: n.to_string(),
                                    kind: "module".into(),
                                });
                            }
                        }
                    }
                    for c in children {
                        if c.kind() != "export_clause" {
                            continue;
                        }
                        let mut cur2 = c.walk();
                        for spec in c.named_children(&mut cur2) {
                            if spec.kind() != "export_specifier" {
                                continue;
                            }
                            let name = field_text(&spec, "alias", src)
                                .or_else(|| field_text(&spec, "name", src));
                            if let Some(n) = name {
                                if !n.is_empty() {
                                    facts.push(SemanticFact::PublicExport {
                                        symbol: n,
                                        kind: "module".into(),
                                    });
                                }
                            }
                        }
                    }
                }
                // Decorators on `export @Controller(...) class X`.
                if nest {
                    let mut cur = node.walk();
                    for dec in node.named_children(&mut cur) {
                        if dec.kind() != "decorator" {
                            continue;
                        }
                        if let Some(target) = decorated_target(&dec, src, &ctx) {
                            if let Some(name) = decorator_name(&dec, src) {
                                facts.push(SemanticFact::Annotation {
                                    name: name.clone(),
                                    target: target.clone(),
                                });
                                if name == "Module" {
                                    nest_module_arrays(&dec, src, &mut facts, &target);
                                }
                            }
                        }
                    }
                }
            }
            "class_declaration" | "abstract_class_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| node_text(&n, src).to_string())
                    .unwrap_or_default();
                if name.is_empty() {
                    continue;
                }
                // Extension contracts: a class implementing an interface is
                // an implementation of that extension point. Emitted only
                // when the interface is not a symbol declared in this file
                // (a same-file interface would never materialize a CONTRACT
                // entity; cross-file interfaces are the idiomatic shape).
                if module_level {
                    for iface in implemented_interfaces(&node, src) {
                        if symbols.iter().any(|s| s.name == iface) {
                            continue;
                        }
                        facts.push(SemanticFact::Registration {
                            owner: name.clone(),
                            kind: "extension".into(),
                            target: iface,
                        });
                    }
                }
                if module_level && is_exported(&node) {
                    facts.push(SemanticFact::PublicExport {
                        symbol: name.clone(),
                        kind: "class".into(),
                    });
                }
                // Class fields: `readonly` modifier -> immutable.
                if module_level {
                    if let Some(body) = node.child_by_field_name("body") {
                        let mut cur = body.walk();
                        for f in body.named_children(&mut cur) {
                            if f.kind() != "public_field_definition" {
                                continue;
                            }
                            let fname = f
                                .child_by_field_name("name")
                                .or_else(|| {
                                    f.named_children(&mut f.walk())
                                        .find(|c| c.kind() == "property_identifier")
                                })
                                .map(|n| node_text(&n, src).to_string())
                                .unwrap_or_default();
                            if fname.is_empty() {
                                continue;
                            }
                            let mutable =
                                !f.children(&mut f.walk()).any(|c| c.kind() == "readonly");
                            facts.push(SemanticFact::Field {
                                owner: name.clone(),
                                name: fname,
                                mutable,
                            });
                        }
                    }
                }
                // Static factory methods (`static of/create/from/...`) and
                // fluent builder chains (`.withX()/.setX()/.addX()` returning
                // `this`) make the class a factory/builder.
                if module_level {
                    if let Some(body) = node.child_by_field_name("body") {
                        let mut cur = body.walk();
                        for m in body.named_children(&mut cur) {
                            if m.kind() != "method_definition" {
                                continue;
                            }
                            let mname = m
                                .child_by_field_name("name")
                                .map(|n| node_text(&n, src).to_string())
                                .unwrap_or_default();
                            if mname.is_empty() {
                                continue;
                            }
                            // `static` is a direct keyword child of the
                            // method_definition (no modifiers field here).
                            let is_static = m
                                .children(&mut m.walk())
                                .any(|k| k.kind() == "static");
                            if is_static && facts::is_factory_name("typescript", &mname) {
                                facts.push(SemanticFact::Registration {
                                    owner: name.clone(),
                                    kind: "factory".into(),
                                    target: name.clone(),
                                });
                                facts.push(SemanticFact::PublicExport {
                                    symbol: format!("{name}.{mname}"),
                                    kind: "method".into(),
                                });
                            } else if facts::is_builder_chain_method(&mname)
                                && method_returns_this(m, src)
                            {
                                facts.push(SemanticFact::Registration {
                                    owner: name.clone(),
                                    kind: "builder".into(),
                                    target: name.clone(),
                                });
                            }
                        }
                    }
                }
                // Decorators on a plain (non-exported) decorated class.
                if nest {
                    let mut cur = node.walk();
                    for dec in node.named_children(&mut cur) {
                        if dec.kind() != "decorator" {
                            continue;
                        }
                        if let Some(target) = decorated_target(&dec, src, &ctx) {
                            if let Some(dname) = decorator_name(&dec, src) {
                                facts.push(SemanticFact::Annotation {
                                    name: dname.clone(),
                                    target: target.clone(),
                                });
                                if dname == "Module" {
                                    nest_module_arrays(&dec, src, &mut facts, &target);
                                }
                            }
                        }
                    }
                }
            }
            "class_body" => {
                // Member decorators (`@Get()` above a method).
                if nest {
                    let mut cur = node.walk();
                    for dec in node.named_children(&mut cur) {
                        if dec.kind() != "decorator" {
                            continue;
                        }
                        if let Some(target) = decorated_target(&dec, src, &ctx) {
                            if let Some(dname) = decorator_name(&dec, src) {
                                facts.push(SemanticFact::Annotation {
                                    name: dname,
                                    target,
                                });
                            }
                        }
                    }
                }
            }
            "function_declaration" | "generator_function_declaration" => {
                if module_level {
                    let n = node
                        .child_by_field_name("name")
                        .map(|n| node_text(&n, src).to_string())
                        .unwrap_or_default();
                    if n.is_empty() {
                        continue;
                    }
                    if is_exported(&node) {
                        facts.push(SemanticFact::PublicExport {
                            symbol: n.clone(),
                            kind: "function".into(),
                        });
                        module_fns.insert(n.clone());
                    }
                    // Module-level factory functions (vue `createApp`,
                    // axios-style `createInstance`/`createClient`).
                    if facts::is_factory_name("typescript", &n) {
                        facts.push(SemanticFact::Registration {
                            owner: n.clone(),
                            kind: "factory".into(),
                            target: n,
                        });
                    }
                }
            }
            "method_definition" => {
                // Per-class method names for the serializer/deserializer
                // pair rule.
                if let Some(class) = &ctx.class {
                    let n = node
                        .child_by_field_name("name")
                        .map(|n| node_text(&n, src).to_string())
                        .unwrap_or_default();
                    if !n.is_empty() {
                        class_methods.entry(class.clone()).or_default().insert(n);
                    }
                }
            }
            "interface_declaration" => {
                if module_level && is_exported(&node) {
                    let n = node
                        .child_by_field_name("name")
                        .map(|n| node_text(&n, src).to_string())
                        .unwrap_or_default();
                    if !n.is_empty() {
                        facts.push(SemanticFact::PublicExport {
                            symbol: n,
                            kind: "interface".into(),
                        });
                    }
                }
            }
            "type_alias_declaration" => {
                if module_level && is_exported(&node) {
                    let n = node
                        .child_by_field_name("name")
                        .map(|n| node_text(&n, src).to_string())
                        .unwrap_or_default();
                    if !n.is_empty() {
                        facts.push(SemanticFact::PublicExport {
                            symbol: n,
                            kind: "type".into(),
                        });
                    }
                }
            }
            "enum_declaration" => {
                if module_level && is_exported(&node) {
                    let n = node
                        .child_by_field_name("name")
                        .map(|n| node_text(&n, src).to_string())
                        .unwrap_or_default();
                    if !n.is_empty() {
                        facts.push(SemanticFact::PublicExport {
                            symbol: n,
                            kind: "enum".into(),
                        });
                    }
                }
            }
            "lexical_declaration" => {
                let exported = is_exported(&node);
                let mut cur = node.walk();
                let declarators: Vec<Node> = node
                    .named_children(&mut cur)
                    .filter(|d| d.kind() == "variable_declarator")
                    .collect();
                // Wave 11: reactive declarations (svelte $state/$derived/
                // $props, vue ref/reactive/computed, react useState/
                // useReducer/useContext, mobx observable/action/computed) and
                // zod schema definitions/compositions — at any nesting level.
                for d in &declarators {
                    let Some(name_node) = d.child_by_field_name("name") else {
                        continue;
                    };
                    // React `const [count, setCount] = useState(0)` — the
                    // first destructured element names the state value.
                    let name = match name_node.kind() {
                        "identifier" => Some(node_text(&name_node, src).to_string()),
                        "array_pattern" => {
                            let mut c = name_node.walk();
                            let els: Vec<Node> =
                                name_node.named_children(&mut c).collect();
                            els.into_iter().find_map(|el| {
                                (el.kind() == "identifier")
                                    .then(|| node_text(&el, src).to_string())
                            })
                        }
                        _ => None,
                    };
                    let Some(name) = name else {
                        continue;
                    };
                    let Some(v) = d.child_by_field_name("value") else {
                        continue;
                    };
                    if let Some(access) = reactive_access(&v, src, react, vue, mobx, svelte) {
                        let owner = ctx
                            .caller
                            .clone()
                            .or_else(|| ctx.class.clone())
                            .unwrap_or_else(|| module_name.clone());
                        facts.push(SemanticFact::ReactiveState {
                            owner: owner.clone(),
                            name: name.clone(),
                            access: access.to_string(),
                            expr: bound_expr(&v, src),
                        });
                        reactive_names
                            .entry(owner)
                            .or_default()
                            .insert(name.clone());
                    }
                    // zod schema definition: `export const X = z.object(...)`.
                    if module_level && exported && zod && zod_object_expr(&v, src, &zod_locals) {
                        facts.push(SemanticFact::SchemaDefinition {
                            owner: name.clone(),
                            name: name.clone(),
                            expr: bound_expr(&v, src),
                        });
                    }
                    // zod composition: `const X = Base.extend(...)` /
                    // `Base.merge(...)` with a locally declared base.
                    if module_level && zod {
                        if let Some(parent) = zod_compose_parent(&v, src) {
                            if local_consts.contains(&parent) {
                                facts.push(SemanticFact::SchemaComposition {
                                    owner: name.clone(),
                                    name: name.clone(),
                                    parent,
                                    expr: bound_expr(&v, src),
                                });
                            }
                        }
                    }
                }
                if module_level {
                    // `let` bindings are mutable module state; `const` is
                    // intent-immutable (skip). The declaration keyword is the
                    // `kind` field (`let`/`const`/`var`).
                    let is_let = node
                        .child_by_field_name("kind")
                        .map(|k| node_text(&k, src) == "let")
                        .unwrap_or(false);
                    for d in &declarators {
                        if d.kind() != "variable_declarator" {
                            continue;
                        }
                        let name = d
                            .child_by_field_name("name")
                            .map(|n| node_text(&n, src).to_string())
                            .unwrap_or_default();
                        if name.is_empty() {
                            continue;
                        }
                        if is_let {
                            facts.push(SemanticFact::Field {
                                owner: module_name.clone(),
                                name: name.clone(),
                                mutable: true,
                            });
                        }
                        if exported {
                            facts.push(SemanticFact::PublicExport {
                                symbol: name.clone(),
                                kind: "const".into(),
                            });
                        }
                        // Module-level arrow/function consts with factory-ish
                        // names (vue `createApp = (...) => ...`).
                        if facts::is_factory_name("typescript", &name) {
                            let fn_val = d.child_by_field_name("value").map(|v| {
                                matches!(
                                    v.kind(),
                                    "arrow_function" | "function_expression" | "generator_function"
                                )
                            });
                            if fn_val.unwrap_or(false) {
                                facts.push(SemanticFact::Registration {
                                    owner: name.clone(),
                                    kind: "factory".into(),
                                    target: name.clone(),
                                });
                            }
                        }
                        // next.config: the exported config object registers
                        // with the next framework (filename-verified).
                        if next_config {
                            if let Some(v) = d.child_by_field_name("value") {
                                if v.kind() == "object" {
                                    facts.push(SemanticFact::Registration {
                                        owner: name.clone(),
                                        kind: "next-config".into(),
                                        target: "next".into(),
                                    });
                                }
                            }
                        }
                        // Object-literal factory namespaces (zod `z = {
                        // object(...), string(...) }`, axios `axios = {
                        // create(...) }`): function-valued properties with
                        // factory-ish names are factories of that namespace.
                        if let Some(v) = d.child_by_field_name("value") {
                            if v.kind() == "object" {
                                let mut cur2 = v.walk();
                                for pair in v.named_children(&mut cur2) {
                                    if pair.kind() != "pair" {
                                        continue;
                                    }
                                    let key = pair
                                        .child_by_field_name("key")
                                        .map(|k| node_text(&k, src).to_string())
                                        .unwrap_or_default();
                                    if key.is_empty()
                                        || !facts::is_namespace_factory_key(&key)
                                    {
                                        continue;
                                    }
                                    let fn_val = pair
                                        .child_by_field_name("value")
                                        .map(|vv| {
                                            matches!(
                                                vv.kind(),
                                                "arrow_function" | "function_expression"
                                            )
                                        })
                                        .unwrap_or(false);
                                    if fn_val {
                                        facts.push(SemanticFact::Registration {
                                            owner: name.clone(),
                                            kind: "factory".into(),
                                            target: key,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "call_expression" => {
                let Some(function) = node.child_by_field_name("function") else {
                    continue;
                };
                // Wave 11: zod schema validation (`UserSchema.parse(x)` /
                // `UserSchema.safeParse(x)`), import-gated on zod.
                if zod {
                    if let Some(target) = zod_parse_target(&function, src) {
                        let owner = ctx
                            .caller
                            .clone()
                            .or_else(|| ctx.const_owner.clone())
                            .or_else(|| ctx.class.clone());
                        if let Some(owner) = owner {
                            facts.push(SemanticFact::SchemaValidation {
                                owner,
                                target,
                                expr: bound_expr(&node, src),
                            });
                        }
                    }
                    // inline schema construction in any context
                    // (`z.object({...})` inside test bodies, handlers,
                    // callbacks, and non-exported const values) — the
                    // concrete code form is the fact; write.rs dedupes by
                    // expr (entity id = expr) and accumulates counts so the
                    // atlas surfaces only the *repeated* DSL surface. The
                    // module-level exported-const path above still emits
                    // named definitions.
                    if let Some(expr) = zod_object_expr_text(&function, src, &zod_locals) {
                        let owner = ctx
                            .caller
                            .clone()
                            .or_else(|| ctx.const_owner.clone())
                            .or_else(|| ctx.class.clone())
                            .unwrap_or_else(|| module_name.clone());
                        facts.push(SemanticFact::SchemaDefinition {
                            owner,
                            name: expr.clone(),
                            expr,
                        });
                    }
                }
                // Express registrations (import-verified): app/router/server/
                // api/route/express .get/.post/.../.use(...).
                if express {
                    if let Some(method) = receiver_ident(
                        &function,
                        src,
                        &["app", "router", "server", "api", "route", "express"],
                    ) {
                        let owner = ctx
                            .caller
                            .clone()
                            .unwrap_or_else(|| receiver_text(&function, src));
                        if method == "use" {
                            let target = last_named_arg(&node, src).or_else(|| {
                                first_string_arg(&node, src).or_else(|| {
                                    node.child_by_field_name("arguments")
                                        .and_then(|a| {
                                            a.named_children(&mut a.walk()).next()
                                        })
                                        .map(|a| match a.kind() {
                                            "call_expression" => a
                                                .child_by_field_name("function")
                                                .map(|f| normalize_callee(node_text(&f, src)))
                                                .unwrap_or_default(),
                                            _ => normalize_callee(node_text(&a, src)),
                                        })
                                })
                            });
                            if let Some(t) = target {
                                if !t.is_empty() {
                                    facts.push(SemanticFact::Registration {
                                        owner,
                                        kind: "middleware".into(),
                                        target: t,
                                    });
                                }
                            }
                        } else if matches!(
                            method.as_str(),
                            "get" | "post" | "put" | "delete" | "patch" | "all"
                        ) {
                            if let Some(p) = first_string_arg(&node, src) {
                                facts.push(SemanticFact::Registration {
                                    owner,
                                    kind: "route".into(),
                                    target: format!("{} {p}", method.to_uppercase()),
                                });
                            }
                        }
                    }
                }
                // React useEffect / Svelte onMount callbacks.
                if function.kind() == "identifier" {
                    let callee = node_text(&function, src);
                    let framework_cb =
                        (react && callee == "useEffect") || (svelte && callee == "onMount");
                    if framework_cb {
                        if let Some(owner) = ctx.caller.clone() {
                            if let Some(cb) = last_named_arg(&node, src) {
                                facts.push(SemanticFact::Callback { owner, callback: cb });
                            }
                        }
                    }
                }
                // DOM addEventListener (receiver-verified: window/document/
                // globalThis).
                if let Some(method) =
                    receiver_ident(&function, src, &["window", "document", "globalThis"])
                {
                    if method == "addEventListener" {
                        if let Some(owner) = ctx.caller.clone() {
                            if let Some(cb) = last_named_arg(&node, src) {
                                facts.push(SemanticFact::Callback { owner, callback: cb });
                            }
                        }
                    }
                }
            }
            "identifier" => {
                // Wave 11: reactive read accesses — a reference to a
                // declared reactive name owned by the current caller.
                if !reactive_names.is_empty() {
                    let owner = ctx
                        .caller
                        .clone()
                        .unwrap_or_else(|| module_name.clone());
                    if let Some(names) = reactive_names.get(&owner) {
                        let t = node_text(&node, src);
                        if names.contains(t)
                            && !is_reactive_decl_position(&node)
                            && !t.starts_with('$')
                        {
                            facts.push(SemanticFact::ReactiveState {  
                                owner,
                                name: t.to_string(),
                                access: "read".into(), expr: String::new() });
                        }
                    }
                }
            }
            "assignment_expression" | "augmented_assignment_expression" => {
                // Wave 11: reactive write accesses (`count = 5` on a
                // declared reactive).
                if !reactive_names.is_empty() {
                    if let Some(left) = node.child_by_field_name("left") {
                        if left.kind() == "identifier" {
                            let owner = ctx
                                .caller
                                .clone()
                                .unwrap_or_else(|| module_name.clone());
                            if reactive_names
                                .get(&owner)
                                .map(|s| s.contains(node_text(&left, src)))
                                .unwrap_or(false)
                            {
                                facts.push(SemanticFact::ReactiveState {  
                                    owner,
                                    name: node_text(&left, src).to_string(),
                                    access: "write".into(), expr: String::new() });
                            }
                        }
                    }
                }
                // next.config: `module.exports = nextConfig`.
                if next_config {
                    if let Some(left) = node.child_by_field_name("left") {
                        if left.kind() == "member_expression" && node_text(&left, src) == "module.exports" {
                            if let Some(right) = node.child_by_field_name("right") {
                                if right.kind() == "identifier" {
                                    let owner = node_text(&right, src);
                                    if !owner.is_empty() {
                                        facts.push(SemanticFact::Registration {
                                            owner: owner.to_string(),
                                            kind: "next-config".into(),
                                            target: "next".into(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "member_expression" | "subscript_expression" => {
                if let Some(key) = env_key(&node, src) {
                    let owner = ctx
                        .caller
                        .clone()
                        .or_else(|| ctx.const_owner.clone())
                        .or_else(|| ctx.class.clone());
                    if let Some(owner) = owner {
                        facts.push(SemanticFact::Configuration { owner, key });
                    }
                }
            }
            _ => {}
        }

        // Push children with adjusted fact context. Decorator subtrees are
        // consumed above; skipping them here avoids double-reporting.
        let mut children: Vec<Node> = Vec::new();
        {
            let mut cur = node.walk();
            for c in node.named_children(&mut cur) {
                if c.kind() == "decorator" {
                    continue;
                }
                children.push(c);
            }
        }
        let child_ctx: FactCtx = match node.kind() {
            "function_declaration" | "generator_function_declaration" => {
                if module_level {
                    let n = node
                        .child_by_field_name("name")
                        .map(|n| node_text(&n, src).to_string())
                        .unwrap_or_default();
                    if !n.is_empty() {
                        FactCtx {
                            caller: Some(n),
                            class: None,
                            const_owner: ctx.const_owner.clone(),
                        }
                    } else {
                        ctx.clone()
                    }
                } else {
                    ctx.clone()
                }
            }
            "method_definition" => {
                let n = node
                    .child_by_field_name("name")
                    .map(|n| node_text(&n, src).to_string())
                    .unwrap_or_default();
                if n.is_empty() {
                    ctx.clone()
                } else {
                    let full = match &ctx.class {
                        Some(c) if !c.is_empty() => format!("{c}.{n}"),
                        Some(_) => n,
                        None => n,
                    };
                    FactCtx {
                        caller: Some(full),
                        class: None,
                        const_owner: ctx.const_owner.clone(),
                    }
                }
            }
            "class_declaration" | "abstract_class_declaration" => {
                if module_level {
                    let n = node
                        .child_by_field_name("name")
                        .map(|n| node_text(&n, src).to_string())
                        .unwrap_or_default();
                    if !n.is_empty() {
                        FactCtx { caller: None, class: Some(n), const_owner: None }
                    } else {
                        ctx.clone()
                    }
                } else {
                    ctx.clone()
                }
            }
            "lexical_declaration" => {
                // Module-level arrow/function consts become callers; every
                // declarator value reads config under the const's name.
                let mut cur = node.walk();
                for d in node.named_children(&mut cur) {
                    if d.kind() != "variable_declarator" {
                        continue;
                    }
                    let name = d
                        .child_by_field_name("name")
                        .map(|n| node_text(&n, src).to_string())
                        .unwrap_or_default();
                    if name.is_empty() {
                        continue;
                    }
                    let mut d_ctx = ctx.clone();
                    if module_level {
                        d_ctx.const_owner = Some(name.clone());
                        if let Some(v) = d.child_by_field_name("value") {
                            if matches!(
                                v.kind(),
                                "arrow_function" | "function_expression" | "generator_function"
                            ) {
                                d_ctx.caller = Some(name.clone());
                            }
                        }
                    }
                    let dchildren: Vec<Node> = d.named_children(&mut d.walk()).collect();
                    for c in dchildren.iter().rev() {
                        frames.push((*c, d_ctx.clone()));
                    }
                }
                continue;
            }
            "public_field_definition" => {
                // Field initializers read config under the class name.
                FactCtx {
                    caller: None,
                    class: ctx.class.clone(),
                    const_owner: ctx.class.clone(),
                }
            }
            _ => ctx.clone(),
        };
        for c in children.iter().rev() {
            frames.push((*c, child_ctx.clone()));
        }
    }

    // Contract subclass evidence (Contract ontology): serializer/
    // deserializer pairs. An exported module-level function pair
    // (`toJson`+`fromJson`, `serialize`+`deserialize`) or a class method
    // pair around the class is a Serialization contract; the surface is
    // the `ser/de` pair string. Deterministic: sorted names, first
    // matching side wins. Owner is the serializer function (a declared
    // symbol) or the class.
    let pair = |members: &BTreeSet<String>| -> Option<(String, String)> {
        let ser = members.iter().find(|m| is_serialize_side(m))?;
        let de = members.iter().find(|m| is_deserialize_side(m))?;
        Some((ser.clone(), de.clone()))
    };
    if let Some((ser, de)) = pair(&module_fns) {
        facts.push(SemanticFact::Registration {
            owner: ser.clone(),
            kind: "serialization".to_string(),
            target: format!("{ser}/{de}"),
        });
    }
    for (class, members) in &class_methods {
        if let Some((ser, de)) = pair(members) {
            facts.push(SemanticFact::Registration {
                owner: class.clone(),
                kind: "serialization".to_string(),
                target: format!("{ser}/{de}"),
            });
        }
    }

    facts.sort_by_key(fact_sort_key);
    facts.dedup();
    facts
}

/// Wave 11 queue consumers (import-gated, deterministic document order):
/// bullmq `new Worker("queue", handler)` and amqplib
/// `channel.consume("queue", handler)` — both emit SUBSCRIBES store refs so
/// the atlas seeds Queue invocation surfaces. kafkajs `consumer.subscribe`
/// is already covered by the receiver-based store-ref path.
fn queue_consumers(root: &Node, src: &[u8], imports: &[Import]) -> Vec<StoreRef> {
    let bullmq = has_import(imports, |m| m == "bullmq");
    let amqplib = has_import(imports, |m| m == "amqplib");
    if !bullmq && !amqplib {
        return Vec::new();
    }
    let mut out: Vec<StoreRef> = Vec::new();
    let mut frames: Vec<(Node, Option<String>)> = vec![(*root, None)];
    while let Some((node, caller)) = frames.pop() {
        if node.is_error() || node.is_missing() {
            continue;
        }
        let mut next_caller = caller.clone();
        match node.kind() {
            "new_expression" if bullmq => {
                let is_worker = node
                    .child_by_field_name("constructor")
                    .map(|c| c.kind() == "identifier" && node_text(&c, src) == "Worker")
                    .unwrap_or(false);
                if is_worker {
                    if let Some(target) = first_string_arg(&node, src) {
                        out.push(StoreRef {
                            caller: caller.clone(),
                            store: "bullmq".into(),
                            technology: Some("bullmq".into()),
                            op: StoreOp::Subscribe,
                            target: Some(target),
                            line: line_of(&node),
                        });
                    }
                }
            }
            "call_expression" if amqplib => {
                if let Some(f) = node.child_by_field_name("function") {
                    if f.kind() == "member_expression" {
                        let method = f
                            .child_by_field_name("property")
                            .map(|p| node_text(&p, src))
                            .unwrap_or("");
                        if method == "consume" {
                            if let Some(target) = first_string_arg(&node, src) {
                                out.push(StoreRef {
                                    caller: caller.clone(),
                                    store: "amqp".into(),
                                    technology: Some("rabbitmq".into()),
                                    op: StoreOp::Subscribe,
                                    target: Some(target),
                                    line: line_of(&node),
                                });
                            }
                        }
                    }
                }
            }
            "function_declaration" | "method_definition" => {
                if let Some(name) = node
                    .child_by_field_name("name")
                    .map(|n| node_text(&n, src).to_string())
                {
                    if !name.is_empty() {
                        next_caller = Some(match &caller {
                            Some(c) if node.kind() == "method_definition" => {
                                format!("{c}.{name}")
                            }
                            _ => name,
                        });
                    }
                }
            }
            _ => {}
        }
        let mut children: Vec<Node> = Vec::new();
        {
            let mut cur = node.walk();
            for c in node.named_children(&mut cur) {
                children.push(c);
            }
        }
        for c in children.iter().rev() {
            frames.push((*c, next_caller.clone()));
        }
    }
    out
}

fn receiver_text(function: &Node, src: &[u8]) -> String {
    function
        .child_by_field_name("object")
        .map(|o| node_text(&o, src).to_string())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text<'a>(node: &Node, src: &'a [u8]) -> &'a str {    node.utf8_text(src).unwrap_or("")
}

fn line_of(node: &Node) -> u32 {
    node.start_position().row as u32 + 1
}

/// The defining source expression of a node, bounded to 200 chars
/// (single-line, whitespace-collapsed) — enough to carry the concrete
/// code form (`z.object({ name: z.string() })`) into the atlas without
/// flooding it with a whole multi-line schema literal.
fn bound_expr(node: &Node, src: &[u8]) -> String {
    let text = node_text(node, src);
    let one_line = collapse_ws(text);
    let mut out = one_line;
    if out.chars().count() > 200 {
        out = out.chars().take(197).collect::<String>() + "...";
    }
    out
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
/// True when a class method body contains `return this;` (fluent builder
/// evidence for `.withX()/.setX()/.addX()` chains). The returned `this`
/// is a direct child of the `return_statement` (no `argument` field in
/// this grammar version).
fn method_returns_this(node: Node, src: &[u8]) -> bool {
    let Some(body) = node.child_by_field_name("body") else {
        return false;
    };
    let mut cur = body.walk();
    for c in body.named_children(&mut cur) {
        if c.kind() != "return_statement" {
            continue;
        }
        let mut c2 = c.walk();
        if c.children(&mut c2).any(|k| node_text(&k, src) == "this") {
            return true;
        }
    }
    false
}

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

/// CFG evidence for a call site: `(conditional, control_block, inside_loop,
/// inside_try)`. True when the call sits inside a conditional/loop/try/
/// catch/switch body within its enclosing function — the ONLY evidence that
/// turns call fanout into control-flow branching. Stops at the nearest
/// function boundary so calls inside conditionally-defined closures are not
/// marked. The nearest control block wins; loop/try nesting accumulates
/// independently of it.
fn ts_call_cfg(node: Node) -> (bool, Option<&'static str>, bool, bool) {
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
            "function_declaration" | "function_expression" | "arrow_function"
            | "method_definition" | "class_declaration" | "program" | "module" => break,
            "if_statement" => {
                block.get_or_insert("if");
            }
            "else_clause" => {
                block.get_or_insert("else");
            }
            "for_statement" | "for_in_statement" | "for_of_statement" => {
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
            "switch_statement" => {
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

/// True when the call is awaited (`await expr`) or is a `Promise.all(...)`
/// call — the flow edge is Async. `callee` is the normalized callee text.
fn ts_call_is_awaited(node: Node, callee: &str) -> bool {
    if callee == "Promise.all" {
        return true;
    }
    let mut cur = node.parent();
    while let Some(anc) = cur {
        match anc.kind() {
            "function_declaration" | "function_expression" | "arrow_function"
            | "method_definition" | "class_declaration" | "program" | "module" => return false,
            "await_expression" => return true,
            _ => cur = anc.parent(),
        }
    }
    false
}

/// True when the call's result is consumed (assigned/returned/compared/
/// passed/awaited) rather than discarded as a bare expression statement.
fn ts_call_returns_value(node: Node) -> bool {
    let mut cur = node.parent();
    while let Some(anc) = cur {
        match anc.kind() {
            "function_declaration" | "function_expression" | "arrow_function"
            | "method_definition" | "class_declaration" | "program" | "module"
            | "expression_statement" => return false,
            "await_expression" | "parenthesized_expression" => cur = anc.parent(),
            _ => return true,
        }
    }
    false
}

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
            "prisma" | "sequelize" | "typeorm" | "mongoose" | "knex" | "Model" => {
                if segments.is_empty() {
                    return None;
                }
                let op = orm_op(method)?;
                // model nearest the root (`prisma.user.create` -> user), or
                // the receiver itself for bare `Model.create(...)`.
                let target = if segments.len() >= 2 {
                    segments.last().cloned()
                } else {
                    Some(r.to_string())
                };
                let technology = if r == "mongoose" || r == "Model" { "mongodb" } else { "sql" };
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
                // drizzle chains (`db.insert(t).values(...)`) are handled
                // by the inner `db.insert(t)` call, which is recorded on its
                // own; chain verbs like `values`/`set` map to no op here.
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
/// `db.insert(users)` etc. are direct writes with an identifier target
/// (drizzle-style table/model argument).
fn sql_op(method: &str, call: &Node, src: &[u8]) -> Option<(StoreOp, Option<String>)> {
    match method {
        "query" | "execute" => {
            let sql = sql_text_arg(call, src)?;
            Some((sniff_sql_op(&sql), sniff_sql_target(&sql)))
        }
        "insert" | "update" | "delete" => {
            Some((StoreOp::Write, first_ident_arg(call, src)))
        }
        "select" => Some((StoreOp::Query, first_ident_arg(call, src))),
        _ => None,
    }
}

/// First identifier argument of a call (drizzle `db.insert(users)` table).
fn first_ident_arg(call: &Node, src: &[u8]) -> Option<String> {
    let args = call.child_by_field_name("arguments")?;
    let mut cur = args.walk();
    for a in args.named_children(&mut cur) {
        if a.kind() == "identifier" {
            return Some(node_text(&a, src).to_string());
        }
    }
    None
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
    fn wave12_inline_zod_object_captured_in_function_body() {
        // `z.object({...})` inside a function body (the zod test/handler
        // form) must emit an inline SchemaDefinition carrying the expr.
        let ef = extract(
            "src/probe.ts",
            "import { z } from \"zod/v4\";\nfunction f() {\n  const schema = z.object({ name: z.string() });\n  schema.parse({ name: \"x\" });\n}\n",
        );
        let defs: Vec<&str> = ef
            .facts
            .iter()
            .filter_map(|f| match f {
                SemanticFact::SchemaDefinition { expr, .. } if !expr.is_empty() => {
                    Some(expr.as_str())
                }
                _ => None,
            })
            .collect();
        assert!(
            defs.contains(&"z.object({ name: z.string() })"),
            "inline def with expr: {defs:?}"
        );
    }

    #[test]
    fn wave12_inline_zod_object_in_test_callback() {
        // zod's real test form: `import * as z from "zod/v4"` + the
        // z.object call inside a `test("...", () => {...})` callback.
        let ef = extract(
            "src/probe.test.ts",
            "import * as z from \"zod/v4\";\nimport { test } from \"vitest\";\n\ntest(\"obj\", () => {\n  const schema = z.object({ name: z.string() });\n});\n",
        );
        let defs: Vec<&str> = ef
            .facts
            .iter()
            .filter_map(|f| match f {
                SemanticFact::SchemaDefinition { expr, .. } if !expr.is_empty() => {
                    Some(expr.as_str())
                }
                _ => None,
            })
            .collect();
        assert!(
            defs.contains(&"z.object({ name: z.string() })"),
            "inline def in test callback: {defs:?}"
        );
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
    fn cfg_evidence_blocks_await_and_order() {
        let ef = extract(
            "src/app.ts",
            r#"async function process(payload: any) {
  try {
    const valid = validate(payload);
    if (valid) {
      save(payload);
    } else {
      reject(payload);
    }
  } catch (e) {
    log(e);
  }
  await persist(payload);
  await Promise.all([first(), second()]);
  fanout();
}
"#,
        );
        let calls = &ef.calls;
        // validate, save, reject, log, persist, Promise.all, first, second,
        // fanout
        assert_eq!(calls.len(), 9, "{calls:?}");
        let orders: Vec<u32> = calls.iter().map(|c| c.lexical_order).collect();
        assert_eq!(orders, vec![0, 1, 2, 3, 4, 5, 6, 7, 8], "{orders:?}");
        let by_callee = |name: &str| -> &Call {
            calls
                .iter()
                .find(|c| c.callee == name)
                .unwrap_or_else(|| panic!("call {name}"))
        };
        let validate = by_callee("validate");
        assert_eq!(validate.control_block.as_deref(), Some("try"));
        assert!(validate.inside_try);
        let save = by_callee("save");
        assert_eq!(save.control_block.as_deref(), Some("if"));
        assert!(save.inside_try, "if nested in try is still inside_try");
        let reject = by_callee("reject");
        assert_eq!(reject.control_block.as_deref(), Some("else"));
        let log = by_callee("log");
        assert_eq!(log.control_block.as_deref(), Some("catch"));
        assert!(log.inside_try);

        // awaited calls: `await persist(...)` and `Promise.all([...])`.
        let persist = by_callee("persist");
        assert!(persist.awaited);
        let promise_all = by_callee("Promise.all");
        assert!(promise_all.awaited);
        let first = by_callee("first");
        assert!(first.awaited, "first() inside awaited Promise.all is awaited with the batch");
        let second = by_callee("second");
        assert!(second.awaited, "second() inside awaited Promise.all is awaited with the batch");
        let fanout = by_callee("fanout");
        assert!(!fanout.conditional);
        assert_eq!(fanout.control_block, None);
        assert_eq!(fanout.lexical_order, 8);

        // returns_value: assigned result used; bare statements discarded.
        assert!(validate.returns_value);
        assert!(!save.returns_value);
        assert!(!persist.returns_value, "bare `await persist(...)` discards the result");
        assert!(!fanout.returns_value);
    }

    #[test]
    fn cfg_evidence_switch_and_loops() {
        let ef = extract(
            "src/app.ts",
            r#"function route(kind: string) {
  switch (kind) {
    case "a": alpha(); break;
    case "b": beta(); break;
  }
  for (const x of items) {
    if (x.ok) probe(x); else drop(x);
  }
  while (running) poll();
}
"#,
        );
        let calls = &ef.calls;
        let by_callee = |name: &str| -> &Call {
            calls
                .iter()
                .find(|c| c.callee == name)
                .unwrap_or_else(|| panic!("call {name}"))
        };
        let alpha = by_callee("alpha");
        assert_eq!(alpha.control_block.as_deref(), Some("switch"));
        assert!(alpha.conditional);
        let probe = by_callee("probe");
        assert_eq!(probe.control_block.as_deref(), Some("if"));
        assert!(probe.inside_loop);
        let drop = by_callee("drop");
        assert_eq!(drop.control_block.as_deref(), Some("else"));
        assert!(drop.inside_loop);
        let poll = by_callee("poll");
        assert_eq!(poll.control_block.as_deref(), Some("while"));
        assert!(poll.inside_loop);
        assert_eq!(poll.lexical_order, 4, "switch x2, for-if, for-else, while");
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
    fn store_refs_drizzle_and_model() {
        let ef = extract(
            "src/data.ts",
            r#"import { db } from "./drizzle";
import { users, logs } from "./schema";
import mongoose from "mongoose";

async function w() {
  await db.insert(users).values({ name: "x" });
  await db.update(logs).set({ level: "info" });
  await db.select();
  await Model.create({ name: "y" });
  await Model.findById("1");
}
"#,
        );
        let sr = |store: &str, op: StoreOp, target: &str, caller: &str| {
            ef.store_refs.iter().any(|s| {
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
        // drizzle: chain verbs inherit the inner op + table argument
        assert!(sr("db", StoreOp::Write, "users", "w"));
        assert!(sr("db", StoreOp::Write, "logs", "w"));
        assert!(sr_none("db", StoreOp::Query, "w")); // db.select()
        // mongoose-style Model receiver
        assert!(sr("Model", StoreOp::Write, "Model", "w"));
        assert!(sr("Model", StoreOp::Query, "Model", "w"));
        assert_eq!(ef.store_refs.len(), 5);
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

    // -----------------------------------------------------------------------
    // Semantic facts (Wave 9)
    // -----------------------------------------------------------------------

    fn has_fact(facts: &[SemanticFact], want: &SemanticFact) -> bool {
        facts.iter().any(|f| f == want)
    }

    #[test]
    fn facts_public_exports() {
        let ef = extract(
            "src/lib.ts",
            r#"export function add(a: number): number { return a; }
function hidden() {}
export class User { name: string = ""; }
export interface Named { name: string; }
export type Alias = string;
export enum Color { Red }
export const LIMIT = 10;
export { add as plus } from "./util";
export * from "./types";
export default DEFAULT_VALUE;
const DEFAULT_VALUE = 1;
export { hidden };
"#,
        );
        let want = |s: &str, k: &str| SemanticFact::PublicExport { symbol: s.into(), kind: k.into() };
        // plain exports with their kinds
        assert!(has_fact(&ef.facts, &want("add", "function")), "{:?}", ef.facts);
        assert!(has_fact(&ef.facts, &want("User", "class")), "{:?}", ef.facts);
        assert!(has_fact(&ef.facts, &want("Named", "interface")), "{:?}", ef.facts);
        assert!(has_fact(&ef.facts, &want("Alias", "type")), "{:?}", ef.facts);
        assert!(has_fact(&ef.facts, &want("Color", "enum")), "{:?}", ef.facts);
        assert!(has_fact(&ef.facts, &want("LIMIT", "const")), "{:?}", ef.facts);
        // re-exports + default export + plain `export { a }`
        assert!(has_fact(&ef.facts, &want("plus", "module")), "{:?}", ef.facts);
        assert!(has_fact(&ef.facts, &want("./types", "module")), "{:?}", ef.facts);
        assert!(has_fact(&ef.facts, &want("DEFAULT_VALUE", "module")), "{:?}", ef.facts);
        assert!(has_fact(&ef.facts, &want("hidden", "module")), "{:?}", ef.facts);
        // nothing for the non-exported helper
        assert!(!has_fact(&ef.facts, &want("hidden", "function")), "{:?}", ef.facts);
    }

    #[test]
    fn facts_nest_annotations_fields_and_module_registrations() {
        let ef = extract(
            "src/app.module.ts",
            r#"import { Controller, Get, Module, Injectable } from "@nestjs/common";

@Injectable()
export class UsersService {
  private readonly base: string = "/users";
  retries = 0;
  constructor(private readonly svc: string) {}
}

@Controller("users")
export class UsersController {
  constructor(private readonly svc: UsersService) {}
  @Get()
  async list(): Promise<string[]> { return []; }
}

@Module({
  controllers: [UsersController],
  providers: [UsersService],
})
export class AppModule {}
"#,
        );
        // annotations
        assert!(has_fact(&ef.facts, &SemanticFact::Annotation { name: "Injectable".into(), target: "UsersService".into() }), "{:?}", ef.facts);
        assert!(has_fact(&ef.facts, &SemanticFact::Annotation { name: "Controller".into(), target: "UsersController".into() }), "{:?}", ef.facts);
        assert!(has_fact(&ef.facts, &SemanticFact::Annotation { name: "Get".into(), target: "UsersController.list".into() }), "{:?}", ef.facts);
        assert!(has_fact(&ef.facts, &SemanticFact::Annotation { name: "Module".into(), target: "AppModule".into() }), "{:?}", ef.facts);
        // fields with mutability
        assert!(has_fact(&ef.facts, &SemanticFact::Field { owner: "UsersService".into(), name: "base".into(), mutable: false }), "{:?}", ef.facts);
        assert!(has_fact(&ef.facts, &SemanticFact::Field { owner: "UsersService".into(), name: "retries".into(), mutable: true }), "{:?}", ef.facts);
        // module registrations
        assert!(has_fact(&ef.facts, &SemanticFact::Registration { owner: "AppModule".into(), kind: "controllers".into(), target: "UsersController".into() }), "{:?}", ef.facts);
        assert!(has_fact(&ef.facts, &SemanticFact::Registration { owner: "AppModule".into(), kind: "providers".into(), target: "UsersService".into() }), "{:?}", ef.facts);
    }

    #[test]
    fn facts_annotations_gated_on_nest_import() {
        // No @nestjs import: decorators are not framework facts.
        let ef = extract(
            "src/plain.ts",
            r#"class Foo {
  @Get()
  list() {}
}
"#,
        );
        assert!(ef.facts.is_empty(), "{:?}", ef.facts);
        // A plain method named get is never a route/registration either.
        let ef2 = extract(
            "src/plain2.ts",
            r#"class Router {
  get(path: string) { return path; }
}
"#,
        );
        assert!(ef2.facts.is_empty(), "{:?}", ef2.facts);
    }

    #[test]
    fn facts_express_registrations() {
        let ef = extract(
            "src/server.ts",
            r#"import express from "express";
const app = express();
app.use(express.json());
app.use("/api", apiRouter);
app.get("/health", healthHandler);
function setup(server: any) {
  server.get("/x", h);
}
"#,
        );
        // module-level: owner falls back to the app receiver (a written symbol)
        assert!(has_fact(&ef.facts, &SemanticFact::Registration { owner: "app".into(), kind: "middleware".into(), target: "express.json".into() }), "{:?}", ef.facts);
        assert!(has_fact(&ef.facts, &SemanticFact::Registration { owner: "app".into(), kind: "middleware".into(), target: "apiRouter".into() }), "{:?}", ef.facts);
        assert!(has_fact(&ef.facts, &SemanticFact::Registration { owner: "app".into(), kind: "route".into(), target: "GET /health".into() }), "{:?}", ef.facts);
        // inside a function the owner is the function
        assert!(has_fact(&ef.facts, &SemanticFact::Registration { owner: "setup".into(), kind: "route".into(), target: "GET /x".into() }), "{:?}", ef.facts);
    }

    #[test]
    fn facts_express_registrations_gated_on_import() {
        // No express import: api.get is not an express registration.
        let ef = extract(
            "src/other.ts",
            r#"const api = { get: (p: string, h: any) => {} };
api.get("/thing", handler);
"#,
        );
        assert!(ef.facts.is_empty(), "{:?}", ef.facts);
    }

    #[test]
    fn facts_configuration_reads() {
        let ef = extract(
            "src/config.ts",
            r#"import express from "express";
const app = express();
const PORT = process.env.PORT;
const NAMED = process.env["API_KEY"];
function read(): string { return process.env.DB_URL; }
class Client {
  static readonly endpoint = process.env.ENDPOINT;
  readSecret() { return process.env.TOKEN; }
}
"#,
        );
        assert!(has_fact(&ef.facts, &SemanticFact::Configuration { owner: "PORT".into(), key: "PORT".into() }), "{:?}", ef.facts);
        assert!(has_fact(&ef.facts, &SemanticFact::Configuration { owner: "NAMED".into(), key: "API_KEY".into() }), "{:?}", ef.facts);
        assert!(has_fact(&ef.facts, &SemanticFact::Configuration { owner: "read".into(), key: "DB_URL".into() }), "{:?}", ef.facts);
        assert!(has_fact(&ef.facts, &SemanticFact::Configuration { owner: "Client".into(), key: "ENDPOINT".into() }), "{:?}", ef.facts);
        assert!(has_fact(&ef.facts, &SemanticFact::Configuration { owner: "Client.readSecret".into(), key: "TOKEN".into() }), "{:?}", ef.facts);
    }

    #[test]
    fn facts_callbacks() {
        let ef = extract(
            "src/app.tsx",
            r#"import { useEffect } from "react";
import { onMount } from "svelte";
function handleClick() {}
function handleAuth() {}
export const App = () => {
  useEffect(handleAuth, []);
  document.addEventListener("click", handleClick);
  return null;
};
window.addEventListener("load", handleLoad);
function handleLoad() {}
"#,
        );
        assert!(has_fact(&ef.facts, &SemanticFact::Callback { owner: "App".into(), callback: "handleAuth".into() }), "{:?}", ef.facts);
        assert!(has_fact(&ef.facts, &SemanticFact::Callback { owner: "App".into(), callback: "handleClick".into() }), "{:?}", ef.facts);
        // module-level window listener: no enclosing symbol -> no fact
        assert!(!has_fact(&ef.facts, &SemanticFact::Callback { owner: "handleLoad".into(), callback: "handleLoad".into() }), "{:?}", ef.facts);
        // svelte onMount (import-verified)
        let ef2 = extract(
            "src/comp.svelte.ts",
            r#"import { onMount } from "svelte";
export function init() {
  onMount(refresh);
}
function refresh() {}
"#,
        );
        assert!(has_fact(&ef2.facts, &SemanticFact::Callback { owner: "init".into(), callback: "refresh".into() }), "{:?}", ef2.facts);
        // no react import -> useEffect is not a framework callback
        let ef3 = extract(
            "src/plain3.ts",
            r#"function App() { useEffect(foo); }
function foo() {}
"#,
        );
        assert!(ef3.facts.is_empty(), "{:?}", ef3.facts);
    }

    #[test]
    fn facts_next_config() {
        let ef = extract(
            "next.config.js",
            r#"const nextConfig = { rewrites: () => [] };
module.exports = nextConfig;
"#,
        );
        // deduped: const object + module.exports assignment are one fact
        let regs: Vec<&SemanticFact> = ef
            .facts
            .iter()
            .filter(|f| matches!(f, SemanticFact::Registration { kind, .. } if kind == "next-config"))
            .collect();
        assert_eq!(regs.len(), 1, "{:?}", ef.facts);
        assert!(has_fact(&ef.facts, &SemanticFact::Registration { owner: "nextConfig".into(), kind: "next-config".into(), target: "next".into() }), "{:?}", ef.facts);
        // a non-next.config file never registers next
        let ef2 = extract(
            "src/other.ts",
            r#"const cfg = { x: 1 };
module.exports = cfg;
"#,
        );
        assert!(ef2.facts.is_empty(), "{:?}", ef2.facts);
    }

    #[test]
    fn facts_deterministic() {
        let src = r#"import { Controller, Get, Module } from "@nestjs/common";
import express from "express";
const app = express();
app.get("/health", h);
export function h() {}
@Controller("x")
export class C {
  @Get()
  m() { return process.env.KEY; }
}
@Module({ controllers: [C] })
export class M {}
"#;
        let a = extract("src/a.ts", src);
        let b = extract("src/a.ts", src);
        assert_eq!(a.facts, b.facts, "deterministic across runs");
        assert!(!a.facts.is_empty());
    }

    #[test]
    fn facts_hostile_input_never_panics() {
        let nasty = "export {";
        let ef = extract("src/broken.ts", nasty);
        assert!(ef.facts.is_empty());
        let ef2 = extract("src/broken2.ts", "@Module({ controllers: [");
        assert!(ef2.facts.is_empty());
        let ef3 = extract("src/broken3.ts", "app.get(\" / broken\nprocess.env.");
        assert!(ef3.facts.is_empty());
    }

    fn regs(ef: &ExtractedFile) -> Vec<(&str, &str, &str)> {
        ef.facts
            .iter()
            .filter_map(|f| match f {
                SemanticFact::Registration { owner, kind, target } => {
                    Some((owner.as_str(), kind.as_str(), target.as_str()))
                }
                _ => None,
            })
            .collect()
    }

    fn fields(ef: &ExtractedFile) -> Vec<(&str, &str, bool)> {
        ef.facts
            .iter()
            .filter_map(|f| match f {
                SemanticFact::Field { owner, name, mutable } => {
                    Some((owner.as_str(), name.as_str(), *mutable))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn facts_module_level_let_is_state() {
        // `let` bindings at module level are mutable STATE owned by the
        // module symbol (file stem); `const` is intent-immutable → skipped.
        let ef = extract(
            "src/client.ts",
            "let timeout = 3000;\nconst VERSION = \"1.0\";\nexport function createClient(cfg: unknown) { return timeout; }\n",
        );
        let fs = fields(&ef);
        assert!(
            fs.contains(&("client", "timeout", true)),
            "module let global missing: {fs:?}"
        );
        assert!(
            !fs.iter().any(|(_, n, _)| *n == "VERSION"),
            "const must not be mutable state: {fs:?}"
        );
        // the module symbol owns the global
        assert!(
            ef.symbols
                .iter()
                .any(|s| s.name == "client" && s.kind == SymbolKind::Module),
            "module symbol missing: {:?}",
            ef.symbols.iter().map(|s| s.name.as_str()).collect::<Vec<_>>()
        );
        // createClient is a module factory
        assert!(
            regs(&ef).contains(&("createClient", "factory", "createClient")),
            "factory registration missing: {:?}",
            regs(&ef)
        );
    }

    #[test]
    fn facts_class_static_factory_and_fluent_builder() {
        let ef = extract(
            "src/pool.ts",
            "export class Pool {\n  static create(opts: unknown): Pool { return new Pool(opts); }\n  static of(x: unknown): Pool { return new Pool(x); }\n  withTimeout(ms: number): this { this.ms = ms; return this; }\n}\n",
        );
        let rs = regs(&ef);
        assert!(
            rs.contains(&("Pool", "factory", "Pool")),
            "static factory missing: {rs:?}"
        );
        assert!(
            rs.contains(&("Pool", "builder", "Pool")),
            "fluent builder missing: {rs:?}"
        );
        // the factory method itself is public surface
        assert!(
            ef.facts.iter().any(|f| matches!(
                f,
                SemanticFact::PublicExport { symbol, kind } if symbol == "Pool.create" && kind == "method"
            )),
            "factory method export missing: {:?}",
            ef.facts
        );
    }

    #[test]
    fn facts_object_literal_factory_namespace() {
        // zod-style `z = { object(...), string(...) }` / axios-style
        // `axios = { create(...) }`.
        let ef = extract(
            "src/z.ts",
            "export const z = {\n  object: (shape: unknown) => new ZodObject(shape),\n  string: () => new ZodString(),\n  name: \"zod\",\n};\n",
        );
        let rs = regs(&ef);
        assert!(
            rs.contains(&("z", "factory", "object")),
            "object factory missing: {rs:?}"
        );
        assert!(
            rs.contains(&("z", "factory", "string")),
            "string factory missing: {rs:?}"
        );
        assert!(
            !rs.iter().any(|(_, _, t)| *t == "name"),
            "non-function property must not be a factory: {rs:?}"
        );
    }

    // -------------------------------------------------------------------
    // Wave 11: zod schema contracts + reactive state + queue consumers
    // -------------------------------------------------------------------

    fn schemas(ef: &ExtractedFile) -> Vec<(&str, &str, &str)> {
        ef.facts
            .iter()
            .filter_map(|f| match f {
                SemanticFact::SchemaDefinition {   owner, name, .. }=> {
                    Some(("def", owner.as_str(), name.as_str()))
                }
                SemanticFact::SchemaComposition {   owner, name: _, parent, .. }=> {
                    Some(("compose", owner.as_str(), parent.as_str()))
                }
                SemanticFact::SchemaValidation {   owner, target, .. }=> {
                    Some(("validate", owner.as_str(), target.as_str()))
                }
                _ => None,
            })
            .collect()
    }

    fn reactives(ef: &ExtractedFile) -> Vec<(&str, &str, &str)> {
        ef.facts
            .iter()
            .filter_map(|f| match f {
                SemanticFact::ReactiveState {   owner, name, access, .. }=> {
                    Some((owner.as_str(), name.as_str(), access.as_str()))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn wave11_zod_schema_definition_validation_composition() {
        let ef = extract(
            "src/schema.ts",
            "import { z } from \"zod\";\n\nexport const UserSchema = z.object({\n  id: z.number(),\n  name: z.string(),\n});\n\nexport const AdminSchema = UserSchema.extend({ role: z.string() });\n\nfunction validate(data: unknown): void {\n  UserSchema.parse(data);\n  AdminSchema.safeParse(data);\n}\n",
        );
        let sc = schemas(&ef);
        assert!(
            sc.contains(&("def", "UserSchema", "UserSchema")),
            "zod const must be a SchemaDefinition: {sc:?}"
        );
        assert!(
            sc.contains(&("compose", "AdminSchema", "UserSchema")),
            "extend of a local schema must be a SchemaComposition: {sc:?}"
        );
        assert!(
            sc.contains(&("validate", "validate", "UserSchema")),
            "parse call must emit SchemaValidation: {sc:?}"
        );
        assert!(
            sc.contains(&("validate", "validate", "AdminSchema")),
            "safeParse call must emit SchemaValidation: {sc:?}"
        );
        // dedupe: two validation calls on the same schema → one fact
        let ef2 = extract(
            "src/schema2.ts",
            "import { z } from \"zod\";\n\nexport const S = z.object({ a: z.string() });\n\nfunction f(data: unknown) {\n  S.parse(data);\n  S.parse(data);\n}\n",
        );
        let validates: Vec<&SemanticFact> = ef2
            .facts
            .iter()
            .filter(|f| matches!(f, SemanticFact::SchemaValidation { .. }))
            .collect();
        assert_eq!(validates.len(), 1, "duplicate validation facts: {validates:?}");
        // Wave 12: facts carry the defining expression (the concrete code
        // form), so the atlas can render `schema: S = z.object({ a: z.string() })`.
        let def_expr: Vec<&str> = ef2
            .facts
            .iter()
            .filter_map(|f| match f {
                SemanticFact::SchemaDefinition { expr, .. } => Some(expr.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            def_expr.contains(&"z.object({ a: z.string() })"),
            "definition expr must be the z.object call: {def_expr:?}"
        );
        let val_expr: Vec<&str> = ef2
            .facts
            .iter()
            .filter_map(|f| match f {
                SemanticFact::SchemaValidation { expr, .. } => Some(expr.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            val_expr.contains(&"S.parse(data)"),
            "validation expr must be the parse call: {val_expr:?}"
        );
    }

    #[test]
    fn wave11_zod_requires_import() {
        // `.object`/`.parse` without the zod import is not a schema.
        let ef = extract(
            "src/nozod.ts",
            "export const S = z.object({ a: z.string() });\n\nfunction f(data: unknown) {\n  S.parse(data);\n}\n",
        );
        assert!(
            !ef.facts.iter().any(|f| matches!(
                f,
                SemanticFact::SchemaDefinition { .. }
                    | SemanticFact::SchemaComposition { .. }
                    | SemanticFact::SchemaValidation { .. }
            )),
            "no zod import → no schema facts: {:?}",
            ef.facts
        );
    }

    #[test]
    fn wave11_reactive_state_react_vue_svelte_mobx() {
        // React: useState declares state; assignments/reads within the
        // component function.
        let ef = extract(
            "src/Count.tsx",
            "import React, { useState } from \"react\";\n\nexport function Counter() {\n  const [count, setCount] = useState(0);\n  count + 1;\n  setCount(count + 1);\n  return count;\n}\n",
        );
        let rs = reactives(&ef);
        assert!(
            rs.contains(&("Counter", "count", "state")),
            "useState declaration missing: {rs:?}"
        );
        assert!(
            rs.contains(&("Counter", "count", "read")),
            "useState read missing: {rs:?}"
        );
        // Vue: ref declares state; computed derives.
        let ef2 = extract(
            "src/store.ts",
            "import { ref, computed } from \"vue\";\n\nexport const count = ref(0);\nexport const double = computed(() => count.value * 2);\n\nexport function bump() {\n  count.value += 1;\n}\n",
        );
        let rs2 = reactives(&ef2);
        assert!(
            rs2.contains(&("store", "count", "state")),
            "vue ref declaration missing: {rs2:?}"
        );
        assert!(
            rs2.contains(&("store", "double", "derive")),
            "vue computed derivation missing: {rs2:?}"
        );
        // Svelte: $state declares; $derived derives; $props reads.
        let ef3 = extract(
            "src/state.svelte.ts",
            "import { $state, $derived } from \"svelte\";\n\nexport function makeStore() {\n  let count = $state(0);\n  let double = $derived(count * 2);\n  count = 5;\n  return double;\n}\n",
        );
        let rs3 = reactives(&ef3);
        assert!(
            rs3.contains(&("makeStore", "count", "state")),
            "svelte $state declaration missing: {rs3:?}"
        );
        assert!(
            rs3.contains(&("makeStore", "double", "derive")),
            "svelte $derived missing: {rs3:?}"
        );
        assert!(
            rs3.contains(&("makeStore", "count", "write")),
            "svelte assignment write missing: {rs3:?}"
        );
        // Mobx: observable declares; action writes; computed derives.
        let ef4 = extract(
            "src/mob.ts",
            "import { observable, action, computed } from \"mobx\";\n\nconst store = observable({ n: 1 });\nconst double = computed(() => store.n * 2);\nfunction inc() {\n  action(() => { store.n += 1; })();\n}\n",
        );
        let rs4 = reactives(&ef4);
        assert!(
            rs4.contains(&("mob", "store", "state")),
            "mobx observable missing: {rs4:?}"
        );
        assert!(
            rs4.contains(&("mob", "double", "derive")),
            "mobx computed missing: {rs4:?}"
        );
        // no framework import → no reactive facts
        let ef5 = extract(
            "src/plain.ts",
            "export function f() {\n  const x = useState(0);\n  return x;\n}\n",
        );
        assert!(
            !ef5
                .facts
                .iter()
                .any(|f| matches!(f, SemanticFact::ReactiveState { .. })),
            "no framework import → no reactive facts: {:?}",
            ef5.facts
        );
    }

    #[test]
    fn wave11_queue_consumers() {
        // bullmq Worker subscribes to a queue.
        let ef = extract(
            "src/worker.ts",
            "import { Worker } from \"bullmq\";\n\nnew Worker(\"emails\", async (job) => {\n  return job.data;\n});\n",
        );
        assert!(
            ef.store_refs.iter().any(|sr| {
                sr.op == StoreOp::Subscribe
                    && sr.store == "bullmq"
                    && sr.target.as_deref() == Some("emails")
            }),
            "bullmq worker missing: {:?}",
            ef.store_refs
        );
        // amqplib channel.consume subscribes to a queue.
        let ef2 = extract(
            "src/amqp.ts",
            "import amqp from \"amqplib\";\n\nasync function start() {\n  const conn = await amqp.connect(\"amqp://localhost\");\n  const ch = await conn.createChannel();\n  await ch.consume(\"jobs\", (msg) => {});\n}\n",
        );
        assert!(
            ef2.store_refs.iter().any(|sr| {
                sr.op == StoreOp::Subscribe
                    && sr.store == "amqp"
                    && sr.target.as_deref() == Some("jobs")
            }),
            "amqplib consume missing: {:?}",
            ef2.store_refs
        );
        // no queue import → nothing
        let ef3 = extract(
            "src/plain.ts",
            "const w = new Worker(\"x\");\nch.consume(\"y\", () => {});\n",
        );
        assert!(
            ef3.store_refs.is_empty(),
            "no queue import → no consumers: {:?}",
            ef3.store_refs
        );
    }
}

