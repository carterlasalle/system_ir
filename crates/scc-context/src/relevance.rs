//! Relevance-first candidate generation (BM25 + anchors + query routing).
//!
//! This is an experimental lens. Production [`crate::rank::collect_lexical_candidates_full`]
//! is unchanged. Importance (PPR) is never added into the BM25 score here.

use crate::rank::ScoredEntity;
use scc_core::{
    kinds, relevance_hits, route_query, LexDoc, QueryShape, RankingArm, WEIGHT_BODY, WEIGHT_DOC,
    WEIGHT_NAME, WEIGHT_PATH,
};
use scc_graph::TrustedGraphView;

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
/// added into the BM25 number.
// trace:v1 id=impl.scc.context.relevance-collect work=WORK-ripwire-lessons-phase2 satisfies=REQ-ranking-arms-inspectable,REQ-exact-anchors
pub fn collect_relevance_candidates(
    view: &TrustedGraphView,
    query: &str,
    limit: usize,
    arm: RankingArm,
) -> Vec<ScoredEntity> {
    if matches!(arm, RankingArm::ProductionBlended | RankingArm::NoLexical) {
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
    let mut hits = relevance_hits(query, &docs);

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
                    for (e, hit) in entities.iter().zip(hits.iter_mut()) {
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
            for r in view.out_pred(&sid, scc_core::predicates::CALLS) {
                if out.iter().any(|c| c.id == r.object) {
                    continue;
                }
                if let Some(e) = view.entity(&r.object) {
                    out.push(ScoredEntity {
                        id: e.id.clone(),
                        kind: e.kind.clone(),
                        name: e.name.clone(),
                        score: 0.0,
                        reason: "graph-expand".into(),
                    });
                    if out.len() >= limit {
                        break;
                    }
                }
            }
        }
    }
    out
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
        let empty = collect_relevance_candidates(&view, "handleList", 8, RankingArm::ProductionBlended);
        assert!(empty.is_empty(), "production arm must not silently switch to BM25");
        let hits = collect_relevance_candidates(&view, "handleList", 8, RankingArm::LexicalThenGraph);
        assert_eq!(hits[0].name, "handleList");
        assert_eq!(hits[0].reason, "anchor");
    }

    #[test]
    // trace:v1 id=test.scc.context.query-routed-name-first verifies=REQ-query-shape-router exercises=impl.scc.context.relevance-collect
    fn query_routed_identifier_prefers_anchor() {
        let (_d, store, graph) = indexed_view(vec![
            sym("s:noise", "loginHelper", "src/login.ts", "handles the list of users"),
            sym("s:hit", "handleList", "src/server.ts", ""),
        ]);
        let view = scc_graph::TrustedGraphView::new(&graph, &store, &[], scc_graph::TrustPolicy::default());
        let hits = collect_relevance_candidates(&view, "handleList", 8, RankingArm::QueryRouted);
        assert_eq!(hits[0].id, "s:hit");
        assert_eq!(hits[0].reason, "anchor");
    }
}
