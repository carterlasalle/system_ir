//! Relevance-first candidate generation (BM25 + anchors + query routing).
//!
//! This is an experimental lens. Production [`crate::rank::collect_lexical_candidates_full`]
//! is unchanged. Importance (PPR) is never added into the BM25 score here.

use crate::rank::ScoredEntity;
use scc_core::{
    entity_id, kinds, predicates, relevance_hits, relevance_hits_with_stats, route_query,
    Bm25CorpusStats, LexDoc, QueryShape, RankingArm, WEIGHT_BODY, WEIGHT_DOC, WEIGHT_NAME,
    WEIGHT_PATH,
};
use scc_graph::TrustedGraphView;
use std::collections::HashSet;
use std::path::Path;

const RELEVANCE_KINDS: &[&str] = &[
    kinds::SYMBOL,
    kinds::COMPONENT,
    kinds::ROUTE,
    kinds::CONTRACT,
    kinds::STATE,
    kinds::FILE,
    kinds::FLOW,
    kinds::SCHEMA,
];

// trace:exempt reason=internal-detail
fn attr_str(e: &scc_core::Entity, key: &str) -> String {
    e.attributes
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

// trace:exempt reason=internal-detail
fn entity_to_doc(e: &scc_core::Entity) -> LexDoc {
    let path = attr_str(e, "file");
    let path = if path.is_empty() {
        attr_str(e, "path")
    } else {
        path
    };
    let doc = attr_str(e, "docstring");
    let mut body = attr_str(e, "signature");
    if body.is_empty() {
        body = attr_str(e, "responsibility");
    }
    LexDoc {
        id: e.id.clone(),
        fields: vec![
            scc_core::LexField {
                text: e.name.clone(),
                weight: WEIGHT_NAME,
            },
            scc_core::LexField {
                text: path,
                weight: WEIGHT_PATH,
            },
            scc_core::LexField {
                text: doc,
                weight: WEIGHT_DOC,
            },
            scc_core::LexField {
                text: body,
                weight: WEIGHT_BODY,
            },
        ],
    }
}

/// Collect relevance candidates. `arm` selects which scores are emitted;
/// BM25 and exact-anchor stay separate fields in [`scc_core::RelevanceHit`].
/// Graph expansion, when requested, is tagged `graph-expand` and is not
/// added into the BM25 number. Co-change and doc-mention extras are
/// inspectable reasons with score 0.0 — never fused into BM25.
// trace:v1 id=impl.scc.context.relevance-collect work=WORK-ripwire-lessons-phase2 satisfies=REQ-ranking-arms-inspectable,REQ-exact-anchors,REQ-query-mentions,REQ-cochange-retrieval-evidence,REQ-declared-mentions,REQ-bm25-persist,REQ-implement-fix-pr-review-comments-without-collapsing-scc-type-script-no
pub fn collect_relevance_candidates(
    view: &TrustedGraphView,
    query: &str,
    limit: usize,
    arm: RankingArm,
    repo_root: Option<&Path>,
    corpus: Option<&Bm25CorpusStats>,
) -> Vec<ScoredEntity> {
    if matches!(
        arm,
        RankingArm::ProductionBlended
            | RankingArm::NoLexical
            | RankingArm::NoTaskPpr
            | RankingArm::NoGlobalPpr
    ) {
        // PPR ablations run through `build_surface_staged`, not this BM25 lens.
        return Vec::new();
    }
    let plan = route_query(query);
    let shape = if arm == RankingArm::QueryRouted {
        plan.shape
    } else {
        QueryShape::Conceptual
    };

    let entities: Vec<&scc_core::Entity> = view
        .entities()
        .filter(|e| RELEVANCE_KINDS.contains(&e.kind.as_str()))
        .collect();
    let docs: Vec<LexDoc> = entities.iter().map(|e| entity_to_doc(e)).collect();
    let mut hits = match corpus {
        Some(stats) => relevance_hits_with_stats(query, &docs, stats),
        None => relevance_hits(query, &docs),
    };

    if arm == RankingArm::QueryRouted {
        match shape {
            QueryShape::Identifier | QueryShape::SymbolLike | QueryShape::PathLike => {
                // name-first: drop pure body matches that are not anchors
                // when any anchor exists
                if hits.iter().any(|h| h.exact_anchor) {
                    hits.retain(|h| h.exact_anchor || h.bm25 > 0.0);
                }
            }
            QueryShape::StackTrace | QueryShape::ErrorMessage => {
                if plan.prefer_locus && !plan.loci.is_empty() {
                    let paths: Vec<&str> = plan.loci.iter().map(|l| l.path.as_str()).collect();
                    for hit in hits.iter_mut() {
                        let Some(e) = view.entity(&hit.id) else {
                            continue;
                        };
                        let file = attr_str(e, "file");
                        if paths.iter().any(|p| file.ends_with(p) || file == *p) {
                            hit.exact_anchor = true;
                        }
                    }
                    hits.sort_by(|a, b| {
                        b.exact_anchor
                            .cmp(&a.exact_anchor)
                            .then_with(|| {
                                b.bm25
                                    .partial_cmp(&a.bm25)
                                    .unwrap_or(std::cmp::Ordering::Equal)
                            })
                            .then_with(|| a.id.cmp(&b.id))
                    });
                }
            }
            QueryShape::Architecture | QueryShape::Flow | QueryShape::State | QueryShape::Impact
            | QueryShape::Conceptual => {}
        }
    }

    let mut out: Vec<ScoredEntity> = Vec::new();
    for h in hits.into_iter().take(limit.saturating_mul(2)) {
        if h.bm25 <= 0.0 && !h.exact_anchor {
            continue;
        }
        let Some(e) = view.entity(&h.id) else {
            continue;
        };
        let reason = if h.exact_anchor { "anchor" } else { "bm25" };
        out.push(ScoredEntity {
            id: e.id.clone(),
            kind: e.kind.clone(),
            name: e.name.clone(),
            score: h.bm25,
            reason: reason.to_string(),
        });
        if out.len() >= limit {
            break;
        }
    }

    if arm == RankingArm::LexicalThenGraph {
        let seeds: Vec<String> = out.iter().map(|c| c.id.clone()).collect();
        for sid in seeds {
            for r in view.out_pred(&sid, predicates::CALLS) {
                if push_unique(view, &mut out, &r.object, 0.0, "graph-expand") && out.len() >= limit
                {
                    break;
                }
            }
        }
    }

    if !matches!(
        arm,
        RankingArm::ProductionBlended | RankingArm::NoLexical
    ) {
        if arm != RankingArm::NoSemantic {
            attach_doc_mention_evidence(view, &mut out, limit);
        }
        if arm != RankingArm::NoCochange {
            if let Some(root) = repo_root {
                attach_cochange_evidence(view, &mut out, root, limit);
            }
        }
    }
    out
}

// trace:exempt reason=internal-detail
fn push_unique(
    view: &TrustedGraphView,
    out: &mut Vec<ScoredEntity>,
    id: &str,
    score: f64,
    reason: &str,
) -> bool {
    if out.iter().any(|c| c.id == id) {
        return false;
    }
    let Some(e) = view.entity(id) else {
        return false;
    };
    out.push(ScoredEntity {
        id: e.id.clone(),
        kind: e.kind.clone(),
        name: e.name.clone(),
        score,
        reason: reason.to_string(),
    });
    true
}

/// Docs that DECLARED_AS a current hit, and symbols a hit doc names.
/// Score stays 0.0 — never added into BM25.
// trace:exempt reason=internal-detail
fn attach_doc_mention_evidence(view: &TrustedGraphView, out: &mut Vec<ScoredEntity>, limit: usize) {
    let seeds: Vec<String> = out.iter().map(|c| c.id.clone()).collect();
    for sid in seeds {
        if out.len() >= limit {
            return;
        }
        for r in view.in_pred(&sid, predicates::DECLARED_AS) {
            if push_unique(view, out, &r.subject, 0.0, "doc-mention") && out.len() >= limit {
                return;
            }
        }
        for r in view.out_pred(&sid, predicates::DECLARED_AS) {
            if push_unique(view, out, &r.object, 0.0, "doc-mention") && out.len() >= limit {
                return;
            }
        }
    }
}

// trace:exempt reason=internal-detail
fn entity_file_path(e: &scc_core::Entity) -> Option<String> {
    if e.kind == kinds::FILE {
        return Some(e.name.clone());
    }
    let file = attr_str(e, "file");
    if file.is_empty() {
        None
    } else {
        Some(file)
    }
}

// trace:exempt reason=internal-detail
fn files_statically_coupled(view: &TrustedGraphView, a: &str, b: &str) -> bool {
    let aid = entity_id(&view.graph.repo_id, kinds::FILE, a);
    let bid = entity_id(&view.graph.repo_id, kinds::FILE, b);
    if view
        .out_pred(&aid, predicates::IMPORTS)
        .iter()
        .any(|r| r.object == bid)
        || view
            .out_pred(&bid, predicates::IMPORTS)
            .iter()
            .any(|r| r.object == aid)
    {
        return true;
    }
    let mut a_syms: HashSet<String> = HashSet::new();
    let mut b_syms: HashSet<String> = HashSet::new();
    for e in view.entities() {
        if e.kind != kinds::SYMBOL {
            continue;
        }
        let f = attr_str(e, "file");
        if f == a {
            a_syms.insert(e.id.clone());
        } else if f == b {
            b_syms.insert(e.id.clone());
        }
    }
    for sid in &a_syms {
        for r in view.out_pred(sid, predicates::CALLS) {
            if b_syms.contains(&r.object) {
                return true;
            }
        }
    }
    for sid in &b_syms {
        for r in view.out_pred(sid, predicates::CALLS) {
            if a_syms.contains(&r.object) {
                return true;
            }
        }
    }
    false
}

/// Historical co-change partners. Never fused into BM25/PPR.
// trace:v1 id=impl.scc.context.cochange-evidence work=WORK-ripwire-lessons-phase2 satisfies=REQ-cochange-retrieval-evidence
fn attach_cochange_evidence(
    view: &TrustedGraphView,
    out: &mut Vec<ScoredEntity>,
    repo_root: &Path,
    limit: usize,
) {
    let Ok(pairs) = scc_graph::cochange::cochange_pairs(
        repo_root,
        scc_graph::cochange::COCHANGE_MIN_COMMITS,
    ) else {
        return;
    };
    if pairs.is_empty() {
        return;
    }
    let seeds: Vec<(String, String)> = out
        .iter()
        .filter_map(|c| {
            view.entity(&c.id)
                .and_then(|e| entity_file_path(e).map(|p| (c.id.clone(), p)))
        })
        .collect();
    for (_id, path) in seeds {
        if out.len() >= limit {
            return;
        }
        for pair in &pairs {
            let partner = if pair.a == path {
                Some(pair.b.as_str())
            } else if pair.b == path {
                Some(pair.a.as_str())
            } else {
                None
            };
            let Some(partner) = partner else { continue };
            let fid = entity_id(&view.graph.repo_id, kinds::FILE, partner);
            let reason = if files_statically_coupled(view, &path, partner) {
                "cochange"
            } else {
                "cochange-surprise"
            };
            let _ = push_unique(view, out, &fid, 0.0, reason);
            if out.len() >= limit {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scc_core::Entity;
    use scc_store::Store;

    // trace:exempt reason=internal-detail
    fn indexed_view(entities: Vec<Entity>) -> (tempfile::TempDir, Store, scc_graph::RealityGraph) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        for e in &entities {
            let file = e
                .attributes
                .get("file")
                .and_then(|v| v.as_str())
                .unwrap_or("a.ts");
            store.insert_entity(e, &[file.to_string()]).unwrap();
        }
        let graph = scc_graph::RealityGraph::load(&store).unwrap();
        (dir, store, graph)
    }

    // trace:exempt reason=internal-detail
    fn sym(id: &str, name: &str, file: &str, doc: &str) -> Entity {
        let mut e = Entity::new(id, kinds::SYMBOL, name);
        e.attr("file", serde_json::json!(file));
        e.attr("docstring", serde_json::json!(doc));
        e
    }

    #[test]
    // trace:v1 id=test.scc.context.relevance-arm-not-production verifies=REQ-ranking-arms-inspectable exercises=impl.scc.context.relevance-collect
    fn production_arm_emits_nothing_from_relevance_lens() {
        let (_d, store, graph) = indexed_view(vec![sym("s:a", "handleList", "src/a.ts", "")]);
        let view = scc_graph::TrustedGraphView::new(&graph, &store, &[], scc_graph::TrustPolicy::default());
        let empty = collect_relevance_candidates(
            &view,
            "handleList",
            8,
            RankingArm::ProductionBlended,
            None,
            None,
        );
        assert!(empty.is_empty(), "production arm must not silently switch to BM25");
        let hits = collect_relevance_candidates(
            &view,
            "handleList",
            8,
            RankingArm::LexicalThenGraph,
            None,
            None,
        );
        assert_eq!(hits[0].name, "handleList");
        assert_eq!(hits[0].reason, "anchor");
        for arm in [RankingArm::NoTaskPpr, RankingArm::NoGlobalPpr] {
            let empty = collect_relevance_candidates(&view, "handleList", 8, arm, None, None);
            assert!(
                empty.is_empty(),
                "{arm:?} is a surface-pipeline ablation, not this BM25 lens"
            );
        }
    }

    #[test]
    // trace:v1 id=test.scc.context.relevance-locus-by-id verifies=REQ-exact-anchors,REQ-implement-fix-pr-review-comments-without-collapsing-scc-type-script-no exercises=impl.scc.context.relevance-collect
    fn stack_locus_matches_hits_by_entity_id() {
        let (_d, store, graph) = indexed_view(vec![
            sym("s:noise", "alpha", "src/other.py", ""),
            sym("s:hit", "beta", "src/app.py", ""),
        ]);
        let view =
            scc_graph::TrustedGraphView::new(&graph, &store, &[], scc_graph::TrustPolicy::default());
        let query = "Traceback (most recent call last):\n  File \"src/app.py\", line 11, in inner\nValueError: boom\n";
        let hits = collect_relevance_candidates(
            &view,
            query,
            8,
            RankingArm::QueryRouted,
            None,
            None,
        );
        let hit = hits
            .iter()
            .find(|h| h.id == "s:hit")
            .expect("locus file must remain a hit");
        assert_eq!(hit.reason, "anchor");
        assert!(
            hits.iter()
                .filter(|h| h.id == "s:noise")
                .all(|h| h.reason != "anchor"),
            "unrelated file must not inherit locus from a positional zip: {hits:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.context.query-routed-name-first verifies=REQ-query-shape-router exercises=impl.scc.context.relevance-collect
    fn query_routed_identifier_prefers_anchor() {
        let (_d, store, graph) = indexed_view(vec![
            sym("s:noise", "loginHelper", "src/login.ts", "handles the list of users"),
            sym("s:hit", "handleList", "src/server.ts", ""),
        ]);
        let view = scc_graph::TrustedGraphView::new(&graph, &store, &[], scc_graph::TrustPolicy::default());
        let hits = collect_relevance_candidates(&view, "handleList", 8, RankingArm::QueryRouted, None, None);
        assert_eq!(hits[0].id, "s:hit");
        assert_eq!(hits[0].reason, "anchor");
    }

    #[test]
    // trace:v1 id=test.scc.context.doc-mention-evidence verifies=REQ-declared-mentions exercises=impl.scc.context.relevance-collect
    fn doc_mention_is_inspectable_zero_score() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let sid = "s:hit".to_string();
        store
            .insert_entity(&sym(&sid, "handleList", "src/a.ts", ""), &["src/a.ts".into()])
            .unwrap();
        let fid = entity_id(&store.repo_id, kinds::FILE, "README.md");
        store
            .insert_entity(
                &Entity::new(fid.clone(), kinds::FILE, "README.md"),
                &["README.md".into()],
            )
            .unwrap();
        let rel = scc_core::Relationship::new(
            "rel:mention",
            fid,
            predicates::DECLARED_AS,
            sid,
            scc_core::Provenance::Declared,
        );
        store.insert_relationship(&rel, "README.md").unwrap();
        let graph = scc_graph::RealityGraph::load(&store).unwrap();
        let view =
            scc_graph::TrustedGraphView::new(&graph, &store, &[], scc_graph::TrustPolicy::default());
        let hits = collect_relevance_candidates(
            &view,
            "handleList",
            16,
            RankingArm::LexicalThenGraph,
            None,
            None,
        );
        let doc = hits
            .iter()
            .find(|h| h.reason == "doc-mention")
            .expect("doc that names the hit must surface");
        assert_eq!(doc.name, "README.md");
        assert_eq!(doc.score, 0.0);
        let bm25_hit = hits.iter().find(|h| h.name == "handleList").unwrap();
        assert!(bm25_hit.score >= 0.0);
        assert_ne!(bm25_hit.reason, "doc-mention");
        let nosem = collect_relevance_candidates(
            &view,
            "handleList",
            16,
            RankingArm::NoSemantic,
            None,
            None,
        );
        assert!(
            nosem.iter().all(|h| h.reason != "doc-mention"),
            "NoSemantic must skip doc-mention extras: {nosem:?}"
        );
        assert!(nosem.iter().any(|h| h.name == "handleList"));
    }

    // trace:exempt reason=test-helper
    fn git_init(dir: &Path) {
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "SCC Test"],
            vec!["config", "commit.gpgsign", "false"],
        ] {
            let out = std::process::Command::new("git")
                .args(&args)
                .current_dir(dir)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?} failed");
        }
    }

    #[test]
    // trace:v1 id=test.scc.context.cochange-evidence verifies=REQ-cochange-retrieval-evidence exercises=impl.scc.context.cochange-evidence
    fn cochange_partners_are_inspectable_and_skipped_by_nocochange() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("src")).unwrap();
        git_init(&root);
        std::fs::write(root.join("src/a.py"), "def handleList():\n    return 1\n").unwrap();
        std::fs::write(root.join("src/b.py"), "def other():\n    return 2\n").unwrap();
        for msg in ["c1", "c2"] {
            let out = std::process::Command::new("git")
                .args(["add", "-A"])
                .current_dir(&root)
                .output()
                .unwrap();
            assert!(out.status.success());
            let out = std::process::Command::new("git")
                .args(["commit", "-q", "-m", msg])
                .current_dir(&root)
                .output()
                .unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            std::fs::write(root.join("src/a.py"), format!("def handleList():\n    return {msg}\n"))
                .unwrap();
            std::fs::write(root.join("src/b.py"), format!("def other():\n    return {msg}\n"))
                .unwrap();
        }
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let a_id = entity_id(&store.repo_id, kinds::FILE, "src/a.py");
        let b_id = entity_id(&store.repo_id, kinds::FILE, "src/b.py");
        store
            .insert_entity(&Entity::new(a_id.clone(), kinds::FILE, "src/a.py"), &["src/a.py".into()])
            .unwrap();
        store
            .insert_entity(&Entity::new(b_id.clone(), kinds::FILE, "src/b.py"), &["src/b.py".into()])
            .unwrap();
        store
            .insert_entity(
                &sym("s:hit", "handleList", "src/a.py", ""),
                &["src/a.py".into()],
            )
            .unwrap();
        let graph = scc_graph::RealityGraph::load(&store).unwrap();
        let view =
            scc_graph::TrustedGraphView::new(&graph, &store, &[], scc_graph::TrustPolicy::default());
        let with = collect_relevance_candidates(
            &view,
            "handleList",
            16,
            RankingArm::LexicalThenGraph,
            Some(&root),
            None,
        );
        assert!(
            with.iter()
                .any(|h| h.id == b_id && h.reason.starts_with("cochange")),
            "expected cochange partner, got {with:?}"
        );
        assert!(
            with.iter()
                .filter(|h| h.reason.starts_with("cochange"))
                .all(|h| h.score == 0.0)
        );
        let without = collect_relevance_candidates(
            &view,
            "handleList",
            16,
            RankingArm::NoCochange,
            Some(&root),
            None,
        );
        assert!(
            without
                .iter()
                .all(|h| !h.reason.starts_with("cochange")),
            "NoCochange must omit co-change evidence: {without:?}"
        );
    }

    #[test]
    // trace:v1 id=test.scc.context.bm25-corpus-idf verifies=REQ-bm25-persist exercises=impl.scc.core.bm25-warm-hits,impl.scc.context.relevance-collect
    fn experimental_lens_uses_persisted_corpus_idf_not_slice() {
        let (_d, store, graph) = indexed_view(vec![
            sym("s:a", "handleList", "src/a.ts", "list handler"),
            sym("s:b", "other", "src/b.ts", "unrelated"),
            sym("s:c", "handleThing", "src/c.ts", ""),
        ]);
        let view =
            scc_graph::TrustedGraphView::new(&graph, &store, &[], scc_graph::TrustPolicy::default());
        let docs: Vec<LexDoc> = view
            .entities()
            .filter(|e| e.kind == kinds::SYMBOL)
            .map(entity_to_doc)
            .collect();
        let stats = Bm25CorpusStats::from_docs(&docs);
        let one = vec![docs
            .iter()
            .find(|d| d.id == "s:a")
            .cloned()
            .expect("s:a")];
        let slice_hits = relevance_hits("handleList", &one);
        let corpus_hits = relevance_hits_with_stats("handleList", &one, &stats);
        assert!(
            (slice_hits[0].bm25 - corpus_hits[0].bm25).abs() > 1e-12,
            "corpus IDF must differ from 1-doc slice IDF"
        );
        let prod = collect_relevance_candidates(
            &view,
            "handleList",
            8,
            RankingArm::ProductionBlended,
            None,
            Some(&stats),
        );
        assert!(
            prod.is_empty(),
            "production arm must ignore persisted BM25 even when stats are supplied"
        );
        let hits = collect_relevance_candidates(
            &view,
            "handleList",
            8,
            RankingArm::LexicalThenGraph,
            None,
            Some(&stats),
        );
        assert_eq!(hits[0].name, "handleList");
    }
}
