//! System IR core types.
//!
//! These types mirror `docs/system-ir.schema.json` exactly so that a `SystemIr`
//! document serializes to the documented export format without translation.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: &str = "0.1.0";

// ---------------------------------------------------------------------------
// Provenance
// ---------------------------------------------------------------------------

/// Evidence class of a fact, per docs/SYSTEM_IR_SCHEMA.md §5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Provenance {
    /// Direct syntax/configuration evidence.
    Extracted,
    /// Resolved through compiler/LSP/type/binding resolution.
    Resolved,
    /// Runtime evidence.
    Observed,
    /// Declared architectural intent.
    Declared,
    /// Heuristic/LLM claim.
    Inferred,
    /// Evidence no longer valid for the active revision.
    Stale,
}

impl Provenance {
    pub fn as_str(&self) -> &'static str {
        match self {
            Provenance::Extracted => "EXTRACTED",
            Provenance::Resolved => "RESOLVED",
            Provenance::Observed => "OBSERVED",
            Provenance::Declared => "DECLARED",
            Provenance::Inferred => "INFERRED",
            Provenance::Stale => "STALE",
        }
    }

    /// Default confidence per docs/SYSTEM_IR_SCHEMA.md §9.
    pub fn default_confidence(&self) -> f64 {
        match self {
            Provenance::Extracted => 1.0,
            Provenance::Resolved => 0.98,
            Provenance::Observed => 1.0,
            Provenance::Declared => 1.0,
            Provenance::Inferred => 0.7,
            Provenance::Stale => 0.0,
        }
    }

    /// STALE facts may never enter trusted context (only as warnings).
    pub fn is_trusted(&self) -> bool {
        !matches!(self, Provenance::Stale)
    }
}

// ---------------------------------------------------------------------------
// Severity / kinds
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    pub fn rank(&self) -> u8 {
        match self {
            Severity::Info => 0,
            Severity::Low => 1,
            Severity::Medium => 2,
            Severity::High => 3,
            Severity::Critical => 4,
        }
    }
}

/// Flow view kinds (System Atlas), per docs/SYSTEM_IR_SCHEMA.md §7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlowKind {
    Architecture,
    Workflow,
    Sequence,
    Dataflow,
    Lifecycle,
}

impl FlowKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            FlowKind::Architecture => "architecture",
            FlowKind::Workflow => "workflow",
            FlowKind::Sequence => "sequence",
            FlowKind::Dataflow => "dataflow",
            FlowKind::Lifecycle => "lifecycle",
        }
    }
}

pub fn flow_kind_str(k: &FlowKind) -> &'static str {
    k.as_str()
}

/// Evidence source type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EvidenceType {
    Source,
    Config,
    Runtime,
    Test,
    Intent,
    History,
}

// ---------------------------------------------------------------------------
// Archetype (Ontology phase — deterministic repo classification)
// ---------------------------------------------------------------------------

/// Repository archetype, detected deterministically from graph evidence
/// (routes, exports, cli/framework signals, deployment/workspace shape) by
/// `scc_graph::archetype::detect_archetype`. `Unknown` is the honest
/// fallback when no signal fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Archetype {
    /// HTTP routes + deployment units, no library-scale export ratio.
    ServiceApplication,
    /// cli-subcommand entrypoints or main fns with clap/cobra/argparse.
    Cli,
    /// Exported-symbol ratio over total symbols > 0.5, few/no routes.
    LibrarySdk,
    /// Routes + framework registrations + middleware facts.
    WebFramework,
    /// parse/analyze/transform/generate-style phase symbols.
    CompilerLanguageTool,
    /// plugin/middleware/DI registrations dominating.
    PluginFramework,
    /// docker/k8s/terraform manifests + deployment units, few app symbols.
    InfrastructureProject,
    /// workspace packages >= 3 + multiple deployment units.
    MonorepoPlatform,
    /// No signal fired.
    Unknown,
}

impl Archetype {
    pub fn as_str(&self) -> &'static str {
        match self {
            Archetype::ServiceApplication => "service_application",
            Archetype::Cli => "cli",
            Archetype::LibrarySdk => "library_sdk",
            Archetype::WebFramework => "web_framework",
            Archetype::CompilerLanguageTool => "compiler_language_tool",
            Archetype::PluginFramework => "plugin_framework",
            Archetype::InfrastructureProject => "infrastructure_project",
            Archetype::MonorepoPlatform => "monorepo_platform",
            Archetype::Unknown => "unknown",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Archetype::ServiceApplication => "service application",
            Archetype::Cli => "cli",
            Archetype::LibrarySdk => "library/sdk",
            Archetype::WebFramework => "web framework",
            Archetype::CompilerLanguageTool => "compiler/language tool",
            Archetype::PluginFramework => "plugin framework",
            Archetype::InfrastructureProject => "infrastructure project",
            Archetype::MonorepoPlatform => "monorepo platform",
            Archetype::Unknown => "unknown",
        }
    }

    /// All archetypes in the deterministic tie-break precedence order
    /// (first entry wins a score tie).
    pub const PRECEDENCE: [Archetype; 9] = [
        Archetype::MonorepoPlatform,
        Archetype::InfrastructureProject,
        Archetype::WebFramework,
        Archetype::ServiceApplication,
        Archetype::Cli,
        Archetype::LibrarySdk,
        Archetype::CompilerLanguageTool,
        Archetype::PluginFramework,
        Archetype::Unknown,
    ];
}

// ---------------------------------------------------------------------------
// Core records
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub indexed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
}

impl Entity {
    pub fn new(id: impl Into<String>, kind: impl Into<String>, name: impl Into<String>) -> Self {
        Entity {
            id: id.into(),
            kind: kind.into(),
            name: name.into(),
            attributes: BTreeMap::new(),
            evidence: Vec::new(),
        }
    }

    pub fn attr(&mut self, key: &str, value: impl Into<serde_json::Value>) -> &mut Self {
        self.attributes.insert(key.to_string(), value.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub id: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub provenance: Provenance,
    pub confidence: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub verified_at: String,
}

impl Relationship {
    pub fn new(
        id: impl Into<String>,
        subject: impl Into<String>,
        predicate: impl Into<String>,
        object: impl Into<String>,
        provenance: Provenance,
    ) -> Self {
        Relationship {
            id: id.into(),
            subject: subject.into(),
            predicate: predicate.into(),
            object: object.into(),
            provenance,
            confidence: provenance.default_confidence(),
            evidence: Vec::new(),
            verified_at: String::new(),
        }
    }

    pub fn with_confidence(mut self, c: f64) -> Self {
        self.confidence = c;
        self
    }

    pub fn with_evidence(mut self, evidence: Vec<String>) -> Self {
        self.evidence = evidence;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowStep {
    pub id: String,
    pub order: u32,
    pub actor: String,
    pub operation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#async: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_outcome: Option<String>,
    #[serde(default)]
    pub provenance: Option<Provenance>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flow {
    pub id: String,
    pub kind: FlowKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    pub steps: Vec<FlowStep>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invariant {
    pub id: String,
    pub statement: String,
    pub severity: Severity,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enforced_by: Vec<String>,
    #[serde(default)]
    pub provenance: Option<Provenance>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub id: String,
    #[serde(rename = "type")]
    pub r#type: EvidenceType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extractor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extractor_version: Option<String>,
}

impl Evidence {
    pub fn source(id: impl Into<String>, path: impl Into<String>) -> Self {
        Evidence {
            id: id.into(),
            r#type: EvidenceType::Source,
            path: Some(path.into()),
            symbol: None,
            start_line: None,
            end_line: None,
            revision: None,
            content_hash: None,
            extractor: None,
            extractor_version: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Canonical Causal FlowGraph (Wave 3 — the behavioral truth)
// ---------------------------------------------------------------------------

/// Edge kind in the canonical causal graph (P1, docs/SYSTEM_DESIGN.md §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlowEdgeKind {
    /// Sequential causality.
    Next,
    /// Alternative execution path (fanout from evidence: call fanout,
    /// exception handlers, task spawn, runtime trace variants, declared).
    Branch,
    /// Failure edge to an error/exception handler or failure outcome.
    Error,
    /// Retry edge (back to the retried operation).
    Retry,
    /// Fallback edge to the degraded path.
    Fallback,
    /// Asynchronous dispatch.
    Async,
    /// Message/event publication.
    Publish,
    /// Message/event consumption.
    Consume,
    /// Convergence of concurrent paths.
    Join,
    /// Return/terminal edge.
    Return,
    /// Timeout edge.
    Timeout,
    /// Compensation/rollback edge.
    Compensation,
}

/// One operation node in the canonical flow graph. The canonical graph
/// retains individual operations — component-level grouping (ComponentSpan)
/// happens only at display/context time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowNode {
    /// Index within the graph (0-based).
    pub id: u32,
    /// Actor entity id (symbol or component).
    pub actor: String,
    /// Operation label.
    pub operation: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowEdge {
    pub from: u32,
    pub to: u32,
    pub kind: FlowEdgeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(default)]
    pub provenance: Option<Provenance>,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
}

/// The canonical causal representation of one flow: a graph, never a
/// flattened linear step list. Alternate execution paths are preserved as
/// branch edges; false sequential causality is impossible by construction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowGraph {
    pub id: String,
    pub kind: FlowKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    #[serde(default)]
    pub nodes: Vec<FlowNode>,
    #[serde(default)]
    pub edges: Vec<FlowEdge>,
    /// Node indices that start the graph.
    #[serde(default)]
    pub entrypoints: Vec<u32>,
    /// Node indices with no outgoing causal edge (returns).
    #[serde(default)]
    pub exits: Vec<u32>,
    #[serde(default)]
    pub provenance_summary: BTreeMap<String, usize>,
}

// ---------------------------------------------------------------------------
// System Atlas (Wave 2 — the startup architecture artifact)
// ---------------------------------------------------------------------------

/// One architectural component in the atlas. Purpose is the highest-ranked
/// responsibility claim; consumes/produces come from data-flow edges;
/// upstream/downstream from dependency edges; retry/failure from extracted
/// failure behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtlasComponent {
    pub name: String,
    pub purpose: String,
    /// Implementation facts: directory paths AND member symbol names (the
    /// component compiler's `implementation` attribute carries both). The
    /// structured model exposes the full fact layer; the rendered atlas
    /// shows only [`AtlasComponent::implementation_paths`] to stay compact.
    #[serde(default)]
    pub implementation: Vec<String>,
    /// The directory-path subset of `implementation` — the compact view the
    /// rendered ARCHITECTURE block and IMPLEMENTATION MAP use.
    #[serde(default)]
    pub implementation_paths: Vec<String>,
    /// Member symbols attributed to the component (compile-time fact).
    #[serde(default)]
    pub symbols: Vec<String>,
    #[serde(default)]
    pub consumes: Vec<String>,
    #[serde(default)]
    pub produces: Vec<String>,
    #[serde(default)]
    pub upstream: Vec<String>,
    #[serde(default)]
    pub downstream: Vec<String>,
    #[serde(default)]
    pub failure_behavior: Vec<String>,
    #[serde(default)]
    pub owns: Vec<AtlasOwnershipClaim>,
    /// Architectural layer assigned by the hierarchy clusterer:
    /// `code_region | component | subsystem | service`.
    #[serde(default)]
    pub layer: String,
    /// Immediate container entity id (subsystem/service) for merged
    /// members; `None` for unmerged leaves.
    #[serde(default)]
    pub parent: Option<String>,
}

/// One hierarchical container (service or subsystem) with its direct member
/// entity ids (component ids, or subsystem ids nested inside a service).
/// Deterministic: `members` sorted by entity id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtlasHierarchyNode {
    /// Container entity id (`repo://…/service/…` or `repo://…/subsystem/…`).
    pub id: String,
    pub name: String,
    /// `"service"` | `"subsystem"`
    pub kind: String,
    #[serde(default)]
    pub members: Vec<String>,
}

/// A typed ownership claim (provenance preserved — DECLARED intent never
/// promoted).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtlasOwnershipClaim {
    pub target: String,
    pub provenance: String,
}

/// A condensed flow: steps collapsed to "Actor: operation" lines, with
/// branch/async/failure markers preserved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtlasFlow {
    pub name: String,
    pub kind: FlowKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    #[serde(default)]
    pub steps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtlasEntrypoint {
    pub name: String,
    pub kind: String,
    pub trigger: String,
    #[serde(default)]
    pub symbol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtlasInvariant {
    pub statement: String,
    pub severity: Severity,
}

/// One first-class contract in the atlas (Wave 9): a typed, evidence-backed
/// contract surface (http/cli/event/config/annotation) with its producer
/// symbol and the symbols that consume it. `operations` carries the concrete
/// contract strings (route `GET /api/x`, flag `--paging`, event
/// `user.created`, config key `DEBUG`, annotation `router.get`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contract {
    pub id: String,
    /// `"http" | "cli" | "event" | "config" | "annotation"`.
    pub kind: String,
    /// Producer entity id (handler symbol, owning symbol, topic, ...).
    #[serde(default)]
    pub producer: String,
    /// Consuming entity ids (symbols with HANDLES/CONSUMES/READS edges).
    #[serde(default)]
    pub consumers: Vec<String>,
    /// Concrete contract strings rendered as `{kind}: {operation}`.
    #[serde(default)]
    pub operations: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
}

impl Contract {
    pub fn new(
        id: impl Into<String>,
        kind: impl Into<String>,
        producer: impl Into<String>,
    ) -> Self {
        Contract {
            id: id.into(),
            kind: kind.into(),
            producer: producer.into(),
            consumers: Vec::new(),
            operations: Vec::new(),
            evidence: Vec::new(),
        }
    }
}

/// How a symbol can be invoked from outside the process (Wave 9): the
/// invocation surfaces the flow compiler seeds entrypoints from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationSurfaceKind {
    /// OS process spawn / executable entry.
    Process,
    /// HTTP route handler.
    Http,
    /// CLI subcommand / flag surface.
    Cli,
    /// Public API export (EXPORTS evidence).
    PublicApi,
    /// Event/topic handler.
    Event,
    /// Queue consumer (SUBSCRIBES evidence).
    Queue,
    /// Scheduled job.
    Schedule,
    /// Plugin/extension registration.
    Plugin,
    /// Framework callback (HANDLES_CALLBACK evidence).
    FrameworkCallback,
    /// Lifecycle callback (JUnit @Before*/@After* annotation facts).
    Lifecycle,
}

impl InvocationSurfaceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            InvocationSurfaceKind::Process => "process",
            InvocationSurfaceKind::Http => "http",
            InvocationSurfaceKind::Cli => "cli",
            InvocationSurfaceKind::PublicApi => "public_api",
            InvocationSurfaceKind::Event => "event",
            InvocationSurfaceKind::Queue => "queue",
            InvocationSurfaceKind::Schedule => "schedule",
            InvocationSurfaceKind::Plugin => "plugin",
            InvocationSurfaceKind::FrameworkCallback => "framework_callback",
            InvocationSurfaceKind::Lifecycle => "lifecycle",
        }
    }
}

/// One invocation surface: a symbol reachable from outside the process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvocationSurface {
    pub symbol: String,
    pub kind: InvocationSurfaceKind,
    pub trigger: String,
}

/// The full System Atlas: structured architecture before rendering. This is
/// the machine model handed to agents at session start (docs/SYSTEM_DESIGN.md
/// §8, Wave 2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemAtlas {
    pub repository: String,
    pub revision: String,
    pub indexed_at: String,
    pub freshness: String,
    pub purpose: String,
    #[serde(default)]
    pub components: Vec<AtlasComponent>,
    #[serde(default)]
    pub entrypoints: Vec<AtlasEntrypoint>,
    #[serde(default)]
    pub contracts: Vec<Contract>,
    /// Explicit uncertainty/coverage map (Wave 9): section key -> line.
    /// What the model knows AND what it does not.
    #[serde(default)]
    pub coverage: BTreeMap<String, String>,
    #[serde(default)]
    pub flows: Vec<AtlasFlow>,
    #[serde(default)]
    pub invariants: Vec<AtlasInvariant>,
    #[serde(default)]
    pub deployment_units: Vec<String>,
    #[serde(default)]
    pub external_systems: Vec<String>,
    #[serde(default)]
    pub trust_boundaries: Vec<String>,
    #[serde(default)]
    pub async_boundaries: Vec<String>,
    #[serde(default)]
    pub implementation_map: BTreeMap<String, Vec<String>>,
    /// Data stores / data entities written by components (WRITES-derived),
    /// rendered as a DATA STORES list under DATA OWNERSHIP.
    #[serde(default)]
    pub data_stores: Vec<String>,
    /// Detected repository archetype (deterministic evidence scoring).
    #[serde(default)]
    pub archetype: Option<Archetype>,
    /// STATE & DATA AUTHORITY (ontology phase): section key
    /// (persistent|runtime|configuration|caches|derived) -> deterministic
    /// `COMPONENT owns/reads TARGET (PROV)`-style lines.
    #[serde(default)]
    pub state_authority: BTreeMap<String, Vec<String>>,
    /// Hierarchical architecture containers (services first, then
    /// subsystems) with their direct member entity ids.
    #[serde(default)]
    pub hierarchy: Vec<AtlasHierarchyNode>,
    #[serde(default)]
    pub evidence_summary: BTreeMap<String, usize>,
    #[serde(default)]
    pub warnings: Vec<String>,
    /// PUBLIC API (Wave 10 COMPILER-gap attack): component name ->
    /// sorted public-export names (EXPORT entities + exported module-level
    /// symbols). Rendered as `component: exports...` compact lines.
    #[serde(default)]
    pub public_api: BTreeMap<String, Vec<String>>,
    /// FRAMEWORK SEMANTICS (Wave 10): component name -> sorted semantic
    /// lines (`annotates X`, `registers Y (kind)`, `handles callback Z`)
    /// from ANNOTATES/REGISTERS/HANDLES_CALLBACK facts.
    #[serde(default)]
    pub framework_semantics: BTreeMap<String, Vec<String>>,
    /// PIPELINE (Wave 10, CompilerLanguageTool archetype): phase-named
    /// symbols/files grouped by stage (`parse`/`analyze`/`transform`/
    /// `generate`/`emit`/`other`), rendered as `[stage] symbol` lines.
    #[serde(default)]
    pub pipeline: Vec<String>,
    /// LANDMARKS (Wave 10): notable exports + annotated targets, bounded
    /// (~40) — the informational one-zoom-deeper symbol list.
    #[serde(default)]
    pub landmarks: Vec<String>,
}

// ---------------------------------------------------------------------------
// Whole-document export
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemIr {
    pub schema_version: String,
    pub repository: Repository,
    pub snapshot: Snapshot,
    pub entities: Vec<Entity>,
    pub relationships: Vec<Relationship>,
    pub flows: Vec<Flow>,
    pub invariants: Vec<Invariant>,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
}

impl SystemIr {
    pub fn empty(repository: Repository, snapshot: Snapshot) -> Self {
        SystemIr {
            schema_version: SCHEMA_VERSION.to_string(),
            repository,
            snapshot,
            entities: Vec::new(),
            relationships: Vec::new(),
            flows: Vec::new(),
            invariants: Vec::new(),
            evidence: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Identifiers
// ---------------------------------------------------------------------------

/// Sanitize a free-form name into a stable URI key.
pub fn sanitize_key(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_dash = false;
    for c in input.chars() {
        let ok = c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '/';
        if ok {
            // keep path separators but normalize repeated dashes
            if c == '/' {
                out.push('/');
                prev_dash = false;
            } else if c == '-' || c == '_' {
                if !prev_dash {
                    out.push('-');
                }
                prev_dash = true;
            } else {
                out.push(c.to_ascii_lowercase());
                prev_dash = false;
            }
        } else {
            if !prev_dash {
                out.push('-');
            }
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("unnamed");
    }
    out
}

/// `repo://{repo}/{kind}/{key}` stable identifier.
pub fn entity_id(repo: &str, kind: &str, key: &str) -> String {
    format!("repo://{}/{}/{}", sanitize_key(repo), kind, sanitize_key(key))
}

/// Percent-encode a path/name component for use inside an entity id while
/// preserving case and common separators (`/`, `.`, `_`, `-`). Collision-free
/// where `sanitize_key` would risk merging distinct names.
pub fn encode_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        let keep = b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-');
        if keep {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{b:02X}"));
        }
    }
    out
}

/// Inverse of `encode_component`: percent-decodes `%XX` sequences back to
/// bytes. Used by benchmark/impact tooling to map entity ids back to names.
pub fn decode_component(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Stable symbol id: `repo://{repo}/symbol/{encoded-file}/{encoded-name}`.
pub fn symbol_id(repo: &str, file: &str, name: &str) -> String {
    format!(
        "repo://{}/symbol/{}/{}",
        sanitize_key(repo),
        encode_component(file),
        encode_component(name)
    )
}

/// Evidence id namespace: `evidence:{n}` — stable within a snapshot, assigned
/// by the store.
pub fn evidence_id(n: u64) -> String {
    format!("evidence:{n}")
}

/// Relationship id: `rel:{n}` — stable within a snapshot, assigned by store.
pub fn relationship_id(n: u64) -> String {
    format!("rel:{n}")
}

// ---------------------------------------------------------------------------
// Entity kind / predicate constants
// ---------------------------------------------------------------------------

pub mod kinds {
    pub const FILE: &str = "file";
    pub const SYMBOL: &str = "symbol";
    pub const PACKAGE: &str = "package";
    pub const MODULE: &str = "module";
    pub const SYSTEM: &str = "system";
    pub const SUBSYSTEM: &str = "subsystem";
    pub const SERVICE: &str = "service";
    pub const COMPONENT: &str = "component";
    pub const DEPLOYMENT_UNIT: &str = "deployment_unit";
    pub const ROUTE: &str = "route";
    pub const ENDPOINT: &str = "endpoint";
    pub const EVENT: &str = "event";
    pub const TOPIC: &str = "topic";
    pub const QUEUE: &str = "queue";
    pub const DATA_ENTITY: &str = "data";
    pub const DATA_STORE: &str = "store";
    pub const TABLE: &str = "table";
    pub const COLLECTION: &str = "collection";
    pub const CACHE: &str = "cache";
    pub const EXTERNAL_SYSTEM: &str = "external_system";
    pub const EXTERNAL_API: &str = "external_api";
    pub const CONFIGURATION: &str = "configuration";
    pub const FEATURE_FLAG: &str = "feature_flag";
    pub const SECRET_REFERENCE: &str = "secret_reference";
    pub const CONTRACT: &str = "contract";
    pub const INVARIANT: &str = "invariant";
    pub const TEST: &str = "test";
    pub const TEST_SUITE: &str = "test_suite";
    pub const FLOW: &str = "flow";
    pub const WORKFLOW: &str = "workflow";
    pub const STATE: &str = "state";
    // Semantic fact layer (Wave 9): first-class representations the
    // extractors emit for framework/library semantics.
    pub const EXPORT: &str = "export";
    pub const ANNOTATION: &str = "annotation";
    pub const FIELD: &str = "field";
    pub const REGISTRY: &str = "registry";
    pub const MIDDLEWARE: &str = "middleware";
    pub const DI_BINDING: &str = "di_binding";
    pub const TRANSITION: &str = "transition";
    pub const RESOURCE: &str = "resource";
    pub const TRUST_BOUNDARY: &str = "trust_boundary";
    pub const SECURITY_CONTROL: &str = "security_control";
    pub const RUNTIME_OBSERVATION: &str = "runtime_observation";
}

pub mod predicates {
    pub const CONTAINS: &str = "contains";
    pub const IMPLEMENTS: &str = "implements";
    pub const INHERITS: &str = "inherits";
    pub const IMPORTS: &str = "imports";
    pub const CALLS: &str = "calls";
    pub const READS: &str = "reads";
    pub const WRITES: &str = "writes";
    pub const QUERIES: &str = "queries";
    pub const OWNS: &str = "owns";
    pub const PUBLISHES: &str = "publishes";
    pub const CONSUMES: &str = "consumes";
    pub const SUBSCRIBES: &str = "subscribes";
    pub const PRODUCES: &str = "produces";
    pub const TRANSFORMS: &str = "transforms";
    pub const VALIDATES: &str = "validates";
    pub const ROUTES_TO: &str = "routes_to";
    pub const HANDLES: &str = "handles";
    pub const INVOKES: &str = "invokes";
    pub const DEPENDS_ON: &str = "depends_on";
    pub const DEPLOYED_WITH: &str = "deployed_with";
    pub const DEPLOYED_IN: &str = "deployed_in";
    pub const CONFIGURED_BY: &str = "configured_by";
    pub const PROTECTED_BY: &str = "protected_by";
    pub const CROSSES_BOUNDARY: &str = "crosses_boundary";
    pub const ENFORCES: &str = "enforces";
    pub const TESTED_BY: &str = "tested_by";
    pub const PARTICIPATES_IN: &str = "participates_in";
    pub const PRECEDES: &str = "precedes";
    pub const FOLLOWS: &str = "follows";
    pub const BRANCHES_TO: &str = "branches_to";
    pub const RETRIES: &str = "retries";
    pub const FALLS_BACK_TO: &str = "falls_back_to";
    pub const OBSERVED_AS: &str = "observed_as";
    pub const DECLARED_AS: &str = "declared_as";
    pub const IMPLEMENTED_BY: &str = "implemented_by";
    // Semantic fact layer (Wave 9)
    pub const EXPORTS: &str = "exports";
    pub const ANNOTATES: &str = "annotates";
    pub const REGISTERS: &str = "registers";
    pub const INJECTS: &str = "injects";
    pub const HANDLES_CALLBACK: &str = "handles_callback";
    pub const DECORATES: &str = "decorates";

    /// All predicates in the documented ontology.
    pub const ALL: &[&str] = &[
        CONTAINS, IMPLEMENTS, INHERITS, IMPORTS, CALLS, READS, WRITES, QUERIES, OWNS, PUBLISHES,
        CONSUMES, SUBSCRIBES, PRODUCES, TRANSFORMS, VALIDATES, ROUTES_TO, HANDLES, INVOKES,
        DEPENDS_ON, DEPLOYED_WITH, DEPLOYED_IN, CONFIGURED_BY, PROTECTED_BY, CROSSES_BOUNDARY,
        ENFORCES, TESTED_BY, PARTICIPATES_IN, PRECEDES, FOLLOWS, BRANCHES_TO, RETRIES,
        FALLS_BACK_TO, OBSERVED_AS, DECLARED_AS, IMPLEMENTED_BY,
    ];
}

// ---------------------------------------------------------------------------
// Token budgeting
// ---------------------------------------------------------------------------

/// Rough token estimate: 4 characters per token (byte-based for ASCII, but we
/// operate on char count which is a close approximation across scripts).
pub fn estimate_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    chars.div_ceil(4)
}

/// Hard-truncate `text` to at most `budget` tokens, preferring a clean cut at
/// a line boundary.
pub fn truncate_to_budget(text: &str, budget: usize) -> String {
    if estimate_tokens(text) <= budget {
        return text.to_string();
    }
    let max_chars = budget.saturating_mul(4);
    let mut end = 0;
    let mut chars = 0;
    for (i, c) in text.char_indices() {
        chars += 1;
        if chars > max_chars {
            break;
        }
        end = i + c.len_utf8();
    }
    // Back off to the previous newline for a clean boundary (but keep at least
    // half the budget worth of content).
    let min_chars = max_chars / 2;
    let mut cut = end;
    if let Some(nl) = text[..end].rfind('\n') {
        let prefix_chars = text[..nl].chars().count();
        if prefix_chars >= min_chars {
            cut = nl;
        }
    }
    let mut out = text[..cut].to_string();
    out.push_str("\n… (truncated by token budget)");
    out
}

// ---------------------------------------------------------------------------
// Misc
// ---------------------------------------------------------------------------

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_encode_decode_roundtrip() {
        for name in [
            "Normalizer",
            "GET /api/transcripts/:id handler",
            "OrderStateMachine.advance",
            "test_normalization_preserves_raw",
            "src/app.ts",
            "a b%c",
        ] {
            assert_eq!(decode_component(&encode_component(name)), name, "{name}");
        }
    }

    #[test]
    fn component_ids_are_collision_free() {
        assert_ne!(
            encode_component("foo_bar"),
            encode_component("foo-bar"),
            "underscore and dash must not collide"
        );
    }

    #[test]
    fn contract_kind_renders_as_operation_lines() {
        let mut c = Contract::new(
            "repo://repo/contract/http/get--api-x",
            "http",
            "repo://repo/symbol/main.py/handler",
        );
        c.operations.push("GET /api/x".into());
        c.consumers.push("repo://repo/symbol/main.py/handler".into());
        let lines: Vec<String> = c
            .operations
            .iter()
            .map(|op| format!("{}: {}", c.kind, op))
            .collect();
        assert_eq!(lines, vec!["http: GET /api/x"]);
    }

    #[test]
    fn invocation_surface_kinds_stringify() {
        assert_eq!(InvocationSurfaceKind::PublicApi.as_str(), "public_api");
        assert_eq!(InvocationSurfaceKind::Queue.as_str(), "queue");
        assert_eq!(InvocationSurfaceKind::Lifecycle.as_str(), "lifecycle");
        assert_eq!(InvocationSurfaceKind::FrameworkCallback.as_str(), "framework_callback");
        // serde roundtrip is snake_case-stable
        let json = serde_json::to_string(&InvocationSurfaceKind::PublicApi).unwrap();
        assert_eq!(json, "\"public_api\"");
        let back: InvocationSurfaceKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, InvocationSurfaceKind::PublicApi);
    }
}
