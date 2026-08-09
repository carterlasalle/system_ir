//! Extraction model: the contract between language extractors and the
//! Reality Compiler.
//!
//! Extractors are pure functions: `(path, content) -> ExtractedFile`. They
//! perform syntax-level extraction only. Cross-file resolution happens later
//! in `resolve.rs`.

use serde::{Deserialize, Serialize};

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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Everything a language extractor returns for one file.
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
}

/// A language extractor. Must be deterministic and side-effect free.
pub trait LanguageExtractor: Send + Sync {
    fn language(&self) -> &'static str;
    fn extract(&self, file: &SourceFile) -> ExtractedFile;
}
