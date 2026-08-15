//! Token-budget selection over ranked context candidates (Wave 14B).
//!
//! Three deterministic, no-panic selectors:
//! - [`select_with_budget`]: greedy value/token knapsack over
//!   [`ContextItem`]s; required items are always kept.
//! - [`mmr_diversify`]: Maximal Marginal Relevance — penalizes candidates
//!   similar to already-selected ones so one component/owner group cannot
//!   crowd out the rest.
//! - [`enforce_quotas`]: the spec's global surface budget (30% public/
//!   entrypoint, 25% core impl, 15% types/interfaces, 10% state owners,
//!   10% contract APIs, 10% flow-critical) as per-kind caps over a ranked
//!   list, preserving rank order.

use scc_core::ContextItem;
use std::collections::HashMap;

/// Select indices into `items` within a token `budget`. Required items come
/// first and are never dropped (even when they alone exceed the budget —
/// required context is never silently cut); the remaining budget is filled
/// greedily by value/token, highest first. Deterministic: ties break on
/// index order.
// trace:v1 id=impl.scc.selector work=WORK-SCC-014 satisfies=REQ-SCC-IR
pub fn select_with_budget(items: &[ContextItem], budget: usize) -> Vec<usize> {
    let mut selected: Vec<usize> = Vec::new();
    let mut spent: usize = 0;
    for (i, item) in items.iter().enumerate() {
        if item.required {
            selected.push(i);
            spent = spent.saturating_add(item.token_cost);
        }
    }
    let mut rest: Vec<usize> = (0..items.len()).filter(|i| !items[*i].required).collect();
    rest.sort_by(|a, b| {
        let va = items[*a].value / items[*a].token_cost.max(1) as f64;
        let vb = items[*b].value / items[*b].token_cost.max(1) as f64;
        vb.partial_cmp(&va)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.cmp(b))
    });
    for i in rest {
        let cost = items[i].token_cost;
        if spent.saturating_add(cost) <= budget {
            selected.push(i);
            spent += cost;
        }
    }
    selected
}

/// Maximal Marginal Relevance selection: repeatedly pick the candidate
/// maximizing `lambda * score - (1 - lambda) * max_similarity(selected)`,
/// up to `budget` items. `lambda` is clamped to [0, 1]; 0.5 is the default
/// (see [`mmr_diversify_default`]). Deterministic: ties keep the earlier
/// candidate in `ranked` order.
// trace:exempt reason=internal-detail
pub fn mmr_diversify(
    ranked: &[(String, f64)],
    similarity: impl Fn(&str, &str) -> f64,
    lambda: f64,
    budget: usize,
) -> Vec<String> {
    let n = ranked.len();
    let budget = budget.min(n);
    let lambda = lambda.clamp(0.0, 1.0);
    let mut selected: Vec<usize> = Vec::with_capacity(budget);
    let mut picked = vec![false; n];
    while selected.len() < budget {
        let mut best: Option<(usize, f64)> = None;
        for i in 0..n {
            if picked[i] {
                continue;
            }
            let (id, score) = &ranked[i];
            let mut sim = 0.0_f64;
            for &j in &selected {
                let s = similarity(id, &ranked[j].0);
                if s > sim {
                    sim = s;
                }
            }
            let v = lambda * score - (1.0 - lambda) * sim;
            if best.is_none_or(|(_, bv)| v > bv) {
                best = Some((i, v));
            }
        }
        match best {
            Some((i, _)) => {
                selected.push(i);
                picked[i] = true;
            }
            None => break,
        }
    }
    selected.into_iter().map(|i| ranked[i].0.clone()).collect()
}

/// [`mmr_diversify`] with the spec's default `lambda = 0.5`.
// trace:exempt reason=internal-detail
pub fn mmr_diversify_default(
    ranked: &[(String, f64)],
    similarity: impl Fn(&str, &str) -> f64,
    budget: usize,
) -> Vec<String> {
    mmr_diversify(ranked, similarity, 0.5, budget)
}

/// Enforce per-kind quotas over a ranked list, preserving rank order: a
/// candidate is kept only while its kind has quota left. `quotas` maps a
/// kind to its share of the ranked length (clamped to [0, 1], applied via
/// rounding); kinds without a quota entry are uncapped. This implements the
/// spec's global surface budget — 30% public/entrypoint, 25% core impl,
/// 15% types/interfaces, 10% state owners, 10% contract APIs, 10%
/// flow-critical (quota keys `public`, `core`, `types`, `state`,
/// `contract`, `flow`).
// trace:exempt reason=internal-detail
pub fn enforce_quotas(
    ranked: &[(String, f64)],
    kind_of: impl Fn(&str) -> &str,
    quotas: &[(String, f64)],
) -> Vec<String> {
    let n = ranked.len();
    let mut caps: HashMap<&str, usize> = HashMap::new();
    for (kind, frac) in quotas {
        caps.insert(kind.as_str(), (frac.clamp(0.0, 1.0) * n as f64).round() as usize);
    }
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut out: Vec<String> = Vec::new();
    for (id, _) in ranked {
        let k = kind_of(id);
        let take = match caps.get(k) {
            None => true,
            Some(&cap) => {
                let cnt = counts.entry(k).or_insert(0);
                if *cnt < cap {
                    *cnt += 1;
                    true
                } else {
                    false
                }
            }
        };
        if take {
            out.push(id.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

// trace:exempt reason=internal-detail
    fn item(id: &str, value: f64, token_cost: usize, required: bool) -> ContextItem {
        ContextItem {
            id: id.to_string(),
            value,
            token_cost,
            required,
            group: None,
        }
    }

    // ---- (g) budget selection: drops low-value, never drops required ----

    #[test]
// trace:exempt reason=internal-detail
    fn budget_selection_drops_low_value_and_keeps_required() {
        let items = vec![
            item("required", 0.1, 120, true),
            item("high", 0.9, 50, false),
            item("mid", 0.5, 50, false),
            item("low", 0.1, 50, false),
        ];
        // Budget 170: required (120) + highest value/token (high, 50).
        let sel = select_with_budget(&items, 170);
        assert_eq!(sel, vec![0, 1]);
        // Mid/low are dropped; required is present.
        assert!(sel.contains(&0));
        assert!(!sel.contains(&2) && !sel.contains(&3));

        // Budget exhausted by required alone: still never drops required.
        let sel = select_with_budget(&items, 50);
        assert_eq!(sel, vec![0]);

        // Budget 0: only required.
        let sel = select_with_budget(&items, 0);
        assert_eq!(sel, vec![0]);

        // Empty input.
        assert!(select_with_budget(&[], 100).is_empty());

        // No required: pure value/token greedy.
        let no_req = vec![item("a", 0.1, 100, false), item("b", 0.9, 10, false)];
        let sel = select_with_budget(&no_req, 100);
        assert_eq!(sel, vec![1]);
    }

    // ---- (e) MMR diversity: same-owner DTOs are capped ----

    #[test]
// trace:exempt reason=internal-detail
    fn mmr_caps_same_owner_dtos() {
        let ranked: Vec<(String, f64)> = (0..5)
            .map(|i| (format!("Order.dto{i}"), 0.9 - 0.1 * i as f64))
            .collect();
        // All five share the same owner group → similarity 1.0 between any
        // pair. With a small budget, at most 2 are selected.
        let same_owner = |a: &str, b: &str| -> f64 {
            if a.starts_with("Order.") && b.starts_with("Order.") {
                1.0
            } else {
                0.0
            }
        };
        let sel = mmr_diversify_default(&ranked, same_owner, 2);
        assert_eq!(sel.len(), 2);
        // The two highest-scoring survive (identical penalty).
        assert_eq!(sel[0], "Order.dto0");
        assert_eq!(sel[1], "Order.dto1");

        // Budget larger than the list returns everything.
        let all = mmr_diversify_default(&ranked, same_owner, 10);
        assert_eq!(all.len(), 5);
    }

    #[test]
// trace:exempt reason=internal-detail
    fn mmr_keeps_diverse_items() {
        let ranked = vec![
            ("a".to_string(), 0.9),
            ("b".to_string(), 0.8),
            ("c".to_string(), 0.7),
        ];
        // Distinct groups → similarity 0 → pure score order.
        let distinct = |_: &str, _: &str| 0.0;
        let sel = mmr_diversify_default(&ranked, distinct, 3);
        assert_eq!(sel, vec!["a".to_string(), "b".to_string(), "c".to_string()]);

        // Lambda 0 = pure diversity: second pick is the least similar to
        // the first (score order with all-zero similarity ties breaks to
        // the earliest).
        let sel = mmr_diversify(&ranked, distinct, 0.0, 3);
        assert_eq!(sel.len(), 3);

        // Empty ranked / zero budget.
        assert!(mmr_diversify_default(&[], distinct, 5).is_empty());
        assert!(mmr_diversify_default(&ranked, distinct, 0).is_empty());
    }

    // ---- (f) quota enforcement keeps type balance ----

    #[test]
// trace:exempt reason=internal-detail
    fn quotas_keep_type_balance() {
        // 60 ranked items; counts per kind deliberately over-represent the
        // first three kinds.
        let mut ranked: Vec<(String, f64)> = Vec::new();
        for i in 0..25 {
            ranked.push((format!("pub:{i}"), 1.0 - i as f64 / 100.0));
        }
        for i in 0..20 {
            ranked.push((format!("core:{i}"), 1.0 - i as f64 / 100.0));
        }
        for i in 0..12 {
            ranked.push((format!("types:{i}"), 1.0 - i as f64 / 100.0));
        }
        for i in 0..4 {
            ranked.push((format!("state:{i}"), 1.0 - i as f64 / 100.0));
        }
        for i in 0..4 {
            ranked.push((format!("contract:{i}"), 1.0 - i as f64 / 100.0));
        }
        for i in 0..4 {
            ranked.push((format!("flow:{i}"), 1.0 - i as f64 / 100.0));
        }
        assert_eq!(ranked.len(), 69);

        fn kind_of(id: &str) -> &str {
            if id.starts_with("pub:") {
                "public"
            } else if id.starts_with("core:") {
                "core"
            } else if id.starts_with("types:") {
                "types"
            } else if id.starts_with("state:") {
                "state"
            } else if id.starts_with("contract:") {
                "contract"
            } else if id.starts_with("flow:") {
                "flow"
            } else {
                "other"
            }
        }
        fn other_kind(_: &str) -> &str {
            "other"
        }
        let quotas = vec![
            ("public".to_string(), 0.30),
            ("core".to_string(), 0.25),
            ("types".to_string(), 0.15),
            ("state".to_string(), 0.10),
            ("contract".to_string(), 0.10),
            ("flow".to_string(), 0.10),
        ];
        let sel = enforce_quotas(&ranked, kind_of, &quotas);
        let count = |k: &str| sel.iter().filter(|id| kind_of(id.as_str()) == k).count();
        // Caps: 30%·69≈20.7→21, 25%·69≈17.3→17, 15%·69≈10.4→10,
        // 10%·69≈6.9→7 each; under-represented kinds keep their counts.
        assert_eq!(count("public"), 21);
        assert_eq!(count("core"), 17);
        assert_eq!(count("types"), 10);
        assert_eq!(count("state"), 4);
        assert_eq!(count("contract"), 4);
        assert_eq!(count("flow"), 4);
        assert_eq!(sel.len(), 60);

        // Rank order is preserved: the first accepted public item is the
        // first public item in the ranked list.
        assert_eq!(sel[0], "pub:0");
        assert_eq!(sel[17], "pub:17");
        assert_eq!(sel[21], "core:0");

        // Kinds without a quota are uncapped.
        let sel_other = enforce_quotas(
            &[("x".to_string(), 0.5), ("y".to_string(), 0.5)],
            other_kind,
            &[],
        );
        assert_eq!(sel_other.len(), 2);
    }
}
