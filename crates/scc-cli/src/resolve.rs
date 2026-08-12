//! `scc resolve --lsp`: upgrade EXTRACTED call edges to RESOLVED through
//! every available semantic backend (Wave 4 §23): .py -> pyright, TS/JS ->
//! typescript-language-server. Requires an existing index; missing tools
//! degrade with a hint (exit 0).

use crate::open_store;
use std::path::Path;

pub const MAX_CALL_SITES: usize = scc_indexer::resolver::MAX_CALL_SITES;

/// `scc resolve --lsp` — delegates to the shared language-aware dispatcher.
pub fn cmd_resolve_lsp(root: &Path) -> crate::Result<()> {
    let store = open_store(root)?;
    if store.latest_snapshot()?.is_none() {
        return Err(crate::CliError::Other(
            "no index found — run `scc index` before `scc resolve --lsp`".to_string(),
        ));
    }
    let report =
        scc_indexer::resolver::resolve_repository(&store, root, MAX_CALL_SITES)
            .map_err(crate::CliError::Other)?;
    if report.remaining_candidates == 0 && report.backends_used.is_empty() {
        println!("no EXTRACTED call edges to resolve");
        return Ok(());
    }
    for missing in &report.backends_missing {
        println!("{missing} not found — install to resolve that language");
    }
    println!(
        "lsp ({}): {} upgraded, {} unresolved, {} errors; {} file(s) still candidate",
        report.backends_used.join("+"),
        report.upgraded,
        report.unresolved,
        report.errors,
        report.remaining_candidates
    );
    Ok(())
}
