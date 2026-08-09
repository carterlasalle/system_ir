//! Evidence importers (docs/EPICS_AND_TICKETS.md, docs/PRD.md §7): SCIP
//! (Sourcegraph code intelligence protocol) indexes and Narsil-CCG layered
//! graphs (the shape `scc export ccg` produces).
//!
//! Both importers are defensive: malformed entries are skipped and counted in
//! [`ImportReport::errors`]; only unreadable files or top-level JSON that is
//! not a JSON object return `Err`. All ids are content-derived and
//! deterministic (same input, same ids) using the same blake3 schemes as
//! `crate::write`.
#![allow(clippy::too_many_arguments)]

use crate::write::{evidence_id, rel_id};
use scc_core::kinds;
use scc_core::{
    entity_id, symbol_id, Entity, Evidence, EvidenceType, Provenance, Relationship,
};
use scc_store::Store;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub mod beads;
pub mod cbm;
pub mod context7;
pub mod gitnexus;
pub mod hindsight;

/// Adapter capability manifest (docs/SECURITY.md §6, SCC-224): every
/// evidence provider declares its filesystem scope, network access,
/// subprocess usage, and credential use. Third-party adapters must not
/// exceed their declared capabilities (SCC-225 sandbox policy).
#[derive(Debug, Clone, serde::Serialize)]
pub struct AdapterManifest {
    pub name: &'static str,
    pub description: &'static str,
    /// "repo-read" | "repo-read-write" | "external-file"
    pub filesystem: &'static str,
    pub network: bool,
    pub subprocess: bool,
    pub credentials: bool,
}

/// All adapters and their declared capabilities. Native extractors and the
/// bundled importers are repo-read only; the LSP adapters spawn a local
/// language server (no network, no credentials).
pub fn adapter_manifests() -> Vec<AdapterManifest> {
    vec![
        AdapterManifest {
            name: "native",
            description: "built-in tree-sitter extractors (python/typescript/infra/config)",
            filesystem: "repo-read",
            network: false,
            subprocess: false,
            credentials: false,
        },
        AdapterManifest {
            name: "scip",
            description: "SCIP index importer",
            filesystem: "external-file",
            network: false,
            subprocess: false,
            credentials: false,
        },
        AdapterManifest {
            name: "ccg",
            description: "Narsil CCG importer",
            filesystem: "external-file",
            network: false,
            subprocess: false,
            credentials: false,
        },
        AdapterManifest {
            name: "gitnexus",
            description: "GitNexus evidence export importer",
            filesystem: "external-file",
            network: false,
            subprocess: false,
            credentials: false,
        },
        AdapterManifest {
            name: "lsp-pyright",
            description: "LSP definition resolution via pyright",
            filesystem: "repo-read",
            network: false,
            subprocess: true,
            credentials: false,
        },
        AdapterManifest {
            name: "lsp-tsserver",
            description: "LSP definition resolution via typescript-language-server",
            filesystem: "repo-read",
            network: false,
            subprocess: true,
            credentials: false,
        },
        AdapterManifest {
            name: "configrefs",
            description: "config-reference post-pass",
            filesystem: "repo-read",
            network: false,
            subprocess: false,
            credentials: false,
        },
        AdapterManifest {
            name: "failures",
            description: "failure-pattern post-pass (except/circuit/dlq)",
            filesystem: "repo-read",
            network: false,
            subprocess: false,
            credentials: false,
        },
        AdapterManifest {
            name: "runtime",
            description: "OpenTelemetry trace ingestion",
            filesystem: "repo-read",
            network: false,
            subprocess: false,
            credentials: false,
        },
    ]
}

/// Sandbox policy check (SCC-225): verify a manifest is within the allowed
/// default profile — no network, no credentials, subprocess only for
/// explicitly declared server adapters. Returns the violation list.
pub fn sandbox_violations(m: &AdapterManifest) -> Vec<String> {
    let mut out = Vec::new();
    if m.network {
        out.push("network access not allowed in the default profile".into());
    }
    if m.credentials {
        out.push("credential access not allowed in the default profile".into());
    }
    if m.subprocess && !(m.name.starts_with("lsp-")) {
        out.push("subprocess not allowed outside declared server adapters".into());
    }
    out
}

/// Aggregate result of one import run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub symbols: usize,
    pub calls: usize,
    pub imports: usize,
    pub errors: usize,
}

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

fn get_str<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn roles_of(v: &Value) -> u64 {
    v.get("symbol_roles").and_then(Value::as_u64).unwrap_or(0)
}

fn range_of(v: &Value) -> Option<(u64, u64)> {
    let a = v.get("range")?.as_array()?;
    if a.len() < 2 {
        return None;
    }
    Some((a[0].as_u64()?, a[1].as_u64()?))
}

/// SCIP definition flag: roles include DEFINITION (0x1), or any relationship
/// entry declares `is_definition: true`.
fn is_definition(v: &Value) -> bool {
    roles_of(v) & 1 != 0
        || v.get("relationships").and_then(Value::as_array).is_some_and(|rels| {
            rels.iter().any(|r| {
                r.get("is_definition")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
        })
}

/// Last moniker segment: the part after the final `#`, falling back to the
/// part after the final space, then the whole string.
fn short_name(symbol: &str) -> String {
    if let Some((_, tail)) = symbol.rsplit_once('#') {
        if !tail.is_empty() {
            return tail.to_string();
        }
    }
    if let Some((_, tail)) = symbol.rsplit_once(' ') {
        if !tail.is_empty() {
            return tail.to_string();
        }
    }
    symbol.to_string()
}

/// Join a JSON array of strings (e.g. `documentation`) into one docstring.
fn strings_field(v: &Value, key: &str) -> Option<String> {
    let parts: Vec<String> = v
        .get(key)
        .and_then(Value::as_array)?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

fn make_evidence(
    id: String,
    typ: EvidenceType,
    path: &str,
    symbol: Option<&str>,
    extractor: &str,
    version: Option<&str>,
) -> Evidence {
    Evidence {
        id,
        r#type: typ,
        path: Some(path.to_string()),
        symbol: symbol.map(str::to_string),
        start_line: None,
        end_line: None,
        revision: None,
        content_hash: None,
        extractor: Some(extractor.to_string()),
        extractor_version: version.map(str::to_string),
    }
}

fn scip_evidence(file: &str, kind: &str, symbol: &str, version: Option<&str>) -> Evidence {
    make_evidence(
        evidence_id(file, kind, symbol, 0),
        EvidenceType::Source,
        file,
        Some(symbol),
        "scip",
        version,
    )
}

fn ccg_evidence(file: &str, kind: &str, symbol: &str, typ: EvidenceType) -> Evidence {
    make_evidence(
        evidence_id(file, kind, symbol, 0),
        typ,
        file,
        Some(symbol),
        "ccg",
        None,
    )
}

/// Create (once per unique id) an `external_api` entity for an unresolved
/// referenced symbol; returns the entity id.
fn ensure_external_api(
    store: &Store,
    repo: &str,
    name: &str,
    source: &str,
    extractor: &str,
    ev_kind: &str,
    version: Option<&str>,
    created: &mut HashSet<String>,
) -> Result<String, String> {
    let id = entity_id(repo, kinds::EXTERNAL_API, name);
    if created.insert(id.clone()) {
        let mut e = Entity::new(id.clone(), kinds::EXTERNAL_API, name.to_string());
        let ev = make_evidence(
            evidence_id(source, ev_kind, name, 0),
            EvidenceType::Source,
            source,
            Some(name),
            extractor,
            version,
        );
        store
            .insert_evidence(&ev)
            .map_err(|e| format!("{extractor}: evidence: {e}"))?;
        e.evidence.push(ev.id.clone());
        store
            .insert_entity(&e, &[source.to_string()])
            .map_err(|e| format!("{extractor}: entity: {e}"))?;
    }
    Ok(id)
}

/// Create (once per unique id) a symbol entity + `symbols` row + evidence for
/// a SCIP definition; returns the entity id and bumps `report.symbols` only
/// on first creation.
fn ensure_scip_symbol(
    store: &Store,
    repo: &str,
    file: &str,
    symbol: &str,
    docstring: Option<&str>,
    extractor_version: Option<&str>,
    created: &mut HashSet<String>,
    report: &mut ImportReport,
) -> Result<String, String> {
    let name = short_name(symbol);
    let id = symbol_id(repo, file, &name);
    if created.contains(&id) {
        return Ok(id);
    }
    let mut se = Entity::new(id.clone(), kinds::SYMBOL, name.clone());
    se.attr("kind", serde_json::json!("symbol"));
    se.attr("file", serde_json::json!(file));
    se.attr("scip", serde_json::json!(symbol));
    if let Some(doc) = docstring {
        se.attr("docstring", serde_json::json!(crate::write::truncate(doc, 240)));
    }
    let ev = scip_evidence(file, "symbol", &name, extractor_version);
    store
        .insert_evidence(&ev)
        .map_err(|e| format!("scip: evidence: {e}"))?;
    se.evidence.push(ev.id.clone());
    store
        .insert_entity(&se, &[file.to_string()])
        .map_err(|e| format!("scip: entity: {e}"))?;
    store
        .insert_symbol(file, &name, "function", None, 0, 0, true, docstring)
        .map_err(|e| format!("scip: symbol row: {e}"))?;
    created.insert(id.clone());
    report.symbols += 1;
    Ok(id)
}

// ---------------------------------------------------------------------------
// SCIP importer
// ---------------------------------------------------------------------------

/// A definition seen anywhere in the index: where it lives. Byte ranges are
/// tracked per document in `ScipDoc::defs` (containment is a same-document
/// relation); this map only resolves a symbol string to its canonical
/// definition site.
struct ScipDef {
    file: String,
    entity: String,
}

/// Per-document state gathered in pass 1, resolved in pass 2.
struct ScipDoc {
    file: String,
    file_id: String,
    /// (symbol, range, entity id) — definition occurrences in this document.
    defs: Vec<(String, (u64, u64), String)>,
    /// (symbol, range) — reference/read occurrences in this document.
    refs: Vec<(String, (u64, u64))>,
    /// Referenced symbols from non-definition `symbols` entries (imports).
    imports: Vec<String>,
}

/// Import a SCIP index (`index.scip`, JSON form). See module docs for the
/// exact rules; malformed entries are skipped and counted, never fatal.
pub fn import_scip(store: &Store, path: &Path) -> Result<ImportReport, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("scip: cannot read {}: {e}", path.display()))?;
    let root: Value = serde_json::from_str(&text)
        .map_err(|e| format!("scip: invalid JSON in {}: {e}", path.display()))?;
    let root = root
        .as_object()
        .ok_or("scip: top-level JSON must be an object")?;

    let repo = &store.repo_id;
    let extractor_version = root
        .get("metadata")
        .and_then(|m| m.get("tool"))
        .and_then(|t| t.get("version"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let mut report = ImportReport::default();
    let mut created: HashSet<String> = HashSet::new();
    let mut defs: HashMap<String, ScipDef> = HashMap::new();
    let mut docs: Vec<ScipDoc> = Vec::new();
    let mut call_edges: HashSet<(String, String)> = HashSet::new();
    let mut import_edges: HashSet<(String, String)> = HashSet::new();
    let mut external_created: HashSet<String> = HashSet::new();
    let source = path.to_string_lossy().to_string();

    let documents = root
        .get("documents")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    // Pass 1: file entities, definitions, per-document refs/imports. The
    // global `defs` map is complete only after this pass, so import/call
    // resolution (pass 2) is order-independent.
    for docv in &documents {
        if docv.as_object().is_none() {
            report.errors += 1;
            continue;
        }
        let Some(file) = get_str(docv, "relative_path") else {
            report.errors += 1;
            continue;
        };
        let file_id = entity_id(repo, kinds::FILE, file);

        let mut fe = Entity::new(file_id.clone(), kinds::FILE, file.to_string());
        if let Some(language) = get_str(docv, "language") {
            fe.attr("language", serde_json::json!(language));
        }
        let fev = scip_evidence(file, "file", file, extractor_version.as_deref());
        store
            .insert_evidence(&fev)
            .map_err(|e| format!("scip: evidence: {e}"))?;
        fe.evidence.push(fev.id.clone());
        store
            .insert_entity(&fe, std::slice::from_ref(&source))
            .map_err(|e| format!("scip: entity: {e}"))?;

        let mut doc = ScipDoc {
            file: file.to_string(),
            file_id: file_id.clone(),
            defs: Vec::new(),
            refs: Vec::new(),
            imports: Vec::new(),
        };

        // symbols array: definitions declared here, import references
        // pointing elsewhere.
        if let Some(syms) = docv.get("symbols").and_then(Value::as_array) {
            for entry in syms {
                let Some(sym) = get_str(entry, "symbol") else {
                    report.errors += 1;
                    continue;
                };
                if is_definition(entry) {
                    let docstring = strings_field(entry, "documentation");
                    let entity = ensure_scip_symbol(
                        store,
                        repo,
                        file,
                        sym,
                        docstring.as_deref(),
                        extractor_version.as_deref(),
                        &mut created,
                        &mut report,
                    )?;
                    defs.entry(sym.to_string()).or_insert(ScipDef {
                        file: file.to_string(),
                        entity,
                    });
                } else if let Some(rels) = entry.get("relationships").and_then(Value::as_array) {
                    for rel in rels {
                        let Some(referenced) = get_str(rel, "symbol") else {
                            report.errors += 1;
                            continue;
                        };
                        let is_def_link = rel
                            .get("is_definition")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        if is_def_link || roles_of(rel) & 2 == 0 {
                            continue;
                        }
                        doc.imports.push(referenced.to_string());
                    }
                }
            }
        }

        // occurrences: definitions (roles & 1) and reference/read roles.
        if let Some(occs) = docv.get("occurrences").and_then(Value::as_array) {
            for oc in occs {
                let Some(sym) = get_str(oc, "symbol") else {
                    report.errors += 1;
                    continue;
                };
                let roles = roles_of(oc);
                if roles & 1 != 0 {
                    let Some(range) = range_of(oc) else {
                        report.errors += 1;
                        continue;
                    };
                    let docstring = strings_field(oc, "override_documentation");
                    let entity = ensure_scip_symbol(
                        store,
                        repo,
                        file,
                        sym,
                        docstring.as_deref(),
                        extractor_version.as_deref(),
                        &mut created,
                        &mut report,
                    )?;
                    defs.entry(sym.to_string()).or_insert(ScipDef {
                        file: file.to_string(),
                        entity: entity.clone(),
                    });
                    doc.defs.push((sym.to_string(), range, entity));
                } else if roles & (2 | 4) != 0 {
                    let Some(range) = range_of(oc) else {
                        report.errors += 1;
                        continue;
                    };
                    doc.refs.push((sym.to_string(), range));
                }
                // other roles (0, forward-definition, ...) are ignored
            }
        }

        docs.push(doc);
    }

    // Pass 2: resolve imports and calls against the complete definition map.
    for doc in &docs {
        for referenced in &doc.imports {
            let Some(target) = defs.get(referenced) else {
                continue; // no definition anywhere: nothing to import from
            };
            if target.file == doc.file {
                continue; // same-document definition, not an import
            }
            let target_file_id = entity_id(repo, kinds::FILE, &target.file);
            if !import_edges.insert((doc.file_id.clone(), target_file_id.clone())) {
                continue;
            }
            let ev = scip_evidence(&doc.file, "import", referenced, extractor_version.as_deref());
            store
                .insert_evidence(&ev)
                .map_err(|e| format!("scip: evidence: {e}"))?;
            let rel = Relationship::new(
                rel_id(&["imports", &doc.file_id, &target_file_id]),
                doc.file_id.clone(),
                scc_core::predicates::IMPORTS,
                target_file_id,
                Provenance::Resolved,
            )
            .with_evidence(vec![ev.id.clone()]);
            store
                .insert_relationship(&rel, &doc.file)
                .map_err(|e| format!("scip: relationship: {e}"))?;
            report.imports += 1;
        }

        for (sym, range) in &doc.refs {
            let Some(target) = defs.get(sym) else {
                // No definition in the index: file -> external_api edge.
                let ext_id = ensure_external_api(
                    store,
                    repo,
                    sym,
                    &doc.file,
                    "scip",
                    "external",
                    extractor_version.as_deref(),
                    &mut external_created,
                )?;
                if !call_edges.insert((doc.file_id.clone(), ext_id.clone())) {
                    continue;
                }
                let ev = scip_evidence(&doc.file, "call", sym, extractor_version.as_deref());
                store
                    .insert_evidence(&ev)
                    .map_err(|e| format!("scip: evidence: {e}"))?;
                let rel = Relationship::new(
                    rel_id(&["calls", &doc.file_id, &ext_id]),
                    doc.file_id.clone(),
                    scc_core::predicates::CALLS,
                    ext_id,
                    Provenance::Extracted,
                )
                .with_confidence(0.8)
                .with_evidence(vec![ev.id.clone()]);
                store
                    .insert_relationship(&rel, &doc.file)
                    .map_err(|e| format!("scip: relationship: {e}"))?;
                report.calls += 1;
                continue;
            };
            // Subject: the innermost definition occurrence in this document
            // whose range contains the reference; fall back to the file.
            let subject = doc
                .defs
                .iter()
                .filter(|(_, (s, e), _)| *s <= range.0 && range.1 <= *e)
                .min_by_key(|(_, (s, e), _)| e - s)
                .map(|(_, _, id)| id.clone())
                .unwrap_or_else(|| doc.file_id.clone());
            if !call_edges.insert((subject.clone(), target.entity.clone())) {
                continue;
            }
            let ev = scip_evidence(&doc.file, "call", sym, extractor_version.as_deref());
            store
                .insert_evidence(&ev)
                .map_err(|e| format!("scip: evidence: {e}"))?;
            let rel = Relationship::new(
                rel_id(&["calls", &subject, &target.entity]),
                subject,
                scc_core::predicates::CALLS,
                target.entity.clone(),
                Provenance::Resolved,
            )
            .with_confidence(0.99)
            .with_evidence(vec![ev.id.clone()]);
            store
                .insert_relationship(&rel, &doc.file)
                .map_err(|e| format!("scip: relationship: {e}"))?;
            report.calls += 1;
        }
    }

    Ok(report)
}

// ---------------------------------------------------------------------------
// CCG importer
// ---------------------------------------------------------------------------

/// Import a Narsil-CCG layered graph (the shape `scc export ccg` produces):
/// L1 architecture entries become intent-evidenced entities, L2 symbols
/// become symbol entities + `symbols` rows, and `calls` (top-level field or
/// `attributes.calls`) become RESOLVED call edges resolved by id, then by
/// name, then to `external_api`.
pub fn import_ccg(store: &Store, path: &Path) -> Result<ImportReport, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("ccg: cannot read {}: {e}", path.display()))?;
    let root: Value = serde_json::from_str(&text)
        .map_err(|e| format!("ccg: invalid JSON in {}: {e}", path.display()))?;
    let root = root
        .as_object()
        .ok_or("ccg: top-level JSON must be an object")?;

    let repo = &store.repo_id;
    let mut report = ImportReport::default();
    let source = path.to_string_lossy().to_string();
    let layers = root
        .get("layers")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    // L1 architecture -> intent-evidenced entities.
    if let Some(arch) = layers
        .get("L1")
        .and_then(|l| l.get("architecture"))
        .and_then(Value::as_array)
    {
        for entry in arch {
            if entry.as_object().is_none() {
                report.errors += 1;
                continue;
            }
            let Some(name) = get_str(entry, "name") else {
                report.errors += 1;
                continue;
            };
            let kind = get_str(entry, "kind").unwrap_or(kinds::COMPONENT);
            let id = get_str(entry, "id")
                .map(str::to_string)
                .unwrap_or_else(|| entity_id(repo, kind, name));
            let mut e = Entity::new(id.clone(), kind, name.to_string());
            if let Some(attrs) = entry.get("attributes").and_then(Value::as_object) {
                for (k, v) in attrs {
                    e.attributes.insert(k.clone(), v.clone());
                }
            }
            let ev = ccg_evidence(name, "architecture", kind, EvidenceType::Intent);
            store
                .insert_evidence(&ev)
                .map_err(|e| format!("ccg: evidence: {e}"))?;
            e.evidence.push(ev.id.clone());
            store
                .insert_entity(&e, std::slice::from_ref(&source))
                .map_err(|e| format!("ccg: entity: {e}"))?;
        }
    }

    // L2 symbols: pass 1 imports all symbols so `calls` resolve regardless of
    // array order; pass 2 creates the call edges.
    let mut by_id: HashMap<String, String> = HashMap::new();
    let mut by_name: HashMap<String, String> = HashMap::new();
    let mut pending_calls: Vec<(String, Vec<String>)> = Vec::new();
    let mut call_edges: HashSet<(String, String)> = HashSet::new();
    let mut external_created: HashSet<String> = HashSet::new();

    if let Some(syms) = layers
        .get("L2")
        .and_then(|l| l.get("symbols"))
        .and_then(Value::as_array)
    {
        for entry in syms {
            if entry.as_object().is_none() {
                report.errors += 1;
                continue;
            }
            let Some(name) = get_str(entry, "name") else {
                report.errors += 1;
                continue;
            };
            let file = get_str(entry, "file").unwrap_or("");
            let kind = get_str(entry, "kind").unwrap_or("symbol");
            let id = get_str(entry, "id")
                .map(str::to_string)
                .unwrap_or_else(|| symbol_id(repo, file, name));
            let mut e = Entity::new(id.clone(), kinds::SYMBOL, name.to_string());
            e.attr("kind", serde_json::json!(kind));
            e.attr("file", serde_json::json!(file));
            let ev = ccg_evidence(file, "symbol", name, EvidenceType::Source);
            store
                .insert_evidence(&ev)
                .map_err(|e| format!("ccg: evidence: {e}"))?;
            e.evidence.push(ev.id.clone());
            store
                .insert_entity(&e, std::slice::from_ref(&source))
                .map_err(|e| format!("ccg: entity: {e}"))?;
            store
                .insert_symbol(file, name, kind, None, 0, 0, true, None)
                .map_err(|e| format!("ccg: symbol row: {e}"))?;
            by_id.entry(id.clone()).or_insert_with(|| id.clone());
            by_name.entry(name.to_string()).or_insert_with(|| id.clone());
            report.symbols += 1;

            let calls = entry
                .get("calls")
                .cloned()
                .or_else(|| entry.get("attributes").and_then(|a| a.get("calls")).cloned());
            if let Some(arr) = calls.and_then(|c| c.as_array().cloned()) {
                let total = arr.len();
                let callees: Vec<String> = arr
                    .into_iter()
                    .filter_map(|c| c.as_str().map(str::to_string))
                    .collect();
                report.errors += total - callees.len();
                if !callees.is_empty() {
                    pending_calls.push((id.clone(), callees));
                }
            }
        }

        for (subject, callees) in &pending_calls {
            for callee in callees {
                let object = match by_id
                    .get(callee)
                    .or_else(|| by_name.get(callee))
                    .cloned()
                {
                    Some(id) => id,
                    None => ensure_external_api(
                        store,
                        repo,
                        callee,
                        &source,
                        "ccg",
                        "call",
                        None,
                        &mut external_created,
                    )?,
                };
                if !call_edges.insert((subject.clone(), object.clone())) {
                    continue;
                }
                let ev = ccg_evidence(&source, "call", callee, EvidenceType::Source);
                store
                    .insert_evidence(&ev)
                    .map_err(|e| format!("ccg: evidence: {e}"))?;
                let rel = Relationship::new(
                    rel_id(&["calls", subject, &object]),
                    subject.clone(),
                    scc_core::predicates::CALLS,
                    object,
                    Provenance::Resolved,
                )
                .with_evidence(vec![ev.id.clone()]);
                store
                    .insert_relationship(&rel, &source)
                    .map_err(|e| format!("ccg: relationship: {e}"))?;
                report.calls += 1;
            }
        }
    }

    Ok(report)
}

#[cfg(test)]
mod manifest_tests {
    use super::*;

    #[test]
    fn all_manifests_are_within_default_profile() {
        let manifests = adapter_manifests();
        assert!(manifests.len() >= 8);
        for m in &manifests {
            let v = sandbox_violations(m);
            assert!(v.is_empty(), "{} violates sandbox: {v:?}", m.name);
        }
    }

    #[test]
    fn lsp_adapters_declare_subprocess() {
        let manifests = adapter_manifests();
        let m = manifests
            .iter()
            .find(|m| m.name == "lsp-pyright")
            .unwrap();
        assert!(m.subprocess);
        assert!(!m.network);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scc_core::predicates;
    use tempfile::TempDir;

    const SCIP_FIXTURE: &str = r#"{
      "metadata": {"version": 0, "tool": {"name": "scip-python", "version": "1.2.3"}},
      "documents": [
        {
          "language": "python",
          "relative_path": "a/util.py",
          "occurrences": [
            {"range": [0, 20], "symbol": "scip-python python pkg a util#helper", "symbol_roles": 1},
            {"range": [100, 130], "symbol": "scip-python python pkg a util#other", "symbol_roles": 1},
            {"range": [105, 125], "symbol": "scip-python python pkg b main#use_it", "symbol_roles": 2}
          ],
          "symbols": [
            {"symbol": "scip-python python pkg a util#helper", "documentation": ["Helper doc."],
             "relationships": [{"symbol": "scip-python python pkg a util#helper", "is_definition": true, "symbol_roles": 1, "signature": {}}]},
            {"symbol": "scip-python python pkg a util#other",
             "relationships": [{"symbol": "scip-python python pkg a util#other", "is_definition": true, "symbol_roles": 1}]}
          ],
          "diagnostics": []
        },
        {
          "language": "python",
          "relative_path": "b/main.py",
          "occurrences": [
            {"range": [0, 30], "symbol": "scip-python python pkg b main#use_it", "symbol_roles": 1},
            {"range": [5, 20], "symbol": "scip-python python pkg a util#helper", "symbol_roles": 2},
            {"range": [6, 19], "symbol": "scip-python python pkg a util#other", "symbol_roles": 4}
          ],
          "symbols": [
            {"symbol": "scip-python python pkg b main#use_it",
             "relationships": [{"symbol": "scip-python python pkg b main#use_it", "is_definition": true, "symbol_roles": 1}]},
            {"symbol": "scip-python python pkg a util#helper",
             "relationships": [{"symbol": "scip-python python pkg a util#helper", "is_definition": false, "symbol_roles": 2}]}
          ],
          "diagnostics": []
        }
      ]
    }"#;

    fn store() -> (Store, TempDir) {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&root.join("scc.db"), &root).unwrap();
        (store, dir)
    }

    fn write(dir: &TempDir, name: &str, text: &str) -> std::path::PathBuf {
        let p = dir.path().join(name);
        std::fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn scip_imports_definitions_calls_and_imports() {
        let (store, dir) = store();
        let p = write(&dir, "index.scip", SCIP_FIXTURE);

        let rep = import_scip(&store, &p).unwrap();
        assert_eq!(
            rep,
            ImportReport {
                symbols: 3,
                calls: 3,
                imports: 1,
                errors: 0
            }
        );

        let helper = symbol_id("repo", "a/util.py", "helper");
        let other = symbol_id("repo", "a/util.py", "other");
        let use_it = symbol_id("repo", "b/main.py", "use_it");

        // symbol entities with scip attribute and file attribute
        let se = store.get_entity(&use_it).unwrap().unwrap();
        assert_eq!(se.kind, kinds::SYMBOL);
        assert_eq!(
            se.attributes.get("scip").and_then(Value::as_str),
            Some("scip-python python pkg b main#use_it")
        );
        assert_eq!(
            se.attributes.get("file").and_then(Value::as_str),
            Some("b/main.py")
        );
        // docstring from symbols entry documentation
        let he = store.get_entity(&helper).unwrap().unwrap();
        assert_eq!(
            he.attributes.get("docstring").and_then(Value::as_str),
            Some("Helper doc.")
        );

        // symbols table rows, kind "function"
        let rows = store.symbols_in_file("b/main.py").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, "use_it");
        assert_eq!(rows[0].2, "function");

        // RESOLVED call edge use_it -> helper (reference inside def range)
        let calls = store
            .relationships_between(&use_it, predicates::CALLS, &helper)
            .unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].provenance, Provenance::Resolved);
        assert!((calls[0].confidence - 0.99).abs() < 1e-9);
        let ev = store.get_evidence(&calls[0].evidence[0]).unwrap().unwrap();
        assert_eq!(ev.extractor.as_deref(), Some("scip"));
        assert_eq!(ev.extractor_version.as_deref(), Some("1.2.3"));
        assert_eq!(ev.r#type, EvidenceType::Source);

        // read access (roles 4) also becomes a call: use_it -> other
        assert_eq!(
            store
                .relationships_between(&use_it, predicates::CALLS, &other)
                .unwrap()
                .len(),
            1
        );
        // cross-document reference inside other's range: other -> use_it
        assert_eq!(
            store
                .relationships_between(&other, predicates::CALLS, &use_it)
                .unwrap()
                .len(),
            1
        );

        // import edge: b/main.py imports a/util.py, RESOLVED
        let main_file = entity_id("repo", kinds::FILE, "b/main.py");
        let util_file = entity_id("repo", kinds::FILE, "a/util.py");
        let imports = store
            .relationships_between(&main_file, predicates::IMPORTS, &util_file)
            .unwrap();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].provenance, Provenance::Resolved);

        // deterministic ids: re-importing produces no new rows
        import_scip(&store, &p).unwrap();
        assert_eq!(store.all_relationships().unwrap().len(), 3 + 1);
    }

    #[test]
    fn scip_unresolved_references_become_external_api_edges() {
        let (store, dir) = store();
        let p = write(
            &dir,
            "external.scip",
            r#"{
              "metadata": {"tool": {"name": "scip-python", "version": "0.5"}},
              "documents": [
                {"language": "python", "relative_path": "x.py",
                 "occurrences": [
                   {"range": [0, 10], "symbol": "scip-python python pkg x#local_fn", "symbol_roles": 1},
                   {"range": [40, 55], "symbol": "scip-python python pkg nowhere#thing", "symbol_roles": 2},
                   {"range": [60, 70], "symbol": "scip-python python pkg x#local_fn", "symbol_roles": 2}
                 ],
                 "symbols": [{"symbol": "scip-python python pkg x#local_fn",
                              "relationships": [{"symbol": "scip-python python pkg x#local_fn", "is_definition": true, "symbol_roles": 1}]}],
                 "diagnostics": []}
              ]
            }"#,
        );

        let rep = import_scip(&store, &p).unwrap();
        assert_eq!(
            rep,
            ImportReport {
                symbols: 1,
                calls: 2,
                imports: 0,
                errors: 0
            }
        );

        // unknown symbol -> file -> external_api, EXTRACTED 0.8
        let ext = entity_id("repo", kinds::EXTERNAL_API, "scip-python python pkg nowhere#thing");
        assert!(store.get_entity(&ext).unwrap().is_some());
        let xfile = entity_id("repo", kinds::FILE, "x.py");
        let edges = store.relationships_between(&xfile, predicates::CALLS, &ext).unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].provenance, Provenance::Extracted);
        assert!((edges[0].confidence - 0.8).abs() < 1e-9);

        // reference to a defined symbol outside any definition range ->
        // subject falls back to the file entity
        let local = symbol_id("repo", "x.py", "local_fn");
        let edges = store
            .relationships_between(&xfile, predicates::CALLS, &local)
            .unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].provenance, Provenance::Resolved);
    }

    #[test]
    fn scip_malformed_input_never_panics() {
        let (store, dir) = store();
        let bad = write(&dir, "bad.json", "{not json");
        assert!(import_scip(&store, &bad).is_err());
        std::fs::write(&bad, "[]").unwrap();
        assert!(import_scip(&store, &bad).is_err());

        let p = write(
            &dir,
            "malformed.scip",
            r#"{
              "metadata": {"tool": {"name": "scip-python"}},
              "documents": [
                "not-an-object",
                {"language": "python", "occurrences": [{"range": [0, 5], "symbol": "scip-python python pkg m#a", "symbol_roles": 1}]},
                {"language": "python", "relative_path": "z.py",
                 "occurrences": [
                   "junk",
                   {"range": [0, 5], "symbol_roles": 1},
                   {"range": [0, 5], "symbol": "scip-python python pkg m#a"}
                 ],
                 "symbols": [
                   42,
                   {"relationships": []},
                   {"symbol": "scip-python python pkg m#b",
                    "relationships": [{"symbol": "scip-python python pkg m#b", "is_definition": false, "symbol_roles": 2}]}
                 ]}
              ]
            }"#,
        );

        let rep = import_scip(&store, &p).unwrap();
        // 1 non-object doc + 1 doc without relative_path + 1 junk occurrence
        // + 1 occurrence without symbol + 1 non-object symbol entry
        // + 1 symbol entry without symbol
        assert_eq!(rep.errors, 6);
        assert_eq!(rep.symbols, 0);
        assert_eq!(rep.calls, 0);
        assert_eq!(rep.imports, 0);
    }

    #[test]
    fn ccg_imports_architecture_symbols_and_calls() {
        let (store, dir) = store();
        let p = write(
            &dir,
            "graph.ccg",
            r#"{
              "schema": "ccg",
              "producer": "scc",
              "layers": {
                "L0": {"manifest": {"repository": "demo", "entity_count": 4}},
                "L1": {"architecture": [
                  {"id": "repo://demo/component/auth", "name": "auth", "kind": "component", "attributes": {"lang": "rust"}},
                  {"name": "payments", "kind": "service"}
                ]},
                "L2": {"symbols": [
                  {"name": "run", "kind": "function", "file": "src/main.rs", "calls": ["parse_args", "repo://demo/symbol/src/lib.rs/do_work"]},
                  {"name": "parse_args", "kind": "function", "file": "src/cli.rs"},
                  {"id": "repo://demo/symbol/src/lib.rs/do_work", "name": "do_work", "kind": "function", "file": "src/lib.rs"},
                  {"name": "mystery", "kind": "function", "file": "src/ghost.rs", "attributes": {"calls": ["ghost_fn"]}}
                ]}
              }
            }"#,
        );

        let rep = import_ccg(&store, &p).unwrap();
        assert_eq!(
            rep,
            ImportReport {
                symbols: 4,
                calls: 3,
                imports: 0,
                errors: 0
            }
        );

        // L1: given id used, attributes copied, evidence type intent
        let auth = store.get_entity("repo://demo/component/auth").unwrap().unwrap();
        assert_eq!(auth.kind, "component");
        assert_eq!(auth.attributes.get("lang").and_then(Value::as_str), Some("rust"));
        let ev = store.get_evidence(&auth.evidence[0]).unwrap().unwrap();
        assert_eq!(ev.r#type, EvidenceType::Intent);
        assert_eq!(ev.extractor.as_deref(), Some("ccg"));
        // L1: derived id when absent
        assert!(
            store
                .get_entity(&entity_id("repo", "service", "payments"))
                .unwrap()
                .is_some()
        );

        // L2: symbols table rows
        let rows = store.symbols_in_file("src/cli.rs").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, "parse_args");

        // calls resolved by name match
        let run = symbol_id("repo", "src/main.rs", "run");
        let parse_args = symbol_id("repo", "src/cli.rs", "parse_args");
        let edges = store
            .relationships_between(&run, predicates::CALLS, &parse_args)
            .unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].provenance, Provenance::Resolved);
        // calls resolved by id match
        let do_work = "repo://demo/symbol/src/lib.rs/do_work";
        assert_eq!(
            store.relationships_between(&run, predicates::CALLS, do_work).unwrap().len(),
            1
        );
        // unresolved callee (attributes.calls) -> external_api
        let mystery = symbol_id("repo", "src/ghost.rs", "mystery");
        let ghost = entity_id("repo", kinds::EXTERNAL_API, "ghost_fn");
        assert!(store.get_entity(&ghost).unwrap().is_some());
        let edges = store
            .relationships_between(&mystery, predicates::CALLS, &ghost)
            .unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].provenance, Provenance::Resolved);
    }

    #[test]
    fn ccg_malformed_input_never_panics() {
        let (store, dir) = store();
        let bad = write(&dir, "bad.ccg", "[1, 2]");
        assert!(import_ccg(&store, &bad).is_err());
        std::fs::write(&bad, "not json").unwrap();
        assert!(import_ccg(&store, &bad).is_err());

        let p = write(
            &dir,
            "malformed.ccg",
            r#"{
              "schema": "ccg",
              "layers": {
                "L1": {"architecture": [42, {"name": "ok", "kind": "component"}]},
                "L2": {"symbols": [
                  {"name": "a", "file": "a.py"},
                  "junk",
                  {"file": "b.py"},
                  {"name": "c", "file": "c.py", "calls": [7, "d"]}
                ]}
              }
            }"#,
        );

        let rep = import_ccg(&store, &p).unwrap();
        // 42 + "junk" + missing name + non-string callee
        assert_eq!(rep.errors, 4);
        assert_eq!(rep.symbols, 2);
        // "d" unresolved -> external_api call edge
        assert_eq!(rep.calls, 1);
    }
}
