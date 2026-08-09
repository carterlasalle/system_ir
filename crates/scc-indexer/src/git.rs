//! Git snapshot resolution (docs/DATA_STRATEGY.md L0).

use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct GitInfo {
    pub revision: String,
    pub branch: Option<String>,
    pub remote_url: Option<String>,
}

/// Resolve current git state via the `git` CLI. Never fails: falls back to a
/// dirty-worktree marker.
pub fn resolve_git(root: &Path) -> GitInfo {
    let rev = run_git(root, &["rev-parse", "HEAD"]);
    let branch = run_git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .filter(|b| b != "HEAD");
    let remote = run_git(root, &["remote", "get-url", "origin"]);

    let revision = rev
        .map(|r| {
            // also record dirty state so content-hash invalidation is the
            // authoritative freshness check
            let dirty = run_git(root, &["status", "--porcelain"])
                .map(|s| !s.is_empty())
                .unwrap_or(false);
            if dirty { format!("{r}-dirty") } else { r }
        })
        .unwrap_or_else(|| "dirty".to_string());

    GitInfo {
        revision,
        branch: branch.filter(|b| !b.is_empty()),
        remote_url: remote.filter(|r| !r.is_empty()),
    }
}

fn run_git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_git_dir_falls_back() {
        let dir = tempfile::TempDir::new().unwrap();
        let info = resolve_git(dir.path());
        assert!(!info.revision.is_empty());
    }
}
