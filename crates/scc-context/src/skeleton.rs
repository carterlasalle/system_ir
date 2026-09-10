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
/// Maximum depth-3 lines per priority top-level directory (below).
const MAX_DEPTH3_PER_TOP: usize = 8;
/// Depth-3 file/subdir lines each, per priority top.
const MAX_DEPTH3_KIDS: usize = 4;

/// Manifest filenames that mark a directory as high-information.
const MANIFESTS: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "go.mod",
    "pyproject.toml",
    "setup.cfg",
    "pom.xml",
    "build.gradle",
];

/// A top directory is high-information when it roots a package or a
/// source tree: a manifest directly under it (`top/Cargo.toml`), a
/// manifest one level down (`top/crate/Cargo.toml`), or a `src/` tree.
/// Priority tops expand one level deeper (depth 3, capped) so workspace
/// members, source files, and entrypoints show without raising the
/// global depth (deep low-value trees still collapse).
// trace:exempt reason=internal-detail
fn priority_top(top: &str, members: &[String]) -> bool {
    members.iter().any(|m| {
        let rest = m.strip_prefix(top).unwrap_or(m).trim_start_matches('/');
        if rest == "src" || rest.starts_with("src/") {
            return true;
        }
        let mut segs = rest.split('/');
        match (segs.next(), segs.next(), segs.next()) {
            (Some(_), None, _) => MANIFESTS.contains(&rest),
            (Some(_), Some(last), None) => MANIFESTS.contains(&last),
            _ => false,
        }
    })
}

/// Build the skeleton from indexed repo-relative paths. Priority order:
/// every top-level entry first (mandatory, collapses with a count only
/// past MAX_TOP), then depth-2 expansions of production/example/sdk
/// trees (test/fixture/benchmark trees render as one labeled line each),
/// then depth-3 expansions of priority tops only. Lines that do not fit
/// the token budget are dropped and counted, never cut mid-line.
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
    let mut expansions: Vec<(bool, Vec<String>, Vec<String>)> = Vec::new();
    for (top, members) in tops.iter().take(MAX_TOP) {
        let role = scc_graph::components::component_role(std::slice::from_ref(top));
        top_lines.push(format!("- {top}/ [{role}] ({} files)", members.len()));
        if role == "production" || role == "example" || role == "sdk" {
            let priority = priority_top(top, members);
            let (d2, d3) = expand_top(top, members, priority);
            expansions.push((priority, d2, d3));
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
    for (priority, d2, d3) in &expansions {
        for line in d2.iter().take(MAX_EXPAND_PER_TOP) {
            let cost = estimate_tokens(line);
            if used + cost > budget_tokens {
                collapsed += 1;
                continue;
            }
            used += cost;
            lines.push(line.clone());
        }
        collapsed += d2.len().saturating_sub(MAX_EXPAND_PER_TOP);
        // Depth 3 exists only for priority tops (built only for them, so
        // non-priority collapse counts are untouched by this feature).
        if *priority {
            for line in d3.iter().take(MAX_DEPTH3_PER_TOP) {
                let cost = estimate_tokens(line);
                if used + cost > budget_tokens {
                    collapsed += 1;
                    continue;
                }
                used += cost;
                lines.push(line.clone());
            }
            collapsed += d3.len().saturating_sub(MAX_DEPTH3_PER_TOP);
        }
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

/// Second-level entries of one top directory, deterministic. Returns
/// `(depth2, depth3)`: depth-3 grandchildren are built ONLY for priority
/// tops (workspace/package/source roots) and stay empty otherwise.
// trace:exempt reason=internal-detail
fn expand_top(top: &str, members: &[String], priority: bool) -> (Vec<String>, Vec<String>) {
    let mut kids: BTreeMap<String, usize> = BTreeMap::new();
    // kid dir -> its direct members (for depth-3 grandchildren)
    let mut grand: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut loose_files: Vec<String> = Vec::new();
    for m in members {
        let rest = m.strip_prefix(top).unwrap_or(m).trim_start_matches('/');
        match rest.split_once('/') {
            Some((kid, _)) => {
                *kids.entry(kid.to_string()).or_default() += 1;
                if priority {
                    grand.entry(kid.to_string()).or_default().push(m.clone());
                }
            }
            None => loose_files.push(rest.to_string()),
        }
    }
    loose_files.sort();
    loose_files.dedup();
    let mut d2 = Vec::new();
    for f in loose_files.into_iter().take(8) {
        d2.push(format!("  - {top}/{f}"));
    }
    for (kid, n) in &kids {
        d2.push(format!("  - {top}/{kid}/ ({n} files)"));
    }
    let mut d3 = Vec::new();
    if priority {
        for (kid, paths) in &grand {
            let prefix = format!("{top}/{kid}/");
            let mut files: Vec<String> = Vec::new();
            let mut subdirs: BTreeMap<String, usize> = BTreeMap::new();
            for full in paths {
                let rest = full.strip_prefix(&prefix).unwrap_or(full);
                match rest.split_once('/') {
                    Some((sub, _)) => *subdirs.entry(sub.to_string()).or_default() += 1,
                    None => files.push(rest.to_string()),
                }
            }
            files.sort();
            files.dedup();
            for f in files.into_iter().take(MAX_DEPTH3_KIDS) {
                d3.push(format!("    - {prefix}{f}"));
            }
            for (sub, n) in subdirs.iter().take(MAX_DEPTH3_KIDS) {
                d3.push(format!("    - {prefix}{sub}/ ({n} files)"));
            }
        }
    }
    (d2, d3)
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
    // trace:v1 id=test.scc.context.skeleton-depth3 verifies=REQ-SI-NX53P4B7 exercises=impl.crates-scc-context-src-skeleton.build-skeleton
    fn skeleton_expands_priority_tops_one_level_deeper() {
        // crates/ roots a package (crates/scc-core/Cargo.toml): depth-3
        // grandchildren show. plains/ has no manifest: depth-2 only.
        let sk = build_skeleton(
            &paths(&[
                "crates/scc-core/Cargo.toml",
                "crates/scc-core/src/lib.rs",
                "crates/scc-core/src/resolve.rs",
                "plains/a.txt",
                "plains/sub/b.txt",
            ]),
            800,
        );
        assert!(sk.text.contains("    - crates/scc-core/src/ (2 files)"), "{}", sk.text);
        assert!(sk.text.contains("    - crates/scc-core/Cargo.toml"), "{}", sk.text);
        assert!(!sk.text.contains("      "), "no depth-4: {}", sk.text);
        // budget still binds with depth-3 content
        let big: Vec<String> = (0..200)
            .map(|i| format!("crates/svc{i}/Cargo.toml"))
            .chain((0..200).map(|i| format!("crates/svc{i}/src/lib.rs")))
            .collect();
        let sk2 = build_skeleton(&big, 200);
        assert!(
            estimate_tokens(&sk2.text) <= 200,
            "skeleton must fit budget: {}",
            estimate_tokens(&sk2.text)
        );
    }

    #[test]
    // trace:v1 id=test.scc.context.skeleton-budget-policy verifies=REQ-SI-NX53P4B7 exercises=impl.crates-scc-context-src-skeleton.skeleton-budget
    fn skeleton_budget_is_bounded_fraction() {
        assert_eq!(skeleton_budget(20_000), 800);
        assert_eq!(skeleton_budget(1_000), 100);
        assert!(skeleton_budget(64) <= 64);
    }
}
