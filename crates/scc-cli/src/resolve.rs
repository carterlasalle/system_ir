//! `scc resolve --lsp`: upgrade EXTRACTED call edges to RESOLVED using the
//! pyright language server (Phase 7, EPIC-120). Requires an existing index;
//! degrades gracefully when pyright is not installed (exit 0 with a hint).

use crate::open_store;
use scc_core::Provenance;
use std::collections::BTreeMap;
use std::path::Path;

/// Maximum call sites processed in one run.
pub const MAX_CALL_SITES: usize = 500;

/// `scc resolve --lsp`
///
/// Collects every source file holding `calls` relationships with provenance
/// `EXTRACTED`, resolves the call sites through pyright (batched per file),
/// and replaces each EXTRACTED edge with a RESOLVED edge to the true target
/// symbol. Prints a summary; the caller recompiles the derived layer after.
pub fn cmd_resolve_lsp(root: &Path) -> crate::Result<()> {
    let store = open_store(root)?;
    if store.latest_snapshot()?.is_none() {
        return Err(crate::CliError::Other(
            "no index found — run `scc index` before `scc resolve --lsp`".to_string(),
        ));
    }

    // Files with EXTRACTED calls edges, capped at MAX_CALL_SITES total sites.
    let all_rels = store.all_relationships()?;
    let mut files: BTreeMap<String, usize> = BTreeMap::new();
    let mut remaining = MAX_CALL_SITES;
    for (path, _h, _lang, _kind, _size) in store.all_files()? {
        let ids = store.relationship_ids_with_source(&path, scc_core::predicates::CALLS)?;
        if ids.is_empty() {
            continue;
        }
        let extracted = all_rels
            .iter()
            .filter(|r| {
                r.predicate == scc_core::predicates::CALLS
                    && r.provenance == Provenance::Extracted
                    && ids.contains(&r.id)
            })
            .count();
        if extracted > 0 {
            let take = extracted.min(remaining);
            files.insert(path, take);
            remaining -= take;
            if remaining == 0 {
                break;
            }
        }
    }

    if files.is_empty() {
        println!("no EXTRACTED call edges to resolve");
        return Ok(());
    }

    // Wave 4 §23 language-aware dispatch: .py -> pyright, TS/JS ->
    // typescript-language-server (tsserver). Each server is started only
    // when its language has files to resolve; missing tools degrade with a
    // hint instead of failing the whole run.
    let is_ts = |f: &str| {
        f.ends_with(".ts") || f.ends_with(".tsx") || f.ends_with(".js") || f.ends_with(".jsx")
    };
    let py_files: Vec<&String> = files.keys().filter(|f| f.ends_with(".py")).collect();
    let ts_files: Vec<&String> = files.keys().filter(|f| is_ts(f)).collect();

    let mut upgraded = 0usize;
    let mut unresolved = 0usize;
    let mut errors = 0usize;
    let mut fatal: Option<String> = None;

    if !py_files.is_empty() {
        let mut resolver = match scc_indexer::lsp::start_pyright(root) {
            Ok(r) => r,
            Err(e) if e.contains("pyright not found") => {
                println!("pyright not found — install with: npm install -g pyright");
                return Ok(());
            }
            Err(e) => return Err(crate::CliError::Other(e)),
        };
        for file in &py_files {
            match resolver.resolve_call_definitions(&store, file) {
                Ok(r) => {
                    upgraded += r.upgraded;
                    unresolved += r.unresolved;
                    errors += r.errors;
                }
                Err(e) => {
                    fatal = Some(e);
                    break;
                }
            }
        }
        drop(resolver);
    }

    if !ts_files.is_empty() {
        let mut resolver = match scc_indexer::lsp_ts::start_tsserver(root) {
            Ok(r) => r,
            Err(e) if e.contains("tsserver not found") => {
                println!(
                    "tsserver not found — install with: npm install -g typescript-language-server typescript"
                );
                return Ok(());
            }
            Err(e) => return Err(crate::CliError::Other(e)),
        };
        for file in &ts_files {
            match resolver.resolve_call_definitions(&store, file) {
                Ok(r) => {
                    upgraded += r.upgraded;
                    unresolved += r.unresolved;
                    errors += r.errors;
                }
                Err(e) => {
                    fatal = Some(e);
                    break;
                }
            }
        }
    }

    if let Some(e) = fatal {
        println!("lsp error: {e}");
    }
    println!("lsp: {upgraded} upgraded, {unresolved} unresolved, {errors} errors");
    Ok(())
}
