//! Import normalization and cross-file call resolution (SCC-025, SCC-026).
//!
//! Resolution rules, in priority order:
//! 1. receiver classification (this/self vs named vs field-chain vs type);
//! 2. unique typed local/param (Rule 2) then class-name receiver `Cls.m()`
//!    (Rule 2c: unique class-like def, local/param names veto, no spray;
//!    CHA/base walk when `Cls` itself does not define `m`);
//! 3. function-alias bind for bare `f()` (local then file-scope; never fall
//!    back to a same-named global);
//! 4. local symbol in the same file;
//! 5. imported member (`import { a as b }` / `from m import a`) resolved to
//!    the target file's exported symbol;
//! 6. imported module namespace (`import * as ns`, `import m`) → member on
//!    the target file;
//! 7. Rule 3 include-file: path-precise Internal import files; pin a
//!    bare name when exactly one imported file defines that callable
//!    (same-file defs stay on the local ladder; 0 or ≥2 imported
//!    defining files stay unresolved; never basename-guess);
//! 8. `self`/`this` sibling methods, else unique class-like / CHA on the
//!    enclosing class; `super` walks bases only. Plus one-hop `self.field.m()` / `this.field.m()` when the field type is unique;
//! 9. confirmed external import root → `external_api` entity;
//! 10. otherwise: unresolved (counted; never silently treated as external).
//!
//! Step-A import files are language-gated unique-or-degrade (Ripwire analog):
//! `.rs` → `crate::` / `super::` / `self::` / `mod:x`; `.py` → `mod.py` or
//! `mod/__init__.py` (relative: includer dir; absolute: file + repo-root +
//! unique source-root fallback); `.ts`/`.js` → relative `./`/`../` only
//! (bare specifiers stay External); `.go` → unique `{path}.go` or production
//! files in `{path}/` (multi-file packages expand); `.java` → unique
//! `dots/to/Type.java` (star imports contribute nothing); C/C++ quote
//! `#include "foo.h"` joins the includer directory (exact hit, never
//! basename-guess); angle `#include <stdio.h>` stays External. Two distinct
//! package identities or type files contribute nothing. Go/Java never steal
//! `.py`/`.ts`.
//!
//! Native resolution is EXTRACTED (candidate), never RESOLVED.

use crate::model::{Call, FnBind, Import, ImportType, Symbol, SymbolKind, TypeBind};
use crate::recv::{classify_callee, split_recv_path, RecvFact};
use scc_core::{RecvKind, ReferenceKind, ResolutionClass};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

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
    /// Function-alias binds for this file (`f = helper`).
    pub fn_binds: Vec<FnBind>,
    /// Simple-ident bases for class-like symbols in this file.
    pub class_bases: BTreeMap<String, Vec<String>>,
}

/// Module resolution result for an import.
#[derive(Debug, Clone)]
pub enum ImportTarget {
    /// Resolved to a file in the repo. `name_map`: local name -> exported name.
    Internal {
        file: String,
        name_map: HashMap<String, String>,
        namespace: bool,
    },
    /// Bare specifier treated as external system/api.
    External { name: String },
    /// Relative or project-looking specifier that did not resolve to a file.
    /// Not evidence that the target is outside the repository.
    Unresolved { name: String },
}

/// Unique-or-degrade probe: 0 hits, exactly one distinct file, or ≥2 files.
enum UniqueHit {
    None,
    One(String),
    Ambiguous,
}

enum GoPkgHit {
    None,
    Package(Vec<String>),
    Ambiguous,
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

    /// Attach extract-time function-alias binds used by bare `f()` pinning.
    pub fn set_fn_binds(&mut self, path: &str, binds: &[FnBind]) {
        if let Some(fs) = self.files.get_mut(path) {
            fs.fn_binds = binds.to_vec();
        }
    }

    /// Attach extract-time class heritage used by Rule 2c CHA.
    pub fn set_class_bases(&mut self, path: &str, bases: &[(String, Vec<String>)]) {
        if let Some(fs) = self.files.get_mut(path) {
            fs.class_bases = bases.iter().cloned().collect();
        }
    }

    pub fn file(&self, path: &str) -> Option<&FileSymbols> {
        self.files.get(path)
    }

    /// Resolve an import statement to a file in the repo.
    pub fn resolve_import(&self, from_file: &str, import: &Import) -> ImportTarget {
        if from_file.ends_with(".rs") {
            return self.resolve_rust_import_target(from_file, import);
        }
        if from_file.ends_with(".py") {
            return self.resolve_python_import_target(from_file, import);
        }
        if is_typescript_file(from_file) {
            return self.resolve_ts_import_target(from_file, import);
        }
        if from_file.ends_with(".go") {
            return self.resolve_go_import_target(from_file, import);
        }
        if from_file.ends_with(".java") {
            return self.resolve_java_import_target(from_file, import);
        }
        if is_cfamily_file(from_file) {
            return self.resolve_c_include_target(from_file, import);
        }
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
            for root in SOURCE_ROOTS {
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

    fn resolve_rust_import_target(&self, from_file: &str, import: &Import) -> ImportTarget {
        match self.resolve_rust_import(from_file, &import.module) {
            Some(file) => ImportTarget::Internal {
                file,
                name_map: HashMap::new(),
                namespace: import.r#type == ImportType::Module,
            },
            None if rust_is_project_import(&import.module) => ImportTarget::Unresolved {
                name: import.module.clone(),
            },
            None => ImportTarget::External {
                name: import.module.clone(),
            },
        }
    }

    /// Ripwire `resolveRustImport`: unique-or-degrade path-precise Step-A.
    /// `crate::` / `super::` / `self::` / `mod:x` map to exactly one `.rs` or
    /// `/mod.rs`. Two hits (e.g. `a.rs` and `a/mod.rs`) degrade. Bare/`std::`
    /// paths are not project imports.
    // trace:v1 id=impl.scc.resolve.rust-import work=WORK-phase-27-of-scc-x-ripwire-lessons-absorb-rust-step-a-path-precise-impor satisfies=REQ-implement-phase-27-of-scc-x-ripwire-lessons-absorb-rust-step-a-path-p implements=PLAN-phase-27-of-scc-x-ripwire-lessons-absorb-rust-step-a-path-precise-impor
    fn resolve_rust_import(&self, from_file: &str, target: &str) -> Option<String> {
        if target.is_empty() || target.contains(['{', '}', ',', ' ']) {
            return None;
        }
        let dir = from_file.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        if let Some(mod_name) = target.strip_prefix("mod:") {
            if mod_name.is_empty() || mod_name.contains(':') {
                return None;
            }
            return self.rust_unique_file([
                rust_join(dir, &format!("{mod_name}.rs")),
                rust_join(dir, &format!("{mod_name}/mod.rs")),
            ]);
        }
        let segs: Vec<&str> = target.split("::").collect();
        if segs.is_empty() || segs.iter().any(|s| s.is_empty()) {
            return None;
        }
        let (base, path): (String, &[&str]) = match segs[0] {
            "crate" => (self.rust_crate_root_dir()?, &segs[1..]),
            "self" => (dir.to_string(), &segs[1..]),
            "super" => (rust_join(dir, ".."), &segs[1..]),
            _ => return None,
        };
        if path.is_empty() {
            return None;
        }
        let full = path.join("/");
        let mut cands = vec![
            rust_join(&base, &format!("{full}.rs")),
            rust_join(&base, &format!("{full}/mod.rs")),
        ];
        if path.len() >= 2 {
            let parent = path[..path.len() - 1].join("/");
            cands.push(rust_join(&base, &format!("{parent}.rs")));
            cands.push(rust_join(&base, &format!("{parent}/mod.rs")));
        }
        self.rust_unique_file(cands)
    }

    fn rust_crate_root_dir(&self) -> Option<String> {
        let mut files: Vec<&str> = self.all_files.iter().map(String::as_str).collect();
        files.sort_unstable();
        for p in files {
            let is_lib = p == "lib.rs" || p.ends_with("/lib.rs");
            let is_main = p == "main.rs" || p.ends_with("/main.rs");
            if is_lib || is_main {
                return Some(
                    p.rsplit_once('/')
                        .map(|(d, _)| d.to_string())
                        .unwrap_or_default(),
                );
            }
        }
        None
    }

    fn rust_unique_file(&self, candidates: impl IntoIterator<Item = String>) -> Option<String> {
        match self.unique_existing(candidates) {
            UniqueHit::One(f) => Some(f),
            _ => None,
        }
    }

    fn unique_existing(&self, candidates: impl IntoIterator<Item = String>) -> UniqueHit {
        let mut hit: Option<String> = None;
        for raw in candidates {
            let c = normalize_module_path(&raw);
            if c.is_empty() || !self.all_files.contains(&c) {
                continue;
            }
            match &hit {
                None => hit = Some(c),
                Some(prev) if prev != &c => return UniqueHit::Ambiguous,
                Some(_) => {}
            }
        }
        match hit {
            Some(f) => UniqueHit::One(f),
            None => UniqueHit::None,
        }
    }

    fn resolve_python_import_target(&self, from_file: &str, import: &Import) -> ImportTarget {
        import_hit_to_target(
            self.resolve_python_import(from_file, &import.module),
            &import.module,
            import.r#type == ImportType::Module,
            !import.module.starts_with('.'),
        )
    }

    /// Ripwire `resolvePythonImport`: unique-or-degrade path-precise Step-A.
    /// Dots→slashes; probe `mod.py` then `mod/__init__.py`. Relative: includer
    /// dir only. Absolute: file-relative, repo-root, and unique SCC source-root
    /// fallbacks as one set. Two hits (e.g. `pkg.py` and `pkg/__init__.py`)
    /// degrade. Never steal `.ts`/`.js` files.
    // trace:v1 id=impl.scc.resolve.python-import work=WORK-phase-28-of-scc-x-ripwire-lessons-absorb-python-and-type-script-step-a-p satisfies=REQ-implement-phase-28-of-scc-x-ripwire-lessons-absorb-python-and-type-scr implements=PLAN-phase-28-of-scc-x-ripwire-lessons-absorb-python-and-type-script-step-a-p
    fn resolve_python_import(&self, from_file: &str, target: &str) -> UniqueHit {
        if target.is_empty() {
            return UniqueHit::None;
        }
        let dir = from_file.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        if target.starts_with("./") || target.starts_with("../") {
            let joined = normalize_module_path(&rust_join(dir, target));
            if joined.ends_with(".py") {
                return self.unique_existing(std::iter::once(joined));
            }
            return self.unique_existing(python_probe_pair("", "", &joined));
        }
        let n_dots = target.bytes().take_while(|&b| b == b'.').count();
        let tail = &target[n_dots..];
        let mod_path = tail.replace('.', "/");
        let is_relative = n_dots > 0;
        let rel_prefix = "../".repeat(n_dots.saturating_sub(1));
        let mut cands = Vec::new();
        cands.extend(python_probe_pair(dir, &rel_prefix, &mod_path));
        if !is_relative {
            cands.extend(python_probe_pair("", "", &mod_path));
            for root in SOURCE_ROOTS {
                cands.extend(python_probe_pair(root, "", &mod_path));
            }
        }
        self.unique_existing(cands)
    }

    fn resolve_ts_import_target(&self, from_file: &str, import: &Import) -> ImportTarget {
        if !import.module.starts_with('.') {
            return ImportTarget::External {
                name: import.module.clone(),
            };
        }
        import_hit_to_target(
            self.resolve_ts_import(from_file, &import.module),
            &import.module,
            import.r#type == ImportType::Module,
            false,
        )
    }

    /// Ripwire `resolveTsImport`: unique-or-degrade path-precise Step-A.
    /// Relative `./x` / `../a/b` probe exact, then a fixed extension list,
    /// then index files, relative-to-includer. Bare specifiers are External
    /// (no tsconfig aliases). Two hits (e.g. `x.ts` and `x/index.ts`) degrade.
    /// Never steal `.py` files.
    // trace:v1 id=impl.scc.resolve.ts-import work=WORK-phase-28-of-scc-x-ripwire-lessons-absorb-python-and-type-script-step-a-p satisfies=REQ-implement-phase-28-of-scc-x-ripwire-lessons-absorb-python-and-type-scr implements=PLAN-phase-28-of-scc-x-ripwire-lessons-absorb-python-and-type-script-step-a-p
    fn resolve_ts_import(&self, from_file: &str, target: &str) -> UniqueHit {
        if target.is_empty() || !target.starts_with('.') {
            return UniqueHit::None;
        }
        let dir = from_file.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let mut cands = Vec::with_capacity(1 + TS_FILE_EXTS.len() + TS_INDEX_RELS.len());
        cands.push(rust_join(dir, target));
        for ext in TS_FILE_EXTS {
            cands.push(rust_join(dir, &format!("{target}{ext}")));
        }
        for rel in TS_INDEX_RELS {
            cands.push(rust_join(dir, &format!("{target}{rel}")));
        }
        self.unique_existing(cands)
    }

    /// Expand one extracted import to one or more resolve targets.
    /// Go packages with several production `.go` files become one Internal
    /// per file so Rule 3 sees the whole package. Other languages stay 1:1.
    // trace:v1 id=impl.scc.resolve.import-expanded work=WORK-phase-29-of-scc-x-ripwire-lessons-absorb-remaining-language-gated-step satisfies=REQ-implement-phase-29-of-scc-x-ripwire-lessons-absorb-remaining-language implements=PLAN-phase-29-of-scc-x-ripwire-lessons-absorb-remaining-language-gated-step
    pub fn resolve_import_expanded(&self, from_file: &str, import: &Import) -> Vec<ImportTarget> {
        if from_file.ends_with(".go") {
            return self.resolve_go_import_targets(from_file, import);
        }
        vec![self.resolve_import(from_file, import)]
    }

    pub fn resolved_imports(&self, from_file: &str, imports: &[Import]) -> Vec<ResolvedImport> {
        imports
            .iter()
            .flat_map(|imp| {
                self.resolve_import_expanded(from_file, imp)
                    .into_iter()
                    .map(|target| ResolvedImport {
                        local_file: from_file.to_string(),
                        module: imp.module.clone(),
                        names: imp.names.clone(),
                        line: imp.line,
                        target,
                    })
            })
            .collect()
    }

    fn resolve_go_import_target(&self, from_file: &str, import: &Import) -> ImportTarget {
        go_pkg_hit_to_target(self.resolve_go_package(from_file, &import.module), import)
    }

    fn resolve_go_import_targets(&self, from_file: &str, import: &Import) -> Vec<ImportTarget> {
        match self.resolve_go_package(from_file, &import.module) {
            GoPkgHit::Package(files) => files
                .into_iter()
                .map(|file| ImportTarget::Internal {
                    file,
                    name_map: HashMap::new(),
                    namespace: import.r#type == ImportType::Module,
                })
                .collect(),
            other => vec![go_pkg_hit_to_target(other, import)],
        }
    }

    /// Go package Step-A: unique `{path}.go` or the production `.go` files
    /// directly in `{path}/`. Both layouts together degrade. Never steal
    /// `.py`/`.ts`. No `go.mod`, no module-path suffix guessing.
    // trace:v1 id=impl.scc.resolve.go-import work=WORK-phase-29-of-scc-x-ripwire-lessons-absorb-remaining-language-gated-step satisfies=REQ-implement-phase-29-of-scc-x-ripwire-lessons-absorb-remaining-language implements=PLAN-phase-29-of-scc-x-ripwire-lessons-absorb-remaining-language-gated-step
    fn resolve_go_package(&self, from_file: &str, target: &str) -> GoPkgHit {
        if target.is_empty() {
            return GoPkgHit::None;
        }
        let dir = from_file.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        if target.starts_with("./") || target.starts_with("../") {
            let joined = normalize_module_path(&rust_join(dir, target));
            return self.go_consider_path(&joined);
        }
        let mut hit = self.go_consider_path(target);
        for root in SOURCE_ROOTS {
            hit = merge_go_pkg(hit, self.go_consider_path(&format!("{root}/{target}")));
        }
        hit
    }

    fn go_consider_path(&self, path: &str) -> GoPkgHit {
        let path = normalize_module_path(path);
        if path.is_empty() {
            return GoPkgHit::None;
        }
        let file = format!("{path}.go");
        let file_hit = is_go_prod_file(&file) && self.all_files.contains(&file);
        let dir_files = self.go_prod_files_in_dir(&path);
        match (file_hit, dir_files.is_empty()) {
            (true, true) => GoPkgHit::Package(vec![file]),
            (false, false) => GoPkgHit::Package(dir_files),
            (true, false) => GoPkgHit::Ambiguous,
            (false, true) => GoPkgHit::None,
        }
    }

    fn go_prod_files_in_dir(&self, dir: &str) -> Vec<String> {
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let mut files: Vec<String> = self
            .all_files
            .iter()
            .filter(|p| {
                is_go_prod_file(p)
                    && matches!(
                        p.strip_prefix(&prefix),
                        Some(rest) if !rest.is_empty() && !rest.contains('/')
                    )
            })
            .cloned()
            .collect();
        files.sort();
        files
    }

    fn resolve_java_import_target(&self, _from_file: &str, import: &Import) -> ImportTarget {
        if import.module.ends_with(".*") {
            return ImportTarget::Unresolved {
                name: import.module.clone(),
            };
        }
        import_hit_to_target(
            self.resolve_java_import(&import.module),
            &import.module,
            import.r#type == ImportType::Module,
            true,
        )
    }

    /// Java type-import Step-A: `com.foo.Bar` → unique `com/foo/Bar.java`.
    /// Static `com.foo.Bar.BAZ` can pin `Bar.java` when the member file
    /// misses. Star imports contribute nothing. Never steal `.py`/`.ts`.
    /// Not namespace-as-file.
    // trace:v1 id=impl.scc.resolve.java-import work=WORK-phase-29-of-scc-x-ripwire-lessons-absorb-remaining-language-gated-step satisfies=REQ-implement-phase-29-of-scc-x-ripwire-lessons-absorb-remaining-language implements=PLAN-phase-29-of-scc-x-ripwire-lessons-absorb-remaining-language-gated-step
    fn resolve_java_import(&self, target: &str) -> UniqueHit {
        if target.is_empty() {
            return UniqueHit::None;
        }
        let path = target.replace('.', "/");
        let specific = self.unique_existing(java_file_candidates(&path));
        if !matches!(specific, UniqueHit::None) {
            return specific;
        }
        match path.rsplit_once('/') {
            Some((parent, _)) if !parent.is_empty() => {
                self.unique_existing(java_file_candidates(parent))
            }
            _ => UniqueHit::None,
        }
    }

    /// C-family `#include` Step-A (Ripwire `includeLangOf` CFamily +
    /// `joinNormalizeLookup`). Quote `"foo.h"` is an exact lexical join with
    /// the includer directory — a hit pins, a miss is Unresolved, never a
    /// basename guess. Angle `<stdio.h>` is External even when a same-named
    /// header exists in-repo. Direct includes only (no `-I`, no
    /// `compile_commands.json`, no transitive closure).
    // trace:v1 id=impl.scc.resolve.c-include work=WORK-phase-30-of-scc-x-ripwire-lessons-absorb-c-and-c-extractors-with-path satisfies=REQ-implement-phase-30-of-scc-x-ripwire-lessons-absorb-c-and-c-extracto implements=PLAN-phase-30-of-scc-x-ripwire-lessons-absorb-c-and-c-extractors-with-path
    fn resolve_c_include_target(&self, from_file: &str, import: &Import) -> ImportTarget {
        let module = import.module.trim();
        if module.len() >= 2 && module.starts_with('<') && module.ends_with('>') {
            return ImportTarget::External {
                name: import.module.clone(),
            };
        }
        let relative = module.trim_matches('"');
        if relative.is_empty() {
            return ImportTarget::Unresolved {
                name: import.module.clone(),
            };
        }
        let dir = from_file.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let joined = normalize_module_path(&rust_join(dir, relative));
        if joined.is_empty() {
            return ImportTarget::Unresolved {
                name: import.module.clone(),
            };
        }
        if self.all_files.contains(&joined) {
            ImportTarget::Internal {
                file: joined,
                name_map: HashMap::new(),
                namespace: import.r#type == ImportType::Module,
            }
        } else {
            ImportTarget::Unresolved {
                name: import.module.clone(),
            }
        }
    }
}

fn rust_is_project_import(module: &str) -> bool {
    module.starts_with("mod:")
        || module == "crate"
        || module.starts_with("crate::")
        || module == "self"
        || module.starts_with("self::")
        || module == "super"
        || module.starts_with("super::")
}

fn rust_join(base: &str, rel: &str) -> String {
    if base.is_empty() {
        rel.to_string()
    } else {
        format!("{base}/{rel}")
    }
}

const SOURCE_ROOTS: [&str; 7] = [
    "src", "svc", "lib", "app", "services", "service", "packages",
];

const TS_FILE_EXTS: [&str; 7] = [".ts", ".tsx", ".d.ts", ".js", ".jsx", ".mjs", ".cjs"];
const TS_INDEX_RELS: [&str; 4] = ["/index.ts", "/index.tsx", "/index.js", "/index.jsx"];

fn is_typescript_file(path: &str) -> bool {
    path.ends_with(".ts")
        || path.ends_with(".tsx")
        || path.ends_with(".js")
        || path.ends_with(".jsx")
        || path.ends_with(".mjs")
        || path.ends_with(".cjs")
}

fn is_cfamily_file(path: &str) -> bool {
    path.ends_with(".c")
        || path.ends_with(".h")
        || path.ends_with(".cc")
        || path.ends_with(".cpp")
        || path.ends_with(".cxx")
        || path.ends_with(".hpp")
        || path.ends_with(".hh")
        || path.ends_with(".hxx")
}

fn python_probe_pair(base: &str, rel_prefix: &str, mod_path: &str) -> [String; 2] {
    let py = rust_join(base, &format!("{rel_prefix}{mod_path}.py"));
    let init = if mod_path.is_empty() {
        rust_join(base, &format!("{rel_prefix}__init__.py"))
    } else {
        rust_join(base, &format!("{rel_prefix}{mod_path}/__init__.py"))
    };
    [py, init]
}

fn import_hit_to_target(
    hit: UniqueHit,
    module: &str,
    namespace: bool,
    miss_is_external: bool,
) -> ImportTarget {
    match hit {
        UniqueHit::One(file) => ImportTarget::Internal {
            file,
            name_map: HashMap::new(),
            namespace,
        },
        UniqueHit::Ambiguous => ImportTarget::Unresolved {
            name: module.to_string(),
        },
        UniqueHit::None if miss_is_external => ImportTarget::External {
            name: module.to_string(),
        },
        UniqueHit::None => ImportTarget::Unresolved {
            name: module.to_string(),
        },
    }
}

fn go_pkg_hit_to_target(hit: GoPkgHit, import: &Import) -> ImportTarget {
    match hit {
        GoPkgHit::Package(files) => match files.as_slice() {
            [file] => ImportTarget::Internal {
                file: file.clone(),
                name_map: HashMap::new(),
                namespace: import.r#type == ImportType::Module,
            },
            _ => ImportTarget::Unresolved {
                name: import.module.clone(),
            },
        },
        GoPkgHit::Ambiguous => ImportTarget::Unresolved {
            name: import.module.clone(),
        },
        GoPkgHit::None => ImportTarget::External {
            name: import.module.clone(),
        },
    }
}

/// Unique-or-degrade a namespace member across one or more Internal files.
/// Multi-file Go packages expand to several files; last-write must not win.
fn unique_namespace_member(
    index: &SymbolIndex,
    files: &[String],
    method: &str,
) -> Option<String> {
    if method.is_empty() {
        return None;
    }
    let mut found: Option<String> = None;
    for file in files {
        let Some((_, id)) = index
            .files
            .get(file.as_str())
            .and_then(|fs| fs.by_name.get(method))
        else {
            continue;
        };
        if found.as_ref().is_some_and(|existing| existing != id) {
            return None;
        }
        found = Some(id.clone());
    }
    found
}

fn merge_go_pkg(a: GoPkgHit, b: GoPkgHit) -> GoPkgHit {
    match (a, b) {
        (GoPkgHit::None, x) | (x, GoPkgHit::None) => x,
        (GoPkgHit::Ambiguous, _) | (_, GoPkgHit::Ambiguous) => GoPkgHit::Ambiguous,
        (GoPkgHit::Package(fa), GoPkgHit::Package(fb)) if fa == fb => GoPkgHit::Package(fa),
        (GoPkgHit::Package(_), GoPkgHit::Package(_)) => GoPkgHit::Ambiguous,
    }
}

fn is_go_prod_file(path: &str) -> bool {
    path.ends_with(".go") && !path.ends_with("_test.go")
}

const JAVA_SOURCE_ROOTS: [&str; 2] = ["src/main/java", "src/test/java"];

fn java_file_candidates(path: &str) -> Vec<String> {
    let mut out = vec![format!("{path}.java")];
    for root in SOURCE_ROOTS {
        out.push(format!("{root}/{path}.java"));
    }
    for root in JAVA_SOURCE_ROOTS {
        out.push(format!("{root}/{path}.java"));
    }
    out
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
    namespaces: &HashMap<&str, Vec<String>>,
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
    if let Some(ns_files) = namespaces.get(root) {
        let ns_file = match ns_files.as_slice() {
            [f] => f.as_str(),
            _ => return None,
        };
        if let Some(fs) = index.files.get(ns_file) {
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
    // namespace imports: local ns name -> target file(s). Go packages expand
    // to one Internal per production file; unique-or-degrade across them.
    let mut namespaces: HashMap<&str, Vec<String>> = HashMap::new();
    for ri in resolved_imports {
        match &ri.target {
            ImportTarget::Internal {
                file, namespace, ..
            } => {
                if *namespace {
                    // `import * as ns from 'm'`, python `import a.b [as c]`:
                    // local name binds the module
                    for (local, imported) in &ri.names {
                        if imported == "default" {
                            // `import x from 'm'` binds the default export
                            let exported = default_symbol_name(index, file);
                            binding.insert(local.as_str(), (file.clone(), exported));
                        } else {
                            let files = namespaces.entry(local.as_str()).or_default();
                            if !files.iter().any(|f| f == file) {
                                files.push(file.clone());
                            }
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
                    binding.insert(
                        local.as_str(),
                        (format!("external:{name}"), imported.clone()),
                    );
                }
            }
            ImportTarget::Unresolved { .. } => {}
        }
    }
    let imported_files: BTreeSet<&str> = resolved_imports
        .iter()
        .filter_map(|ri| match &ri.target {
            ImportTarget::Internal { file, .. } => Some(file.as_str()),
            _ => None,
        })
        .collect();

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
    let fn_binds: &[FnBind] = index
        .files
        .get(path)
        .map(|f| f.fn_binds.as_slice())
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

        if let Some(id) =
            field_type_callee_id(recv, &fact, &call, path, index, type_binds, &binding)
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

        // super.m() / super().m(): walk bases only — never the enclosing class.
        if recv == RecvKind::Super {
            if split_recv_path(&call.callee).len() == 2 {
                if let Some(class) = enclosing_class(&call, path, index) {
                    if let Some(id) = rule1_enclosing_method(index, &class, method, true) {
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
            }
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

        // this/self → sibling of the enclosing class, else unique class-like / CHA.
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
                if let Some(id) = rule1_enclosing_method(index, &class, method, false) {
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

        // Function-alias bind: `f = helper; f()` — before the local-name
        // ladder, so a same-named global is never a false edge.
        if recv == RecvKind::None {
            match fn_ptr_hit(
                &call,
                root,
                path,
                index,
                fn_binds,
                &binding,
                &imported_files,
            ) {
                FnPtrHit::Pin(id) => {
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
                FnPtrHit::Block => {
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
                FnPtrHit::Miss => {}
            }
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

        // Namespace import member (`import * as ns`, python `import m`,
        // Go `pkg.Helper` across an expanded package).
        if let Some(ns_files) = namespaces.get(root) {
            if recv != RecvKind::None {
                if let Some(id) = unique_namespace_member(index, ns_files, method) {
                    out.push(emit(
                        caller_id,
                        Some(id),
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

        // Rule 3: unique imported defining file for a bare name.
        if recv == RecvKind::None {
            if let Some(id) = rule3_include_file_id(index, path, root, &imported_files) {
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

        // Rule 2c: `Cls.m()` — the receiver token is the type. Typed locals
        // win (Rule 2); any local/param of that name vetoes the class pin.
        if recv == RecvKind::StaticType {
            if let Some(id) = static_type_callee_id(&call, &fact, path, index, type_binds, &binding)
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

        // Type-qualified / local class method.
        if matches!(recv, RecvKind::TypeQualified | RecvKind::NamedVariable) {
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
                    if let Some(id) = pin_type_method(index, path, ty, method, &binding) {
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
                    recv, &fact, &call, path, index, type_binds, &binding,
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

/// Ripwire `fnPtrBindingTarget` analog: a visible var→function bind for a
/// bare `f()` pins through the binding and never falls back to a global `f`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FnPtrHit {
    Miss,
    Pin(String),
    Block,
}

// trace:v1 id=impl.scc.resolve.fn-alias work=WORK-phase-25-of-scc-x-ripwire-lessons-absorb-extract-time-function-alias-bi satisfies=REQ-implement-phase-25-of-scc-x-ripwire-lessons-absorb-extract-time-funct implements=PLAN-phase-25-of-scc-x-ripwire-lessons-absorb-extract-time-function-alias-bi
fn fn_ptr_hit(
    call: &Call,
    root: &str,
    path: &str,
    index: &SymbolIndex,
    fn_binds: &[FnBind],
    binding: &HashMap<&str, (String, String)>,
    imported_files: &BTreeSet<&str>,
) -> FnPtrHit {
    if root.is_empty() {
        return FnPtrHit::Miss;
    }
    let scope = call.caller.as_deref().unwrap_or("");
    let local_has = has_fn_bind(fn_binds, Some(scope), root);
    let file_has = has_fn_bind(fn_binds, Some(""), root);
    if !local_has && !file_has {
        return FnPtrHit::Miss;
    }
    let local_t = unique_fn_target(fn_binds, Some(scope), root);
    let file_t = unique_fn_target(fn_binds, Some(""), root);
    let chosen = if local_has && file_has {
        match (local_t, file_t) {
            (Some(a), Some(b)) if a == b && !a.is_empty() => Some(a),
            _ => None,
        }
    } else if local_has {
        local_t.filter(|s| !s.is_empty())
    } else {
        file_t.filter(|s| !s.is_empty())
    };
    match chosen {
        Some(name) => unique_function_id(index, path, name, binding, imported_files)
            .map(FnPtrHit::Pin)
            .unwrap_or(FnPtrHit::Block),
        None => FnPtrHit::Block,
    }
}

fn unique_fn_target<'a>(binds: &'a [FnBind], scope: Option<&str>, var: &str) -> Option<&'a str> {
    let scope = scope.unwrap_or("");
    let mut targets: Vec<&str> = binds
        .iter()
        .filter(|b| b.scope == scope && b.name == var)
        .map(|b| b.target.as_str())
        .collect();
    targets.sort_unstable();
    targets.dedup();
    if targets.len() == 1 {
        Some(targets[0])
    } else {
        None
    }
}

fn has_fn_bind(binds: &[FnBind], scope: Option<&str>, var: &str) -> bool {
    let scope = scope.unwrap_or("");
    binds.iter().any(|b| b.scope == scope && b.name == var)
}

/// Unique in-repo Function named `name`, or Const with a signature
/// (TypeScript function-valued `const helper = () => {}`). Local hit
/// wins; a local non-callable symbol or import binding vetoes the rest
/// of the repo. Rule 3 then pins a unique imported defining file.
/// Two same-named callables across files stay unresolved unless Rule 3
/// narrows. Classes and signature-less consts (`LIMIT = 10`) are never pins.
fn unique_function_id(
    index: &SymbolIndex,
    local_path: &str,
    name: &str,
    binding: &HashMap<&str, (String, String)>,
    imported_files: &BTreeSet<&str>,
) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    if let Some(fs) = index.files.get(local_path) {
        if let Some((sym, id)) = fs.by_name.get(name) {
            return is_fn_alias_target(sym).then(|| id.clone());
        }
    }
    if let Some((target_file, exported)) = binding.get(name) {
        if target_file.starts_with("external:") {
            return None;
        }
        let fs = index.files.get(target_file.as_str())?;
        let (sym, id) = fs.by_name.get(exported.as_str())?;
        return is_fn_alias_target(sym).then(|| id.clone());
    }
    if let Some(id) = rule3_include_file_id(index, local_path, name, imported_files) {
        return Some(id);
    }
    let mut found: Option<String> = None;
    for fs in index.files.values() {
        let Some((sym, id)) = fs.by_name.get(name) else {
            continue;
        };
        if !is_fn_alias_target(sym) {
            continue;
        }
        if found.as_ref().is_some_and(|existing| existing != id) {
            return None;
        }
        found = Some(id.clone());
    }
    found
}

fn is_fn_alias_target(sym: &Symbol) -> bool {
    sym.kind == SymbolKind::Function || (sym.kind == SymbolKind::Const && sym.signature.is_some())
}

/// Ripwire `rule3IncludeFile` analog: pin a bare name to the unique
/// path-precise Internal import file that defines it. Same-file names
/// stay on the local ladder. Unresolved/external imports contribute
/// nothing. 0 or ≥2 imported defining files stay unresolved. Never
/// invents a target and never basename-guesses.
// trace:v1 id=impl.scc.resolve.rule3-include-file work=WORK-phase-26-of-scc-x-ripwire-lessons-absorb-rule-3-import-include-file-nar satisfies=REQ-implement-phase-26-of-scc-x-ripwire-lessons-absorb-rule-3-import-incl implements=PLAN-phase-26-of-scc-x-ripwire-lessons-absorb-rule-3-import-include-file-nar
fn rule3_include_file_id(
    index: &SymbolIndex,
    caller_path: &str,
    name: &str,
    imported_files: &BTreeSet<&str>,
) -> Option<String> {
    if name.is_empty() || imported_files.is_empty() {
        return None;
    }
    if index
        .files
        .get(caller_path)
        .is_some_and(|fs| fs.by_name.contains_key(name))
    {
        return None;
    }
    let mut chosen: Option<&str> = None;
    for file in imported_files {
        if *file == caller_path {
            continue;
        }
        let Some(fs) = index.files.get(*file) else {
            continue;
        };
        let Some((sym, _)) = fs.by_name.get(name) else {
            continue;
        };
        if !is_fn_alias_target(sym) {
            continue;
        }
        match chosen {
            None => chosen = Some(*file),
            Some(_) => return None,
        }
    }
    let file = chosen?;
    let fs = index.files.get(file)?;
    let (sym, id) = fs.by_name.get(name)?;
    is_fn_alias_target(sym).then(|| id.clone())
}

/// Unique type for `(scope, var)` or None when missing / tombstoned (≥2 types).
// trace:v1 id=impl.scc.resolve.type-narrow work=WORK-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing-from-unique satisfies=REQ-implement-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing
fn unique_bound_type<'a>(binds: &'a [TypeBind], scope: Option<&str>, var: &str) -> Option<&'a str> {
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
    pin_type_method(index, path, ty, &fact.method, binding)
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
    pin_type_method(index, path, ty, &fact.method, binding)
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
    pin_type_method(index, path, ty, &fact.method, binding)
}

fn is_class_like(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Class | SymbolKind::Interface | SymbolKind::Type
    )
}

/// Unique in-repo `{Type}.{method}` on a Class/Interface/Type named `type_name`,
/// else the unique base at the shallowest CHA level that uniquely defines it.
/// Two class-like defs that both expose the method stay unresolved (no spray).
/// Two same-named class-likes refuse heritage merge. Modules do not count.
// trace:v1 id=impl.scc.resolve.class-name work=WORK-phase-21-of-scc-x-ripwire-lessons-absorb-rule-2c-class-name-receiver-p satisfies=REQ-implement-phase-21-of-scc-x-ripwire-lessons-absorb-rule-2c-class-name implements=PLAN-phase-21-of-scc-x-ripwire-lessons-absorb-rule-2c-class-name-receiver-p
fn unique_class_like_method(index: &SymbolIndex, type_name: &str, method: &str) -> Option<String> {
    if type_name.is_empty() || method.is_empty() {
        return None;
    }
    unique_defining_class_like_method(index, type_name, method)
        .or_else(|| method_on_bases(index, type_name, method))
}

/// Own-type pin only: unique `{Type}.{method}` on a class-like named `type_name`.
fn unique_defining_class_like_method(
    index: &SymbolIndex,
    type_name: &str,
    method: &str,
) -> Option<String> {
    let key = format!("{type_name}.{method}");
    let mut found: Option<String> = None;
    for fs in index.files.values() {
        let Some((sym, _)) = fs.by_name.get(type_name) else {
            continue;
        };
        if !is_class_like(sym.kind) {
            continue;
        }
        let Some(id) = fs
            .methods
            .get(&key)
            .map(|(_, id)| id)
            .or_else(|| fs.by_name.get(&key).map(|(_, id)| id))
        else {
            continue;
        };
        if found.as_ref().is_some_and(|existing| existing != id) {
            return None;
        }
        found = Some(id.clone());
    }
    found
}

/// Direct bases of the unique class-like named `type_name`. Two same-named
/// class-likes → `None` (do not merge heritage). Missing map → empty vec.
fn class_bases_of_unique(index: &SymbolIndex, type_name: &str) -> Option<Vec<String>> {
    let mut found: Option<Vec<String>> = None;
    for fs in index.files.values() {
        let Some((sym, _)) = fs.by_name.get(type_name) else {
            continue;
        };
        if !is_class_like(sym.kind) {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(fs.class_bases.get(type_name).cloned().unwrap_or_default());
    }
    found
}

/// Ripwire Rule 1 (`rule1BaseWalk`): pin `self.m()` / `this.m()` via own
/// type then CHA; `super.m()` / `super().m()` walk bases only (`skip_self`).
/// Two hitting bases at one level stay unresolved. No C++ bare-name Rule 1.
/// Field chains never reach here.
// trace:v1 id=impl.scc.resolve.rule1 work=WORK-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-super-base-wa satisfies=REQ-implement-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-s implements=PLAN-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-super-base-wa
fn rule1_enclosing_method(
    index: &SymbolIndex,
    class: &str,
    method: &str,
    skip_self: bool,
) -> Option<String> {
    if skip_self {
        method_on_bases(index, class, method)
    } else {
        unique_class_like_method(index, class, method)
    }
}

/// Ripwire `methodOnTypeOrBases` BFS: shallowest level with exactly one
/// hitting base wins; two at one level refuse. Cap 16 visited names.
// trace:v1 id=impl.scc.resolve.cha-bases work=WORK-phase-22-of-scc-x-ripwire-lessons-absorb-rule-2c-cha-method-on-type-or-ba satisfies=REQ-implement-phase-22-of-scc-x-ripwire-lessons-absorb-rule-2c-cha-meth implements=PLAN-phase-22-of-scc-x-ripwire-lessons-absorb-rule-2c-cha-method-on-type-or-ba
fn method_on_bases(index: &SymbolIndex, type_name: &str, method: &str) -> Option<String> {
    const CHA_WALK_CAP: usize = 16;
    let mut visited: Vec<String> = Vec::with_capacity(CHA_WALK_CAP);
    visited.push(type_name.to_string());
    let mut begin = 0;
    while begin < visited.len() {
        let end = visited.len();
        for i in begin..end {
            let Some(bases) = class_bases_of_unique(index, &visited[i]) else {
                continue;
            };
            for base in bases {
                if visited.len() >= CHA_WALK_CAP {
                    break;
                }
                if !visited.iter().any(|v| v == &base) {
                    visited.push(base);
                }
            }
        }
        let mut found: Option<String> = None;
        for name in &visited[end..] {
            let Some(id) = unique_defining_class_like_method(index, name, method) else {
                continue;
            };
            if found.as_ref().is_some_and(|existing| existing != &id) {
                return None;
            }
            found = Some(id);
        }
        if found.is_some() {
            return found;
        }
        begin = end;
    }
    None
}

/// Local/import `{Type}.{method}`, else unique class-like / CHA.
fn pin_type_method(
    index: &SymbolIndex,
    local_path: &str,
    ty: &str,
    method: &str,
    binding: &HashMap<&str, (String, String)>,
) -> Option<String> {
    method_id_for_type(index, local_path, ty, method, binding)
        .or_else(|| unique_class_like_method(index, ty, method))
}

/// Pin `Cls.m()` (StaticType). A unique typed bind for `Cls` is Rule 2 and
/// wins; any bind including an untyped shadow vetoes the class-name pin.
fn static_type_callee_id(
    call: &Call,
    fact: &RecvFact,
    path: &str,
    index: &SymbolIndex,
    type_binds: &[TypeBind],
    binding: &HashMap<&str, (String, String)>,
) -> Option<String> {
    let root = fact.root.as_str();
    let method = fact.method.as_str();
    if has_bind(type_binds, call.caller.as_deref(), root) {
        let ty = unique_bound_type(type_binds, call.caller.as_deref(), root)?;
        if ty.is_empty() {
            return None;
        }
        return pin_type_method(index, path, ty, method, binding);
    }
    unique_class_like_method(index, root, method)
}

/// Real `{Type}.{method}` definition locally or on the imported type only.
fn method_id_for_type(
    index: &SymbolIndex,
    local_path: &str,
    ty: &str,
    method: &str,
    binding: &HashMap<&str, (String, String)>,
) -> Option<String> {
    if ty.is_empty() || method.is_empty() {
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
        assert_eq!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "a.py", "normalize"))
        );
        assert_eq!(
            resolved[0].provenance,
            scc_core::Provenance::Extracted,
            "native resolution is evidence-grade (candidate), never RESOLVED (section 26)"
        );
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
            target: ImportTarget::Internal {
                file: "b.py".into(),
                name_map: HashMap::new(),
                namespace: false,
            },
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
        let resolved = resolve_calls(
            "a.py",
            &calls,
            &[mk_symbol("main", SymbolKind::Function)],
            &resolved_imports,
            &idx,
            "repo",
        );
        assert_eq!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "b.py", "resolve"))
        );
        // unused import var
        let _ = &import;
    }

    #[test]
    fn resolves_namespace_member() {
        let mut idx = SymbolIndex::new("repo");
        idx.add_file(
            "svc/asr.py",
            &[mk_symbol("transcribe", SymbolKind::Function)],
        );
        idx.add_file("main.py", &[mk_symbol("run", SymbolKind::Function)]);
        let ri = ResolvedImport {
            local_file: "main.py".into(),
            module: "svc.asr".into(),
            target: ImportTarget::Internal {
                file: "svc/asr.py".into(),
                name_map: HashMap::new(),
                namespace: true,
            },
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
        let resolved = resolve_calls(
            "main.py",
            &calls,
            &[mk_symbol("run", SymbolKind::Function)],
            &[ri],
            &idx,
            "repo",
        );
        assert_eq!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "svc/asr.py", "transcribe"))
        );
    }

    #[test]
    fn resolves_self_method() {
        let mut idx = SymbolIndex::new("repo");
        let mut m = mk_symbol("Worker.handle", SymbolKind::Method);
        m.parent = Some("Worker".into());
        let mut m2 = mk_symbol("Worker.helper", SymbolKind::Method);
        m2.parent = Some("Worker".into());
        let syms = vec![mk_symbol("Worker", SymbolKind::Class), m, m2];
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
        assert_eq!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "w.py", "Worker.helper"))
        );
    }

    #[test]
    fn external_import_becomes_external_api() {
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("a.ts", &[mk_symbol("main", SymbolKind::Function)]);
        let ri = ResolvedImport {
            local_file: "a.ts".into(),
            module: "express".into(),
            target: ImportTarget::External {
                name: "express".into(),
            },
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
        let resolved = resolve_calls(
            "a.ts",
            &calls,
            &[mk_symbol("main", SymbolKind::Function)],
            &[ri],
            &idx,
            "repo",
        );
        assert_eq!(
            resolved[0].callee_id,
            Some(scc_core::entity_id(
                "repo",
                scc_core::kinds::EXTERNAL_API,
                "express"
            ))
        );
    }

    #[test]
    fn module_path_resolution() {
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("pkg/sub.py", &[]);
        idx.add_file("pkg/__init__.py", &[]);
        idx.add_file("src/util.py", &[]);
        idx.add_file("web/index.ts", &[]);
        assert_eq!(
            idx.resolve_module_path("pkg.sub"),
            Some("pkg/sub.py".into())
        );
        assert_eq!(
            idx.resolve_module_path("pkg"),
            Some("pkg/__init__.py".into())
        );
        assert_eq!(
            idx.resolve_module_path("./web"),
            Some("web/index.ts".into())
        );
        assert_eq!(idx.resolve_module_path("util"), Some("src/util.py".into()));
        assert_eq!(idx.resolve_module_path("nonexistent"), None);
    }

    fn rust_imp(module: &str) -> Import {
        Import {
            module: module.into(),
            names: vec![],
            line: 1,
            r#type: ImportType::Member,
        }
    }

    #[test]
    // trace:v1 id=test.scc.resolve.rust-import verifies=REQ-implement-phase-27-of-scc-x-ripwire-lessons-absorb-rust-step-a-path-p exercises=impl.scc.resolve.rust-import
    fn rust_step_a_unique_or_degrade_and_never_basename() {
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("src/lib.rs", &[]);
        idx.add_file("src/geo/mod.rs", &[]);
        idx.add_file("src/other/mod.rs", &[]);
        idx.add_file("src/util.rs", &[]);
        idx.add_file("src/amb.rs", &[]);
        idx.add_file("src/amb/mod.rs", &[]);
        idx.add_file("src/consumer.rs", &[]);

        match idx.resolve_import("src/lib.rs", &rust_imp("mod:geo")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "src/geo/mod.rs"),
            other => panic!("mod:geo expected geo/mod.rs, got {other:?}"),
        }
        match idx.resolve_import("src/lib.rs", &rust_imp("mod:geo")) {
            ImportTarget::Internal { file, .. } => {
                assert_ne!(file, "src/other/mod.rs", "must not basename-guess other/mod.rs")
            }
            other => panic!("mod:geo expected Internal, got {other:?}"),
        }
        match idx.resolve_import("src/lib.rs", &rust_imp("mod:util")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "src/util.rs"),
            other => panic!("mod:util expected util.rs, got {other:?}"),
        }
        match idx.resolve_import("src/consumer.rs", &rust_imp("crate::geo::helper")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "src/geo/mod.rs"),
            other => panic!("crate::geo::helper expected geo/mod.rs, got {other:?}"),
        }
        match idx.resolve_import("src/consumer.rs", &rust_imp("crate::util::utilfn")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "src/util.rs"),
            other => panic!("crate::util::utilfn expected util.rs, got {other:?}"),
        }
        match idx.resolve_import("src/consumer.rs", &rust_imp("crate::amb::dupfn")) {
            ImportTarget::Unresolved { name } => assert_eq!(name, "crate::amb::dupfn"),
            other => panic!("amb.rs + amb/mod.rs must degrade, got {other:?}"),
        }
        match idx.resolve_import("src/consumer.rs", &rust_imp("std::collections::HashMap")) {
            ImportTarget::External { name } => assert_eq!(name, "std::collections::HashMap"),
            other => panic!("std:: must stay External, got {other:?}"),
        }
        match idx.resolve_import("src/consumer.rs", &rust_imp("crate::{geo, util}")) {
            ImportTarget::Unresolved { .. } => {}
            other => panic!("brace group must degrade, got {other:?}"),
        }

        let mut no_root = SymbolIndex::new("repo");
        no_root.add_file("src/consumer.rs", &[]);
        no_root.add_file("src/geo/mod.rs", &[]);
        match no_root.resolve_import("src/consumer.rs", &rust_imp("crate::geo::helper")) {
            ImportTarget::Unresolved { name } => assert_eq!(name, "crate::geo::helper"),
            other => panic!("no crate root must degrade, got {other:?}"),
        }

        let mut py = SymbolIndex::new("repo");
        py.add_file("w.py", &[]);
        py.add_file("crate.rs", &[]);
        py.add_file("src/lib.rs", &[]);
        match py.resolve_import("w.py", &rust_imp("crate::geo::helper")) {
            ImportTarget::External { name } => assert_eq!(name, "crate::geo::helper"),
            other => panic!("Python caller must not use Rust Step-A, got {other:?}"),
        }
    }

    #[test]
    // trace:v1 id=test.scc.resolve.rust-import-super verifies=REQ-implement-phase-27-of-scc-x-ripwire-lessons-absorb-rust-step-a-path-p exercises=impl.scc.resolve.rust-import
    fn rust_super_and_self_resolve_relative_to_includer() {
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("src/lib.rs", &[]);
        idx.add_file("src/foo/bar.rs", &[]);
        idx.add_file("src/foo/cfg.rs", &[]);
        idx.add_file("src/cfg.rs", &[]);
        match idx.resolve_import("src/foo/bar.rs", &rust_imp("self::cfg")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "src/foo/cfg.rs"),
            other => panic!("self::cfg expected src/foo/cfg.rs, got {other:?}"),
        }
        match idx.resolve_import("src/foo/bar.rs", &rust_imp("super::cfg")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "src/cfg.rs"),
            other => panic!("super::cfg expected src/cfg.rs, got {other:?}"),
        }
    }

    #[test]
    // trace:v1 id=test.scc.resolve.python-import verifies=REQ-implement-phase-28-of-scc-x-ripwire-lessons-absorb-python-and-type-scr exercises=impl.scc.resolve.python-import
    fn python_step_a_unique_or_degrade_and_never_basename() {
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("a.py", &[]);
        idx.add_file("other/a.py", &[]);
        idx.add_file("other/mod.py", &[]);
        idx.add_file("pkg/mod.py", &[]);
        idx.add_file("pkg/__init__.py", &[]);
        idx.add_file("rel/sibling.py", &[]);
        idx.add_file("rel/relcaller.py", &[]);
        idx.add_file("other/caller2.py", &[]);
        idx.add_file("caller.py", &[]);
        idx.add_file("web/index.ts", &[]);
        idx.add_file("src/util.py", &[]);

        match idx.resolve_import("caller.py", &rust_imp("a")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "a.py"),
            other => panic!("import a expected a.py, got {other:?}"),
        }
        match idx.resolve_import("caller.py", &rust_imp("a")) {
            ImportTarget::Internal { file, .. } => {
                assert_ne!(file, "other/a.py", "must not basename-guess other/a.py")
            }
            other => panic!("import a expected Internal, got {other:?}"),
        }
        match idx.resolve_import("caller.py", &rust_imp("pkg.mod")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "pkg/mod.py"),
            other => panic!("pkg.mod expected pkg/mod.py, got {other:?}"),
        }
        match idx.resolve_import("caller.py", &rust_imp("pkg.mod")) {
            ImportTarget::Internal { file, .. } => {
                assert_ne!(file, "other/mod.py", "must not basename-guess other/mod.py")
            }
            other => panic!("pkg.mod expected Internal, got {other:?}"),
        }
        match idx.resolve_import("caller.py", &rust_imp("pkg")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "pkg/__init__.py"),
            other => panic!("import pkg expected pkg/__init__.py, got {other:?}"),
        }
        match idx.resolve_import("other/caller2.py", &rust_imp("other.a")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "other/a.py"),
            other => panic!("other.a expected other/a.py, got {other:?}"),
        }
        match idx.resolve_import("rel/relcaller.py", &rust_imp(".sibling")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "rel/sibling.py"),
            other => panic!("from .sibling expected rel/sibling.py, got {other:?}"),
        }
        match idx.resolve_import("caller.py", &rust_imp("os")) {
            ImportTarget::External { name } => assert_eq!(name, "os"),
            other => panic!("stdlib os must stay External, got {other:?}"),
        }
        match idx.resolve_import("w.py", &rust_imp("util")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "src/util.py"),
            other => panic!("unique src/util.py must still pin, got {other:?}"),
        }
        match idx.resolve_import("caller.py", &rust_imp("web")) {
            ImportTarget::External { name } => assert_eq!(name, "web"),
            other => panic!("Python must not steal web/index.ts, got {other:?}"),
        }

        let mut amb = SymbolIndex::new("repo");
        amb.add_file("pkg.py", &[]);
        amb.add_file("pkg/__init__.py", &[]);
        amb.add_file("w.py", &[]);
        match amb.resolve_import("w.py", &rust_imp("pkg")) {
            ImportTarget::Unresolved { name } => assert_eq!(name, "pkg"),
            other => panic!("pkg.py + pkg/__init__.py must degrade, got {other:?}"),
        }

        let mut roots = SymbolIndex::new("repo");
        roots.add_file("util.py", &[]);
        roots.add_file("src/util.py", &[]);
        roots.add_file("w.py", &[]);
        match roots.resolve_import("w.py", &rust_imp("util")) {
            ImportTarget::Unresolved { name } => assert_eq!(name, "util"),
            other => panic!("util.py + src/util.py must degrade, got {other:?}"),
        }

        let mut nested = SymbolIndex::new("repo");
        nested.add_file("pkg/sub/w.py", &[]);
        nested.add_file("pkg/models.py", &[]);
        nested.add_file("pkg/sub/models.py", &[]);
        match nested.resolve_import("pkg/sub/w.py", &rust_imp("..models")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "pkg/models.py"),
            other => panic!("from ..models expected pkg/models.py, got {other:?}"),
        }
        match nested.resolve_import("pkg/sub/w.py", &rust_imp("..models")) {
            ImportTarget::Internal { file, .. } => {
                assert_ne!(file, "pkg/sub/models.py", "must not stay in the child dir")
            }
            other => panic!("from ..models expected Internal, got {other:?}"),
        }
        match nested.resolve_import("pkg/sub/w.py", &rust_imp(".")) {
            ImportTarget::Internal { file, .. } => {
                panic!("from . without __init__.py must not guess, got {file}")
            }
            ImportTarget::Unresolved { name } => assert_eq!(name, "."),
            other => panic!("from . miss must be Unresolved, got {other:?}"),
        }
        nested.add_file("pkg/sub/__init__.py", &[]);
        match nested.resolve_import("pkg/sub/w.py", &rust_imp(".")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "pkg/sub/__init__.py"),
            other => panic!("from . expected pkg/sub/__init__.py, got {other:?}"),
        }
    }

    #[test]
    // trace:v1 id=test.scc.resolve.ts-import verifies=REQ-implement-phase-28-of-scc-x-ripwire-lessons-absorb-python-and-type-scr exercises=impl.scc.resolve.ts-import
    fn ts_step_a_unique_or_degrade_and_never_basename() {
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("x.ts", &[]);
        idx.add_file("other/x.ts", &[]);
        idx.add_file("a/b.ts", &[]);
        idx.add_file("other/b.ts", &[]);
        idx.add_file("idx/index.ts", &[]);
        idx.add_file("caller.ts", &[]);
        idx.add_file("other/caller2.ts", &[]);
        idx.add_file("react.ts", &[]);
        idx.add_file("util.py", &[]);

        match idx.resolve_import("caller.ts", &rust_imp("./x")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "x.ts"),
            other => panic!("./x expected x.ts, got {other:?}"),
        }
        match idx.resolve_import("caller.ts", &rust_imp("./x")) {
            ImportTarget::Internal { file, .. } => {
                assert_ne!(file, "other/x.ts", "must not basename-guess other/x.ts")
            }
            other => panic!("./x expected Internal, got {other:?}"),
        }
        match idx.resolve_import("caller.ts", &rust_imp("./a/b")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "a/b.ts"),
            other => panic!("./a/b expected a/b.ts, got {other:?}"),
        }
        match idx.resolve_import("caller.ts", &rust_imp("./a/b")) {
            ImportTarget::Internal { file, .. } => {
                assert_ne!(file, "other/b.ts", "must not basename-guess other/b.ts")
            }
            other => panic!("./a/b expected Internal, got {other:?}"),
        }
        match idx.resolve_import("caller.ts", &rust_imp("./idx")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "idx/index.ts"),
            other => panic!("./idx expected idx/index.ts, got {other:?}"),
        }
        match idx.resolve_import("other/caller2.ts", &rust_imp("./x")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "other/x.ts"),
            other => panic!("other/ ./x expected other/x.ts, got {other:?}"),
        }
        match idx.resolve_import("caller.ts", &rust_imp("react")) {
            ImportTarget::External { name } => assert_eq!(name, "react"),
            other => panic!("bare react must stay External even if react.ts exists, got {other:?}"),
        }
        match idx.resolve_import("caller.ts", &rust_imp("./util")) {
            ImportTarget::Unresolved { name } => assert_eq!(name, "./util"),
            other => panic!("TS must not steal util.py, got {other:?}"),
        }
        match idx.resolve_import("caller.ts", &rust_imp("./missing")) {
            ImportTarget::Unresolved { name } => assert_eq!(name, "./missing"),
            other => panic!("relative miss must be Unresolved, got {other:?}"),
        }

        let mut amb = SymbolIndex::new("repo");
        amb.add_file("x.ts", &[]);
        amb.add_file("x/index.ts", &[]);
        amb.add_file("caller.ts", &[]);
        match amb.resolve_import("caller.ts", &rust_imp("./x")) {
            ImportTarget::Unresolved { name } => assert_eq!(name, "./x"),
            other => panic!("x.ts + x/index.ts must degrade, got {other:?}"),
        }

        let mut py = SymbolIndex::new("repo");
        py.add_file("w.py", &[]);
        py.add_file("x.ts", &[]);
        match py.resolve_import("w.py", &rust_imp("./x")) {
            ImportTarget::Unresolved { .. } => {}
            other => panic!("Python path-style ./x must not steal x.ts, got {other:?}"),
        }
    }

    #[test]
    // trace:v1 id=test.scc.resolve.go-import verifies=REQ-implement-phase-29-of-scc-x-ripwire-lessons-absorb-remaining-language exercises=impl.scc.resolve.go-import,impl.scc.resolve.import-expanded
    fn go_step_a_unique_or_degrade_and_never_steal() {
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("store.go", &[]);
        idx.add_file("other/store.go", &[]);
        idx.add_file("fmt.py", &[]);
        idx.add_file("web/index.ts", &[]);
        idx.add_file("main.go", &[]);
        match idx.resolve_import("main.go", &rust_imp("store")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "store.go"),
            other => panic!("import store expected store.go, got {other:?}"),
        }
        match idx.resolve_import("main.go", &rust_imp("store")) {
            ImportTarget::Internal { file, .. } => {
                assert_ne!(file, "other/store.go", "must not basename-guess other/store.go")
            }
            other => panic!("import store expected Internal, got {other:?}"),
        }
        match idx.resolve_import("main.go", &rust_imp("fmt")) {
            ImportTarget::External { name } => assert_eq!(name, "fmt"),
            other => panic!("fmt must stay External even if fmt.py exists, got {other:?}"),
        }
        match idx.resolve_import("main.go", &rust_imp("web")) {
            ImportTarget::External { name } => assert_eq!(name, "web"),
            other => panic!("Go must not steal web/index.ts, got {other:?}"),
        }

        let mut pkg = SymbolIndex::new("repo");
        pkg.add_file("pkg/a.go", &[mk_symbol("helper", SymbolKind::Function)]);
        pkg.add_file("pkg/b.go", &[mk_symbol("other", SymbolKind::Function)]);
        pkg.add_file("pkg/a_test.go", &[mk_symbol("helper", SymbolKind::Function)]);
        pkg.add_file("main.go", &[mk_symbol("run", SymbolKind::Function)]);
        match pkg.resolve_import("main.go", &rust_imp("pkg")) {
            ImportTarget::Unresolved { name } => assert_eq!(name, "pkg"),
            other => panic!("multi-file package must not pick one file, got {other:?}"),
        }
        let expanded = pkg.resolve_import_expanded("main.go", &rust_imp("pkg"));
        let files: Vec<String> = expanded
            .iter()
            .filter_map(|t| match t {
                ImportTarget::Internal { file, .. } => Some(file.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(files, vec!["pkg/a.go".to_string(), "pkg/b.go".to_string()]);
        assert!(
            !files.iter().any(|f| f.ends_with("_test.go")),
            "production package must not include _test.go: {files:?}"
        );

        let mut amb = SymbolIndex::new("repo");
        amb.add_file("pkg.go", &[]);
        amb.add_file("pkg/a.go", &[]);
        amb.add_file("main.go", &[]);
        match amb.resolve_import("main.go", &rust_imp("pkg")) {
            ImportTarget::Unresolved { name } => assert_eq!(name, "pkg"),
            other => panic!("pkg.go + pkg/ must degrade, got {other:?}"),
        }

        let mut src = SymbolIndex::new("repo");
        src.add_file("src/util.go", &[]);
        src.add_file("main.go", &[]);
        match src.resolve_import("main.go", &rust_imp("util")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "src/util.go"),
            other => panic!("unique src/util.go must pin, got {other:?}"),
        }

        let mut two = SymbolIndex::new("repo");
        two.add_file("util.go", &[]);
        two.add_file("src/util.go", &[]);
        two.add_file("main.go", &[]);
        match two.resolve_import("main.go", &rust_imp("util")) {
            ImportTarget::Unresolved { name } => assert_eq!(name, "util"),
            other => panic!("util.go + src/util.go must degrade, got {other:?}"),
        }

        let mut only = SymbolIndex::new("repo");
        only.add_file("other/store.go", &[]);
        only.add_file("y.go", &[]);
        only.add_file("main.go", &[]);
        match only.resolve_import("main.go", &rust_imp("store")) {
            ImportTarget::External { name } => assert_eq!(name, "store"),
            other => panic!("must not basename-guess other/store.go, got {other:?}"),
        }
        match only.resolve_import("main.go", &rust_imp("github.com/x/y")) {
            ImportTarget::External { name } => assert_eq!(name, "github.com/x/y"),
            other => panic!("must not suffix-guess y.go from module path, got {other:?}"),
        }

        let mut rel = SymbolIndex::new("repo");
        rel.add_file("cmd/main.go", &[]);
        rel.add_file("cmd/store.go", &[]);
        rel.add_file("store.go", &[]);
        match rel.resolve_import("cmd/main.go", &rust_imp("./store")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "cmd/store.go"),
            other => panic!("relative ./store must pin cmd/store.go, got {other:?}"),
        }

        let pkg_imp = Import {
            module: "pkg".into(),
            names: vec![("pkg".into(), "pkg".into())],
            line: 1,
            r#type: ImportType::Module,
        };
        let imps = pkg.resolved_imports("main.go", &[pkg_imp]);
        let symbols = [mk_symbol("run", SymbolKind::Function)];
        let bare = resolve_calls(
            "main.go",
            &[Call {
                caller: Some("run".into()),
                callee: "helper".into(),
                line: 3,
                known_receiver: false,
                ..Default::default()
            }
            .finish()],
            &symbols,
            &imps,
            &pkg,
            "repo",
        );
        assert_eq!(
            bare[0].callee_id,
            Some(scc_core::symbol_id("repo", "pkg/a.go", "helper")),
            "Rule 3 must pin unique helper in the expanded package"
        );
        assert_ne!(
            bare[0].callee_id,
            Some(scc_core::symbol_id("repo", "pkg/a_test.go", "helper")),
            "_test.go must not participate in the production package"
        );
        let qual = resolve_calls(
            "main.go",
            &[Call {
                caller: Some("run".into()),
                callee: "pkg.helper".into(),
                line: 4,
                known_receiver: true,
                ..Default::default()
            }
            .finish()],
            &symbols,
            &imps,
            &pkg,
            "repo",
        );
        assert_eq!(
            qual[0].callee_id,
            Some(scc_core::symbol_id("repo", "pkg/a.go", "helper")),
            "pkg.helper must unique-or-degrade across expanded package files"
        );
    }

    #[test]
    // trace:v1 id=test.scc.resolve.java-import verifies=REQ-implement-phase-29-of-scc-x-ripwire-lessons-absorb-remaining-language exercises=impl.scc.resolve.java-import
    fn java_step_a_unique_or_degrade_and_never_steal() {
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("com/foo/Bar.java", &[]);
        idx.add_file("other/Bar.java", &[]);
        idx.add_file("Bar.py", &[]);
        idx.add_file("Main.java", &[]);
        match idx.resolve_import("Main.java", &rust_imp("com.foo.Bar")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "com/foo/Bar.java"),
            other => panic!("com.foo.Bar expected com/foo/Bar.java, got {other:?}"),
        }
        match idx.resolve_import("Main.java", &rust_imp("com.foo.Bar")) {
            ImportTarget::Internal { file, .. } => {
                assert_ne!(file, "other/Bar.java", "must not basename-guess other/Bar.java")
            }
            other => panic!("com.foo.Bar expected Internal, got {other:?}"),
        }
        match idx.resolve_import("Main.java", &rust_imp("java.util.List")) {
            ImportTarget::External { name } => assert_eq!(name, "java.util.List"),
            other => panic!("java.util.List must stay External, got {other:?}"),
        }
        match idx.resolve_import("Main.java", &rust_imp("com.foo.*")) {
            ImportTarget::Unresolved { name } => assert_eq!(name, "com.foo.*"),
            other => panic!("star import must not guess a file, got {other:?}"),
        }
        match idx.resolve_import("Main.java", &rust_imp("Bar")) {
            ImportTarget::External { name } => assert_eq!(name, "Bar"),
            other => panic!("Java must not steal Bar.py, got {other:?}"),
        }

        let mut maven = SymbolIndex::new("repo");
        maven.add_file(
            "src/main/java/com/example/Service.java",
            &[mk_symbol("Service", SymbolKind::Class)],
        );
        maven.add_file("App.java", &[]);
        match maven.resolve_import("App.java", &rust_imp("com.example.Service")) {
            ImportTarget::Internal { file, .. } => {
                assert_eq!(file, "src/main/java/com/example/Service.java")
            }
            other => panic!("unique src/main/java Service must pin, got {other:?}"),
        }
        match maven.resolve_import("App.java", &rust_imp("com.example.Service.save")) {
            ImportTarget::Internal { file, .. } => {
                assert_eq!(file, "src/main/java/com/example/Service.java")
            }
            other => panic!("static member import must pin parent type file, got {other:?}"),
        }

        let mut amb = SymbolIndex::new("repo");
        amb.add_file("com/foo/Bar.java", &[]);
        amb.add_file("src/main/java/com/foo/Bar.java", &[]);
        amb.add_file("Main.java", &[]);
        match amb.resolve_import("Main.java", &rust_imp("com.foo.Bar")) {
            ImportTarget::Unresolved { name } => assert_eq!(name, "com.foo.Bar"),
            other => panic!("two Bar.java files must degrade, got {other:?}"),
        }
    }

    #[test]
    // trace:v1 id=test.scc.resolve.c-include verifies=REQ-implement-phase-30-of-scc-x-ripwire-lessons-absorb-c-and-c-extracto exercises=impl.scc.resolve.c-include
    fn c_quote_include_is_exact_join_angle_is_external_never_steals() {
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("src/foo.h", &[mk_symbol("helper", SymbolKind::Function)]);
        idx.add_file("other/foo.h", &[mk_symbol("helper", SymbolKind::Function)]);
        idx.add_file("stdio.h", &[mk_symbol("helper", SymbolKind::Function)]);
        idx.add_file("fmt.py", &[mk_symbol("helper", SymbolKind::Function)]);
        idx.add_file("src/main.c", &[mk_symbol("main", SymbolKind::Function)]);

        match idx.resolve_import("src/main.c", &rust_imp("foo.h")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "src/foo.h"),
            other => panic!("quote foo.h expected src/foo.h, got {other:?}"),
        }
        match idx.resolve_import("src/main.c", &rust_imp("foo.h")) {
            ImportTarget::Internal { file, .. } => {
                assert_ne!(file, "other/foo.h", "must not basename-guess other/foo.h")
            }
            other => panic!("quote foo.h expected Internal, got {other:?}"),
        }
        match idx.resolve_import("src/main.c", &rust_imp("missing.h")) {
            ImportTarget::Unresolved { name } => assert_eq!(name, "missing.h"),
            other => panic!("missing quote include must be Unresolved, got {other:?}"),
        }
        match idx.resolve_import("src/main.c", &rust_imp("<stdio.h>")) {
            ImportTarget::External { name } => assert_eq!(name, "<stdio.h>"),
            other => panic!("angle include must stay External even if stdio.h exists, got {other:?}"),
        }
        match idx.resolve_import("src/main.c", &rust_imp("fmt")) {
            ImportTarget::Unresolved { name } => assert_eq!(name, "fmt"),
            other => panic!("C must not steal fmt.py, got {other:?}"),
        }

        let mut cpp = SymbolIndex::new("repo");
        cpp.add_file("src/foo.hpp", &[mk_symbol("helper", SymbolKind::Function)]);
        cpp.add_file("src/main.cpp", &[mk_symbol("main", SymbolKind::Function)]);
        match cpp.resolve_import("src/main.cpp", &rust_imp("foo.hpp")) {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "src/foo.hpp"),
            other => panic!("C++ quote include expected src/foo.hpp, got {other:?}"),
        }
        cpp.add_file("main.py", &[]);
        match cpp.resolve_import("main.py", &rust_imp("foo.hpp")) {
            ImportTarget::External { name } => assert_eq!(name, "foo.hpp"),
            other => panic!("Python must not take the C-family join path, got {other:?}"),
        }
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
    // trace:v1 id=test.scc.resolve.class-name-pin verifies=REQ-implement-phase-21-of-scc-x-ripwire-lessons-absorb-rule-2c-class-name exercises=impl.scc.resolve.class-name
    fn unique_class_name_receiver_pins_that_method() {
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
        let resolved = resolve_calls(
            "w.py",
            &[Call {
                caller: Some("handle".into()),
                callee: "Order.process".into(),
                line: 9,
                known_receiver: false,
                ..Default::default()
            }
            .finish()],
            &syms,
            &[],
            &idx,
            "repo",
        );
        assert_eq!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "w.py", "Order.process"))
        );
        assert_ne!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "w.py", "Invoice.process"))
        );
        assert_eq!(resolved[0].class, ResolutionClass::ResolvedInternal);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.class-name-typed-shadow verifies=REQ-implement-phase-21-of-scc-x-ripwire-lessons-absorb-rule-2c-class-name exercises=impl.scc.resolve.class-name
    fn typed_param_named_like_class_pins_bound_type() {
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
                name: "Order".into(),
                type_name: "Invoice".into(),
                line: 8,
            }],
        );
        let resolved = resolve_calls(
            "w.py",
            &[Call {
                caller: Some("handle".into()),
                callee: "Order.process".into(),
                line: 9,
                known_receiver: false,
                ..Default::default()
            }
            .finish()],
            &syms,
            &[],
            &idx,
            "repo",
        );
        assert_eq!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "w.py", "Invoice.process"))
        );
        assert_ne!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "w.py", "Order.process"))
        );
    }

    #[test]
    // trace:v1 id=test.scc.resolve.class-name-untyped-veto verifies=REQ-implement-phase-21-of-scc-x-ripwire-lessons-absorb-rule-2c-class-name exercises=impl.scc.resolve.class-name
    fn untyped_param_named_like_class_vetoes_class_pin() {
        let mut idx = SymbolIndex::new("repo");
        let mut order_p = mk_symbol("Order.process", SymbolKind::Method);
        order_p.parent = Some("Order".into());
        let handle = mk_symbol("handle", SymbolKind::Function);
        let syms = vec![mk_symbol("Order", SymbolKind::Class), order_p, handle];
        idx.add_file("w.py", &syms);
        idx.set_type_binds(
            "w.py",
            &[TypeBind {
                scope: "handle".into(),
                name: "Order".into(),
                type_name: String::new(),
                line: 8,
            }],
        );
        let resolved = resolve_calls(
            "w.py",
            &[Call {
                caller: Some("handle".into()),
                callee: "Order.process".into(),
                line: 9,
                known_receiver: false,
                ..Default::default()
            }
            .finish()],
            &syms,
            &[],
            &idx,
            "repo",
        );
        assert_eq!(resolved[0].callee_id, None);
        assert_eq!(resolved[0].class, ResolutionClass::UnresolvedLikelyInternal);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.class-name-split verifies=REQ-implement-phase-21-of-scc-x-ripwire-lessons-absorb-rule-2c-class-name exercises=impl.scc.resolve.class-name
    fn two_defining_classes_do_not_pin_class_name_receiver() {
        let mut idx = SymbolIndex::new("repo");
        let mut a_p = mk_symbol("Order.process", SymbolKind::Method);
        a_p.parent = Some("Order".into());
        idx.add_file("a.py", &[mk_symbol("Order", SymbolKind::Class), a_p]);
        let mut b_p = mk_symbol("Order.process", SymbolKind::Method);
        b_p.parent = Some("Order".into());
        idx.add_file("b.py", &[mk_symbol("Order", SymbolKind::Class), b_p]);
        let handle = mk_symbol("handle", SymbolKind::Function);
        idx.add_file("w.py", std::slice::from_ref(&handle));
        let resolved = resolve_calls(
            "w.py",
            &[Call {
                caller: Some("handle".into()),
                callee: "Order.process".into(),
                line: 2,
                known_receiver: false,
                ..Default::default()
            }
            .finish()],
            &[handle],
            &[],
            &idx,
            "repo",
        );
        assert_eq!(resolved[0].callee_id, None);
        assert_eq!(resolved[0].class, ResolutionClass::UnresolvedLikelyInternal);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.class-name-cross-file verifies=REQ-implement-phase-21-of-scc-x-ripwire-lessons-absorb-rule-2c-class-name exercises=impl.scc.resolve.class-name
    fn unique_class_in_other_file_pins_class_name_receiver() {
        let mut idx = SymbolIndex::new("repo");
        let mut order_p = mk_symbol("Order.process", SymbolKind::Method);
        order_p.parent = Some("Order".into());
        idx.add_file(
            "order.py",
            &[mk_symbol("Order", SymbolKind::Class), order_p],
        );
        let handle = mk_symbol("handle", SymbolKind::Function);
        idx.add_file("w.py", std::slice::from_ref(&handle));
        let resolved = resolve_calls(
            "w.py",
            &[Call {
                caller: Some("handle".into()),
                callee: "Order.process".into(),
                line: 2,
                known_receiver: false,
                ..Default::default()
            }
            .finish()],
            &[handle],
            &[],
            &idx,
            "repo",
        );
        assert_eq!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "order.py", "Order.process"))
        );
        assert_eq!(resolved[0].class, ResolutionClass::ResolvedInternal);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.class-name-module verifies=REQ-implement-phase-21-of-scc-x-ripwire-lessons-absorb-rule-2c-class-name exercises=impl.scc.resolve.class-name
    fn module_name_is_not_a_class_name_receiver() {
        let mut idx = SymbolIndex::new("repo");
        let mut join = mk_symbol("Os.join", SymbolKind::Function);
        join.parent = Some("Os".into());
        let handle = mk_symbol("handle", SymbolKind::Function);
        let syms = vec![mk_symbol("Os", SymbolKind::Module), join, handle];
        idx.add_file("w.py", &syms);
        let resolved = resolve_calls(
            "w.py",
            &[Call {
                caller: Some("handle".into()),
                callee: "Os.join".into(),
                line: 2,
                known_receiver: false,
                ..Default::default()
            }
            .finish()],
            &syms,
            &[],
            &idx,
            "repo",
        );
        assert_eq!(resolved[0].callee_id, None);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.cha-base-pin verifies=REQ-implement-phase-22-of-scc-x-ripwire-lessons-absorb-rule-2c-cha-meth exercises=impl.scc.resolve.cha-bases
    fn class_name_receiver_pins_unique_base_method() {
        let mut idx = SymbolIndex::new("repo");
        let mut open = mk_symbol("IERS.open", SymbolKind::Method);
        open.parent = Some("IERS".into());
        idx.add_file("iers.py", &[mk_symbol("IERS", SymbolKind::Class), open]);
        let iers_b = mk_symbol("IERS_B", SymbolKind::Class);
        idx.add_file("iers_b.py", std::slice::from_ref(&iers_b));
        idx.set_class_bases("iers_b.py", &[("IERS_B".into(), vec!["IERS".into()])]);
        let handle = mk_symbol("handle", SymbolKind::Function);
        idx.add_file("w.py", std::slice::from_ref(&handle));
        let resolved = resolve_calls(
            "w.py",
            &[Call {
                caller: Some("handle".into()),
                callee: "IERS_B.open".into(),
                line: 2,
                known_receiver: false,
                ..Default::default()
            }
            .finish()],
            std::slice::from_ref(&handle),
            &[],
            &idx,
            "repo",
        );
        assert_eq!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "iers.py", "IERS.open"))
        );
        assert_eq!(resolved[0].class, ResolutionClass::ResolvedInternal);
        assert_eq!(resolved[0].provenance, scc_core::Provenance::Extracted);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.cha-base-split verifies=REQ-implement-phase-22-of-scc-x-ripwire-lessons-absorb-rule-2c-cha-meth exercises=impl.scc.resolve.cha-bases
    fn two_bases_defining_same_method_stay_unresolved() {
        let mut idx = SymbolIndex::new("repo");
        let mut a_m = mk_symbol("A.m", SymbolKind::Method);
        a_m.parent = Some("A".into());
        idx.add_file("a.py", &[mk_symbol("A", SymbolKind::Class), a_m]);
        let mut b_m = mk_symbol("B.m", SymbolKind::Method);
        b_m.parent = Some("B".into());
        idx.add_file("b.py", &[mk_symbol("B", SymbolKind::Class), b_m]);
        let c = mk_symbol("C", SymbolKind::Class);
        idx.add_file("c.py", std::slice::from_ref(&c));
        idx.set_class_bases("c.py", &[("C".into(), vec!["A".into(), "B".into()])]);
        let handle = mk_symbol("handle", SymbolKind::Function);
        idx.add_file("w.py", std::slice::from_ref(&handle));
        let resolved = resolve_calls(
            "w.py",
            &[Call {
                caller: Some("handle".into()),
                callee: "C.m".into(),
                line: 2,
                known_receiver: false,
                ..Default::default()
            }
            .finish()],
            std::slice::from_ref(&handle),
            &[],
            &idx,
            "repo",
        );
        assert_eq!(resolved[0].callee_id, None);
        assert_eq!(resolved[0].class, ResolutionClass::UnresolvedLikelyInternal);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.cha-grandparent verifies=REQ-implement-phase-22-of-scc-x-ripwire-lessons-absorb-rule-2c-cha-meth exercises=impl.scc.resolve.cha-bases
    fn class_name_receiver_pins_grandparent_when_parent_has_no_method() {
        let mut idx = SymbolIndex::new("repo");
        let mut open = mk_symbol("IERS.open", SymbolKind::Method);
        open.parent = Some("IERS".into());
        idx.add_file("iers.py", &[mk_symbol("IERS", SymbolKind::Class), open]);
        let iers_a = mk_symbol("IERS_A", SymbolKind::Class);
        idx.add_file("mid.py", std::slice::from_ref(&iers_a));
        idx.set_class_bases("mid.py", &[("IERS_A".into(), vec!["IERS".into()])]);
        let iers_b = mk_symbol("IERS_B", SymbolKind::Class);
        idx.add_file("leaf.py", std::slice::from_ref(&iers_b));
        idx.set_class_bases("leaf.py", &[("IERS_B".into(), vec!["IERS_A".into()])]);
        let handle = mk_symbol("handle", SymbolKind::Function);
        idx.add_file("w.py", std::slice::from_ref(&handle));
        let resolved = resolve_calls(
            "w.py",
            &[Call {
                caller: Some("handle".into()),
                callee: "IERS_B.open".into(),
                line: 2,
                known_receiver: false,
                ..Default::default()
            }
            .finish()],
            std::slice::from_ref(&handle),
            &[],
            &idx,
            "repo",
        );
        assert_eq!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "iers.py", "IERS.open"))
        );
    }

    #[test]
    // trace:v1 id=test.scc.resolve.cha-typed-local verifies=REQ-implement-phase-22-of-scc-x-ripwire-lessons-absorb-rule-2c-cha-meth exercises=impl.scc.resolve.cha-bases
    fn typed_local_of_derived_class_pins_base_method() {
        let mut idx = SymbolIndex::new("repo");
        let mut open = mk_symbol("IERS.open", SymbolKind::Method);
        open.parent = Some("IERS".into());
        let handle = mk_symbol("handle", SymbolKind::Function);
        let syms = vec![
            mk_symbol("IERS", SymbolKind::Class),
            open,
            mk_symbol("IERS_B", SymbolKind::Class),
            handle.clone(),
        ];
        idx.add_file("w.py", &syms);
        idx.set_class_bases("w.py", &[("IERS_B".into(), vec!["IERS".into()])]);
        idx.set_type_binds(
            "w.py",
            &[TypeBind {
                scope: "handle".into(),
                name: "x".into(),
                type_name: "IERS_B".into(),
                line: 8,
            }],
        );
        let resolved = resolve_calls(
            "w.py",
            &[Call {
                caller: Some("handle".into()),
                callee: "x.open".into(),
                line: 9,
                known_receiver: false,
                ..Default::default()
            }
            .finish()],
            &syms,
            &[],
            &idx,
            "repo",
        );
        assert_eq!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "w.py", "IERS.open"))
        );
    }

    #[test]
    // trace:v1 id=test.scc.resolve.self-cha verifies=REQ-implement-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-s exercises=impl.scc.resolve.rule1
    fn self_open_on_derived_class_pins_unique_base() {
        let mut idx = SymbolIndex::new("repo");
        let mut open = mk_symbol("IERS.open", SymbolKind::Method);
        open.parent = Some("IERS".into());
        idx.add_file("iers.py", &[mk_symbol("IERS", SymbolKind::Class), open]);
        let mut run = mk_symbol("IERS_B.run", SymbolKind::Method);
        run.parent = Some("IERS_B".into());
        let iers_b = mk_symbol("IERS_B", SymbolKind::Class);
        idx.add_file("iers_b.py", &[iers_b, run.clone()]);
        idx.set_class_bases("iers_b.py", &[("IERS_B".into(), vec!["IERS".into()])]);
        for callee in ["self.open", "this.open"] {
            let resolved = resolve_calls(
                "iers_b.py",
                &[Call {
                    caller: Some("IERS_B.run".into()),
                    callee: callee.into(),
                    line: 3,
                    known_receiver: true,
                    ..Default::default()
                }
                .finish()],
                std::slice::from_ref(&run),
                &[],
                &idx,
                "repo",
            );
            assert_eq!(
                resolved[0].callee_id,
                Some(scc_core::symbol_id("repo", "iers.py", "IERS.open")),
                "{callee}"
            );
        }
    }

    #[test]
    // trace:v1 id=test.scc.resolve.self-sibling-beats-cha verifies=REQ-implement-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-s exercises=impl.scc.resolve.rule1
    fn self_open_prefers_own_class_over_base() {
        let mut idx = SymbolIndex::new("repo");
        let mut base_open = mk_symbol("IERS.open", SymbolKind::Method);
        base_open.parent = Some("IERS".into());
        idx.add_file(
            "iers.py",
            &[mk_symbol("IERS", SymbolKind::Class), base_open],
        );
        let mut own_open = mk_symbol("IERS_B.open", SymbolKind::Method);
        own_open.parent = Some("IERS_B".into());
        let mut run = mk_symbol("IERS_B.run", SymbolKind::Method);
        run.parent = Some("IERS_B".into());
        let syms = vec![
            mk_symbol("IERS_B", SymbolKind::Class),
            own_open,
            run.clone(),
        ];
        idx.add_file("iers_b.py", &syms);
        idx.set_class_bases("iers_b.py", &[("IERS_B".into(), vec!["IERS".into()])]);
        let resolved = resolve_calls(
            "iers_b.py",
            &[Call {
                caller: Some("IERS_B.run".into()),
                callee: "self.open".into(),
                line: 4,
                known_receiver: true,
                ..Default::default()
            }
            .finish()],
            &syms,
            &[],
            &idx,
            "repo",
        );
        assert_eq!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "iers_b.py", "IERS_B.open"))
        );
    }

    #[test]
    // trace:v1 id=test.scc.resolve.super-cha verifies=REQ-implement-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-s exercises=impl.scc.resolve.rule1
    fn super_open_skips_own_class_and_pins_base() {
        let mut idx = SymbolIndex::new("repo");
        let mut base_open = mk_symbol("IERS.open", SymbolKind::Method);
        base_open.parent = Some("IERS".into());
        idx.add_file(
            "iers.py",
            &[mk_symbol("IERS", SymbolKind::Class), base_open],
        );
        let mut own_open = mk_symbol("IERS_B.open", SymbolKind::Method);
        own_open.parent = Some("IERS_B".into());
        let mut run = mk_symbol("IERS_B.run", SymbolKind::Method);
        run.parent = Some("IERS_B".into());
        let syms = vec![
            mk_symbol("IERS_B", SymbolKind::Class),
            own_open,
            run.clone(),
        ];
        idx.add_file("iers_b.py", &syms);
        idx.set_class_bases("iers_b.py", &[("IERS_B".into(), vec!["IERS".into()])]);
        for callee in ["super.open", "super().open"] {
            let resolved = resolve_calls(
                "iers_b.py",
                &[Call {
                    caller: Some("IERS_B.run".into()),
                    callee: callee.into(),
                    line: 4,
                    known_receiver: true,
                    ..Default::default()
                }
                .finish()],
                &syms,
                &[],
                &idx,
                "repo",
            );
            assert_eq!(
                resolved[0].callee_id,
                Some(scc_core::symbol_id("repo", "iers.py", "IERS.open")),
                "{callee}"
            );
            assert_eq!(resolved[0].recv, RecvKind::Super, "{callee}");
        }
    }

    #[test]
    // trace:v1 id=test.scc.resolve.self-cha-split verifies=REQ-implement-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-s exercises=impl.scc.resolve.rule1
    fn self_method_two_hitting_bases_unresolved() {
        let mut idx = SymbolIndex::new("repo");
        let mut a_m = mk_symbol("A.m", SymbolKind::Method);
        a_m.parent = Some("A".into());
        idx.add_file("a.py", &[mk_symbol("A", SymbolKind::Class), a_m]);
        let mut b_m = mk_symbol("B.m", SymbolKind::Method);
        b_m.parent = Some("B".into());
        idx.add_file("b.py", &[mk_symbol("B", SymbolKind::Class), b_m]);
        let mut run = mk_symbol("C.run", SymbolKind::Method);
        run.parent = Some("C".into());
        idx.add_file("c.py", &[mk_symbol("C", SymbolKind::Class), run.clone()]);
        idx.set_class_bases("c.py", &[("C".into(), vec!["A".into(), "B".into()])]);
        let resolved = resolve_calls(
            "c.py",
            &[Call {
                caller: Some("C.run".into()),
                callee: "self.m".into(),
                line: 3,
                known_receiver: true,
                ..Default::default()
            }
            .finish()],
            std::slice::from_ref(&run),
            &[],
            &idx,
            "repo",
        );
        assert_eq!(resolved[0].callee_id, None);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.super-cha-split verifies=REQ-implement-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-s exercises=impl.scc.resolve.rule1
    fn super_method_two_hitting_bases_unresolved() {
        let mut idx = SymbolIndex::new("repo");
        let mut a_m = mk_symbol("A.m", SymbolKind::Method);
        a_m.parent = Some("A".into());
        idx.add_file("a.py", &[mk_symbol("A", SymbolKind::Class), a_m]);
        let mut b_m = mk_symbol("B.m", SymbolKind::Method);
        b_m.parent = Some("B".into());
        idx.add_file("b.py", &[mk_symbol("B", SymbolKind::Class), b_m]);
        let mut own = mk_symbol("C.m", SymbolKind::Method);
        own.parent = Some("C".into());
        let mut run = mk_symbol("C.run", SymbolKind::Method);
        run.parent = Some("C".into());
        let syms = vec![mk_symbol("C", SymbolKind::Class), own, run.clone()];
        idx.add_file("c.py", &syms);
        idx.set_class_bases("c.py", &[("C".into(), vec!["A".into(), "B".into()])]);
        let resolved = resolve_calls(
            "c.py",
            &[Call {
                caller: Some("C.run".into()),
                callee: "super().m".into(),
                line: 4,
                known_receiver: true,
                ..Default::default()
            }
            .finish()],
            &syms,
            &[],
            &idx,
            "repo",
        );
        assert_eq!(resolved[0].callee_id, None);
        assert_eq!(resolved[0].recv, RecvKind::Super);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.super-field-chain verifies=REQ-implement-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-s exercises=impl.scc.resolve.rule1
    fn super_field_chain_stays_unresolved() {
        let mut idx = SymbolIndex::new("repo");
        let mut open = mk_symbol("IERS.open", SymbolKind::Method);
        open.parent = Some("IERS".into());
        idx.add_file("iers.py", &[mk_symbol("IERS", SymbolKind::Class), open]);
        let mut run = mk_symbol("IERS_B.run", SymbolKind::Method);
        run.parent = Some("IERS_B".into());
        idx.add_file(
            "iers_b.py",
            &[mk_symbol("IERS_B", SymbolKind::Class), run.clone()],
        );
        idx.set_class_bases("iers_b.py", &[("IERS_B".into(), vec!["IERS".into()])]);
        let resolved = resolve_calls(
            "iers_b.py",
            &[Call {
                caller: Some("IERS_B.run".into()),
                callee: "super().client.open".into(),
                line: 3,
                known_receiver: true,
                ..Default::default()
            }
            .finish()],
            std::slice::from_ref(&run),
            &[],
            &idx,
            "repo",
        );
        assert_eq!(resolved[0].callee_id, None);
        assert_eq!(resolved[0].recv, RecvKind::Super);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.self-field-not-rule1 verifies=REQ-implement-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-s exercises=impl.scc.resolve.rule1
    fn self_field_chain_does_not_use_rule1() {
        let mut idx = SymbolIndex::new("repo");
        let mut open = mk_symbol("IERS.open", SymbolKind::Method);
        open.parent = Some("IERS".into());
        idx.add_file("iers.py", &[mk_symbol("IERS", SymbolKind::Class), open]);
        let mut run = mk_symbol("IERS_B.run", SymbolKind::Method);
        run.parent = Some("IERS_B".into());
        idx.add_file(
            "iers_b.py",
            &[mk_symbol("IERS_B", SymbolKind::Class), run.clone()],
        );
        idx.set_class_bases("iers_b.py", &[("IERS_B".into(), vec!["IERS".into()])]);
        let resolved = resolve_calls(
            "iers_b.py",
            &[Call {
                caller: Some("IERS_B.run".into()),
                callee: "self.client.open".into(),
                line: 3,
                known_receiver: true,
                ..Default::default()
            }
            .finish()],
            std::slice::from_ref(&run),
            &[],
            &idx,
            "repo",
        );
        assert_eq!(resolved[0].callee_id, None);
        assert_eq!(resolved[0].recv, RecvKind::FieldOfSelf);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.rust-trait-cha verifies=REQ-implement-phase-24-of-scc-x-ripwire-lessons-absorb-rust-impl-trait-fo exercises=impl.scc.extract.rust.trait-impl
    fn rust_trait_impl_pins_unique_trait_method() {
        let mut idx = SymbolIndex::new("repo");
        let mut open = mk_symbol("Open.open", SymbolKind::Method);
        open.parent = Some("Open".into());
        idx.add_file("open.rs", &[mk_symbol("Open", SymbolKind::Interface), open]);
        let handle = mk_symbol("handle", SymbolKind::Function);
        let iers_b = mk_symbol("IERS_B", SymbolKind::Class);
        idx.add_file("w.rs", &[iers_b, handle.clone()]);
        idx.set_class_bases("w.rs", &[("IERS_B".into(), vec!["Open".into()])]);
        idx.set_type_binds(
            "w.rs",
            &[TypeBind {
                scope: "handle".into(),
                name: "x".into(),
                type_name: "IERS_B".into(),
                line: 4,
            }],
        );
        let resolved = resolve_calls(
            "w.rs",
            &[Call {
                caller: Some("handle".into()),
                callee: "x.open".into(),
                line: 5,
                known_receiver: false,
                ..Default::default()
            }
            .finish()],
            std::slice::from_ref(&handle),
            &[],
            &idx,
            "repo",
        );
        assert_eq!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "open.rs", "Open.open"))
        );
    }

    #[test]
    // trace:v1 id=test.scc.resolve.rust-trait-inherent-wins verifies=REQ-implement-phase-24-of-scc-x-ripwire-lessons-absorb-rust-impl-trait-fo exercises=impl.scc.resolve.cha-bases
    fn rust_inherent_impl_wins_over_trait_cha() {
        let mut idx = SymbolIndex::new("repo");
        let mut trait_open = mk_symbol("Open.open", SymbolKind::Method);
        trait_open.parent = Some("Open".into());
        idx.add_file(
            "open.rs",
            &[mk_symbol("Open", SymbolKind::Interface), trait_open],
        );
        let mut own = mk_symbol("IERS_B.open", SymbolKind::Method);
        own.parent = Some("IERS_B".into());
        let handle = mk_symbol("handle", SymbolKind::Function);
        let syms = vec![mk_symbol("IERS_B", SymbolKind::Class), own, handle.clone()];
        idx.add_file("w.rs", &syms);
        idx.set_class_bases("w.rs", &[("IERS_B".into(), vec!["Open".into()])]);
        idx.set_type_binds(
            "w.rs",
            &[TypeBind {
                scope: "handle".into(),
                name: "x".into(),
                type_name: "IERS_B".into(),
                line: 6,
            }],
        );
        let resolved = resolve_calls(
            "w.rs",
            &[Call {
                caller: Some("handle".into()),
                callee: "x.open".into(),
                line: 7,
                known_receiver: false,
                ..Default::default()
            }
            .finish()],
            &syms,
            &[],
            &idx,
            "repo",
        );
        assert_eq!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "w.rs", "IERS_B.open"))
        );
    }

    #[test]
    // trace:v1 id=test.scc.resolve.rust-trait-split verifies=REQ-implement-phase-24-of-scc-x-ripwire-lessons-absorb-rust-impl-trait-fo exercises=impl.scc.resolve.cha-bases
    fn rust_two_traits_defining_method_unresolved() {
        let mut idx = SymbolIndex::new("repo");
        let mut a_m = mk_symbol("A.m", SymbolKind::Method);
        a_m.parent = Some("A".into());
        idx.add_file("a.rs", &[mk_symbol("A", SymbolKind::Interface), a_m]);
        let mut b_m = mk_symbol("B.m", SymbolKind::Method);
        b_m.parent = Some("B".into());
        idx.add_file("b.rs", &[mk_symbol("B", SymbolKind::Interface), b_m]);
        let handle = mk_symbol("handle", SymbolKind::Function);
        idx.add_file("c.rs", &[mk_symbol("C", SymbolKind::Class), handle.clone()]);
        idx.set_class_bases("c.rs", &[("C".into(), vec!["A".into(), "B".into()])]);
        idx.set_type_binds(
            "c.rs",
            &[TypeBind {
                scope: "handle".into(),
                name: "x".into(),
                type_name: "C".into(),
                line: 3,
            }],
        );
        let resolved = resolve_calls(
            "c.rs",
            &[Call {
                caller: Some("handle".into()),
                callee: "x.m".into(),
                line: 4,
                known_receiver: false,
                ..Default::default()
            }
            .finish()],
            std::slice::from_ref(&handle),
            &[],
            &idx,
            "repo",
        );
        assert_eq!(resolved[0].callee_id, None);
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
        idx.add_file("orders.py", &[mk_symbol("Order", SymbolKind::Class), save]);
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
    // trace:v1 id=test.scc.resolve.python.ident-copy verifies=REQ-implement-phase-17-of-scc-x-ripwire-lessons-python-extract-time-ident exercises=impl.scc.extract.python.ident-copy
    fn python_ident_copy_pin_and_tombstone() {
        use crate::model::{LanguageExtractor, SourceFile};
        use crate::python::PythonExtractor;
        let src = "class Order:\n    def process(self):\n        pass\nclass Invoice:\n    def process(self):\n        pass\ndef handle(x: Order):\n    y = x\n    y.process()\n    y.inner.process()\ndef mixed():\n    z = Order()\n    w = z\n    w = Invoice()\n    w.process()\n";
        let ef = PythonExtractor::default().extract(&SourceFile::new("w.py", src));
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("w.py", &ef.symbols);
        idx.set_type_binds("w.py", &ef.type_binds);
        let resolved = resolve_calls("w.py", &ef.calls, &ef.symbols, &[], &idx, "repo");
        let handle_id = scc_core::symbol_id("repo", "w.py", "handle");
        let y = resolved
            .iter()
            .find(|c| c.callee_name == "y.process" && c.caller_id == handle_id)
            .expect("y.process");
        assert_eq!(
            y.callee_id,
            Some(scc_core::symbol_id("repo", "w.py", "Order.process"))
        );
        let chain = resolved
            .iter()
            .find(|c| c.callee_name == "y.inner.process")
            .expect("longer chain call");
        assert_eq!(chain.callee_id, None, "longer chain must stay unresolved");
        let w = resolved
            .iter()
            .find(|c| c.callee_name == "w.process")
            .expect("w.process");
        assert_eq!(w.callee_id, None, "conflicting copy/ctor must not pin");
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
    // trace:v1 id=test.scc.resolve.go.local-type verifies=REQ-implement-phase-15-of-scc-x-ripwire-lessons-go-and-rust-extract-time exercises=impl.scc.extract.go.local-type
    fn go_local_and_param_pin_and_assignment_tombstones() {
        use crate::go::GoExtractor;
        use crate::model::{LanguageExtractor, SourceFile};
        let src = r#"
package app
type Order struct{}
func (o *Order) Process() {}
type Invoice struct{}
func (i *Invoice) Process() {}
func handle(x *Order) {
	y := &Order{}
	y.Process()
	x.Process()
	y.inner.Process()
}
func mixed() {
	z := &Order{}
	z = &Invoice{}
	z.Process()
}
func factory() {
	x := MakeOrder()
	x.Process()
}
"#;
        let ef = GoExtractor::default().extract(&SourceFile::new("w.go", src));
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("w.go", &ef.symbols);
        idx.set_type_binds("w.go", &ef.type_binds);
        let resolved = resolve_calls("w.go", &ef.calls, &ef.symbols, &[], &idx, "repo");
        let handle_id = scc_core::symbol_id("repo", "w.go", "handle");
        let y = resolved
            .iter()
            .find(|c| c.callee_name == "y.Process")
            .expect("y.Process");
        assert_eq!(
            y.callee_id,
            Some(scc_core::symbol_id("repo", "w.go", "Order.Process"))
        );
        let x = resolved
            .iter()
            .find(|c| c.callee_name == "x.Process" && c.caller_id == handle_id)
            .expect("x.Process");
        assert_eq!(
            x.callee_id,
            Some(scc_core::symbol_id("repo", "w.go", "Order.Process"))
        );
        let z = resolved
            .iter()
            .find(|c| c.callee_name == "z.Process")
            .expect("z.Process");
        assert_eq!(
            z.callee_id, None,
            "conflicting local assignment must not pin"
        );
        assert_eq!(z.recv, RecvKind::NamedVariable);
        let chain = resolved
            .iter()
            .find(|c| c.callee_name == "y.inner.Process")
            .expect("longer chain call");
        assert_eq!(chain.callee_id, None, "longer chain must stay unresolved");
        let factory_id = scc_core::symbol_id("repo", "w.go", "factory");
        let factory = resolved
            .iter()
            .find(|c| c.callee_name == "x.Process" && c.caller_id == factory_id)
            .expect("factory x.Process");
        assert_eq!(factory.callee_id, None, "opaque factory call must not pin");
    }

    #[test]
    // trace:v1 id=test.scc.resolve.rust.local-type verifies=REQ-implement-phase-15-of-scc-x-ripwire-lessons-go-and-rust-extract-time exercises=impl.scc.extract.rust.local-type
    fn rust_local_and_param_pin_and_assignment_tombstones() {
        use crate::model::{LanguageExtractor, SourceFile};
        use crate::rust::RustExtractor;
        let src = r#"
struct Order {}
impl Order { fn process(&self) {} }
struct Invoice {}
impl Invoice { fn process(&self) {} }
fn handle(x: Order) {
    let y = Order {};
    y.process();
    x.process();
    y.inner.process();
}
fn mixed() {
    let mut z = Order {};
    z = Invoice {};
    z.process();
}
fn factory() {
    let x = make_order();
    x.process();
}
"#;
        let ef = RustExtractor::default().extract(&SourceFile::new("w.rs", src));
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("w.rs", &ef.symbols);
        idx.set_type_binds("w.rs", &ef.type_binds);
        let resolved = resolve_calls("w.rs", &ef.calls, &ef.symbols, &[], &idx, "repo");
        let handle_id = scc_core::symbol_id("repo", "w.rs", "handle");
        let y = resolved
            .iter()
            .find(|c| c.callee_name == "y.process")
            .expect("y.process");
        assert_eq!(
            y.callee_id,
            Some(scc_core::symbol_id("repo", "w.rs", "Order.process"))
        );
        let x = resolved
            .iter()
            .find(|c| c.callee_name == "x.process" && c.caller_id == handle_id)
            .expect("x.process");
        assert_eq!(
            x.callee_id,
            Some(scc_core::symbol_id("repo", "w.rs", "Order.process"))
        );
        let z = resolved
            .iter()
            .find(|c| c.callee_name == "z.process")
            .expect("z.process");
        assert_eq!(
            z.callee_id, None,
            "conflicting local assignment must not pin"
        );
        assert_eq!(z.recv, RecvKind::NamedVariable);
        let chain = resolved
            .iter()
            .find(|c| c.callee_name == "y.inner.process")
            .expect("longer chain call");
        assert_eq!(chain.callee_id, None, "longer chain must stay unresolved");
        let factory_id = scc_core::symbol_id("repo", "w.rs", "factory");
        let factory = resolved
            .iter()
            .find(|c| c.callee_name == "x.process" && c.caller_id == factory_id)
            .expect("factory x.process");
        assert_eq!(factory.callee_id, None, "opaque factory call must not pin");
    }

    #[test]
    // trace:v1 id=test.scc.resolve.go.type-assert verifies=REQ-implement-phase-16-of-scc-x-ripwire-lessons-extract-time-type-asserti exercises=impl.scc.extract.go.type-assert
    fn go_type_assert_and_conversion_pin_and_tombstone() {
        use crate::go::GoExtractor;
        use crate::model::{LanguageExtractor, SourceFile};
        let src = r#"
package app
type Order struct{}
func (o *Order) Process() {}
type Invoice struct{}
func (i *Invoice) Process() {}
func handle(v any) {
	x := v.(*Order)
	x.Process()
	y := Order(v)
	y.Process()
	x.inner.Process()
}
func mixed(v any) {
	z := v.(*Order)
	z = v.(*Invoice)
	z.Process()
}
"#;
        let ef = GoExtractor::default().extract(&SourceFile::new("w.go", src));
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("w.go", &ef.symbols);
        idx.set_type_binds("w.go", &ef.type_binds);
        let resolved = resolve_calls("w.go", &ef.calls, &ef.symbols, &[], &idx, "repo");
        let handle_id = scc_core::symbol_id("repo", "w.go", "handle");
        let x = resolved
            .iter()
            .find(|c| c.callee_name == "x.Process" && c.caller_id == handle_id)
            .expect("x.Process");
        assert_eq!(
            x.callee_id,
            Some(scc_core::symbol_id("repo", "w.go", "Order.Process"))
        );
        let y = resolved
            .iter()
            .find(|c| c.callee_name == "y.Process")
            .expect("y.Process");
        assert_eq!(
            y.callee_id,
            Some(scc_core::symbol_id("repo", "w.go", "Order.Process"))
        );
        let chain = resolved
            .iter()
            .find(|c| c.callee_name == "x.inner.Process")
            .expect("longer chain call");
        assert_eq!(chain.callee_id, None, "longer chain must stay unresolved");
        let z = resolved
            .iter()
            .find(|c| c.callee_name == "z.Process")
            .expect("z.Process");
        assert_eq!(z.callee_id, None, "conflicting assertion must not pin");
    }

    #[test]
    // trace:v1 id=test.scc.resolve.rust.type-cast verifies=REQ-implement-phase-16-of-scc-x-ripwire-lessons-extract-time-type-asserti exercises=impl.scc.extract.rust.type-cast
    fn rust_as_cast_pin_and_tombstone() {
        use crate::model::{LanguageExtractor, SourceFile};
        use crate::rust::RustExtractor;
        let src = r#"
struct Order {}
impl Order { fn process(&self) {} }
struct Invoice {}
impl Invoice { fn process(&self) {} }
fn handle(v: Order) {
    let y = v as Order;
    y.process();
    y.inner.process();
}
fn mixed(v: Order, w: Invoice) {
    let mut z = v as Order;
    z = w as Invoice;
    z.process();
}
"#;
        let ef = RustExtractor::default().extract(&SourceFile::new("w.rs", src));
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("w.rs", &ef.symbols);
        idx.set_type_binds("w.rs", &ef.type_binds);
        let resolved = resolve_calls("w.rs", &ef.calls, &ef.symbols, &[], &idx, "repo");
        let y = resolved
            .iter()
            .find(|c| c.callee_name == "y.process")
            .expect("y.process");
        assert_eq!(
            y.callee_id,
            Some(scc_core::symbol_id("repo", "w.rs", "Order.process"))
        );
        let chain = resolved
            .iter()
            .find(|c| c.callee_name == "y.inner.process")
            .expect("longer chain call");
        assert_eq!(chain.callee_id, None, "longer chain must stay unresolved");
        let z = resolved
            .iter()
            .find(|c| c.callee_name == "z.process")
            .expect("z.process");
        assert_eq!(z.callee_id, None, "conflicting as-cast must not pin");
    }

    #[test]
    // trace:v1 id=test.scc.resolve.ts.type-cast verifies=REQ-implement-phase-16-of-scc-x-ripwire-lessons-extract-time-type-asserti exercises=impl.scc.extract.ts.type-cast
    fn ts_as_cast_pin_and_tombstone() {
        use crate::model::{LanguageExtractor, SourceFile};
        use crate::typescript::TypeScriptExtractor;
        let src = "class Order { process() {} }\nclass Invoice { process() {} }\nfunction handle(v: unknown) {\n  const y = v as Order;\n  y.process();\n  y.inner.process();\n}\nfunction mixed(v: unknown) {\n  let x = v as Order;\n  x = v as Invoice;\n  x.process();\n}\n";
        let ef = TypeScriptExtractor::default().extract(&SourceFile::new("w.ts", src));
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("w.ts", &ef.symbols);
        idx.set_type_binds("w.ts", &ef.type_binds);
        let resolved = resolve_calls("w.ts", &ef.calls, &ef.symbols, &[], &idx, "repo");
        let handle_id = scc_core::symbol_id("repo", "w.ts", "handle");
        let y = resolved
            .iter()
            .find(|c| c.callee_name == "y.process" && c.caller_id == handle_id)
            .expect("y.process");
        assert_eq!(
            y.callee_id,
            Some(scc_core::symbol_id("repo", "w.ts", "Order.process"))
        );
        let chain = resolved
            .iter()
            .find(|c| c.callee_name == "y.inner.process")
            .expect("longer chain call");
        assert_eq!(chain.callee_id, None, "longer chain must stay unresolved");
        let mixed_id = scc_core::symbol_id("repo", "w.ts", "mixed");
        let x = resolved
            .iter()
            .find(|c| c.callee_name == "x.process" && c.caller_id == mixed_id)
            .expect("x.process");
        assert_eq!(x.callee_id, None, "conflicting as-cast must not pin");
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
            resolved[0].callee_id, None,
            "Python owned.process must not pin via Java field-as-receiver"
        );
        assert_eq!(resolved[0].recv, RecvKind::NamedVariable);
    }

    #[test]
    // trace:v1 id=test.scc.resolve.fn-alias verifies=REQ-implement-phase-25-of-scc-x-ripwire-lessons-absorb-extract-time-funct exercises=impl.scc.resolve.fn-alias
    fn python_fn_alias_pins_and_never_sprays_same_named_global() {
        use crate::model::{LanguageExtractor, SourceFile};
        use crate::python::PythonExtractor;
        let src = "def helper():\n    return 1\ndef f():\n    return 2\ndef run():\n    f = helper\n    f()\ndef mixed():\n    g = helper\n    g = f\n    g()\ndef lam():\n    h = lambda: 1\n    h()\nf2 = helper\ndef file_scope():\n    f2()\n";
        let ef = PythonExtractor::default().extract(&SourceFile::new("w.py", src));
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("w.py", &ef.symbols);
        idx.set_fn_binds("w.py", &ef.fn_binds);
        let resolved = resolve_calls("w.py", &ef.calls, &ef.symbols, &[], &idx, "repo");
        let run_id = scc_core::symbol_id("repo", "w.py", "run");
        let hit = resolved
            .iter()
            .find(|c| c.callee_name == "f" && c.caller_id == run_id)
            .expect("run f()");
        assert_eq!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "w.py", "helper"))
        );
        assert_ne!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "w.py", "f")),
            "must not spray to the same-named global function"
        );
        assert_eq!(hit.class, ResolutionClass::ResolvedInternal);
        let mixed_id = scc_core::symbol_id("repo", "w.py", "mixed");
        let g = resolved
            .iter()
            .find(|c| c.callee_name == "g" && c.caller_id == mixed_id)
            .expect("mixed g()");
        assert_eq!(g.callee_id, None, "two targets must tombstone");
        let lam_id = scc_core::symbol_id("repo", "w.py", "lam");
        let h = resolved
            .iter()
            .find(|c| c.callee_name == "h" && c.caller_id == lam_id)
            .expect("lam h()");
        assert_eq!(h.callee_id, None, "lambda must block the name ladder");
        assert_ne!(h.callee_id, Some(scc_core::symbol_id("repo", "w.py", "f")));
        let fs_id = scc_core::symbol_id("repo", "w.py", "file_scope");
        let f2 = resolved
            .iter()
            .find(|c| c.callee_name == "f2" && c.caller_id == fs_id)
            .expect("file_scope f2()");
        assert_eq!(
            f2.callee_id,
            Some(scc_core::symbol_id("repo", "w.py", "helper")),
            "file-scope alias must pin helper"
        );
    }

    #[test]
    // trace:v1 id=test.scc.resolve.fn-alias-split verifies=REQ-implement-phase-25-of-scc-x-ripwire-lessons-absorb-extract-time-funct exercises=impl.scc.resolve.fn-alias
    fn two_same_named_functions_across_files_stay_unresolved() {
        use crate::model::{LanguageExtractor, SourceFile};
        use crate::python::PythonExtractor;
        let a = PythonExtractor::default()
            .extract(&SourceFile::new("a.py", "def helper():\n    return 1\n"));
        let b = PythonExtractor::default()
            .extract(&SourceFile::new("b.py", "def helper():\n    return 2\n"));
        let w = PythonExtractor::default().extract(&SourceFile::new(
            "w.py",
            "def run():\n    f = helper\n    f()\n",
        ));
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("a.py", &a.symbols);
        idx.add_file("b.py", &b.symbols);
        idx.add_file("w.py", &w.symbols);
        idx.set_fn_binds("w.py", &w.fn_binds);
        let resolved = resolve_calls("w.py", &w.calls, &w.symbols, &[], &idx, "repo");
        let hit = resolved.iter().find(|c| c.callee_name == "f").expect("f()");
        assert_eq!(hit.callee_id, None, "two helpers must not spray");
    }

    #[test]
    // trace:v1 id=test.scc.resolve.fn-alias-const verifies=REQ-implement-phase-25-of-scc-x-ripwire-lessons-absorb-extract-time-funct,REQ-implement-fix-pr-review-comments-without-collapsing-scc-type-script-no exercises=impl.scc.resolve.fn-alias
    fn ts_function_valued_const_pins_and_limit_does_not() {
        use crate::model::{LanguageExtractor, SourceFile, SymbolKind};
        use crate::typescript::TypeScriptExtractor;
        let src = "const helper = () => 1;\nconst LIMIT = 10;\nfunction f() { return 2; }\nfunction run() {\n  const g = helper;\n  g();\n}\nfunction bad() {\n  const h = LIMIT;\n  h();\n}\n";
        let ef = TypeScriptExtractor::default().extract(&SourceFile::new("w.ts", src));
        let helper = ef
            .symbols
            .iter()
            .find(|s| s.name == "helper")
            .expect("helper");
        assert_eq!(helper.kind, SymbolKind::Const);
        assert!(
            helper.signature.is_some(),
            "function-valued const needs a signature"
        );
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("w.ts", &ef.symbols);
        idx.set_fn_binds("w.ts", &ef.fn_binds);
        let resolved = resolve_calls("w.ts", &ef.calls, &ef.symbols, &[], &idx, "repo");
        let run_id = scc_core::symbol_id("repo", "w.ts", "run");
        let hit = resolved
            .iter()
            .find(|c| c.callee_name == "g" && c.caller_id == run_id)
            .expect("run g()");
        assert_eq!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "w.ts", "helper"))
        );
        assert_ne!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "w.ts", "f")),
            "must not spray to a same-named global function"
        );
        assert_eq!(hit.class, ResolutionClass::ResolvedInternal);
        let bad_id = scc_core::symbol_id("repo", "w.ts", "bad");
        let h = resolved
            .iter()
            .find(|c| c.callee_name == "h" && c.caller_id == bad_id)
            .expect("bad h()");
        assert_eq!(h.callee_id, None, "signature-less const LIMIT must not pin");
    }

    fn internal_import(file: &str, names: &[(&str, &str)]) -> ResolvedImport {
        ResolvedImport {
            local_file: "w.py".into(),
            module: file.to_string(),
            target: ImportTarget::Internal {
                file: file.into(),
                name_map: HashMap::new(),
                namespace: false,
            },
            names: names
                .iter()
                .map(|(local, imported)| ((*local).to_string(), (*imported).to_string()))
                .collect(),
            line: 1,
        }
    }

    fn helper_call(caller: &str) -> Call {
        Call {
            caller: Some(caller.into()),
            callee: "helper".into(),
            line: 3,
            known_receiver: true,
            ..Default::default()
        }
        .finish()
    }

    #[test]
    // trace:v1 id=test.scc.resolve.rule3-include-file verifies=REQ-implement-phase-26-of-scc-x-ripwire-lessons-absorb-rule-3-import-incl exercises=impl.scc.resolve.rule3-include-file
    fn rule3_unique_imported_file_pins_and_controls_refuse() {
        let mut idx = SymbolIndex::new("repo");
        idx.add_file(
            "a.py",
            &[
                mk_symbol("helper", SymbolKind::Function),
                mk_symbol("foo", SymbolKind::Function),
            ],
        );
        idx.add_file("b.py", &[mk_symbol("helper", SymbolKind::Function)]);
        let run = mk_symbol("run", SymbolKind::Function);
        idx.add_file("w.py", std::slice::from_ref(&run));
        let calls = vec![helper_call("run")];
        let run_syms = std::slice::from_ref(&run);

        let only_a = resolve_calls(
            "w.py",
            &calls,
            run_syms,
            &[internal_import("a.py", &[("foo", "foo")])],
            &idx,
            "repo",
        );
        let hit = only_a
            .iter()
            .find(|c| c.callee_name == "helper")
            .expect("helper()");
        assert_eq!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "a.py", "helper")),
            "exactly one imported defining file must pin"
        );
        assert_ne!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "b.py", "helper"))
        );
        assert_eq!(hit.class, ResolutionClass::ResolvedInternal);

        let neither = resolve_calls("w.py", &calls, run_syms, &[], &idx, "repo");
        assert_eq!(
            neither[0].callee_id, None,
            "no import must not spray to either helper"
        );

        let both = resolve_calls(
            "w.py",
            &calls,
            run_syms,
            &[
                internal_import("a.py", &[("foo", "foo")]),
                internal_import("b.py", &[]),
            ],
            &idx,
            "repo",
        );
        assert_eq!(
            both[0].callee_id, None,
            "two imported defining files must stay unresolved"
        );

        let unresolved = resolve_calls(
            "w.py",
            &calls,
            run_syms,
            &[ResolvedImport {
                local_file: "w.py".into(),
                module: "./missing".into(),
                target: ImportTarget::Unresolved {
                    name: "./missing".into(),
                },
                names: vec![],
                line: 1,
            }],
            &idx,
            "repo",
        );
        assert_eq!(
            unresolved[0].callee_id, None,
            "unresolved import must not contribute a Rule 3 file"
        );

        let local = mk_symbol("helper", SymbolKind::Function);
        idx.add_file("w.py", &[run.clone(), local.clone()]);
        let same_file = resolve_calls(
            "w.py",
            &calls,
            &[run.clone(), local],
            &[internal_import("a.py", &[])],
            &idx,
            "repo",
        );
        assert_eq!(
            same_file[0].callee_id,
            Some(scc_core::symbol_id("repo", "w.py", "helper")),
            "same-file def must win"
        );
        assert_ne!(
            same_file[0].callee_id,
            Some(scc_core::symbol_id("repo", "a.py", "helper"))
        );
    }

    #[test]
    // trace:v1 id=test.scc.resolve.rule3-path-precise verifies=REQ-implement-phase-26-of-scc-x-ripwire-lessons-absorb-rule-3-import-incl exercises=impl.scc.resolve.rule3-include-file
    fn rule3_is_path_precise_not_basename() {
        let mut idx = SymbolIndex::new("repo");
        idx.add_file(
            "pkg/a.py",
            &[
                mk_symbol("helper", SymbolKind::Function),
                mk_symbol("foo", SymbolKind::Function),
            ],
        );
        idx.add_file("other/a.py", &[mk_symbol("helper", SymbolKind::Function)]);
        let run = mk_symbol("run", SymbolKind::Function);
        idx.add_file("w.py", std::slice::from_ref(&run));
        let import = Import {
            module: "pkg.a".into(),
            names: vec![("foo".into(), "foo".into())],
            line: 1,
            r#type: ImportType::Member,
        };
        let target = idx.resolve_import("w.py", &import);
        match &target {
            ImportTarget::Internal { file, .. } => assert_eq!(file, "pkg/a.py"),
            other => panic!("expected Internal pkg/a.py, got {other:?}"),
        }
        let resolved = resolve_calls(
            "w.py",
            &[helper_call("run")],
            &[run],
            &[ResolvedImport {
                local_file: "w.py".into(),
                module: import.module,
                target,
                names: vec![("foo".into(), "foo".into())],
                line: 1,
            }],
            &idx,
            "repo",
        );
        assert_eq!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "pkg/a.py", "helper"))
        );
        assert_ne!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "other/a.py", "helper")),
            "must not basename-guess other/a.py"
        );
    }

    #[test]
    // trace:v1 id=test.scc.resolve.rule3-unique-imported verifies=REQ-implement-phase-26-of-scc-x-ripwire-lessons-absorb-rule-3-import-incl exercises=impl.scc.resolve.rule3-include-file
    fn rule3_pins_unique_imported_when_globally_unique() {
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("a.py", &[mk_symbol("helper", SymbolKind::Function)]);
        let run = mk_symbol("run", SymbolKind::Function);
        idx.add_file("w.py", std::slice::from_ref(&run));
        let resolved = resolve_calls(
            "w.py",
            &[helper_call("run")],
            &[run],
            &[internal_import("a.py", &[])],
            &idx,
            "repo",
        );
        assert_eq!(
            resolved[0].callee_id,
            Some(scc_core::symbol_id("repo", "a.py", "helper")),
            "SCC has no unique-across-repo bare ladder; unique imported file still pins"
        );
    }

    #[test]
    // trace:v1 id=test.scc.resolve.rule3-fn-alias verifies=REQ-implement-phase-26-of-scc-x-ripwire-lessons-absorb-rule-3-import-incl exercises=impl.scc.resolve.rule3-include-file,impl.scc.resolve.fn-alias
    fn rule3_fn_alias_pins_unique_imported_helper() {
        use crate::model::{LanguageExtractor, SourceFile};
        use crate::python::PythonExtractor;
        let a = PythonExtractor::default()
            .extract(&SourceFile::new("a.py", "def helper():\n    return 1\n"));
        let b = PythonExtractor::default()
            .extract(&SourceFile::new("b.py", "def helper():\n    return 2\n"));
        let w = PythonExtractor::default().extract(&SourceFile::new(
            "w.py",
            "from a import foo\ndef run():\n    f = helper\n    f()\n",
        ));
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("a.py", &a.symbols);
        idx.add_file("b.py", &b.symbols);
        idx.add_file("w.py", &w.symbols);
        idx.set_fn_binds("w.py", &w.fn_binds);
        let imports: Vec<ResolvedImport> = w
            .imports
            .iter()
            .map(|imp| ResolvedImport {
                local_file: "w.py".into(),
                module: imp.module.clone(),
                names: imp.names.clone(),
                line: imp.line,
                target: idx.resolve_import("w.py", imp),
            })
            .collect();
        let resolved = resolve_calls("w.py", &w.calls, &w.symbols, &imports, &idx, "repo");
        let hit = resolved.iter().find(|c| c.callee_name == "f").expect("f()");
        assert_eq!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "a.py", "helper")),
            "alias of helper must Rule-3 pin the unique imported defining file"
        );
        assert_ne!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "b.py", "helper"))
        );
    }

    #[test]
    // trace:v1 id=test.scc.resolve.rule3-python-wildcard verifies=REQ-implement-phase-26-of-scc-x-ripwire-lessons-absorb-rule-3-import-incl exercises=impl.scc.resolve.rule3-include-file
    fn python_wildcard_import_pins_unique_imported_helper() {
        use crate::model::{LanguageExtractor, SourceFile};
        use crate::python::PythonExtractor;
        let a = PythonExtractor::default().extract(&SourceFile::new(
            "a.py",
            "def helper():\n    return 1\ndef foo():\n    return 0\n",
        ));
        let b = PythonExtractor::default()
            .extract(&SourceFile::new("b.py", "def helper():\n    return 2\n"));
        let w = PythonExtractor::default().extract(&SourceFile::new(
            "w.py",
            "from a import *\ndef run():\n    return helper()\n",
        ));
        assert!(
            w.imports
                .iter()
                .any(|i| i.module == "a" && i.names.is_empty()),
            "wildcard must not bind helper by name: {:?}",
            w.imports
        );
        let mut idx = SymbolIndex::new("repo");
        idx.add_file("a.py", &a.symbols);
        idx.add_file("b.py", &b.symbols);
        idx.add_file("w.py", &w.symbols);
        let imports: Vec<ResolvedImport> = w
            .imports
            .iter()
            .map(|imp| ResolvedImport {
                local_file: "w.py".into(),
                module: imp.module.clone(),
                names: imp.names.clone(),
                line: imp.line,
                target: idx.resolve_import("w.py", imp),
            })
            .collect();
        let resolved = resolve_calls("w.py", &w.calls, &w.symbols, &imports, &idx, "repo");
        let hit = resolved
            .iter()
            .find(|c| c.callee_name == "helper")
            .expect("helper()");
        assert_eq!(
            hit.callee_id,
            Some(scc_core::symbol_id("repo", "a.py", "helper"))
        );
        assert_eq!(hit.class, ResolutionClass::ResolvedInternal);
    }
}
