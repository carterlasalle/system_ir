//! Import normalization and cross-file call resolution (SCC-025, SCC-026).
//!
//! Resolution rules, in priority order:
//! 1. receiver classification (this/self vs named vs field-chain vs type);
//! 2. local symbol in the same file;
//! 3. imported member (`import { a as b }` / `from m import a`) resolved to
//!    the target file's exported symbol;
//! 4. imported module namespace (`import * as ns`, `import m`) → member on
//!    the target file;
//! 5. `self`/`this` sibling methods, plus one-hop `self.field.m()` / `this.field.m()` when the field type is unique;
//! 6. confirmed external import root → `external_api` entity;
//! 7. otherwise: unresolved (counted; never silently treated as external).
//!
//! Native resolution is EXTRACTED (candidate), never RESOLVED.

use crate::model::{Call, Import, ImportType, Symbol, SymbolKind, TypeBind};
use crate::recv::{classify_callee, split_recv_path, RecvFact};
use scc_core::{RecvKind, ReferenceKind, ResolutionClass};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct ResolvedCall {
    /// Caller entity id: symbol entity, or file entity for module-level calls.
    pub caller_id: String,
    /// Callee entity id when resolved.
    pub callee_id: Option<String>,
    /// Callee name as written (display).
    pub callee_name: String,
    /// Resolved via local/import/binding (RESOLVED) or external (EXTRACTED).
    pub provenance: scc_core::Provenance,
    pub confidence: f64,
    pub line: u32,
    pub class: ResolutionClass,
    pub recv: RecvKind,
    /// Number of remaining in-repo candidates after narrowing (0/1/k).
    pub candidates: u32,
}

/// Symbol index for one file.
#[derive(Debug, Clone, Default)]
pub struct FileSymbols {
    /// name -> (symbol, entity id)
    pub by_name: BTreeMap<String, (Symbol, String)>,
    /// methods: "ClassName.method" -> entity id (for self/this resolution)
    pub methods: BTreeMap<String, (String, String)>, // key -> (class, entity id)
    pub file_entity_id: String,
    /// Local type binds for this file (constructor / annotation / param).
    pub type_binds: Vec<TypeBind>,
}

/// Module resolution result for an import.
#[derive(Debug, Clone)]
pub enum ImportTarget {
    /// Resolved to a file in the repo. `name_map`: local name -> exported name.
    Internal { file: String, name_map: HashMap<String, String>, namespace: bool },
    /// Bare specifier treated as external system/api.
    External { name: String },
    /// Relative or project-looking specifier that did not resolve to a file.
    /// Not evidence that the target is outside the repository.
    Unresolved { name: String },
}

#[derive(Debug, Clone)]
pub struct ResolvedImport {
    pub local_file: String,
    pub module: String,
    pub target: ImportTarget,
    pub names: Vec<(String, String)>,
    pub line: u32,
}

/// All files' symbols keyed by repo-relative path.
// trace:exempt reason=internal-detail
pub struct SymbolIndex {
    pub files: HashMap<String, FileSymbols>,
    /// All internal file paths (for module resolution).
    pub all_files: HashSet<String>,
    pub repo_id: String,
}

impl SymbolIndex {
    pub fn new(repo_id: &str) -> Self {
        SymbolIndex {
            files: HashMap::new(),
            all_files: HashSet::new(),
            repo_id: repo_id.to_string(),
        }
    }

    pub fn add_file(&mut self, path: &str, symbols: &[Symbol]) {
        self.all_files.insert(path.to_string());
        let mut fs = FileSymbols {
            file_entity_id: scc_core::entity_id(&self.repo_id, scc_core::kinds::FILE, path),
            ..Default::default()
        };
        for s in symbols {
            let id = scc_core::symbol_id(&self.repo_id, path, &s.name);
            fs.by_name.insert(s.name.clone(), (s.clone(), id.clone()));
            if s.kind == SymbolKind::Method {
                if let Some(parent) = &s.parent {
                    fs.methods.insert(s.name.clone(), (parent.clone(), id));
                }
            }
        }
        self.files.insert(path.to_string(), fs);
    }

    /// Attach extract-time type binds used by NamedVariable narrowing.
    pub fn set_type_binds(&mut self, path: &str, binds: &[TypeBind]) {
        if let Some(fs) = self.files.get_mut(path) {
            fs.type_binds = binds.to_vec();
        }
    }

    pub fn file(&self, path: &str) -> Option<&FileSymbols> {
        self.files.get(path)
    }

    /// Resolve an import statement to a file in the repo.
    pub fn resolve_import(&self, from_file: &str, import: &Import) -> ImportTarget {
        if import.module.starts_with('.') {
            // relative: join with dir of from_file, then normalize ./ and ..
            let dir = from_file.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            let joined = if dir.is_empty() {
                import.module.clone()
            } else {
                format!("{dir}/{}", import.module)
            };
            let joined = normalize_module_path(&joined);
            match self.resolve_module_path(&joined) {
                Some(f) => ImportTarget::Internal {
                    file: f,
                    name_map: HashMap::new(),
                    namespace: import.r#type == ImportType::Module,
                },
                None => ImportTarget::Unresolved {
                    name: import.module.clone(),
                },
            }
        } else {
            // top-level: python modules or bare TS specifiers
            match self.resolve_module_path(&import.module) {
                Some(f) => ImportTarget::Internal {
                    file: f,
                    name_map: HashMap::new(),
                    namespace: import.r#type == ImportType::Module,
                },
                None => ImportTarget::External {
                    name: import.module.clone(),
                },
            }
        }
    }

    /// Try `a/b`, `a/b.py`, `a/b/__init__.py`, `a/b/index.ts` etc. Also tries
    /// a `src/` prefix for python-style layouts.
    fn resolve_module_path(&self, module: &str) -> Option<String> {
        let module = normalize_module_path(module);
        let candidates = self.candidate_paths(&module);
        for c in candidates {
            if self.all_files.contains(&c) {
                return Some(c);
            }
        }
        // source-root fallback: `foo` / `a/b` may live under a conventional
        // source root (src, svc, lib, app, services, packages) — common for
        // python and typescript repos. Deterministic order; first match wins.
        if !module.starts_with('.') && !module.starts_with('/') {
            for root in ["src", "svc", "lib", "app", "services", "service", "packages"] {
                for c in self.candidate_paths(&format!("{root}/{module}")) {
                    if self.all_files.contains(&c) {
                        return Some(c);
                    }
                }
            }
        }
        None
    }

    fn candidate_paths(&self, module: &str) -> Vec<String> {
        let mut out = Vec::new();
        out.push(module.to_string());
        let last = module.rsplit('/').next().unwrap_or(module);
        if module.contains(".") && !module.ends_with(".py") && !module.ends_with(".ts") {
            // python dotted module: `pkg.sub` -> pkg/sub.py, pkg/sub/__init__.py
            let as_path = module.replace('.', "/");
            out.push(format!("{as_path}.py"));
            out.push(format!("{as_path}/__init__.py"));
        }
        if last == "__init__" {
            return out;
        }
        out.push(format!("{module}.py"));
        out.push(format!("{module}/__init__.py"));
        out.push(format!("{module}.ts"));
        out.push(format!("{module}.tsx"));
        out.push(format!("{module}.js"));
        out.push(format!("{module}.jsx"));
        out.push(format!("{module}/index.ts"));
        out.push(format!("{module}/index.tsx"));
        out.push(format!("{module}/index.js"));
        out.push(format!("{module}/index.jsx"));
        out
    }
}

/// Collapse `.`/`..` segments in a module path.
fn normalize_module_path(p: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => continue,
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// Seed a field-chain's imported root without pinning the method name.
///
/// `db.users.findMany` through `import { db } from "./db"` yields the `db`
/// export (UnresolvedLikelyInternal). An external import root yields
/// ConfirmedExternal. `self.client.process` is not handled here.
fn field_chain_root_callee(
    root: &str,
    binding: &HashMap<&str, (String, String)>,
    namespaces: &HashMap<&str, String>,
    index: &SymbolIndex,
    repo_id: &str,
) -> Option<(String, ResolutionClass)> {
    if let Some((target_file, exported)) = binding.get(root) {
        if let Some(external) = target_file.strip_prefix("external:") {
            return Some((
                scc_core::entity_id(repo_id, scc_core::kinds::EXTERNAL_API, external),
                ResolutionClass::ConfirmedExternal,
            ));
        }
        let id = index
            .files
            .get(target_file.as_str())
            .and_then(|fs| fs.by_name.get(exported).map(|(_, id)| id.clone()))
            .unwrap_or_else(|| scc_core::symbol_id(repo_id, target_file, exported));
        return Some((id, ResolutionClass::UnresolvedLikelyInternal));
    }
    if let Some(ns_file) = namespaces.get(root) {
        if let Some(fs) = index.files.get(ns_file.as_str()) {
            if let Some((_, id)) = fs.by_name.get(root) {
                return Some((id.clone(), ResolutionClass::UnresolvedLikelyInternal));
            }
            return Some((
                fs.file_entity_id.clone(),
                ResolutionClass::UnresolvedLikelyInternal,
            ));
        }
        return Some((
            scc_core::entity_id(repo_id, scc_core::kinds::FILE, ns_file),
            ResolutionClass::UnresolvedLikelyInternal,
        ));
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn emit(
    caller_id: String,
    callee_id: Option<String>,
    callee_name: String,
    confidence: f64,
    line: u32,
    class: ResolutionClass,
    recv: RecvKind,
    candidates: u32,
) -> ResolvedCall {
    ResolvedCall {
        caller_id,
        callee_id,
        callee_name,
        provenance: scc_core::Provenance::Extracted,
        confidence,
        line,
        class,
        recv,
        candidates,
    }
}

fn enclosing_class(call: &Call, path: &str, index: &SymbolIndex) -> Option<String> {
    let c = call.caller.as_deref()?;
    let base = c.split('.').next().unwrap_or(c);
    if let Some((class, _id)) = index.files.get(path).and_then(|fs| fs.methods.get(c)) {
        Some(class.clone())
    } else {
        Some(base.to_string())
    }
}

fn sibling_method<'a>(
    methods_by_class: &HashMap<&'a str, Vec<&'a Symbol>>,
    class: &str,
    method: &str,
) -> Option<&'a Symbol> {
    methods_by_class.get(class).and_then(|ms| {
        ms.iter()
            .find(|s| s.name == format!("{class}.{method}") || s.name == method)
            .copied()
    })
}

/// Resolve all calls in one file against the full symbol index.
/// Receiver classification is first-class: field-chains are not pinned
/// to the intermediate name; missing internals are not labeled external.
// trace:v1 id=impl.scc.resolve work=WORK-ripwire-lessons-phase1 satisfies=REQ-receiver-aware-resolution
pub fn resolve_calls(
    path: &str,
    calls: &[Call],
    symbols: &[Symbol],
    resolved_imports: &[ResolvedImport],
    index: &SymbolIndex,
    repo_id: &str,
) -> Vec<ResolvedCall> {
    // local name -> (target file, exported name)
    let mut binding: HashMap<&str, (String, String)> = HashMap::new();
    // namespace imports: local ns name -> target file
    let mut namespaces: HashMap<&str, String> = HashMap::new();
    for ri in resolved_imports {
        match &ri.target {
            ImportTarget::Internal { file, namespace, .. } => {
                if *namespace {
                    // `import * as ns from 'm'`, python `import a.b [as c]`:
                    // local name binds the module
                    for (local, imported) in &ri.names {
                        if imported == "default" {
                            // `import x from 'm'` binds the default export
                            let exported = default_symbol_name(index, file);
                            binding.insert(local.as_str(), (file.clone(), exported));
                        } else {
                            namespaces.insert(local.as_str(), file.clone());
                        }
                    }
                } else {
                    for (local, imported) in &ri.names {
                        let exported = if imported == "default" {
                            // default export: symbol named `default`, else a
                            // deterministic fallback
                            default_symbol_name(index, file)
                        } else {
                            imported.clone()
                        };
                        binding.insert(local.as_str(), (file.clone(), exported));
                    }
                }
            }
            ImportTarget::External { name } => {
                for (local, imported) in &ri.names {
                    binding.insert(local.as_str(), (format!("external:{name}"), imported.clone()));
                }
            }
            ImportTarget::Unresolved { .. } => {}
        }
    }

    // local symbols by name
    let local: HashMap<&str, &Symbol> = symbols.iter().map(|s| (s.name.as_str(), s)).collect();
    // methods by class
    let methods_by_class: HashMap<&str, Vec<&Symbol>> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Method)
        .filter_map(|s| s.parent.as_deref().map(|p| (p, s)))
        .fold(HashMap::new(), |mut m, (p, s)| {
            m.entry(p).or_default().push(s);
            m
        });

    let type_binds: &[TypeBind] = index
        .files
        .get(path)
        .map(|f| f.type_binds.as_slice())
        .unwrap_or(&[]);

    let caller_ctx = |call: &Call| -> String {
        match &call.caller {
            Some(cname) => {
                // caller may be "ClassName.method" for methods
                scc_core::symbol_id(repo_id, path, cname)
            }
            None => scc_core::entity_id(repo_id, scc_core::kinds::FILE, path),
        }
    };

    let mut out = Vec::new();
    for call in calls {
        let call = call.clone().finish();
        if call.role != ReferenceKind::Call && call.role != ReferenceKind::Construct {
            // Type/import/macro mentions are not execution edges.
            continue;
        }
        let caller_id = caller_ctx(&call);
        let fact = classify_callee(&call.callee);
        let recv = if call.recv == RecvKind::Unknown {
            fact.recv
        } else {
            call.recv
        };
        let method = fact.method.as_str();
        let root = fact.root.as_str();

        if let Some(id) = field_type_callee_id(recv, &fact, &call, path, index, type_binds, &binding) {
            out.push(emit(
                caller_id,
                Some(id),
                call.callee.clone(),
                0.9,
                call.line,
                ResolutionClass::ResolvedInternal,
                recv,
                1,
            ));
            continue;
        }
        if let Some(id) =
            receiver_field_type_callee_id(recv, &fact, &call, path, index, type_binds, &binding)
        {
            out.push(emit(
                caller_id,
                Some(id),
                call.callee.clone(),
                0.9,
                call.line,
                ResolutionClass::ResolvedInternal,
                recv,
                1,
            ));
            continue;
        }

        // Field chains: do not pin the intermediate name or the terminal
        // method (`self.client.process`, `db.users.findMany`). If the chain
        // is rooted at a resolved import (`import { db }`), seed that object
        // so flows know the function touches `db` without claiming
        // `findMany` resolved.
        if recv.is_field_chain() {
            let seeded = if recv == RecvKind::FieldOfVariable {
                field_chain_root_callee(root, &binding, &namespaces, index, repo_id)
            } else {
                None
            };
            let (callee_id, class, conf, cand) = match seeded {
                Some((id, ResolutionClass::ConfirmedExternal)) => {
                    (Some(id), ResolutionClass::ConfirmedExternal, 0.8, 0)
                }
                Some((id, class)) => (Some(id), class, 0.55, 1),
                None => (None, ResolutionClass::UnresolvedLikelyInternal, 0.4, 0),
            };
            out.push(emit(
                caller_id,
                callee_id,
                call.callee.clone(),
                conf,
                call.line,
                class,
                recv,
                cand,
            ));
            continue;
        }

        // super(): never spray onto the caller's class.
        if recv == RecvKind::Super {
            out.push(emit(
                caller_id,
                None,
                call.callee.clone(),
                0.4,
                call.line,
                ResolutionClass::UnresolvedLikelyInternal,
                recv,
                0,
            ));
            continue;
        }

        // this/self → sibling of the enclosing class only.
        if recv.is_instance_self() {
            if let Some(class) = enclosing_class(&call, path, index) {
                if let Some(m) = sibling_method(&methods_by_class, &class, method) {
                    out.push(emit(
                        caller_id,
                        Some(scc_core::symbol_id(repo_id, path, &m.name)),
                        call.callee.clone(),
                        0.98,
                        call.line,
                        ResolutionClass::ResolvedInternal,
                        recv,
                        1,
                    ));
                    continue;
                }
            }
            out.push(emit(
                caller_id,
                None,
                call.callee.clone(),
                0.5,
                call.line,
                ResolutionClass::UnresolvedLikelyInternal,
                recv,
                0,
            ));
            continue;
        }

        // Bare local symbol.
        if recv == RecvKind::None {
            if let Some(sym) = local.get(root) {
                if sym.kind.is_callable() {
                    out.push(emit(
                        caller_id,
                        Some(scc_core::symbol_id(repo_id, path, &sym.name)),
                        call.callee.clone(),
                        0.99,
                        call.line,
                        ResolutionClass::ResolvedInternal,
                        recv,
                        1,
                    ));
                    continue;
                }
            }
        }

        // Imported member binding (root of callee).
        if let Some((target_file, exported)) = binding.get(root) {
            if let Some(external) = target_file.strip_prefix("external:") {
                out.push(emit(
                    caller_id,
                    Some(scc_core::entity_id(
                        repo_id,
                        scc_core::kinds::EXTERNAL_API,
                        external,
                    )),
                    call.callee.clone(),
                    0.8,
                    call.line,
                    ResolutionClass::ConfirmedExternal,
                    recv,
                    0,
                ));
                continue;
            }
            let member = if matches!(recv, RecvKind::NamedVariable | RecvKind::StaticType) {
                Some(method)
            } else if recv == RecvKind::None {
                None
            } else {
                Some(method)
            };
            let callee_id = match index.files.get(target_file.as_str()) {
                Some(fs) => {
                    if let Some(member) = member {
                        if let Some((_, id)) = fs.by_name.get(&format!("{exported}.{member}")) {
                            Some(id.clone())
                        } else {
                            fs.by_name.get(exported).map(|(_, id)| id.clone())
                        }
                    } else {
                        fs.by_name.get(exported).map(|(_, id)| id.clone())
                    }
                }
                None => None,
            };
            let class = if callee_id.is_some() {
                ResolutionClass::ResolvedInternal
            } else {
                ResolutionClass::UnresolvedLikelyInternal
            };
            out.push(emit(
                caller_id,
                callee_id,
                call.callee.clone(),
                0.95,
                call.line,
                class,
                recv,
                1,
            ));
            continue;
        }

        // Namespace import member (`import * as ns`, python `import m`).
        if let Some(ns_file) = namespaces.get(root) {
            if recv != RecvKind::None {
                if let Some((_, id)) = index
                    .files
                    .get(ns_file.as_str())
                    .and_then(|fs| fs.by_name.get(method))
                {
                    out.push(emit(
                        caller_id,
                        Some(id.clone()),
                        call.callee.clone(),
                        0.97,
                        call.line,
                        ResolutionClass::ResolvedInternal,
                        recv,
                        1,
                    ));
                    continue;
                }
            }
            out.push(emit(
                caller_id,
                None,
                call.callee.clone(),
                0.5,
                call.line,
                ResolutionClass::UnresolvedLikelyInternal,
                recv,
                0,
            ));
            continue;
        }

        // Type-qualified / static type / local class method.
        if matches!(recv, RecvKind::StaticType | RecvKind::TypeQualified | RecvKind::NamedVariable)
        {
            if let Some(sym) = local.get(root) {
                if let Some((_, mid)) = index
                    .files
                    .get(path)
                    .and_then(|fs| fs.methods.get(&format!("{}.{}", sym.name, method)))
                {
                    out.push(emit(
                        caller_id,
                        Some(mid.clone()),
                        call.callee.clone(),
                        0.9,
                        call.line,
                        ResolutionClass::ResolvedInternal,
                        recv,
                        1,
                    ));
                    continue;
                }
            }
            if recv == RecvKind::NamedVariable {
                if let Some(ty) = unique_bound_type(type_binds, call.caller.as_deref(), root) {
                    if let Some(id) =
                        method_id_for_type(index, path, ty, method, &binding)
                    {
                        out.push(emit(
                            caller_id,
                            Some(id),
                            call.callee.clone(),
                            0.9,
                            call.line,
                            ResolutionClass::ResolvedInternal,
                            recv,
                            1,
                        ));
                        continue;
                    }
                }
                if let Some(id) = unprefixed_field_type_callee_id(
                    recv,
                    &fact,
                    &call,
                    path,
                    index,
                    type_binds,
                    &binding,
                ) {
                    out.push(emit(
                        caller_id,
                        Some(id),
                        call.callee.clone(),
                        0.9,
                        call.line,
                        ResolutionClass::ResolvedInternal,
                        recv,
                        1,
                    ));
                    continue;
                }
                // Unknown typed variable: do not spray to every same-name method.
                out.push(emit(
                    caller_id,
                    None,
                    call.callee.clone(),
                    0.4,
                    call.line,
                    ResolutionClass::UnresolvedLikelyInternal,
                    recv,
                    0,
                ));
                continue;
            }
        }

        // Bare name that did not bind: unknown, not external.
        let class = if recv == RecvKind::None {
            ResolutionClass::Unknown
        } else {
            ResolutionClass::UnresolvedLikelyInternal
        };
        out.push(emit(
            caller_id,
            None,
            call.callee.clone(),
            0.5,
            call.line,
            class,
            recv,
            0,
        ));
    }
    out
}

/// Aggregate honesty gauges from native resolution (no LSP/SCIP).
pub fn quality_from_calls(calls: &[ResolvedCall]) -> scc_core::AnalysisQuality {
    let mut q = scc_core::AnalysisQuality::default();
    for c in calls {
        q.record_call(c.class, false);
    }
    q
}

/// Unique type for `(scope, var)` or None when missing / tombstoned (≥2 types).
// trace:v1 id=impl.scc.resolve.type-narrow work=WORK-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing-from-unique satisfies=REQ-implement-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing
fn unique_bound_type<'a>(
    binds: &'a [TypeBind],
    scope: Option<&str>,
    var: &str,
) -> Option<&'a str> {
    let scope = scope.unwrap_or("");
    let mut types: Vec<&str> = binds
        .iter()
        .filter(|b| b.scope == scope && b.name == var)
        .map(|b| b.type_name.as_str())
        .collect();
    types.sort_unstable();
    types.dedup();
    if types.len() == 1 {
        Some(types[0])
    } else {
        None
    }
}

/// True when any bind exists for `(scope, var)`, including tombstones and
/// untyped local shadows. A local of any kind vetoes field-as-receiver.
fn has_bind(binds: &[TypeBind], scope: Option<&str>, var: &str) -> bool {
    let scope = scope.unwrap_or("");
    binds.iter().any(|b| b.scope == scope && b.name == var)
}

/// Pin `self.x.m()` / `this.x.m()` (exactly one field hop) to `{Type}.m`
/// when the enclosing class has a unique field type and that method exists.
// trace:v1 id=impl.scc.resolve.field-type-narrow work=WORK-phase-8-of-scc-x-ripwire-lessons-one-hop-field-type-narrowing-unique-c satisfies=REQ-implement-phase-8-of-scc-x-ripwire-lessons-one-hop-field-type-narrowi
fn field_type_callee_id(
    recv: RecvKind,
    fact: &RecvFact,
    call: &Call,
    path: &str,
    index: &SymbolIndex,
    type_binds: &[TypeBind],
    binding: &HashMap<&str, (String, String)>,
) -> Option<String> {
    if !matches!(recv, RecvKind::FieldOfSelf | RecvKind::FieldOfThis) {
        return None;
    }
    if split_recv_path(&call.callee).len() != 3 {
        return None;
    }
    let field = fact.field_name.as_deref()?;
    let class = enclosing_class(call, path, index)?;
    let ty = unique_bound_type(type_binds, Some(&class), field)?;
    method_id_for_type(index, path, ty, &fact.method, binding)
}

/// Pin `s.field.m()` (exactly one field hop) when `s` is uniquely typed as
/// the enclosing type and that type has a unique field type for `field`.
// trace:v1 id=impl.scc.resolve.receiver-field-type-narrow work=WORK-phase-10-of-scc-x-ripwire-lessons-go-one-hop-receiver-field-type-narrow satisfies=REQ-implement-phase-10-of-scc-x-ripwire-lessons-go-one-hop-receiver-field implements=PLAN-phase-10-of-scc-x-ripwire-lessons-go-one-hop-receiver-field-type-narrow
fn receiver_field_type_callee_id(
    recv: RecvKind,
    fact: &RecvFact,
    call: &Call,
    path: &str,
    index: &SymbolIndex,
    type_binds: &[TypeBind],
    binding: &HashMap<&str, (String, String)>,
) -> Option<String> {
    if recv != RecvKind::FieldOfVariable {
        return None;
    }
    if split_recv_path(&call.callee).len() != 3 {
        return None;
    }
    let root = fact.recv_var.as_deref().unwrap_or(fact.root.as_str());
    let class = enclosing_class(call, path, index)?;
    let root_ty = unique_bound_type(type_binds, call.caller.as_deref(), root)?;
    if root_ty != class {
        return None;
    }
    let field = fact.field_name.as_deref()?;
    let ty = unique_bound_type(type_binds, Some(&class), field)?;
    method_id_for_type(index, path, ty, &fact.method, binding)
}

/// Pin Java `repo.save()` (exactly one named hop, no `this.`) to `{Type}.save`
/// when `repo` is a unique field of the enclosing class and is not shadowed
/// by a parameter or local. Python/TS/Go/Rust omit `this`/`self` only as a
/// NameError / compile error, so this rule is `.java`-gated (Ripwire Rule 2b
/// analogue; Ripwire itself gates C++/ObjC).
// trace:v1 id=impl.scc.resolve.unprefixed-field-type work=WORK-phase-12-of-scc-x-ripwire-lessons-java-unprefixed-field-as-receiver-typ satisfies=REQ-implement-phase-12-of-scc-x-ripwire-lessons-java-unprefixed-field-as implements=PLAN-phase-12-of-scc-x-ripwire-lessons-java-unprefixed-field-as-receiver-typ
fn unprefixed_field_type_callee_id(
    recv: RecvKind,
    fact: &RecvFact,
    call: &Call,
    path: &str,
    index: &SymbolIndex,
    type_binds: &[TypeBind],
    binding: &HashMap<&str, (String, String)>,
) -> Option<String> {
    if recv != RecvKind::NamedVariable {
        return None;
    }
    if !path.ends_with(".java") {
        return None;
    }
    if split_recv_path(&call.callee).len() != 2 {
        return None;
    }
    let root = fact.recv_var.as_deref().unwrap_or(fact.root.as_str());
    if has_bind(type_binds, call.caller.as_deref(), root) {
        return None;
    }
    let class = enclosing_class(call, path, index)?;
    let ty = unique_bound_type(type_binds, Some(&class), root)?;
    if ty.is_empty() {
        return None;
    }
    method_id_for_type(index, path, ty, &fact.method, binding)
}

/// Real `{Type}.{method}` definition locally or on the imported type only.
fn method_id_for_type(
    index: &SymbolIndex,
    local_path: &str,
    ty: &str,
    method: &str,
    binding: &HashMap<&str, (String, String)>,
) -> Option<String> {
    if method.is_empty() {
        return None;
    }
    let local_key = format!("{ty}.{method}");
    if let Some(fs) = index.files.get(local_path) {
        if let Some((_, id)) = fs.by_name.get(&local_key) {
            return Some(id.clone());
        }
        if let Some((_, id)) = fs.methods.get(&local_key) {
            return Some(id.clone());
        }
    }
    if let Some((target_file, exported)) = binding.get(ty) {
        if target_file.starts_with("external:") {
            return None;
        }
        let key = format!("{exported}.{method}");
        if let Some(fs) = index.files.get(target_file.as_str()) {
            if let Some((_, id)) = fs.by_name.get(&key) {
                return Some(id.clone());
            }
            if let Some((_, id)) = fs.methods.get(&key) {
                return Some(id.clone());
            }
        }
    }
    None
}

fn default_symbol_name(index: &SymbolIndex, file: &str) -> String {
    if let Some(fs) = index.files.get(file) {
        if fs.by_name.contains_key("default") {
            return "default".to_string();
        }
        let mut candidates: Vec<String> = fs.by_name.keys().cloned().collect();
        candidates.sort_by_key(|n| (n.len(), n.clone()));
        if let Some(first) = candidates.into_iter().next() {
            return first;
        }
    }
    "default".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Import, ImportType};

// trace:exempt reason=internal-detail
    fn mk_symbol(name: &str, kind: SymbolKind) -> Symbol {
        Symbol {
            name: name.into(),
            kind,
            signature: None,
            decl_header: None,
            start_line: 1,
            end_line: 2,
            exported: true,
            docstring: None,
            parent: None,
        }
    }

    #[test]
    fn resolves_local_calls() {
        let mut idx = SymbolIndex::new("repo");
        let syms = vec![mk_symbol("normalize", SymbolKind::Function)];
        idx.add_file("a.py", &syms);
        let calls = vec![Call {
            caller: Some("normalize".into()),
            callee: "normalize".into(),
            line: 3,
            known_receiver: true,
            conditional: false,
            ..Default::default()
        }];
        let resolved = resolve_calls("a.py", &calls, &syms, &[], &idx, "repo");
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].callee_id, Some(scc_core::symbol_id("repo", "a.py", "normalize")));
        assert_eq!(resolved[0].provenance, scc_core::Provenance::Extracted, "native resolution is evidence-grade (candidate), never RESOLVED (section 26)");
    }

    #[test]
    fn resolves_imported_member_with_alias() {
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("b.py", &[mk_symbol("resolve", SymbolKind::Function)]);
        idx.add_file("a.py", &[mk_symbol("main", SymbolKind::Function)]);
        let import = Import {
            module: "b".into(),
            names: vec![("r".into(), "resolve".into())],
            line: 1,
            r#type: ImportType::Member,
        };
        let resolved_imports = vec![ResolvedImport {
            local_file: "a.py".into(),
            module: "b".into(),
            target: ImportTarget::Internal { file: "b.py".into(), name_map: HashMap::new(), namespace: false },
            names: vec![("r".into(), "resolve".into())],
            line: 1,
        }];
        let calls = vec![Call {
            caller: Some("main".into()),
            callee: "r".into(),
            line: 3,
            known_receiver: true,
            conditional: false,
            ..Default::default()
        }];
        let resolved = resolve_calls("a.py", &calls, &[mk_symbol("main", SymbolKind::Function)], &resolved_imports, &idx, "repo");
        assert_eq!(resolved[0].callee_id, Some(scc_core::symbol_id("repo", "b.py", "resolve")));
        // unused import var
        let _ = &import;
    }

    #[test]
    fn resolves_namespace_member() {
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("svc/asr.py", &[mk_symbol("transcribe", SymbolKind::Function)]);
        idx.add_file("main.py", &[mk_symbol("run", SymbolKind::Function)]);
        let ri = ResolvedImport {
            local_file: "main.py".into(),
            module: "svc.asr".into(),
            target: ImportTarget::Internal { file: "svc/asr.py".into(), name_map: HashMap::new(), namespace: true },
            names: vec![("asr".into(), "*".into())],
            line: 1,
        };
        let calls = vec![Call {
            caller: Some("run".into()),
            callee: "asr.transcribe".into(),
            line: 3,
            known_receiver: true,
            conditional: false,
            ..Default::default()
        }];
        let resolved = resolve_calls("main.py", &calls, &[mk_symbol("run", SymbolKind::Function)], &[ri], &idx, "repo");
        assert_eq!(resolved[0].callee_id, Some(scc_core::symbol_id("repo", "svc/asr.py", "transcribe")));
    }

    #[test]
    fn resolves_self_method() {
        let mut idx = SymbolIndex::new("repo");
        let mut m = mk_symbol("Worker.handle", SymbolKind::Method);
        m.parent = Some("Worker".into());
        let mut m2 = mk_symbol("Worker.helper", SymbolKind::Method);
        m2.parent = Some("Worker".into());
        let syms = vec![
            mk_symbol("Worker", SymbolKind::Class),
            m,
            m2,
        ];
        idx.add_file("w.py", &syms);
        let calls = vec![Call {
            caller: Some("Worker.handle".into()),
            callee: "self.helper".into(),
            line: 3,
            known_receiver: true,
            conditional: false,
            ..Default::default()
        }];
        let resolved = resolve_calls("w.py", &calls, &syms, &[], &idx, "repo");
        assert_eq!(resolved[0].callee_id, Some(scc_core::symbol_id("repo", "w.py", "Worker.helper")));
    }

    #[test]
    fn external_import_becomes_external_api() {
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("a.ts", &[mk_symbol("main", SymbolKind::Function)]);
        let ri = ResolvedImport {
            local_file: "a.ts".into(),
            module: "express".into(),
            target: ImportTarget::External { name: "express".into() },
            names: vec![("express".into(), "default".into())],
            line: 1,
        };
        let calls = vec![Call {
            caller: Some("main".into()),
            callee: "express".into(),
            line: 3,
            known_receiver: false,
            conditional: false,
            ..Default::default()
        }];
        let resolved = resolve_calls("a.ts", &calls, &[mk_symbol("main", SymbolKind::Function)], &[ri], &idx, "repo");
        assert_eq!(
            resolved[0].callee_id,
            Some(scc_core::entity_id("repo", scc_core::kinds::EXTERNAL_API, "express"))
        );
    }

    #[test]
    fn module_path_resolution() {
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("pkg/sub.py", &[]);
        idx.add_file("pkg/__init__.py", &[]);
        idx.add_file("src/util.py", &[]);
        idx.add_file("web/index.ts", &[]);
        assert_eq!(idx.resolve_module_path("pkg.sub"), Some("pkg/sub.py".into()));
        assert_eq!(idx.resolve_module_path("pkg"), Some("pkg/__init__.py".into()));
        assert_eq!(idx.resolve_module_path("./web"), Some("web/index.ts".into()));
        assert_eq!(idx.resolve_module_path("util"), Some("src/util.py".into()));
        assert_eq!(idx.resolve_module_path("nonexistent"), None);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.this-not-other-class verifies=REQ-receiver-aware-resolution exercises=impl.scc.resolve
    fn this_process_does_not_bind_other_class() {
        let mut idx = SymbolIndex::new("repo");
        let mut a_h = mk_symbol("A.handle", SymbolKind::Method);
        a_h.parent = Some("A".into());
        let mut a_p = mk_symbol("A.process", SymbolKind::Method);
        a_p.parent = Some("A".into());
        let mut b_p = mk_symbol("B.process", SymbolKind::Method);
        b_p.parent = Some("B".into());
        let syms = vec![
            mk_symbol("A", SymbolKind::Class),
            a_h,
            a_p,
            mk_symbol("B", SymbolKind::Class),
            b_p,
        ];
        idx.add_file("w.py", &syms);
        let calls = vec![Call {
            caller: Some("A.handle".into()),
            callee: "this.process".into(),
            line: 3,
            known_receiver: true,
            ..Default::default()
        }
        .finish()];
        let resolved = resolve_calls("w.py", &calls, &syms, &[], &idx, "repo");
        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "w.py", "A.process"))
        );
        assert_ne!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "w.py", "B.process"))
        );
        assert_eq!(resolved[0].class, ResolutionClass::ResolvedInternal);
        assert_eq!(resolved[0].recv, RecvKind::This);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.field-chain-not-pinned verifies=REQ-receiver-aware-resolution exercises=impl.scc.resolve
    fn field_chain_is_not_pinned_to_intermediate_method() {
        let mut idx = SymbolIndex::new("repo");
        let mut h = mk_symbol("A.handle", SymbolKind::Method);
        h.parent = Some("A".into());
        let mut client = mk_symbol("A.client", SymbolKind::Method);
        client.parent = Some("A".into());
        let mut process = mk_symbol("A.process", SymbolKind::Method);
        process.parent = Some("A".into());
        let syms = vec![mk_symbol("A", SymbolKind::Class), h, client, process];
        idx.add_file("w.py", &syms);
        let calls = vec![Call {
            caller: Some("A.handle".into()),
            callee: "self.client.process".into(),
            line: 4,
            known_receiver: true,
            ..Default::default()
        }
        .finish()];
        let resolved = resolve_calls("w.py", &calls, &syms, &[], &idx, "repo");
        assert_eq!(resolved[0].callee_id, None);
        assert_eq!(resolved[0].class, ResolutionClass::UnresolvedLikelyInternal);
        assert_eq!(resolved[0].recv, RecvKind::FieldOfSelf);
        let q = quality_from_calls(&resolved);
        assert_eq!(q.calls.external, 0, "unresolved must not count as external");
        assert_eq!(q.calls.likely_internal_unresolved, 1);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.field-chain-import-root verifies=REQ-receiver-aware-resolution exercises=impl.scc.resolve
    fn field_chain_through_imported_object_seeds_root_not_method() {
        let mut idx = SymbolIndex::new("repo");
        let db_sym = mk_symbol("db", SymbolKind::Const);
        idx.add_file("db.ts", &[db_sym]);
        let handle = mk_symbol("handleList", SymbolKind::Function);
        idx.add_file("server.ts", std::slice::from_ref(&handle));
        let imports = vec![ResolvedImport {
            local_file: "server.ts".into(),
            module: "./db".into(),
            target: ImportTarget::Internal {
                file: "db.ts".into(),
                name_map: HashMap::new(),
                namespace: false,
            },
            names: vec![("db".into(), "db".into())],
            line: 1,
        }];
        let calls = vec![Call {
            caller: Some("handleList".into()),
            callee: "db.users.findMany".into(),
            line: 4,
            known_receiver: false,
            ..Default::default()
        }
        .finish()];
        let resolved = resolve_calls("server.ts", &calls, &[handle], &imports, &idx, "repo");
        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "db.ts", "db")),
            "seed the imported object, not findMany"
        );
        assert_ne!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "db.ts", "findMany"))
        );
        assert_eq!(resolved[0].class, ResolutionClass::UnresolvedLikelyInternal);
        assert_eq!(resolved[0].recv, RecvKind::FieldOfVariable);
        let q = quality_from_calls(&resolved);
        assert_eq!(q.calls.resolved, 0, "must not claim findMany resolved");
        assert_eq!(q.calls.likely_internal_unresolved, 1);
        assert_eq!(q.calls.external, 0);
    }

    #[test]
    fn relative_import_miss_is_unresolved_not_external() {
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("a.py", &[]);
        let import = Import {
            module: "./missing".into(),
            names: vec![("x".into(), "x".into())],
            line: 1,
            r#type: ImportType::Member,
        };
        match idx.resolve_import("a.py", &import) {
            ImportTarget::Unresolved { name } => assert_eq!(name, "./missing"),
            other => panic!("expected Unresolved, got {other:?}"),
        }
    }

    #[test]
    fn named_variable_does_not_spray_same_name_methods() {
        let mut idx = SymbolIndex::new("repo");
        let mut a = mk_symbol("A.process", SymbolKind::Method);
        a.parent = Some("A".into());
        let mut b = mk_symbol("B.process", SymbolKind::Method);
        b.parent = Some("B".into());
        let main = mk_symbol("run", SymbolKind::Function);
        let syms = vec![
            mk_symbol("A", SymbolKind::Class),
            a,
            mk_symbol("B", SymbolKind::Class),
            b,
            main,
        ];
        idx.add_file("w.py", &syms);
        let calls = vec![Call {
            caller: Some("run".into()),
            callee: "obj.process".into(),
            line: 9,
            known_receiver: false,
            ..Default::default()
        }
        .finish()];
        let resolved = resolve_calls("w.py", &calls, &syms, &[], &idx, "repo");
        assert_eq!(resolved[0].callee_id, None);
        assert_eq!(resolved[0].class, ResolutionClass::UnresolvedLikelyInternal);
        let q = quality_from_calls(&resolved);
        assert_eq!(q.calls.likely_internal_unresolved, 1);
        assert_eq!(q.calls.external, 0);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.type-narrow-local verifies=REQ-implement-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing exercises=impl.scc.resolve.type-narrow
    fn unique_constructor_bind_pins_named_var_to_that_class() {
        let mut idx = SymbolIndex::new("repo");
        let mut order_p = mk_symbol("Order.process", SymbolKind::Method);
        order_p.parent = Some("Order".into());
        let mut inv_p = mk_symbol("Invoice.process", SymbolKind::Method);
        inv_p.parent = Some("Invoice".into());
        let handle = mk_symbol("handle", SymbolKind::Function);
        let syms = vec![
            mk_symbol("Order", SymbolKind::Class),
            order_p,
            mk_symbol("Invoice", SymbolKind::Class),
            inv_p,
            handle,
        ];
        idx.add_file("w.py", &syms);
        idx.set_type_binds(
            "w.py",
            &[TypeBind {
                scope: "handle".into(),
                name: "x".into(),
                type_name: "Order".into(),
                line: 8,
            }],
        );
        let calls = vec![Call {
            caller: Some("handle".into()),
            callee: "x.process".into(),
            line: 9,
            known_receiver: false,
            ..Default::default()
        }
        .finish()];
        let resolved = resolve_calls("w.py", &calls, &syms, &[], &idx, "repo");
        assert_eq!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "w.py", "Order.process"))
        );
        assert_ne!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "w.py", "Invoice.process"))
        );
        assert_eq!(resolved[0].class, ResolutionClass::ResolvedInternal);
        assert_eq!(resolved[0].provenance, scc_core::Provenance::Extracted);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.type-narrow-tombstone verifies=REQ-implement-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing exercises=impl.scc.resolve.type-narrow
    fn two_types_for_same_var_do_not_narrow() {
        let mut idx = SymbolIndex::new("repo");
        let mut order_p = mk_symbol("Order.process", SymbolKind::Method);
        order_p.parent = Some("Order".into());
        let mut inv_p = mk_symbol("Invoice.process", SymbolKind::Method);
        inv_p.parent = Some("Invoice".into());
        let handle = mk_symbol("handle", SymbolKind::Function);
        let syms = vec![
            mk_symbol("Order", SymbolKind::Class),
            order_p,
            mk_symbol("Invoice", SymbolKind::Class),
            inv_p,
            handle,
        ];
        idx.add_file("w.py", &syms);
        idx.set_type_binds(
            "w.py",
            &[
                TypeBind {
                    scope: "handle".into(),
                    name: "x".into(),
                    type_name: "Order".into(),
                    line: 8,
                },
                TypeBind {
                    scope: "handle".into(),
                    name: "x".into(),
                    type_name: "Invoice".into(),
                    line: 9,
                },
            ],
        );
        let calls = vec![Call {
            caller: Some("handle".into()),
            callee: "x.process".into(),
            line: 10,
            known_receiver: false,
            ..Default::default()
        }
        .finish()];
        let resolved = resolve_calls("w.py", &calls, &syms, &[], &idx, "repo");
        assert_eq!(resolved[0].callee_id, None);
        assert_eq!(resolved[0].class, ResolutionClass::UnresolvedLikelyInternal);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.type-narrow-import verifies=REQ-implement-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing exercises=impl.scc.resolve.type-narrow
    fn imported_type_bind_pins_without_spraying() {
        let mut idx = SymbolIndex::new("repo");
        let mut save = mk_symbol("Order.save", SymbolKind::Method);
        save.parent = Some("Order".into());
        idx.add_file(
            "orders.py",
            &[mk_symbol("Order", SymbolKind::Class), save],
        );
        let run = mk_symbol("run", SymbolKind::Function);
        idx.add_file("app.py", std::slice::from_ref(&run));
        idx.set_type_binds(
            "app.py",
            &[TypeBind {
                scope: "run".into(),
                name: "o".into(),
                type_name: "Ord".into(),
                line: 3,
            }],
        );
        let ri = ResolvedImport {
            local_file: "app.py".into(),
            module: "orders".into(),
            target: ImportTarget::Internal {
                file: "orders.py".into(),
                name_map: HashMap::new(),
                namespace: false,
            },
            names: vec![("Ord".into(), "Order".into())],
            line: 1,
        };
        let calls = vec![Call {
            caller: Some("run".into()),
            callee: "o.save".into(),
            line: 4,
            known_receiver: false,
            ..Default::default()
        }
        .finish()];
        let resolved = resolve_calls("app.py", &calls, &[run], &[ri], &idx, "repo");
        assert_eq!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "orders.py", "Order.save"))
        );
        assert_eq!(resolved[0].class, ResolutionClass::ResolvedInternal);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.type-narrow-extract verifies=REQ-implement-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing exercises=impl.scc.resolve.type-narrow
    fn python_extract_then_resolve_pins_constructor() {
        use crate::model::{LanguageExtractor, SourceFile};
        use crate::python::PythonExtractor;
        let src = "class Order:\n    def process(self):\n        pass\nclass Invoice:\n    def process(self):\n        pass\ndef handle():\n    x = Order()\n    return x.process()\n";
        let ef = PythonExtractor::default().extract(&SourceFile::new("w.py", src));
        assert!(
            ef.type_binds
                .iter()
                .any(|b| b.name == "x" && b.type_name == "Order"),
            "binds: {:?}",
            ef.type_binds
        );
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("w.py", &ef.symbols);
        idx.set_type_binds("w.py", &ef.type_binds);
        let resolved = resolve_calls("w.py", &ef.calls, &ef.symbols, &[], &idx, "repo");
        let hit = resolved
            .iter()
            .find(|c| c.callee_name == "x.process")
            .expect("x.process call");
        assert_eq!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "w.py", "Order.process"))
        );
        assert_ne!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "w.py", "Invoice.process"))
        );
        assert_eq!(hit.provenance, scc_core::Provenance::Extracted);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.field-type-narrow verifies=REQ-implement-phase-8-of-scc-x-ripwire-lessons-one-hop-field-type-narrowi exercises=impl.scc.resolve.field-type-narrow
    fn unique_field_type_pins_self_field_method() {
        let mut idx = SymbolIndex::new("repo");
        let mut run = mk_symbol("Svc.run", SymbolKind::Method);
        run.parent = Some("Svc".into());
        let mut order_p = mk_symbol("Order.process", SymbolKind::Method);
        order_p.parent = Some("Order".into());
        let mut inv_p = mk_symbol("Invoice.process", SymbolKind::Method);
        inv_p.parent = Some("Invoice".into());
        let mut init = mk_symbol("Svc.__init__", SymbolKind::Method);
        init.parent = Some("Svc".into());
        let syms = vec![
            mk_symbol("Order", SymbolKind::Class),
            order_p,
            mk_symbol("Invoice", SymbolKind::Class),
            inv_p,
            mk_symbol("Svc", SymbolKind::Class),
            init,
            run,
        ];
        idx.add_file("w.py", &syms);
        idx.set_type_binds(
            "w.py",
            &[TypeBind {
                scope: "Svc".into(),
                name: "owned".into(),
                type_name: "Order".into(),
                line: 8,
            }],
        );
        let calls = vec![Call {
            caller: Some("Svc.run".into()),
            callee: "self.owned.process".into(),
            line: 12,
            known_receiver: true,
            ..Default::default()
        }
        .finish()];
        let resolved = resolve_calls("w.py", &calls, &syms, &[], &idx, "repo");
        assert_eq!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "w.py", "Order.process"))
        );
        assert_ne!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "w.py", "Invoice.process"))
        );
        assert_eq!(resolved[0].recv, RecvKind::FieldOfSelf);
        assert_eq!(resolved[0].class, ResolutionClass::ResolvedInternal);
        assert_eq!(resolved[0].provenance, scc_core::Provenance::Extracted);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.field-type-tombstone verifies=REQ-implement-phase-8-of-scc-x-ripwire-lessons-one-hop-field-type-narrowi exercises=impl.scc.resolve.field-type-narrow
    fn two_field_types_do_not_pin() {
        let mut idx = SymbolIndex::new("repo");
        let mut run = mk_symbol("Svc.run", SymbolKind::Method);
        run.parent = Some("Svc".into());
        let mut order_p = mk_symbol("Order.process", SymbolKind::Method);
        order_p.parent = Some("Order".into());
        let mut inv_p = mk_symbol("Invoice.process", SymbolKind::Method);
        inv_p.parent = Some("Invoice".into());
        let syms = vec![
            mk_symbol("Order", SymbolKind::Class),
            order_p,
            mk_symbol("Invoice", SymbolKind::Class),
            inv_p,
            mk_symbol("Svc", SymbolKind::Class),
            run,
        ];
        idx.add_file("w.py", &syms);
        idx.set_type_binds(
            "w.py",
            &[
                TypeBind {
                    scope: "Svc".into(),
                    name: "x".into(),
                    type_name: "Order".into(),
                    line: 8,
                },
                TypeBind {
                    scope: "Svc".into(),
                    name: "x".into(),
                    type_name: "Invoice".into(),
                    line: 9,
                },
            ],
        );
        let calls = vec![Call {
            caller: Some("Svc.run".into()),
            callee: "self.x.process".into(),
            line: 10,
            known_receiver: true,
            ..Default::default()
        }
        .finish()];
        let resolved = resolve_calls("w.py", &calls, &syms, &[], &idx, "repo");
        assert_eq!(resolved[0].callee_id, None);
        assert_eq!(resolved[0].class, ResolutionClass::UnresolvedLikelyInternal);
        assert_eq!(resolved[0].recv, RecvKind::FieldOfSelf);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.field-type-chain-honest verifies=REQ-implement-phase-8-of-scc-x-ripwire-lessons-one-hop-field-type-narrowi exercises=impl.scc.resolve.field-type-narrow
    fn longer_field_chain_stays_unresolved_even_with_field_type() {
        let mut idx = SymbolIndex::new("repo");
        let mut run = mk_symbol("Svc.run", SymbolKind::Method);
        run.parent = Some("Svc".into());
        let mut order_exec = mk_symbol("Order.execute", SymbolKind::Method);
        order_exec.parent = Some("Order".into());
        let syms = vec![
            mk_symbol("Order", SymbolKind::Class),
            order_exec,
            mk_symbol("Svc", SymbolKind::Class),
            run,
        ];
        idx.add_file("w.py", &syms);
        idx.set_type_binds(
            "w.py",
            &[TypeBind {
                scope: "Svc".into(),
                name: "owned".into(),
                type_name: "Order".into(),
                line: 8,
            }],
        );
        let calls = vec![Call {
            caller: Some("Svc.run".into()),
            callee: "self.owned.db.execute".into(),
            line: 12,
            known_receiver: true,
            ..Default::default()
        }
        .finish()];
        let resolved = resolve_calls("w.py", &calls, &syms, &[], &idx, "repo");
        assert_eq!(resolved[0].callee_id, None, "must not pin Order.execute");
        assert_eq!(resolved[0].recv, RecvKind::FieldOfSelf);
        assert_eq!(resolved[0].class, ResolutionClass::UnresolvedLikelyInternal);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.field-type-extract verifies=REQ-implement-phase-8-of-scc-x-ripwire-lessons-one-hop-field-type-narrowi exercises=impl.scc.resolve.field-type-narrow
    fn python_extract_then_resolve_pins_self_field() {
        use crate::model::{LanguageExtractor, SourceFile};
        use crate::python::PythonExtractor;
        let src = "class Order:\n    def process(self):\n        pass\nclass Invoice:\n    def process(self):\n        pass\nclass Svc:\n    def __init__(self):\n        self.owned = Order()\n    def run(self):\n        return self.owned.process()\n";
        let ef = PythonExtractor::default().extract(&SourceFile::new("w.py", src));
        assert!(
            ef.type_binds
                .iter()
                .any(|b| b.scope == "Svc" && b.name == "owned" && b.type_name == "Order"),
            "binds: {:?}",
            ef.type_binds
        );
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("w.py", &ef.symbols);
        idx.set_type_binds("w.py", &ef.type_binds);
        let resolved = resolve_calls("w.py", &ef.calls, &ef.symbols, &[], &idx, "repo");
        let hit = resolved
            .iter()
            .find(|c| c.callee_name == "self.owned.process")
            .expect("self.owned.process call");
        assert_eq!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "w.py", "Order.process"))
        );
        assert_ne!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "w.py", "Invoice.process"))
        );
        assert_eq!(hit.provenance, scc_core::Provenance::Extracted);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.java.field-type-extract verifies=REQ-implement-phase-9-of-scc-x-ripwire-lessons-java-one-hop-field-type-na exercises=impl.scc.extract.java.field-type
    fn java_extract_then_resolve_pins_this_field() {
        use crate::java::JavaExtractor;
        use crate::model::{LanguageExtractor, SourceFile};
        let src = r#"
class Order { void process() {} }
class Invoice { void process() {} }
class Svc {
    Svc() { this.owned = new Order(); }
    void run() { this.owned.process(); }
    void chain() { this.owned.inner.process(); }
}
"#;
        let ef = JavaExtractor::default().extract(&SourceFile::new("W.java", src));
        assert!(
            ef.type_binds
                .iter()
                .any(|b| b.scope == "Svc" && b.name == "owned" && b.type_name == "Order"),
            "binds: {:?}",
            ef.type_binds
        );
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("W.java", &ef.symbols);
        idx.set_type_binds("W.java", &ef.type_binds);
        let resolved = resolve_calls("W.java", &ef.calls, &ef.symbols, &[], &idx, "repo");
        let hit = resolved
            .iter()
            .find(|c| c.callee_name == "this.owned.process")
            .expect("this.owned.process call");
        assert_eq!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "W.java", "Order.process"))
        );
        assert_ne!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "W.java", "Invoice.process"))
        );
        assert_eq!(hit.provenance, scc_core::Provenance::Extracted);
        assert_eq!(hit.recv, RecvKind::FieldOfThis);
        let chain = resolved
            .iter()
            .find(|c| c.callee_name == "this.owned.inner.process")
            .expect("longer chain call");
        assert_eq!(chain.callee_id, None, "longer chain must stay unresolved");
        assert_eq!(chain.recv, RecvKind::FieldOfThis);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.go.receiver-field-type-extract verifies=REQ-implement-phase-10-of-scc-x-ripwire-lessons-go-one-hop-receiver-field exercises=impl.scc.resolve.receiver-field-type-narrow
    fn go_extract_then_resolve_pins_receiver_field() {
        use crate::go::GoExtractor;
        use crate::model::{LanguageExtractor, SourceFile};
        let src = r#"
package app
type Order struct{}
func (o *Order) Process() {}
type Invoice struct{}
func (i *Invoice) Process() {}
type Svc struct { owned *Order }
func (s *Svc) Run() { s.owned.Process() }
func (s *Svc) Chain() { s.owned.inner.Process() }
"#;
        let ef = GoExtractor::default().extract(&SourceFile::new("w.go", src));
        assert!(
            ef.type_binds
                .iter()
                .any(|b| b.scope == "Svc" && b.name == "owned" && b.type_name == "Order"),
            "binds: {:?}",
            ef.type_binds
        );
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("w.go", &ef.symbols);
        idx.set_type_binds("w.go", &ef.type_binds);
        let resolved = resolve_calls("w.go", &ef.calls, &ef.symbols, &[], &idx, "repo");
        let hit = resolved
            .iter()
            .find(|c| c.callee_name == "s.owned.Process")
            .expect("s.owned.Process call");
        assert_eq!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "w.go", "Order.Process"))
        );
        assert_ne!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "w.go", "Invoice.Process"))
        );
        assert_eq!(hit.provenance, scc_core::Provenance::Extracted);
        assert_eq!(hit.recv, RecvKind::FieldOfVariable);
        let chain = resolved
            .iter()
            .find(|c| c.callee_name == "s.owned.inner.Process")
            .expect("longer chain call");
        assert_eq!(chain.callee_id, None, "longer chain must stay unresolved");
        assert_eq!(chain.recv, RecvKind::FieldOfVariable);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.go.receiver-field-type-tombstone verifies=REQ-implement-phase-10-of-scc-x-ripwire-lessons-go-one-hop-receiver-field exercises=impl.scc.resolve.receiver-field-type-narrow
    fn two_receiver_field_types_do_not_pin() {
        let mut idx = SymbolIndex::new("repo");
        let mut run = mk_symbol("Svc.Run", SymbolKind::Method);
        run.parent = Some("Svc".into());
        let mut order_p = mk_symbol("Order.Process", SymbolKind::Method);
        order_p.parent = Some("Order".into());
        let mut inv_p = mk_symbol("Invoice.Process", SymbolKind::Method);
        inv_p.parent = Some("Invoice".into());
        let syms = vec![
            mk_symbol("Order", SymbolKind::Type),
            order_p,
            mk_symbol("Invoice", SymbolKind::Type),
            inv_p,
            mk_symbol("Svc", SymbolKind::Type),
            run,
        ];
        idx.add_file("w.go", &syms);
        idx.set_type_binds(
            "w.go",
            &[
                TypeBind {
                    scope: "Svc".into(),
                    name: "owned".into(),
                    type_name: "Order".into(),
                    line: 1,
                },
                TypeBind {
                    scope: "Svc".into(),
                    name: "owned".into(),
                    type_name: "Invoice".into(),
                    line: 2,
                },
                TypeBind {
                    scope: "Svc.Run".into(),
                    name: "s".into(),
                    type_name: "Svc".into(),
                    line: 3,
                },
            ],
        );
        let calls = vec![Call {
            caller: Some("Svc.Run".into()),
            callee: "s.owned.Process".into(),
            line: 4,
            known_receiver: true,
            ..Default::default()
        }
        .finish()];
        let resolved = resolve_calls("w.go", &calls, &syms, &[], &idx, "repo");
        assert_eq!(resolved[0].callee_id, None, "tombstone must not pin");
        assert_eq!(resolved[0].recv, RecvKind::FieldOfVariable);
        assert_eq!(resolved[0].class, ResolutionClass::UnresolvedLikelyInternal);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.go.field-assign-tombstone verifies=REQ-implement-phase-13-of-scc-x-ripwire-lessons-1-go-extract-time-recei exercises=impl.scc.extract.go.field-assign
    fn go_conflicting_field_assignment_does_not_pin() {
        use crate::go::GoExtractor;
        use crate::model::{LanguageExtractor, SourceFile};
        let src = r#"
package app
type Order struct{}
func (o *Order) Process() {}
type Invoice struct{}
func (i *Invoice) Process() {}
type Svc struct { owned *Order }
func (s *Svc) Run() {
	s.owned = &Invoice{}
	s.owned.Process()
}
"#;
        let ef = GoExtractor::default().extract(&SourceFile::new("w.go", src));
        let types: Vec<_> = ef
            .type_binds
            .iter()
            .filter(|b| b.scope == "Svc" && b.name == "owned")
            .map(|b| b.type_name.as_str())
            .collect();
        assert!(
            types.contains(&"Order") && types.contains(&"Invoice"),
            "assignment must tombstone fuel: {types:?}"
        );
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("w.go", &ef.symbols);
        idx.set_type_binds("w.go", &ef.type_binds);
        let resolved = resolve_calls("w.go", &ef.calls, &ef.symbols, &[], &idx, "repo");
        let hit = resolved
            .iter()
            .find(|c| c.callee_name == "s.owned.Process")
            .expect("s.owned.Process call");
        assert_eq!(hit.callee_id, None, "conflicting assignment must not pin");
        assert_eq!(hit.recv, RecvKind::FieldOfVariable);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.go.field-assign-same verifies=REQ-implement-phase-13-of-scc-x-ripwire-lessons-1-go-extract-time-recei exercises=impl.scc.extract.go.field-assign
    fn go_same_type_field_assignment_still_pins() {
        use crate::go::GoExtractor;
        use crate::model::{LanguageExtractor, SourceFile};
        let src = r#"
package app
type Order struct{}
func (o *Order) Process() {}
type Invoice struct{}
func (i *Invoice) Process() {}
type Svc struct { owned *Order }
func (s *Svc) Run() {
	s.owned = &Order{}
	s.owned.Process()
}
"#;
        let ef = GoExtractor::default().extract(&SourceFile::new("w.go", src));
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("w.go", &ef.symbols);
        idx.set_type_binds("w.go", &ef.type_binds);
        let resolved = resolve_calls("w.go", &ef.calls, &ef.symbols, &[], &idx, "repo");
        let hit = resolved
            .iter()
            .find(|c| c.callee_name == "s.owned.Process")
            .expect("s.owned.Process call");
        assert_eq!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "w.go", "Order.Process"))
        );
        assert_ne!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "w.go", "Invoice.Process"))
        );
    }

    #[test]
    // trace:v1 id=test.scc.resolve.rust.field-type-extract verifies=REQ-implement-phase-11-of-scc-x-ripwire-lessons-rust-one-hop-self-field-t exercises=impl.scc.extract.rust.field-type
    fn rust_extract_then_resolve_pins_self_field() {
        use crate::model::{LanguageExtractor, SourceFile};
        use crate::rust::RustExtractor;
        let src = r#"
struct Order;
impl Order { fn process(&self) {} }
struct Invoice;
impl Invoice { fn process(&self) {} }
struct Svc { owned: Order }
impl Svc {
    fn run(&self) { self.owned.process(); }
    fn chain(&self) { self.owned.inner.process(); }
}
"#;
        let ef = RustExtractor::default().extract(&SourceFile::new("w.rs", src));
        assert!(
            ef.type_binds
                .iter()
                .any(|b| b.scope == "Svc" && b.name == "owned" && b.type_name == "Order"),
            "binds: {:?}",
            ef.type_binds
        );
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("w.rs", &ef.symbols);
        idx.set_type_binds("w.rs", &ef.type_binds);
        let resolved = resolve_calls("w.rs", &ef.calls, &ef.symbols, &[], &idx, "repo");
        let hit = resolved
            .iter()
            .find(|c| c.callee_name == "self.owned.process")
            .expect("self.owned.process call");
        assert_eq!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "w.rs", "Order.process"))
        );
        assert_ne!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "w.rs", "Invoice.process"))
        );
        assert_eq!(hit.provenance, scc_core::Provenance::Extracted);
        assert_eq!(hit.recv, RecvKind::FieldOfSelf);
        let chain = resolved
            .iter()
            .find(|c| c.callee_name == "self.owned.inner.process")
            .expect("longer chain call");
        assert_eq!(chain.callee_id, None, "longer chain must stay unresolved");
        assert_eq!(chain.recv, RecvKind::FieldOfSelf);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.rust.field-assign-tombstone verifies=REQ-implement-phase-14-of-scc-x-ripwire-lessons-rust-extract-time-self-fi exercises=impl.scc.extract.rust.field-assign
    fn rust_conflicting_field_assignment_does_not_pin() {
        use crate::model::{LanguageExtractor, SourceFile};
        use crate::rust::RustExtractor;
        let src = r#"
struct Order {}
impl Order { fn process(&self) {} }
struct Invoice {}
impl Invoice { fn process(&self) {} }
struct Svc { owned: Order }
impl Svc {
    fn run(&mut self) {
        self.owned = Invoice {};
        self.owned.process();
    }
}
"#;
        let ef = RustExtractor::default().extract(&SourceFile::new("w.rs", src));
        let types: Vec<_> = ef
            .type_binds
            .iter()
            .filter(|b| b.scope == "Svc" && b.name == "owned")
            .map(|b| b.type_name.as_str())
            .collect();
        assert!(
            types.contains(&"Order") && types.contains(&"Invoice"),
            "assignment must tombstone fuel: {types:?}"
        );
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("w.rs", &ef.symbols);
        idx.set_type_binds("w.rs", &ef.type_binds);
        let resolved = resolve_calls("w.rs", &ef.calls, &ef.symbols, &[], &idx, "repo");
        let hit = resolved
            .iter()
            .find(|c| c.callee_name == "self.owned.process")
            .expect("self.owned.process call");
        assert_eq!(hit.callee_id, None, "conflicting assignment must not pin");
        assert_eq!(hit.recv, RecvKind::FieldOfSelf);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.rust.field-assign-same verifies=REQ-implement-phase-14-of-scc-x-ripwire-lessons-rust-extract-time-self-fi exercises=impl.scc.extract.rust.field-assign
    fn rust_same_type_field_assignment_still_pins() {
        use crate::model::{LanguageExtractor, SourceFile};
        use crate::rust::RustExtractor;
        let src = r#"
struct Order {}
impl Order { fn process(&self) {} }
struct Invoice {}
impl Invoice { fn process(&self) {} }
struct Svc { owned: Order }
impl Svc {
    fn run(&mut self) {
        self.owned = Order {};
        self.owned.process();
    }
}
"#;
        let ef = RustExtractor::default().extract(&SourceFile::new("w.rs", src));
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("w.rs", &ef.symbols);
        idx.set_type_binds("w.rs", &ef.type_binds);
        let resolved = resolve_calls("w.rs", &ef.calls, &ef.symbols, &[], &idx, "repo");
        let hit = resolved
            .iter()
            .find(|c| c.callee_name == "self.owned.process")
            .expect("self.owned.process call");
        assert_eq!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "w.rs", "Order.process"))
        );
        assert_ne!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "w.rs", "Invoice.process"))
        );
    }

    #[test]
    // trace:v1 id=test.scc.resolve.java.unprefixed-field-type verifies=REQ-implement-phase-12-of-scc-x-ripwire-lessons-java-unprefixed-field-as exercises=impl.scc.resolve.unprefixed-field-type
    fn java_unprefixed_field_pins_and_local_shadows() {
        use crate::java::JavaExtractor;
        use crate::model::{LanguageExtractor, SourceFile};
        let src = r#"
class Order { void process() {} }
class Invoice { void process() {} }
class Svc {
    private Order owned;
    void run() { owned.process(); }
    void chain() { owned.inner.process(); }
    void shadowed() {
        Invoice owned = new Invoice();
        owned.process();
    }
}
"#;
        let ef = JavaExtractor::default().extract(&SourceFile::new("W.java", src));
        assert!(
            ef.type_binds
                .iter()
                .any(|b| b.scope == "Svc" && b.name == "owned" && b.type_name == "Order"),
            "field bind missing: {:?}",
            ef.type_binds
        );
        assert!(
            ef.type_binds.iter().any(|b| b.scope == "Svc.shadowed"
                && b.name == "owned"
                && b.type_name == "Invoice"),
            "local shadow bind missing: {:?}",
            ef.type_binds
        );
        assert!(
            ef.calls.iter().any(|c| c.callee == "owned.process"),
            "unprefixed call missing: {:?}",
            ef.calls.iter().map(|c| &c.callee).collect::<Vec<_>>()
        );
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("W.java", &ef.symbols);
        idx.set_type_binds("W.java", &ef.type_binds);
        let resolved = resolve_calls("W.java", &ef.calls, &ef.symbols, &[], &idx, "repo");
        let hit = resolved
            .iter()
            .find(|c| c.callee_name == "owned.process" && c.caller_id.contains("Svc.run"))
            .or_else(|| {
                resolved
                    .iter()
                    .filter(|c| c.callee_name == "owned.process")
                    .min_by_key(|c| c.line)
            })
            .expect("unprefixed owned.process");
        assert_eq!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "W.java", "Order.process"))
        );
        assert_ne!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "W.java", "Invoice.process"))
        );
        assert_eq!(hit.provenance, scc_core::Provenance::Extracted);
        assert_eq!(hit.recv, RecvKind::NamedVariable);
        let chain = resolved
            .iter()
            .find(|c| c.callee_name == "owned.inner.process")
            .expect("longer unprefixed chain");
        assert_eq!(chain.callee_id, None, "longer chain must stay unresolved");
        let shadowed = resolved
            .iter()
            .find(|c| c.callee_name == "owned.process" && c.line > hit.line)
            .or_else(|| {
                resolved
                    .iter()
                    .filter(|c| c.callee_name == "owned.process")
                    .max_by_key(|c| c.line)
            })
            .expect("shadowed owned.process");
        assert_eq!(
            shadowed.callee_id,
            Some(scc_core::symbol_id("repo", "W.java", "Invoice.process")),
            "local Invoice must win over field Order"
        );
    }

    #[test]
    // trace:v1 id=test.scc.resolve.java.unprefixed-field-python-safe verifies=REQ-implement-phase-12-of-scc-x-ripwire-lessons-java-unprefixed-field-as exercises=impl.scc.resolve.unprefixed-field-type
    fn python_unprefixed_name_does_not_use_java_field_rule() {
        let mut idx = SymbolIndex::new("repo");
        let mut run = mk_symbol("Svc.run", SymbolKind::Method);
        run.parent = Some("Svc".into());
        let mut order_p = mk_symbol("Order.process", SymbolKind::Method);
        order_p.parent = Some("Order".into());
        let mut inv_p = mk_symbol("Invoice.process", SymbolKind::Method);
        inv_p.parent = Some("Invoice".into());
        let syms = vec![
            mk_symbol("Order", SymbolKind::Class),
            order_p,
            mk_symbol("Invoice", SymbolKind::Class),
            inv_p,
            mk_symbol("Svc", SymbolKind::Class),
            run,
        ];
        idx.add_file("w.py", &syms);
        idx.set_type_binds(
            "w.py",
            &[TypeBind {
                scope: "Svc".into(),
                name: "owned".into(),
                type_name: "Order".into(),
                line: 1,
            }],
        );
        let calls = vec![Call {
            caller: Some("Svc.run".into()),
            callee: "owned.process".into(),
            line: 4,
            known_receiver: true,
            ..Default::default()
        }
        .finish()];
        let resolved = resolve_calls("w.py", &calls, &syms, &[], &idx, "repo");
        assert_eq!(
            resolved[0].callee_id,
            None,
            "Python owned.process must not pin via Java field-as-receiver"
        );
        assert_eq!(resolved[0].recv, RecvKind::NamedVariable);
    }
}
