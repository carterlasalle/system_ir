//! Import normalization and cross-file call resolution (SCC-025, SCC-026).
//!
//! Resolution rules, in priority order:
//! 1. local symbol in the same file;
//! 2. imported member (`import { a as b }` / `from m import a`) resolved to
//!    the target file's exported symbol;
//! 3. imported module namespace (`import * as ns`, `import m`) → member on
//!    the target file;
//! 4. `self`/`this` receiver → sibling method of the enclosing class;
//! 5. external import root → `external_api` entity;
//! 6. otherwise: unresolved candidate (dropped from the graph, counted).

use crate::model::{Call, Import, ImportType, Symbol, SymbolKind};
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
}

/// Symbol index for one file.
#[derive(Debug, Clone, Default)]
pub struct FileSymbols {
    /// name -> (symbol, entity id)
    pub by_name: BTreeMap<String, (Symbol, String)>,
    /// methods: "ClassName.method" -> entity id (for self/this resolution)
    pub methods: BTreeMap<String, (String, String)>, // key -> (class, entity id)
    pub file_entity_id: String,
}

/// Module resolution result for an import.
#[derive(Debug, Clone)]
pub enum ImportTarget {
    /// Resolved to a file in the repo. `name_map`: local name -> exported name.
    Internal { file: String, name_map: HashMap<String, String>, namespace: bool },
    /// Bare specifier treated as external system/api.
    External { name: String },
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
                None => ImportTarget::External {
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

/// Resolve all calls in one file against the full symbol index.
/// `self_caller` maps `self`/`this` calls to the enclosing class.
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
        let caller_id = caller_ctx(call);
        let (root, rest): (&str, &str) = match call.callee.split_once('.') {
            Some((r, rest)) => (r, rest),
            None => (call.callee.as_str(), ""),
        };

        // 1. self/this → sibling method
        if root == "self" || root == "this" {
            let target = rest.split('.').next().unwrap_or("");
            if !target.is_empty() {
                // caller's class
                let class = call.caller.as_deref().map(|c| {
                    let base = c.split('.').next().unwrap_or(c);
                    if let Some((class, _id)) = index
                        .files
                        .get(path)
                        .and_then(|fs| fs.methods.get(c))
                    {
                        class.clone()
                    } else {
                        base.to_string()
                    }
                });
                if let Some(class) = class {
                    if let Some(m) = methods_by_class.get(class.as_str()).and_then(|ms| {
                        ms.iter().find(|s| s.name == format!("{class}.{target}"))
                    }) {
                        out.push(ResolvedCall {
                            caller_id,
                            callee_id: Some(scc_core::symbol_id(repo_id, path, &m.name)),
                            callee_name: call.callee.clone(),
                            provenance: scc_core::Provenance::Extracted,
                            confidence: 0.98,
                            line: call.line,
                        });
                        continue;
                    }
                }
            }
            out.push(ResolvedCall {
                caller_id,
                callee_id: None,
                callee_name: call.callee.clone(),
                provenance: scc_core::Provenance::Extracted,
                confidence: 0.5,
                line: call.line,
            });
            continue;
        }

        // 2. local symbol
        if let Some(sym) = local.get(root) {
            if sym.kind.is_callable() {
                out.push(ResolvedCall {
                    caller_id,
                    callee_id: Some(scc_core::symbol_id(repo_id, path, &sym.name)),
                    callee_name: call.callee.clone(),
                    provenance: scc_core::Provenance::Extracted,
                    confidence: 0.99,
                    line: call.line,
                });
                continue;
            }
        }

        // 3. imported member binding
        if let Some((target_file, exported)) = binding.get(root) {
            if let Some(external) = target_file.strip_prefix("external:") {
                out.push(ResolvedCall {
                    caller_id,
                    callee_id: Some(scc_core::entity_id(
                        repo_id,
                        scc_core::kinds::EXTERNAL_API,
                        external,
                    )),
                    callee_name: call.callee.clone(),
                    provenance: scc_core::Provenance::Extracted,
                    confidence: 0.8,
                    line: call.line,
                });
                continue;
            }
            let member = if rest.is_empty() {
                None
            } else {
                Some(rest.split('.').next().unwrap_or(""))
            };
            let callee_id = match index.files.get(target_file) {
                Some(fs) => {
                    if let Some(member) = member {
                        // method on an imported class: `Exported.member`
                        if let Some((_, id)) = fs.by_name.get(&format!("{exported}.{member}")) {
                            Some(id.clone())
                        } else {
                            // fall back to the imported symbol itself
                            fs.by_name.get(exported).map(|(_, id)| id.clone())
                        }
                    } else {
                        fs.by_name.get(exported).map(|(_, id)| id.clone())
                    }
                }
                None => None,
            };
            out.push(ResolvedCall {
                caller_id,
                callee_id,
                callee_name: call.callee.clone(),
                provenance: scc_core::Provenance::Extracted,
                confidence: 0.95,
                line: call.line,
            });
            continue;
        }

        // 4. namespace import member (`import * as ns`, python `import m`)
        if let Some(ns_file) = namespaces.get(root) {
            if !rest.is_empty() {
                let member = rest.split('.').next().unwrap_or("");
                if let Some((_, id)) = index
                    .files
                    .get(ns_file)
                    .and_then(|fs| fs.by_name.get(member))
                {
                    out.push(ResolvedCall {
                        caller_id,
                        callee_id: Some(id.clone()),
                        callee_name: call.callee.clone(),
                        provenance: scc_core::Provenance::Extracted,
                        confidence: 0.97,
                        line: call.line,
                    });
                    continue;
                }
            }
            out.push(ResolvedCall {
                caller_id,
                callee_id: None,
                callee_name: call.callee.clone(),
                provenance: scc_core::Provenance::Extracted,
                confidence: 0.5,
                line: call.line,
            });
            continue;
        }

        // 5. local class called with method (e.g. `Logger.error(...)`)
        if !rest.is_empty() {
            let member = rest.split('.').next().unwrap_or("");
            if let Some(sym) = local.get(root) {
                if let Some((_, mid)) = index
                    .files
                    .get(path)
                    .and_then(|fs| fs.methods.get(&format!("{}.{}", sym.name, member)))
                {
                    out.push(ResolvedCall {
                        caller_id,
                        callee_id: Some(mid.clone()),
                        callee_name: call.callee.clone(),
                        provenance: scc_core::Provenance::Extracted,
                        confidence: 0.9,
                        line: call.line,
                    });
                    continue;
                }
            }
        }

        // 6. unknown
        out.push(ResolvedCall {
            caller_id,
            callee_id: None,
            callee_name: call.callee.clone(),
            provenance: scc_core::Provenance::Extracted,
            confidence: 0.5,
            line: call.line,
        });
    }
    out
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

    fn mk_symbol(name: &str, kind: SymbolKind) -> Symbol {
        Symbol {
            name: name.into(),
            kind,
            signature: None,
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
}
