//! Structural Source compiler (Wave 14, Level 2 of the context stack,
//! docs/SYSTEM_DESIGN.md Wave 14): a semantic skeleton of an implementation
//! slice. Denser than signature compression: per symbol it preserves the
//! signature plus the calls, state access, events, contracts, and control
//! skeleton the extractor actually evidenced — and never fabricates
//! sequence beyond CFG/flow truth.
//!
//! Every unit carries provenance back to the exact source (`source:
//! <path>:L<start>-L<end>`, `representation`, `revision`) so compressed
//! output is never mistaken for source text.
//!
//! ## CFG/control evidence the store actually holds
//!
//! The indexer records, per symbol entity (`write.rs`): `call_order`
//! (callee -> min lexical order), `call_blocks` (callee -> nearest
//! control-block kind: if/else/for/while/try/catch/match/switch/with/do/
//! loop/finally/select), `awaited_calls`, `call_returns` (callees whose
//! result is consumed — the return evidence), and `conditional_calls`
//! (older fallback: callees inside any branch, kind unknown). Control
//! blocks carry no nesting depth and no condition expressions — the
//! skeleton therefore emits one control line per callee, labeled by the
//! callee, with that callee's lines nested beneath it.

use crate::ContextCompiler;
use scc_core::{kinds, predicates, Relationship, StructuralSourceUnit};
use std::collections::BTreeMap;

/// Representation label for deep structural units.
const STRUCTURAL: &str = "STRUCTURAL";
/// Representation label for fallback (signature-only) units.
const SIGNATURES: &str = "SIGNATURES";

/// CFG attributes the indexer writes on symbol entities.
const ATTR_CALL_ORDER: &str = "call_order";
const ATTR_CALL_BLOCKS: &str = "call_blocks";
const ATTR_CONDITIONAL_CALLS: &str = "conditional_calls";
const ATTR_CALL_RETURNS: &str = "call_returns";

/// Edge predicates the skeleton renders, in deterministic priority order
/// for same-line ties.
const EDGE_ORDER: &[(&str, &str)] = &[
    (predicates::CALLS, "CALL"),
    (predicates::READS, "READ"),
    (predicates::QUERIES, "QUERY"),
    (predicates::WRITES, "WRITE"),
    (predicates::TRANSFORMS, "TRANSFORM"),
    (predicates::PUBLISHES, "EVENT"),
    (predicates::SUBSCRIBES, "QUEUE"),
    (predicates::REGISTERS, "CONTRACT"),
    (predicates::CONSUMES, "CONTRACT"),
    (predicates::PRODUCES, "CONTRACT"),
    (predicates::PARTICIPATES_IN, "CONTRACT"),
];

/// Contract verbs only render when the edge target is a CONTRACT entity.
const CONTRACT_VERBS: &[&str] = &[
    predicates::REGISTERS,
    predicates::CONSUMES,
    predicates::PRODUCES,
    predicates::PARTICIPATES_IN,
];

/// Callee names containing any of these tokens are logging noise and are
/// stripped from the skeleton.
const LOGGING_TOKENS: &[&str] = &["log", "debug", "info", "warn", "print"];

/// Map a control-block kind to its skeleton verb.
// trace:exempt reason=internal-detail
fn control_verb(kind: &str) -> String {
    match kind {
        "if" | "else" => "IF".to_string(),
        "for" | "while" | "do" | "loop" => "LOOP".to_string(),
        "try" | "catch" | "finally" => "TRY".to_string(),
        "match" | "switch" => "MATCH".to_string(),
        other => other.to_uppercase(),
    }
}

/// Smallest evidence line across an edge's evidence ids, or `default` when
/// the edge carries no line-bearing evidence.
// trace:exempt reason=internal-detail
fn evidence_line(compiler: &ContextCompiler, edge: &Relationship, default: u32) -> u32 {
    let evmap = compiler.evidence_map();
    let mut min: Option<u32> = None;
    for id in &edge.evidence {
        if let Some(ev) = evmap.get(id) {
            if let Some(l) = ev.start_line {
                min = Some(min.map_or(l, |m| m.min(l)));
            }
        }
    }
    min.unwrap_or(default)
}

/// First sentence of a docstring, kept as a `# <doc>` line.
// trace:exempt reason=internal-detail
fn first_sentence(doc: &str) -> String {
    let doc = doc.trim();
    if doc.is_empty() {
        return String::new();
    }
    let mut end = doc.len();
    for (i, c) in doc.char_indices() {
        if c == '\n' {
            end = i;
            break;
        }
        if c == '.' {
            let rest = &doc[i + 1..];
            if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\n') {
                end = i + 1;
                break;
            }
        }
    }
    doc[..end].trim().to_string()
}

/// Whether a callee name is logging noise.
// trace:exempt reason=internal-detail
fn is_logging(name: &str) -> bool {
    let lower = name.to_lowercase();
    LOGGING_TOKENS.iter().any(|t| lower.contains(t))
}

/// `true` when the symbol carries any CFG evidence attribute.
// trace:exempt reason=internal-detail
fn has_cfg_evidence(sym: &scc_core::Entity) -> bool {
    [ATTR_CALL_ORDER, ATTR_CALL_BLOCKS, ATTR_CONDITIONAL_CALLS]
        .iter()
        .any(|a| sym.attributes.contains_key(*a))
}

// trace:exempt reason=internal-detail
fn attr_u32(sym: &scc_core::Entity, key: &str) -> u32 {
    sym.attributes
        .get(key)
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32
}

// trace:exempt reason=internal-detail
fn attr_str<'e>(sym: &'e scc_core::Entity, key: &str) -> Option<&'e str> {
    sym.attributes.get(key).and_then(|v| v.as_str())
}

/// Build one symbol's body lines from the trusted view.
// trace:exempt reason=internal-detail
fn symbol_body(
    compiler: &ContextCompiler,
    sym: &scc_core::Entity,
) -> Vec<String> {
    let view = &compiler.view;
    let sym_start = attr_u32(sym, "start_line");
    let name_of = |id: &str| view.name_of(id);

    // callee -> control-block kind (CFG evidence, per-callee).
    let call_blocks: BTreeMap<String, String> = sym
        .attributes
        .get(ATTR_CALL_BLOCKS)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    // callees whose result is consumed — the return evidence.
    let call_returns: std::collections::BTreeSet<String> = sym
        .attributes
        .get(ATTR_CALL_RETURNS)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    // Collect (line, rank, name, rendered) per trusted edge.
    let mut raw: Vec<(u32, usize, String, String)> = Vec::new();
    for (rank, (pred, verb)) in EDGE_ORDER.iter().enumerate() {
        for edge in view.out_pred(&sym.id, pred) {
            let target = view.entity(&edge.object);
            let name = name_of(&edge.object);
            if CONTRACT_VERBS.contains(pred) {
                let is_contract = target.map(|t| t.kind == kinds::CONTRACT).unwrap_or(false);
                if !is_contract {
                    continue;
                }
            }
            let line = evidence_line(compiler, edge, sym_start);
            if *pred == predicates::CALLS {
                if is_logging(&name) {
                    continue;
                }
                let is_ctor = target.map(|t| t.kind == "class").unwrap_or(false);
                let v = if is_ctor { "CONSTRUCT" } else { "CALL" };
                raw.push((line, rank, name.clone(), format!("{v} {name}")));
            } else {
                raw.push((line, rank, name.clone(), format!("{verb} {name}")));
            }
        }
    }
    raw.sort();
    // Dedupe consecutive identical rendered lines (same callee at the same
    // or adjacent evidence sites).
    let mut dedup: Vec<(u32, usize, String, String)> = Vec::new();
    for r in raw {
        if dedup.last().map(|d| d.3 == r.3).unwrap_or(false) {
            continue;
        }
        dedup.push(r);
    }

    // Emit with control nesting: a callee with a recorded block kind gets a
    // control line, and its own lines nest one level deeper.
    let mut out: Vec<String> = Vec::new();
    for (_, _, name, rendered) in &dedup {
        let is_call = rendered.starts_with("CALL ") || rendered.starts_with("CONSTRUCT ");
        if is_call {
            if let Some(kind) = call_blocks.get(name) {
                let ctl = format!("  {} {name}", control_verb(kind));
                if out.last().map(|l| l == &ctl).unwrap_or(false) {
                    // same control line already open; keep nesting
                } else {
                    out.push(ctl);
                }
                out.push(format!("    {rendered}"));
            } else {
                out.push(format!("  {rendered}"));
            }
            if call_returns.contains(name) {
                let ret = format!("    RETURN {name}");
                if out.last().map(|l| l == &ret).unwrap_or(false) {
                    // already emitted
                } else {
                    out.push(ret);
                }
            }
        } else {
            out.push(format!("  {rendered}"));
        }
    }
    // Final consecutive dedupe (e.g. RETURN emitted twice at one site).
    let mut lines: Vec<String> = Vec::new();
    for l in out {
        if lines.last().map(|p| p == &l).unwrap_or(false) {
            continue;
        }
        lines.push(l);
    }
    lines
}

/// The file's symbols sorted by (start_line, name), via CONTAINS edges of
/// the file entity (kind SYMBOL only).
// trace:exempt reason=internal-detail
fn file_symbols<'c>(compiler: &'c ContextCompiler<'_>, file_id: &str) -> Vec<&'c scc_core::Entity> {
    let view = &compiler.view;
    let mut syms: Vec<&scc_core::Entity> = view
        .out_pred(file_id, predicates::CONTAINS)
        .iter()
        .filter_map(|r| view.entity(&r.object))
        .filter(|e| e.kind == kinds::SYMBOL)
        .collect();
    syms.sort_by(|a, b| {
        let la = attr_u32(a, "start_line");
        let lb = attr_u32(b, "start_line");
        (la, &a.name).cmp(&(lb, &b.name))
    });
    syms
}

/// Build one file's structural unit (deep or fallback). `None` when the
/// file is unknown to the trusted view or contains no symbols.
// trace:exempt reason=internal-detail
fn build_unit(compiler: &ContextCompiler, path: &str) -> Option<StructuralSourceUnit> {
    let repo = &compiler.view.graph.repo_id;
    let file_id = scc_core::entity_id(repo, kinds::FILE, path);
    let syms = file_symbols(compiler, &file_id);
    if syms.is_empty() {
        return None;
    }

    let deep = syms.iter().any(|s| has_cfg_evidence(s));
    let (min_line, max_line) = syms.iter().fold((u32::MAX, 0u32), |(mn, mx), s| {
        let start = attr_u32(s, "start_line");
        let end = attr_u32(s, "end_line").max(start);
        (mn.min(start), mx.max(end))
    });
    let (min_line, max_line) = if min_line == u32::MAX { (0, 0) } else { (min_line, max_line) };

    let mut content = String::new();
    if deep {
        for sym in &syms {
            if !content.is_empty() {
                content.push('\n');
            }
            let sig = attr_str(sym, "signature")
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    let k = attr_str(sym, "kind").unwrap_or(&sym.kind);
                    format!("{k} {}", sym.name)
                });
            content.push_str(&sig);
            content.push('\n');
            if let Some(doc) = attr_str(sym, "docstring") {
                let sentence = first_sentence(doc);
                if !sentence.is_empty() {
                    content.push_str("# ");
                    content.push_str(&sentence);
                    content.push('\n');
                }
            }
            let body = symbol_body(compiler, sym);
            if !body.is_empty() {
                content.push('\n');
                for l in &body {
                    content.push_str(l);
                    content.push('\n');
                }
            }
        }
    } else {
        // Fallback: imports + type declarations/signatures.
        let view = &compiler.view;
        let mut imports: Vec<(u32, String)> = view
            .out_pred(&file_id, predicates::IMPORTS)
            .iter()
            .map(|r| {
                let line = evidence_line(compiler, r, 0);
                let name = view.name_of(&r.object);
                (line, format!("IMPORT {name}"))
            })
            .collect();
        imports.sort();
        let mut seen_imports: Vec<String> = Vec::new();
        for (_, l) in imports {
            if seen_imports.last().map(|p| p == &l).unwrap_or(false) {
                continue;
            }
            seen_imports.push(l);
        }
        for l in seen_imports {
            content.push_str(&l);
            content.push('\n');
        }
        for sym in &syms {
            if !content.is_empty() {
                content.push('\n');
            }
            let sig = attr_str(sym, "signature")
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    let k = attr_str(sym, "kind").unwrap_or(&sym.kind);
                    format!("{k} {}", sym.name)
                });
            content.push_str(&sig);
            content.push('\n');
            if let Some(doc) = attr_str(sym, "docstring") {
                let sentence = first_sentence(doc);
                if !sentence.is_empty() {
                    content.push_str("# ");
                    content.push_str(&sentence);
                    content.push('\n');
                }
            }
        }
    }
    let content = content.trim_end().to_string();

    Some(StructuralSourceUnit {
        path: path.to_string(),
        source: format!("source: {path}:L{min_line}-L{max_line}"),
        representation: if deep { STRUCTURAL.to_string() } else { SIGNATURES.to_string() },
        revision: compiler.revision(),
        content,
    })
}

/// Build structural-source units for the requested paths (deduped, order
/// preserved), capped at `max_units`. Files unknown to the trusted view or
/// containing no symbols produce no unit.
// trace:v1 id=impl.scc.structural_source work=WORK-SCC-014 satisfies=REQ-SCC-IR
pub fn structural_source(
    compiler: &ContextCompiler,
    paths: &[String],
    max_units: usize,
) -> Vec<StructuralSourceUnit> {
    let mut units: Vec<StructuralSourceUnit> = Vec::new();
    if max_units == 0 {
        return units;
    }
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for p in paths {
        if units.len() >= max_units {
            break;
        }
        if seen.contains(p) {
            continue;
        }
        seen.insert(p.clone());
        if let Some(u) = build_unit(compiler, p) {
            units.push(u);
        }
    }
    units
}

/// Render units as the spec's format:
///
/// ```text
/// <path>
///
/// source: <path>:L<min>-L<max>
/// representation: STRUCTURAL
/// revision: <rev>
///
/// <signature>
/// # <doc>
///
///   CALL worker
///   WRITE db
/// ```
// trace:exempt reason=internal-detail
pub fn render_structural(units: &[StructuralSourceUnit]) -> String {
    let mut out = String::new();
    for (i, u) in units.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&u.path);
        out.push_str("\n\n");
        out.push_str(&u.source);
        out.push('\n');
        out.push_str("representation: ");
        out.push_str(&u.representation);
        out.push('\n');
        out.push_str("revision: ");
        out.push_str(&u.revision);
        out.push_str("\n\n");
        out.push_str(&u.content);
        out.push('\n');
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use scc_core::{
        entity_id, kinds, predicates, relationship_id, symbol_id, Entity, Evidence,
        EvidenceType, Provenance, Relationship,
    };
    use scc_graph::RealityGraph;
    use scc_store::Store;
    use std::collections::HashMap;

// trace:exempt reason=internal-detail
    fn evidence(id: &str, path: &str, line: u32) -> Evidence {
        let mut e = Evidence::source(id, path);
        e.start_line = Some(line);
        e.r#type = EvidenceType::Source;
        e
    }

    /// Fixture repo:
    /// - app.py: `handler` (L10-L14) calls `worker` (L12, inside an `if`
    ///   block, result returned) and `debug_log` (L13, logging — must be
    ///   stripped), writes store `db` (L14). CFG attrs present -> deep.
    /// - lib.py: `reader` (L3-L8) with no call evidence, imports `util`
    ///   -> fallback (signature-only).
// trace:exempt reason=internal-detail
    fn fixture() -> (tempfile::TempDir, Store, RealityGraph) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let repo = "repo";
        store.upsert_file("app.py", "h1", "python", "source", 20).unwrap();
        store.upsert_file("lib.py", "h2", "python", "source", 20).unwrap();

        let file_id = entity_id(repo, kinds::FILE, "app.py");
        let lib_id = entity_id(repo, kinds::FILE, "lib.py");
        let mk = |n: &str| symbol_id(repo, "app.py", n);
        let db_id = entity_id(repo, kinds::DATA_STORE, "db");

        let mut entities: HashMap<String, Entity> = HashMap::new();

        // handler symbol with CFG evidence
        let mut handler = Entity::new(mk("handler"), kinds::SYMBOL, "handler");
        handler.attr("file", serde_json::json!("app.py"));
        handler.attr("signature", serde_json::json!("def handler(x: str) -> str:"));
        handler.attr("start_line", serde_json::json!(10));
        handler.attr("end_line", serde_json::json!(14));
        handler.attr(
            "docstring",
            serde_json::json!("Normalizes a raw transcript. Deprecated: use normalize_v2."),
        );
        handler.attr("call_order", serde_json::json!({"worker": 0, "debug_log": 1}));
        handler.attr("call_blocks", serde_json::json!({"worker": "if"}));
        handler.attr("call_returns", serde_json::json!(["worker"]));
        entities.insert(handler.id.clone(), handler);

        // helper symbols
        for n in ["worker", "debug_log"] {
            let mut s = Entity::new(mk(n), kinds::SYMBOL, n);
            s.attr("file", serde_json::json!("app.py"));
            s.attr("signature", serde_json::json!(format!("def {n}(y):")));
            s.attr("start_line", serde_json::json!(20));
            s.attr("end_line", serde_json::json!(21));
            entities.insert(s.id.clone(), s);
        }

        // store entity
        let mut de = Entity::new(db_id.clone(), kinds::DATA_STORE, "db");
        de.attr("technology", serde_json::json!("postgres"));
        entities.insert(de.id.clone(), de);

        // lib.py reader (no CFG evidence)
        let reader_id = symbol_id(repo, "lib.py", "reader");
        let mut reader = Entity::new(reader_id.clone(), kinds::SYMBOL, "reader");
        reader.attr("file", serde_json::json!("lib.py"));
        reader.attr("signature", serde_json::json!("def reader(path: str) -> list:"));
        reader.attr("start_line", serde_json::json!(3));
        reader.attr("end_line", serde_json::json!(8));
        reader.attr("docstring", serde_json::json!("Reads lines from a file."));
        entities.insert(reader.id.clone(), reader);

        // util external module for lib.py imports
        let util_id = entity_id(repo, kinds::EXTERNAL_API, "util");
        entities.insert(
            util_id.clone(),
            Entity::new(util_id.clone(), kinds::EXTERNAL_API, "util"),
        );

        let mut out: HashMap<String, Vec<Relationship>> = HashMap::new();
        let mut inn: HashMap<String, Vec<Relationship>> = HashMap::new();
        let mut n = 0u64;
        let mut rel = |s: String,
                       pred: &str,
                       o: String,
                       ev: Vec<String>,
                       inn: &mut HashMap<String, Vec<Relationship>>| {
            n += 1;
            let r = Relationship::new(
                relationship_id(n),
                s.clone(),
                pred,
                o,
                Provenance::Extracted,
            )
            .with_evidence(ev);
            out.entry(s.clone()).or_default().push(r.clone());
            inn.entry(r.object.clone()).or_default().push(r);
        };

        // evidence rows (in store so the compiler's evidence map has lines)
        store.insert_evidence(&evidence("evidence:10", "app.py", 12)).unwrap();
        store.insert_evidence(&evidence("evidence:11", "app.py", 13)).unwrap();
        store.insert_evidence(&evidence("evidence:12", "app.py", 14)).unwrap();
        store.insert_evidence(&evidence("evidence:13", "app.py", 12)).unwrap();
        store.insert_evidence(&evidence("evidence:20", "lib.py", 1)).unwrap();
        let sev = Evidence::source("evidence:21", "app.py");
        store.insert_evidence(&sev).unwrap();

        // app.py: file contains handler
        rel(
            file_id.clone(),
            predicates::CONTAINS,
            mk("handler"),
            vec!["evidence:21".to_string()],
            &mut inn,
        );
        // handler CALLS worker (two sites at L12 -> dedupe) + CALLS debug_log (L13)
        rel(
            mk("handler"),
            predicates::CALLS,
            mk("worker"),
            vec!["evidence:10".to_string()],
            &mut inn,
        );
        rel(
            mk("handler"),
            predicates::CALLS,
            mk("worker"),
            vec!["evidence:13".to_string()],
            &mut inn,
        );
        rel(
            mk("handler"),
            predicates::CALLS,
            mk("debug_log"),
            vec!["evidence:11".to_string()],
            &mut inn,
        );
        // handler WRITES db (L14)
        rel(
            mk("handler"),
            predicates::WRITES,
            db_id.clone(),
            vec!["evidence:12".to_string()],
            &mut inn,
        );
        // lib.py: file contains reader + file imports util
        rel(
            lib_id.clone(),
            predicates::CONTAINS,
            reader_id,
            vec!["evidence:20".to_string()],
            &mut inn,
        );
        rel(
            lib_id.clone(),
            predicates::IMPORTS,
            util_id,
            vec!["evidence:20".to_string()],
            &mut inn,
        );

        let graph = RealityGraph {
            repo_id: repo.to_string(),
            entities,
            out,
            inn,
            components: vec![],
            flows: vec![],
            invariants: vec![],
        };
        (dir, store, graph)
    }

// trace:exempt reason=internal-detail
    fn compiler<'a>(
        store: &'a Store,
        graph: &'a RealityGraph,
    ) -> ContextCompiler<'a> {
        ContextCompiler::new(
            store,
            graph,
            crate::ContextSettings::default(),
            Vec::new(),
        )
    }

    #[test]
// trace:exempt reason=internal-detail
    fn structural_unit_has_call_write_return_in_evidence_order() {
        let (_d, store, graph) = fixture();
        let c = compiler(&store, &graph);
        let units = structural_source(&c, &["app.py".to_string()], 10);

        assert_eq!(units.len(), 1, "one unit for app.py");
        let u = &units[0];
        assert_eq!(u.path, "app.py");
        // provenance header
        assert_eq!(u.source, "source: app.py:L10-L14");
        assert_eq!(u.representation, "STRUCTURAL");
        assert_eq!(u.revision, c.revision());

        let content = &u.content;
        // signature + doc (first sentence only)
        assert!(content.contains("def handler(x: str) -> str:"), "{content}");
        assert!(content.contains("# Normalizes a raw transcript"), "{content}");
        assert!(!content.contains("Deprecated"), "{content}");
        // control skeleton from call_blocks
        assert!(content.contains("  IF worker"), "{content}");
        assert!(content.contains("    CALL worker"), "{content}");
        assert!(content.contains("    RETURN worker"), "{content}");
        assert!(content.contains("  WRITE db"), "{content}");
        // logging calls stripped
        assert!(!content.contains("debug_log"), "{content}");
        // dedupe: two CALLS worker sites collapse to one line
        assert_eq!(content.matches("CALL worker").count(), 1, "{content}");
        // evidence order: call (L12) before write (L14)
        let call_i = content.find("CALL worker").unwrap();
        let write_i = content.find("WRITE db").unwrap();
        let ctl_i = content.find("IF worker").unwrap();
        assert!(ctl_i < call_i && call_i < write_i, "{content}");
    }

    #[test]
// trace:exempt reason=internal-detail
    fn fallback_produces_signatures_for_file_without_cfg_evidence() {
        let (_d, store, graph) = fixture();
        let c = compiler(&store, &graph);
        let units = structural_source(&c, &["lib.py".to_string()], 10);

        assert_eq!(units.len(), 1);
        let u = &units[0];
        assert_eq!(u.path, "lib.py");
        assert_eq!(u.source, "source: lib.py:L3-L8");
        assert_eq!(u.representation, "SIGNATURES");
        assert!(u.content.contains("IMPORT util"), "{}", u.content);
        assert!(u.content.contains("def reader(path: str) -> list:"), "{}", u.content);
        assert!(u.content.contains("# Reads lines from a file."), "{}", u.content);
        // no body skeleton in the fallback
        assert!(!u.content.contains("CALL "), "{}", u.content);
        assert!(!u.content.contains("  WRITE"), "{}", u.content);
    }

    #[test]
// trace:exempt reason=internal-detail
    fn render_structural_emits_provenance_header_per_unit() {
        let (_d, store, graph) = fixture();
        let c = compiler(&store, &graph);
        let units = structural_source(
            &c,
            &["app.py".to_string(), "lib.py".to_string()],
            10,
        );
        assert_eq!(units.len(), 2);
        let rendered = render_structural(&units);
        assert!(rendered.contains("app.py\n\nsource: app.py:L10-L14"), "{rendered}");
        assert!(rendered.contains("representation: STRUCTURAL"), "{rendered}");
        assert!(rendered.contains("lib.py\n\nsource: lib.py:L3-L8"), "{rendered}");
        assert!(rendered.contains("representation: SIGNATURES"), "{rendered}");
        assert!(rendered.contains("revision: "), "{rendered}");
        // body lines survive rendering
        assert!(rendered.contains("    CALL worker"), "{rendered}");
        assert!(rendered.contains("  WRITE db"), "{rendered}");
    }

    #[test]
// trace:exempt reason=internal-detail
    fn unknown_and_duplicate_paths_are_handled_deterministically() {
        let (_d, store, graph) = fixture();
        let c = compiler(&store, &graph);
        // unknown path (not in store) -> skipped; duplicate path -> once;
        // max_units caps.
        let units = structural_source(
            &c,
            &[
                "nope.py".to_string(),
                "app.py".to_string(),
                "app.py".to_string(),
                "lib.py".to_string(),
            ],
            1,
        );
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].path, "app.py");
        let units = structural_source(&c, &["app.py".to_string()], 0);
        assert!(units.is_empty());
    }
}
