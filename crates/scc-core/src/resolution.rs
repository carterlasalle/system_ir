//! Receiver classification, resolution honesty, and representation choice.
//!
//! These types are the extraction/resolution ontology. They do **not**
//! replace System Atlas entities, Reality Graph kinds, or provenance.
//! They describe how a reference was captured and how confidently it
//! was bound — so agents can see incomplete truth instead of silent
//! false certainty.

use serde::{Deserialize, Serialize};

/// How the callee's receiver was written at the call site.
///
/// Captured during extraction (or recovered from the callee string) so
/// resolution can distinguish `this.method()` from `other.method()`
/// without re-parsing. Field-chain receivers are classified rather than
/// silently treated as the first identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
// trace:v1 id=impl.scc.core.recv-kind work=WORK-ripwire-lessons-phase1 satisfies=REQ-receiver-aware-resolution
pub enum RecvKind {
    None,
    This,
    #[serde(rename = "SELF")]
    SelfRecv,
    NamedVariable,
    TypeQualified,
    FieldOfThis,
    FieldOfSelf,
    FieldOfVariable,
    Super,
    StaticType,
    #[default]
    Unknown,
}

// trace:exempt reason=internal-detail
impl RecvKind {
    // trace:exempt reason=internal-detail
    pub fn as_str(self) -> &'static str {
        match self {
            RecvKind::None => "NONE",
            RecvKind::This => "THIS",
            RecvKind::SelfRecv => "SELF",
            RecvKind::NamedVariable => "NAMED_VARIABLE",
            RecvKind::TypeQualified => "TYPE_QUALIFIED",
            RecvKind::FieldOfThis => "FIELD_OF_THIS",
            RecvKind::FieldOfSelf => "FIELD_OF_SELF",
            RecvKind::FieldOfVariable => "FIELD_OF_VARIABLE",
            RecvKind::Super => "SUPER",
            RecvKind::StaticType => "STATIC_TYPE",
            RecvKind::Unknown => "UNKNOWN_RECEIVER",
        }
    }

    /// Receivers whose method must be a sibling of the enclosing type.
    // trace:exempt reason=internal-detail
    pub fn is_instance_self(self) -> bool {
        matches!(self, RecvKind::This | RecvKind::SelfRecv)
    }

    /// Receivers that walk through an intermediate field. One-hop
    /// `self.x.m()` / `this.x.m()` may pin via a unique field type; longer
    /// chains stay unresolved and must not pretend the intermediate name
    /// is the method.
    // trace:exempt reason=internal-detail
    pub fn is_field_chain(self) -> bool {
        matches!(
            self,
            RecvKind::FieldOfThis | RecvKind::FieldOfSelf | RecvKind::FieldOfVariable
        )
    }
}

/// Outcome of native (non-LSP/SCIP) reference resolution.
///
/// `UnresolvedLikelyInternal` and `ConfirmedExternal` are distinct:
/// failing to find a target is not evidence that the target lives outside
/// the repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
// trace:v1 id=impl.scc.core.resolution-class work=WORK-ripwire-lessons-phase1 satisfies=REQ-resolution-honesty-gauges
pub enum ResolutionClass {
    ResolvedInternal,
    UnresolvedLikelyInternal,
    ConfirmedExternal,
    #[default]
    Unknown,
}

// trace:exempt reason=internal-detail
impl ResolutionClass {
    // trace:exempt reason=internal-detail
    pub fn as_str(self) -> &'static str {
        match self {
            ResolutionClass::ResolvedInternal => "resolved_internal",
            ResolutionClass::UnresolvedLikelyInternal => "unresolved_likely_internal",
            ResolutionClass::ConfirmedExternal => "confirmed_external",
            ResolutionClass::Unknown => "unknown",
        }
    }
}

/// Compact analyzer-health summary. Concise by default; counts are floors
/// of what this index actually observed.
/// Ordinal confidence ladder for call resolution. These are NOT calibrated
/// probabilities — no experiment measured them. They encode a deliberate
/// ranking (exact pins outrank typed-receiver matches outrank lexical
/// fallbacks), and every production emit site must use these names instead
/// of bare literals so the ranking stays reviewable in one place.
// trace:exempt reason=const-data
pub mod confidence {
    /// LSP definition resolution.
    pub const LSP_EXACT: f64 = 0.99;
    /// SCIP definition resolution (compiler-exact, same tier as LSP).
    pub const SCIP_EXACT: f64 = 0.99;
    /// Unique pin through a structural rule: bare-local callable, typed
    /// receiver, field type, Rule-1 enclosing class, Rule-3 include file,
    /// or fn-alias binding. Same tier as definition exactness: narrowing
    /// converged on exactly one target.
    pub const UNIQUE_PIN: f64 = 0.99;
    /// `self`/`this` method found on a sibling of the enclosing class.
    pub const SIBLING_METHOD: f64 = 0.98;
    /// Namespace-qualified member pin (`ns.member`, `pkg.Symbol`).
    pub const NAMESPACE_PIN: f64 = 0.97;
    /// Imported member pin (binding resolved through an import).
    pub const IMPORTED_MEMBER: f64 = 0.95;
    /// Typed-receiver tier: field-type and Rule-1 enclosing-class matches.
    /// Shares its value with unique pins; kept as a name so call sites
    /// state which rule family produced the edge.
    pub const TYPED_RECEIVER: f64 = 0.9;
    /// Confirmed external target (`external:` import, seeded root).
    pub const CONFIRMED_EXTERNAL: f64 = 0.8;
    /// Strong same-system edge below definition-exactness: bridge stitch,
    /// state-authority write. Structural, not pinned by a resolver rule.
    pub const STRONG_LINK: f64 = 0.8;
    /// Seeded field-chain root (object known, method not pinned).
    pub const SEEDED_ROOT: f64 = 0.55;
    /// No rule fired; recorded for coverage honesty, never an edge.
    pub const UNRESOLVED_FALLBACK: f64 = 0.5;
    /// Likely-internal target that no rule could pin.
    pub const UNRESOLVED_LIKELY_INTERNAL: f64 = 0.4;
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
// trace:v1 id=impl.scc.core.analysis-quality work=WORK-ripwire-lessons-phase1 satisfies=REQ-resolution-honesty-gauges
pub struct AnalysisQuality {
    pub calls: CallQuality,
    pub files: FileQuality,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub matched_doc_mentions: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unmatched_doc_mentions: u32,
}

// trace:exempt reason=internal-detail
fn is_zero(n: &u32) -> bool {
    *n == 0
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
// trace:v1 id=impl.scc.core.call-quality work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-NX53P4B7
pub struct CallQuality {
    pub resolved: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub precise: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub heuristic: u32,
    pub likely_internal_unresolved: u32,
    pub external: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unknown: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
// trace:v1 id=impl.scc.core.file-quality work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-NX53P4B7
pub struct FileQuality {
    pub parsed: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unsupported: u32,
}

// trace:exempt reason=internal-detail
impl AnalysisQuality {
// trace:v1 id=impl.scc.core.analysis-quality.record-call work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-NX53P4B7
    pub fn record_call(&mut self, class: ResolutionClass, precise: bool) {
        match class {
            ResolutionClass::ResolvedInternal => {
                self.calls.resolved += 1;
                if precise {
                    self.calls.precise += 1;
                } else {
                    self.calls.heuristic += 1;
                }
            }
            ResolutionClass::UnresolvedLikelyInternal => {
                self.calls.likely_internal_unresolved += 1
            }
            ResolutionClass::ConfirmedExternal => self.calls.external += 1,
            ResolutionClass::Unknown => self.calls.unknown += 1,
        }
    }

    /// One-line machine-readable summary for context packs.
    // trace:exempt reason=internal-detail
    pub fn merge(&mut self, other: &AnalysisQuality) {
        self.calls.resolved += other.calls.resolved;
        self.calls.precise += other.calls.precise;
        self.calls.heuristic += other.calls.heuristic;
        self.calls.likely_internal_unresolved += other.calls.likely_internal_unresolved;
        self.calls.external += other.calls.external;
        self.calls.unknown += other.calls.unknown;
        self.files.parsed += other.files.parsed;
        self.files.unsupported += other.files.unsupported;
        self.matched_doc_mentions += other.matched_doc_mentions;
        self.unmatched_doc_mentions += other.unmatched_doc_mentions;
    }

    // trace:exempt reason=internal-detail
    pub fn compact_line(&self) -> String {
        let mut line = format!(
            "calls: resolved={} precise={} heuristic={} likely_internal_unresolved={} external={} | files: parsed={} unsupported={}",
            self.calls.resolved,
            self.calls.precise,
            self.calls.heuristic,
            self.calls.likely_internal_unresolved,
            self.calls.external,
            self.files.parsed,
            self.files.unsupported,
        );
        if self.matched_doc_mentions > 0 || self.unmatched_doc_mentions > 0 {
            line.push_str(&format!(
                " | mentions: matched={} unmatched={}",
                self.matched_doc_mentions, self.unmatched_doc_mentions
            ));
        }
        line
    }
}

/// Body representation selected under the exact-source-dominance rule:
/// never spend more tokens to be clever.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
// trace:v1 id=impl.scc.core.representation-kind work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub enum RepresentationKind {
    Exact,
    Structural,
    Signatures,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// trace:v1 id=impl.scc.core.representation-choice work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct RepresentationChoice {
    pub kind: RepresentationKind,
    pub reason: String,
    pub exact_cost: usize,
    pub structural_cost: usize,
}

/// If exact source costs no more than the generated skeleton, serve exact.
// trace:v1 id=impl.scc.core.choose-representation work=WORK-ripwire-lessons-phase1 satisfies=REQ-exact-source-dominance
pub fn choose_representation(exact_cost: usize, structural_cost: usize) -> RepresentationChoice {
    if exact_cost <= structural_cost {
        RepresentationChoice {
            kind: RepresentationKind::Exact,
            reason: "exact_body_cheaper_than_structural".into(),
            exact_cost,
            structural_cost,
        }
    } else {
        let saved = structural_cost
            .saturating_mul(100)
            .checked_div(exact_cost)
            .map(|pct| 100usize.saturating_sub(pct))
            .unwrap_or(0);
        RepresentationChoice {
            kind: RepresentationKind::Structural,
            reason: format!("structural_saved_{saved}_percent"),
            exact_cost,
            structural_cost,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // trace:v1 id=test.scc.core.exact-source-dominance verifies=REQ-exact-source-dominance exercises=impl.scc.core.choose-representation
    fn exact_wins_when_not_more_expensive() {
        let c = choose_representation(40, 40);
        assert_eq!(c.kind, RepresentationKind::Exact);
        assert_eq!(c.reason, "exact_body_cheaper_than_structural");
        let cheaper = choose_representation(10, 80);
        assert_eq!(cheaper.kind, RepresentationKind::Exact);
    }

    #[test]
    // trace:v1 id=test.scc.core.structural-wins verifies=REQ-exact-source-dominance exercises=impl.scc.core.choose-representation
    fn structural_wins_when_it_saves_tokens() {
        let c = choose_representation(100, 29);
        assert_eq!(c.kind, RepresentationKind::Structural);
        assert!(c.reason.contains("structural_saved_"), "{}", c.reason);
    }

    #[test]
    // trace:v1 id=test.scc.core.analysis-quality-buckets verifies=REQ-resolution-honesty-gauges exercises=impl.scc.core.analysis-quality
    fn record_call_buckets_are_honest() {
        let mut q = AnalysisQuality::default();
        q.record_call(ResolutionClass::ResolvedInternal, true);
        q.record_call(ResolutionClass::ResolvedInternal, false);
        q.record_call(ResolutionClass::ConfirmedExternal, false);
        q.record_call(ResolutionClass::UnresolvedLikelyInternal, false);
        q.record_call(ResolutionClass::Unknown, false);
        assert_eq!(q.calls.resolved, 2);
        assert_eq!(q.calls.precise, 1);
        assert_eq!(q.calls.heuristic, 1);
        assert_eq!(q.calls.external, 1);
        assert_eq!(q.calls.likely_internal_unresolved, 1);
        assert_eq!(q.calls.unknown, 1);
        assert!(q.compact_line().contains("precise=1"));
        assert!(q.compact_line().contains("external=1"));
    }
}
