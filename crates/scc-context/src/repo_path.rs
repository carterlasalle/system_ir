//! Keep Task Context file reads inside the indexed repository.

use std::path::{Component, Path, PathBuf};

/// True when `path` is a repo-relative path with no `..` or absolute root.
// trace:exempt reason=internal-detail
pub(crate) fn is_repo_relative(path: &str) -> bool {
    if path.is_empty() || path.contains('\0') {
        return false;
    }
    let p = Path::new(path);
    if p.is_absolute() {
        return false;
    }
    !p.components().any(|c| matches!(c, Component::ParentDir))
}

/// Canonical path of `path` when it resolves inside `root`.
// trace:v1 id=impl.scc.context.repo-sandbox work=WORK-fix-pr-review-comments-without-collapsing-scc-type-script-non-null-asser satisfies=REQ-implement-fix-pr-review-comments-without-collapsing-scc-type-script-no implements=PLAN-fix-pr-review-comments-without-collapsing-scc-type-script-non-null-asser
pub(crate) fn resolve_repo_path(root: &Path, path: &str) -> Option<PathBuf> {
    if !is_repo_relative(path) {
        return None;
    }
    let root_c = root.canonicalize().ok()?;
    let canon = root_c.join(path).canonicalize().ok()?;
    canon.starts_with(&root_c).then_some(canon)
}

// trace:exempt reason=internal-detail
pub(crate) fn read_repo_file(root: &Path, path: &str) -> Option<Vec<u8>> {
    let canon = resolve_repo_path(root, path)?;
    std::fs::read(canon).ok()
}

// trace:exempt reason=internal-detail
pub(crate) fn read_repo_text(root: &Path, path: &str) -> Option<String> {
    String::from_utf8(read_repo_file(root, path)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // trace:v1 id=test.scc.context.repo-sandbox verifies=REQ-implement-fix-pr-review-comments-without-collapsing-scc-type-script-no exercises=impl.scc.context.repo-sandbox
    fn rejects_absolute_and_dotdot_paths() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("ok.py"), "x\n").unwrap();
        assert!(resolve_repo_path(root, "ok.py").is_some());
        assert!(resolve_repo_path(root, "/etc/passwd").is_none());
        assert!(resolve_repo_path(root, "../secret").is_none());
        assert!(!is_repo_relative("/etc/passwd"));
        assert!(!is_repo_relative("../../secret"));
    }
}
