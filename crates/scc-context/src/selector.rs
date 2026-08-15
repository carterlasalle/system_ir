//! Token-budget selection over ranked context candidates (Wave 14B/15.1).
//!
//! Four deterministic, no-panic selectors:
//! - [`select_with_budget`]: greedy value/token knapsack over
//!   [`ContextItem`]s with a soft budget + hard maximum; required items
//!   are always kept (the caller compresses them when they alone exceed
//!   the hard max).
//! - [`select_in_order`]: the `optimizer`-off variant — rank-order cut,
//!   no value/token reordering.
//! - [`mmr_diversify`]: Maximal Marginal Relevance — penalizes candidates
//!   similar to already-selected ones so one component/owner group cannot
//!   crowd out the rest.
//! - [`enforce_quotas`]: TOKEN-aware per-kind caps (fraction × available
//!   tokens, adapted to the pool's running average token cost) over a
//!   ranked list, preserving rank order.

use scc_core::ContextItem;
use std::collections::HashMap;

/// Select indices into `items` within a token `budget`. Required items come
/// first and are never dropped (even when they alone exceed the budget —
/// required context is never silently cut); the remaining budget is filled
/// greedily by value/token, highest first. Deterministic: ties break on
/// index order.
///
/// `hard_max` is the absolute token ceiling (soft target + 20% by default):
/// required items may push the total over `budget` but the caller
/// structurally compresses them when they alone exceed `hard_max` (the
/// item costs then already reflect the compressed render); pool items are
/// only added while the total stays within both the soft budget and the
/// hard maximum.
// trace:v1 id=impl.scc.selector work=WORK-SCC-015 satisfies=REQ-SCC-IR
pub fn select_with_budget(items: &[ContextItem], budget: usize, hard_max: usize) -> Vec<usize> {
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
        let next = spent.saturating_add(cost);
        if next <= budget && next <= hard_max {
            selected.push(i);
            spent = next;
        }
    }
    selected
}

/// Budget selection in list order (the `optimizer`-off ablation of
/// [`select_with_budget`]): walk items in importance order, keep required
/// items unconditionally, and accept pool items while the total token
/// spend stays within both the soft `budget` and the `hard_max` ceiling.
/// No value/token reordering — a candidate's rank order decides.
// trace:v1 id=impl.scc.selector.select-in-order work=WORK-SCC-015 satisfies=REQ-SCC-IR
pub fn select_in_order(items: &[ContextItem], budget: usize, hard_max: usize) -> Vec<usize> {
    let mut selected: Vec<usize> = Vec::new();
    let mut spent: usize = 0;
    for (i, item) in items.iter().enumerate() {
        if item.required {
            selected.push(i);
            spent = spent.saturating_add(item.token_cost);
            continue;
        }
        let next = spent.saturating_add(item.token_cost);
        if next <= budget && next <= hard_max {
            selected.push(i);
            spent = next;
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

/// Enforce per-kind quotas over a ranked list, preserving rank order —
/// TOKEN-aware (reviewer item 6): each kind's cap is a token budget
/// `fraction * available_tokens`; a candidate is accepted only while
/// accepting it would not push its kind's accumulated spend over that
/// cap. Because the cap is a token budget applied over actual per-
/// candidate costs, the effective entry allowance adapts to the selected
/// pool's running average token cost (expensive candidates exhaust a
/// kind's allocation faster, cheap ones admit more entries). A candidate
/// that would exceed its group cap is skipped — the next candidate of
/// another group takes its place, so the remaining groups rebalance.
///
/// This implements the spec's global surface budget — 30% public/
/// entrypoint, 25% core impl, 15% types/interfaces, 10% state owners,
/// 10% contract APIs, 10% flow-critical (quota keys `public`, `core`,
/// `types`, `state`, `contract`, `flow`) — scaled to the tokens actually
/// available to the pool (`available_tokens` = budget minus required
/// spend; required entries are partitioned out before this stage and may
/// exceed their group allocation).
// trace:exempt reason=internal-detail
pub fn enforce_quotas(
    ranked: &[(String, f64)],
    kind_of: impl Fn(&str) -> &str,
    quotas: &[(String, f64)],
    available_tokens: usize,
    token_cost: impl Fn(&str) -> usize,
) -> Vec<String> {
    let mut caps: HashMap<&str, usize> = HashMap::new();
    for (kind, frac) in quotas {
        let cap = (frac.clamp(0.0, 1.0) * available_tokens as f64).round() as usize;
        caps.insert(kind.as_str(), cap);
    }
    let mut spent: HashMap<&str, usize> = HashMap::new();
    let mut out: Vec<String> = Vec::new();
    for (id, _) in ranked {
        let k = kind_of(id);
        let take = match caps.get(k) {
            None => true,
            Some(&cap) => {
                let s = spent.entry(k).or_insert(0);
                let next = s.saturating_add(token_cost(id));
                if next <= cap {
                    *s = next;
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
        let sel = select_with_budget(&items, 170, 204);
        assert_eq!(sel, vec![0, 1]);
        // Mid/low are dropped; required is present.
        assert!(sel.contains(&0));
        assert!(!sel.contains(&2) && !sel.contains(&3));

        // Budget exhausted by required alone: still never drops required.
        let sel = select_with_budget(&items, 50, 204);
        assert_eq!(sel, vec![0]);

        // Budget 0: only required.
        let sel = select_with_budget(&items, 0, 204);
        assert_eq!(sel, vec![0]);

        // Empty input.
        assert!(select_with_budget(&[], 100, 100).is_empty());

        // No required: pure value/token greedy.
        let no_req = vec![item("a", 0.1, 100, false), item("b", 0.9, 10, false)];
        let sel = select_with_budget(&no_req, 100, 100);
        assert_eq!(sel, vec![1]);

        // Hard max never drops required — a ceiling below the required
        // spend still selects it (the caller compresses the render
        // instead); pool items must fit the soft budget AND the hard max.
        let sel = select_with_budget(&items, 50, 60);
        assert_eq!(sel, vec![0]);
        let sel = select_with_budget(&items, 10, 5);
        assert_eq!(sel, vec![0]);
    }

    // ---- optimizer-off: rank-order selection ----

    #[test]
// trace:exempt reason=internal-detail
    fn select_in_order_keeps_rank_order() {
        let items = vec![
            item("required", 0.1, 120, true),
            item("high", 0.9, 50, false),
            item("mid", 0.5, 50, false),
            item("low", 0.1, 50, false),
        ];
        // Rank-order cut: required first, then items in list order while
        // the total fits — no value/token reordering.
        let sel = select_in_order(&items, 170, 204);
        assert_eq!(sel, vec![0, 1]);
        // One token less: required alone; mid/low are cut by budget.
        let sel = select_in_order(&items, 169, 204);
        assert_eq!(sel, vec![0]);
        // Required is never dropped, even at a zero budget.
        let sel = select_in_order(&items, 0, 204);
        assert_eq!(sel, vec![0]);
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

    // ---- (f) token-aware quota enforcement ----

    #[test]
// trace:exempt reason=internal-detail
    fn quotas_are_token_aware() {
        // One kind dominates the candidate pool (100 of 140 candidates);
        // caps derive from available TOKENS, not candidate counts.
        let mut ranked: Vec<(String, f64)> = Vec::new();
        for i in 0..100 {
            ranked.push((format!("pub:{i}"), 1.0 - i as f64 / 200.0));
        }
        for i in 0..20 {
            ranked.push((format!("core:{i}"), 1.0 - i as f64 / 200.0));
        }
        for i in 0..20 {
            ranked.push((format!("types:{i}"), 1.0 - i as f64 / 200.0));
        }

        fn kind_of(id: &str) -> &str {
            if id.starts_with("pub:") {
                "public"
            } else if id.starts_with("core:") {
                "core"
            } else {
                "types"
            }
        }
        fn other_kind(_: &str) -> &str {
            "other"
        }
        let quotas = vec![
            ("public".to_string(), 0.50),
            ("core".to_string(), 0.25),
            ("types".to_string(), 0.25),
        ];
        // 1400 tokens available: caps 700/350/350. At 10 tokens each the
        // dominant kind is capped at 70 entries (not its 100-candidate
        // count share); the under-represented kinds keep all 20 entries.
        let cost10 = |_: &str| 10usize;
        let sel = enforce_quotas(&ranked, kind_of, &quotas, 1400, cost10);
        let count = |k: &str| sel.iter().filter(|id| kind_of(id.as_str()) == k).count();
        assert_eq!(count("public"), 70);
        assert_eq!(count("core"), 20);
        assert_eq!(count("types"), 20);
        // Rank order is preserved: the first accepted per kind is the
        // first candidate of that kind in the ranked list.
        assert_eq!(sel[0], "pub:0");
        assert_eq!(sel[70], "core:0");

        // Expensive candidates exhaust a kind's allocation faster: at 50
        // tokens each the public cap (700) admits 14 entries, not 70.
        let cost50 = |_: &str| 50usize;
        let sel50 = enforce_quotas(&ranked, kind_of, &quotas, 1400, cost50);
        assert_eq!(
            sel50.iter().filter(|id| kind_of(id.as_str()) == "public").count(),
            14
        );

        // A capped-out dominant kind rebalances to the next group: the
        // first core candidate follows the last accepted public one.
        let sel = enforce_quotas(&ranked, kind_of, &quotas, 1400, cost10);
        let core_first = sel.iter().position(|id| kind_of(id.as_str()) == "core").unwrap();
        assert_eq!(sel[core_first], "core:0");

        // Kinds without a quota entry are uncapped.
        let sel_other = enforce_quotas(
            &[("x".to_string(), 0.5), ("y".to_string(), 0.5)],
            other_kind,
            &[],
            100,
            |_: &str| 10,
        );
        assert_eq!(sel_other.len(), 2);
    }
}
