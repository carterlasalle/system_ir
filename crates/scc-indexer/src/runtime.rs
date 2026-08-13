//! Runtime observation ingestion and static-vs-observed reconciliation.
//!
//! Two ingestion paths feed the shared `runtime_edges` aggregation table:
//!
//! - [`ingest_otlp_json`] accepts OTLP/JSON trace payloads
//!   (`{"resourceSpans": [...]}`). Spans are grouped into per-trace trees and
//!   each span produces an edge `(parent_service, span_service)` — or
//!   `("root", span_service)` when the span has no parent in the payload.
//!   Counts are additive, latency is a count-weighted running average, errors
//!   are additive.
//! - [`ingest_simple_edges`] accepts the legacy `[{"source","target","count"}]`
//!   shape and only aggregates counts.
//!
//! [`reconcile`] compares three sources at component granularity: observed
//! edges, static evidence-grade `calls` relationships (mapped symbol-to-
//! component), and declared intent (flow claims with a `steps` list).
//! Differences become drift findings (`undeclared_observed` HIGH,
//! `declared_unobserved` MEDIUM, `static_unobserved` LOW) written to the
//! store, deduplicated across runs.
//!
//! [`ingest_otlp_json`] additionally records per-trace path signatures
//! (root-to-leaf service paths, deduped per trace) into the
//! `trace_signatures` table.

use scc_core::{kinds, now_rfc3339, predicates, Entity, Provenance};
use scc_store::rusqlite::params;
use scc_store::{ModelEpochKind, Store};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// One aggregated runtime call edge.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RuntimeEdge {
    pub source: String,
    pub target: String,
    pub count: u64,
    pub latency_ms: f64,
    pub errors: u64,
    pub last_observed: String,
}

/// Summary of an ingestion pass.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct TraceStats {
    pub spans: usize,
    pub edges: usize,
    pub errors: usize,
}

/// Static vs observed call-edge comparison, two-way (static vs observed)
/// plus three-way (declared intent vs static vs observed).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct Reconciliation {
    /// `"src -> tgt"` edges seen at runtime.
    pub observed_edges: Vec<String>,
    /// `"src -> tgt"` edges from RESOLVED static `calls` relationships.
    pub static_edges: Vec<String>,
    /// Edges present in both sets.
    pub matched: Vec<String>,
    /// Observed but never declared in static analysis.
    pub observed_not_static: Vec<String>,
    /// Declared statically but never observed at runtime.
    pub static_not_observed: Vec<String>,
    // ---- three-way diff (Wave 6): declared vs static vs observed ----
    /// Declared edges from intent claims (source "flow" carrying a "steps"
    /// list), joined component-wise. Empty when no declared flows exist.
    pub declared: Vec<String>,
    /// Observed but absent from the declared architecture: `observed \ declared`
    /// (drift kind `undeclared_observed`, HIGH). Includes the synthetic
    /// "root" head for traces.
    pub observed_only: Vec<String>,
    /// Declared but never observed: `declared \ observed` (drift kind
    /// `declared_unobserved`, MEDIUM). Only populated when runtime data
    /// exists.
    pub declared_only: Vec<String>,
    /// Statically reachable but neither declared nor observed:
    /// `static \ (declared ∪ observed)` (drift kind `static_unobserved`,
    /// LOW). Only populated when runtime data exists.
    pub static_only: Vec<String>,
}

// ---------------------------------------------------------------------------
// OTLP/JSON ingestion
// ---------------------------------------------------------------------------

/// Minimal OTLP/JSON trace payload (proto3 JSON mapping).
#[derive(Deserialize, Default)]
struct OtlpPayload {
    #[serde(default, rename = "resourceSpans")]
    resource_spans: Vec<ResourceSpans>,
}

#[derive(Deserialize, Default)]
struct ResourceSpans {
    #[serde(default)]
    resource: Option<Resource>,
    #[serde(default, rename = "scopeSpans")]
    scope_spans: Vec<ScopeSpans>,
}

#[derive(Deserialize, Default)]
struct Resource {
    #[serde(default)]
    attributes: Vec<Attr>,
}

#[derive(Deserialize, Default)]
struct Attr {
    #[serde(default)]
    key: String,
    #[serde(default)]
    value: Option<serde_json::Value>,
}

#[derive(Deserialize, Default)]
struct ScopeSpans {
    #[serde(default)]
    spans: Vec<OtlpSpan>,
}

#[derive(Deserialize, Default, Clone)]
struct OtlpSpan {
    #[serde(default, rename = "traceId")]
    trace_id: Option<String>,
    #[serde(default, rename = "spanId")]
    span_id: Option<String>,
    #[serde(default, rename = "parentSpanId")]
    parent_span_id: Option<String>,
    #[serde(default, rename = "startTimeUnixNano")]
    start_time_unix_nano: Option<serde_json::Value>,
    #[serde(default, rename = "endTimeUnixNano")]
    end_time_unix_nano: Option<serde_json::Value>,
    #[serde(default)]
    status: Option<OtlpStatus>,
}

#[derive(Deserialize, Default, Clone)]
struct OtlpStatus {
    #[serde(default)]
    code: Option<serde_json::Value>,
}

/// A span with the service name of its resource block attached.
struct SpanInfo {
    trace_id: String,
    span_id: String,
    parent_span_id: Option<String>,
    service: String,
    latency_ms: f64,
    error: bool,
}

/// Read a string attribute value; tolerates both `{"stringValue": "..."}`
/// and a bare string value.
fn attr_string(attrs: &[Attr], key: &str) -> Option<String> {
    let value = attrs.iter().find(|a| a.key == key)?.value.as_ref()?;
    if let Some(s) = value.as_str() {
        return Some(s.to_string());
    }
    value
        .get("stringValue")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
}

/// Tolerates numeric and string-encoded integer fields (proto3 JSON encodes
/// 64-bit ints as strings).
fn json_u64(v: &serde_json::Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

/// Cap on root-to-leaf paths collected per trace before the trace is
/// skipped as non-canonical. Prevents a hostile wide/fan-out tree from
/// exploding into an exponential number of signatures.
const MAX_TRACE_PATHS: usize = 256;

/// Depth-first walk from a root span, merging every root-to-leaf path into
/// `out` keyed by its service labels (including the synthetic "root" head):
/// `(latency sum, error sum, path count)` per distinct path. Iterative (no
/// recursion depth hazard) and deterministic (children visited in span-id
/// order; identical paths are merged, not duplicated). When the path budget
/// is exhausted `exploded` is set and the walk stops — the caller should
/// skip that trace entirely.
fn walk_trace_paths<'a>(
    root: &'a SpanInfo,
    trace_spans: &'a HashMap<String, SpanInfo>,
    out: &mut BTreeMap<Vec<String>, (f64, u64, u64)>,
    exploded: &mut bool,
) {
    // stack entries: (span, path-so-far, latency-so-far, errors-so-far)
    let mut stack: Vec<(&SpanInfo, Vec<String>, f64, u64)> =
        vec![(root, vec!["root".to_string()], 0.0, 0)];
    while let Some((span, prefix, latency, errors)) = stack.pop() {
        if *exploded {
            return;
        }
        let mut path = prefix;
        path.push(span.service.clone());
        let latency = latency + span.latency_ms;
        let errors = errors + u64::from(span.error);
        let mut children: Vec<&SpanInfo> = trace_spans
            .values()
            .filter(|s| s.parent_span_id.as_deref() == Some(span.span_id.as_str()))
            .collect();
        children.sort_by_key(|s| s.span_id.clone());
        if children.is_empty() {
            if out.len() >= MAX_TRACE_PATHS {
                *exploded = true;
                return;
            }
            let entry = out.entry(path).or_insert((0.0, 0, 0));
            entry.0 += latency;
            entry.1 += errors;
            entry.2 += 1;
        } else {
            for c in children {
                stack.push((c, path.clone(), latency, errors));
            }
        }
    }
}

/// Ingest an OTLP/JSON trace payload into `runtime_edges`.
///
/// Spans without a resolvable parent in the same trace contribute edges from
/// `"root"`. An empty body yields zero stats; malformed JSON returns `Err`.
// trace:v1 id=impl.scc.runtime work=WORK-SCC-001 satisfies=REQ-SCC-DATA
pub fn ingest_otlp_json(store: &Store, body: &str) -> Result<TraceStats, String> {
    if body.trim().is_empty() {
        return Ok(TraceStats {
            spans: 0,
            edges: 0,
            errors: 0,
        });
    }
    let payload: OtlpPayload =
        serde_json::from_str(body).map_err(|e| format!("invalid OTLP JSON: {e}"))?;

    // Flatten spans, attaching each span's service name from its resource.
    let mut spans: Vec<SpanInfo> = Vec::new();
    for rs in payload.resource_spans {
        let resource_attrs: &[Attr] = rs
            .resource
            .as_ref()
            .map(|r| r.attributes.as_slice())
            .unwrap_or(&[]);
        let service = attr_string(resource_attrs, "service.name")
            .unwrap_or_else(|| "unknown".to_string());
        for ss in rs.scope_spans {
            for sp in ss.spans {
                let latency_ms = sp
                    .start_time_unix_nano
                    .as_ref()
                    .and_then(json_u64)
                    .zip(sp.end_time_unix_nano.as_ref().and_then(json_u64))
                    .map(|(start, end)| end.saturating_sub(start) as f64 / 1e6)
                    .unwrap_or(0.0);
                let error = sp
                    .status
                    .as_ref()
                    .and_then(|s| s.code.as_ref())
                    .and_then(json_u64)
                    .map(|code| code == 2)
                    .unwrap_or(false);
                spans.push(SpanInfo {
                    trace_id: sp.trace_id.unwrap_or_default(),
                    span_id: sp.span_id.unwrap_or_default(),
                    parent_span_id: sp.parent_span_id.filter(|p| !p.is_empty()),
                    service: service.clone(),
                    latency_ms,
                    error,
                });
            }
        }
    }

    // Group spans by trace, then connect children to their parents.
    let mut by_trace: BTreeMap<String, HashMap<String, SpanInfo>> = BTreeMap::new();
    for span in spans {
        let trace_id = span.trace_id.clone();
        by_trace
            .entry(trace_id)
            .or_default()
            .insert(span.span_id.clone(), span);
    }

    // Aggregate per (source_service, target_service).
    let mut agg: BTreeMap<(String, String), (u64, f64, u64)> = BTreeMap::new();
    for trace_spans in by_trace.values() {
        for span in trace_spans.values() {
            let source = span
                .parent_span_id
                .as_ref()
                .and_then(|pid| trace_spans.get(pid))
                .map(|parent| parent.service.clone())
                .unwrap_or_else(|| "root".to_string());
            let entry = agg
                .entry((source, span.service.clone()))
                .or_insert((0, 0.0, 0));
            entry.0 += 1;
            entry.1 += span.latency_ms;
            if span.error {
                entry.2 += 1;
            }
        }
    }

    // Upsert: additive counts/errors, count-weighted latency average. The
    // aggregated latency is a SUM over this batch; store the per-observation
    // average (docs/TEST_PLAN.md §15).
    let mut edges = 0usize;
    for ((source, target), (count, latency, errors)) in &agg {
        let avg_latency = if *count > 0 { latency / *count as f64 } else { 0.0 };
        store
            .conn
            .execute(
                "INSERT INTO runtime_edges (source, target, count, latency_ms, errors, last_observed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(source, target) DO UPDATE SET
                   count = runtime_edges.count + excluded.count,
                   latency_ms = (runtime_edges.latency_ms * runtime_edges.count + excluded.latency_ms * excluded.count)
                                / (runtime_edges.count + excluded.count),
                   errors = runtime_edges.errors + excluded.errors,
                   last_observed = excluded.last_observed",
                params![source, target, *count as i64, avg_latency, *errors as i64, now_rfc3339()],
            )
            .map_err(|e| format!("upsert runtime edge: {e}"))?;
        edges += 1;
    }

    // Trace path signatures (Wave 6): walk each trace from its root span
    // and emit one signature per root-to-leaf path. Labels are the same
    // service labels edges are resolved with, prefixed by the synthetic
    // "root" head so a signature reads "root -> api -> db". Identical
    // paths are merged per trace (dedupe per trace); only traces with
    // >= 2 spans produce signatures. Each distinct signature upserts once
    // per trace occurrence with the average path latency and the summed
    // path errors, so `count` stays honest.
    let mut sig_count = 0usize;
    for trace_spans in by_trace.values() {
        if trace_spans.len() < 2 {
            continue; // a single-span trace has no meaningful path
        }
        let roots: Vec<&SpanInfo> = trace_spans
            .values()
            .filter(|s| {
                s.parent_span_id
                    .as_ref()
                    .map(|p| !trace_spans.contains_key(p))
                    .unwrap_or(true)
            })
            .collect();
        if roots.is_empty() {
            continue;
        }
        let mut paths: BTreeMap<Vec<String>, (f64, u64, u64)> = BTreeMap::new();
        let mut exploded = false;
        for root in &roots {
            walk_trace_paths(root, trace_spans, &mut paths, &mut exploded);
            if exploded {
                break;
            }
        }
        if exploded {
            // pathological fan-out: no canonical signature for this trace
            continue;
        }
        for (labels, (latency, errors, path_count)) in paths {
            let avg_latency = if path_count > 0 { latency / path_count as f64 } else { 0.0 };
            store
                .upsert_trace_signature(&labels.join(" -> "), avg_latency, errors)
                .map_err(|e| format!("upsert trace signature: {e}"))?;
            sig_count += 1;
        }
    }

    // runtime truth changed — invalidate epoch-keyed context packs
    if edges > 0 || sig_count > 0 {
        store
            .bump_epoch(ModelEpochKind::Runtime)
            .map_err(|e| format!("bump runtime epoch: {e}"))?;
    }

    let total_spans = agg.values().map(|(count, _, _)| *count).sum::<u64>() as usize;
    let error_spans = agg.values().map(|(_, _, errors)| *errors).sum::<u64>() as usize;
    Ok(TraceStats {
        spans: total_spans,
        edges,
        errors: error_spans,
    })
}

// ---------------------------------------------------------------------------
// Legacy simple-edge ingestion
// ---------------------------------------------------------------------------

/// Ingest `[{"source","target","count"}]` edges (count defaults to 1).
///
/// Only counts and `last_observed` are updated; latency/error aggregates from
/// trace ingestion are left untouched. No trace-path signatures are recorded
/// here: this shape carries no trace structure (no span tree to walk), so
/// only aggregated edges are produced.
pub fn ingest_simple_edges(store: &Store, body: &str) -> Result<TraceStats, String> {
    if body.trim().is_empty() {
        return Ok(TraceStats {
            spans: 0,
            edges: 0,
            errors: 0,
        });
    }
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("invalid edges JSON: {e}"))?;
    let items: Vec<&serde_json::Value> = match &value {
        serde_json::Value::Array(items) => items.iter().collect(),
        serde_json::Value::Object(_) => vec![&value],
        _ => {
            return Err(
                "invalid edges JSON: expected an array of {source, target, count} objects".into(),
            )
        }
    };

    let mut edges = 0usize;
    for item in items {
        let source = item
            .get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "invalid edges JSON: edge missing string 'source'".to_string())?;
        let target = item
            .get("target")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "invalid edges JSON: edge missing string 'target'".to_string())?;
        let count = item.get("count").and_then(json_u64).unwrap_or(1);
        store
            .conn
            .execute(
                "INSERT INTO runtime_edges (source, target, count, last_observed)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(source, target) DO UPDATE SET
                   count = runtime_edges.count + excluded.count,
                   last_observed = excluded.last_observed",
                params![source, target, count as i64, now_rfc3339()],
            )
            .map_err(|e| format!("upsert runtime edge: {e}"))?;
        edges += 1;
    }
    // runtime truth changed — invalidate epoch-keyed context packs
    if edges > 0 {
        store
            .bump_epoch(ModelEpochKind::Runtime)
            .map_err(|e| format!("bump runtime epoch: {e}"))?;
    }
    Ok(TraceStats {
        spans: 0,
        edges,
        errors: 0,
    })
}

// ---------------------------------------------------------------------------
// Reads and reconciliation
// ---------------------------------------------------------------------------

/// All aggregated runtime edges, ordered by (source, target).
pub fn runtime_edges(store: &Store) -> Result<Vec<RuntimeEdge>, String> {
    let mut stmt = store
        .conn
        .prepare(
            "SELECT source, target, count, latency_ms, errors, last_observed
             FROM runtime_edges ORDER BY source, target",
        )
        .map_err(|e| format!("prepare runtime edges: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, f64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
            ))
        })
        .map_err(|e| format!("query runtime edges: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        let (source, target, count, latency_ms, errors, last_observed) =
            row.map_err(|e| format!("read runtime edge: {e}"))?;
        out.push(RuntimeEdge {
            source,
            target,
            count: count as u64,
            latency_ms,
            errors: errors as u64,
            last_observed,
        });
    }
    Ok(out)
}

/// Map a symbol entity id to its component name.
///
/// The symbol's `file` attribute is matched against each component's
/// `implementation.paths` (exact or directory-prefix match). Unmappable
/// symbols fall back to the symbol name itself; unknown ids fall back to the
/// id's final segment.
fn component_name(store: &Store, id: &str, components: &[Entity]) -> String {
    let sym = match store.get_entity(id) {
        Ok(Some(sym)) => sym,
        _ => {
            return id
                .rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or(id)
                .to_string()
        }
    };
    let file = match sym.attributes.get("file").and_then(|v| v.as_str()) {
        Some(file) => file,
        None => return sym.name,
    };
    for c in components {
        let paths = c
            .attributes
            .get("implementation")
            .and_then(|i| i.get("paths"))
            .and_then(|p| p.as_array());
        if let Some(paths) = paths {
            for p in paths {
                if let Some(p) = p.as_str() {
                    let matches = match file.strip_prefix(p) {
                        Some("") => true,
                        Some(rest) => rest.starts_with('/'),
                        None => false,
                    };
                    if matches {
                        return c.name.clone();
                    }
                }
            }
        }
    }
    sym.name
}

/// Declared edges from intent claims: source `"flow"` claims carrying a
/// `steps` list are joined component-wise — each consecutive pair becomes a
/// `"A -> B"` edge. Step entries are component names (strings) or objects
/// with a `component` field. Honest by construction: a claim without a
/// `steps` list declares no edges, and if no declared flows exist the
/// declared set is empty (findings then reduce to observed-only vs
/// static-only comparisons).
fn declared_edges(store: &Store) -> Result<BTreeSet<String>, String> {
    let claims = store
        .intent_claims()
        .map_err(|e| format!("load intent claims: {e}"))?;
    let mut out: BTreeSet<String> = BTreeSet::new();
    for (source, claim) in claims {
        if source != "flow" {
            continue;
        }
        let Some(steps) = claim["steps"].as_array() else {
            continue;
        };
        let labels: Vec<String> = steps
            .iter()
            .filter_map(|s| {
                s.as_str()
                    .map(String::from)
                    .or_else(|| {
                        s.get("component")
                            .and_then(|c| c.as_str())
                            .map(String::from)
                    })
            })
            .collect();
        for pair in labels.windows(2) {
            out.insert(format!("{} -> {}", pair[0], pair[1]));
        }
    }
    Ok(out)
}

/// Write drift findings for the three-way diff, skipping any (kind,
/// message) already present among unresolved findings so repeated
/// reconciles stay idempotent. Returns whether anything new was written.
fn persist_reconcile_findings(
    store: &Store,
    findings: &[(String, String, String)],
) -> Result<bool, String> {
    if findings.is_empty() {
        return Ok(false);
    }
    let existing: BTreeSet<(String, String)> = store
        .drift_findings(true)
        .map_err(|e| format!("load drift findings: {e}"))?
        .into_iter()
        .map(|(_, kind, _, msg, _)| (kind, msg))
        .collect();
    let mut changed = false;
    for (kind, severity, msg) in findings {
        if existing.contains(&(kind.clone(), msg.clone())) {
            continue;
        }
        store
            .add_drift_finding(kind, severity, msg)
            .map_err(|e| format!("record drift finding: {e}"))?;
        changed = true;
    }
    Ok(changed)
}

/// Compare declared intent vs static evidence-grade `calls` vs observed
/// runtime edges, all at component granularity.
///
/// Three-way drift findings (drift_findings kinds):
/// - `undeclared_observed` (HIGH): observed \ declared — executed at runtime
///   but absent from the declared architecture
/// - `declared_unobserved` (MEDIUM): declared \ observed — declared but never
///   observed (only when runtime data exists)
/// - `static_unobserved` (LOW): static \ (declared ∪ observed) — statically
///   reachable but neither declared nor observed (only when runtime data
///   exists)
///
/// Findings are persisted (deduplicated across runs) and the Derived epoch
/// is bumped when new findings land, so epoch-keyed packs pick them up.
pub fn reconcile(store: &Store) -> Result<Reconciliation, String> {
    let observed: BTreeSet<String> = runtime_edges(store)?
        .into_iter()
        .map(|e| format!("{} -> {}", e.source, e.target))
        .collect();

    let rels = store
        .all_relationships()
        .map_err(|e| format!("load relationships: {e}"))?;
    let components = store
        .entities_by_kind(kinds::COMPONENT)
        .map_err(|e| format!("load components: {e}"))?;

    let mut static_set: BTreeSet<String> = BTreeSet::new();
    for rel in rels {
        // evidence-grade static edges: EXTRACTED (native candidates) and
        // RESOLVED (LSP/SCIP proof)
        if rel.predicate != predicates::CALLS
            || !matches!(rel.provenance, Provenance::Extracted | Provenance::Resolved)
        {
            continue;
        }
        if !rel.subject.contains("/symbol/") || !rel.object.contains("/symbol/") {
            continue;
        }
        let source = component_name(store, &rel.subject, &components);
        let target = component_name(store, &rel.object, &components);
        static_set.insert(format!("{source} -> {target}"));
    }

    let declared = declared_edges(store)?;
    let has_runtime = !observed.is_empty();

    // three-way diff -> drift findings (edge strings as messages)
    let mut findings: Vec<(String, String, String)> = Vec::new();
    for e in observed.difference(&declared) {
        findings.push((
            "undeclared_observed".into(),
            "high".into(),
            e.clone(),
        ));
    }
    if has_runtime {
        for e in declared.difference(&observed) {
            findings.push((
                "declared_unobserved".into(),
                "medium".into(),
                e.clone(),
            ));
        }
        let known: BTreeSet<String> = declared.union(&observed).cloned().collect();
        for e in static_set.difference(&known) {
            findings.push((
                "static_unobserved".into(),
                "low".into(),
                e.clone(),
            ));
        }
    }
    if persist_reconcile_findings(store, &findings)? {
        store
            .bump_epoch(ModelEpochKind::Derived)
            .map_err(|e| format!("bump derived epoch: {e}"))?;
    }

    let observed_only: Vec<String> = observed.difference(&declared).cloned().collect();
    let declared_only: Vec<String> = if has_runtime {
        declared.difference(&observed).cloned().collect()
    } else {
        Vec::new()
    };
    let known: BTreeSet<String> = declared.union(&observed).cloned().collect();
    let static_only: Vec<String> = if has_runtime {
        static_set.difference(&known).cloned().collect()
    } else {
        Vec::new()
    };

    Ok(Reconciliation {
        observed_edges: observed.iter().cloned().collect(),
        static_edges: static_set.iter().cloned().collect(),
        matched: observed.intersection(&static_set).cloned().collect(),
        observed_not_static: observed.difference(&static_set).cloned().collect(),
        static_not_observed: static_set.difference(&observed).cloned().collect(),
        declared: declared.into_iter().collect(),
        observed_only,
        declared_only,
        static_only,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use scc_core::{symbol_id, Relationship};
    use tempfile::TempDir;

    fn tmp_store() -> (Store, TempDir) {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        (store, dir)
    }

    fn find_edge<'a>(edges: &'a [RuntimeEdge], source: &str, target: &str) -> &'a RuntimeEdge {
        edges
            .iter()
            .find(|e| e.source == source && e.target == target)
            .unwrap_or_else(|| panic!("edge {source} -> {target} not found"))
    }

    /// Trace: root span in `api`, one child in `api`, one error child in `db`.
    fn otlp_trace() -> String {
        serde_json::json!({
            "resourceSpans": [
                {
                    "resource": { "attributes": [{ "key": "service.name", "value": { "stringValue": "api" } }] },
                    "scopeSpans": [{ "spans": [
                        {
                            "traceId": "t1", "spanId": "a", "name": "GET /x",
                            "startTimeUnixNano": "0", "endTimeUnixNano": "10000000",
                            "status": { "code": 0 }
                        },
                        {
                            "traceId": "t1", "spanId": "b", "parentSpanId": "a", "name": "local work",
                            "startTimeUnixNano": "0", "endTimeUnixNano": "2000000",
                            "status": { "code": 0 }
                        }
                    ]}]
                },
                {
                    "resource": { "attributes": [{ "key": "service.name", "value": { "stringValue": "db" } }] },
                    "scopeSpans": [{ "spans": [
                        {
                            "traceId": "t1", "spanId": "c", "parentSpanId": "a", "name": "SELECT 1",
                            "startTimeUnixNano": "1000000", "endTimeUnixNano": "6000000",
                            "status": { "code": 2 }
                        }
                    ]}]
                }
            ]
        })
        .to_string()
    }

    #[test]
    fn otlp_ingest_builds_edges_and_stats() {
        let (store, _dir) = tmp_store();
        let stats = ingest_otlp_json(&store, &otlp_trace()).unwrap();
        assert_eq!(
            stats,
            TraceStats {
                spans: 3,
                edges: 3,
                errors: 1,
            }
        );

        let edges = runtime_edges(&store).unwrap();
        assert_eq!(edges.len(), 3);
        // root span -> api
        let root_api = find_edge(&edges, "root", "api");
        assert_eq!(root_api.count, 1);
        assert!((root_api.latency_ms - 10.0).abs() < 1e-9);
        assert_eq!(root_api.errors, 0);
        // api child -> api
        let api_api = find_edge(&edges, "api", "api");
        assert_eq!(api_api.count, 1);
        assert!((api_api.latency_ms - 2.0).abs() < 1e-9);
        // api -> db with the error span
        let api_db = find_edge(&edges, "api", "db");
        assert_eq!(api_db.count, 1);
        assert!((api_db.latency_ms - 5.0).abs() < 1e-9);
        assert_eq!(api_db.errors, 1);
        assert!(!api_db.last_observed.is_empty());
    }

    #[test]
    fn otlp_ingest_upsert_is_additive_and_averaging() {
        let (store, _dir) = tmp_store();
        ingest_otlp_json(&store, &otlp_trace()).unwrap();
        ingest_otlp_json(&store, &otlp_trace()).unwrap();

        let edges = runtime_edges(&store).unwrap();
        let api_db = find_edge(&edges, "api", "db");
        assert_eq!(api_db.count, 2);
        assert!((api_db.latency_ms - 5.0).abs() < 1e-9, "average unchanged");
        assert_eq!(api_db.errors, 2);

        // Third ingest with a longer duration for span c: running average
        // shifts toward the new value (5ms * 2 + 15ms) / 3 = 8.333ms.
        let mut trace: serde_json::Value = serde_json::from_str(&otlp_trace()).unwrap();
        trace["resourceSpans"][1]["scopeSpans"][0]["spans"][0]["endTimeUnixNano"] =
            serde_json::json!("16000000");
        ingest_otlp_json(&store, &trace.to_string()).unwrap();

        let edges = runtime_edges(&store).unwrap();
        let api_db = find_edge(&edges, "api", "db");
        assert_eq!(api_db.count, 3);
        assert!((api_db.latency_ms - 25.0 / 3.0).abs() < 1e-9);
        assert_eq!(api_db.errors, 3);
    }

    #[test]
    fn otlp_ingest_defensive_inputs() {
        let (store, _dir) = tmp_store();
        // Empty body -> zero stats, no error.
        let stats = ingest_otlp_json(&store, "").unwrap();
        assert_eq!(stats, TraceStats { spans: 0, edges: 0, errors: 0 });
        assert!(runtime_edges(&store).unwrap().is_empty());

        // Well-formed JSON without spans -> zero stats.
        let stats = ingest_otlp_json(&store, "{}").unwrap();
        assert_eq!(stats, TraceStats { spans: 0, edges: 0, errors: 0 });

        // Malformed JSON -> Err.
        assert!(ingest_otlp_json(&store, "not json").is_err());
        assert!(ingest_otlp_json(&store, "{\"resourceSpans\": [}").is_err());
    }

    #[test]
    fn simple_edges_ingest_shape() {
        let (store, _dir) = tmp_store();
        let body =
            r#"[{"source": "api", "target": "db", "count": 3}, {"source": "web", "target": "api"}]"#;
        let stats = ingest_simple_edges(&store, body).unwrap();
        assert_eq!(stats, TraceStats { spans: 0, edges: 2, errors: 0 });

        let edges = runtime_edges(&store).unwrap();
        assert_eq!(find_edge(&edges, "api", "db").count, 3);
        assert_eq!(find_edge(&edges, "web", "api").count, 1);

        // Additive on re-ingest; latency/errors untouched.
        ingest_simple_edges(&store, body).unwrap();
        let edges = runtime_edges(&store).unwrap();
        assert_eq!(find_edge(&edges, "api", "db").count, 6);
        assert_eq!(find_edge(&edges, "api", "db").errors, 0);
        assert_eq!(find_edge(&edges, "api", "db").latency_ms, 0.0);

        // A bare object is accepted as a single edge.
        let stats = ingest_simple_edges(&store, r#"{"source": "x", "target": "y"}"#).unwrap();
        assert_eq!(stats.edges, 1);
        assert_eq!(find_edge(&runtime_edges(&store).unwrap(), "x", "y").count, 1);

        // Empty body and malformed JSON.
        assert_eq!(ingest_simple_edges(&store, "").unwrap().edges, 0);
        assert!(ingest_simple_edges(&store, "42").is_err());
        assert!(ingest_simple_edges(&store, r#"[{"source": "a"}]"#).is_err());
    }

    fn component(id: &str, name: &str, paths: &[&str]) -> Entity {
        let mut c = Entity::new(id, kinds::COMPONENT, name);
        c.attr(
            "implementation",
            serde_json::json!({ "paths": paths, "symbols": [] }),
        );
        c
    }

    fn symbol_entity(file: &str, name: &str) -> Entity {
        let mut s = Entity::new(symbol_id("repo", file, name), kinds::SYMBOL, name);
        s.attr("file", file);
        s
    }

    #[test]
    fn reconcile_static_vs_observed() {
        let (store, _dir) = tmp_store();

        // Components: api owns src/api, db owns src/db.
        store
            .insert_entity(&component("repo://repo/component/api", "api", &["src/api"]), &[])
            .unwrap();
        store
            .insert_entity(&component("repo://repo/component/db", "db", &["src/db"]), &[])
            .unwrap();

        // Symbols with file attributes.
        let fetch_user = symbol_id("repo", "src/api/handler.rs", "fetch_user");
        let query = symbol_id("repo", "src/db/pool.rs", "query");
        let helper = symbol_id("repo", "src/tools/util.rs", "helper"); // no component
        for e in [
            symbol_entity("src/api/handler.rs", "fetch_user"),
            symbol_entity("src/db/pool.rs", "query"),
            symbol_entity("src/tools/util.rs", "helper"),
        ] {
            store.insert_entity(&e, &[]).unwrap();
        }

        // Static RESOLVED calls: fetch_user -> query (api -> db), fetch_user -> helper (api -> helper).
        let rel1 = Relationship::new(
            "rel:1",
            &fetch_user,
            predicates::CALLS,
            &query,
            Provenance::Resolved,
        );
        store.insert_relationship(&rel1, "src/api/handler.rs").unwrap();
        let rel2 = Relationship::new(
            "rel:2",
            &fetch_user,
            predicates::CALLS,
            &helper,
            Provenance::Resolved,
        );
        store.insert_relationship(&rel2, "src/api/handler.rs").unwrap();

        // Not calls, and not RESOLVED: both must be ignored.
        let rel3 = Relationship::new(
            "rel:3",
            &fetch_user,
            predicates::IMPORTS,
            &query,
            Provenance::Resolved,
        );
        store.insert_relationship(&rel3, "src/api/handler.rs").unwrap();
        let rel4 = Relationship::new(
            "rel:4",
            &fetch_user,
            predicates::CALLS,
            &query,
            Provenance::Extracted,
        );
        store.insert_relationship(&rel4, "src/api/handler.rs").unwrap();

        // Observed edges: api -> db (matches static), web -> api (runtime only).
        ingest_simple_edges(
            &store,
            r#"[{"source": "api", "target": "db"}, {"source": "web", "target": "api"}]"#,
        )
        .unwrap();

        let r = reconcile(&store).unwrap();
        assert_eq!(
            r.observed_edges,
            vec!["api -> db".to_string(), "web -> api".to_string()]
        );
        assert_eq!(
            r.static_edges,
            vec!["api -> db".to_string(), "api -> helper".to_string()]
        );
        assert_eq!(r.matched, vec!["api -> db".to_string()]);
        assert_eq!(r.observed_not_static, vec!["web -> api".to_string()]);
        assert_eq!(r.static_not_observed, vec!["api -> helper".to_string()]);
    }

    #[test]
    fn reconcile_empty_store_is_empty() {
        let (store, _dir) = tmp_store();
        let r = reconcile(&store).unwrap();
        assert!(r.observed_edges.is_empty());
        assert!(r.static_edges.is_empty());
        assert!(r.matched.is_empty());
        assert!(r.observed_not_static.is_empty());
        assert!(r.static_not_observed.is_empty());
        assert!(r.declared.is_empty());
        assert!(r.observed_only.is_empty());
        assert!(r.declared_only.is_empty());
        assert!(r.static_only.is_empty());
    }

    /// Trace: one `api` root span with two `db` children — the two
    /// root-to-leaf paths dedupe to a single "root -> api -> db" signature
    /// from a 3-span tree. Span `c` is an error.
    fn otlp_signature_trace() -> String {
        serde_json::json!({
            "resourceSpans": [
                {
                    "resource": { "attributes": [{ "key": "service.name", "value": { "stringValue": "api" } }] },
                    "scopeSpans": [{ "spans": [
                        {
                            "traceId": "t2", "spanId": "a", "name": "GET /x",
                            "startTimeUnixNano": "0", "endTimeUnixNano": "10000000",
                            "status": { "code": 0 }
                        }
                    ]}]
                },
                {
                    "resource": { "attributes": [{ "key": "service.name", "value": { "stringValue": "db" } }] },
                    "scopeSpans": [{ "spans": [
                        {
                            "traceId": "t2", "spanId": "b", "parentSpanId": "a", "name": "SELECT 1",
                            "startTimeUnixNano": "1000000", "endTimeUnixNano": "6000000",
                            "status": { "code": 0 }
                        },
                        {
                            "traceId": "t2", "spanId": "c", "parentSpanId": "a", "name": "SELECT 2",
                            "startTimeUnixNano": "2000000", "endTimeUnixNano": "7000000",
                            "status": { "code": 2 }
                        }
                    ]}]
                }
            ]
        })
        .to_string()
    }

    #[test]
    fn otlp_ingest_records_trace_signatures() {
        let (store, _dir) = tmp_store();
        assert!(store.trace_signatures().unwrap().is_empty());

        let stats = ingest_otlp_json(&store, &otlp_signature_trace()).unwrap();
        assert_eq!(
            stats,
            TraceStats {
                spans: 3,
                // (root, api) + (api, db) — the two db spans merge into one edge
                edges: 2,
                errors: 1,
            }
        );

        let sigs = store.trace_signatures().unwrap();
        assert_eq!(sigs.len(), 1, "identical root-to-leaf paths dedupe per trace");
        let (sig, count, latency, errors, last) = &sigs[0];
        assert_eq!(sig, "root -> api -> db");
        assert_eq!(*count, 1);
        // avg over the deduped paths: (10ms + 5ms) twice -> 15.0
        assert!((latency - 15.0).abs() < 1e-9, "latency {latency}");
        assert_eq!(*errors, 1);
        assert!(!last.is_empty());

        // a second ingest increments the occurrence count and errors
        ingest_otlp_json(&store, &otlp_signature_trace()).unwrap();
        let sigs = store.trace_signatures().unwrap();
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].1, 2);
        assert_eq!(sigs[0].3, 2);
        assert!((sigs[0].2 - 15.0).abs() < 1e-9);
    }

    #[test]
    fn otlp_ingest_signatures_for_branched_and_single_span_traces() {
        let (store, _dir) = tmp_store();
        // the shared fixture branches root(api) -> api and root(api) -> db
        ingest_otlp_json(&store, &otlp_trace()).unwrap();
        let sigs = store.trace_signatures().unwrap();
        assert_eq!(sigs.len(), 2);
        let labels: Vec<&str> = sigs.iter().map(|s| s.0.as_str()).collect();
        assert!(labels.contains(&"root -> api -> api"));
        assert!(labels.contains(&"root -> api -> db"));

        // single-span traces produce no signature
        let single = serde_json::json!({
            "resourceSpans": [{
                "resource": { "attributes": [{ "key": "service.name", "value": { "stringValue": "api" } }] },
                "scopeSpans": [{ "spans": [
                    { "traceId": "t9", "spanId": "a", "name": "solo" }
                ]}]
            }]
        })
        .to_string();
        ingest_otlp_json(&store, &single).unwrap();
        let sigs = store.trace_signatures().unwrap();
        assert_eq!(sigs.len(), 2, "single-span trace adds no signature");
    }

    #[test]
    fn reconcile_three_way_findings() {
        let (store, _dir) = tmp_store();

        // Components: api owns src/api, db owns src/db; helper unmappable.
        store
            .insert_entity(&component("repo://repo/component/api", "api", &["src/api"]), &[])
            .unwrap();
        store
            .insert_entity(&component("repo://repo/component/db", "db", &["src/db"]), &[])
            .unwrap();
        let fetch_user = symbol_id("repo", "src/api/handler.rs", "fetch_user");
        let query = symbol_id("repo", "src/db/pool.rs", "query");
        let helper = symbol_id("repo", "src/tools/util.rs", "helper");
        for e in [
            symbol_entity("src/api/handler.rs", "fetch_user"),
            symbol_entity("src/db/pool.rs", "query"),
            symbol_entity("src/tools/util.rs", "helper"),
        ] {
            store.insert_entity(&e, &[]).unwrap();
        }
        // Static RESOLVED calls: api -> db (observed too), api -> helper (never).
        for (rid, subject, object) in [
            ("rel:1", &fetch_user, &query),
            ("rel:2", &fetch_user, &helper),
        ] {
            let rel =
                Relationship::new(rid, subject, predicates::CALLS, object, Provenance::Resolved);
            store.insert_relationship(&rel, "src/api/handler.rs").unwrap();
        }
        // Declared flow steps: api -> db -> cache (db -> cache never observed).
        store
            .replace_intent_claims(&[(
                "flow".into(),
                serde_json::json!({
                    "name": "fetch",
                    "entrypoint": "fetch_user",
                    "steps": ["api", "db", "cache"],
                }),
            )])
            .unwrap();
        // Observed: api -> db (declared + static), web -> api (undeclared).
        ingest_simple_edges(
            &store,
            r#"[{"source": "api", "target": "db"}, {"source": "web", "target": "api"}]"#,
        )
        .unwrap();

        let r = reconcile(&store).unwrap();
        // three-way sets
        assert_eq!(r.declared, vec!["api -> db", "db -> cache"]);
        assert_eq!(r.observed_only, vec!["web -> api"]);
        assert_eq!(r.declared_only, vec!["db -> cache"]);
        assert_eq!(r.static_only, vec!["api -> helper"]);
        // two-way sets unchanged
        assert_eq!(r.matched, vec!["api -> db"]);
        assert_eq!(r.observed_not_static, vec!["web -> api"]);
        assert_eq!(r.static_not_observed, vec!["api -> helper"]);

        // drift findings written with the right kinds, severities, messages
        let findings = store.drift_findings(true).unwrap();
        let mut by_kind: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut by_kind_sev: BTreeMap<String, String> = BTreeMap::new();
        for (_, kind, sev, msg, _) in &findings {
            by_kind.entry(kind.clone()).or_default().push(msg.clone());
            by_kind_sev.entry(kind.clone()).or_insert_with(|| sev.clone());
        }
        assert_eq!(by_kind.get("undeclared_observed").unwrap(), &vec!["web -> api".to_string()]);
        assert_eq!(by_kind.get("declared_unobserved").unwrap(), &vec!["db -> cache".to_string()]);
        assert_eq!(by_kind.get("static_unobserved").unwrap(), &vec!["api -> helper".to_string()]);
        assert_eq!(by_kind_sev.get("undeclared_observed").unwrap(), "high");
        assert_eq!(by_kind_sev.get("declared_unobserved").unwrap(), "medium");
        assert_eq!(by_kind_sev.get("static_unobserved").unwrap(), "low");

        // idempotent: reconciling again adds no duplicate findings
        reconcile(&store).unwrap();
        assert_eq!(store.drift_findings(true).unwrap().len(), findings.len());
    }

    #[test]
    fn reconcile_three_way_without_runtime_data() {
        let (store, _dir) = tmp_store();
        store
            .insert_entity(&component("repo://repo/component/api", "api", &["src/api"]), &[])
            .unwrap();
        store
            .insert_entity(&component("repo://repo/component/db", "db", &["src/db"]), &[])
            .unwrap();
        let fetch_user = symbol_id("repo", "src/api/handler.rs", "fetch_user");
        let query = symbol_id("repo", "src/db/pool.rs", "query");
        for e in [
            symbol_entity("src/api/handler.rs", "fetch_user"),
            symbol_entity("src/db/pool.rs", "query"),
        ] {
            store.insert_entity(&e, &[]).unwrap();
        }
        let rel = Relationship::new(
            "rel:1",
            &fetch_user,
            predicates::CALLS,
            &query,
            Provenance::Resolved,
        );
        store.insert_relationship(&rel, "src/api/handler.rs").unwrap();
        store
            .replace_intent_claims(&[(
                "flow".into(),
                serde_json::json!({ "name": "f", "steps": ["api", "db"] }),
            )])
            .unwrap();

        // No runtime data: declared_unobserved and static_unobserved are
        // suppressed (they only make sense once observations exist), and no
        // finding fires because observed is empty.
        let r = reconcile(&store).unwrap();
        assert!(r.declared_only.is_empty(), "declared_unobserved requires runtime data");
        assert!(r.static_only.is_empty(), "static_unobserved requires runtime data");
        assert!(r.observed_only.is_empty());
        assert!(store.drift_findings(true).unwrap().is_empty());
    }
}
