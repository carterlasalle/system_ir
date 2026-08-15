//! Context ledger (Wave 14E): what the agent has already seen, persisted in
//! the store cache per model epoch. The ledger is the novelty-suppression
//! source for task deltas: already-visible information is not re-injected
//! unless it changed or is recontextualized (Aider's chat-file treatment,
//! generalized to symbols/files/components/flows).

use scc_core::ContextLedger;
use scc_store::Store;
use std::collections::BTreeSet;

const LEDGER_KEY_PREFIX: &str = "ledger:";

/// Loads/saves the per-epoch `scc_core::ContextLedger` in the store cache
/// (key `ledger:<epoch>`, serde JSON).
// trace:exempt reason=internal-detail
pub struct ContextLedgerStore<'a> {
    store: &'a Store,
    epoch: String,
}

// trace:exempt reason=internal-detail
impl<'a> ContextLedgerStore<'a> {
    /// Construct the ledger store pinned to the current model epoch.
    // trace:v1 id=impl.scc.context.ledger work=WORK-SCC-014 satisfies=REQ-SCC-IR
    pub fn new(store: &'a Store) -> Self {
        let epoch = store.cache_epoch().unwrap_or_else(|_| "no-epoch".into());
        ContextLedgerStore { store, epoch }
    }

    /// The model epoch this ledger is pinned to.
// trace:exempt reason=internal-detail
    pub fn epoch(&self) -> &str {
        &self.epoch
    }

// trace:exempt reason=internal-detail
    fn key(&self) -> String {
        format!("{LEDGER_KEY_PREFIX}{}", self.epoch)
    }

// trace:exempt reason=internal-detail
    fn empty(&self) -> ContextLedger {
        ContextLedger {
            model_epoch: self.epoch.clone(),
            ..Default::default()
        }
    }

    /// Load the visible set for this epoch. An absent or unreadable cache
    /// entry yields an empty ledger — never panics, never fabricates.
// trace:exempt reason=internal-detail
    pub fn load(&self) -> ContextLedger {
        match self.store.cache_get(&self.key(), &self.epoch) {
            Ok(Some(json)) => serde_json::from_str(&json).unwrap_or_else(|_| self.empty()),
            _ => self.empty(),
        }
    }

    /// Persist the ledger for this epoch (best-effort: cache failures never
    /// fail the caller).
// trace:exempt reason=internal-detail
    pub fn save(&self, ledger: &ContextLedger) {
        if let Ok(json) = serde_json::to_string(ledger) {
            let _ = self.store.cache_put(&self.key(), &json, &self.epoch);
        }
    }

    /// Kind-scoped id sets of everything visible this epoch:
    /// `(symbols, files, components, flows)`.
// trace:exempt reason=internal-detail
    pub fn visible_ids(
        &self,
    ) -> (BTreeSet<String>, BTreeSet<String>, BTreeSet<String>, BTreeSet<String>) {
        let led = self.load();
        (
            led.visible_symbols,
            led.visible_files,
            led.visible_components,
            led.visible_flows,
        )
    }

    /// Record what the agent actually saw this epoch. The caller MUST pass
    /// kind-scoped sets derived from *rendered* ids only (audit fix: budget-
    /// omitted candidates are never marked visible — the ledger describes
    /// the delivered artifact, not the candidate pool). Merges into the
    /// existing epoch ledger; best-effort persistence.
    // trace:v1 id=impl.scc.context.ledger.record-visible work=WORK-SCC-014 satisfies=REQ-SCC-IR
    pub fn record_visible(
        &self,
        symbols: &BTreeSet<String>,
        files: &BTreeSet<String>,
        components: &BTreeSet<String>,
        flows: &BTreeSet<String>,
    ) {
        let mut led = self.load();
        led.visible_symbols.extend(symbols.iter().cloned());
        led.visible_files.extend(files.iter().cloned());
        led.visible_components.extend(components.iter().cloned());
        led.visible_flows.extend(flows.iter().cloned());
        self.save(&led);
    }
}

/// Novelty penalty for one symbol: how much re-injection should cost when
/// the symbol was already shown this epoch.
///
/// Returns `0.1` when the symbol is already visible AND unchanged AND not a
/// critical anchor; `1.0` otherwise. Multiplied into an entry's importance,
/// the penalty sinks already-seen unchanged info below new/changed APIs in
/// the task-delta budget — the spec's "already-visible info not re-injected
/// unless changed/recontextualized". Critical anchors (ids recorded in the
/// ledger's component/flow sets) always re-inject at full weight: the task
/// delta never re-dumps the Atlas, and architecture anchors must stay
/// available to consumers that need them unconditionally.
///
/// Consumed by the one authoritative surface service
/// (`surface::build_surface` Task mode) via `SurfaceMode::Task { visible }`.
// trace:v1 id=impl.scc.context.ledger.novelty work=WORK-SCC-014 satisfies=REQ-SCC-IR
pub fn novelty_penalty(visible: &ContextLedger, symbol_id: &str, changed: bool) -> f64 {
    let already_visible = visible.visible_symbols.contains(symbol_id)
        || visible.visible_entities.contains(symbol_id);
    let critical = visible.visible_components.contains(symbol_id)
        || visible.visible_flows.contains(symbol_id);
    if already_visible && !changed && !critical {
        0.1
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

// trace:exempt reason=unit-test
    fn test_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        (dir, store)
    }

    #[test]
// trace:exempt reason=internal-detail
    fn novelty_penalty_suppresses_seen_unchanged_only() {
        let mut led = ContextLedger::default();
        led.visible_symbols.insert("repo://r/symbol/a.py/seen".into());

        // already visible + unchanged -> suppressed (0.1)
        assert_eq!(novelty_penalty(&led, "repo://r/symbol/a.py/seen", false), 0.1);
        // visible_entities counts as visible too
        led.visible_entities.insert("repo://r/symbol/b.py/ent".into());
        assert_eq!(novelty_penalty(&led, "repo://r/symbol/b.py/ent", false), 0.1);
        // never seen -> full weight (it is the new info the delta wants)
        assert_eq!(novelty_penalty(&led, "repo://r/symbol/c.py/new", false), 1.0);
        // seen but changed (recontextualized) -> full weight
        assert_eq!(novelty_penalty(&led, "repo://r/symbol/a.py/seen", true), 1.0);
    }

    #[test]
// trace:exempt reason=internal-detail
    fn novelty_penalty_never_suppresses_critical_anchors() {
        let mut led = ContextLedger::default();
        led.visible_symbols.insert("repo://r/symbol/a.py/arch".into());
        led.visible_components.insert("repo://r/symbol/a.py/arch".into());
        led.visible_flows.insert("repo://r/symbol/b.py/flow".into());
        // critical (component/flow anchor) even though seen + unchanged
        assert_eq!(novelty_penalty(&led, "repo://r/symbol/a.py/arch", false), 1.0);
        assert_eq!(novelty_penalty(&led, "repo://r/symbol/b.py/flow", false), 1.0);
    }

    #[test]
// trace:exempt reason=internal-detail
    fn ledger_roundtrip_via_store_cache() {
        let (_dir, store) = test_store();
        let ls = ContextLedgerStore::new(&store);
        // empty on first load
        let led = ls.load();
        assert_eq!(led.model_epoch, ls.epoch());
        assert!(led.visible_symbols.is_empty());

        let mut led = led;
        led.visible_symbols.insert("repo://r/symbol/a.py/x".into());
        led.visible_files.insert("a.py".into());
        led.visible_components.insert("repo://r/component/c".into());
        led.visible_flows.insert("repo://r/flow/f".into());
        led.last_task = Some("fix checkout".into());
        ls.save(&led);

        let ls2 = ContextLedgerStore::new(&store);
        let loaded = ls2.load();
        assert_eq!(loaded.visible_symbols, led.visible_symbols);
        assert_eq!(loaded.visible_files, led.visible_files);
        assert_eq!(loaded.visible_components, led.visible_components);
        assert_eq!(loaded.visible_flows, led.visible_flows);
        assert_eq!(loaded.last_task, led.last_task);

        let (syms, files, comps, flows) = ls2.visible_ids();
        assert_eq!(syms, led.visible_symbols);
        assert_eq!(files, led.visible_files);
        assert_eq!(comps, led.visible_components);
        assert_eq!(flows, led.visible_flows);
    }

    #[test]
// trace:exempt reason=internal-detail
    fn record_visible_persists_rendered_ids_only() {
        let (_dir, store) = test_store();
        let ls = ContextLedgerStore::new(&store);
        let mut syms = BTreeSet::new();
        syms.insert("repo://r/symbol/a.py/rendered".into());
        let mut comps = BTreeSet::new();
        comps.insert("repo://r/component/c".into());
        let mut flows = BTreeSet::new();
        flows.insert("repo://r/flow/f".into());

        ls.record_visible(&syms, &BTreeSet::new(), &comps, &flows);
        let led = ls.load();
        assert!(led.visible_symbols.contains("repo://r/symbol/a.py/rendered"));
        assert!(led.visible_components.contains("repo://r/component/c"));
        assert!(led.visible_flows.contains("repo://r/flow/f"));
        // only rendered ids were recorded — nothing else leaked in
        assert_eq!(led.visible_symbols.len(), 1);
        assert!(led.visible_files.is_empty());

        // A second recording merges (rendered ids accumulate per epoch).
        let mut more = BTreeSet::new();
        more.insert("repo://r/symbol/b.py/new".into());
        ls.record_visible(&more, &BTreeSet::new(), &BTreeSet::new(), &BTreeSet::new());
        let led = ls.load();
        assert!(led.visible_symbols.contains("repo://r/symbol/a.py/rendered"));
        assert!(led.visible_symbols.contains("repo://r/symbol/b.py/new"));
        assert_eq!(led.visible_symbols.len(), 2);
    }

    #[test]
// trace:exempt reason=internal-detail
    fn ledger_is_epoch_scoped() {
        let (_dir, store) = test_store();
        let e0 = store.cache_epoch().unwrap();
        let ls = ContextLedgerStore::new(&store);
        let mut led = ls.load();
        led.visible_symbols.insert("repo://r/symbol/a.py/x".into());
        ls.save(&led);
        assert!(ls.load().visible_symbols.contains("repo://r/symbol/a.py/x"));

        // a new epoch (re-index) starts a fresh ledger; the old one is
        // unreachable via the current epoch
        store.bump_epoch(scc_store::ModelEpochKind::Source).unwrap();
        let ls2 = ContextLedgerStore::new(&store);
        assert_ne!(ls2.epoch(), e0);
        assert!(ls2.load().visible_symbols.is_empty());
    }
}
