//! Marker-aware replacement of the `<!-- SCC-SECTION -->` managed block
//! in AGENTS.md (and equivalent instruction files).
//!
//! Replacement is `before-opening + new section + after-closing`. User
//! text after the closing marker is preserved. A rewrite that kept only
//! the prefix before `<!-- SCC-SECTION` deleted later user edits.

/// Opening marker prefix. A valid tag is this prefix followed by a tag
/// boundary (whitespace or `-->`), not a longer token such as
/// `<!-- SCC-SECTION-NOTES -->`.
pub const SCC_SECTION_OPEN: &str = "<!-- SCC-SECTION";
/// Closing marker (exact).
pub const SCC_SECTION_CLOSE: &str = "<!-- /SCC-SECTION -->";

/// OMP-setup managed section markers. The OMP and Codex installers share
/// instruction files (both may patch the root AGENTS.md), so each owns a
/// DISTINCT marker pair — one shared pair would make the second installer
/// delete the first installer's section (last-writer-wins data loss).
pub const SCC_OMP_SECTION_OPEN: &str = "<!-- SCC-OMP-SECTION -->";
pub const SCC_OMP_SECTION_CLOSE: &str = "<!-- /SCC-OMP-SECTION -->";

// trace:exempt reason=internal-helper
fn is_open_tag_boundary(rest: &str) -> bool {
    matches!(
        rest.chars().next(),
        Some(' ') | Some('\t') | Some('\n') | Some('\r')
    ) || rest.starts_with("-->")
}

/// Byte indices of opening markers. Prefix opens (no trailing `-->`)
/// require a tag boundary so `<!-- SCC-SECTION-NOTES -->` is not a hit.
/// Exact opens that already end with `-->` use literal `find`.
// trace:exempt reason=internal-helper
fn find_open_markers(existing: &str, open: &str) -> Vec<usize> {
    let exact = open.ends_with("-->");
    let mut from = 0;
    let mut out = Vec::new();
    while let Some(rel) = existing[from..].find(open) {
        let start = from + rel;
        if exact {
            out.push(start);
            from = start + open.len();
            continue;
        }
        let rest = &existing[start + open.len()..];
        if is_open_tag_boundary(rest) {
            out.push(start);
        }
        from = start + open.len();
    }
    out
}

/// First complete `(open, close)` pair whose close is before the next
/// opening marker. An orphan open is never paired with a later section's
/// close — that would delete the user tail this rewrite exists to keep.
// trace:exempt reason=internal-helper
fn find_complete_pair(existing: &str, open: &str, close: &str) -> Option<(usize, usize)> {
    let opens = find_open_markers(existing, open);
    for (i, start) in opens.iter().copied().enumerate() {
        let search_from = start + open.len();
        let Some(rel) = existing[search_from..].find(close) else {
            continue;
        };
        let end = search_from + rel;
        let next_open = opens.get(i + 1).copied();
        if next_open.map(|n| end < n).unwrap_or(true) {
            return Some((start, end));
        }
    }
    None
}

/// Replace a managed section delimited by an explicit marker pair.
/// `replace_scc_section` is this function with the legacy shared markers.
// trace:v1 id=impl.scc.agents-md.replace-section work=WORK-SCC-001 satisfies=REQ-SCC-API,REQ-implement-resolve-merge-conflicts-with-origin-main-and-address-remaini
pub fn replace_scc_section(existing: &str, new_section: &str) -> String {
    replace_scc_section_in(existing, new_section, SCC_SECTION_OPEN, SCC_SECTION_CLOSE)
}

/// Replace a managed section delimited by an explicit marker pair,
/// keeping everything before the opening marker and everything after the
/// closing marker. Pairing never spans a second opening marker. If no
/// complete pair is present, the new section is appended and later user
/// text (including an orphaned open) is preserved.
// trace:v1 id=impl.scc.agents-md.replace-section-in work=WORK-SCC-001 satisfies=REQ-SCC-API,REQ-implement-resolve-merge-conflicts-with-origin-main-and-address-remaini
pub fn replace_scc_section_in(
    existing: &str,
    new_section: &str,
    open: &str,
    close: &str,
) -> String {
    let (before, after) = match find_complete_pair(existing, open, close) {
        Some((start, end)) => {
            let before = existing[..start].trim_end();
            let after = existing[end + close.len()..].trim_start();
            (before, after)
        }
        // No complete pair (lookalike only, unpaired open, or unmarked):
        // keep the whole document (including later user text) and append.
        None => (existing.trim(), ""),
    };
    join_parts(before, new_section, after)
}

// trace:exempt reason=internal-helper
fn join_parts(before: &str, section: &str, after: &str) -> String {
    let mut out = String::new();
    if !before.is_empty() {
        out.push_str(before);
        out.push_str("\n\n");
    }
    out.push_str(section);
    if !section.ends_with('\n') {
        out.push('\n');
    }
    if !after.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
        out.push_str(after);
        if !after.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// Resolve the instruction file `scc setup omp` should patch.
///
/// OMP treats `.omp/AGENTS.md` as higher-priority same-scope context than
/// the root `AGENTS.md`. Required installer behavior:
/// - existing `.omp/AGENTS.md` → patch that (OMP's winning source)
/// - else existing root `AGENTS.md` → patch root, do NOT create a
///   shadowing `.omp/AGENTS.md`
/// - else create the canonical root `AGENTS.md`
/// - if both exist: patch `.omp/AGENTS.md` and surface a warning
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// trace:exempt reason=const-data
pub enum AgentsTarget {
    Omp,
    Root,
}

#[derive(Debug, Clone)]
// trace:exempt reason=const-data
pub struct AgentsResolution {
    pub target: AgentsTarget,
    pub both_exist: bool,
}

// trace:v1 id=impl.scc.agents-md.resolve-omp work=WORK-SCC-001 satisfies=REQ-SCC-API
pub fn resolve_omp_agents_path(root: &std::path::Path) -> AgentsResolution {
    let omp = root.join(".omp").join("AGENTS.md");
    let root_agents = root.join("AGENTS.md");
    let omp_exists = omp.is_file();
    let root_exists = root_agents.is_file();
    if omp_exists {
        AgentsResolution {
            target: AgentsTarget::Omp,
            both_exist: root_exists,
        }
    } else {
        // Existing root AGENTS.md, or neither file: patch/create root.
        // Never introduce a shadowing `.omp/AGENTS.md`.
        AgentsResolution {
            target: AgentsTarget::Root,
            both_exist: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // trace:v1 id=test.scc-cli-agents-md.preserves-after-close work=WORK-SCC-001 verifies=REQ-SCC-API,REQ-implement-p0-omp-integration-correctness-and-writable-benchmark-scient
    fn replace_preserves_text_after_closing_marker() {
        let existing = "BEFORE\n<!-- SCC-SECTION -->\nold\n<!-- /SCC-SECTION -->\nAFTER USER\n";
        let out = replace_scc_section(existing, "<!-- SCC-SECTION -->\nNEW\n<!-- /SCC-SECTION -->\n");
        assert!(out.starts_with("BEFORE"), "{out}");
        assert!(out.contains("NEW"), "{out}");
        assert!(out.contains("AFTER USER"), "later user text must survive: {out}");
        assert!(!out.contains("old"), "{out}");
    }

    #[test]
    // trace:v1 id=test.scc-cli-agents-md.append-when-unmarked work=WORK-SCC-001 verifies=REQ-SCC-API
    fn replace_appends_when_no_markers() {
        let out = replace_scc_section("USER\n", "<!-- SCC-SECTION -->\nNEW\n<!-- /SCC-SECTION -->\n");
        assert!(out.starts_with("USER"), "{out}");
        assert!(out.contains("NEW"), "{out}");
    }

    #[test]
    // trace:v1 id=test.scc-cli-agents-md.lookalike-open-is-not-a-marker work=WORK-SCC-001 verifies=REQ-SCC-API,REQ-implement-p0-omp-integration-correctness-and-writable-benchmark-scient
    fn replace_ignores_lookalike_open_prefix_and_preserves_later_text() {
        let existing = "USER\n<!-- SCC-SECTION-NOTES -->\nkeep later\n";
        let out = replace_scc_section(existing, "<!-- SCC-SECTION -->\nNEW\n<!-- /SCC-SECTION -->\n");
        assert!(out.contains("<!-- SCC-SECTION-NOTES -->"), "{out}");
        assert!(out.contains("keep later"), "later user text must survive a lookalike tag: {out}");
        assert!(out.contains("NEW"), "{out}");
    }

    #[test]
    // trace:v1 id=test.scc-cli-agents-md.unpaired-open-preserves-tail work=WORK-SCC-001 verifies=REQ-SCC-API,REQ-implement-p0-omp-integration-correctness-and-writable-benchmark-scient
    fn replace_unpaired_open_preserves_later_user_content() {
        let existing = "BEFORE\n<!-- SCC-SECTION -->\nbroken managed without close\nKEEP AFTER\n";
        let out = replace_scc_section(existing, "<!-- SCC-SECTION -->\nNEW\n<!-- /SCC-SECTION -->\n");
        assert!(out.contains("KEEP AFTER"), "unpaired open must not drop the tail: {out}");
        assert!(out.contains("NEW"), "{out}");
    }

    #[test]
    // trace:v1 id=test.scc-cli-agents-md.unpaired-open-replace-twice-preserves-tail work=WORK-resolve-merge-conflicts-with-origin-main-and-address-remaining-code-rabbi verifies=REQ-implement-resolve-merge-conflicts-with-origin-main-and-address-remaini,REQ-implement-p0-omp-integration-correctness-and-writable-benchmark-scient exercises=impl.scc.agents-md.replace-section-in
    fn replace_twice_on_unpaired_open_still_preserves_tail() {
        let existing = "BEFORE\n<!-- SCC-SECTION -->\nbroken managed without close\nKEEP AFTER\n";
        let section = "<!-- SCC-SECTION -->\nNEW\n<!-- /SCC-SECTION -->\n";
        let once = replace_scc_section(existing, section);
        let twice = replace_scc_section(&once, "<!-- SCC-SECTION -->\nNEWER\n<!-- /SCC-SECTION -->\n");
        assert!(
            twice.contains("KEEP AFTER"),
            "second replace must not pair the orphan open with the later close: {twice}"
        );
        assert!(twice.contains("NEWER"), "{twice}");
        assert!(!twice.contains("\nNEW\n"), "replaced body must not keep the first NEW: {twice}");
    }
}
