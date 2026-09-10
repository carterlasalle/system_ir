//! Repository Skeleton: deterministic physical-layout evidence for startup.
//!
//! The skeleton answers "what physically exists here" from the indexed file
//! inventory — indisputable layout facts that ground the inferred
//! architecture in the System Atlas. It is NOT a semantic layer and never
//! overrides semantic evidence: components, flows, and contracts come from
//! the graph, not from directory names.
//!
//! Invariants: deterministic per file set (BTree iteration only), hard
//! token-bounded (the caller passes the budget; overflow collapses lines,
//! never silently exceeds), role-labeled via the shared
//! [`scc_graph::components::component_role`] classifier.

use scc_core::estimate_tokens;
use std::collections::BTreeMap;

/// Skeleton budget policy: 10% of the startup total, floored so tiny repos
/// still show their top level, capped so the skeleton stays in the hundreds
/// of tokens. Receipt: default startup total is 20_000 → 800-token skeleton.
// trace:v1 id=impl.crates-scc-context-src-skeleton.skeleton-budget work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-NX53P4B7
pub fn skeleton_budget(total: usize) -> usize {
    (total / 10).clamp(64, 800).min(total.max(64))
}

/// Rendered skeleton plus honesty counts.
// trace:v1 id=impl.crates-scc-context-src-skeleton.skeleton work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-NX53P4B7
pub struct Skeleton {
    /// Rendered lines (no section header; the assembler adds that).
    pub text: String,
    /// Top-level entries shown.
    pub top_entries: usize,
    /// Indexed files folded into the tree (visible or collapsed).
    pub files_seen: usize,
    /// Lines collapsed away by caps or the token budget.
    pub collapsed: usize,
}

/// Maximum top-level entries before the top list itself collapses.
const MAX_TOP: usize = 64;
/// Maximum expansion lines per production top-level directory.
const MAX_EXPAND_PER_TOP: usize = 10;

/// Build the skeleton from indexed repo-relative paths. Priority order:
/// every top-level entry first (mandatory, collapses with a count only
/// past MAX_TOP), then depth-2 expansions of production/example trees;
/// test/fixture/benchmark trees render as one labeled line each. Lines
/// that do not fit the token budget are dropped and counted, never cut
/// mid-line.
// trace:v1 id=impl.crates-scc-context-src-skeleton.build-skeleton work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-NX53P4B7
pub fn build_skeleton(paths: &[String], budget_tokens: usize) -> Skeleton {
    let mut tops: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut loose: Vec<String> = Vec::new();
    for p in paths {
        match p.split_once('/') {
            Some((top, _)) => tops.entry(top.to_string()).or_default().push(p.clone()),
            None => loose.push(p.clone()),
        }
    }
    loose.sort();
    loose.dedup();

    let mut top_lines: Vec<String> = Vec::new();
    let mut expansions: Vec<Vec<String>> = Vec::new();
    for (top, members) in tops.iter().take(MAX_TOP) {
        let role = scc_graph::components::component_role(std::slice::from_ref(top));
        top_lines.push(format!("- {top}/ [{role}] ({} files)", members.len()));
        if role == "production" || role == "example" {
            expansions.push(expand_top(top, members));
        }
    }
    for f in &loose {
        top_lines.push(format!("- {f}"));
    }
    let omitted_tops = tops.len().saturating_sub(MAX_TOP);

    let mut lines: Vec<String> = Vec::new();
    let mut collapsed: usize = omitted_tops;
    let mut used = 0usize;
    for line in top_lines {
        let cost = estimate_tokens(&line);
        if used + cost > budget_tokens {
            collapsed += 1;
            continue;
        }
        used += cost;
        lines.push(line);
    }
    for block in &expansions {
        for line in block.iter().take(MAX_EXPAND_PER_TOP) {
            let cost = estimate_tokens(line);
            if used + cost > budget_tokens {
                collapsed += 1;
                continue;
            }
            used += cost;
            lines.push(line.clone());
        }
        collapsed += block.len().saturating_sub(MAX_EXPAND_PER_TOP);
    }
    let text = if collapsed > 0 {
        format!(
            "{}\n... (+{collapsed} entries omitted over skeleton budget)",
            lines.join("\n")
        )
    } else {
        lines.join("\n")
    };
    Skeleton {
        text,
        top_entries: tops.len().min(MAX_TOP) + loose.len(),
        files_seen: paths.len(),
        collapsed,
    }
}

/// Second-level entries of one top directory, deterministic.
// trace:exempt reason=internal-detail
fn expand_top(top: &str, members: &[String]) -> Vec<String> {
    let mut kids: BTreeMap<String, usize> = BTreeMap::new();
    let mut loose_files: Vec<String> = Vec::new();
    for m in members {
        let rest = m.strip_prefix(top).unwrap_or(m).trim_start_matches('/');
        match rest.split_once('/') {
            Some((kid, _)) => *kids.entry(kid.to_string()).or_default() += 1,
            None => loose_files.push(rest.to_string()),
        }
    }
    loose_files.sort();
    loose_files.dedup();
    let mut out = Vec::new();
    for f in loose_files.into_iter().take(8) {
        out.push(format!("  - {top}/{f}"));
    }
    for (kid, n) in &kids {
        out.push(format!("  - {top}/{kid}/ ({n} files)"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // trace:exempt reason=test-helper
    fn paths(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    // trace:v1 id=test.scc.context.skeleton-deterministic verifies=REQ-SI-NX53P4B7 exercises=impl.crates-scc-context-src-skeleton.build-skeleton
    fn skeleton_is_deterministic_and_role_labeled() {
        let a = build_skeleton(
            &paths(&[
                "crates/scc-core/src/lib.rs",
                "crates/scc-cli/src/main.rs",
                "fixtures/http-service-python/main.py",
                "Cargo.toml",
            ]),
            800,
        );
        let b = build_skeleton(
            &paths(&[
                "Cargo.toml",
                "fixtures/http-service-python/main.py",
                "crates/scc-cli/src/main.rs",
                "crates/scc-core/src/lib.rs",
            ]),
            800,
        );
        assert_eq!(a.text, b.text, "input order must not matter");
        assert!(a.text.contains("crates/ [production]"), "{}", a.text);
        assert!(a.text.contains("fixtures/ [fixture]"), "{}", a.text);
        assert!(a.text.contains("- Cargo.toml"), "{}", a.text);
        assert!(a.text.contains("scc-core/"), "{}", a.text);
        assert_eq!(a.collapsed, 0);
        assert_eq!(a.files_seen, 4);
    }

    #[test]
    // trace:v1 id=test.scc.context.skeleton-budget verifies=REQ-SI-NX53P4B7 exercises=impl.crates-scc-context-src-skeleton.build-skeleton
    fn skeleton_never_exceeds_its_budget() {
        let many: Vec<String> = (0..300)
            .map(|i| format!("crates/svc{i}/src/mod{i}/file{i}.rs"))
            .collect();
        let sk = build_skeleton(&many, 200);
        assert!(
            estimate_tokens(&sk.text) <= 200,
            "skeleton must fit budget: {}",
            estimate_tokens(&sk.text)
        );
        assert!(sk.collapsed > 0, "overflow must be counted, not silent");
        assert!(sk.text.contains("omitted over skeleton budget"));
    }

    #[test]
    // trace:v1 id=test.scc.context.skeleton-budget-policy verifies=REQ-SI-NX53P4B7 exercises=impl.crates-scc-context-src-skeleton.skeleton-budget
    fn skeleton_budget_is_bounded_fraction() {
        assert_eq!(skeleton_budget(20_000), 800);
        assert_eq!(skeleton_budget(1_000), 100);
        assert!(skeleton_budget(64) <= 64);
    }
}
