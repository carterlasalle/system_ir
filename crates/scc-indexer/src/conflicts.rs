//! Resolution conflict model (SCC-125): when an LSP upgrade moves a call
//! edge to a *different* target than the native index recorded, that
//! discrepancy is a signal worth keeping — the native resolver's view of
//! the call site disagreed with the language server's binding analysis.
//!
//! The CLI layer captures the EXTRACTED edges before running the resolver,
//! diffs them against the RESOLVED edges afterwards, and feeds the resulting
//! [`UpgradeRecord`]s into [`record_resolution_conflicts`]. Every record
//! whose target changed (old object was `external_api` or a different
//! symbol) becomes a `resolution_conflict` drift finding (severity `low`).
//!
//! The module is deliberately thin: [`classify`] is pure (no store access),
//! [`persist`] writes drift findings with exact-message deduplication, and
//! [`record_resolution_conflicts`] composes the two.

use scc_store::Store;
use std::collections::HashSet;

/// One LSP upgrade, as diffed by the CLI layer: a call site whose edge
/// changed target after resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradeRecord {
    /// Callee name as written at the call site (evidence `symbol`).
    pub callee: String,
    /// Native target before the upgrade (usually an `external_api` entity).
    pub old_object: String,
    /// LSP-resolved target after the upgrade (a symbol entity).
    pub new_object: String,
    /// Call-site line (1-based).
    pub line: u32,
}

/// Result of classifying upgrade records.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConflictReport {
    /// Number of upgrades that changed the target (old != new).
    pub conflicts: usize,
    /// One drift-finding message per conflict, ready to persist.
    pub records: Vec<String>,
}

/// Drift finding kind stamped on resolution conflicts.
pub const DRIFT_KIND: &str = "resolution_conflict";
/// Severity: native/LSP disagreement is informational, not a failure.
pub const DRIFT_SEVERITY: &str = "low";

/// Classify upgrade records into a conflict report (pure — no store access).
///
/// A record is a conflict when the upgrade *changed* the target
/// (`old_object != new_object`); an upgrade that merely re-confirmed the
/// native target is not a conflict. Messages follow the canonical shape
/// `<file>:<line> call to <callee> resolved by LSP to <new>; native index
/// had <old>`.
pub fn classify(file: &str, upgrades: &[UpgradeRecord]) -> ConflictReport {
    let mut report = ConflictReport::default();
    let mut seen = HashSet::new();
    for u in upgrades {
        if u.old_object == u.new_object {
            continue; // target unchanged — the resolver agreed with the native index
        }
        let msg = format!(
            "{}:{} call to {} resolved by LSP to {}; native index had {}",
            file, u.line, u.callee, u.new_object, u.old_object
        );
        if seen.insert(msg.clone()) {
            report.conflicts += 1;
            report.records.push(msg);
        }
    }
    report
}

/// Persist a conflict report as `resolution_conflict` drift findings.
///
/// Exact-duplicate messages already present in `drift_findings` are skipped,
/// so re-running resolution over the same upgrade never stacks duplicates.
pub fn persist(store: &Store, report: &ConflictReport, _file: &str) -> Result<(), String> {
    if report.records.is_empty() {
        return Ok(());
    }
    let existing: HashSet<String> = store
        .drift_findings(false)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(_, _, _, msg, _)| msg)
        .collect();
    for msg in &report.records {
        if existing.contains(msg) {
            continue;
        }
        store
            .add_drift_finding(DRIFT_KIND, DRIFT_SEVERITY, msg)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Classify upgrade records for one file and persist the conflicts.
pub fn record_resolution_conflicts(
    store: &Store,
    file: &str,
    upgrades: &[UpgradeRecord],
) -> Result<ConflictReport, String> {
    let report = classify(file, upgrades);
    persist(store, &report, file)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scc_store::Store;

    fn record(callee: &str, old: &str, new: &str, line: u32) -> UpgradeRecord {
        UpgradeRecord {
            callee: callee.to_string(),
            old_object: old.to_string(),
            new_object: new.to_string(),
            line,
        }
    }

    #[test]
    fn classify_counts_only_target_changes() {
        let ext = "repo://r/external_api/helper-pkg";
        let sym = "repo://r/symbol/lib/helper_pkg/impl.py/helper";
        let upgrades = vec![
            record("helper", ext, sym, 5),             // conflict: external -> symbol
            record("same", sym, sym, 9),               // agreement: unchanged
            record("other", "repo://r/external_api/x", "repo://r/external_api/y", 3), // conflict
        ];
        let report = classify("b.py", &upgrades);
        assert_eq!(report.conflicts, 2);
        assert_eq!(report.records.len(), 2);
        assert_eq!(
            report.records[0],
            "b.py:5 call to helper resolved by LSP to repo://r/symbol/lib/helper_pkg/impl.py/helper; native index had repo://r/external_api/helper-pkg"
        );
        // identical records dedupe within one report
        let dup = vec![
            record("helper", ext, sym, 5),
            record("helper", ext, sym, 5),
        ];
        assert_eq!(classify("b.py", &dup).conflicts, 1);
    }

    #[test]
    fn persist_writes_drift_findings_and_dedupes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("scc.db"), dir.path()).unwrap();
        let report = classify(
            "b.py",
            &[record(
                "helper",
                "repo://r/external_api/helper-pkg",
                "repo://r/symbol/lib/helper_pkg/impl.py/helper",
                5,
            )],
        );
        persist(&store, &report, "b.py").unwrap();

        let findings = store.drift_findings(false).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].1, DRIFT_KIND);
        assert_eq!(findings[0].2, DRIFT_SEVERITY);
        assert!(findings[0].3.contains("b.py:5 call to helper"));

        // second persist is a no-op (exact-duplicate message)
        persist(&store, &report, "b.py").unwrap();
        assert_eq!(store.drift_findings(false).unwrap().len(), 1);
    }

    #[test]
    fn record_resolution_conflicts_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("scc.db"), dir.path()).unwrap();
        let report = record_resolution_conflicts(
            &store,
            "src/main.ts",
            &[record(
                "helper",
                "repo://r/external_api/-app/util",
                "repo://r/symbol/lib/util/impl.ts/helper",
                4,
            )],
        )
        .unwrap();
        assert_eq!(report.conflicts, 1);
        let findings = store.drift_findings(false).unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].3.starts_with("src/main.ts:4 call to helper"));
        // empty reports persist nothing
        let report = record_resolution_conflicts(&store, "src/main.ts", &[]).unwrap();
        assert_eq!(report.conflicts, 0);
        assert_eq!(store.drift_findings(false).unwrap().len(), 1);
    }
}
