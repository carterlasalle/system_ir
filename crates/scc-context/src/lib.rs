//! Context Compiler (EPIC-060, docs/CONTEXT_COMPILER.md).
//!
//! Converts a large System IR into the smallest high-recall context pack for
//! an agent task. Packs are structured text with bounded token budgets,
//! evidence status, and warnings. STALE facts never enter trusted sections.

pub mod packs;
pub mod rank;

use scc_core::estimate_tokens;
use scc_graph::{RealityGraph, TrustedGraphView, TrustPolicy};
use scc_store::Store;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct ContextSettings {
    pub startup_tokens: usize,
    pub task_tokens: usize,
    pub include_low_confidence_inference: bool,
    /// Salt for the task-pack cache: derived from the active ranker
    /// configuration so enabling/disabling embeddings invalidates cached
    /// packs.
    pub rank_salt: String,
}

impl Default for ContextSettings {
    fn default() -> Self {
        ContextSettings {
            startup_tokens: 6000,
            task_tokens: 10000,
            include_low_confidence_inference: false,
            rank_salt: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPack {
    pub kind: String,
    pub repository_revision: String,
    pub content: String,
    #[serde(default)]
    pub entity_ids: Vec<String>,
    #[serde(default)]
    pub evidence_summary: BTreeMap<String, usize>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub tokens: usize,
    #[serde(default)]
    pub budget: usize,
    /// Token count of the full (untruncated) pack, before budget dropping.
    #[serde(default)]
    pub original_tokens: usize,
    /// Sections dropped for budget, by title (never critical sections).
    #[serde(default)]
    pub dropped_sections: Vec<String>,
    /// Content was cut mid-section (never happens for critical sections).
    #[serde(default)]
    pub hard_truncated: bool,
    /// The minimum safe pack exceeds the soft budget: content was NOT
    /// silently truncated; the pack is complete and over budget.
    #[serde(default)]
    pub exceeded_soft_budget: bool,
    /// True when the delivered content differs from the full pack
    /// (dropped sections, hard truncation, or budget overshoot).
    #[serde(default)]
    pub truncated: bool,
    /// RTK output-compression policy hints (docs/API_AND_INTEGRATIONS.md §11):
    /// what to preserve when compressing shell output for the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression_policy: Option<serde_json::Value>,
}

impl ContextPack {
    fn new(kind: &str, revision: &str) -> Self {
        ContextPack {
            kind: kind.to_string(),
            repository_revision: revision.to_string(),
            content: String::new(),
            entity_ids: Vec::new(),
            evidence_summary: BTreeMap::new(),
            warnings: Vec::new(),
            tokens: 0,
            budget: 0,
            original_tokens: 0,
            dropped_sections: Vec::new(),
            hard_truncated: false,
            exceeded_soft_budget: false,
            truncated: false,
            compression_policy: None,
        }
    }
}

pub struct ContextCompiler<'a> {
    pub store: &'a Store,
    /// Trusted view: the only way this compiler may query the reality graph.
    /// STALE facts are excluded and surfaced as warnings; INFERRED facts
    /// below the confidence floor are excluded unless explicitly allowed.
    pub view: TrustedGraphView<'a>,
    pub settings: ContextSettings,
    /// Repository-relative paths whose content hash no longer matches the
    /// indexed snapshot (mirrored from the view for cache-key hashing).
    pub stale_paths: Vec<String>,
}

impl<'a> ContextCompiler<'a> {
    pub fn new(
        store: &'a Store,
        graph: &'a RealityGraph,
        settings: ContextSettings,
        stale_paths: Vec<String>,
    ) -> Self {
        // The inferred-confidence floor is governed by
        // `include_low_confidence_inference`: low-confidence labeled
        // inference is excluded from trusted context by default.
        let floor = if settings.include_low_confidence_inference {
            0.0
        } else {
            0.85
        };
        let view = TrustedGraphView::new(
            graph,
            store,
            &stale_paths,
            TrustPolicy::default().with_inferred_floor(floor),
        );
        ContextCompiler {
            store,
            view,
            settings,
            stale_paths,
        }
    }

    pub fn revision(&self) -> String {
        self.store
            .latest_snapshot()
            .ok()
            .flatten()
            .map(|s| s.revision)
            .unwrap_or_else(|| "not-indexed".to_string())
    }

    pub fn is_stale_path(&self, path: &str) -> bool {
        self.stale_paths.iter().any(|p| p == path)
    }

    /// Provenance accounting for a set of entity ids.
    pub fn evidence_summary(&self, entity_ids: &[String]) -> BTreeMap<String, usize> {
        let mut m: BTreeMap<String, usize> = BTreeMap::new();
        for id in entity_ids {
            if let Some(e) = self.view.entity(id) {
                for ev_id in &e.evidence {
                    if let Some(ev) = self.store.get_evidence(ev_id).ok().flatten() {
                        let path = ev.path.clone().unwrap_or_default();
                        if self.is_stale_path(&path) {
                            *m.entry("STALE".into()).or_insert(0) += 1;
                            continue;
                        }
                        let et = match ev.r#type {
                            scc_core::EvidenceType::Source => "SOURCE",
                            scc_core::EvidenceType::Config => "CONFIG",
                            scc_core::EvidenceType::Runtime => "RUNTIME",
                            scc_core::EvidenceType::Test => "TEST",
                            scc_core::EvidenceType::Intent => "INTENT",
                            scc_core::EvidenceType::History => "HISTORY",
                        };
                        *m.entry(et.to_string()).or_insert(0) += 1;
                    }
                }
            }
        }
        for r in self.view.all_rels() {
            if entity_ids.contains(&r.subject)
                && !r.evidence.is_empty() {
                    let key = r.provenance.as_str().to_string();
                    *m.entry(key).or_insert(0) += 1;
                }
        }
        m
    }

    /// Apply the token budget to a pack (P0 rendering contract): the pack
    /// builder has already dropped lowest-priority sections; this records
    /// honest accounting. Critical content is never hard-truncated — if the
    /// minimum safe pack still exceeds the budget, the pack stays complete
    /// and `exceeded_soft_budget` is set instead of silently cutting facts.
    pub fn apply_budget(&mut self, pack: &mut ContextPack, budget: usize) {
        pack.budget = budget;
        if pack.original_tokens == 0 {
            pack.original_tokens = estimate_tokens(&pack.content);
        }
        pack.tokens = estimate_tokens(&pack.content);
        let over = pack.tokens > budget;
        pack.truncated = pack.hard_truncated || !pack.dropped_sections.is_empty() || over;
        if over && !pack.hard_truncated {
            pack.exceeded_soft_budget = true;
        }
    }

    // ---- top-level operations ----

    pub fn system_overview(&self) -> ContextPack {
        packs::overview(self)
    }

    pub fn task_context(
        &self,
        goal: &str,
        files: &[String],
        symbols: &[String],
        token_budget: Option<usize>,
    ) -> ContextPack {
        self.task_context_with_rankers(goal, files, symbols, token_budget, None, None)
    }

    /// `task_context` with optional semantic scorer and reranker (SCC-071).
    pub fn task_context_with_rankers(
        &self,
        goal: &str,
        files: &[String],
        symbols: &[String],
        token_budget: Option<usize>,
        scorer: Option<&dyn crate::rank::SemanticScorer>,
        reranker: Option<&dyn crate::rank::Reranker>,
    ) -> ContextPack {
        let budget = token_budget.unwrap_or(self.settings.task_tokens).max(512);
        // Cache (P0 trust contract): keyed by (goal, inputs, budget,
        // ranker salt) + the model epoch + the stale-path set. A file that
        // changed since indexing makes the key differ even without a
        // re-index, so a previously fresh pack can never be served after
        // its evidence is stale.
        let epoch = self.store.cache_epoch().unwrap_or_else(|_| "no-epoch".into());
        let key = {
            let mut h = blake3::Hasher::new();
            h.update(goal.as_bytes());
            for f in files {
                h.update(f.as_bytes());
            }
            for s in symbols {
                h.update(s.as_bytes());
            }
            h.update(budget.to_string().as_bytes());
            h.update(self.settings.rank_salt.as_bytes());
            h.update(epoch.as_bytes());
            let mut stale: Vec<&String> = self.stale_paths.iter().collect();
            stale.sort();
            for p in stale {
                h.update(p.as_bytes());
                h.update(b"\0");
            }
            format!("task:{}", &h.finalize().to_hex()[..20])
        };
        if let Ok(Some(cached)) = self.store.cache_get(&key, &epoch) {
            if let Ok(pack) = serde_json::from_str::<ContextPack>(&cached) {
                return pack;
            }
        }
        let pack =
            packs::task_with_rankers(self, goal, files, symbols, budget, scorer, reranker);
        if let Ok(json) = serde_json::to_string(&pack) {
            let _ = self.store.cache_put(&key, &json, &epoch);
        }
        pack
    }

    pub fn component_context(&self, id: &str) -> ContextPack {
        packs::component(self, id)
    }

    pub fn flow_context(&self, id: &str) -> ContextPack {
        packs::flow(self, id)
    }

    pub fn impact_context(
        &self,
        files: &[String],
        symbols: &[String],
        diff_base: Option<&str>,
    ) -> ContextPack {
        packs::impact(self, files, symbols, diff_base)
    }

    pub fn verify_context(&self) -> ContextPack {
        packs::verify(self)
    }
}
