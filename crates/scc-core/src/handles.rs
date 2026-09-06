//! Stable content handles for lazy exact-source retrieval.
//!
//! Handles identify a repository object at a ModelEpoch. They are
//! root-aware, collision-resistant (path + name, never basename-only),
//! and carry a content hash so a stale target can be refused instead of
//! silently served.

use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

/// Kind of object a handle names. Heterogeneous on purpose: SCC entities
/// are not only symbols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HandleKind {
    Symbol,
    Component,
    Flow,
    Contract,
    State,
    Route,
    File,
    Span,
}

impl HandleKind {
    pub fn as_str(self) -> &'static str {
        match self {
            HandleKind::Symbol => "symbol",
            HandleKind::Component => "component",
            HandleKind::Flow => "flow",
            HandleKind::Contract => "contract",
            HandleKind::State => "state",
            HandleKind::Route => "route",
            HandleKind::File => "file",
            HandleKind::Span => "span",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "symbol" => HandleKind::Symbol,
            "component" => HandleKind::Component,
            "flow" => HandleKind::Flow,
            "contract" => HandleKind::Contract,
            "state" => HandleKind::State,
            "route" => HandleKind::Route,
            "file" => HandleKind::File,
            "span" => HandleKind::Span,
            _ => return None,
        })
    }
}

/// Deterministic, epoch-scoped content identifier.
///
/// Format (not locked against future collision/freshness changes):
/// `scc://{repo}/{epoch}/{kind}/{encoded_key}@{content_hash16}`
///
/// `encoded_key` is percent-encoded so paths and `::` scopes stay portable
/// across MCP calls. `content_hash16` is FNV-1a-64 of the referent file
/// bytes (hex); empty hash means identity-only (no freshness gate).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
// trace:v1 id=impl.scc.core.content-handle work=WORK-ripwire-lessons-phase1 satisfies=REQ-stable-content-handles
pub struct ContentHandle {
    pub repo: String,
    pub epoch: String,
    pub kind: HandleKind,
    pub key: String,
    /// 16-char hex FNV-1a of file bytes; empty if unknown.
    pub content_hash: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HandleError {
    #[error("malformed handle")]
    Malformed,
    #[error("stale handle: content hash mismatch")]
    Stale,
    #[error("ambiguous handle")]
    Ambiguous,
    #[error("unknown handle kind")]
    UnknownKind,
}

impl ContentHandle {
    pub fn new(
        repo: impl Into<String>,
        epoch: impl Into<String>,
        kind: HandleKind,
        key: impl Into<String>,
        content_hash: impl Into<String>,
    ) -> Self {
        ContentHandle {
            repo: repo.into(),
            epoch: epoch.into(),
            kind,
            key: key.into(),
            content_hash: content_hash.into(),
        }
    }

    /// Symbol handle: `path::qualified_name` (never basename-only).
    pub fn for_symbol(
        repo: &str,
        epoch: &str,
        path: &str,
        name: &str,
        content_hash: &str,
    ) -> Self {
        ContentHandle::new(
            repo,
            epoch,
            HandleKind::Symbol,
            format!("{path}::{name}"),
            content_hash,
        )
    }

    pub fn for_file(repo: &str, epoch: &str, path: &str, content_hash: &str) -> Self {
        ContentHandle::new(repo, epoch, HandleKind::File, path, content_hash)
    }

    pub fn for_span(
        repo: &str,
        epoch: &str,
        path: &str,
        start_line: u32,
        end_line: u32,
        content_hash: &str,
    ) -> Self {
        ContentHandle::new(
            repo,
            epoch,
            HandleKind::Span,
            format!("{path}:L{start_line}-L{end_line}"),
            content_hash,
        )
    }

    pub fn parse(s: &str) -> Result<Self, HandleError> {
        let rest = s.strip_prefix("scc://").ok_or(HandleError::Malformed)?;
        let (repo, rest) = rest.split_once('/').ok_or(HandleError::Malformed)?;
        let (epoch, rest) = rest.split_once('/').ok_or(HandleError::Malformed)?;
        let (kind_s, rest) = rest.split_once('/').ok_or(HandleError::Malformed)?;
        let kind = HandleKind::parse(kind_s).ok_or(HandleError::UnknownKind)?;
        let (key_enc, hash) = match rest.rsplit_once('@') {
            Some((k, h)) => (k, h.to_string()),
            None => (rest, String::new()),
        };
        if repo.is_empty() || epoch.is_empty() || key_enc.is_empty() {
            return Err(HandleError::Malformed);
        }
        Ok(ContentHandle {
            repo: percent_decode(repo),
            epoch: epoch.to_string(),
            kind,
            key: percent_decode(key_enc),
            content_hash: hash,
        })
    }

    /// True when `actual_hash` matches the handle's freshness pin.
    /// Identity-only handles (empty hash) never fail this check.
    pub fn matches_content(&self, actual_hash: &str) -> bool {
        self.content_hash.is_empty() || self.content_hash == actual_hash
    }

    pub fn refuse_if_stale(&self, actual_hash: &str) -> Result<(), HandleError> {
        if self.matches_content(actual_hash) {
            Ok(())
        } else {
            Err(HandleError::Stale)
        }
    }
}

impl fmt::Display for ContentHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "scc://{}/{}/{}/{}",
            percent_encode(&self.repo),
            self.epoch,
            self.kind.as_str(),
            percent_encode(&self.key)
        )?;
        if !self.content_hash.is_empty() {
            write!(f, "@{}", self.content_hash)?;
        }
        Ok(())
    }
}

/// FNV-1a 64-bit, hex (16 chars). Stable across processes; used for
/// handle freshness, not cryptographic integrity.
pub fn fnv1a64_hex(bytes: &[u8]) -> String {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut h = OFFSET;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(PRIME);
    }
    format!("{h:016x}")
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // trace:v1 id=test.scc.core.handle-roundtrip verifies=REQ-stable-content-handles exercises=impl.scc.core.content-handle
    fn roundtrip_is_stable_and_path_aware() {
        let h = ContentHandle::for_symbol(
            "repo",
            "epoch1",
            "src/foo.ts",
            "Class::method",
            "deadbeefcafebabe",
        );
        let s = h.to_string();
        assert!(s.starts_with("scc://"), "{s}");
        assert!(!s.contains("foo.ts::Class") || s.contains("src"), "{s}");
        let parsed = ContentHandle::parse(&s).unwrap();
        assert_eq!(parsed, h);
        assert_eq!(parsed.key, "src/foo.ts::Class::method");
    }

    #[test]
    fn basename_only_is_not_the_identity() {
        let a = ContentHandle::for_symbol("r", "e", "a/foo.rs", "n", "1");
        let b = ContentHandle::for_symbol("r", "e", "b/foo.rs", "n", "1");
        assert_ne!(a.to_string(), b.to_string());
    }

    #[test]
    fn stale_hash_refuses() {
        let h = ContentHandle::for_file("r", "e", "x.rs", "aaaaaaaaaaaaaaaa");
        assert_eq!(h.refuse_if_stale("bbbbbbbbbbbbbbbb"), Err(HandleError::Stale));
        assert!(h.refuse_if_stale("aaaaaaaaaaaaaaaa").is_ok());
        let open = ContentHandle::for_file("r", "e", "x.rs", "");
        assert!(open.refuse_if_stale("anything").is_ok());
    }

    #[test]
    fn malformed_refuses() {
        assert_eq!(ContentHandle::parse("not-a-handle"), Err(HandleError::Malformed));
        assert_eq!(ContentHandle::parse("scc://"), Err(HandleError::Malformed));
    }

    #[test]
    fn fnv_is_deterministic() {
        assert_eq!(fnv1a64_hex(b"abc"), fnv1a64_hex(b"abc"));
        assert_ne!(fnv1a64_hex(b"abc"), fnv1a64_hex(b"abd"));
    }
}
