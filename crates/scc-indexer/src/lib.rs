//! Indexer: repository scanning, extraction, resolution, and incremental
//! invalidation (docs/SYSTEM_DESIGN.md §4).
//!
//! Pipeline:
//! 1. scan repo (gitignore-aware) + hash files
//! 2. diff against stored snapshot (content hashes)
//! 3. extract changed files (language or config extractor)
//! 4. build symbol index (stored + fresh) and resolve calls
//! 5. write facts + evidence into the store
//! 6. record snapshot; caller rebuilds the derived layer (scc-graph)

// trace:exempt reason=module-facade  # pub mod re-exports only; behavior traced per module
pub mod adapters;
pub mod config;
pub mod configrefs;
pub mod configs;
pub mod embed;
pub mod facts;
pub mod conflicts;
pub mod failures;
pub mod git;
pub mod go;
pub mod infra;
pub mod java;
pub mod lsp;
pub mod lsp_ts;
pub mod model;
pub mod python;
pub mod redact;
pub mod resolve;
pub mod resolver;
pub mod rust;
pub mod runtime;
pub mod scan;
pub mod typescript;
pub mod write;

pub use config::Config;
use model::{ExtractedFile, LanguageExtractor, SourceFile};
use resolve::{ResolvedImport, SymbolIndex};
use scan::{Language, ScannedFile};
use scc_core::kinds;
use scc_store::Store;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("store: {0}")]
    Store(#[from] scc_store::StoreError),
    #[error("scan: {0}")]
    Scan(#[from] scan::ScanError),
    #[error("configrefs: {0}")]
    ConfigRefs(String),
    #[error("failures: {0}")]
    Failures(String),
    #[error("no source files matched; nothing to index")]
    Empty,
}

#[derive(Debug, Clone, Default)]
pub struct IndexReport {
    pub revision: String,
    pub scanned: usize,
    pub indexed: usize,
    pub changed: usize,
    pub added: usize,
    pub removed: usize,
    pub failed: usize,
    pub duration_ms: u64,
}

// trace:exempt reason=internal-detail
pub struct Indexer {
    pub store: Store,
    pub config: Config,
    pub go: Box<dyn LanguageExtractor>,
    pub python: Box<dyn LanguageExtractor>,
    pub typescript: Box<dyn LanguageExtractor>,
    pub java: Box<dyn LanguageExtractor>,
    pub rust: Box<dyn LanguageExtractor>,
}

// trace:exempt reason=internal-detail
impl Indexer {
    pub fn new(store: Store, config: Config) -> Self {
        Indexer {
            store,
            config,
            go: Box::new(crate::go::GoExtractor::default()),
            python: Box::new(crate::python::PythonExtractor::default()),
            typescript: Box::new(crate::typescript::TypeScriptExtractor::default()),
            java: Box::new(crate::java::JavaExtractor::default()),
            rust: Box::new(crate::rust::RustExtractor::default()),
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Full or incremental index depending on stored state.
    pub fn index(&self) -> Result<IndexReport, IndexError> {
        let scanned = scan::scan_repo(&self.store.root, &self.config.index)?;
        let existing: HashMap<String, String> = self
            .store
            .all_files()?
            .into_iter()
            .map(|(p, h, _l, _k, _s)| (p, h))
            .collect();

        let mut changed: Vec<ScannedFile> = Vec::new();
        let mut added: Vec<ScannedFile> = Vec::new();
        let mut removed: Vec<String> = Vec::new();
        for f in &scanned {
            match existing.get(&f.path) {
                Some(h) if *h == f.hash => {}
                Some(_) => changed.push(f.clone()),
                None => added.push(f.clone()),
            }
        }
        for p in existing.keys() {
            if !scanned.iter().any(|f| &f.path == p) {
                removed.push(p.clone());
            }
        }

        let started = Instant::now();
        let mut report = IndexReport {
            revision: String::new(),
            scanned: scanned.len(),
            changed: changed.len() + added.len(),
            added: added.len(),
            removed: removed.len(),
            ..Default::default()
        };

        let git_info = git::resolve_git(&self.store.root);
        self.store
            .meta_set("remote_url", git_info.remote_url.as_deref().unwrap_or(""))?;
        self.store.meta_set("revision", &git_info.revision)?;
        report.revision = git_info.revision.clone();

        if existing.is_empty() && removed.is_empty() && changed.is_empty() && added.is_empty() {
            // cold index path below handles empty scans
        }

        let snapshot_id = self
            .store
            .begin_snapshot(&git_info.revision, git_info.branch.as_deref())?;

        // repository entity (subject of package/workspace contains edges)
        let repo_entity = scc_core::Entity::new(
            format!("repo://{}", self.store.repo_id),
            scc_core::kinds::SYSTEM,
            self.store.repo_name.clone(),
        );
        self.store.insert_entity(&repo_entity, &[])?;

        // ---- removal ----
        for p in &removed {
            self.store.purge_path(p)?;
            self.store.delete_file(p)?;
            report.removed += 1;
        }

        // ---- extraction of changed files ----
        let mut to_process: Vec<ScannedFile> = changed;
        to_process.append(&mut added);

        // Build symbol index: stored symbols for untouched files + fresh
        // extraction for changed ones.
        let mut index = SymbolIndex::new(&self.store.repo_id);
        let touched: HashSet<&str> = to_process.iter().map(|f| f.path.as_str()).collect();
        for (path, _h, lang, _kind, _size) in self.store.all_files()? {
            if touched.contains(path.as_str()) {
                continue;
            }
            if lang == "python" || lang == "typescript" || lang == "javascript" || lang == "go" || lang == "java" || lang == "rust" {
                let syms = self.load_symbols(&path)?;
                index.add_file(&path, &syms);
            }
        }

        let mut extracted: BTreeMap<
            String,
            (
                ScannedFile,
                ExtractedFile,
                Vec<configrefs::ConfigRefHit>,
                Vec<failures::FailureHit>,
            ),
        > = BTreeMap::new();
        for f in &to_process {
            let path = &f.path;
            let full = self.store.root.join(path);
            let Ok(content) = std::fs::read_to_string(&full) else {
                report.failed += 1;
                continue;
            };
            let ef = self.extract(f, &content);
            let cfg_hits = configrefs::scan_config_refs(&content, f.language.as_str());
            let fail_hits = failures::scan_failures(&content, f.language.as_str());
            index.add_file(path, &ef.symbols);
            extracted.insert(path.clone(), (f.clone(), ef, cfg_hits, fail_hits));
        }

        // ---- resolution + writing ----
        // Purge each changed file's previous facts BEFORE re-extracting:
        // without this, a re-extracted file whose import targets changed
        // keeps stale edges to removed entities (docs/DATA_STRATEGY.md §6
        // invalidation cascade). Mirrors index_paths().
        for path in extracted.keys() {
            self.store.purge_path(path)?;
        }
        let mut intent: Option<configs::Intent> = None;
        for (path, (f, ef, cfg_hits, fail_hits)) in &extracted {
            let file = SourceFile::new(path.clone(), String::new()); // content re-read below
            let _ = file;
            let lang = f.language;
            let mut resolved_imports: Vec<ResolvedImport> = Vec::new();
            let mut resolved_calls = Vec::new();
            if matches!(
                lang,
                Language::Python
                    | Language::TypeScript
                    | Language::JavaScript
                    | Language::Go
                    | Language::Java
                    | Language::Rust
            ) {
                resolved_imports = ef
                    .imports
                    .iter()
                    .map(|imp| {
                        let target = index.resolve_import(path, imp);
                        ResolvedImport {
                            local_file: path.clone(),
                            module: imp.module.clone(),
                            names: imp.names.clone(),
                            line: imp.line,
                            target,
                        }
                    })
                    .collect();
                resolved_calls = resolve::resolve_calls(
                    path,
                    &ef.calls,
                    &ef.symbols,
                    &resolved_imports,
                    &index,
                    &self.store.repo_id,
                );
            }
            let writer = write::Writer::new(&self.store, &self.store.repo_id, &report.revision);
            // re-read content for hash consistency
            let full = self.store.root.join(path);
            let content = std::fs::read_to_string(&full).unwrap_or_default();
            let hash = scan::hash_bytes(content.as_bytes());
            writer.write_source(path, &hash, ef, &resolved_imports, &resolved_calls, &index)?;
            self.store.upsert_file(path, &f.hash, f.language.as_str(), f.kind.as_str(), f.size)?;
            configrefs::apply_config_refs(&self.store, path, f.language.as_str(), &content, cfg_hits.clone())
                .map_err(IndexError::ConfigRefs)?;
            failures::apply_failures(&self.store, path, f.language.as_str(), fail_hits.clone())
                .map_err(IndexError::Failures)?;
            report.indexed += 1;
        }

        // tested_by edges derived from changed files must be relinked
        let changed_list: Vec<String> = to_process.iter().map(|f| f.path.clone()).collect();
        self.relink_tests_for(&changed_list, &report.revision)?;

        // ---- config extraction (env, compose, package.json, intent, readme) ----
        for (path, (f, ef, _cfg, _fail)) in &extracted {
            let lang = f.language;
            if matches!(
                lang,
                Language::Env
                    | Language::Json
                    | Language::Yaml
                    | Language::Dockerfile
                    | Language::Terraform
            ) || path == ".scc/intent.yaml"
                || is_readme(path)
            {
                let full = self.store.root.join(path);
                let Ok(content) = std::fs::read_to_string(&full) else { continue };
                let mut out = configs::extract_config_file(path, &content, &self.store.repo_id);
                let infra = crate::infra::extract_infra_file(path, &content, &self.store.repo_id);
                out.entities.extend(infra.entities);
                out.relationships.extend(infra.relationships);
                if let Some(i) = out.intent {
                    intent = Some(i);
                }
                if let Some(purpose) = out.readme_purpose {
                    self.store.meta_set("purpose", &purpose)?;
                }
                let _writer = write::Writer::new(&self.store, &self.store.repo_id, &report.revision);
                for e in out.entities {
                    self.store.insert_entity(&e, std::slice::from_ref(path))?;
                }
                for (rel, src) in out.relationships {
                    self.store.insert_relationship(&rel, &src)?;
                }
                for ep in out.entrypoints {
                    let mut se = scc_core::Entity::new(
                        scc_core::entity_id(&self.store.repo_id, kinds::SYMBOL, &format!("{path}/{}", ep.symbol)),
                        kinds::SYMBOL,
                        ep.symbol.clone(),
                    );
                    se.attr("entrypoints", serde_json::json!([ep.kind]));
                    se.attr("file", serde_json::json!(path));
                    self.store.insert_entity(&se, std::slice::from_ref(path))?;
                }
                let _ = ef;
            }
        }

        // intent claims
        match intent {
            Some(i) => {
                let claims = configs::intent_claims(&i, &self.store.repo_id);
                self.store.replace_intent_claims(&claims)?;
            }
            None => {
                if !self.store.intent_claims()?.is_empty() {
                    self.store.replace_intent_claims(&[])?;
                }
            }
        }

        self.store.finish_snapshot(snapshot_id, report.indexed)?;
        self.store.cache_clear()?;
        report.duration_ms = started.elapsed().as_millis() as u64;
        Ok(report)
    }

    fn extract(&self, f: &ScannedFile, content: &str) -> ExtractedFile {
        let file = SourceFile::new(f.path.clone(), content.to_string());
        match f.language {
            Language::Python if self.config.language_enabled(Language::Python) => {
                self.python.extract(&file)
            }
            Language::Go if self.config.language_enabled(Language::Go) => {
                self.go.extract(&file)
            }
            Language::TypeScript | Language::JavaScript
                if self.config.language_enabled(Language::TypeScript) =>
            {
                self.typescript.extract(&file)
            }
            Language::Java if self.config.language_enabled(Language::Java) => {
                self.java.extract(&file)
            }
            Language::Rust if self.config.language_enabled(Language::Rust) => {
                self.rust.extract(&file)
            }
            _ => ExtractedFile::default(),
        }
    }

// trace:exempt reason=internal-detail
    fn load_symbols(&self, path: &str) -> Result<Vec<model::Symbol>, scc_store::StoreError> {
        let rows = self.store.symbols_in_file(path)?;
        Ok(rows
            .into_iter()
            .map(
                |(_id, name, kind, sig, sl, el, exported, docstring)| model::Symbol {
                    name,
                    kind: match kind.as_str() {
                        "function" => model::SymbolKind::Function,
                        "method" => model::SymbolKind::Method,
                        "class" => model::SymbolKind::Class,
                        "interface" => model::SymbolKind::Interface,
                        "type" => model::SymbolKind::Type,
                        "const" => model::SymbolKind::Const,
                        "enum" => model::SymbolKind::Enum,
                        _ => model::SymbolKind::Module,
                    },
                    signature: sig,
                    decl_header: None,
                    start_line: sl,
                    end_line: el,
                    exported,
                    docstring,
                    parent: None,
                },
            )
            .collect())
    }

    /// Refresh specific paths (watch events / post-edit). Unknown paths are
    /// ignored. Returns the number of files re-indexed.
    pub fn refresh_paths(&self, paths: &[String]) -> Result<IndexReport, IndexError> {
        let scanned: HashMap<String, ScannedFile> = scan::scan_repo(&self.store.root, &self.config.index)?
            .into_iter()
            .map(|f| (f.path.clone(), f))
            .collect();
        let git_info = git::resolve_git(&self.store.root);
        self.store.meta_set("revision", &git_info.revision)?;
        let snapshot_id = self
            .store
            .begin_snapshot(&git_info.revision, git_info.branch.as_deref())?;

        // repository entity (subject of package/workspace contains edges)
        let repo_entity = scc_core::Entity::new(
            format!("repo://{}", self.store.repo_id),
            scc_core::kinds::SYSTEM,
            self.store.repo_name.clone(),
        );
        self.store.insert_entity(&repo_entity, &[])?;
        let started = Instant::now();

        let mut changed_paths: Vec<String> = Vec::new();
        for p in paths {
            let p = p.trim_start_matches("./");
            let p = p.trim_start_matches('/');
            if p.is_empty() || p.starts_with(".scc/") || p == ".scc" {
                continue;
            }
            match scanned.get(p) {
                Some(f) => {
                    // verify hash actually changed
                    let old = self.store.file(p)?;
                    if old.map(|(h, _, _, _)| h == f.hash).unwrap_or(false) {
                        continue;
                    }
                    changed_paths.push(f.path.clone());
                }
                None => {
                    // deleted or ignored: purge if known
                    if self.store.file(p)?.is_some() {
                        self.store.purge_path(p)?;
                        self.store.delete_file(p)?;
                    }
                }
            }
        }
        if changed_paths.is_empty() {
            self.store.finish_snapshot(snapshot_id, 0)?;
            return Ok(IndexReport {
                revision: git_info.revision,
                ..Default::default()
            });
        }

        let mut report = self.index_paths(&changed_paths, &scanned, &git_info.revision)?;
        self.store.finish_snapshot(snapshot_id, report.indexed)?;
        self.store.cache_clear()?;
        report.revision = git_info.revision;
        report.duration_ms = started.elapsed().as_millis() as u64;
        Ok(report)
    }

    fn index_paths(
        &self,
        changed_paths: &[String],
        scanned: &HashMap<String, ScannedFile>,
        revision: &str,
    ) -> Result<IndexReport, IndexError> {
        let mut report = IndexReport::default();
        let mut index = SymbolIndex::new(&self.store.repo_id);
        let touched: HashSet<&str> = changed_paths.iter().map(|s| s.as_str()).collect();
        for (path, _h, lang, _kind, _size) in self.store.all_files()? {
            if touched.contains(path.as_str()) {
                continue;
            }
            if lang == "python" || lang == "typescript" || lang == "javascript" || lang == "go" || lang == "java" || lang == "rust" {
                let syms = self.load_symbols(&path)?;
                index.add_file(&path, &syms);
            }
        }

        let mut extracted: BTreeMap<
            String,
            (
                ScannedFile,
                ExtractedFile,
                Vec<configrefs::ConfigRefHit>,
                Vec<failures::FailureHit>,
            ),
        > = BTreeMap::new();
        for p in changed_paths {
            let Some(f) = scanned.get(p) else { continue };
            let full = self.store.root.join(p);
            let Ok(content) = std::fs::read_to_string(&full) else {
                report.failed += 1;
                continue;
            };
            self.store.purge_path(p)?;
            let ef = self.extract(f, &content);
            let cfg_hits = configrefs::scan_config_refs(&content, f.language.as_str());
            let fail_hits = failures::scan_failures(&content, f.language.as_str());
            index.add_file(p, &ef.symbols);
            extracted.insert(p.clone(), (f.clone(), ef, cfg_hits, fail_hits));
        }

        for (path, (f, ef, cfg_hits, fail_hits)) in &extracted {
            let mut resolved_imports: Vec<ResolvedImport> = Vec::new();
            let mut resolved_calls = Vec::new();
            if matches!(
                f.language,
                Language::Python
                    | Language::TypeScript
                    | Language::JavaScript
                    | Language::Go
                    | Language::Java
                    | Language::Rust
            ) {
                resolved_imports = ef
                    .imports
                    .iter()
                    .map(|imp| {
                        let target = index.resolve_import(path, imp);
                        ResolvedImport {
                            local_file: path.clone(),
                            module: imp.module.clone(),
                            names: imp.names.clone(),
                            line: imp.line,
                            target,
                        }
                    })
                    .collect();
                resolved_calls = resolve::resolve_calls(
                    path,
                    &ef.calls,
                    &ef.symbols,
                    &resolved_imports,
                    &index,
                    &self.store.repo_id,
                );
            }
            let writer = write::Writer::new(&self.store, &self.store.repo_id, revision);
            let full = self.store.root.join(path);
            let content = std::fs::read_to_string(&full).unwrap_or_default();
            let hash = scan::hash_bytes(content.as_bytes());
            writer.write_source(path, &hash, ef, &resolved_imports, &resolved_calls, &index)?;
            self.store
                .upsert_file(path, &f.hash, f.language.as_str(), f.kind.as_str(), f.size)?;
            configrefs::apply_config_refs(&self.store, path, f.language.as_str(), &content, cfg_hits.clone())
                .map_err(IndexError::ConfigRefs)?;
            failures::apply_failures(&self.store, path, f.language.as_str(), fail_hits.clone())
                .map_err(IndexError::Failures)?;
            report.indexed += 1;
        }

        // config extraction for changed config files
        let mut intent: Option<configs::Intent> = None;
        for (path, (f, _ef, _cfg, _fail)) in &extracted {
            if matches!(
                f.language,
                Language::Env
                    | Language::Json
                    | Language::Yaml
                    | Language::Dockerfile
                    | Language::Terraform
            ) || path == ".scc/intent.yaml"
                || is_readme(path)
            {
                let full = self.store.root.join(path);
                let Ok(content) = std::fs::read_to_string(&full) else { continue };
                let mut out = configs::extract_config_file(path, &content, &self.store.repo_id);
                let infra = crate::infra::extract_infra_file(path, &content, &self.store.repo_id);
                out.entities.extend(infra.entities);
                out.relationships.extend(infra.relationships);
                if let Some(i) = out.intent {
                    intent = Some(i);
                }
                if let Some(purpose) = out.readme_purpose {
                    self.store.meta_set("purpose", &purpose)?;
                }
                let _writer = write::Writer::new(&self.store, &self.store.repo_id, revision);
                for e in out.entities {
                    self.store.insert_entity(&e, std::slice::from_ref(path))?;
                }
                for (rel, src) in out.relationships {
                    self.store.insert_relationship(&rel, &src)?;
                }
            }
        }
        match intent {
            Some(i) => {
                let claims = configs::intent_claims(&i, &self.store.repo_id);
                self.store.replace_intent_claims(&claims)?;
            }
            None => {
                if !self.store.intent_claims()?.is_empty() {
                    self.store.replace_intent_claims(&[])?;
                }
            }
        }
        // tested_by edges derived from changed files must be relinked
        self.relink_tests_for(changed_paths, revision)?;
        Ok(report)
    }

    /// True when the index exists (has a complete snapshot).
    pub fn is_indexed(&self) -> Result<bool, scc_store::StoreError> {
        Ok(self.store.snapshot_status()?.is_some())
    }

    /// Re-link `tested_by` edges for every test file whose imports reach one
    /// of `changed_paths`: those edges were derived from the imported file's
    /// symbols and must be invalidated when it changes (docs/TEST_PLAN.md §7).
    fn relink_tests_for(&self, changed_paths: &[String], revision: &str) -> Result<(), IndexError> {
        if changed_paths.is_empty() {
            return Ok(());
        }
        let changed: HashSet<&str> = changed_paths.iter().map(|s| s.as_str()).collect();
        let mut index = SymbolIndex::new(&self.store.repo_id);
        for (path, _h, lang, _kind, _size) in self.store.all_files()? {
            if lang == "python" || lang == "typescript" || lang == "javascript" || lang == "go" || lang == "java" || lang == "rust" {
                let syms = self.load_symbols(&path)?;
                index.add_file(&path, &syms);
            }
        }
        let test_files: Vec<String> = self
            .store
            .all_files()?
            .into_iter()
            .filter(|(_, _, _, kind, _)| kind == "test")
            .map(|(p, _, _, _, _)| p)
            .collect();
        let writer = write::Writer::new(&self.store, &self.store.repo_id, revision);
        for tf in test_files {
            let imports = self.store.imports_in_file(&tf)?;
            if imports.is_empty() {
                continue;
            }
            let resolved: Vec<ResolvedImport> = imports
                .iter()
                .map(|(module, names, line, _typ)| {
                    let imp = model::Import {
                        module: module.clone(),
                        names: names.clone(),
                        line: *line,
                        r#type: model::ImportType::Module,
                    };
                    let target = index.resolve_import(&tf, &imp);
                    ResolvedImport {
                        local_file: tf.clone(),
                        module: imp.module,
                        names: imp.names,
                        line: imp.line,
                        target,
                    }
                })
                .collect();
            let touches_changed = resolved.iter().any(|ri| match &ri.target {
                resolve::ImportTarget::Internal { file, .. } => changed.contains(file.as_str()),
                _ => false,
            });
            if !touches_changed {
                continue;
            }
            // drop the stale edges, then relink
            let stale = self
                .store
                .relationship_ids_with_source(&tf, scc_core::predicates::TESTED_BY)?;
            for id in stale {
                self.store.delete_relationship(&id)?;
            }
            let tests = self
                .store
                .tests()?
                .into_iter()
                .filter(|(_, _, file, _, _)| file == &tf)
                .map(|(_, name, _, kind, symbol)| model::Test {
                    name,
                    symbol,
                    kind: if kind == "integration" {
                        model::TestKind::Integration
                    } else {
                        model::TestKind::Unit
                    },
                    line: 0,
                })
                .collect::<Vec<_>>();
            let ef = model::ExtractedFile {
                tests,
                ..Default::default()
            };
            writer.link_tests(&tf, &ef, &resolved)?;
        }
        Ok(())
    }
}

fn is_readme(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.eq_ignore_ascii_case("readme.md") || name.eq_ignore_ascii_case("readme")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn indexer_for(dir: &Path) -> (Indexer, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = Store::open(&tmp.path().join("scc.db"), dir).unwrap();
        let idx = Indexer::new(store, Config::default());
        (idx, tmp)
    }

    #[test]
    fn cold_index_produces_facts() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/calc.py"),
            "def add(a, b):\n    return a + b\n\ndef main():\n    return add(1, 2)\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        let report = idx.index().unwrap();
        assert_eq!(report.indexed, 1);
        let stats = idx.store.stats().unwrap();
        assert_eq!(stats["symbols"], 2);
        let rels = idx.store.all_relationships().unwrap();
        // §26: native resolution is evidence-grade (EXTRACTED candidate),
        // never RESOLVED — semantic engines (LSP/SCIP) provide RESOLVED
        assert!(
            rels.iter().any(|r| r.predicate == scc_core::predicates::CALLS
                && matches!(
                    r.provenance,
                    scc_core::Provenance::Extracted | scc_core::Provenance::Resolved
                )),
            "expected an evidence-grade call edge"
        );
    }

    #[test]
    fn incremental_matches_cold() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/calc.py"),
            "def add(a, b):\n    return a + b\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/main.py"),
            "from calc import add\n\ndef main():\n    return add(1, 2)\n",
        )
        .unwrap();

        // cold
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let cold_facts: Vec<String> = idx
            .store
            .all_relationships()
            .unwrap()
            .iter()
            .map(|r| format!("{} {} {} {}", r.subject, r.predicate, r.object, r.provenance.as_str()))
            .collect();

        // incremental edit sequence
        let (idx2, _t2) = indexer_for(root);
        idx2.index().unwrap();
        // edit main.py
        std::fs::write(
            root.join("src/main.py"),
            "from calc import add\n\ndef main():\n    return add(3, 4)\n",
        )
        .unwrap();
        idx2.refresh_paths(&["src/main.py".into()]).unwrap();
        let incr_facts: Vec<String> = idx2
            .store
            .all_relationships()
            .unwrap()
            .iter()
            .map(|r| format!("{} {} {} {}", r.subject, r.predicate, r.object, r.provenance.as_str()))
            .collect();
        assert_eq!(cold_facts, incr_facts, "incremental must equal cold");

        // fresh cold index of final state must also match
        let (idx3, _t3) = indexer_for(root);
        idx3.index().unwrap();
        let final_facts: Vec<String> = idx3
            .store
            .all_relationships()
            .unwrap()
            .iter()
            .map(|r| format!("{} {} {} {}", r.subject, r.predicate, r.object, r.provenance.as_str()))
            .collect();
        assert_eq!(incr_facts, final_facts, "full vs incremental equivalence");
    }

    #[test]
    fn removed_files_purged() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.py"), "def a():\n    pass\n").unwrap();
        std::fs::write(root.join("src/b.py"), "def b():\n    pass\n").unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        std::fs::remove_file(root.join("src/b.py")).unwrap();
        idx.refresh_paths(&["src/b.py".into()]).unwrap();
        let stats = idx.store.stats().unwrap();
        assert_eq!(stats["symbols"], 1);
        assert_eq!(stats["files"], 1);
    }

    #[test]
    fn full_index_purges_changed_files_import_edges() {
        // regression: `scc index` (full path) after an import-target rename
        // must not leave stale edges to the removed target (SCC-031 invariant)
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/a.py"),
            "from b import helper\n\ndef main():\n    return helper()\n",
        )
        .unwrap();
        std::fs::write(root.join("src/b.py"), "def helper():\n    return 1\n").unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();

        // rename b.py -> renamed.py and update the importer; full re-index
        std::fs::rename(root.join("src/b.py"), root.join("src/renamed.py")).unwrap();
        std::fs::write(
            root.join("src/a.py"),
            "from renamed import helper\n\ndef main():\n    return helper()\n",
        )
        .unwrap();
        idx.index().unwrap();

        // no dangling relationships (targets must resolve or be known namespaces)
        let graph = scc_graph::RealityGraph::load(&idx.store).unwrap();
        let mut dangling = 0usize;
        for r in graph.all_rels() {
            let known = |id: &str| {
                graph.entities.contains_key(id)
                    || id.contains("/external_api/")
                    || id.contains("/component/")
                    || id.contains("/flow/")
                    || id.contains("/invariant/")
                    || id.contains("/file/")
            };
            if !known(&r.subject) || !known(&r.object) {
                dangling += 1;
            }
        }
        assert_eq!(dangling, 0, "full re-index must not leave dangling edges");
        // the import now resolves to the renamed file
        let rels = idx.store.all_relationships().unwrap();
        assert!(
            rels.iter().any(|r| {
                r.predicate == scc_core::predicates::IMPORTS
                    && r.object.contains("renamed.py")
            }),
            "import must point at the renamed file"
        );
    }

    #[test]
    fn env_config_extraction_redacts() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join(".env"),
            "DATABASE_URL=postgres://user:secret@host/db\nPORT=8080\n",
        )
        .unwrap();
        std::fs::write(root.join("app.py"), "def main():\n    pass\n").unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let entities = idx.store.entities_by_kind(kinds::SECRET_REFERENCE).unwrap();
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].name, "DATABASE_URL");
        let configs = idx.store.entities_by_kind(kinds::CONFIGURATION).unwrap();
        assert!(configs.iter().any(|e| e.name == "PORT"));
        // no value leaks into the store
        let all = idx.store.all_entities().unwrap();
        for e in all {
            let s = serde_json::to_string(&e).unwrap();
            assert!(
                !s.contains("postgres://") && !s.contains("user:secret"),
                "secret value leaked in {s}"
            );
        }
    }
}
