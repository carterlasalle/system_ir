//! C and C++ language extractors (tree-sitter based).
//!
//! Pure, deterministic extraction: `(path, content) -> ExtractedFile`.
//! Syntax-level only; cross-file resolution happens in `resolve.rs`.
//! Quote `#include "foo.h"` and angle `#include <stdio.h>` are captured
//! as imports; Step-A file identity is path-precise (includer dir join),
//! never basename-guessed. Direct includes only — no transitive closure.

use crate::model::{
    Call, Entrypoint, ExtractedFile, Import, ImportType, LanguageExtractor, SemanticFact,
    SourceFile, Symbol, SymbolKind,
};
use tree_sitter::{Node, Parser};

// trace:v1 id=impl.scc.extract.cfamily work=WORK-phase-30-of-scc-x-ripwire-lessons-absorb-c-and-c-extractors-with-path satisfies=REQ-implement-phase-30-of-scc-x-ripwire-lessons-absorb-c-and-c-extracto implements=PLAN-phase-30-of-scc-x-ripwire-lessons-absorb-c-and-c-extractors-with-path
/// Shared walker over tree-sitter-c or tree-sitter-cpp.
pub struct CFamilyExtractor {
    language: tree_sitter::Language,
    id: &'static str,
}

// trace:exempt reason=internal-detail
impl CFamilyExtractor {
    // trace:exempt reason=internal-detail
    pub fn c() -> Self {
        CFamilyExtractor {
            language: tree_sitter_c::LANGUAGE.into(),
            id: "c",
        }
    }

    // trace:exempt reason=internal-detail
    pub fn cpp() -> Self {
        CFamilyExtractor {
            language: tree_sitter_cpp::LANGUAGE.into(),
            id: "cpp",
        }
    }
}

// trace:exempt reason=internal-detail
impl LanguageExtractor for CFamilyExtractor {
    // trace:exempt reason=internal-detail
    fn language(&self) -> &'static str {
        self.id
    }

    // trace:exempt reason=internal-detail
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

    #[derive(Default)]
    // trace:exempt reason=internal-detail
    struct Ctx {
    symbols: Vec<Symbol>,
    imports: Vec<Import>,
    calls: Vec<Call>,
    entrypoints: Vec<Entrypoint>,
    facts: Vec<SemanticFact>,
    scopes: Vec<Scope>,
    call_seq: std::collections::HashMap<Option<String>, u32>,
}

// trace:exempt reason=internal-detail
struct Scope {
    name: String,
    is_class: bool,
}

// trace:exempt reason=internal-detail
impl Ctx {
    // trace:exempt reason=internal-detail
    fn caller(&self) -> Option<String> {
        self.scopes.last().map(|s| s.name.clone())
    }

    // trace:exempt reason=internal-detail
    fn top_is_class(&self) -> bool {
        self.scopes.last().map(|s| s.is_class).unwrap_or(false)
    }

    // trace:exempt reason=internal-detail
    fn top_name(&self) -> String {
        self.scopes
            .last()
            .map(|s| s.name.clone())
            .unwrap_or_default()
    }

    // trace:exempt reason=internal-detail
    fn into_extracted(self) -> ExtractedFile {
        ExtractedFile {
            symbols: self.symbols,
            imports: self.imports,
            calls: self.calls,
            entrypoints: self.entrypoints,
            facts: self.facts,
            ..Default::default()
        }
    }
}

// trace:exempt reason=internal-detail
impl CFamilyExtractor {
    // trace:exempt reason=internal-detail
    fn walk(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        match node.kind() {
            "function_definition" => self.walk_function(node, ctx, src),
            "declaration" | "field_declaration" => {
                if is_named_function_declarator(node.child_by_field_name("declarator")) {
                    self.walk_function(node, ctx, src);
                } else {
                    self.walk_children(node, ctx, src);
                }
            }
            "class_specifier" | "struct_specifier" => {
                if self.id == "cpp" {
                    self.walk_class(node, ctx, src);
                } else {
                    self.walk_children(node, ctx, src);
                }
            }
            "preproc_include" => {
                self.record_include(node, ctx, src);
                self.walk_children(node, ctx, src);
            }
            "call_expression" => self.record_call(node, ctx, src),
            _ => self.walk_children(node, ctx, src),
        }
    }

    // trace:exempt reason=internal-detail
    fn walk_children(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, ctx, src);
        }
    }

    // trace:exempt reason=internal-detail
    fn walk_class(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let name = clean(node_text(node.child_by_field_name("name"), src));
        if name.is_empty() {
            self.walk_children(node, ctx, src);
            return;
        }
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;
        ctx.symbols.push(Symbol {
            name: name.clone(),
            kind: SymbolKind::Class,
            signature: None,
            decl_header: Some(truncate_chars(&collapse(node_text(Some(node), src)), 200)),
            start_line,
            end_line,
            exported: true,
            docstring: None,
            parent: None,
        });
        ctx.scopes.push(Scope {
            name,
            is_class: true,
        });
        self.walk_children(node, ctx, src);
        ctx.scopes.pop();
    }

    // trace:exempt reason=internal-detail
    fn walk_function(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let declarator = node.child_by_field_name("declarator");
        let raw = declarator
            .map(|d| declarator_name(d, src))
            .unwrap_or_default();
        if raw.is_empty() {
            self.walk_children(node, ctx, src);
            return;
        }
        let in_class = ctx.top_is_class();
        let (sym_name, kind, parent, plain) = if in_class {
            let class = ctx.top_name();
            let plain = raw.rsplit('.').next().unwrap_or(&raw).to_string();
            (
                format!("{class}.{plain}"),
                SymbolKind::Method,
                Some(class),
                plain,
            )
        } else if let Some((cls, method)) = raw.split_once('.') {
            if !cls.is_empty() && !method.is_empty() {
                (
                    raw.clone(),
                    SymbolKind::Method,
                    Some(cls.to_string()),
                    method.to_string(),
                )
            } else {
                (raw.clone(), SymbolKind::Function, None, raw.clone())
            }
        } else {
            (raw.clone(), SymbolKind::Function, None, raw.clone())
        };
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;
        let exported = !has_static(node, src);
        let header = decl_header(node, src);
        ctx.symbols.push(Symbol {
            name: sym_name.clone(),
            kind,
            signature: header.clone(),
            decl_header: header,
            start_line,
            end_line,
            exported,
            docstring: None,
            parent,
        });
        if exported {
            ctx.facts.push(SemanticFact::PublicExport {
                symbol: sym_name.clone(),
                kind: if kind == SymbolKind::Method {
                    "method".to_string()
                } else {
                    "function".to_string()
                },
            });
        }
        if plain == "main" && ctx.scopes.is_empty() && node.child_by_field_name("body").is_some() {
            ctx.entrypoints.push(Entrypoint {
                symbol: "main".to_string(),
                kind: "bin".to_string(),
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

    // trace:exempt reason=internal-detail
    fn record_include(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        let path_node = node.child_by_field_name("path");
        let raw = clean(node_text(path_node, src));
        if raw.is_empty() {
            return;
        }
        let line = node.start_position().row as u32 + 1;
        let module = if raw.starts_with('<') && raw.ends_with('>') {
            raw
        } else {
            strip_c_quotes(&raw).unwrap_or(raw)
        };
        if module.is_empty() {
            return;
        }
        ctx.imports.push(Import {
            module,
            names: Vec::new(),
            line,
            r#type: ImportType::Module,
        });
    }

    // trace:exempt reason=internal-detail
    fn record_call(&self, node: Node, ctx: &mut Ctx, src: &[u8]) {
        if let Some(fn_node) = node.child_by_field_name("function") {
            let callee = collapse(node_text(Some(fn_node), src)).replace("::", ".");
            if !callee.is_empty() {
                let root = callee_root(fn_node);
                let known_receiver = matches!(
                    root.kind(),
                    "identifier" | "field_identifier" | "this" | "type_identifier"
                );
                let caller = ctx.caller();
                let lexical_order = {
                    let seq = ctx.call_seq.entry(caller.clone()).or_insert(0);
                    *seq += 1;
                    *seq - 1
                };
                ctx.calls.push(
                    Call {
                        caller,
                        callee,
                        line: node.start_position().row as u32 + 1,
                        known_receiver,
                        lexical_order,
                        ..Default::default()
                    }
                    .finish(),
                );
            }
        }
        self.walk_children(node, ctx, src);
    }
}

// trace:exempt reason=internal-detail
fn is_named_function_declarator(node: Option<Node>) -> bool {
    let Some(node) = node else {
        return false;
    };
    if node.kind() != "function_declarator" {
        return false;
    }
    matches!(
        node.child_by_field_name("declarator").map(|n| n.kind()),
        Some(
            "identifier"
                | "field_identifier"
                | "qualified_identifier"
                | "qualified_name"
                | "destructor_name"
        )
    )
}

// trace:exempt reason=internal-detail
fn node_text<'a>(node: Option<Node<'a>>, src: &'a [u8]) -> &'a str {
    match node {
        Some(n) => n.utf8_text(src).unwrap_or(""),
        None => "",
    }
}

// trace:exempt reason=internal-detail
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

// trace:exempt reason=internal-detail
fn clean(s: &str) -> String {
    collapse(s.trim())
}

// trace:exempt reason=internal-detail
fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

// trace:exempt reason=internal-detail
fn strip_c_quotes(raw: &str) -> Option<String> {
    let s = raw.trim();
    let bytes = s.as_bytes();
    if bytes.len() >= 2 && bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"' {
        return Some(s[1..s.len() - 1].to_string());
    }
    None
}

// trace:exempt reason=internal-detail
fn has_static(node: Node, src: &[u8]) -> bool {
    let mut c = node.walk();
    let found = node.children(&mut c).any(|ch| {
        ch.kind() == "storage_class_specifier" && node_text(Some(ch), src).contains("static")
    });
    found
}

// trace:exempt reason=internal-detail
fn decl_header(node: Node, src: &[u8]) -> Option<String> {
    let text = collapse(node_text(Some(node), src));
    if text.is_empty() {
        return None;
    }
    let cut = text.find('{').unwrap_or(text.len());
    Some(truncate_chars(text[..cut].trim(), 200))
}

// trace:exempt reason=internal-detail
fn declarator_name(node: Node, src: &[u8]) -> String {
    match node.kind() {
        "identifier" | "field_identifier" | "type_identifier" => clean(node_text(Some(node), src)),
        "destructor_name" => {
            let inner = node
                .named_child(0)
                .map(|n| clean(node_text(Some(n), src)))
                .unwrap_or_default();
            if inner.is_empty() {
                clean(node_text(Some(node), src)).replace('~', "")
            } else {
                inner
            }
        }
        "qualified_identifier" | "qualified_name" | "scope_resolution" => {
            collapse(node_text(Some(node), src)).replace("::", ".")
        }
        _ => {
            if let Some(d) = node.child_by_field_name("declarator") {
                return declarator_name(d, src);
            }
            if let Some(n) = node.child_by_field_name("name") {
                return declarator_name(n, src);
            }
            let mut c = node.walk();
            for ch in node.named_children(&mut c) {
                let n = declarator_name(ch, src);
                if !n.is_empty() {
                    return n;
                }
            }
            String::new()
        }
    }
}

// trace:exempt reason=internal-detail
fn callee_root(mut node: Node) -> Node {
    loop {
        match node.kind() {
            "field_expression" | "pointer_expression" => {
                match node
                    .child_by_field_name("argument")
                    .or_else(|| node.child_by_field_name("object"))
                {
                    Some(inner) => node = inner,
                    None => return node,
                }
            }
            "parenthesized_expression" => match node.named_child(0) {
                Some(inner) => node = inner,
                None => return node,
            },
            _ => return node,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // trace:exempt reason=internal-detail
    fn extract_c(src: &str) -> ExtractedFile {
        CFamilyExtractor::c().extract(&SourceFile::new("main.c", src))
    }

    // trace:exempt reason=internal-detail
    fn extract_cpp(src: &str) -> ExtractedFile {
        CFamilyExtractor::cpp().extract(&SourceFile::new("main.cpp", src))
    }

    // trace:exempt reason=internal-detail
    fn find_symbol<'a>(ef: &'a ExtractedFile, name: &str) -> &'a Symbol {
        ef.symbols
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("symbol {name} not found in {:?}", ef.symbols))
    }

    #[test]
    // trace:v1 id=test.scc.extract.c verifies=REQ-implement-phase-30-of-scc-x-ripwire-lessons-absorb-c-and-c-extracto exercises=impl.scc.extract.cfamily
    fn c_functions_calls_quote_and_angle_includes() {
        let ef = extract_c(
            "#include \"util.h\"\n#include <stdio.h>\n\nint helper(void);\n\nint main(void) {\n    helper();\n    return 0;\n}\n",
        );
        let main = find_symbol(&ef, "main");
        assert_eq!(main.kind, SymbolKind::Function);
        assert!(ef.entrypoints.iter().any(|e| e.symbol == "main"));
        let mods: Vec<&str> = ef.imports.iter().map(|i| i.module.as_str()).collect();
        assert!(mods.contains(&"util.h"), "{mods:?}");
        assert!(mods.contains(&"<stdio.h>"), "{mods:?}");
        assert!(
            ef.calls.iter().any(|c| c.callee == "helper"),
            "calls: {:?}",
            ef.calls
        );
        let proto = extract_c("int helper(void);\n");
        assert_eq!(find_symbol(&proto, "helper").kind, SymbolKind::Function);
    }

    #[test]
    // trace:v1 id=test.scc.extract.cpp verifies=REQ-implement-phase-30-of-scc-x-ripwire-lessons-absorb-c-and-c-extracto exercises=impl.scc.extract.cfamily
    fn cpp_class_method_and_this_call() {
        let ef = extract_cpp(
            "class Foo {\n public:\n  void bar() {}\n  void run() { this->bar(); }\n};\n",
        );
        assert_eq!(find_symbol(&ef, "Foo").kind, SymbolKind::Class);
        let bar = find_symbol(&ef, "Foo.bar");
        assert_eq!(bar.kind, SymbolKind::Method);
        assert_eq!(bar.parent.as_deref(), Some("Foo"));
        assert!(
            ef.calls.iter().any(|c| c.callee.contains("bar")),
            "calls: {:?}",
            ef.calls
        );
    }

    #[test]
    // trace:exempt reason=internal-detail
    fn malformed_input_does_not_panic() {
        let _ = extract_c("int main( { ;;; \x00");
        let _ = extract_cpp("class { void ( {");
    }
}
