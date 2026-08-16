//! System IR core types.
//!
//! These types mirror `docs/system-ir.schema.json` exactly so that a `SystemIr`
//! document serializes to the documented export format without translation.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA_VERSION: &str = "0.1.0";

// ---------------------------------------------------------------------------
// Provenance
// ---------------------------------------------------------------------------

// trace:exempt reason=internal-detail

/// Evidence class of a fact, per docs/SYSTEM_IR_SCHEMA.md §5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
// trace:exempt reason=internal-detail
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

// trace:exempt reason=internal-detail

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
// trace:exempt reason=internal-detail
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

// trace:exempt reason=internal-detail

/// Flow view kinds (System Atlas), per docs/SYSTEM_IR_SCHEMA.md §7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
// trace:exempt reason=internal-detail
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

// trace:exempt reason=internal-detail

/// Repository archetype, detected deterministically from graph evidence
/// (routes, exports, cli/framework signals, deployment/workspace shape) by
/// `scc_graph::archetype::detect_archetype`. `Unknown` is the honest
/// fallback when no signal fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
// trace:exempt reason=internal-detail
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

// trace:exempt reason=internal-detail

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
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

// trace:exempt reason=internal-detail

/// One concrete occurrence of a concept (schema/reactive) in a source file.
///
/// Concept entities (SCHEMA/REACTIVE) are keyed globally by (kind, name) —
/// the same `z.object({...})` in A.ts and B.ts is ONE concept. Occurrences
/// carry per-(concept, path, owner, line) identity instead: each file's
/// occurrence survives independently, so provenance (`sources`) and the
/// derived occurrence count never collapse and a purge of one path never
/// deletes an occurrence another path still has.
#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct Occurrence {
    /// Stable entity id (see [`occurrence_id`]).
    pub id: String,
    /// The concept entity id this occurrence belongs to.
    pub concept: String,
    /// Repository-relative source path.
    pub path: String,
    /// Owning symbol name.
    pub owner: String,
    /// Deterministic site line (owning symbol's start line when the
    /// extractor facts carry no line).
    pub line: u32,
}

// trace:exempt reason=internal-detail

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
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

// trace:exempt reason=internal-detail

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
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

// trace:exempt reason=internal-detail

/// First-class contract subclass (Contract ontology): the semantic contract
/// family, derived by the extractors from general evidence (public fn
/// signatures, builder/factory structure, event producer/consumer pairs,
/// serializer/deserializer pairs, interface+implementations, route/flag/
/// topic/config facts) and rendered by the atlas as per-subclass groups.
/// The legacy `kind` string stays for back-compat; `subclass` is the typed
/// family (`http`/`cli`/`event`/`config`/`public-api`/`extension`/
/// `serialization`/...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
// trace:exempt reason=internal-detail
pub enum ContractSubclass {
    /// A callable contract surface (framework callback, task, annotated
    /// handler, middleware): the framework invokes this callable.
    CallContract,
    /// Public API export (function/method signature surface).
    PublicApi,
    /// HTTP route.
    #[default]
    Http,
    /// RPC method.
    Rpc,
    /// CLI flag / subcommand.
    Cli,
    /// Event (topic with producers/consumers).
    Event,
    /// Message / queue surface.
    Message,
    /// Schema definition (validation/model schema).
    Schema,
    /// Configuration key.
    Configuration,
    /// Plugin registration.
    Plugin,
    /// Extension point: interface + implementations.
    Extension,
    /// Serialization pair (serializer/deserializer around a type).
    Serialization,
}

impl ContractSubclass {
    /// Render prefix used by the atlas CONTRACTS section (stable, sorted:
    /// `http: GET /x`, `cli: --flag`, `event: user.created`, `config: DEBUG`,
    /// `public-api: Class.method`, `extension: PluginX`, `serialization:
    /// toJson/fromJson`).
    pub fn as_str(&self) -> &'static str {
        match self {
            ContractSubclass::CallContract => "call",
            ContractSubclass::PublicApi => "public-api",
            ContractSubclass::Http => "http",
            ContractSubclass::Rpc => "rpc",
            ContractSubclass::Cli => "cli",
            ContractSubclass::Event => "event",
            ContractSubclass::Message => "message",
            ContractSubclass::Schema => "schema",
            ContractSubclass::Configuration => "config",
            ContractSubclass::Plugin => "plugin",
            ContractSubclass::Extension => "extension",
            ContractSubclass::Serialization => "serialization",
        }
    }

    /// Map a contract/registration kind string to its first-class subclass.
    /// `None` for framework-specific registration kinds (`include_router`,
    /// `add_middleware`, ...) that stay framework semantics instead of
    /// first-class contracts. `factory` → PublicApi and `builder` →
    /// Configuration are the ontology's builder/factory rule.
    pub fn from_kind_str(kind: &str) -> Option<ContractSubclass> {
        Some(match kind {
            "http" | "route" => ContractSubclass::Http,
            "cli" => ContractSubclass::Cli,
            "event" | "topic" => ContractSubclass::Event,
            "config" | "configuration" | "next-config" => ContractSubclass::Configuration,
            "factory" | "export" | "public-api" | "public_api" => ContractSubclass::PublicApi,
            "builder" => ContractSubclass::Configuration,
            "serialization" | "serialize" | "deserialize" => ContractSubclass::Serialization,
            "extension" => ContractSubclass::Extension,
            "plugin" => ContractSubclass::Plugin,
            "rpc" => ContractSubclass::Rpc,
            "message" | "queue" => ContractSubclass::Message,
            "schema" => ContractSubclass::Schema,
            "call" | "task" | "bean" | "rule" | "middleware" | "callback" => {
                ContractSubclass::CallContract
            }
            _ => return None,
        })
    }
}

// trace:exempt reason=internal-detail

/// One first-class contract in the atlas (Wave 9): a typed, evidence-backed
/// contract surface (http/cli/event/config/annotation) with its producer
/// symbol and the symbols that consume it. `operations` carries the concrete
/// contract strings (route `GET /api/x`, flag `--paging`, event
/// `user.created`, config key `DEBUG`, annotation `router.get`).
#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct Contract {
    pub id: String,
    /// `"http" | "cli" | "event" | "config" | "annotation"`.
    pub kind: String,
    /// Semantic contract subclass (Contract ontology): the typed family
    /// derived from general evidence — http/cli/event/config from
    /// route/flag/topic/config facts, public-api from exported fn
    /// signatures, serialization from serializer/deserializer pairs,
    /// extension from interface+implementations, and extractor-emitted
    /// registration kinds mapped by `ContractSubclass::from_kind_str`.
    #[serde(default)]
    pub subclass: ContractSubclass,
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
            subclass: ContractSubclass::default(),
            producer: producer.into(),
            consumers: Vec::new(),
            operations: Vec::new(),
            evidence: Vec::new(),
        }
    }

    /// Set the semantic subclass (builder-style; the atlas sets it on the
    /// typed families it derives from entity kinds).
    pub fn with_subclass(mut self, subclass: ContractSubclass) -> Self {
        self.subclass = subclass;
        self
    }
}

// trace:exempt reason=internal-detail

/// How a symbol can be invoked from outside the process (Wave 9): the
/// invocation surfaces the flow compiler seeds entrypoints from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
// trace:exempt reason=internal-detail
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

// trace:exempt reason=internal-detail

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
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

/// Occurrence entity id: collision-free per (concept key, path, owner,
/// line) — the identity occurrences carry so shared concepts never lose
/// per-file provenance. Unlike [`entity_id`], the concept/path/owner
/// components are percent-encoded (`@`-separated), so case and separator
/// distinctions (`a_b` vs `a-b`) never merge distinct occurrences.
// trace:v1 id=impl.scc.core.occurrence work=WORK-SCC-001 satisfies=REQ-SCC-IR
pub fn occurrence_id(repo: &str, concept: &str, path: &str, owner: &str, line: u32) -> String {
    format!(
        "repo://{}/occurrence/{}@{}@{}@{}",
        sanitize_key(repo),
        encode_component(concept),
        encode_component(path),
        encode_component(owner),
        line
    )
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
    // Wave 11: first-class schema and reactive-state contracts.
    pub const SCHEMA: &str = "schema";
    pub const REACTIVE: &str = "reactive";
    // Occurrence layer: one entity per (concept, path, owner, line) so
    // shared concepts never lose per-file provenance (Wave 13).
    pub const OCCURRENCE: &str = "occurrence";
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
    pub const DEFINES: &str = "defines";
    pub const COMPOSES: &str = "composes";
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
    /// An occurrence entity's attachment to its concept entity
    /// (occurrence OCCURS concept).
    pub const OCCURS: &str = "occurs";

    /// All predicates in the documented ontology.
    pub const ALL: &[&str] = &[
        CONTAINS, IMPLEMENTS, INHERITS, IMPORTS, CALLS, READS, WRITES, QUERIES, OWNS, PUBLISHES,
        CONSUMES, SUBSCRIBES, PRODUCES, TRANSFORMS, VALIDATES, ROUTES_TO, HANDLES, INVOKES,
        DEPENDS_ON, DEPLOYED_WITH, DEPLOYED_IN, CONFIGURED_BY, PROTECTED_BY, CROSSES_BOUNDARY,
        ENFORCES, TESTED_BY, PARTICIPATES_IN, PRECEDES, FOLLOWS, BRANCHES_TO, RETRIES,
        FALLS_BACK_TO, OBSERVED_AS, DECLARED_AS, IMPLEMENTED_BY, OCCURS,
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

// ---------------------------------------------------------------------------
// Wave 14: System Surface Map — the actual callable code surface (Aider
// RepoMap equivalent built from System IR). Level 1 of the four-level
// context stack (docs/SYSTEM_DESIGN.md Wave 14).
// ---------------------------------------------------------------------------

/// A source range: file path + 1-based inclusive line span.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct SourceRange {
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
}

// trace:exempt reason=internal-detail
impl SourceRange {
// trace:exempt reason=internal-detail
    pub fn new(path: impl Into<String>, start_line: u32, end_line: u32) -> Self {
        SourceRange {
            path: path.into(),
            start_line,
            end_line,
        }
    }
}

/// Symbol visibility as declared in source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
// trace:exempt reason=internal-detail
pub enum Visibility {
    Public,
    Protected,
    Private,
    Package,
}

// trace:exempt reason=internal-detail
impl Visibility {
// trace:exempt reason=internal-detail
    pub fn as_str(&self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::Protected => "protected",
            Visibility::Private => "private",
            Visibility::Package => "package",
        }
    }
}

/// The kind of code surface a definition exposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
// trace:exempt reason=internal-detail
pub enum SurfaceKind {
    Function,
    Method,
    Constructor,
    Class,
    Interface,
    Trait,
    Type,
    Enum,
    Const,
    Module,
    Record,
}

// trace:exempt reason=internal-detail
impl SurfaceKind {
// trace:exempt reason=internal-detail
    pub fn as_str(&self) -> &'static str {
        match self {
            SurfaceKind::Function => "function",
            SurfaceKind::Method => "method",
            SurfaceKind::Constructor => "constructor",
            SurfaceKind::Class => "class",
            SurfaceKind::Interface => "interface",
            SurfaceKind::Trait => "trait",
            SurfaceKind::Type => "type",
            SurfaceKind::Enum => "enum",
            SurfaceKind::Const => "const",
            SurfaceKind::Module => "module",
            SurfaceKind::Record => "record",
        }
    }
}

/// One function/method parameter in structured form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct SemanticParameter {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<String>,
    /// `&self` / `self` receiver parameters.
    #[serde(default)]
    pub receiver: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default)]
    pub variadic: bool,
}

/// The structured machine form of a signature — the semantic layer over
/// the exact source text. Benchmark matching uses this, never string
/// comparisons of source signatures alone.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct SemanticSignature {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default)]
    pub visibility: Option<Visibility>,
    #[serde(default)]
    pub async_: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub generic_parameters: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<SemanticParameter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returns: Option<String>,
    /// `where` / trait-bound constraints (`T: Send + Sync`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<String>,
}

/// Why a surface entry earned its rank (explainability; `scc surface
/// --explain` renders this).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct SurfaceRank {
    pub task_ppr: f64,
    pub global_ppr: f64,
    pub lexical: f64,
    pub semantic: f64,
    pub confidence: f64,
    pub criticality: f64,
    pub change_risk: f64,
    pub novelty: f64,
    pub total: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
}

// trace:exempt reason=internal-detail
impl Default for SurfaceRank {
// trace:exempt reason=internal-detail
    fn default() -> Self {
        SurfaceRank {
            task_ppr: 0.0,
            global_ppr: 0.0,
            lexical: 0.0,
            semantic: 0.0,
            confidence: 0.0,
            criticality: 0.0,
            change_risk: 0.0,
            novelty: 0.0,
            total: 0.0,
            reasons: Vec::new(),
        }
    }
}

/// One ranked definition on the system surface — a callable/typeable
/// reality of the architecture, with exact signatures and architectural
/// meaning attached.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct SurfaceEntry {
    pub id: String,
    pub symbol_id: String,
    pub qualified_name: String,
    pub kind: SurfaceKind,
    pub path: String,
    pub range: SourceRange,
    /// Exact source representation of the signature.
    pub source_signature: String,
    /// Whitespace/dialect-normalized signature (dedupe/comparison/index).
    pub canonical_signature: String,
    pub semantic_signature: SemanticSignature,
    pub visibility: Visibility,
    #[serde(default)]
    pub exported: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subsystem: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flows: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contracts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub state_authorities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invocation_surfaces: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub callers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub callees: Vec<String>,
    pub provenance: Provenance,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub rank: SurfaceRank,
}

/// A definition deliberately omitted by a token-budget cut — the artifact
/// never silently implies completeness.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct SurfaceOmission {
    pub count: usize,
    pub kind: String,
    pub reason: String,
}

/// The System Surface Map: the ranked actual-API layer of a repository,
/// built from System IR (Level 1 of the context stack).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct SystemSurfaceMap {
    pub repository: String,
    pub revision: String,
    pub epoch: String,
    pub entries: Vec<SurfaceEntry>,
    #[serde(default)]
    pub token_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub omitted: Vec<SurfaceOmission>,
}

/// The production surface render: the budget-selected subset plus honest
/// omission accounting (Wave 14F). `rendered_ids` are exactly the entries
/// the agent sees (ledger recording MUST use only these — omitted
/// candidates are never marked visible); `omitted_ids` are every candidate
/// the pipeline cut. `omissions` summarizes the cuts by kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct SurfaceRenderResult {
    /// The rendered surface text (header + selected entry blocks).
    pub text: String,
    /// Entry ids actually rendered, in selection order.
    pub rendered_ids: Vec<String>,
    /// Candidate entry ids the pipeline omitted (budget/quotas/diversity).
    pub omitted_ids: Vec<String>,
    /// Per-kind omission summaries.
    pub omissions: Vec<SurfaceOmission>,
    /// Token estimate of `text`.
    pub token_count: usize,
}

/// One node in the heterogeneous ranking universe (Wave 14B): any rankable
/// entity — symbol, component, subsystem, service, flow, contract, state,
/// reactive, route, topic, queue, store, schema, or file. The ranker walks
/// edges whose endpoints are rankable entities, so architectural importance
/// (flows, contracts, state) participates in PageRank directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct RankNode {
    /// The entity id (`repo://{repo}/{kind}/{key}`).
    pub id: String,
    /// Entity kind (`scc_core::kinds::*`).
    pub kind: String,
    /// Entity display name.
    pub name: String,
}

/// A normalized reference between two symbols (Wave 14): the graph the
/// ranker walks. Many SCC relationships already express these concepts;
/// this layer normalizes them for ranking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct ReferenceEdge {
    pub source_symbol: String,
    pub target_symbol: String,
    pub kind: ReferenceKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locations: Vec<SourceRange>,
    pub provenance: Provenance,
    #[serde(default)]
    pub confidence: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
// trace:exempt reason=internal-detail
pub enum ReferenceKind {
    Read,
    Write,
    Call,
    TypeUse,
    Instantiate,
    Implement,
    Extend,
    Decorate,
    Register,
    Import,
    Export,
}

// trace:exempt reason=internal-detail
impl ReferenceKind {
// trace:exempt reason=internal-detail
    pub fn as_str(&self) -> &'static str {
        match self {
            ReferenceKind::Read => "read",
            ReferenceKind::Write => "write",
            ReferenceKind::Call => "call",
            ReferenceKind::TypeUse => "type_use",
            ReferenceKind::Instantiate => "instantiate",
            ReferenceKind::Implement => "implement",
            ReferenceKind::Extend => "extend",
            ReferenceKind::Decorate => "decorate",
            ReferenceKind::Register => "register",
            ReferenceKind::Import => "import",
            ReferenceKind::Export => "export",
        }
    }
}

/// A token-optimized context candidate (Aider-style hard budget search).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct ContextItem {
    pub id: String,
    pub value: f64,
    pub token_cost: usize,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

/// The startup/task context budget split (Wave 14 dynamic budgets).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct ContextBudget {
    pub total: usize,
    pub atlas: usize,
    pub surface: usize,
    pub task_delta: usize,
    pub structural_source: usize,
}

// trace:exempt reason=internal-detail
impl Default for ContextBudget {
// trace:exempt reason=internal-detail
    fn default() -> Self {
        ContextBudget {
            total: 20_000,
            atlas: 13_000,
            surface: 7_000,
            task_delta: 3_000,
            structural_source: 6_000,
        }
    }
}

/// Absolute ceiling on the massive-tier surface slice: a 20k-entity repo
/// must not hand the model an unbounded surface even under a huge total.
const MASSIVE_SURFACE_CAP: usize = 10_000;

// trace:exempt reason=internal-detail  # impl grouping; adaptive below is traced
impl ContextBudget {
    /// Adaptive startup split: scale the Atlas/Surface allocation by repo
    /// complexity instead of the fixed 13:7 default. Tiers by entity count
    /// (component count also escalates to `large`):
    ///
    /// - tiny (`entity_count < 200`): 55/45 — a small atlas leaves room
    ///   for a proportionally larger surface;
    /// - normal: 60/40;
    /// - large (`entity_count > 5000` or `component_count > 30`): 65/35 —
    ///   the atlas dominates;
    /// - massive (`entity_count > 20_000`): 70/30 with the surface slice
    ///   capped absolutely ([`MASSIVE_SURFACE_CAP`]) and no candidate
    ///   boost.
    ///
    /// Within a tier, the actual candidate pool feeds the surface share:
    /// every 2,000 surface candidates earns up to +5 percentage points
    /// (surface never over 50% of the total), so a repo whose surface map
    /// is genuinely large gets a proportionally larger surface slice.
    /// `total` is caller-supplied — adaptive scales the SPLIT, never the
    /// total. Deterministic and no-panic. Defaults stay for callers that
    /// do not adapt.
    // trace:v1 id=impl.scc.core.budget-adaptive work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching satisfies=REQ-adaptive-startup-budgets
    pub fn adaptive(
        total: usize,
        entity_count: usize,
        component_count: usize,
        _flow_count: usize,
        surface_candidates: usize,
    ) -> ContextBudget {
        let (surface_pct, boost, cap): (f64, f64, Option<usize>) = if entity_count > 20_000 {
            (30.0, 0.0, Some(MASSIVE_SURFACE_CAP))
        } else if entity_count > 5_000 || component_count > 30 {
            (35.0, 5.0, None)
        } else if entity_count < 200 {
            (45.0, 5.0, None)
        } else {
            (40.0, 5.0, None)
        };
        let boost_pp = (surface_candidates / 2_000).min(boost as usize) as f64;
        let surface_pct = (surface_pct + boost_pp).min(50.0);
        let atlas_pct = 100.0 - surface_pct;
        let mut surface = ((total as f64) * surface_pct / 100.0).round() as usize;
        if let Some(c) = cap {
            surface = surface.min(c);
        }
        let atlas = ((total as f64) * atlas_pct / 100.0).round() as usize;
        let def = ContextBudget::default();
        ContextBudget {
            total,
            atlas,
            surface,
            task_delta: def.task_delta,
            structural_source: def.structural_source,
        }
    }
}

/// What the agent has already seen — novelty suppression source (the
/// general form of Aider treating chat files specially).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct ContextLedger {
    pub model_epoch: String,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub visible_entities: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub visible_symbols: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub visible_files: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub visible_components: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub visible_flows: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_task: Option<String>,
}

/// One structural-source unit: semantic skeleton of an implementation
/// slice (Level 2), with provenance back to the exact source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct StructuralSourceUnit {
    pub path: String,
    /// `source: <path>:L<start>-L<end>` provenance line.
    pub source: String,
    pub representation: String,
    pub revision: String,
    pub content: String,
}

/// The deterministic startup artifact (Atlas + Surface), hash-stable per
/// epoch so prompt caches hit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct ContextArtifact {
    pub kind: String,
    pub epoch: String,
    pub renderer_version: String,
    pub trust_policy: String,
    pub budget: ContextBudget,
    /// Deterministic config-only hash (epoch + renderer + policy + budget) —
    /// the prompt-cache key, stable per epoch. Field name kept for the JSON
    /// contract.
    pub sha256: String,
    /// Hash over the *actual rendered content* (config preimage + rendered
    /// text), so a content change that keeps the config identical still
    /// changes the hash (the audit's name/content mismatch fix).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content_hash: String,
    pub text: String,
}

/// One task-seed resolution: task language -> SCC entities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct TaskSeed {
    pub kind: String,
    pub id: String,
    pub weight: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
// trace:exempt reason=unit-test
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
// trace:exempt reason=unit-test
    fn component_ids_are_collision_free() {
        assert_ne!(
            encode_component("foo_bar"),
            encode_component("foo-bar"),
            "underscore and dash must not collide"
        );
    }

// trace:exempt reason=internal-detail

    #[test]
// trace:exempt reason=internal-detail
    fn occurrence_ids_are_collision_free_per_site() {
        // distinct paths/owners/lines/concepts never merge
        assert_ne!(
            occurrence_id("r", "expr", "a.ts", "A", 1),
            occurrence_id("r", "expr", "b.ts", "A", 1)
        );
        assert_ne!(
            occurrence_id("r", "expr", "a.ts", "A", 1),
            occurrence_id("r", "expr", "a.ts", "B", 1)
        );
        assert_ne!(
            occurrence_id("r", "expr", "a.ts", "A", 1),
            occurrence_id("r", "expr", "a.ts", "A", 2)
        );
        assert_ne!(
            occurrence_id("r", "expr1", "a.ts", "A", 1),
            occurrence_id("r", "expr2", "a.ts", "A", 1)
        );
        // case/separator distinctions survive (unlike sanitize_key)
        assert_ne!(
            occurrence_id("r", "expr", "a_b.ts", "A", 1),
            occurrence_id("r", "expr", "a-b.ts", "A", 1)
        );
        assert_ne!(
            occurrence_id("r", "expr", "a.ts", "Foo", 1),
            occurrence_id("r", "expr", "a.ts", "foo", 1)
        );
        // deterministic
        assert_eq!(
            occurrence_id("r", "expr", "a.ts", "A", 1),
            occurrence_id("r", "expr", "a.ts", "A", 1)
        );
    }

// trace:exempt reason=internal-detail

    #[test]
// trace:exempt reason=internal-detail
    fn occurrence_roundtrips_through_serde() {
        let o = Occurrence {
            id: occurrence_id("r", "z.object({ x: z.string() })", "src/a.ts", "make", 7),
            concept: entity_id("r", kinds::SCHEMA, "z.object({ x: z.string() })"),
            path: "src/a.ts".into(),
            owner: "make".into(),
            line: 7,
        };
        let json = serde_json::to_string(&o).unwrap();
        let back: Occurrence = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, o.id);
        assert_eq!(back.concept, o.concept);
        assert_eq!(back.line, 7);
    }

// trace:exempt reason=internal-detail

    #[test]
// trace:exempt reason=internal-detail
    fn adaptive_budget_scales_split_by_repo_complexity() {
        // tiny repo: 55/45 split — a small atlas leaves room for a
        // proportionally larger surface (45% share, vs the normal 40%)
        let tiny = ContextBudget::adaptive(20_000, 50, 2, 1, 10);
        let tiny_ratio = tiny.surface as f64 / tiny.total as f64;
        assert!(
            (tiny_ratio - 0.45).abs() <= 0.01,
            "tiny: surface share {tiny_ratio}"
        );
        assert_eq!(tiny.total, 20_000);
        assert!(tiny.atlas + tiny.surface <= 20_000);

        // normal repo: 60/40
        let normal = ContextBudget::adaptive(20_000, 1_000, 5, 4, 10);
        let normal_ratio = normal.surface as f64 / normal.total as f64;
        assert!(
            (normal_ratio - 0.40).abs() <= 0.01,
            "normal: surface share {normal_ratio}"
        );

        // large repo (component-heavy): 65/35
        let large = ContextBudget::adaptive(20_000, 1_000, 40, 4, 10);
        let large_ratio = large.surface as f64 / large.total as f64;
        assert!(
            (large_ratio - 0.35).abs() <= 0.01,
            "large: surface share {large_ratio}"
        );

        // massive repo: 70/30 and the surface slice is absolutely capped
        let massive = ContextBudget::adaptive(20_000, 25_000, 60, 30, 10);
        assert!(massive.atlas > massive.surface);
        let massive_ratio = massive.surface as f64 / massive.total as f64;
        assert!(
            (massive_ratio - 0.30).abs() <= 0.01,
            "massive: surface share {massive_ratio}"
        );
        // the absolute cap binds under a huge total
        let massive_huge = ContextBudget::adaptive(100_000, 25_000, 60, 30, 10);
        assert_eq!(
            massive_huge.surface, MASSIVE_SURFACE_CAP,
            "massive surface absolutely capped"
        );

        // tiny and large must produce different atlas/surface splits
        assert_ne!(
            tiny.atlas as f64 / tiny.surface as f64,
            large.atlas as f64 / large.surface as f64,
            "tiny vs large splits must differ"
        );
    }

    #[test]
// trace:exempt reason=internal-detail
    fn adaptive_budget_candidate_pool_boosts_surface_share() {
        // 10k candidates: +5pp surface share (10_000 / 2_000 = 5, capped at 5)
        let boosted = ContextBudget::adaptive(20_000, 1_000, 5, 4, 10_000);
        let boosted_ratio = boosted.surface as f64 / boosted.total as f64;
        assert!(
            (boosted_ratio - 0.45).abs() <= 0.01,
            "boosted: surface share {boosted_ratio}"
        );
        // surface never over 50% of the total
        let huge = ContextBudget::adaptive(20_000, 100, 2, 1, 100_000);
        assert!(huge.surface <= huge.total / 2, "surface capped at 50%");
    }

    #[test]
// trace:exempt reason=internal-detail
    fn adaptive_budget_is_deterministic_and_total_preserving() {
        let a = ContextBudget::adaptive(15_000, 3_000, 8, 6, 500);
        let b = ContextBudget::adaptive(15_000, 3_000, 8, 6, 500);
        assert_eq!(a, b);
        assert_eq!(a.total, 15_000);
        assert!(a.atlas + a.surface <= 15_000);
    }

    #[test]
// trace:exempt reason=unit-test
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
// trace:exempt reason=unit-test
    fn contract_subclass_ontology_maps_and_renders() {
        // Render prefixes per-subclass (the atlas CONTRACTS group prefixes).
        assert_eq!(ContractSubclass::Http.as_str(), "http");
        assert_eq!(ContractSubclass::Cli.as_str(), "cli");
        assert_eq!(ContractSubclass::Event.as_str(), "event");
        assert_eq!(ContractSubclass::Configuration.as_str(), "config");
        assert_eq!(ContractSubclass::PublicApi.as_str(), "public-api");
        assert_eq!(ContractSubclass::Extension.as_str(), "extension");
        assert_eq!(ContractSubclass::Serialization.as_str(), "serialization");
        assert_eq!(ContractSubclass::CallContract.as_str(), "call");
        assert_eq!(ContractSubclass::Rpc.as_str(), "rpc");
        assert_eq!(ContractSubclass::Message.as_str(), "message");
        assert_eq!(ContractSubclass::Schema.as_str(), "schema");
        assert_eq!(ContractSubclass::Plugin.as_str(), "plugin");

        // Derivation from contract/registration kind strings (the ontology
        // mapping the atlas applies to extractor-emitted facts).
        assert_eq!(ContractSubclass::from_kind_str("http"), Some(ContractSubclass::Http));
        assert_eq!(ContractSubclass::from_kind_str("route"), Some(ContractSubclass::Http));
        assert_eq!(ContractSubclass::from_kind_str("cli"), Some(ContractSubclass::Cli));
        assert_eq!(ContractSubclass::from_kind_str("event"), Some(ContractSubclass::Event));
        assert_eq!(
            ContractSubclass::from_kind_str("config"),
            Some(ContractSubclass::Configuration)
        );
        assert_eq!(
            ContractSubclass::from_kind_str("next-config"),
            Some(ContractSubclass::Configuration)
        );
        // builder = Configuration, factory = PublicApi (the ontology rule).
        assert_eq!(
            ContractSubclass::from_kind_str("builder"),
            Some(ContractSubclass::Configuration)
        );
        assert_eq!(
            ContractSubclass::from_kind_str("factory"),
            Some(ContractSubclass::PublicApi)
        );
        assert_eq!(
            ContractSubclass::from_kind_str("serialization"),
            Some(ContractSubclass::Serialization)
        );
        assert_eq!(
            ContractSubclass::from_kind_str("extension"),
            Some(ContractSubclass::Extension)
        );
        assert_eq!(ContractSubclass::from_kind_str("plugin"), Some(ContractSubclass::Plugin));
        assert_eq!(ContractSubclass::from_kind_str("rpc"), Some(ContractSubclass::Rpc));
        assert_eq!(ContractSubclass::from_kind_str("message"), Some(ContractSubclass::Message));
        assert_eq!(ContractSubclass::from_kind_str("schema"), Some(ContractSubclass::Schema));
        assert_eq!(ContractSubclass::from_kind_str("task"), Some(ContractSubclass::CallContract));
        // framework-specific registration kinds are NOT first-class contracts
        assert_eq!(ContractSubclass::from_kind_str("include_router"), None);
        assert_eq!(ContractSubclass::from_kind_str("add_middleware"), None);

        // Contract carries the subclass; `new` defaults to Http, serde
        // roundtrip preserves it, and a missing field (legacy JSON) defaults.
        let mut c = Contract::new(
            "repo://repo/contract/http/get--api-x",
            "http",
            "repo://repo/symbol/main.py/handler",
        );
        assert_eq!(c.subclass, ContractSubclass::Http);
        c.subclass = ContractSubclass::Serialization;
        let json = serde_json::to_string(&c).unwrap();
        let back: Contract = serde_json::from_str(&json).unwrap();
        assert_eq!(back.subclass, ContractSubclass::Serialization);
        let legacy = serde_json::json!({
            "id": "repo://repo/contract/x",
            "kind": "http",
            "producer": "p",
            "operations": ["GET /x"],
            "evidence": [],
            "consumers": [],
        });
        let c2: Contract = serde_json::from_value(legacy).unwrap();
        assert_eq!(c2.subclass, ContractSubclass::Http);
    }

    #[test]
// trace:exempt reason=unit-test
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

    #[test]
// trace:exempt reason=internal-detail
    fn surface_render_result_roundtrips_through_serde() {
        let r = SurfaceRenderResult {
            text: "SCC SYSTEM SURFACE MAP\n\n  function serve\n".into(),
            rendered_ids: vec!["repo://r/symbol/api.py/serve".into()],
            omitted_ids: vec!["repo://r/symbol/api.py/internal".into()],
            omissions: vec![SurfaceOmission {
                count: 1,
                kind: "function".into(),
                reason: "token budget".into(),
            }],
            token_count: 7,
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: SurfaceRenderResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
        assert_eq!(back.rendered_ids[0], "repo://r/symbol/api.py/serve");
        assert_eq!(back.omissions[0].count, 1);
    }

    #[test]
// trace:exempt reason=internal-detail
    fn rank_node_carries_kind_and_name() {
        let n = RankNode {
            id: "repo://r/contract/c1".into(),
            kind: kinds::CONTRACT.into(),
            name: "c1".into(),
        };
        let json = serde_json::to_string(&n).unwrap();
        let back: RankNode = serde_json::from_str(&json).unwrap();
        assert_eq!(back.kind, "contract");
        assert_eq!(back.name, "c1");
    }

    #[test]
// trace:exempt reason=internal-detail
    fn context_artifact_content_hash_roundtrips_and_defaults() {
        // new field roundtrips
        let a = ContextArtifact {
            kind: "startup".into(),
            epoch: "e1".into(),
            renderer_version: "0.1.0".into(),
            trust_policy: "floor=0.85".into(),
            budget: ContextBudget::default(),
            sha256: "abc".into(),
            content_hash: "def".into(),
            text: "body".into(),
        };
        let json = serde_json::to_string(&a).unwrap();
        let back: ContextArtifact = serde_json::from_str(&json).unwrap();
        assert_eq!(back.content_hash, "def");

        // legacy JSON without content_hash deserializes (default empty)
        let legacy = serde_json::json!({
            "kind": "startup",
            "epoch": "e1",
            "renderer_version": "0.1.0",
            "trust_policy": "floor=0.85",
            "budget": ContextBudget::default(),
            "sha256": "abc",
            "text": "body",
        });
        let c: ContextArtifact = serde_json::from_value(legacy).unwrap();
        assert_eq!(c.content_hash, "");
    }
}
