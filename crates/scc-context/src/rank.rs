//! Candidate ranking (docs/CONTEXT_COMPILER.md §4–§6).
//!
//! Score = lexical overlap + FTS rank + graph/flow/ownership expansion.
//! Deterministic and dependency-free (no embeddings in MVP).

use scc_core::kinds;
use scc_graph::RealityGraph;
use scc_store::Store;
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub struct ScoredEntity {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub score: f64,
    /// Why this entity was selected (for evidence/debugging).
    pub reason: String,
}

/// Tokenize free text into lowercase alphanumeric terms (>= 3 chars, no
/// stopwords). Identifiers like `street-name` / `raw_text` split into their
/// pieces so goal terms match them.
pub fn terms(text: &str) -> BTreeSet<String> {
    const STOPWORDS: &[&str] = &[
        "the", "and", "for", "are", "was", "were", "with", "from", "into", "that", "this",
        "not", "but", "you", "our", "all", "can", "has", "had", "its", "who", "what",
    ];
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 3)
        .map(|t| t.to_ascii_lowercase())
        .filter(|t| !STOPWORDS.contains(&t.as_str()))
        .collect()
}

/// Light morphological variants of a term so goal words match identifier
/// stems (`normalization` → `normalize`, `transcripts` → `transcript`).
pub fn stem_variants(term: &str) -> Vec<String> {
    let mut out = vec![term.to_string()];
    if term.len() < 5 {
        return out;
    }
    if let Some(base) = term.strip_suffix("ation") {
        out.push(format!("{base}e")); // normalization -> normalize
    }
    if let Some(base) = term.strip_suffix("ations") {
        out.push(format!("{base}e")); // normalizations -> normalize
    }
    for suf in ["ing", "ings", "es", "ed", "s"] {
        if let Some(base) = term.strip_suffix(suf) {
            if base.len() >= 4 {
                out.push(base.to_string());
            }
            // drop doubled consonants: running -> run
            if base.len() >= 3 && base.ends_with(base.chars().last().unwrap()) {
                let trimmed: String = base.chars().take(base.len() - 1).collect();
                if trimmed.len() >= 3 {
                    out.push(trimmed);
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Prefix-aware term match (light stemming): `normalize` matches
/// `normalizes`/`normalizer`; `transcript` matches `transcripts`.
pub fn term_match(a: &str, b: &str) -> bool {
    if a.len() < 4 || b.len() < 4 {
        return a == b;
    }
    a.starts_with(b) || b.starts_with(a)
}

/// Lexical similarity of an entity to the goal terms: name matches weighted
/// double; prefix matches count.
pub fn entity_similarity(e: &scc_core::Entity, goal_terms: &BTreeSet<String>) -> f64 {
    if goal_terms.is_empty() {
        return 0.0;
    }
    let name_terms = terms(&e.name);
    let mut attr_text = String::new();
    for (k, v) in &e.attributes {
        if k == "docstring" || k == "signature" || k == "responsibility" || k == "path" {
            if let Some(s) = v.as_str() {
                attr_text.push_str(s);
                attr_text.push(' ');
            }
        }
    }
    let attr_terms = terms(&attr_text);
    let name_hits: usize = goal_terms
        .iter()
        .filter(|g| name_terms.iter().any(|n| term_match(g, n)))
        .count();
    let attr_hits: usize = goal_terms
        .iter()
        .filter(|g| attr_terms.iter().any(|n| term_match(g, n)))
        .count();
    (name_hits * 2 + attr_hits) as f64
}

/// Collect candidate entities for a task:
/// 1. lexical: FTS over entities + symbols
/// 2. explicit: named files/symbols
/// 3. graph expansion: containing component, flows, upstream/downstream,
///    ownership, invariants, tests — handled by the pack builder.
/// Pluggable semantic scorer (SCC-071): an optional provider (e.g. an
/// embedding model) contributes a relevance signal fused into the candidate
/// score. Providers are opt-in (`inference.enabled`); the default is the
/// lexical/graph ranker alone. The provider may label evidence, never invent
/// topology.
pub trait SemanticScorer: Send + Sync {
    fn score(&self, goal: &str, entity: &scc_core::Entity) -> f64;
}

pub fn collect_lexical_candidates(
    store: &Store,
    graph: &RealityGraph,
    goal: &str,
    symbols: &[String],
    limit: usize,
) -> Vec<ScoredEntity> {
    collect_lexical_candidates_with(store, graph, goal, symbols, limit, None)
}

/// `collect_lexical_candidates` with an optional semantic scorer fused in.
pub fn collect_lexical_candidates_with(
    store: &Store,
    graph: &RealityGraph,
    goal: &str,
    symbols: &[String],
    limit: usize,
    scorer: Option<&dyn SemanticScorer>,
) -> Vec<ScoredEntity> {
    let goal_terms = terms(goal);
    let sem = |e: &scc_core::Entity| -> f64 { scorer.map(|s| s.score(goal, e)).unwrap_or(0.0) };
    let mut out: Vec<ScoredEntity> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    let push = |e: &scc_core::Entity, base: f64, reason: &str,
                    out: &mut Vec<ScoredEntity>, seen: &mut BTreeSet<String>| {
        if !seen.insert(e.id.clone()) {
            return;
        }
        let sim = entity_similarity(e, &goal_terms);
        let score = base + sim + sem(e);
        out.push(ScoredEntity {
            id: e.id.clone(),
            kind: e.kind.clone(),
            name: e.name.clone(),
            score,
            reason: reason.to_string(),
        });
    };

    // FTS entities
    if !goal_terms.is_empty() {
        let joined: Vec<&str> = goal_terms.iter().map(|s| s.as_str()).collect();
        let query = joined.join(" ");
        if let Ok(hits) = store.search_entities(&query, limit) {
            let rank = limit as f64;
            for (i, e) in hits.iter().enumerate() {
                push(e, (rank - i as f64) / rank, "lexical", &mut out, &mut seen);
            }
        }
        if let Ok(hits) = store.search_symbols(&query, limit) {
            let rank = limit as f64;
            for (i, (name, sig, _kind, file)) in hits.iter().enumerate() {
                let id = scc_core::symbol_id(&graph.repo_id, file, name);
                let mut e = scc_core::Entity::new(id.clone(), kinds::SYMBOL, name.clone());
                e.attr("file", serde_json::json!(file));
                if !sig.is_empty() {
                    e.attr("signature", serde_json::json!(sig));
                }
                push(&e, (rank - i as f64) / rank, "lexical", &mut out, &mut seen);
            }
        }
        // substring fallback with stem variants (FTS prefix matching cannot
        // catch `normalization` → `normalize`)
        for term in &goal_terms {
            for variant in crate::rank::stem_variants(term) {
                if let Ok(hits) = store.search_entities_like(&variant, 6) {
                    for e in hits.iter() {
                        push(e, 0.6, "substring", &mut out, &mut seen);
                    }
                }
                if let Ok(hits) = store.search_symbols_like(&variant, 6) {
                    for (name, sig, _kind, file) in hits.iter() {
                        let id = scc_core::symbol_id(&graph.repo_id, file, name);
                        let mut e = scc_core::Entity::new(id.clone(), kinds::SYMBOL, name.clone());
                        e.attr("file", serde_json::json!(file));
                        if !sig.is_empty() {
                            e.attr("signature", serde_json::json!(sig));
                        }
                        push(&e, 0.6, "substring", &mut out, &mut seen);
                    }
                }
            }
        }
    }

    // semantic proposal pass: with a scorer present, entities with a
    // positive semantic signal surface even when no lexical term matches
    // (docs/CONTEXT_COMPILER.md §4 — embeddings are never truth, but they
    // may propose candidates)
    if scorer.is_some() {
        for e in graph.entities.values() {
            let sem_score = sem(e);
            if sem_score > 0.0 {
                push(e, 0.0, "semantic", &mut out, &mut seen);
            }
        }
    }

    // explicit symbols
    for s in symbols {
        let matches: Vec<String> = graph
            .entities_of_kind(kinds::SYMBOL)
            .into_iter()
            .filter(|e| e.name == *s)
            .map(|e| e.id.clone())
            .collect();
        for id in matches {
            if let Some(e) = graph.entities.get(&id) {
                push(e, 2.0, "explicit-symbol", &mut out, &mut seen);
            }
        }
        // fallback: entity id directly
        if graph.entities.contains_key(s) {
            push(graph.entities.get(s).unwrap(), 2.0, "explicit-id", &mut out, &mut seen);
        }
    }

    // graph expansion: callers (upstream) and callees (downstream) of
    // candidate symbols (docs/CONTEXT_COMPILER.md §6)
    let symbol_candidates: Vec<String> = out
        .iter()
        .filter(|c| c.kind == kinds::SYMBOL)
        .map(|c| c.id.clone())
        .collect();
    for sid in &symbol_candidates {
        for r in graph.in_pred(sid, "calls") {
            if let Some(e) = graph.entities.get(&r.subject) {
                push(e, 1.0, "upstream", &mut out, &mut seen);
            }
        }
        for r in graph.out_pred(sid, "calls") {
            if let Some(e) = graph.entities.get(&r.object) {
                push(e, 0.8, "downstream", &mut out, &mut seen);
            }
        }
    }

    out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(limit.max(20));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_tokenization() {
        let t = terms("change street-name normalization for transcripts");
        assert!(t.contains("street"));
        assert!(t.contains("name"));
        assert!(t.contains("transcripts"));
        assert!(!t.contains("for"));
        let t2 = terms("raw_text");
        assert!(t2.contains("raw"));
        assert!(t2.contains("text"));
    }

    #[test]
    fn prefix_match() {
        assert!(term_match("normalize", "normalizes"));
        assert!(term_match("transcript", "transcripts"));
        assert!(term_match("normalizer", "normalize"));
        assert!(!term_match("cat", "category"));
        assert!(!term_match("run", "running"));
    }

    struct BoostScorer;
    impl SemanticScorer for BoostScorer {
        fn score(&self, _goal: &str, e: &scc_core::Entity) -> f64 {
            if e.name.to_ascii_lowercase().contains("boost") {
                5.0
            } else {
                0.0
            }
        }
    }

    #[test]
    fn semantic_scorer_fuses_into_candidates() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = scc_store::Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let mut e1 = scc_core::Entity::new("repo://r/symbol/a.py/boosted", "symbol", "boosted");
        e1.attr("file", serde_json::json!("a.py"));
        store.insert_entity(&e1, &["a.py".into()]).unwrap();
        let mut e2 = scc_core::Entity::new("repo://r/symbol/a.py/plain", "symbol", "plain");
        e2.attr("file", serde_json::json!("a.py"));
        store.insert_entity(&e2, &["a.py".into()]).unwrap();
        let graph = scc_graph::RealityGraph::load(&store).unwrap();
        let plain = collect_lexical_candidates(&store, &graph, "nothing matches", &[], 10);
        assert_eq!(plain.len(), 0, "lexical-only finds nothing");
        let fused = collect_lexical_candidates_with(
            &store, &graph, "nothing matches", &[], 10, Some(&BoostScorer),
        );
        assert!(
            fused.iter().any(|c| c.name == "boosted" && c.score >= 5.0),
            "semantic signal must surface boosted: {fused:?}"
        );
        assert!(fused[0].name == "boosted", "boosted ranks first: {fused:?}");
    }

    #[test]
    fn similarity_weights_names() {
        let mut e = scc_core::Entity::new("repo://r/component/x", "component", "transcript-normalizer");
        e.attr("responsibility", serde_json::json!("normalizes radio transcripts"));
        let t = terms("normalize transcripts");
        let s = entity_similarity(&e, &t);
        assert!(s > 0.0, "got {s}");
        assert!(s >= 4.0, "expected name+attr hits, got {s}");
    }
}
