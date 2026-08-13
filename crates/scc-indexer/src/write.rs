//! Reality Compiler: normalize extraction + resolution output into
//! evidence-backed store rows (docs/SYSTEM_DESIGN.md §4).
//!
//! All ids are content-derived and deterministic: re-indexing the same
//! content produces identical ids, which makes full-vs-incremental
//! equivalence tests exact (docs/TEST_PLAN.md §7).

use crate::model::{
    ExtractedFile, ImportType, Route, StoreOp, StoreRef, SymbolKind, Test,
    TestKind,
};
use crate::resolve::{ResolvedCall, ResolvedImport, SymbolIndex};
use scc_core::kinds;
use scc_core::{entity_id, Evidence, Provenance, Relationship};
use scc_store::Store;

pub fn rel_id(parts: &[&str]) -> String {
    let mut h = blake3::Hasher::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update(b"|");
    }
    format!("rel:{}", &h.finalize().to_hex()[..12])
}

pub fn evidence_id(path: &str, kind: &str, symbol: &str, line: u32) -> String {
    let mut h = blake3::Hasher::new();
    h.update(path.as_bytes());
    h.update(kind.as_bytes());
    h.update(symbol.as_bytes());
    h.update(line.to_string().as_bytes());
    format!("evidence:{}", &h.finalize().to_hex()[..12])
}

pub struct Writer<'a> {
    pub store: &'a Store,
    pub repo_id: &'a str,
    pub revision: &'a str,
}

/// Per-file write result for the indexer.
pub struct WrittenFile {
    pub symbol_ids: Vec<(String, String)>, // (name, entity id)
    pub resolved_calls: Vec<ResolvedCall>,
    pub routes: Vec<Route>,
    pub tests: Vec<Test>,
    pub store_refs: Vec<StoreRef>,
}

impl<'a> Writer<'a> {
    pub fn new(store: &'a Store, repo_id: &'a str, revision: &'a str) -> Self {
        Writer { store, repo_id, revision }
    }

    fn ev(&self, path: &str, kind: &str, symbol: &str, line: u32) -> Evidence {
        let mut e = Evidence::source(
            evidence_id(path, kind, symbol, line),
            path,
        );
        e.symbol = Some(symbol.to_string());
        e.start_line = Some(line);
        e.revision = Some(self.revision.to_string());
        e.extractor = Some("scc-native".to_string());
        e.extractor_version = Some(env!("CARGO_PKG_VERSION").to_string());
        let _ = kind;
        e
    }

    /// Write one source file's extraction into the store.
    pub fn write_source(
        &self,
        path: &str,
        hash: &str,
        ef: &ExtractedFile,
        resolved_imports: &[ResolvedImport],
        resolved_calls: &[ResolvedCall],
        _index: &SymbolIndex,
    ) -> Result<WrittenFile, scc_store::StoreError> {
        let file_id = entity_id(self.repo_id, kinds::FILE, path);
        // file entity
        let mut fe = scc_core::Entity::new(file_id.clone(), kinds::FILE, path.to_string());
        fe.attr("hash", serde_json::json!(hash));
        self.store.insert_entity(&fe, &[path.to_string()])?;

        // imports: store rows + file imports file / imports external_api edges
        type ImportRow = (String, Vec<(String, String)>, u32, String);
        let mut imports_sql: Vec<ImportRow> = Vec::new();
        for imp in &ef.imports {
            imports_sql.push((
                imp.module.clone(),
                imp.names.clone(),
                imp.line,
                match imp.r#type {
                    ImportType::Module => "module".to_string(),
                    ImportType::Member => "member".to_string(),
                },
            ));
        }
        for ri in resolved_imports {
            let (kind, key) = match &ri.target {
                crate::resolve::ImportTarget::Internal { file, .. } => {
                    (kinds::FILE, file.as_str())
                }
                crate::resolve::ImportTarget::External { name } => {
                    (kinds::EXTERNAL_API, name.as_str())
                }
            };
            let target_id = entity_id(self.repo_id, kind, key);
            // §26: native module-graph resolution is a deterministic
            // candidate (EXTRACTED), never RESOLVED — only compiler/LSP/SCIP
            // proof promotes an edge to RESOLVED.
            let provenance = match &ri.target {
                crate::resolve::ImportTarget::Internal { .. } => Provenance::Extracted,
                crate::resolve::ImportTarget::External { .. } => Provenance::Extracted,
            };
            let ev = self.ev(path, "import", &ri.module, ri.line);
            self.store.insert_evidence(&ev)?;
            let r = Relationship::new(
                rel_id(&["imports", &file_id, &target_id]),
                file_id.clone(),
                scc_core::predicates::IMPORTS,
                target_id,
                provenance,
            )
            .with_evidence(vec![ev.id.clone()]);
            self.store.insert_relationship(&r, path)?;
        }
        if !imports_sql.is_empty() {
            self.store.insert_imports(path, &imports_sql)?;
        }

        // symbol entities
        let mut written = WrittenFile {
            symbol_ids: Vec::new(),
            resolved_calls: resolved_calls.to_vec(),
            routes: ef.routes.clone(),
            tests: ef.tests.clone(),
            store_refs: ef.store_refs.clone(),
        };
        for sym in &ef.symbols {
            let id = scc_core::symbol_id(self.repo_id, path, &sym.name);
            let mut se = scc_core::Entity::new(id.clone(), kinds::SYMBOL, sym.name.clone());
            se.attr("kind", serde_json::json!(core_symbol_kind(sym.kind)));
            se.attr("file", serde_json::json!(path));
            if let Some(sig) = &sym.signature {
                se.attr("signature", serde_json::json!(sig));
            }
            se.attr("exported", serde_json::json!(sym.exported));
            se.attr("start_line", serde_json::json!(sym.start_line));
            se.attr("end_line", serde_json::json!(sym.end_line));
            if let Some(parent) = &sym.parent {
                se.attr("parent", serde_json::json!(parent));
            }
            if let Some(doc) = &sym.docstring {
                se.attr("docstring", serde_json::json!(truncate(doc, 240)));
            }
            let ev = self.ev(path, "symbol", &sym.name, sym.start_line);
            self.store.insert_evidence(&ev)?;
            se.evidence.push(ev.id.clone());
            // control-flow evidence (P1 §19): calls made inside conditional/
            // loop/try bodies are the only evidence that call fanout is a
            // branch. The FlowGraph compiler reads this attribute.
            let conditional: Vec<String> = ef
                .calls
                .iter()
                .filter(|c| c.conditional && c.caller.as_deref() == Some(sym.name.as_str()))
                .map(|c| c.callee.clone())
                .collect();
            if !conditional.is_empty() {
                let mut cond: Vec<String> = conditional;
                cond.sort();
                cond.dedup();
                se.attr("conditional_calls", serde_json::json!(cond));
            }
            // cli_flags: `--flag` list owned by the symbol (sorted, deduped
            // by the extractor).
            if let Some(flags) = ef.cli_flags.get(&sym.name) {
                if !flags.is_empty() {
                    se.attr("cli_flags", serde_json::json!(flags));
                }
            }
            self.store.insert_entity(&se, &[path.to_string()])?;
            self.store.insert_symbol(
                path,
                &sym.name,
                core_symbol_kind(sym.kind),
                sym.signature.as_deref(),
                sym.start_line,
                sym.end_line,
                sym.exported,
                sym.docstring.as_deref(),
            )?;
            written.symbol_ids.push((sym.name.clone(), id.clone()));

            // file contains symbol
            let rel = Relationship::new(
                rel_id(&["contains", &file_id, &id]),
                file_id.clone(),
                scc_core::predicates::CONTAINS,
                id,
                Provenance::Extracted,
            )
            .with_evidence(vec![ev.id.clone()]);
            self.store.insert_relationship(&rel, path)?;
        }

        // retries: attribute on symbol entity
        for r in &ef.retries {
            if let Some((_, id)) = written.symbol_ids.iter().find(|(n, _)| n == &r.symbol) {
                if let Some(mut se) = self.store.get_entity(id)? {
                    se.attributes.insert(
                        "retry_policy".into(),
                        serde_json::json!(r.policy),
                    );
                    let ev = self.ev(path, "retry", &r.symbol, r.line);
                    self.store.insert_evidence(&ev)?;
                    se.evidence.push(ev.id.clone());
                    self.store.insert_entity(&se, &[path.to_string()])?;
                }
            }
        }

        // entrypoints: attribute on symbol entity
        for e in &ef.entrypoints {
            let name = e.symbol.clone();
            let id = if let Some((_, id)) = written.symbol_ids.iter().find(|(n, _)| n == &name) {
                id.clone()
            } else {
                // entrypoint without a symbol (e.g. package.json bin) — file-level
                file_id.clone()
            };
            if let Some(mut se) = self.store.get_entity(&id)? {
                let mut eps: Vec<String> = se
                    .attributes
                    .get("entrypoints")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                eps.push(e.kind.clone());
                se.attributes.insert("entrypoints".into(), serde_json::json!(eps));
                let ev = self.ev(path, "entrypoint", &name, e.line);
                self.store.insert_evidence(&ev)?;
                se.evidence.push(ev.id.clone());
                self.store.insert_entity(&se, &[path.to_string()])?;
            }
        }

        // semantic facts (Wave 9): public exports, annotations, fields,
        // registrations, configuration, callbacks → entities + typed
        // relationships with the owning symbol's evidence
        let sym_evidence = |name: &str| -> Vec<String> {
            written
                .symbol_ids
                .iter()
                .find(|(n, _)| n == name)
                .and_then(|(_, id)| self.store.get_entity(id).ok().flatten())
                .map(|e| e.evidence)
                .unwrap_or_default()
        };
        for fact in &ef.facts {
            match fact {
                crate::model::SemanticFact::PublicExport { symbol, kind } => {
                    if let Some((_, sym_id)) =
                        written.symbol_ids.iter().find(|(n, _)| n == symbol)
                    {
                        let export_id =
                            entity_id(self.repo_id, scc_core::kinds::EXPORT, symbol);
                        let mut se = scc_core::Entity::new(
                            export_id.clone(),
                            scc_core::kinds::EXPORT,
                            symbol.clone(),
                        );
                        se.attr("kind", serde_json::json!(kind));
                        se.evidence = sym_evidence(symbol);
                        self.store.insert_entity(&se, &[path.to_string()])?;
                        let rel = Relationship::new(
                            rel_id(&["exports", sym_id, &export_id]),
                            sym_id.clone(),
                            scc_core::predicates::EXPORTS,
                            export_id,
                            Provenance::Extracted,
                        );
                        self.store.insert_relationship(&rel, path)?;
                    }
                }
                crate::model::SemanticFact::Annotation { name, target } => {
                    if let Some((_, target_id)) =
                        written.symbol_ids.iter().find(|(n, _)| n == target)
                    {
                        let ann_id =
                            entity_id(self.repo_id, scc_core::kinds::ANNOTATION, name);
                        let se = scc_core::Entity::new(
                            ann_id.clone(),
                            scc_core::kinds::ANNOTATION,
                            name.clone(),
                        );
                        self.store.insert_entity(&se, &[path.to_string()])?;
                        let rel = Relationship::new(
                            rel_id(&["annotates", &ann_id, target_id]),
                            ann_id,
                            scc_core::predicates::ANNOTATES,
                            target_id.clone(),
                            Provenance::Extracted,
                        );
                        self.store.insert_relationship(&rel, path)?;
                    }
                }
                crate::model::SemanticFact::Field {
                    owner,
                    name,
                    mutable,
                } => {
                    if let Some((_, owner_id)) =
                        written.symbol_ids.iter().find(|(n, _)| n == owner)
                    {
                        let field_id = entity_id(
                            self.repo_id,
                            scc_core::kinds::FIELD,
                            &format!("{owner}.{name}"),
                        );
                        let mut se = scc_core::Entity::new(
                            field_id.clone(),
                            scc_core::kinds::FIELD,
                            format!("{owner}.{name}"),
                        );
                        se.attr("mutable", serde_json::json!(mutable));
                        se.attr("owner", serde_json::json!(owner));
                        se.evidence = sym_evidence(owner);
                        self.store.insert_entity(&se, &[path.to_string()])?;
                        let rel = Relationship::new(
                            rel_id(&["contains", owner_id, &field_id]),
                            owner_id.clone(),
                            scc_core::predicates::CONTAINS,
                            field_id,
                            Provenance::Extracted,
                        );
                        self.store.insert_relationship(&rel, path)?;
                    }
                }
                crate::model::SemanticFact::Registration {
                    owner,
                    kind,
                    target,
                } => {
                    if let Some((_, owner_id)) =
                        written.symbol_ids.iter().find(|(n, _)| n == owner)
                    {
                        let target_id = match written
                            .symbol_ids
                            .iter()
                            .find(|(n, _)| n == target)
                            .map(|(_, id)| id.clone())
                        {
                            Some(id) => id,
                            None => {
                                // registered surfaces are contract entities
                                // themselves (route/middleware/event targets);
                                // insert them so no dangling object survives
                                let id = entity_id(
                                    self.repo_id,
                                    scc_core::kinds::CONTRACT,
                                    target,
                                );
                                let mut ce = scc_core::Entity::new(
                                    id.clone(),
                                    scc_core::kinds::CONTRACT,
                                    target.clone(),
                                );
                                ce.attr("kind", serde_json::json!(kind));
                                ce.attr("file", serde_json::json!(path));
                                ce.evidence = sym_evidence(owner);
                                self.store.insert_entity(&ce, &[path.to_string()])?;
                                id
                            }
                        };
                        let rel = Relationship::new(
                            rel_id(&["registers", owner_id, &target_id]),
                            owner_id.clone(),
                            scc_core::predicates::REGISTERS,
                            target_id,
                            Provenance::Extracted,
                        );
                        self.store.insert_relationship(&rel, path)?;
                    }
                }
                crate::model::SemanticFact::Configuration { owner, key } => {
                    if let Some((_, owner_id)) =
                        written.symbol_ids.iter().find(|(n, _)| n == owner)
                    {
                        let cfg_id = entity_id(
                            self.repo_id,
                            scc_core::kinds::CONFIGURATION,
                            key,
                        );
                        let se = scc_core::Entity::new(
                            cfg_id.clone(),
                            scc_core::kinds::CONFIGURATION,
                            key.clone(),
                        );
                        self.store.insert_entity(&se, &[path.to_string()])?;
                        let rel = Relationship::new(
                            rel_id(&["configured_by", &cfg_id, owner_id]),
                            cfg_id,
                            scc_core::predicates::CONFIGURED_BY,
                            owner_id.clone(),
                            Provenance::Extracted,
                        );
                        self.store.insert_relationship(&rel, path)?;
                    }
                }
                crate::model::SemanticFact::Callback { owner, callback } => {
                    if let Some((_, owner_id)) =
                        written.symbol_ids.iter().find(|(n, _)| n == owner)
                    {
                        let cb_id = written
                            .symbol_ids
                            .iter()
                            .find(|(n, _)| n == callback)
                            .map(|(_, id)| id.clone())
                            .unwrap_or_else(|| {
                                entity_id(self.repo_id, scc_core::kinds::SYMBOL, callback)
                            });
                        let rel = Relationship::new(
                            rel_id(&["handles_callback", owner_id, &cb_id]),
                            owner_id.clone(),
                            scc_core::predicates::HANDLES_CALLBACK,
                            cb_id,
                            Provenance::Extracted,
                        );
                        self.store.insert_relationship(&rel, path)?;
                    }
                }
            }
        }

        // resolved calls
        let symbol_by_name: std::collections::HashMap<&str, &str> = written
            .symbol_ids
            .iter()
            .map(|(n, id)| (n.as_str(), id.as_str()))
            .collect();
        for rc in resolved_calls {
            let Some(callee_id) = &rc.callee_id else { continue };
            let _ = symbol_by_name; // caller ids come from resolve
            let ev = self.ev(path, "call", &rc.callee_name, rc.line);
            self.store.insert_evidence(&ev)?;
            let rel = Relationship::new(
                rel_id(&["calls", &rc.caller_id, callee_id]),
                rc.caller_id.clone(),
                scc_core::predicates::CALLS,
                callee_id.clone(),
                rc.provenance,
            )
            .with_confidence(rc.confidence)
            .with_evidence(vec![ev.id.clone()]);
            self.store.insert_relationship(&rel, path)?;
        }

        // routes
        for route in &ef.routes {
            let route_id = entity_id(self.repo_id, kinds::ROUTE, &format!("{} {}", route.method, route.path));
            let mut re = scc_core::Entity::new(route_id.clone(), kinds::ROUTE, format!("{} {}", route.method, route.path));
            re.attr("method", serde_json::json!(route.method));
            re.attr("path", serde_json::json!(route.path));
            re.attr("framework", serde_json::json!(route.framework));
            re.attr("file", serde_json::json!(path));
            if let Some(h) = &route.handler {
                let handler_id = scc_core::symbol_id(self.repo_id, path, h);
                re.attr("handler", serde_json::json!(handler_id));
            }
            let ev = self.ev(path, "route", &format!("{} {}", route.method, route.path), route.line);
            self.store.insert_evidence(&ev)?;
            re.evidence.push(ev.id.clone());
            self.store.insert_entity(&re, &[path.to_string()])?;
            // file contains route
            let rel = Relationship::new(
                rel_id(&["contains", &file_id, &route_id]),
                file_id.clone(),
                scc_core::predicates::CONTAINS,
                route_id.clone(),
                Provenance::Extracted,
            )
            .with_evidence(vec![ev.id.clone()]);
            self.store.insert_relationship(&rel, path)?;
            // handler handles route
            if let Some(h) = &route.handler {
                let handler_id = scc_core::symbol_id(self.repo_id, path, h);
                let rel = Relationship::new(
                    rel_id(&["handles", &handler_id, &route_id]),
                    handler_id,
                    scc_core::predicates::HANDLES,
                    route_id,
                    Provenance::Extracted,
                )
                .with_evidence(vec![ev.id.clone()]);
                self.store.insert_relationship(&rel, path)?;
            }
        }

        // tests
        for t in &ef.tests {
            let test_id = entity_id(
                self.repo_id,
                kinds::TEST,
                &format!("{}/{}", path, t.name),
            );
            let mut te = scc_core::Entity::new(test_id.clone(), kinds::TEST, t.name.clone());
            te.attr("file", serde_json::json!(path));
            te.attr("kind", serde_json::json!(match t.kind {
                TestKind::Unit => "unit",
                TestKind::Integration => "integration",
            }));
            if let Some(sym) = &t.symbol {
                te.attr("symbol", serde_json::json!(sym));
            }
            let ev = self.ev(path, "test", &t.name, t.line);
            self.store.insert_evidence(&ev)?;
            te.evidence.push(ev.id.clone());
            self.store.insert_entity(&te, &[path.to_string()])?;
            self.store.insert_test(
                &test_id,
                &t.name,
                path,
                match t.kind {
                    TestKind::Unit => "unit",
                    TestKind::Integration => "integration",
                },
                t.symbol.as_deref(),
            )?;
            // file contains test
            let rel = Relationship::new(
                rel_id(&["contains", &file_id, &test_id]),
                file_id.clone(),
                scc_core::predicates::CONTAINS,
                test_id.clone(),
                Provenance::Extracted,
            )
            .with_evidence(vec![ev.id.clone()]);
            self.store.insert_relationship(&rel, path)?;
        }

        // test → tested symbols (heuristic link to imported symbols)
        self.link_tests(path, ef, resolved_imports)?;

        // store refs
        self.write_store_refs(path, ef, &file_id)?;

        Ok(written)
    }

    /// Heuristically link tests to symbols they exercise: token-match test
    /// names against symbols in files the test file imports.
    pub fn link_tests(
        &self,
        path: &str,
        ef: &ExtractedFile,
        resolved_imports: &[ResolvedImport],
    ) -> Result<(), scc_store::StoreError> {
        let mut imported_files: Vec<String> = Vec::new();
        for ri in resolved_imports {
            if let crate::resolve::ImportTarget::Internal { file, .. } = &ri.target {
                if file != path {
                    imported_files.push(file.clone());
                }
            }
        }
        if imported_files.is_empty() {
            return Ok(());
        }
        let test_tokens: Vec<Vec<String>> = ef
            .tests
            .iter()
            .map(|t| {
                t.name
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|s| s.len() >= 3)
                    .map(|s| s.to_ascii_lowercase())
                    .collect()
            })
            .collect();
        let mut candidates: Vec<(String, String, String, f64)> = Vec::new();
        for ifile in &imported_files {
            for (_, name, kind, _sig, _, _, _, _) in self.store.symbols_in_file(ifile)? {
                if kind == "function" || kind == "class" || kind == "method" || kind == "const" {
                    let lower = name.to_ascii_lowercase();
                    for (i, tokens) in test_tokens.iter().enumerate() {
                        let exact = tokens.contains(&lower);
                        let prefix = tokens.iter().any(|t| common_prefix_len(t, &lower) >= 6);
                        if exact || prefix {
                            candidates.push((
                                ifile.clone(),
                                name.clone(),
                                ef.tests[i].name.clone(),
                                0.65,
                            ));
                            break;
                        }
                    }
                }
            }
        }
        for (ifile, sym_name, test_name, conf) in candidates {
            let sym_id = scc_core::symbol_id(self.repo_id, &ifile, &sym_name);
            let test_id = entity_id(self.repo_id, kinds::TEST, &format!("{}/{}", path, test_name));
            let rel = Relationship::new(
                rel_id(&["tested_by", &sym_id, &test_id]),
                sym_id.clone(),
                scc_core::predicates::TESTED_BY,
                test_id.clone(),
                Provenance::Extracted,
            )
            .with_confidence(conf);
            self.store.insert_relationship(&rel, path)?;
        }
        Ok(())
    }

    fn write_store_refs(
        &self,
        path: &str,
        ef: &ExtractedFile,
        file_id: &str,
    ) -> Result<(), scc_store::StoreError> {
        for sr in &ef.store_refs {
            let store_id = entity_id(self.repo_id, kinds::DATA_STORE, &sr.store);
            let mut se = scc_core::Entity::new(store_id.clone(), kinds::DATA_STORE, sr.store.clone());
            if let Some(tech) = &sr.technology {
                se.attr("technology", serde_json::json!(tech));
            }
            let ev = self.ev(path, "store", &sr.store, sr.line);
            self.store.insert_evidence(&ev)?;
            se.evidence.push(ev.id.clone());
            self.store.insert_entity(&se, &[path.to_string()])?;

            let caller_id = sr
                .caller
                .as_ref()
                .map(|c| scc_core::symbol_id(self.repo_id, path, c))
                .unwrap_or_else(|| file_id.to_string());

            let predicate = match sr.op {
                StoreOp::Read => scc_core::predicates::READS,
                StoreOp::Write => scc_core::predicates::WRITES,
                StoreOp::Query => scc_core::predicates::QUERIES,
                StoreOp::Publish => scc_core::predicates::PUBLISHES,
                StoreOp::Subscribe => scc_core::predicates::SUBSCRIBES,
                StoreOp::Migrate => scc_core::predicates::WRITES,
            };
            let rel = Relationship::new(
                rel_id(&["store", &caller_id, predicate, &store_id]),
                caller_id.clone(),
                predicate,
                store_id.clone(),
                Provenance::Extracted,
            )
            .with_confidence(0.8)
            .with_evidence(vec![ev.id.clone()]);
            self.store.insert_relationship(&rel, path)?;

            // data entity when a target is known (table/model/topic)
            if let Some(target) = &sr.target {
                match sr.op {
                    StoreOp::Publish | StoreOp::Subscribe => {
                        let topic_id = entity_id(self.repo_id, kinds::TOPIC, target);
                        let mut te = scc_core::Entity::new(topic_id.clone(), kinds::TOPIC, target.clone());
                        te.attr("store", serde_json::json!(sr.store));
                        self.store.insert_entity(&te, &[path.to_string()])?;
                        let rel = Relationship::new(
                            rel_id(&["topic", &caller_id, predicate, &topic_id]),
                            caller_id.clone(),
                            predicate,
                            topic_id,
                            Provenance::Extracted,
                        )
                        .with_evidence(vec![ev.id.clone()]);
                        self.store.insert_relationship(&rel, path)?;
                    }
                    _ => {
                        let data_id = entity_id(
                            self.repo_id,
                            kinds::DATA_ENTITY,
                            &format!("{}.{}", sr.store, target),
                        );
                        let mut de = scc_core::Entity::new(data_id.clone(), kinds::DATA_ENTITY, target.clone());
                        de.attr("store", serde_json::json!(sr.store));
                        self.store.insert_entity(&de, &[path.to_string()])?;
                        // store contains data
                        let rel = Relationship::new(
                            rel_id(&["contains", &store_id, &data_id]),
                            store_id.clone(),
                            scc_core::predicates::CONTAINS,
                            data_id,
                            Provenance::Extracted,
                        )
                        .with_evidence(vec![ev.id.clone()]);
                        self.store.insert_relationship(&rel, path)?;
                    }
                }
            }
        }
        Ok(())
    }
}

/// Length of the shared leading prefix of two lowercase strings.
fn common_prefix_len(a: &str, b: &str) -> usize {
    a.chars()
        .zip(b.chars())
        .take_while(|(x, y)| x == y)
        .count()
}

fn core_symbol_kind(k: SymbolKind) -> &'static str {
    match k {
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

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max).collect();
        format!("{t}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SemanticFact, Symbol};

    #[test]
    fn ids_are_deterministic() {
        assert_eq!(evidence_id("a.py", "call", "foo", 3), evidence_id("a.py", "call", "foo", 3));
        assert_ne!(evidence_id("a.py", "call", "foo", 3), evidence_id("a.py", "call", "foo", 4));
        assert_eq!(
            rel_id(&["calls", "x", "y"]),
            rel_id(&["calls", "x", "y"])
        );
    }

    #[test]
    fn truncate_works() {
        assert_eq!(truncate("short", 10), "short");
        assert!(truncate(&"x".repeat(300), 240).ends_with('…'));
    }

    #[test]
    fn semantic_facts_translate_to_entities_and_relationships() {
        // Wave 9: PublicExport + Field + Registration facts must produce
        // typed entities and relationships with the owning symbol's evidence.
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = scc_store::Store::open(&dir.path().join("scc.db"), &root).unwrap();

        let mut ef = ExtractedFile::default();
        ef.symbols.push(Symbol {
            name: "serve".into(),
            kind: SymbolKind::Function,
            signature: None,
            start_line: 1,
            end_line: 2,
            exported: true,
            docstring: None,
            parent: None,
        });
        ef.facts = vec![
            SemanticFact::PublicExport {
                symbol: "serve".into(),
                kind: "function".into(),
            },
            SemanticFact::Field {
                owner: "serve".into(),
                name: "port".into(),
                mutable: true,
            },
            SemanticFact::Registration {
                owner: "serve".into(),
                kind: "route".into(),
                target: "GET /ping".into(),
            },
        ];

        let writer = Writer::new(&store, "r", "rev");
        writer
            .write_source("main.py", "h", &ef, &[], &[], &SymbolIndex::new("r"))
            .unwrap();

        let exps = store.entities_by_kind(scc_core::kinds::EXPORT).unwrap();
        assert_eq!(exps.len(), 1, "{exps:?}");
        assert_eq!(exps[0].name, "serve");
        assert_eq!(
            exps[0].attributes["kind"],
            serde_json::json!("function"),
            "export kind recorded"
        );

        let fields = store.entities_by_kind(scc_core::kinds::FIELD).unwrap();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "serve.port");

        let contracts = store.entities_by_kind(scc_core::kinds::CONTRACT).unwrap();
        assert_eq!(
            contracts.len(),
            1,
            "registration target inserts a contract entity: {contracts:?}"
        );
        let rels = store.all_relationships().unwrap();
        assert!(
            rels.iter()
                .any(|r| r.predicate == scc_core::predicates::EXPORTS),
            "EXPORTS relationship: {rels:?}"
        );
        assert!(
            rels.iter()
                .any(|r| r.predicate == scc_core::predicates::REGISTERS),
            "REGISTERS relationship: {rels:?}"
        );
        assert!(
            rels.iter()
                .any(|r| r.predicate == scc_core::predicates::CONTAINS
                    && r.object.contains("serve.port")),
            "field CONTAINS relationship"
        );
    }
}
