//! Retrieval metrics for ranking-arm ablations.
//!
//! Pure functions. They do not choose a production ranker.

use std::collections::HashSet;

/// Recall@k: |gold ∩ top-k| / |gold|. Empty gold is vacuously 1.0.
// trace:v1 id=impl.scc.core.recall-at-k work=WORK-ripwire-lessons-phase2 satisfies=REQ-retrieval-eval
pub fn recall_at_k(ranked: &[String], gold: &HashSet<String>, k: usize) -> f64 {
    if gold.is_empty() {
        return 1.0;
    }
    if k == 0 || ranked.is_empty() {
        return 0.0;
    }
    let mut seen = HashSet::new();
    let mut hit = 0usize;
    for id in ranked.iter().take(k) {
        if !seen.insert(id) {
            continue;
        }
        if gold.contains(id) {
            hit += 1;
        }
    }
    hit as f64 / gold.len() as f64
}

/// Mean reciprocal rank of the first gold hit (1-based). Zero if none hit.
// trace:v1 id=impl.scc.core.mrr work=WORK-ripwire-lessons-phase2 satisfies=REQ-retrieval-eval
pub fn mean_reciprocal_rank(ranked: &[String], gold: &HashSet<String>) -> f64 {
    if gold.is_empty() {
        return 1.0;
    }
    let mut seen = HashSet::new();
    for (i, id) in ranked.iter().enumerate() {
        if !seen.insert(id) {
            continue;
        }
        if gold.contains(id) {
            return 1.0 / (i as f64 + 1.0);
        }
    }
    0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // trace:v1 id=test.scc.core.recall-mrr verifies=REQ-retrieval-eval exercises=impl.scc.core.recall-at-k,impl.scc.core.mrr
    fn recall_and_mrr_are_deterministic() {
        let ranked = vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ];
        let gold: HashSet<String> = ["b".into(), "d".into()].into_iter().collect();
        assert!((recall_at_k(&ranked, &gold, 1) - 0.0).abs() < f64::EPSILON);
        assert!((recall_at_k(&ranked, &gold, 2) - 0.5).abs() < f64::EPSILON);
        assert!((recall_at_k(&ranked, &gold, 4) - 1.0).abs() < f64::EPSILON);
        assert!((mean_reciprocal_rank(&ranked, &gold) - 0.5).abs() < f64::EPSILON);
        let empty: HashSet<String> = HashSet::new();
        assert_eq!(recall_at_k(&ranked, &empty, 5), 1.0);
        assert_eq!(mean_reciprocal_rank(&[], &gold), 0.0);
    }
}
