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
pub mod bridges;
pub mod config;
pub mod configrefs;
pub mod configs;
pub mod conflicts;
pub mod embed;
pub mod facts;
pub mod failures;
pub mod git;
pub mod go;
pub mod infra;
pub mod java;
pub mod lsp;
pub mod lsp_ts;
pub mod mentions;
pub mod model;
pub mod python;
pub mod recv;
pub mod redact;
pub mod resolve;
pub mod resolver;
pub mod runtime;
pub mod rust;
pub mod scan;
pub mod typescript;
pub mod write;

pub use config::Config;
use model::{ExtractedFile, LanguageExtractor, SourceFile};
use resolve::{ResolvedImport, SymbolIndex};
use scan::{Language, ScannedFile};
use scc_core::kinds;
use scc_store::Store;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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
    pub analysis_quality: scc_core::AnalysisQuality,
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

        // Hash-unchanged importers/callers still hold type-narrowed CALLS
        // into changed files. Collect them before purge so incremental ≡ cold.
        let scanned_by_path: HashMap<String, ScannedFile> = scanned
            .iter()
            .map(|f| (f.path.clone(), f.clone()))
            .collect();
        let mut seeds: Vec<String> = Vec::new();
        seeds.extend(changed.iter().map(|f| f.path.clone()));
        seeds.extend(added.iter().map(|f| f.path.clone()));
        seeds.extend(removed.iter().cloned());
        let cascade = self.with_dependents(&seeds)?;

        // ---- removal ----
        for p in &removed {
            self.store.purge_path(p)?;
            self.store.delete_file(p)?;
            report.removed += 1;
        }

        // changed + added keep scan order (shared-entity last-write is
        // order-sensitive). Hash-unchanged importers/callers append after.
        let removed_set: HashSet<&str> = removed.iter().map(|s| s.as_str()).collect();
        let mut to_process: Vec<ScannedFile> = changed;
        to_process.append(&mut added);
        let mut seen: HashSet<String> = to_process.iter().map(|f| f.path.clone()).collect();
        for p in &cascade {
            if seen.contains(p) || removed_set.contains(p.as_str()) {
                continue;
            }
            if let Some(f) = scanned_by_path.get(p) {
                seen.insert(p.clone());
                to_process.push(f.clone());
            }
        }

        // Build symbol index: stored symbols for untouched files + fresh
        // extraction for changed ones.
        let mut index = SymbolIndex::new(&self.store.repo_id);
        let touched: HashSet<&str> = to_process.iter().map(|f| f.path.as_str()).collect();
        for (path, _h, lang, _kind, _size) in self.store.all_files()? {
            if touched.contains(path.as_str()) {
                continue;
            }
            if lang == "python"
                || lang == "typescript"
                || lang == "javascript"
                || lang == "go"
                || lang == "java"
                || lang == "rust"
            {
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
            index.set_type_binds(path, &ef.type_binds);
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
            record_file_quality(&self.store, path, &resolved_calls)?;
            self.store
                .upsert_file(path, &f.hash, f.language.as_str(), f.kind.as_str(), f.size)?;
            configrefs::apply_config_refs(
                &self.store,
                path,
                f.language.as_str(),
                &content,
                cfg_hits.clone(),
            )
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
            ) || lang.is_config_extract()
                || path == ".scc/intent.yaml"
                || is_readme(path)
            {
                let full = self.store.root.join(path);
                let Ok(content) = std::fs::read_to_string(&full) else {
                    continue;
                };
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
                let _writer =
                    write::Writer::new(&self.store, &self.store.repo_id, &report.revision);
                for e in out.entities {
                    self.store.insert_entity(&e, std::slice::from_ref(path))?;
                }
                for (rel, src) in out.relationships {
                    self.store.insert_relationship(&rel, &src)?;
                }
                for ep in out.entrypoints {
                    let mut se = scc_core::Entity::new(
                        scc_core::entity_id(
                            &self.store.repo_id,
                            kinds::SYMBOL,
                            &format!("{path}/{}", ep.symbol),
                        ),
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

        apply_doc_mentions(&self.store)?;
        bridges::link_rpc_bridges(&self.store)?;

        self.store.finish_snapshot(snapshot_id, report.indexed)?;
        self.store.cache_clear()?;
        report.analysis_quality = persist_analysis_quality(&self.store)?;
        persist_bm25_corpus(&self.store)?;
        report.duration_ms = started.elapsed().as_millis() as u64;
        Ok(report)
    }

    fn extract(&self, f: &ScannedFile, content: &str) -> ExtractedFile {
        let file = SourceFile::new(f.path.clone(), content.to_string());
        match f.language {
            Language::Python if self.config.language_enabled(Language::Python) => {
                self.python.extract(&file)
            }
            Language::Go if self.config.language_enabled(Language::Go) => self.go.extract(&file),
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
        let scanned: HashMap<String, ScannedFile> =
            scan::scan_repo(&self.store.root, &self.config.index)?
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
        let mut deleted_paths: Vec<String> = Vec::new();
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
                    // deleted or ignored: collect before purge so importers
                    // of the removed file can be re-resolved.
                    if self.store.file(p)?.is_some() {
                        deleted_paths.push(p.to_string());
                    }
                }
            }
        }
        let deleted_cascade = self.with_dependents(&deleted_paths)?;
        for p in &deleted_paths {
            self.store.purge_path(p)?;
            self.store.delete_file(p)?;
            drop_file_quality(&self.store, p)?;
        }
        for d in deleted_cascade {
            if scanned.contains_key(&d) && !changed_paths.contains(&d) {
                changed_paths.push(d);
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
        let mut paths: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for p in changed_paths {
            if scanned.contains_key(p) && seen.insert(p.clone()) {
                paths.push(p.clone());
            }
        }
        for d in self.with_dependents(changed_paths)? {
            if scanned.contains_key(&d) && seen.insert(d.clone()) {
                paths.push(d);
            }
        }
        let mut index = SymbolIndex::new(&self.store.repo_id);
        let touched: HashSet<&str> = paths.iter().map(|s| s.as_str()).collect();
        for (path, _h, lang, _kind, _size) in self.store.all_files()? {
            if touched.contains(path.as_str()) {
                continue;
            }
            if lang == "python"
                || lang == "typescript"
                || lang == "javascript"
                || lang == "go"
                || lang == "java"
                || lang == "rust"
            {
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
        for p in &paths {
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
            index.set_type_binds(p, &ef.type_binds);
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
            record_file_quality(&self.store, path, &resolved_calls)?;
            self.store
                .upsert_file(path, &f.hash, f.language.as_str(), f.kind.as_str(), f.size)?;
            configrefs::apply_config_refs(
                &self.store,
                path,
                f.language.as_str(),
                &content,
                cfg_hits.clone(),
            )
            .map_err(IndexError::ConfigRefs)?;
            failures::apply_failures(&self.store, path, f.language.as_str(), fail_hits.clone())
                .map_err(IndexError::Failures)?;
            report.indexed += 1;
        }

        // config extraction for changed config files
        let mut intent: Option<configs::Intent> = None;
        for (path, (f, _ef, _cfg, _fail)) in &extracted {
            if f.language.is_config_extract() || path == ".scc/intent.yaml" || is_readme(path) {
                let full = self.store.root.join(path);
                let Ok(content) = std::fs::read_to_string(&full) else {
                    continue;
                };
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
        self.relink_tests_for(&paths, revision)?;
        apply_doc_mentions(&self.store)?;
        bridges::link_rpc_bridges(&self.store)?;
        report.analysis_quality = persist_analysis_quality(&self.store)?;
        persist_bm25_corpus(&self.store)?;
        Ok(report)
    }

    /// True when the index exists (has a complete snapshot).
    pub fn is_indexed(&self) -> Result<bool, scc_store::StoreError> {
        Ok(self.store.snapshot_status()?.is_some())
    }

    /// `seeds` plus files that IMPORT or CALL into them. Must run before
    /// `purge_path` so incoming edges still exist. Type-narrowed CALLS live
    /// on the caller; refreshing only the callee would leave them stale.
    // trace:v1 id=impl.scc.index.invalidation-cascade work=WORK-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing-from-unique satisfies=REQ-implement-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing
    fn with_dependents(&self, seeds: &[String]) -> Result<Vec<String>, IndexError> {
        let mut out: BTreeSet<String> = seeds.iter().cloned().collect();
        for p in seeds {
            for d in self.store.paths_depending_on(p)? {
                out.insert(d);
            }
        }
        Ok(out.into_iter().collect())
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
            if lang == "python"
                || lang == "typescript"
                || lang == "javascript"
                || lang == "go"
                || lang == "java"
                || lang == "rust"
            {
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

/// Per-file gauges live in store meta, not FILE entity attributes, so
/// System IR export stays incremental≡cold. The map is patched for
/// changed paths and folded into `analysis_quality`.
const META_QUALITY: &str = "analysis_quality";
const META_QUALITY_FILES: &str = "analysis_quality_files";

fn load_quality_files(store: &Store) -> BTreeMap<String, scc_core::AnalysisQuality> {
    store
        .meta_get(META_QUALITY_FILES)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_quality_files(
    store: &Store,
    map: &BTreeMap<String, scc_core::AnalysisQuality>,
) -> Result<(), IndexError> {
    let json = serde_json::to_string(map).unwrap_or_else(|_| "{}".to_string());
    store.meta_set(META_QUALITY_FILES, &json)?;
    Ok(())
}

fn record_file_quality(
    store: &Store,
    path: &str,
    calls: &[resolve::ResolvedCall],
) -> Result<(), IndexError> {
    let mut q = resolve::quality_from_calls(calls);
    q.files.parsed = 1;
    let mut map = load_quality_files(store);
    map.insert(path.to_string(), q);
    save_quality_files(store, &map)
}

fn drop_file_quality(store: &Store, path: &str) -> Result<(), IndexError> {
    let mut map = load_quality_files(store);
    if map.remove(path).is_some() {
        save_quality_files(store, &map)?;
    }
    Ok(())
}

fn apply_doc_mentions(store: &Store) -> Result<(), IndexError> {
    let stats = mentions::write_mentions(store)?;
    let json = serde_json::to_string(&stats).unwrap_or_else(|_| "{}".to_string());
    store.meta_set("doc_mentions", &json)?;
    Ok(())
}

/// Fold per-file gauges into one repo-wide snapshot.
// trace:v1 id=impl.scc.index.persist-analysis-quality work=WORK-ripwire-lessons-phase1 satisfies=REQ-resolution-honesty-gauges
fn persist_analysis_quality(store: &Store) -> Result<scc_core::AnalysisQuality, IndexError> {
    let map = load_quality_files(store);
    let mut q = scc_core::AnalysisQuality::default();
    for part in map.values() {
        q.merge(part);
    }
    if let Ok(Some(raw)) = store.meta_get("doc_mentions") {
        if let Ok(stats) = serde_json::from_str::<mentions::MentionStats>(&raw) {
            q.matched_doc_mentions = stats.matched;
            q.unmatched_doc_mentions = stats.unmatched;
        }
    }
    let json = serde_json::to_string(&q).unwrap_or_else(|_| "{}".to_string());
    store.meta_set(META_QUALITY, &json)?;
    Ok(q)
}

const BM25_META: &str = "bm25_corpus";

/// Persist corpus-wide BM25 stats in store meta (never FILE attributes).
// trace:v1 id=impl.scc.index.persist-bm25 work=WORK-ripwire-lessons-phase2 satisfies=REQ-bm25-persist
fn persist_bm25_corpus(store: &Store) -> Result<(), IndexError> {
    const KINDS: &[&str] = &[
        kinds::SYMBOL,
        kinds::COMPONENT,
        kinds::ROUTE,
        kinds::CONTRACT,
        kinds::STATE,
        kinds::FILE,
        kinds::FLOW,
        kinds::SCHEMA,
    ];
    let entities = store.all_entities()?;
    let docs: Vec<scc_core::LexDoc> = entities
        .iter()
        .filter(|e| KINDS.contains(&e.kind.as_str()))
        .map(|e| {
            let mut path = e
                .attributes
                .get("file")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if path.is_empty() {
                path = e
                    .attributes
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
            }
            if path.is_empty() && e.kind == kinds::FILE {
                path = e.name.as_str();
            }
            let doc = e
                .attributes
                .get("docstring")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let mut body = e
                .attributes
                .get("signature")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if body.is_empty() {
                body = e
                    .attributes
                    .get("responsibility")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
            }
            scc_core::LexDoc::from_parts(&e.id, &e.name, path, doc, body)
        })
        .collect();
    let stats = scc_core::Bm25CorpusStats::from_docs(&docs);
    let json = serde_json::to_string(&stats).unwrap_or_else(|_| "{}".to_string());
    store.meta_set(BM25_META, &json)?;
    Ok(())
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
            rels.iter()
                .any(|r| r.predicate == scc_core::predicates::CALLS
                    && matches!(
                        r.provenance,
                        scc_core::Provenance::Extracted | scc_core::Provenance::Resolved
                    )),
            "expected an evidence-grade call edge"
        );
        assert!(
            report.analysis_quality.calls.resolved >= 1,
            "honesty gauges must count the resolved add() call: {:?}",
            report.analysis_quality
        );
        assert_eq!(
            report.analysis_quality.calls.external, 0,
            "bare local calls are not external"
        );
        let stored = idx
            .store
            .meta_get("analysis_quality")
            .unwrap()
            .expect("persisted");
        let parsed: scc_core::AnalysisQuality = serde_json::from_str(&stored).unwrap();
        assert_eq!(
            parsed.calls.resolved,
            report.analysis_quality.calls.resolved
        );
        assert_eq!(parsed.files.parsed, report.analysis_quality.files.parsed);
    }

    #[test]
    // trace:v1 id=test.scc.index.analysis-quality verifies=REQ-resolution-honesty-gauges exercises=impl.scc.index.persist-analysis-quality
    fn analysis_quality_persists_and_does_not_label_local_calls_external() {
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
        assert!(report.analysis_quality.calls.resolved >= 1);
        assert_eq!(report.analysis_quality.calls.external, 0);
        assert!(report.analysis_quality.files.parsed >= 1);
        let stored = idx.store.meta_get("analysis_quality").unwrap().unwrap();
        assert!(stored.contains("resolved"), "{stored}");
        assert!(
            idx.store
                .meta_get("analysis_quality_files")
                .unwrap()
                .is_some(),
            "per-file gauges live in store meta"
        );
        for e in idx.store.all_entities().unwrap() {
            if e.kind == scc_core::kinds::FILE {
                assert!(
                    !e.attributes.contains_key("analysis_quality"),
                    "gauges must not live on FILE entities (breaks incremental≡cold): {}",
                    e.id
                );
            }
        }
    }

    #[test]
    // trace:v1 id=test.scc.index.bm25-persist verifies=REQ-bm25-persist exercises=impl.scc.index.persist-bm25
    fn bm25_corpus_stats_persist_in_meta_not_file_entities() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/calc.py"),
            "def add(a, b):\n    return a + b\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let stored = idx
            .store
            .meta_get("bm25_corpus")
            .unwrap()
            .expect("bm25_corpus meta");
        let stats: scc_core::Bm25CorpusStats = serde_json::from_str(&stored).unwrap();
        assert!(stats.n >= 1, "expected indexed entities in BM25 corpus");
        assert!(
            stats.df.contains_key("add") || stats.df.keys().any(|k| k.contains("add")),
            "df should include extracted identifier tokens: {:?}",
            stats.df.keys().take(12).collect::<Vec<_>>()
        );
        for e in idx.store.all_entities().unwrap() {
            if e.kind == scc_core::kinds::FILE {
                assert!(
                    !e.attributes.contains_key("bm25_corpus"),
                    "BM25 stats must not live on FILE entities"
                );
            }
        }
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
            .map(|r| {
                format!(
                    "{} {} {} {}",
                    r.subject,
                    r.predicate,
                    r.object,
                    r.provenance.as_str()
                )
            })
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
            .map(|r| {
                format!(
                    "{} {} {} {}",
                    r.subject,
                    r.predicate,
                    r.object,
                    r.provenance.as_str()
                )
            })
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
            .map(|r| {
                format!(
                    "{} {} {} {}",
                    r.subject,
                    r.predicate,
                    r.object,
                    r.provenance.as_str()
                )
            })
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
                r.predicate == scc_core::predicates::IMPORTS && r.object.contains("renamed.py")
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

    #[test]
    // trace:v1 id=test.scc.index.type-narrow verifies=REQ-implement-phase-7-of-scc-x-ripwire-lessons-1-one-hop-type-narrowing exercises=impl.scc.resolve.type-narrow
    fn index_pins_named_var_call_to_constructor_type() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("w.py"),
            "class Order:\n    def process(self):\n        return 1\n\nclass Invoice:\n    def process(self):\n        return 2\n\ndef handle():\n    x = Order()\n    return x.process()\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let order = scc_core::symbol_id(&idx.store.repo_id, "w.py", "Order.process");
        let invoice = scc_core::symbol_id(&idx.store.repo_id, "w.py", "Invoice.process");
        let handle = scc_core::symbol_id(&idx.store.repo_id, "w.py", "handle");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == handle)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == order),
            "handle must CALL Order.process: {calls:?}"
        );
        assert!(
            !calls.iter().any(|r| r.object == invoice),
            "must not spray Invoice.process: {calls:?}"
        );
    }
}
