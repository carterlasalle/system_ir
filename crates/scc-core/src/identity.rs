//! Stable repository identity (mission §XIV).
//!
//! Repository identity must not depend primarily on the checkout directory
//! basename: moving `~/code/backend` to `~/tmp/backend` must not create an
//! entirely different semantic repository when stable identity exists.
//!
//! Precedence (first non-empty wins):
//! 1. explicit operator id (`SCC_REPO_ID` / config) — the escape hatch;
//! 2. canonical sanitized git remote identity (`origin`);
//! 3. checkout directory basename (legacy fallback, unchanged behavior).
//!
//! Credentials embedded in remotes are NEVER part of the identity and are
//! never persisted (see [`sanitize_remote_url`]).

use crate::sanitize_key;

/// Canonicalize a git remote URL into a credential-free identity key.
///
/// Strips userinfo (`user:pass@`), a trailing `.git` suffix, and scheme
/// noise (`https://`, `ssh://`, `git@host:` scp syntax), lowercases, and
/// returns `host/path`. Returns `None` when nothing usable remains.
// trace:v1 id=impl.scc.core.identity.sanitize-remote-url work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn sanitize_remote_url(url: &str) -> Option<String> {
    let mut s = url.trim().to_string();
    if s.is_empty() {
        return None;
    }
    // scp-like syntax: git@host:org/repo(.git)
    if let Some(at) = s.find('@') {
        if !s[..at].contains("://") {
            if let Some(colon) = s[at..].find(':') {
                let host = s[at + 1..at + colon].to_ascii_lowercase();
                let mut path = s[at + colon + 1..].to_string();
                if let Some(stripped) = path.strip_suffix(".git") {
                    path = stripped.to_string();
                }
                let key = format!("{host}/{path}").to_ascii_lowercase();
                return if key.len() > 1 { Some(key) } else { None };
            }
        }
    }
    // scheme://[userinfo@]host/path
    if let Some(scheme_end) = s.find("://") {
        s = s[scheme_end + 3..].to_string();
        if let Some(at) = s.find('@') {
            // userinfo ends at the LAST @ before the first / (passwords may
            // contain @ only percent-encoded, so the last @ is the split).
            let slash = s.find('/').unwrap_or(s.len());
            if at < slash {
                s = s[at + 1..].to_string();
            }
        }
        // drop port (:443/…) — identity, not dialing
        if let Some(slash) = s.find('/') {
            let (hostport, path) = s.split_at(slash);
            let host = hostport.split(':').next().unwrap_or(hostport);
            let mut p = path.trim_start_matches('/').to_string();
            if let Some(stripped) = p.strip_suffix(".git") {
                p = stripped.to_string();
            }
            let key = format!("{}/{p}", host.to_ascii_lowercase()).to_ascii_lowercase();
            return if key.len() > 1 { Some(key) } else { None };
        }
        return None;
    }
    // bare path or host/path without scheme
    let mut p = s;
    if let Some(stripped) = p.strip_suffix(".git") {
        p = stripped.to_string();
    }
    let p = p.to_ascii_lowercase();
    if p.is_empty() {
        None
    } else {
        Some(p)
    }
}

/// Resolve the stable repository id from explicit config, remote, basename.
// trace:v1 id=impl.scc.core.identity.stable-repo-id work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn stable_repo_id(
    explicit: Option<&str>,
    remote_url: Option<&str>,
    basename: &str,
) -> String {
    if let Some(e) = explicit.map(str::trim).filter(|e| !e.is_empty()) {
        return sanitize_key(e);
    }
    if let Some(url) = remote_url {
        if let Some(key) = sanitize_remote_url(url) {
            return sanitize_key(&key);
        }
    }
    sanitize_key(basename)
}

/// Ensure `<state_dir>/repo-id` pins the stable id for fresh checkouts.
///
/// Writes the file only when an explicit or remote-derived id exists; a
/// basename-only checkout writes nothing so a later `git remote add` can
/// still promote the checkout to a stable id. Never overwrites. Returns the
/// id that [`stable_repo_id`] resolves to (whether or not it was written).
// trace:v1 id=impl.scc.core.identity.ensure-repo-id-file work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn ensure_repo_id_file(
    state_dir: &std::path::Path,
    explicit: Option<&str>,
    remote_url: Option<&str>,
    basename: &str,
) -> std::io::Result<String> {
    let id = stable_repo_id(explicit, remote_url, basename);
    let has_signal = explicit.map(str::trim).filter(|e| !e.is_empty()).is_some()
        || remote_url
            .map(|u| sanitize_remote_url(u).is_some())
            .unwrap_or(false);
    let path = state_dir.join("repo-id");
    if has_signal && !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, format!("{id}\n"))?;
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // trace:v1 id=impl.crates-scc-core-src-identity.strips-credentials-from-https-remote work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    fn strips_credentials_from_https_remote() {
        let got = sanitize_remote_url("https://user:s3cret@github.com/acme/billing.git")
            .unwrap();
        assert_eq!(got, "github.com/acme/billing");
        assert!(!got.contains("s3cret"), "secret leaked into identity");
        assert!(!got.contains("user"), "userinfo leaked into identity");
    }

    #[test]
    // trace:v1 id=impl.crates-scc-core-src-identity.parses-scp-syntax-and work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    fn parses_scp_syntax_and_strips_ports() {
        assert_eq!(
            sanitize_remote_url("git@github.com:acme/billing.git").unwrap(),
            "github.com/acme/billing"
        );
        assert_eq!(
            sanitize_remote_url("https://github.com:443/acme/billing").unwrap(),
            "github.com/acme/billing"
        );
    }

    #[test]
    // trace:v1 id=impl.crates-scc-core-src-identity.precedence-is-explicit-then-remote-then-basename work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    fn precedence_is_explicit_then_remote_then_basename() {
        assert_eq!(
            stable_repo_id(Some("Billing "), Some("https://x@h/o/r"), "checkout"),
            "billing"
        );
        assert_eq!(
            stable_repo_id(None, Some("git@github.com:acme/billing.git"), "checkout"),
            "github.com/acme/billing"
        );
        assert_eq!(stable_repo_id(None, None, "checkout"), "checkout");
        assert_eq!(stable_repo_id(Some(""), None, "checkout"), "checkout");
    }

    #[test]
    // trace:v1 id=impl.crates-scc-core-src-identity.empty-and-garbage-remotes-fall-back work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    fn empty_and_garbage_remotes_fall_back() {
        assert!(sanitize_remote_url("").is_none());
        assert_eq!(stable_repo_id(None, Some(""), "b"), "b");
    }

    #[test]
    // trace:v1 id=impl.crates-scc-core-src-identity.repo-id-pin-file work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    fn repo_id_file_pins_only_signalled_ids() {
        let dir = std::env::temp_dir().join(format!("scc-ident-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // remote-derived: pinned
        let id = ensure_repo_id_file(&dir, None, Some("git@github.com:acme/billing.git"), "checkout").unwrap();
        assert_eq!(id, "github.com/acme/billing");
        assert_eq!(std::fs::read_to_string(dir.join("repo-id")).unwrap(), "github.com/acme/billing\n");
        // never overwrites
        let id2 = ensure_repo_id_file(&dir, Some("other"), None, "checkout").unwrap();
        assert_eq!(id2, "other");
        assert_eq!(std::fs::read_to_string(dir.join("repo-id")).unwrap(), "github.com/acme/billing\n");
        std::fs::remove_dir_all(&dir).unwrap();
        // basename-only: no signal, nothing written
        let dir2 = std::env::temp_dir().join(format!("scc-ident2-{}", std::process::id()));
        std::fs::create_dir_all(&dir2).unwrap();
        let id3 = ensure_repo_id_file(&dir2, None, None, "checkout").unwrap();
        assert_eq!(id3, "checkout");
        assert!(!dir2.join("repo-id").exists());
        std::fs::remove_dir_all(&dir2).unwrap();
    }
}
