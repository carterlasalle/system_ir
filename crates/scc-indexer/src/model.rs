//! Extraction model: the contract between language extractors and the
//! Reality Compiler.
//!
//! Extractors are pure functions: `(path, content) -> ExtractedFile`. They
//! perform syntax-level extraction only. Cross-file resolution happens later
//! in `resolve.rs`.

use scc_core::{RecvKind, ReferenceKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A source file handed to an extractor.
#[derive(Debug, Clone)]
// trace:exempt reason=internal-detail
pub struct SourceFile {
    /// Repository-relative path, `/`-separated.
    pub path: String,
    pub content: String,
}

impl SourceFile {
    pub fn new(path: impl Into<String>, content: impl Into<String>) -> Self {
        SourceFile {
            path: path.into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
// trace:exempt reason=internal-detail
// trace:v1 id=impl.scc.model work=WORK-SCC-014 satisfies=REQ-SCC-IR
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Interface,
    Type,
    Const,
    Enum,
    Module,
}

impl SymbolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Class => "class",
            SymbolKind::Interface => "interface",
            SymbolKind::Type => "type",
            SymbolKind::Const => "const",
            SymbolKind::Enum => "enum",
            SymbolKind::Module => "module",
        }
    }

    pub fn is_callable(&self) -> bool {
        matches!(
            self,
            SymbolKind::Function | SymbolKind::Method | SymbolKind::Class | SymbolKind::Const
        )
    }
}

/// A declared symbol (function, class, interface, type, const...).
#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    /// One-line signature, e.g. `def normalize(text: str) -> str`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// Exact declaration header as written in source: the byte span from
    /// the declaration keyword (`def` / `pub fn` / `func` / `public ...` /
    /// `class` ...) through the end of the header (parameter list closing
    /// `)`, return type, `where` clause, `throws` clause, or heritage
    /// clause). Multi-line preserved byte-for-byte; NOT truncated and NOT
    /// reconstructed. `None` when the extractor does not capture it (const,
    /// module, aliases).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decl_header: Option<String>,
    /// 1-based inclusive line range.
    pub start_line: u32,
    pub end_line: u32,
    pub exported: bool,
    /// Docstring / leading JSDoc comment, first paragraph only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docstring: Option<String>,
    /// For methods: `ClassName`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

/// An import statement with the names it binds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Import {
    /// Module specifier as written (e.g. `./services/asr`, `fastapi`, `@app/foo`).
    pub module: String,
    /// Bound names: `(local_name, imported_or_alias)`.
    /// For `import { a as b }`: `[("b", "a")]`. For `import x from "m"`:
    /// `[("x", "default")]`. For `import * as ns`: `[("ns", "*")]`. For
    /// `import m` / `import x, { y }`: `[("x", "default"), ("y", "y")]`.
    /// For Python `from m import a as b`: `[("b", "a")]`; `import m`:
    /// `[("m", "m")]`.
    pub names: Vec<(String, String)>,
    pub line: u32,
    pub r#type: ImportType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportType {
    /// `import m` / `import * as m` — the module itself is bound.
    Module,
    /// `import { a } from` / `from m import a` — members bound.
    Member,
}

/// A call site with its enclosing symbol.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Call {
    /// Enclosing symbol name (function/method) or `None` for module level.
    pub caller: Option<String>,
    /// Callee expression as written, e.g. `normalize`, `client.execute`,
    /// `self.resolve`, `db.query`.
    pub callee: String,
    pub line: u32,
    /// Whether the callee root is a local/imported binding or something
    /// unknown (e.g. an arbitrary member on a parameter).
    pub known_receiver: bool,
    /// Receiver shape recovered from the callee expression (or stamped by
    /// the extractor). Default `Unknown` is filled by [`Call::finish`].
    #[serde(default)]
    pub recv: RecvKind,
    /// Last identifier in a `root.field.method` chain when classified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualifier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recv_var: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_name: Option<String>,
    /// CALL by default. Imports/types/macros must not use this.
    #[serde(default)]
    pub role: ReferenceKind,
    /// Whether the call sits inside a conditional/loop/try body (if/else/
    /// for/while/try/with/match) within its enclosing function — the ONLY
    /// evidence that turns call fanout into control-flow branching.
    #[serde(default)]
    pub conditional: bool,
    /// 0-based index of this call site within its enclosing function, in
    /// source order (a per-function counter). Deterministic CFG evidence:
    /// the FlowGraph compiler orders Next edges by this value within a
    /// caller, so sequential causality matches the code as written.
    #[serde(default)]
    pub lexical_order: u32,
    /// Nearest enclosing control-flow block kind: `if`/`else`/`for`/
    /// `while`/`try`/`catch`/`match`/`switch`/`with`/`do`/`loop`/
    /// `finally`/`select` (language-dependent). `None` when the call sits
    /// in straight-line code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_block: Option<String>,
    /// The call sits inside a loop body (for/while/do/loop), possibly
    /// nested under other blocks.
    #[serde(default)]
    pub inside_loop: bool,
    /// The call sits inside a try/except (catch) body, possibly nested.
    #[serde(default)]
    pub inside_try: bool,
    /// The call is awaited/spawned at this site: python `await`, ts
    /// `await` / `Promise.all`, rust `.await`, go `go` statement. Java has
    /// no syntactic await — always false there.
    #[serde(default)]
    pub awaited: bool,
    /// The call's result is consumed (assigned/returned/compared/passed)
    /// rather than discarded as a bare expression statement.
    #[serde(default)]
    pub returns_value: bool,
}

impl Call {
    /// Fill receiver classification from the callee string when the
    /// extractor did not stamp a more precise `recv`.
    pub fn finish(mut self) -> Self {
        let fact = crate::recv::classify_callee(&self.callee);
        if self.recv == RecvKind::Unknown {
            self.recv = fact.recv;
        }
        if self.qualifier.is_none() {
            self.qualifier = fact.qualifier;
        }
        if self.recv_var.is_none() {
            self.recv_var = fact.recv_var;
        }
        if self.field_name.is_none() {
            self.field_name = fact.field_name;
        }
        if self.role == ReferenceKind::Call && fact.role != ReferenceKind::Call {
            self.role = fact.role;
        }
        self
    }
}

/// An HTTP route declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    pub method: String,
    pub path: String,
    /// Handler symbol name if statically identifiable.
    pub handler: Option<String>,
    pub line: u32,
    pub framework: String,
}

/// A test symbol or suite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Test {
    pub name: String,
    /// Enclosing symbol for the test (function or class).
    pub symbol: Option<String>,
    pub kind: TestKind,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TestKind {
    Unit,
    Integration,
}

/// A store (db/queue/cache) access.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreRef {
    /// Enclosing symbol.
    pub caller: Option<String>,
    /// Store client variable name (e.g. `db`, `redis`, `kafka`).
    pub store: String,
    /// Storage technology hint when identifiable (e.g. `postgres`, `redis`,
    /// `kafka`, `mongo`, `sqlite`, `s3`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub technology: Option<String>,
    pub op: StoreOp,
    /// Entity-ish name involved, e.g. table name from SQL, model name from
    /// `prisma.user.create`, topic for publish/subscribe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
// trace:exempt reason=internal-detail
pub enum StoreOp {
    Read,
    Write,
    Query,
    Publish,
    Subscribe,
    Migrate,
}

impl StoreOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            StoreOp::Read => "read",
            StoreOp::Write => "write",
            StoreOp::Query => "query",
            StoreOp::Publish => "publish",
            StoreOp::Subscribe => "subscribe",
            StoreOp::Migrate => "migrate",
        }
    }
}

/// Retry/backoff policy decoration on a symbol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Retry {
    pub symbol: String,
    /// Policy description, e.g. `tenacity.retry` / `bounded-backoff`.
    pub policy: String,
    pub line: u32,
}

/// A program entrypoint (main guard, CLI command).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entrypoint {
    pub symbol: String,
    /// `main-guard`, `cli`, `bin`
    pub kind: String,
    pub line: u32,
}

/// One semantic fact (Wave 9): a first-class representation beyond
/// symbols/calls/routes. Every fact carries its owning symbol so the
/// writer can attach store evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SemanticFact {
    /// A public API surface element: `(symbol, export_kind)` where
    /// export_kind is class/function/trait/interface/macro/type/module.
    PublicExport { symbol: String, kind: String },
    /// A decorator/annotation attached to `target` (e.g. @app.get,
    /// @Controller, #[derive(...)]).
    Annotation { name: String, target: String },
    /// A class/struct field (state surface).
    Field {
        owner: String,
        name: String,
        mutable: bool,
    },
    /// A framework registration: route/middleware/plugin/DI/event
    /// registration performed by `owner` naming `target` with a `kind`.
    Registration {
        owner: String,
        kind: String,
        target: String,
    },
    /// Configuration ownership: `owner` reads/writes config `key`.
    Configuration { owner: String, key: String },
    /// A callback/hook handled by `owner` (framework invokes it).
    Callback { owner: String, callback: String },
    /// A structured schema/model definition (zod z.object, pydantic
    /// BaseModel, JSON Schema, serde model, go struct tags, Java
    /// validation annotations): `owner` defines schema `name`.
    /// `expr` is the defining source expression when available
    /// (e.g. `z.object({ name: z.string() })`), else empty.
    SchemaDefinition {
        owner: String,
        name: String,
        expr: String,
    },
    /// Schema composition: `owner` composes schema `name` from `parent`
    /// (zod .extend/.merge, pydantic inheritance, serde flatten).
    SchemaComposition {
        owner: String,
        name: String,
        parent: String,
        expr: String,
    },
    /// Schema validation surface: `owner` validates `target` against a
    /// schema (zod .parse/.safeParse, pydantic validators, javax
    /// validation annotations). `expr` is the call expression when
    /// available (e.g. `schema.parse(data)`), else empty.
    SchemaValidation {
        owner: String,
        target: String,
        expr: String,
    },
    /// Reactive state ownership: `owner` declares reactive state `name`
    /// with access `state|read|write|derive` (svelte $state, vue
    /// ref/reactive, react useState/useReducer/context, mobx observable,
    /// signals). `expr` is the declaration expression when available
    /// (e.g. `useState(0)`), else empty.
    ReactiveState {
        owner: String,
        name: String,
        access: String,
        expr: String,
    },
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtractedFile {
    pub symbols: Vec<Symbol>,
    pub imports: Vec<Import>,
    pub calls: Vec<Call>,
    pub routes: Vec<Route>,
    pub tests: Vec<Test>,
    pub store_refs: Vec<StoreRef>,
    pub retries: Vec<Retry>,
    pub entrypoints: Vec<Entrypoint>,
    /// CLI flags owned by a symbol (argparse/click/clap/cobra), keyed by
    /// symbol name. Values are `-`/`--`-prefixed, sorted, deduped.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub cli_flags: BTreeMap<String, Vec<String>>,
    /// Semantic facts (public exports, annotations, fields, registrations,
    /// configuration, callbacks) — additive to the classic extraction.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<SemanticFact>,
    /// Per-scope local and class-field type binds (`x = Order()`, `x: Foo`,
    /// typed params, `self.x = Order()`, class `x: Foo`). Extract-time only;
    /// never stamped onto FILE entity attributes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_binds: Vec<TypeBind>,
    /// Per-scope function-alias binds (`f = helper`). Extract-time only;
    /// never stamped onto FILE entity attributes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fn_binds: Vec<FnBind>,
    /// Simple-ident bases for class-like symbols in this file
    /// (`IERS_B` → `["IERS"]`). Persisted on CLASS entity attributes, never
    /// on FILE entities (incremental≡cold).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub class_bases: Vec<(String, Vec<String>)>,
}

/// One local / field → type name fact used by one-hop NamedVariable and
/// one-hop `self`/`this` field-type narrowing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeBind {
    /// Enclosing function/method name, or class name for field binds;
    /// empty at module scope.
    #[serde(default)]
    pub scope: String,
    /// Local variable, parameter, or class field name.
    pub name: String,
    /// Type name as written (import alias or local class).
    pub type_name: String,
    pub line: u32,
}

/// One local / file-scope var → function ident used by bare `f()` pinning.
/// Empty `target` is a lambda/clobber tombstone (binding exists; no pin).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FnBind {
    /// Enclosing function/method name; empty at module / file scope.
    #[serde(default)]
    pub scope: String,
    /// Local or file-scope variable name.
    pub name: String,
    /// Bound function ident; empty = lambda/clobber tombstone.
    #[serde(default)]
    pub target: String,
    pub line: u32,
}

/// RHS shape for extract-time function-alias binds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FnRhs {
    Ident(String),
    Lambda,
    Other,
}

fn fn_bind_name_ok(name: &str) -> bool {
    !name.is_empty() && !matches!(name, "_" | "self" | "cls" | "this")
}

/// Record a function-alias bind. Ident copies a unique function name;
/// lambda always tombstones; other RHS clobbers only when a bind already
/// exists. Type-copy ident RHS is not a fn bind but clobbers a prior one.
pub fn record_fn_rhs(
    binds: &mut Vec<FnBind>,
    scope: String,
    name: String,
    rhs: FnRhs,
    typed_copy: bool,
    line: u32,
) {
    if !fn_bind_name_ok(&name) {
        return;
    }
    match rhs {
        FnRhs::Ident(target) if !target.is_empty() && !typed_copy => {
            binds.push(FnBind {
                scope,
                name,
                target,
                line,
            });
        }
        FnRhs::Lambda => {
            binds.push(FnBind {
                scope,
                name,
                target: String::new(),
                line,
            });
        }
        FnRhs::Ident(_) | FnRhs::Other => {
            if binds.iter().any(|b| b.scope == scope && b.name == name) {
                binds.push(FnBind {
                    scope,
                    name,
                    target: String::new(),
                    line,
                });
            }
        }
    }
}

/// Sort function-alias binds for deterministic extract output.
pub fn normalize_fn_binds(mut binds: Vec<FnBind>) -> Vec<FnBind> {
    binds.sort_by(|a, b| {
        (&a.scope, &a.name, a.line, &a.target).cmp(&(&b.scope, &b.name, b.line, &b.target))
    });
    binds
}

/// Last simple ident of a heritage clause (`pkg.IERS[T]` → `IERS`).
pub fn simple_heritage_ident(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let t = t.split(['<', '[']).next().unwrap_or(t).trim();
    let t = t.rsplit('.').next().unwrap_or(t).trim();
    if t.is_empty() {
        return None;
    }
    let mut chars = t.chars();
    let first = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    if chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Some(t.to_string())
    } else {
        None
    }
}

/// Sort, dedup, and keep only simple-ident bases for CHA persistence.
pub fn normalize_class_bases(raw: &[(String, Vec<String>)]) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = raw
        .iter()
        .filter_map(|(class, bases)| {
            let mut b: Vec<String> = bases
                .iter()
                .filter_map(|s| simple_heritage_ident(s))
                .collect();
            b.sort();
            b.dedup();
            (!class.is_empty() && !b.is_empty()).then(|| (class.clone(), b))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// A language extractor. Must be deterministic and side-effect free.
pub trait LanguageExtractor: Send + Sync {
    fn language(&self) -> &'static str;
    fn extract(&self, file: &SourceFile) -> ExtractedFile;
}
