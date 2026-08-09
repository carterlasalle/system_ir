//! Context Compiler (EPIC-060, docs/CONTEXT_COMPILER.md).
//!
//! Converts a large System IR into the smallest high-recall context pack for
//! an agent task. Packs are structured text with bounded token budgets,
//! evidence status, and warnings. STALE facts never enter trusted sections.

pub mod packs;
pub mod rank;

use scc_core::{estimate_tokens, truncate_to_budget};
use scc_graph::RealityGraph;
use scc_store::Store;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct ContextSettings {
    pub startup_tokens: usize,
    pub task_tokens: usize,
    pub include_low_confidence_inference: bool,
}

impl Default for ContextSettings {
    fn default() -> Self {
        ContextSettings {
            startup_tokens: 6000,
            task_tokens: 10000,
            include_low_confidence_inference: false,
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
            truncated: false,
            compression_policy: None,
        }
    }
}

pub struct ContextCompiler<'a> {
    pub store: &'a Store,
    pub graph: &'a RealityGraph,
    pub settings: ContextSettings,
    /// Repository-relative paths whose content hash no longer matches the
    /// indexed snapshot. Facts derived from them must be treated as STALE.
    pub stale_paths: Vec<String>,
}

impl<'a> ContextCompiler<'a> {
    pub fn new(
        store: &'a Store,
        graph: &'a RealityGraph,
        settings: ContextSettings,
        stale_paths: Vec<String>,
    ) -> Self {
        ContextCompiler {
            store,
            graph,
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
            if let Some(e) = self.graph.entities.get(id) {
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
        for r in self.graph.all_rels() {
            if entity_ids.contains(&r.subject)
                && !r.evidence.is_empty() {
                    let key = r.provenance.as_str().to_string();
                    *m.entry(key).or_insert(0) += 1;
                }
        }
        m
    }

    /// Apply the token budget to a pack: never cut invariants/ownership/
    /// failure content — those are always in the head sections.
    pub fn apply_budget(&mut self, pack: &mut ContextPack, budget: usize) {
        pack.budget = budget;
        pack.tokens = estimate_tokens(&pack.content);
        if pack.tokens <= budget {
            pack.truncated = false;
            return;
        }
        pack.truncated = true;
        pack.content = truncate_to_budget(&pack.content, budget);
        pack.tokens = estimate_tokens(&pack.content);
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
        let budget = token_budget.unwrap_or(self.settings.task_tokens).max(512);
        // Cache: keyed by (goal, inputs, budget, revision). The store clears
        // the cache on every index/refresh, so hits are only possible while
        // the model is current (docs/DATA_STRATEGY.md §6).
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
            format!("task:{}", &h.finalize().to_hex()[..20])
        };
        let revision = self.revision();
        if let Ok(Some(cached)) = self.store.cache_get(&key, &revision) {
            if let Ok(pack) = serde_json::from_str::<ContextPack>(&cached) {
                return pack;
            }
        }
        let pack = packs::task(self, goal, files, symbols, budget);
        if let Ok(json) = serde_json::to_string(&pack) {
            let _ = self.store.cache_put(&key, &json, &revision);
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
