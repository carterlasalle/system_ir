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
                index.set_class_bases(&path, &self.load_class_bases(&path, &syms)?);
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
            index.set_fn_binds(path, &ef.fn_binds);
            index.set_class_bases(path, &ef.class_bases);
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
            record_file_quality(&self.store, path, f.language, &resolved_calls)?;
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

    fn load_class_bases(
        &self,
        path: &str,
        symbols: &[model::Symbol],
    ) -> Result<Vec<(String, Vec<String>)>, scc_store::StoreError> {
        let mut out: Vec<(String, Vec<String>)> = Vec::new();
        for s in symbols {
            if !matches!(
                s.kind,
                model::SymbolKind::Class | model::SymbolKind::Interface | model::SymbolKind::Type
            ) {
                continue;
            }
            let id = scc_core::symbol_id(&self.store.repo_id, path, &s.name);
            let Some(e) = self.store.get_entity(&id)? else {
                continue;
            };
            let Some(arr) = e.attributes.get("class_bases").and_then(|v| v.as_array()) else {
                continue;
            };
            let mut bases: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            bases.sort();
            bases.dedup();
            if !bases.is_empty() {
                out.push((s.name.clone(), bases));
            }
        }
        Ok(out)
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
                index.set_class_bases(&path, &self.load_class_bases(&path, &syms)?);
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
            index.set_fn_binds(p, &ef.fn_binds);
            index.set_class_bases(p, &ef.class_bases);
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
            record_file_quality(&self.store, path, f.language, &resolved_calls)?;
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
                index.set_class_bases(&path, &self.load_class_bases(&path, &syms)?);
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
    lang: Language,
    calls: &[resolve::ResolvedCall],
) -> Result<(), IndexError> {
    let mut q = resolve::quality_from_calls(calls);
    match scc_core::language_by_id(lang.as_str()).map(|c| c.tier) {
        Some(scc_core::LanguageTier::IndexSearch) | None => {
            q.files.unsupported = 1;
        }
        Some(_) => {
            q.files.parsed = 1;
        }
    }
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
// trace:v1 id=impl.scc.index.persist-analysis-quality work=WORK-ripwire-lessons-phase1 satisfies=REQ-resolution-honesty-gauges,REQ-implement-fix-pr-review-comments-without-collapsing-scc-type-script-no
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
    // trace:v1 id=test.scc.index.analysis-quality verifies=REQ-resolution-honesty-gauges,REQ-implement-fix-pr-review-comments-without-collapsing-scc-type-script-no exercises=impl.scc.index.persist-analysis-quality
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
        std::fs::write(root.join("src/util.c"), "int add(int a, int b) { return a + b; }\n")
            .unwrap();
        let report = idx.index().unwrap();
        assert!(
            report.analysis_quality.files.unsupported >= 1,
            "IndexSearch C must not count as parsed: {:?}",
            report.analysis_quality
        );
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

    #[test]
    // trace:v1 id=test.scc.index.python.ident-copy verifies=REQ-implement-phase-17-of-scc-x-ripwire-lessons-python-extract-time-ident exercises=impl.scc.extract.python.ident-copy
    fn index_pins_python_ident_copy_calls() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("w.py"),
            "class Order:\n    def process(self):\n        return 1\n\nclass Invoice:\n    def process(self):\n        return 2\n\ndef handle(x: Order):\n    y = x\n    return y.process()\n\ndef mixed():\n    z = Order()\n    w = z\n    w = Invoice()\n    return w.process()\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let order = scc_core::symbol_id(&idx.store.repo_id, "w.py", "Order.process");
        let invoice = scc_core::symbol_id(&idx.store.repo_id, "w.py", "Invoice.process");
        let handle = scc_core::symbol_id(&idx.store.repo_id, "w.py", "handle");
        let mixed = scc_core::symbol_id(&idx.store.repo_id, "w.py", "mixed");
        let rels = idx.store.all_relationships().unwrap();
        let handle_calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == handle)
            .collect();
        assert!(
            handle_calls.iter().any(|r| r.object == order),
            "handle must CALL Order.process: {handle_calls:?}"
        );
        assert!(
            !handle_calls.iter().any(|r| r.object == invoice),
            "must not spray Invoice.process: {handle_calls:?}"
        );
        let mixed_calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == mixed)
            .collect();
        assert!(
            !mixed_calls.iter().any(|r| r.object == order),
            "conflicting copy/ctor must not pin Order.process: {mixed_calls:?}"
        );
        assert!(
            !mixed_calls.iter().any(|r| r.object == invoice),
            "must not spray Invoice.process from mixed: {mixed_calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.field-type-narrow verifies=REQ-implement-phase-8-of-scc-x-ripwire-lessons-one-hop-field-type-narrowi exercises=impl.scc.resolve.field-type-narrow
    fn index_pins_self_field_call_to_field_type() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("w.py"),
            "class Order:\n    def process(self):\n        return 1\n\nclass Invoice:\n    def process(self):\n        return 2\n\nclass Svc:\n    def __init__(self):\n        self.owned = Order()\n    def run(self):\n        return self.owned.process()\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let order = scc_core::symbol_id(&idx.store.repo_id, "w.py", "Order.process");
        let invoice = scc_core::symbol_id(&idx.store.repo_id, "w.py", "Invoice.process");
        let run = scc_core::symbol_id(&idx.store.repo_id, "w.py", "Svc.run");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == order),
            "Svc.run must CALL Order.process: {calls:?}"
        );
        assert!(
            !calls.iter().any(|r| r.object == invoice),
            "must not spray Invoice.process: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.java.field-type-narrow verifies=REQ-implement-phase-9-of-scc-x-ripwire-lessons-java-one-hop-field-type-na exercises=impl.scc.extract.java.field-type
    fn index_pins_this_field_call_to_field_type() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("W.java"),
            r#"
class Order { void process() { } }
class Invoice { void process() { } }
class Svc {
    Svc() { this.owned = new Order(); }
    void run() { this.owned.process(); }
}
"#,
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let order = scc_core::symbol_id(&idx.store.repo_id, "W.java", "Order.process");
        let invoice = scc_core::symbol_id(&idx.store.repo_id, "W.java", "Invoice.process");
        let run = scc_core::symbol_id(&idx.store.repo_id, "W.java", "Svc.run");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == order),
            "Svc.run must CALL Order.process: {calls:?}"
        );
        assert!(
            !calls.iter().any(|r| r.object == invoice),
            "must not spray Invoice.process: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.go.receiver-field-type-narrow verifies=REQ-implement-phase-10-of-scc-x-ripwire-lessons-go-one-hop-receiver-field exercises=impl.scc.resolve.receiver-field-type-narrow
    fn index_pins_go_receiver_field_call_to_field_type() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("w.go"),
            r#"
package app
type Order struct{}
func (o *Order) Process() {}
type Invoice struct{}
func (i *Invoice) Process() {}
type Svc struct { owned *Order }
func (s *Svc) Run() { s.owned.Process() }
"#,
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let order = scc_core::symbol_id(&idx.store.repo_id, "w.go", "Order.Process");
        let invoice = scc_core::symbol_id(&idx.store.repo_id, "w.go", "Invoice.Process");
        let run = scc_core::symbol_id(&idx.store.repo_id, "w.go", "Svc.Run");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == order),
            "Svc.Run must CALL Order.Process: {calls:?}"
        );
        assert!(
            !calls.iter().any(|r| r.object == invoice),
            "must not spray Invoice.Process: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.rust.field-type-narrow verifies=REQ-implement-phase-11-of-scc-x-ripwire-lessons-rust-one-hop-self-field-t exercises=impl.scc.extract.rust.field-type
    fn index_pins_rust_self_field_call_to_field_type() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("w.rs"),
            r#"
struct Order;
impl Order { fn process(&self) {} }
struct Invoice;
impl Invoice { fn process(&self) {} }
struct Svc { owned: Order }
impl Svc { fn run(&self) { self.owned.process(); } }
"#,
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let order = scc_core::symbol_id(&idx.store.repo_id, "w.rs", "Order.process");
        let invoice = scc_core::symbol_id(&idx.store.repo_id, "w.rs", "Invoice.process");
        let run = scc_core::symbol_id(&idx.store.repo_id, "w.rs", "Svc.run");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == order),
            "Svc.run must CALL Order.process: {calls:?}"
        );
        assert!(
            !calls.iter().any(|r| r.object == invoice),
            "must not spray Invoice.process: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.rust.field-assign-tombstone verifies=REQ-implement-phase-14-of-scc-x-ripwire-lessons-rust-extract-time-self-fi exercises=impl.scc.extract.rust.field-assign
    fn index_does_not_pin_rust_field_after_conflicting_assignment() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("w.rs"),
            r#"
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
"#,
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let order = scc_core::symbol_id(&idx.store.repo_id, "w.rs", "Order.process");
        let invoice = scc_core::symbol_id(&idx.store.repo_id, "w.rs", "Invoice.process");
        let run = scc_core::symbol_id(&idx.store.repo_id, "w.rs", "Svc.run");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            !calls.iter().any(|r| r.object == order),
            "conflicting assignment must not pin Order.process: {calls:?}"
        );
        assert!(
            !calls.iter().any(|r| r.object == invoice),
            "must not spray Invoice.process either: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.rust.field-assign-same verifies=REQ-implement-phase-14-of-scc-x-ripwire-lessons-rust-extract-time-self-fi exercises=impl.scc.extract.rust.field-assign
    fn index_pins_rust_field_after_same_type_assignment() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("w.rs"),
            r#"
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
"#,
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let order = scc_core::symbol_id(&idx.store.repo_id, "w.rs", "Order.process");
        let invoice = scc_core::symbol_id(&idx.store.repo_id, "w.rs", "Invoice.process");
        let run = scc_core::symbol_id(&idx.store.repo_id, "w.rs", "Svc.run");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == order),
            "same-type assignment must still pin Order.process: {calls:?}"
        );
        assert!(
            !calls.iter().any(|r| r.object == invoice),
            "must not spray Invoice.process: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.java.unprefixed-field-type verifies=REQ-implement-phase-12-of-scc-x-ripwire-lessons-java-unprefixed-field-as exercises=impl.scc.resolve.unprefixed-field-type
    fn index_pins_java_unprefixed_field_call_to_field_type() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("W.java"),
            r#"
class Order { void process() { } }
class Invoice { void process() { } }
class Svc {
    private Order owned;
    void run() { owned.process(); }
}
"#,
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let order = scc_core::symbol_id(&idx.store.repo_id, "W.java", "Order.process");
        let invoice = scc_core::symbol_id(&idx.store.repo_id, "W.java", "Invoice.process");
        let run = scc_core::symbol_id(&idx.store.repo_id, "W.java", "Svc.run");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == order),
            "Svc.run must CALL Order.process via unprefixed field: {calls:?}"
        );
        assert!(
            !calls.iter().any(|r| r.object == invoice),
            "must not spray Invoice.process: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.go.field-assign-tombstone verifies=REQ-implement-phase-13-of-scc-x-ripwire-lessons-1-go-extract-time-recei exercises=impl.scc.extract.go.field-assign
    fn index_does_not_pin_go_field_after_conflicting_assignment() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("w.go"),
            r#"
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
"#,
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let order = scc_core::symbol_id(&idx.store.repo_id, "w.go", "Order.Process");
        let invoice = scc_core::symbol_id(&idx.store.repo_id, "w.go", "Invoice.Process");
        let run = scc_core::symbol_id(&idx.store.repo_id, "w.go", "Svc.Run");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            !calls.iter().any(|r| r.object == order),
            "conflicting assignment must not pin Order.Process: {calls:?}"
        );
        assert!(
            !calls.iter().any(|r| r.object == invoice),
            "must not spray Invoice.Process either: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.go.field-assign-same verifies=REQ-implement-phase-13-of-scc-x-ripwire-lessons-1-go-extract-time-recei exercises=impl.scc.extract.go.field-assign
    fn index_pins_go_field_after_same_type_assignment() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("w.go"),
            r#"
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
"#,
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let order = scc_core::symbol_id(&idx.store.repo_id, "w.go", "Order.Process");
        let invoice = scc_core::symbol_id(&idx.store.repo_id, "w.go", "Invoice.Process");
        let run = scc_core::symbol_id(&idx.store.repo_id, "w.go", "Svc.Run");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == order),
            "same-type assignment must still pin Order.Process: {calls:?}"
        );
        assert!(
            !calls.iter().any(|r| r.object == invoice),
            "must not spray Invoice.Process: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.go.local-type verifies=REQ-implement-phase-15-of-scc-x-ripwire-lessons-go-and-rust-extract-time exercises=impl.scc.extract.go.local-type
    fn index_pins_go_local_and_param_calls() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("w.go"),
            r#"
package app
type Order struct{}
func (o *Order) Process() {}
type Invoice struct{}
func (i *Invoice) Process() {}
func handle(x *Order) {
	y := &Order{}
	y.Process()
	x.Process()
}
func mixed() {
	z := &Order{}
	z = &Invoice{}
	z.Process()
}
"#,
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let order = scc_core::symbol_id(&idx.store.repo_id, "w.go", "Order.Process");
        let invoice = scc_core::symbol_id(&idx.store.repo_id, "w.go", "Invoice.Process");
        let handle = scc_core::symbol_id(&idx.store.repo_id, "w.go", "handle");
        let mixed = scc_core::symbol_id(&idx.store.repo_id, "w.go", "mixed");
        let rels = idx.store.all_relationships().unwrap();
        let handle_calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == handle)
            .collect();
        assert!(
            handle_calls.iter().any(|r| r.object == order),
            "handle must CALL Order.Process: {handle_calls:?}"
        );
        assert!(
            !handle_calls.iter().any(|r| r.object == invoice),
            "must not spray Invoice.Process: {handle_calls:?}"
        );
        let mixed_calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == mixed)
            .collect();
        assert!(
            !mixed_calls.iter().any(|r| r.object == order),
            "conflicting local assignment must not pin Order.Process: {mixed_calls:?}"
        );
        assert!(
            !mixed_calls.iter().any(|r| r.object == invoice),
            "must not spray Invoice.Process from mixed: {mixed_calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.rust.local-type verifies=REQ-implement-phase-15-of-scc-x-ripwire-lessons-go-and-rust-extract-time exercises=impl.scc.extract.rust.local-type
    fn index_pins_rust_local_and_param_calls() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("w.rs"),
            r#"
struct Order {}
impl Order { fn process(&self) {} }
struct Invoice {}
impl Invoice { fn process(&self) {} }
fn handle(x: Order) {
    let y = Order {};
    y.process();
    x.process();
}
fn mixed() {
    let mut z = Order {};
    z = Invoice {};
    z.process();
}
"#,
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let order = scc_core::symbol_id(&idx.store.repo_id, "w.rs", "Order.process");
        let invoice = scc_core::symbol_id(&idx.store.repo_id, "w.rs", "Invoice.process");
        let handle = scc_core::symbol_id(&idx.store.repo_id, "w.rs", "handle");
        let mixed = scc_core::symbol_id(&idx.store.repo_id, "w.rs", "mixed");
        let rels = idx.store.all_relationships().unwrap();
        let handle_calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == handle)
            .collect();
        assert!(
            handle_calls.iter().any(|r| r.object == order),
            "handle must CALL Order.process: {handle_calls:?}"
        );
        assert!(
            !handle_calls.iter().any(|r| r.object == invoice),
            "must not spray Invoice.process: {handle_calls:?}"
        );
        let mixed_calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == mixed)
            .collect();
        assert!(
            !mixed_calls.iter().any(|r| r.object == order),
            "conflicting local assignment must not pin Order.process: {mixed_calls:?}"
        );
        assert!(
            !mixed_calls.iter().any(|r| r.object == invoice),
            "must not spray Invoice.process from mixed: {mixed_calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.go.type-assert verifies=REQ-implement-phase-16-of-scc-x-ripwire-lessons-extract-time-type-asserti exercises=impl.scc.extract.go.type-assert
    fn index_pins_go_type_assert_and_conversion() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("w.go"),
            r#"
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
}
func mixed(v any) {
	z := v.(*Order)
	z = v.(*Invoice)
	z.Process()
}
"#,
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let order = scc_core::symbol_id(&idx.store.repo_id, "w.go", "Order.Process");
        let invoice = scc_core::symbol_id(&idx.store.repo_id, "w.go", "Invoice.Process");
        let handle = scc_core::symbol_id(&idx.store.repo_id, "w.go", "handle");
        let mixed = scc_core::symbol_id(&idx.store.repo_id, "w.go", "mixed");
        let rels = idx.store.all_relationships().unwrap();
        let handle_calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == handle)
            .collect();
        assert!(
            handle_calls.iter().any(|r| r.object == order),
            "handle must CALL Order.Process: {handle_calls:?}"
        );
        assert!(
            !handle_calls.iter().any(|r| r.object == invoice),
            "must not spray Invoice.Process: {handle_calls:?}"
        );
        let mixed_calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == mixed)
            .collect();
        assert!(
            !mixed_calls.iter().any(|r| r.object == order),
            "conflicting assertion must not pin Order.Process: {mixed_calls:?}"
        );
        assert!(
            !mixed_calls.iter().any(|r| r.object == invoice),
            "must not spray Invoice.Process from mixed: {mixed_calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.java.type-cast verifies=REQ-implement-phase-19-of-scc-x-ripwire-lessons-java-extract-time-cast-as exercises=impl.scc.extract.java.type-cast
    fn index_pins_java_cast_local_and_assignment() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("W.java"),
            r#"
class Order { void process() { } }
class Invoice { void process() { } }
class Svc {
    void handle(Object v) {
        var x = (Order) v;
        x.process();
        z = (Order) v;
        z.process();
        x.y.process();
    }
    void mixed(Object v) {
        var y = (Order) v;
        y = (Invoice) v;
        y.process();
    }
    void factory(Object v) {
        var f = MakeOrder();
        f.process();
    }
}
"#,
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let order = scc_core::symbol_id(&idx.store.repo_id, "W.java", "Order.process");
        let invoice = scc_core::symbol_id(&idx.store.repo_id, "W.java", "Invoice.process");
        let handle = scc_core::symbol_id(&idx.store.repo_id, "W.java", "Svc.handle");
        let mixed = scc_core::symbol_id(&idx.store.repo_id, "W.java", "Svc.mixed");
        let factory = scc_core::symbol_id(&idx.store.repo_id, "W.java", "Svc.factory");
        let rels = idx.store.all_relationships().unwrap();
        let handle_calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == handle)
            .collect();
        assert!(
            handle_calls.iter().any(|r| r.object == order),
            "handle must CALL Order.process: {handle_calls:?}"
        );
        assert!(
            !handle_calls.iter().any(|r| r.object == invoice),
            "must not spray Invoice.process: {handle_calls:?}"
        );
        let mixed_calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == mixed)
            .collect();
        assert!(
            !mixed_calls.iter().any(|r| r.object == order),
            "conflicting cast must not pin Order.process: {mixed_calls:?}"
        );
        assert!(
            !mixed_calls.iter().any(|r| r.object == invoice),
            "must not spray Invoice.process from mixed: {mixed_calls:?}"
        );
        let factory_calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == factory)
            .collect();
        assert!(
            !factory_calls.iter().any(|r| r.object == order),
            "opaque factory must not mint a bind: {factory_calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.class-name-pin verifies=REQ-implement-phase-21-of-scc-x-ripwire-lessons-absorb-rule-2c-class-name exercises=impl.scc.resolve.class-name
    fn index_pins_class_name_receiver_to_unique_class_method() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("w.py"),
            "class Order:\n    def process(self):\n        return 1\n\nclass Invoice:\n    def process(self):\n        return 2\n\ndef handle():\n    return Order.process()\n",
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

    #[test]
    // trace:v1 id=test.scc.index.class-name-typed-shadow verifies=REQ-implement-phase-21-of-scc-x-ripwire-lessons-absorb-rule-2c-class-name exercises=impl.scc.resolve.class-name
    fn index_class_name_receiver_typed_param_beats_class() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("w.py"),
            "class Order:\n    def process(self):\n        return 1\n\nclass Invoice:\n    def process(self):\n        return 2\n\ndef handle(Order: Invoice):\n    return Order.process()\n",
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
            calls.iter().any(|r| r.object == invoice),
            "typed param Order: Invoice must CALL Invoice.process: {calls:?}"
        );
        assert!(
            !calls.iter().any(|r| r.object == order),
            "must not pin class Order.process through a typed param: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.class-name-untyped-veto verifies=REQ-implement-phase-21-of-scc-x-ripwire-lessons-absorb-rule-2c-class-name exercises=impl.scc.extract.python.param-shadow
    fn index_class_name_receiver_untyped_param_vetoes_class() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("w.py"),
            "class Order:\n    def process(self):\n        return 1\n\ndef handle(Order):\n    return Order.process()\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let order = scc_core::symbol_id(&idx.store.repo_id, "w.py", "Order.process");
        let handle = scc_core::symbol_id(&idx.store.repo_id, "w.py", "handle");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == handle)
            .collect();
        assert!(
            !calls.iter().any(|r| r.object == order),
            "untyped param must veto class Order.process: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.class-name-split verifies=REQ-implement-phase-21-of-scc-x-ripwire-lessons-absorb-rule-2c-class-name exercises=impl.scc.resolve.class-name
    fn index_class_name_receiver_two_defining_classes_unresolved() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("a.py"),
            "class Order:\n    def process(self):\n        return 1\n",
        )
        .unwrap();
        std::fs::write(
            root.join("b.py"),
            "class Order:\n    def process(self):\n        return 2\n",
        )
        .unwrap();
        std::fs::write(
            root.join("w.py"),
            "def handle():\n    return Order.process()\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let a = scc_core::symbol_id(&idx.store.repo_id, "a.py", "Order.process");
        let b = scc_core::symbol_id(&idx.store.repo_id, "b.py", "Order.process");
        let handle = scc_core::symbol_id(&idx.store.repo_id, "w.py", "handle");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == handle)
            .collect();
        assert!(
            !calls.iter().any(|r| r.object == a || r.object == b),
            "two defining classes must not spray: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.class-name-cross-file verifies=REQ-implement-phase-21-of-scc-x-ripwire-lessons-absorb-rule-2c-class-name exercises=impl.scc.resolve.class-name
    fn index_class_name_receiver_cross_file_unique_class() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("order.py"),
            "class Order:\n    def process(self):\n        return 1\n",
        )
        .unwrap();
        std::fs::write(
            root.join("w.py"),
            "def handle():\n    return Order.process()\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let order = scc_core::symbol_id(&idx.store.repo_id, "order.py", "Order.process");
        let handle = scc_core::symbol_id(&idx.store.repo_id, "w.py", "handle");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == handle)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == order),
            "unique class in another file must pin Order.process: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.cha-base-pin verifies=REQ-implement-phase-22-of-scc-x-ripwire-lessons-absorb-rule-2c-cha-meth exercises=impl.scc.resolve.cha-bases
    fn index_pins_class_name_receiver_through_unique_base() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("iers.py"),
            "class IERS:\n    def open(self):\n        return 1\n",
        )
        .unwrap();
        std::fs::write(root.join("iers_b.py"), "class IERS_B(IERS):\n    pass\n").unwrap();
        std::fs::write(
            root.join("w.py"),
            "def handle():\n    return IERS_B.open()\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let base = scc_core::symbol_id(&idx.store.repo_id, "iers.py", "IERS.open");
        let handle = scc_core::symbol_id(&idx.store.repo_id, "w.py", "handle");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == handle)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == base),
            "IERS_B.open must CALL IERS.open: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.cha-base-split verifies=REQ-implement-phase-22-of-scc-x-ripwire-lessons-absorb-rule-2c-cha-meth exercises=impl.scc.resolve.cha-bases
    fn index_class_name_receiver_two_bases_unresolved() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("a.py"),
            "class A:\n    def m(self):\n        return 1\n",
        )
        .unwrap();
        std::fs::write(
            root.join("b.py"),
            "class B:\n    def m(self):\n        return 2\n",
        )
        .unwrap();
        std::fs::write(root.join("c.py"), "class C(A, B):\n    pass\n").unwrap();
        std::fs::write(root.join("w.py"), "def handle():\n    return C.m()\n").unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let a = scc_core::symbol_id(&idx.store.repo_id, "a.py", "A.m");
        let b = scc_core::symbol_id(&idx.store.repo_id, "b.py", "B.m");
        let handle = scc_core::symbol_id(&idx.store.repo_id, "w.py", "handle");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == handle)
            .collect();
        assert!(
            !calls.iter().any(|r| r.object == a || r.object == b),
            "two hitting bases must not spray: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.cha-base-incremental verifies=REQ-implement-phase-22-of-scc-x-ripwire-lessons-absorb-rule-2c-cha-meth exercises=impl.scc.write.class-bases
    fn index_cha_base_walk_survives_incremental_caller_edit() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("iers.py"),
            "class IERS:\n    def open(self):\n        return 1\n",
        )
        .unwrap();
        std::fs::write(root.join("iers_b.py"), "class IERS_B(IERS):\n    pass\n").unwrap();
        std::fs::write(
            root.join("w.py"),
            "def handle():\n    return IERS_B.open()\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        std::fs::write(
            root.join("w.py"),
            "def handle():\n    return IERS_B.open()\n# touch\n",
        )
        .unwrap();
        idx.index().unwrap();
        let base = scc_core::symbol_id(&idx.store.repo_id, "iers.py", "IERS.open");
        let handle = scc_core::symbol_id(&idx.store.repo_id, "w.py", "handle");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == handle)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == base),
            "incremental caller edit must still CALL IERS.open via persisted heritage: {calls:?}"
        );
        let (cold, _t2) = indexer_for(root);
        cold.index().unwrap();
        let cold_base = scc_core::symbol_id(&cold.store.repo_id, "iers.py", "IERS.open");
        let cold_handle = scc_core::symbol_id(&cold.store.repo_id, "w.py", "handle");
        let cold_hit = cold.store.all_relationships().unwrap().iter().any(|r| {
            r.predicate == scc_core::predicates::CALLS
                && r.subject == cold_handle
                && r.object == cold_base
        });
        assert!(cold_hit, "cold index must also CALL IERS.open");
    }

    #[test]
    // trace:v1 id=test.scc.index.self-cha verifies=REQ-implement-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-s exercises=impl.scc.resolve.rule1
    fn index_self_open_on_derived_pins_unique_base() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("iers.py"),
            "class IERS:\n    def open(self):\n        return 1\n",
        )
        .unwrap();
        std::fs::write(
            root.join("iers_b.py"),
            "class IERS_B(IERS):\n    def run(self):\n        return self.open()\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let base = scc_core::symbol_id(&idx.store.repo_id, "iers.py", "IERS.open");
        let run = scc_core::symbol_id(&idx.store.repo_id, "iers_b.py", "IERS_B.run");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == base),
            "IERS_B.run self.open must CALL IERS.open: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.super-cha verifies=REQ-implement-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-s exercises=impl.scc.recv.super-call
    fn index_super_open_skips_own_class() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("iers.py"),
            "class IERS:\n    def open(self):\n        return 1\n",
        )
        .unwrap();
        std::fs::write(
            root.join("iers_b.py"),
            "class IERS_B(IERS):\n    def open(self):\n        return 2\n    def run(self):\n        return super().open()\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let base = scc_core::symbol_id(&idx.store.repo_id, "iers.py", "IERS.open");
        let own = scc_core::symbol_id(&idx.store.repo_id, "iers_b.py", "IERS_B.open");
        let run = scc_core::symbol_id(&idx.store.repo_id, "iers_b.py", "IERS_B.run");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == base),
            "super().open must CALL IERS.open: {calls:?}"
        );
        assert!(
            !calls.iter().any(|r| r.object == own),
            "super().open must not pin own IERS_B.open: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.self-cha-split verifies=REQ-implement-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-s exercises=impl.scc.resolve.rule1
    fn index_self_two_bases_unresolved() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("a.py"),
            "class A:\n    def m(self):\n        return 1\n",
        )
        .unwrap();
        std::fs::write(
            root.join("b.py"),
            "class B:\n    def m(self):\n        return 2\n",
        )
        .unwrap();
        std::fs::write(
            root.join("c.py"),
            "class C(A, B):\n    def run(self):\n        return self.m()\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let a = scc_core::symbol_id(&idx.store.repo_id, "a.py", "A.m");
        let b = scc_core::symbol_id(&idx.store.repo_id, "b.py", "B.m");
        let run = scc_core::symbol_id(&idx.store.repo_id, "c.py", "C.run");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            !calls.iter().any(|r| r.object == a || r.object == b),
            "two hitting bases must not spray self.m: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.self-cha-java verifies=REQ-implement-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-s exercises=impl.scc.resolve.rule1
    fn index_java_this_and_super_cha() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("Iers.java"),
            "class IERS {\n    void open() {}\n}\nclass IERS_B extends IERS {\n    void open() {}\n    void run() { this.open(); super.open(); }\n}\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let own = scc_core::symbol_id(&idx.store.repo_id, "Iers.java", "IERS_B.open");
        let base = scc_core::symbol_id(&idx.store.repo_id, "Iers.java", "IERS.open");
        let run = scc_core::symbol_id(&idx.store.repo_id, "Iers.java", "IERS_B.run");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == own),
            "this.open must CALL IERS_B.open: {calls:?}"
        );
        assert!(
            calls.iter().any(|r| r.object == base),
            "super.open must CALL IERS.open: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.self-cha-ts verifies=REQ-implement-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-s exercises=impl.scc.resolve.rule1
    fn index_ts_this_and_super_cha() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("iers.ts"),
            "class IERS { open() {} }\nclass IERS_B extends IERS {\n  open() {}\n  run() { this.open(); super.open(); }\n}\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let own = scc_core::symbol_id(&idx.store.repo_id, "iers.ts", "IERS_B.open");
        let base = scc_core::symbol_id(&idx.store.repo_id, "iers.ts", "IERS.open");
        let run = scc_core::symbol_id(&idx.store.repo_id, "iers.ts", "IERS_B.run");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == own),
            "this.open must CALL IERS_B.open: {calls:?}"
        );
        assert!(
            calls.iter().any(|r| r.object == base),
            "super.open must CALL IERS.open: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.self-cha-incremental verifies=REQ-implement-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-s exercises=impl.scc.write.class-bases
    fn index_self_cha_survives_incremental_base_edit() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("iers.py"),
            "class IERS:\n    def open(self):\n        return 1\n",
        )
        .unwrap();
        std::fs::write(
            root.join("iers_b.py"),
            "class IERS_B(IERS):\n    def run(self):\n        return self.open()\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        std::fs::write(
            root.join("iers.py"),
            "class IERS:\n    def open(self):\n        return 1\n# touch\n",
        )
        .unwrap();
        idx.index().unwrap();
        let base = scc_core::symbol_id(&idx.store.repo_id, "iers.py", "IERS.open");
        let run = scc_core::symbol_id(&idx.store.repo_id, "iers_b.py", "IERS_B.run");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == base),
            "incremental base edit must still CALL IERS.open via persisted heritage: {calls:?}"
        );
        let (cold, _t2) = indexer_for(root);
        cold.index().unwrap();
        let cold_base = scc_core::symbol_id(&cold.store.repo_id, "iers.py", "IERS.open");
        let cold_run = scc_core::symbol_id(&cold.store.repo_id, "iers_b.py", "IERS_B.run");
        let cold_hit = cold.store.all_relationships().unwrap().iter().any(|r| {
            r.predicate == scc_core::predicates::CALLS
                && r.subject == cold_run
                && r.object == cold_base
        });
        assert!(cold_hit, "cold index must also CALL IERS.open");
    }

    #[test]
    // trace:v1 id=test.scc.index.rust-trait-cha verifies=REQ-implement-phase-24-of-scc-x-ripwire-lessons-absorb-rust-impl-trait-fo exercises=impl.scc.extract.rust.trait-impl
    fn index_rust_trait_default_pins_typed_receiver() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("open.rs"),
            "pub trait Open {\n    fn open(&self) {}\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("w.rs"),
            "struct IERS_B;\nimpl Open for IERS_B {}\nfn handle(x: IERS_B) {\n    x.open();\n}\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let base = scc_core::symbol_id(&idx.store.repo_id, "open.rs", "Open.open");
        let handle = scc_core::symbol_id(&idx.store.repo_id, "w.rs", "handle");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == handle)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == base),
            "x.open on IERS_B must CALL Open.open via trait CHA: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.rust-trait-inherent verifies=REQ-implement-phase-24-of-scc-x-ripwire-lessons-absorb-rust-impl-trait-fo exercises=impl.scc.extract.rust.class-bases
    fn index_rust_inherent_impl_wins_over_trait() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("open.rs"),
            "pub trait Open {\n    fn open(&self) {}\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("w.rs"),
            "struct IERS_B;\nimpl Open for IERS_B {}\nimpl IERS_B {\n    fn open(&self) {}\n}\nfn handle(x: IERS_B) {\n    x.open();\n}\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let own = scc_core::symbol_id(&idx.store.repo_id, "w.rs", "IERS_B.open");
        let tr = scc_core::symbol_id(&idx.store.repo_id, "open.rs", "Open.open");
        let handle = scc_core::symbol_id(&idx.store.repo_id, "w.rs", "handle");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == handle)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == own),
            "inherent IERS_B.open must win: {calls:?}"
        );
        assert!(
            !calls.iter().any(|r| r.object == tr),
            "must not spray to Open.open: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.rust-trait-split verifies=REQ-implement-phase-24-of-scc-x-ripwire-lessons-absorb-rust-impl-trait-fo exercises=impl.scc.resolve.cha-bases
    fn index_rust_two_traits_unresolved() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "pub trait A {\n    fn m(&self) {}\n}\n").unwrap();
        std::fs::write(root.join("b.rs"), "pub trait B {\n    fn m(&self) {}\n}\n").unwrap();
        std::fs::write(
            root.join("c.rs"),
            "struct C;\nimpl A for C {}\nimpl B for C {}\nfn handle(x: C) {\n    x.m();\n}\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let a = scc_core::symbol_id(&idx.store.repo_id, "a.rs", "A.m");
        let b = scc_core::symbol_id(&idx.store.repo_id, "b.rs", "B.m");
        let handle = scc_core::symbol_id(&idx.store.repo_id, "c.rs", "handle");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == handle)
            .collect();
        assert!(
            !calls.iter().any(|r| r.object == a || r.object == b),
            "two hitting traits must not spray: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.rust-trait-incremental verifies=REQ-implement-phase-24-of-scc-x-ripwire-lessons-absorb-rust-impl-trait-fo exercises=impl.scc.write.class-bases
    fn index_rust_trait_cha_survives_incremental_caller_edit() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("open.rs"),
            "pub trait Open {\n    fn open(&self) {}\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("iers_b.rs"),
            "struct IERS_B;\nimpl Open for IERS_B {}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("w.rs"),
            "fn handle(x: IERS_B) {\n    x.open();\n}\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        std::fs::write(
            root.join("w.rs"),
            "fn handle(x: IERS_B) {\n    x.open();\n    // touch\n}\n",
        )
        .unwrap();
        idx.index().unwrap();
        let base = scc_core::symbol_id(&idx.store.repo_id, "open.rs", "Open.open");
        let handle = scc_core::symbol_id(&idx.store.repo_id, "w.rs", "handle");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == handle)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == base),
            "incremental caller edit must still CALL Open.open via persisted heritage: {calls:?}"
        );
        let (cold, _t2) = indexer_for(root);
        cold.index().unwrap();
        let cold_base = scc_core::symbol_id(&cold.store.repo_id, "open.rs", "Open.open");
        let cold_handle = scc_core::symbol_id(&cold.store.repo_id, "w.rs", "handle");
        let cold_hit = cold.store.all_relationships().unwrap().iter().any(|r| {
            r.predicate == scc_core::predicates::CALLS
                && r.subject == cold_handle
                && r.object == cold_base
        });
        assert!(cold_hit, "cold index must also CALL Open.open");
    }

    #[test]
    // trace:v1 id=test.scc.index.fn-alias verifies=REQ-implement-phase-25-of-scc-x-ripwire-lessons-absorb-extract-time-funct exercises=impl.scc.resolve.fn-alias
    fn index_pins_python_fn_alias_and_not_same_named_global() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("w.py"),
            "def helper():\n    return 1\ndef f():\n    return 2\ndef run():\n    f = helper\n    return f()\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let helper = scc_core::symbol_id(&idx.store.repo_id, "w.py", "helper");
        let global_f = scc_core::symbol_id(&idx.store.repo_id, "w.py", "f");
        let run = scc_core::symbol_id(&idx.store.repo_id, "w.py", "run");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == helper),
            "run must CALL helper via alias: {calls:?}"
        );
        assert!(
            !calls.iter().any(|r| r.object == global_f),
            "must not spray to global f: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.fn-alias-langs verifies=REQ-implement-phase-25-of-scc-x-ripwire-lessons-absorb-extract-time-funct exercises=impl.scc.resolve.fn-alias
    fn index_pins_ts_go_rust_fn_alias_calls() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("w.ts"),
            "function helper() { return 1; }\nfunction f() { return 2; }\nfunction run() {\n  const f = helper;\n  return f();\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("w.go"),
            "package app\nfunc helper() {}\nfunc f() {}\nfunc run() {\n\tf := helper\n\tf()\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("w.rs"),
            "fn helper() {}\nfn f() {}\nfn run() {\n    let f = helper;\n    f();\n}\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let rels = idx.store.all_relationships().unwrap();
        for (path, caller) in [("w.ts", "run"), ("w.go", "run"), ("w.rs", "run")] {
            let helper = scc_core::symbol_id(&idx.store.repo_id, path, "helper");
            let global_f = scc_core::symbol_id(&idx.store.repo_id, path, "f");
            let run = scc_core::symbol_id(&idx.store.repo_id, path, caller);
            let calls: Vec<_> = rels
                .iter()
                .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
                .collect();
            assert!(
                calls.iter().any(|r| r.object == helper),
                "{path} run must CALL helper via alias: {calls:?}"
            );
            assert!(
                !calls.iter().any(|r| r.object == global_f),
                "{path} must not spray to global f: {calls:?}"
            );
        }
    }

    #[test]
    // trace:v1 id=test.scc.index.fn-alias-incremental verifies=REQ-implement-phase-25-of-scc-x-ripwire-lessons-absorb-extract-time-funct exercises=impl.scc.extract.python.fn-alias
    fn index_fn_alias_survives_incremental_caller_edit() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("helpers.py"), "def helper():\n    return 1\n").unwrap();
        std::fs::write(
            root.join("w.py"),
            "from helpers import helper\ndef run():\n    f = helper\n    return f()\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        std::fs::write(
            root.join("w.py"),
            "from helpers import helper\ndef run():\n    f = helper\n    return f()  # touch\n",
        )
        .unwrap();
        idx.index().unwrap();
        let helper = scc_core::symbol_id(&idx.store.repo_id, "helpers.py", "helper");
        let run = scc_core::symbol_id(&idx.store.repo_id, "w.py", "run");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == helper),
            "incremental caller edit must still CALL imported helper: {calls:?}"
        );
        let (cold, _t2) = indexer_for(root);
        cold.index().unwrap();
        let cold_helper = scc_core::symbol_id(&cold.store.repo_id, "helpers.py", "helper");
        let cold_run = scc_core::symbol_id(&cold.store.repo_id, "w.py", "run");
        let cold_hit = cold.store.all_relationships().unwrap().iter().any(|r| {
            r.predicate == scc_core::predicates::CALLS
                && r.subject == cold_run
                && r.object == cold_helper
        });
        assert!(cold_hit, "cold index must also CALL imported helper");
    }

    #[test]
    // trace:v1 id=test.scc.index.fn-alias-const verifies=REQ-implement-phase-25-of-scc-x-ripwire-lessons-absorb-extract-time-funct,REQ-implement-fix-pr-review-comments-without-collapsing-scc-type-script-no exercises=impl.scc.resolve.fn-alias
    fn index_pins_ts_function_valued_const_alias() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("w.ts"),
            "const helper = () => 1;\nconst LIMIT = 10;\nfunction f() { return 2; }\nfunction run() {\n  const g = helper;\n  return g();\n}\nfunction bad() {\n  const h = LIMIT;\n  return h();\n}\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let helper = scc_core::symbol_id(&idx.store.repo_id, "w.ts", "helper");
        let global_f = scc_core::symbol_id(&idx.store.repo_id, "w.ts", "f");
        let run = scc_core::symbol_id(&idx.store.repo_id, "w.ts", "run");
        let bad = scc_core::symbol_id(&idx.store.repo_id, "w.ts", "bad");
        let rels = idx.store.all_relationships().unwrap();
        let run_calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            run_calls.iter().any(|r| r.object == helper),
            "run must CALL function-valued const helper: {run_calls:?}"
        );
        assert!(
            !run_calls.iter().any(|r| r.object == global_f),
            "must not spray to global f: {run_calls:?}"
        );
        let bad_calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == bad)
            .collect();
        assert!(
            !bad_calls.iter().any(|r| r.object == helper),
            "LIMIT alias must not pin helper: {bad_calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.rule3-include-file verifies=REQ-implement-phase-26-of-scc-x-ripwire-lessons-absorb-rule-3-import-incl exercises=impl.scc.resolve.rule3-include-file
    fn index_rule3_python_wildcard_pins_unique_imported_helper() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("a.py"),
            "def helper():\n    return 1\ndef foo():\n    return 0\n",
        )
        .unwrap();
        std::fs::write(root.join("b.py"), "def helper():\n    return 2\n").unwrap();
        std::fs::write(
            root.join("w.py"),
            "from a import *\ndef run():\n    return helper()\n",
        )
        .unwrap();
        std::fs::write(
            root.join("both.py"),
            "from a import *\nfrom b import *\ndef run():\n    return helper()\n",
        )
        .unwrap();
        std::fs::write(root.join("neither.py"), "def run():\n    return helper()\n").unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let a_helper = scc_core::symbol_id(&idx.store.repo_id, "a.py", "helper");
        let b_helper = scc_core::symbol_id(&idx.store.repo_id, "b.py", "helper");
        let run = scc_core::symbol_id(&idx.store.repo_id, "w.py", "run");
        let both = scc_core::symbol_id(&idx.store.repo_id, "both.py", "run");
        let neither = scc_core::symbol_id(&idx.store.repo_id, "neither.py", "run");
        let rels = idx.store.all_relationships().unwrap();
        let run_calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            run_calls.iter().any(|r| r.object == a_helper),
            "wildcard import of a must CALL a.helper: {run_calls:?}"
        );
        assert!(
            !run_calls.iter().any(|r| r.object == b_helper),
            "must not spray to b.helper: {run_calls:?}"
        );
        let both_calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == both)
            .collect();
        assert!(
            !both_calls.iter().any(|r| r.object == a_helper)
                && !both_calls.iter().any(|r| r.object == b_helper),
            "both imported defining files must stay unresolved: {both_calls:?}"
        );
        let neither_calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == neither)
            .collect();
        assert!(
            !neither_calls.iter().any(|r| r.object == a_helper)
                && !neither_calls.iter().any(|r| r.object == b_helper),
            "no import must not spray: {neither_calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.rule3-langs verifies=REQ-implement-phase-26-of-scc-x-ripwire-lessons-absorb-rule-3-import-incl exercises=impl.scc.resolve.rule3-include-file
    fn index_rule3_ts_imported_file_pins_without_named_helper() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("a.ts"),
            "export function helper() { return 1; }\nexport function foo() { return 0; }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("b.ts"),
            "export function helper() { return 2; }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("w.ts"),
            "import { foo } from \"./a\";\nexport function run() { return helper(); }\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let a_helper = scc_core::symbol_id(&idx.store.repo_id, "a.ts", "helper");
        let b_helper = scc_core::symbol_id(&idx.store.repo_id, "b.ts", "helper");
        let run = scc_core::symbol_id(&idx.store.repo_id, "w.ts", "run");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == a_helper),
            "importing ./a must CALL a.helper even when helper is not named: {calls:?}"
        );
        assert!(
            !calls.iter().any(|r| r.object == b_helper),
            "must not spray to b.helper: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.rule3-incremental verifies=REQ-implement-phase-26-of-scc-x-ripwire-lessons-absorb-rule-3-import-incl exercises=impl.scc.resolve.rule3-include-file
    fn index_rule3_survives_incremental_caller_edit() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("pkg")).unwrap();
        std::fs::create_dir_all(root.join("other")).unwrap();
        std::fs::write(
            root.join("pkg/a.py"),
            "def helper():\n    return 1\ndef foo():\n    return 0\n",
        )
        .unwrap();
        std::fs::write(root.join("other/a.py"), "def helper():\n    return 2\n").unwrap();
        std::fs::write(
            root.join("w.py"),
            "from pkg.a import foo\ndef run():\n    return helper()\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        std::fs::write(
            root.join("w.py"),
            "from pkg.a import foo\ndef run():\n    return helper()  # touch\n",
        )
        .unwrap();
        idx.index().unwrap();
        let pkg = scc_core::symbol_id(&idx.store.repo_id, "pkg/a.py", "helper");
        let other = scc_core::symbol_id(&idx.store.repo_id, "other/a.py", "helper");
        let run = scc_core::symbol_id(&idx.store.repo_id, "w.py", "run");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == pkg),
            "path-precise pkg.a must pin after incremental edit: {calls:?}"
        );
        assert!(
            !calls.iter().any(|r| r.object == other),
            "must not basename-guess other/a.py: {calls:?}"
        );
        let (cold, _t2) = indexer_for(root);
        cold.index().unwrap();
        let cold_pkg = scc_core::symbol_id(&cold.store.repo_id, "pkg/a.py", "helper");
        let cold_run = scc_core::symbol_id(&cold.store.repo_id, "w.py", "run");
        let cold_hit = cold.store.all_relationships().unwrap().iter().any(|r| {
            r.predicate == scc_core::predicates::CALLS
                && r.subject == cold_run
                && r.object == cold_pkg
        });
        assert!(cold_hit, "cold index must also CALL pkg/a.py helper");
    }

    #[test]
    // trace:v1 id=test.scc.index.rust-import verifies=REQ-implement-phase-27-of-scc-x-ripwire-lessons-absorb-rust-step-a-path-p exercises=impl.scc.resolve.rust-import
    fn index_rust_crate_use_pins_named_helper_not_decoy() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/geo")).unwrap();
        std::fs::create_dir_all(root.join("src/other")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "mod geo;\nmod util;\n").unwrap();
        std::fs::write(
            root.join("src/geo/mod.rs"),
            "pub fn helper() -> i32 { 1 }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/other/mod.rs"),
            "pub fn helper() -> i32 { 9 }\n",
        )
        .unwrap();
        std::fs::write(root.join("src/util.rs"), "pub fn utilfn() -> i32 { 2 }\n").unwrap();
        std::fs::write(
            root.join("src/consumer.rs"),
            "use crate::geo::helper;\nuse crate::util::utilfn;\nfn run() {\n    helper();\n    utilfn();\n}\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let geo = scc_core::symbol_id(&idx.store.repo_id, "src/geo/mod.rs", "helper");
        let decoy = scc_core::symbol_id(&idx.store.repo_id, "src/other/mod.rs", "helper");
        let util = scc_core::symbol_id(&idx.store.repo_id, "src/util.rs", "utilfn");
        let run = scc_core::symbol_id(&idx.store.repo_id, "src/consumer.rs", "run");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            calls.iter().any(|r| r.object == geo),
            "crate::geo::helper must CALL geo/mod.rs helper: {calls:?}"
        );
        assert!(
            !calls.iter().any(|r| r.object == decoy),
            "must not basename-guess other/mod.rs: {calls:?}"
        );
        assert!(
            calls.iter().any(|r| r.object == util),
            "crate::util::utilfn must CALL util.rs: {calls:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.index.rust-import-degrade verifies=REQ-implement-phase-27-of-scc-x-ripwire-lessons-absorb-rust-step-a-path-p exercises=impl.scc.resolve.rust-import
    fn index_rust_crate_use_degrades_when_rs_and_mod_rs_both_exist() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/amb")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "mod amb;\n").unwrap();
        std::fs::write(root.join("src/amb.rs"), "pub fn dupfn() -> i32 { 1 }\n").unwrap();
        std::fs::write(root.join("src/amb/mod.rs"), "pub fn dupfn() -> i32 { 2 }\n").unwrap();
        std::fs::write(
            root.join("src/caller.rs"),
            "use crate::amb::dupfn;\nfn run() { dupfn(); }\n",
        )
        .unwrap();
        let (idx, _t) = indexer_for(root);
        idx.index().unwrap();
        let a = scc_core::symbol_id(&idx.store.repo_id, "src/amb.rs", "dupfn");
        let b = scc_core::symbol_id(&idx.store.repo_id, "src/amb/mod.rs", "dupfn");
        let run = scc_core::symbol_id(&idx.store.repo_id, "src/caller.rs", "run");
        let rels = idx.store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.subject == run)
            .collect();
        assert!(
            !calls.iter().any(|r| r.object == a) && !calls.iter().any(|r| r.object == b),
            "amb.rs + amb/mod.rs must not guess a CALL: {calls:?}"
        );
    }
}
