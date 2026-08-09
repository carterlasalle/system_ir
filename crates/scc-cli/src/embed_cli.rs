//! CLI wiring for the optional semantic ranker (SCC-071): an
//! `EmbeddingScorer` fusing stored entity embeddings via cosine similarity,
//! a `Reranker` calling a separate `/rerank` model, the `scc embed` command,
//! and the task-pack plumbing that activates when `inference.enabled` is set.

use scc_context::rank::{Reranker, ScoredEntity, SemanticScorer};
use scc_indexer::embed::{cosine, rerank, EmbedConfig, EMBED_KINDS};
use scc_store::Store;
use std::collections::HashMap;
use std::path::Path;

/// Fuses stored entity embeddings with the embedded goal. Vectors are
/// preloaded once per pack generation.
pub struct EmbeddingScorer {
    goal_vector: Vec<f32>,
    vectors: HashMap<String, Vec<f32>>,
}

impl EmbeddingScorer {
    pub fn new(goal: &str, cfg: &EmbedConfig, store: &Store) -> Result<EmbeddingScorer, String> {
        let vectors = scc_indexer::embed::embed_texts(cfg, &[goal])?;
        let goal_vector = vectors
            .into_iter()
            .next()
            .ok_or_else(|| "embedding request returned no vector".to_string())?;
        let mut map = HashMap::new();
        for kind in EMBED_KINDS {
            for e in store.entities_by_kind(kind).map_err(|e| e.to_string())? {
                if let Ok(Some((v, _))) = store.get_embedding(&e.id) {
                    map.insert(e.id, v);
                }
            }
        }
        Ok(EmbeddingScorer {
            goal_vector,
            vectors: map,
        })
    }
}

impl SemanticScorer for EmbeddingScorer {
    fn score(&self, _goal: &str, entity: &scc_core::Entity) -> f64 {
        match self.vectors.get(&entity.id) {
            Some(v) => cosine(&self.goal_vector, v),
            None => 0.0,
        }
    }
}

/// Second-stage reranker calling the configured `/rerank` model on the top
/// candidates. Any failure is a no-op (graceful degradation).
pub struct CliReranker {
    cfg: EmbedConfig,
}

impl CliReranker {
    pub fn new(cfg: &EmbedConfig) -> CliReranker {
        CliReranker { cfg: cfg.clone() }
    }
}

impl Reranker for CliReranker {
    fn rerank(&self, goal: &str, candidates: &mut Vec<ScoredEntity>) {
        if candidates.is_empty() || self.cfg.rerank_model.is_none() {
            return;
        }
        let docs: Vec<String> = candidates
            .iter()
            .take(30)
            .map(|c| format!("{} {}", c.kind, c.name))
            .collect();
        match rerank(&self.cfg, goal, &docs) {
            Ok(scores) => {
                for (c, s) in candidates.iter_mut().take(30).zip(scores.iter()) {
                    // rerank dominates; the lexical residue keeps ties
                    // deterministic
                    c.score = c.score * 0.2 + s * 5.0;
                    c.reason = format!("{} + rerank", c.reason);
                }
                candidates.sort_by(|a, b| {
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            Err(_) => {} // graceful degrade: keep lexical order
        }
    }
}

/// `scc embed` — compute and store embeddings for all embeddable entities.
pub fn cmd_embed(root: &Path) -> crate::Result<()> {
    let config = crate::load_config(root)?;
    if !config.inference.enabled {
        return Err(crate::CliError::Other(
            "inference is disabled — set `inference.enabled: true` in .scc/config.yaml".into(),
        ));
    }
    let store = crate::open_store(root)?;
    if store.snapshot_status()?.is_none() {
        return Err(crate::CliError::Other("not indexed — run `scc index` first".into()));
    }
    let cfg = EmbedConfig::from_config(&config.inference);
    println!(
        "embedding with model '{}' via {}",
        cfg.model, cfg.base_url
    );
    let n =
        scc_indexer::embed::embed_repository(&store, &cfg).map_err(crate::CliError::Other)?;
    store.cache_clear()?; // embeddings changed — drop cached packs
    println!("stored {n} embeddings");
    Ok(())
}

/// Build the scorer/reranker when inference is enabled; any provider failure
/// degrades to (None, None) so the lexical ranker is always the fallback.
pub fn rankers(
    store: &Store,
    config: &scc_indexer::Config,
    goal: &str,
) -> (Option<EmbeddingScorer>, Option<CliReranker>) {
    if !config.inference.enabled {
        return (None, None);
    }
    let cfg = EmbedConfig::from_config(&config.inference);
    let scorer = EmbeddingScorer::new(goal, &cfg, store).ok();
    let reranker = if cfg.rerank_model.is_some() {
        Some(CliReranker::new(&cfg))
    } else {
        None
    };
    (scorer, reranker)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scc_store::Store;

    fn tmp_store() -> (Store, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        (store, dir)
    }

    #[test]
    fn reranker_degrades_without_model() {
        let cfg = EmbedConfig {
            base_url: "http://127.0.0.1:1".into(),
            model: "m".into(),
            api_key: None,
            rerank_model: None,
        };
        let rr = CliReranker::new(&cfg);
        let mut cands = vec![ScoredEntity {
            id: "a".into(),
            kind: "symbol".into(),
            name: "x".into(),
            score: 1.0,
            reason: "lexical".into(),
        }];
        rr.rerank("goal", &mut cands);
        assert_eq!(cands.len(), 1); // no panic, no reorder
    }

    #[test]
    fn scorer_uses_stored_embeddings() {
        let (store, _d) = tmp_store();
        let mut e = scc_core::Entity::new("repo://r/symbol/a.py/boosted", "symbol", "boosted");
        e.attr("file", serde_json::json!("a.py"));
        store.insert_entity(&e, &["a.py".into()]).unwrap();
        // store a vector aligned with a goal vector [1,0,0...]
        let mut v = vec![0.0f32; 8];
        v[0] = 1.0;
        store.put_embedding(&e.id, &v, "test").unwrap();
        let cfg = EmbedConfig {
            base_url: "http://127.0.0.1:1".into(),
            model: "test".into(),
            api_key: None,
            rerank_model: None,
        };
        // scorer construction needs to embed the goal — bypass via a
        // hand-built scorer with a known goal vector
        let scorer = EmbeddingScorer {
            goal_vector: vec![1.0f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            vectors: {
                let mut m = HashMap::new();
                m.insert(e.id.clone(), v);
                m
            },
        };
        assert!((scorer.score("goal", &e) - 1.0).abs() < 1e-6);
        // unrelated entity scores 0
        let other = scc_core::Entity::new("repo://r/symbol/a.py/z", "symbol", "z");
        assert_eq!(scorer.score("goal", &other), 0.0);
    }
}
