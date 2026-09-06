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

impl RecvKind {
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
    pub fn is_instance_self(self) -> bool {
        matches!(self, RecvKind::This | RecvKind::SelfRecv)
    }

    /// Receivers that walk through an intermediate field. Resolution must
    /// not pretend the intermediate name is the method.
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
    AmbiguousInternal,
    UnresolvedLikelyInternal,
    ConfirmedExternal,
    #[default]
    Unknown,
}

impl ResolutionClass {
    pub fn as_str(self) -> &'static str {
        match self {
            ResolutionClass::ResolvedInternal => "resolved_internal",
            ResolutionClass::AmbiguousInternal => "ambiguous_internal",
            ResolutionClass::UnresolvedLikelyInternal => "unresolved_likely_internal",
            ResolutionClass::ConfirmedExternal => "confirmed_external",
            ResolutionClass::Unknown => "unknown",
        }
    }
}

/// Compact analyzer-health summary. Concise by default; counts are floors
/// of what this index actually observed.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
// trace:v1 id=impl.scc.core.analysis-quality work=WORK-ripwire-lessons-phase1 satisfies=REQ-resolution-honesty-gauges
pub struct AnalysisQuality {
    pub calls: CallQuality,
    pub files: FileQuality,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub stale_facts_dropped: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CallQuality {
    pub resolved: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub precise: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub heuristic: u32,
    pub ambiguous: u32,
    pub likely_internal_unresolved: u32,
    pub external: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unknown: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FileQuality {
    pub parsed: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub partial: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unsupported: u32,
}

impl AnalysisQuality {
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
            ResolutionClass::AmbiguousInternal => self.calls.ambiguous += 1,
            ResolutionClass::UnresolvedLikelyInternal => {
                self.calls.likely_internal_unresolved += 1
            }
            ResolutionClass::ConfirmedExternal => self.calls.external += 1,
            ResolutionClass::Unknown => self.calls.unknown += 1,
        }
    }

    /// One-line machine-readable summary for context packs.
    pub fn merge(&mut self, other: &AnalysisQuality) {
        self.calls.resolved += other.calls.resolved;
        self.calls.precise += other.calls.precise;
        self.calls.heuristic += other.calls.heuristic;
        self.calls.ambiguous += other.calls.ambiguous;
        self.calls.likely_internal_unresolved += other.calls.likely_internal_unresolved;
        self.calls.external += other.calls.external;
        self.calls.unknown += other.calls.unknown;
        self.files.parsed += other.files.parsed;
        self.files.partial += other.files.partial;
        self.files.unsupported += other.files.unsupported;
        self.stale_facts_dropped += other.stale_facts_dropped;
    }

    pub fn compact_line(&self) -> String {
        format!(
            "calls: resolved={} precise={} heuristic={} ambiguous={} likely_internal_unresolved={} external={} | files: parsed={} partial={} unsupported={} | stale_facts_dropped={}",
            self.calls.resolved,
            self.calls.precise,
            self.calls.heuristic,
            self.calls.ambiguous,
            self.calls.likely_internal_unresolved,
            self.calls.external,
            self.files.parsed,
            self.files.partial,
            self.files.unsupported,
            self.stale_facts_dropped
        )
    }
}

/// Body representation selected under the exact-source-dominance rule:
/// never spend more tokens to be clever.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RepresentationKind {
    Exact,
    Structural,
    Signatures,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    fn structural_wins_when_it_saves_tokens() {
        let c = choose_representation(100, 29);
        assert_eq!(c.kind, RepresentationKind::Structural);
        assert!(c.reason.contains("structural_saved_"), "{}", c.reason);
    }

    #[test]
    // trace:v1 id=test.scc.core.analysis-quality-buckets verifies=REQ-resolution-honesty-gauges exercises=impl.scc.core.analysis-quality
    fn record_call_buckets_are_honest() {
        let mut q = AnalysisQuality::default();
        q.record_call(ResolutionClass::ResolvedInternal, false);
        q.record_call(ResolutionClass::ConfirmedExternal, false);
        q.record_call(ResolutionClass::UnresolvedLikelyInternal, false);
        q.record_call(ResolutionClass::AmbiguousInternal, false);
        assert_eq!(q.calls.resolved, 1);
        assert_eq!(q.calls.heuristic, 1);
        assert_eq!(q.calls.precise, 0);
        assert_eq!(q.calls.external, 1);
        assert_eq!(q.calls.likely_internal_unresolved, 1);
        assert_eq!(q.calls.ambiguous, 1);
        assert!(q.compact_line().contains("external=1"));
    }
}
