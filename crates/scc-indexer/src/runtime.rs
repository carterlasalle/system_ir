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
//! [`reconcile`] compares observed edges against static RESOLVED `calls`
//! relationships (mapped symbol-to-component) and reports matched, observed-
//! only, and static-only edges.

use scc_core::{kinds, now_rfc3339, predicates, Entity, Provenance};
use scc_store::rusqlite::params;
use scc_store::Store;
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

/// Static vs observed call-edge comparison.
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

/// Ingest an OTLP/JSON trace payload into `runtime_edges`.
///
/// Spans without a resolvable parent in the same trace contribute edges from
/// `"root"`. An empty body yields zero stats; malformed JSON returns `Err`.
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
/// trace ingestion are left untouched.
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

/// Compare observed runtime edges against static RESOLVED `calls`
/// relationships between symbol entities, at component granularity.
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

    Ok(Reconciliation {
        observed_edges: observed.iter().cloned().collect(),
        static_edges: static_set.iter().cloned().collect(),
        matched: observed.intersection(&static_set).cloned().collect(),
        observed_not_static: observed.difference(&static_set).cloned().collect(),
        static_not_observed: static_set.difference(&observed).cloned().collect(),
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
    }
}
