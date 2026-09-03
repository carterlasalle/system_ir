//! Marker-aware replacement of the `<!-- SCC-SECTION -->` managed block
//! in AGENTS.md (and equivalent instruction files).
//!
//! Replacement is `before-opening + new section + after-closing`. User
//! text after the closing marker is preserved. A rewrite that kept only
//! the prefix before `<!-- SCC-SECTION` deleted later user edits.

/// Opening marker prefix (the opening tag may carry attributes).
pub const SCC_SECTION_OPEN: &str = "<!-- SCC-SECTION";
/// Closing marker (exact).
pub const SCC_SECTION_CLOSE: &str = "<!-- /SCC-SECTION -->";

/// OMP-setup managed section markers. The OMP and Codex installers share
/// instruction files (both may patch the root AGENTS.md), so each owns a
/// DISTINCT marker pair — one shared pair would make the second installer
/// delete the first installer's section (last-writer-wins data loss).
pub const SCC_OMP_SECTION_OPEN: &str = "<!-- SCC-OMP-SECTION -->";
pub const SCC_OMP_SECTION_CLOSE: &str = "<!-- /SCC-OMP-SECTION -->";

/// Replace a managed section delimited by an explicit marker pair.
/// `replace_scc_section` is this function with the legacy shared markers.
// trace:v1 id=impl.scc.agents-md.replace-section work=WORK-SCC-001 satisfies=REQ-SCC-API
pub fn replace_scc_section(existing: &str, new_section: &str) -> String {
    replace_scc_section_in(existing, new_section, SCC_SECTION_OPEN, SCC_SECTION_CLOSE)
}

/// Replace a managed section delimited by an explicit marker pair,
/// keeping everything before the opening marker and everything after the
/// closing marker. If no markers are present, the new section is appended
/// after any existing user text.
// trace:v1 id=impl.scc.agents-md.replace-section-in work=WORK-SCC-001 satisfies=REQ-SCC-API
pub fn replace_scc_section_in(
    existing: &str,
    new_section: &str,
    open: &str,
    close: &str,
) -> String {
    let open_idx = existing.find(open);
    let close_idx = existing.find(close);
    let (before, after) = match (open_idx, close_idx) {
        (Some(start), Some(end)) if end >= start => {
            let before = existing[..start].trim_end();
            let after = existing[end + close.len()..].trim_start();
            (before, after)
        }
        (Some(start), None) => {
            // Opening without a close: keep the prefix, drop the broken tail.
            (existing[..start].trim_end(), "")
        }
        _ => (existing.trim(), ""),
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
}
