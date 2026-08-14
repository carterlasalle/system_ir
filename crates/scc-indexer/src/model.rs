//! Extraction model: the contract between language extractors and the
//! Reality Compiler.
//!
//! Extractors are pure functions: `(path, content) -> ExtractedFile`. They
//! perform syntax-level extraction only. Cross-file resolution happens later
//! in `resolve.rs`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A source file handed to an extractor.
#[derive(Debug, Clone)]
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
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    /// One-line signature, e.g. `def normalize(text: str) -> str`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
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
    Field { owner: String, name: String, mutable: bool },
    /// A framework registration: route/middleware/plugin/DI/event
    /// registration performed by `owner` naming `target` with a `kind`.
    Registration { owner: String, kind: String, target: String },
    /// Configuration ownership: `owner` reads/writes config `key`.
    Configuration { owner: String, key: String },
    /// A callback/hook handled by `owner` (framework invokes it).
    Callback { owner: String, callback: String },
    /// A structured schema/model definition (zod z.object, pydantic
    /// BaseModel, JSON Schema, serde model, go struct tags, Java
    /// validation annotations): `owner` defines schema `name`.
    /// `expr` is the defining source expression when available
    /// (e.g. `z.object({ name: z.string() })`), else empty.
    SchemaDefinition { owner: String, name: String, expr: String },
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
}

/// A language extractor. Must be deterministic and side-effect free.
pub trait LanguageExtractor: Send + Sync {
    fn language(&self) -> &'static str;
    fn extract(&self, file: &SourceFile) -> ExtractedFile;
}
