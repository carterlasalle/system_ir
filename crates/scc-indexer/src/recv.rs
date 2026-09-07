//! Receiver classification recovered from callee text.
//!
//! Extractors already store the callee expression (`self.helper`,
//! `this.field.method()`, `Foo::bar`). Classification here lets the
//! resolver refuse field-chain pins and same-name spray without a
//! second AST walk. Extractors may overwrite `Call.recv` when the AST
//! is more precise.

use scc_core::{RecvKind, ReferenceKind};

#[derive(Debug, Clone, PartialEq, Eq)]
// trace:exempt reason=internal-detail
pub struct RecvFact {
    pub recv: RecvKind,
    pub root: String,
    pub method: String,
    pub qualifier: Option<String>,
    pub recv_var: Option<String>,
    pub field_name: Option<String>,
    pub role: ReferenceKind,
}

impl RecvFact {
    fn bare(name: &str, role: ReferenceKind) -> Self {
        RecvFact {
            recv: RecvKind::None,
            root: name.to_string(),
            method: name.to_string(),
            qualifier: None,
            recv_var: None,
            field_name: None,
            role,
        }
    }
}

/// Split `a.b.c` / `A::b` / `a->b` into identifier segments.
pub fn split_recv_path(callee: &str) -> Vec<String> {
    let mut s = callee.trim().to_string();
    if let Some(idx) = s.find("::<") {
        s.truncate(idx);
    } else if let Some(idx) = s.find('<') {
        if s[..idx].contains("::") || s[..idx].contains('.') {
            s.truncate(idx);
        }
    }
    let s = s.trim_end_matches('!');
    let s = s.trim_end_matches("()");
    let mut out = Vec::new();
    let mut cur = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'.' {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            i += 1;
            continue;
        }
        if bytes[i] == b':' && i + 1 < bytes.len() && bytes[i + 1] == b':' {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            i += 2;
            continue;
        }
        if bytes[i] == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'>' {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            i += 2;
            continue;
        }
        cur.push(bytes[i] as char);
        i += 1;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.into_iter()
        .map(|p| p.trim().trim_end_matches('!').trim().to_string())
        .filter(|p| !p.is_empty() && p != "()")
        .collect()
}

fn looks_type_name(name: &str) -> bool {
    name.chars()
        .next()
        .map(|c| c.is_uppercase() || c == '_')
        .unwrap_or(false)
        && name != "SUPER"
}

/// Python `super().open()` arrives as callee `super().open`; the receiver
/// token is still `super`.
// trace:v1 id=impl.scc.recv.super-call work=WORK-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-super-base-wa satisfies=REQ-implement-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-s implements=PLAN-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-super-base-wa
fn super_recv_root(seg: &str) -> &str {
    if seg == "super()" {
        "super"
    } else {
        seg
    }
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Rust/C `ident!` / `ident!(` / `ident![`. TypeScript non-null `expr!.m`
/// is `!.` and stays a Call.
// trace:exempt reason=internal-detail
fn looks_like_macro(callee: &str) -> bool {
    let b = callee.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'!' && i > 0 && is_ident_byte(b[i - 1]) {
            let next = b.get(i + 1).copied();
            if next != Some(b'.') {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Classify a callee expression captured by an extractor.
// trace:v1 id=impl.scc.recv.classify work=WORK-ripwire-lessons-phase1 satisfies=REQ-receiver-aware-resolution,REQ-implement-fix-pr-review-comments-without-collapsing-scc-type-script-no
pub fn classify_callee(callee: &str) -> RecvFact {
    let trimmed = callee.trim();
    let role = if looks_like_macro(trimmed) {
        ReferenceKind::Macro
    } else {
        ReferenceKind::Call
    };
    let segs = split_recv_path(trimmed);
    if segs.is_empty() {
        return RecvFact {
            recv: RecvKind::Unknown,
            root: String::new(),
            method: trimmed.to_string(),
            qualifier: None,
            recv_var: None,
            field_name: None,
            role,
        };
    }
    if segs.len() == 1 {
        return RecvFact::bare(&segs[0], role);
    }
    let root = super_recv_root(segs[0].as_str());
    let method = segs[segs.len() - 1].clone();
    let used_colon = trimmed.contains("::");
    match (root, segs.len()) {
        ("self", 2) | ("Self", 2) => RecvFact {
            recv: RecvKind::SelfRecv,
            root: root.to_string(),
            method,
            qualifier: None,
            recv_var: None,
            field_name: None,
            role,
        },
        ("self", _) | ("Self", _) => RecvFact {
            recv: RecvKind::FieldOfSelf,
            root: root.to_string(),
            method,
            qualifier: None,
            recv_var: None,
            field_name: Some(segs[1].clone()),
            role,
        },
        ("this", 2) => RecvFact {
            recv: RecvKind::This,
            root: "this".into(),
            method,
            qualifier: None,
            recv_var: None,
            field_name: None,
            role,
        },
        ("this", _) => RecvFact {
            recv: RecvKind::FieldOfThis,
            root: "this".into(),
            method,
            qualifier: None,
            recv_var: None,
            field_name: Some(segs[1].clone()),
            role,
        },
        ("cls", 2) => RecvFact {
            recv: RecvKind::SelfRecv,
            root: "cls".into(),
            method,
            qualifier: None,
            recv_var: None,
            field_name: None,
            role,
        },
        ("super", _) => RecvFact {
            recv: RecvKind::Super,
            root: "super".into(),
            method,
            qualifier: None,
            recv_var: None,
            field_name: None,
            role,
        },
        ("crate", _) => RecvFact {
            recv: RecvKind::TypeQualified,
            root: root.to_string(),
            method: method.clone(),
            qualifier: Some(segs[..segs.len() - 1].join("::")),
            recv_var: None,
            field_name: None,
            role,
        },
        _ if used_colon => RecvFact {
            recv: RecvKind::TypeQualified,
            root: root.to_string(),
            method: method.clone(),
            qualifier: Some(segs[..segs.len() - 1].join("::")),
            recv_var: None,
            field_name: None,
            role,
        },
        _ if looks_type_name(root) && segs.len() == 2 => RecvFact {
            recv: RecvKind::StaticType,
            root: root.to_string(),
            method,
            qualifier: Some(root.to_string()),
            recv_var: None,
            field_name: None,
            role,
        },
        _ if segs.len() == 2 => RecvFact {
            recv: RecvKind::NamedVariable,
            root: root.to_string(),
            method,
            qualifier: None,
            recv_var: Some(root.to_string()),
            field_name: None,
            role,
        },
        _ => RecvFact {
            recv: RecvKind::FieldOfVariable,
            root: root.to_string(),
            method,
            qualifier: None,
            recv_var: Some(root.to_string()),
            field_name: Some(segs[1].clone()),
            role,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // trace:v1 id=test.scc.recv.classify-shapes verifies=REQ-receiver-aware-resolution,REQ-implement-fix-pr-review-comments-without-collapsing-scc-type-script-no exercises=impl.scc.recv.classify
    fn classifies_receiver_shapes() {
        let bare = classify_callee("normalize");
        assert_eq!(bare.recv, RecvKind::None);
        assert_eq!(bare.method, "normalize");

        let slf = classify_callee("self.helper");
        assert_eq!(slf.recv, RecvKind::SelfRecv);
        assert_eq!(slf.method, "helper");

        let this = classify_callee("this.process");
        assert_eq!(this.recv, RecvKind::This);
        assert_eq!(this.method, "process");

        let chain = classify_callee("self.client.process");
        assert_eq!(chain.recv, RecvKind::FieldOfSelf);
        assert_eq!(chain.method, "process");
        assert_eq!(chain.field_name.as_deref(), Some("client"));

        let this_chain = classify_callee("this.field.method");
        assert_eq!(this_chain.recv, RecvKind::FieldOfThis);
        assert_eq!(this_chain.method, "method");

        let named = classify_callee("obj.method");
        assert_eq!(named.recv, RecvKind::NamedVariable);
        assert_eq!(named.recv_var.as_deref(), Some("obj"));

        let field_var = classify_callee("obj.field.method");
        assert_eq!(field_var.recv, RecvKind::FieldOfVariable);

        let ty = classify_callee("Logger.error");
        assert_eq!(ty.recv, RecvKind::StaticType);

        let qual = classify_callee("Foo::bar");
        assert_eq!(qual.recv, RecvKind::TypeQualified);
        assert_eq!(qual.qualifier.as_deref(), Some("Foo"));

        let sup = classify_callee("super.method");
        assert_eq!(sup.recv, RecvKind::Super);

        let arrow = classify_callee("obj->method");
        assert_eq!(arrow.recv, RecvKind::NamedVariable);

        let mac = classify_callee("println!");
        assert_eq!(mac.role, ReferenceKind::Macro);
        assert_eq!(mac.recv, RecvKind::None);

        let ts_nn = classify_callee("client!.load");
        assert_eq!(ts_nn.role, ReferenceKind::Call);
        assert_eq!(ts_nn.recv, RecvKind::NamedVariable);
        assert_eq!(ts_nn.recv_var.as_deref(), Some("client"));
        assert_eq!(ts_nn.method, "load");
    }

    #[test]
    // trace:v1 id=test.scc.recv.super-call verifies=REQ-implement-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-s exercises=impl.scc.recv.super-call
    fn python_super_call_is_super_receiver() {
        let sup = classify_callee("super().open");
        assert_eq!(sup.recv, RecvKind::Super);
        assert_eq!(sup.method, "open");
        assert_eq!(sup.root, "super");
    }
}
