//! Candidate ranking (docs/CONTEXT_COMPILER.md §4–§6).
//!
//! Score = lexical overlap + FTS rank + graph/flow/ownership expansion.
//! Deterministic and dependency-free (no embeddings in MVP).

use scc_core::kinds;
use scc_graph::TrustedGraphView;
use scc_store::Store;
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub struct ScoredEntity {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub score: f64,
    /// Why this entity was selected (for evidence/debugging).
    pub reason: String,
}

/// Tokenize free text into lowercase alphanumeric terms (>= 3 chars, no
/// stopwords). Identifiers like `street-name` / `raw_text` split into their
/// pieces so goal terms match them.
pub fn terms(text: &str) -> BTreeSet<String> {
    const STOPWORDS: &[&str] = &[
        "the", "and", "for", "are", "was", "were", "with", "from", "into", "that", "this",
        "not", "but", "you", "our", "all", "can", "has", "had", "its", "who", "what",
    ];
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 3)
        .map(|t| t.to_ascii_lowercase())
        .filter(|t| !STOPWORDS.contains(&t.as_str()))
        .collect()
}

/// Light morphological variants of a term so goal words match identifier
/// stems (`normalization` → `normalize`, `transcripts` → `transcript`).
pub fn stem_variants(term: &str) -> Vec<String> {
    let mut out = vec![term.to_string()];
    if term.len() < 5 {
        return out;
    }
    if let Some(base) = term.strip_suffix("ation") {
        out.push(format!("{base}e")); // normalization -> normalize
    }
    if let Some(base) = term.strip_suffix("ations") {
        out.push(format!("{base}e")); // normalizations -> normalize
    }
    for suf in ["ing", "ings", "es", "ed", "s"] {
        if let Some(base) = term.strip_suffix(suf) {
            if base.len() >= 4 {
                out.push(base.to_string());
            }
            // drop doubled consonants: running -> run
            if base.len() >= 3 && base.ends_with(base.chars().last().unwrap()) {
                let trimmed: String = base.chars().take(base.len() - 1).collect();
                if trimmed.len() >= 3 {
                    out.push(trimmed);
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Prefix-aware term match (light stemming): `normalize` matches
/// `normalizes`/`normalizer`; `transcript` matches `transcripts`.
pub fn term_match(a: &str, b: &str) -> bool {
    if a.len() < 4 || b.len() < 4 {
        return a == b;
    }
    a.starts_with(b) || b.starts_with(a)
}

/// Lexical similarity of an entity to the goal terms: name matches weighted
/// double; prefix matches count.
pub fn entity_similarity(e: &scc_core::Entity, goal_terms: &BTreeSet<String>) -> f64 {
    if goal_terms.is_empty() {
        return 0.0;
    }
    let name_terms = terms(&e.name);
    let mut attr_text = String::new();
    for (k, v) in &e.attributes {
        if k == "docstring" || k == "signature" || k == "responsibility" || k == "path" {
            if let Some(s) = v.as_str() {
                attr_text.push_str(s);
                attr_text.push(' ');
            }
        }
    }
    let attr_terms = terms(&attr_text);
    let name_hits: usize = goal_terms
        .iter()
        .filter(|g| name_terms.iter().any(|n| term_match(g, n)))
        .count();
    let attr_hits: usize = goal_terms
        .iter()
        .filter(|g| attr_terms.iter().any(|n| term_match(g, n)))
        .count();
    (name_hits * 2 + attr_hits) as f64
}

/// Collect candidate entities for a task:
/// 1. lexical: FTS over entities + symbols
/// 2. explicit: named files/symbols
/// 3. graph expansion: containing component, flows, upstream/downstream,
///    ownership, invariants, tests — handled by the pack builder.
///
/// Pluggable semantic scorer (SCC-071): an optional provider (e.g. an
/// embedding model) contributes a relevance signal fused into the candidate
/// score. Providers are opt-in (`inference.enabled`); the default is the
/// lexical/graph ranker alone. The provider may label evidence, never invent
/// topology.
pub trait SemanticScorer: Send + Sync {
    fn score(&self, goal: &str, entity: &scc_core::Entity) -> f64;
}

/// Optional second-stage reranker (e.g. a separate cross-encoder model):
/// reorders the collected candidates after lexical/semantic collection.
/// `rerank` may reorder `candidates` in place; failures degrade gracefully
/// (the trait method should treat errors as no-op).
pub trait Reranker: Send + Sync {
    fn rerank(&self, goal: &str, candidates: &mut Vec<ScoredEntity>);
}

pub fn collect_lexical_candidates(
    store: &Store,
    view: &TrustedGraphView,
    goal: &str,
    symbols: &[String],
    limit: usize,
) -> Vec<ScoredEntity> {
    collect_lexical_candidates_with(store, view, goal, symbols, limit, None)
}

/// `collect_lexical_candidates` with an optional semantic scorer fused in.
pub fn collect_lexical_candidates_with(
    store: &Store,
    view: &TrustedGraphView,
    goal: &str,
    symbols: &[String],
    limit: usize,
    scorer: Option<&dyn SemanticScorer>,
) -> Vec<ScoredEntity> {
    collect_lexical_candidates_full(store, view, goal, symbols, limit, scorer, None)
}
// trace:v1 id=impl.scc.rank work=WORK-SCC-001 satisfies=REQ-SCC-CTX

/// `collect_lexical_candidates` with both a semantic scorer and a reranker.
pub fn collect_lexical_candidates_full(
    store: &Store,
    view: &TrustedGraphView,
    goal: &str,
    symbols: &[String],
    limit: usize,
    scorer: Option<&dyn SemanticScorer>,
    reranker: Option<&dyn Reranker>,
) -> Vec<ScoredEntity> {
    let goal_terms = terms(goal);
    let sem = |e: &scc_core::Entity| -> f64 { scorer.map(|s| s.score(goal, e)).unwrap_or(0.0) };
    let mut out: Vec<ScoredEntity> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    let push = |e: &scc_core::Entity, base: f64, reason: &str,
                    out: &mut Vec<ScoredEntity>, seen: &mut BTreeSet<String>| {
        if !seen.insert(e.id.clone()) {
            return;
        }
        let sim = entity_similarity(e, &goal_terms);
        let score = base + sim + sem(e);
        out.push(ScoredEntity {
            id: e.id.clone(),
            kind: e.kind.clone(),
            name: e.name.clone(),
            score,
            reason: reason.to_string(),
        });
    };

    // FTS entities
    if !goal_terms.is_empty() {
        let joined: Vec<&str> = goal_terms.iter().map(|s| s.as_str()).collect();
        let query = joined.join(" ");
        if let Ok(hits) = store.search_entities(&query, limit) {
            let rank = limit as f64;
            for (i, e) in hits.iter().enumerate() {
                push(e, (rank - i as f64) / rank, "lexical", &mut out, &mut seen);
            }
        }
        if let Ok(hits) = store.search_symbols(&query, limit) {
            let rank = limit as f64;
            for (i, (name, sig, _kind, file)) in hits.iter().enumerate() {
                let id = scc_core::symbol_id(&view.graph.repo_id, file, name);
                let mut e = scc_core::Entity::new(id.clone(), kinds::SYMBOL, name.clone());
                e.attr("file", serde_json::json!(file));
                if !sig.is_empty() {
                    e.attr("signature", serde_json::json!(sig));
                }
                push(&e, (rank - i as f64) / rank, "lexical", &mut out, &mut seen);
            }
        }
        // substring fallback with stem variants (FTS prefix matching cannot
        // catch `normalization` → `normalize`)
        for term in &goal_terms {
            for variant in crate::rank::stem_variants(term) {
                if let Ok(hits) = store.search_entities_like(&variant, 6) {
                    for e in hits.iter() {
                        push(e, 0.6, "substring", &mut out, &mut seen);
                    }
                }
                if let Ok(hits) = store.search_symbols_like(&variant, 6) {
                    for (name, sig, _kind, file) in hits.iter() {
                        let id = scc_core::symbol_id(&view.graph.repo_id, file, name);
                        let mut e = scc_core::Entity::new(id.clone(), kinds::SYMBOL, name.clone());
                        e.attr("file", serde_json::json!(file));
                        if !sig.is_empty() {
                            e.attr("signature", serde_json::json!(sig));
                        }
                        push(&e, 0.6, "substring", &mut out, &mut seen);
                    }
                }
            }
        }
    }

    // semantic proposal pass: with a scorer present, entities with a
    // positive semantic signal surface even when no lexical term matches
    // (docs/CONTEXT_COMPILER.md §4 — embeddings are never truth, but they
    // may propose candidates)
    if scorer.is_some() {
        for e in view.entities() {
            let sem_score = sem(e);
            if sem_score > 0.0 {
                push(e, 0.0, "semantic", &mut out, &mut seen);
            }
        }
    }

    // explicit symbols
    for s in symbols {
        let matches: Vec<String> = view
            .entities_of_kind(kinds::SYMBOL)
            .into_iter()
            .filter(|e| e.name == *s)
            .map(|e| e.id.clone())
            .collect();
        for id in matches {
            if let Some(e) = view.entity(&id) {
                push(e, 2.0, "explicit-symbol", &mut out, &mut seen);
            }
        }
        // fallback: entity id directly
        if view.entity(s).is_some() {
            push(view.entity(s).unwrap(), 2.0, "explicit-id", &mut out, &mut seen);
        }
    }

    // graph expansion: callers (upstream) and callees (downstream) of
    // candidate symbols (docs/CONTEXT_COMPILER.md §6)
    let symbol_candidates: Vec<String> = out
        .iter()
        .filter(|c| c.kind == kinds::SYMBOL)
        .map(|c| c.id.clone())
        .collect();
    for sid in &symbol_candidates {
        for r in view.in_pred(sid, "calls") {
            if let Some(e) = view.entity(&r.subject) {
                push(e, 1.0, "upstream", &mut out, &mut seen);
            }
        }
        for r in view.out_pred(sid, "calls") {
            if let Some(e) = view.entity(&r.object) {
                push(e, 0.8, "downstream", &mut out, &mut seen);
            }
        }
    }

    // Routes are contracts — they must always surface when they match the
    // goal, regardless of lexical crowding from the fact layer (exports/
    // contracts/tests compete for the same candidate budget). Ground-truth
    // recall for route items depends on these surviving truncation.
    for r in view.entities_of_kind(kinds::ROUTE) {
        let name_l = r.name.to_ascii_lowercase();
        if goal_terms.iter().any(|t| name_l.contains(t)) {
            push(r, 3.0, "route-contract", &mut out, &mut seen);
        }
    }

    out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    if let Some(rr) = reranker {
        rr.rerank(goal, &mut out);
    }
    out.truncate((limit + 8).max(20));
    out
}

// ---------------------------------------------------------------------------
// Startup-atlas confidence ranking (Wave 11 — GENERALIZATION II)
// ---------------------------------------------------------------------------
//
// `rank_startup_atlas` orders the startup atlas' three fact sections
// (components, entrypoints, contracts) so the highest-confidence entries
// render first. The rule is precision via ordering, NOT deletion: every
// entry survives, but evidence-backed facts (graph-derived edges, explicit
// surface kinds, producer/consumer contracts) rank above heuristic ones
// (bare-name components, generic flow-compiler entrypoints, schema/reactive
// inferences), so an agent under a token budget reads the strongest facts
// first. The scoring is a pure function of the `SystemAtlas` fields — no
// store access, no heuristics invented from a bare get() — and is fully
// deterministic: ties break on the entry id (lexicographic).

use scc_core::{AtlasComponent, AtlasEntrypoint, Contract, ContractSubclass, SystemAtlas};

/// Confidence (0..1) of one component entry: how much of it is graph-derived
/// evidence vs a bare-name heuristic.
///
/// A component named only (no purpose, no implementation, no edges) is the
/// heuristic floor (`0.2`) — the component compiler can still emit a name
/// from clustering alone. Every attribute that carries an EXTRACTED/
/// graph-resolved fact raises the score: a responsibility claim (purpose),
/// implementation paths + member symbols, consumes/produces and
/// upstream/downstream edges, ownership claims, failure behavior, and a
/// hierarchy layer assigned by the clusterer. The strongest components
/// (all evidence present) reach `1.0`.
pub fn component_confidence(c: &AtlasComponent) -> f64 {
    let mut score: f64 = 0.2; // bare-name heuristic floor
    if !c.purpose.is_empty() {
        score += 0.15;
    }
    if !c.implementation.is_empty() {
        score += 0.10;
    }
    if !c.symbols.is_empty() {
        score += 0.10;
    }
    if !c.consumes.is_empty() {
        score += 0.10;
    }
    if !c.produces.is_empty() {
        score += 0.10;
    }
    if !c.upstream.is_empty() {
        score += 0.10;
    }
    if !c.downstream.is_empty() {
        score += 0.10;
    }
    if !c.owns.is_empty() {
        score += 0.10;
    }
    if !c.failure_behavior.is_empty() {
        score += 0.05;
    }
    if c.layer != "component" {
        score += 0.05; // hierarchy clusterer assigned a real layer
    }
    score.min(1.0)
}

/// Confidence (0..1) of one entrypoint entry by surface kind: extractor
/// evidence-backed kinds outrank the flow compiler's generic heuristic.
///
/// `route`/`http` (ROUTE entities), `cli`/`cli-subcommand` (CLI flag
/// facts), `public_api` (EXPORTS evidence), `event`/`queue` (topic
/// publish/subscribe), and the other invocation-surface kinds all come from
/// explicit extractor facts. The generic `entrypoint` kind is the flow
/// compiler's heuristic marker and ranks lower; an unknown/empty kind is the
/// floor. A resolved symbol id is a weak positive signal on top.
pub fn entrypoint_confidence(e: &AtlasEntrypoint) -> f64 {
    let base: f64 = match e.kind.as_str() {
        "route" | "http" => 1.0,
        "cli" | "cli-subcommand" => 0.95,
        "public_api" | "public-api" => 0.9,
        "event" => 0.85,
        "queue" => 0.85,
        "schedule" => 0.8,
        "process" => 0.8,
        "plugin" => 0.75,
        "framework_callback" => 0.75,
        "lifecycle" => 0.7,
        "entrypoint" => 0.4, // heuristic, not extractor evidence
        _ => 0.3,
    };
    let symbol_bonus = if e.symbol.is_empty() { 0.0 } else { 0.05 };
    (base + symbol_bonus).min(1.0)
}

/// Confidence (0..1) of one contract entry by subclass evidence: concrete
/// surfaces with a producer + consumers outrank schema/reactive inferences.
///
/// `http`/`cli`/`event`/`config` contracts come from ROUTE / CLI-flag /
/// TOPIC / CONFIGURATION entities — the strongest evidence. A real producer
/// symbol, non-empty consumers, and preserved evidence ids each add to the
/// score. `schema` (SchemaDefinition, derived from validation/model
/// schemas) and the other inferred subclasses rank lowest: the contract
/// ontology knows the surface is a schema, but there is no producer or
/// consumer wiring to make it an observed contract. The gap is engineered:
/// even a bare http contract (`0.95`) outranks a fully-wired schema
/// (`0.35 + 0.10 + 0.05 + 0.05 = 0.55`).
pub fn contract_confidence(c: &Contract) -> f64 {
    let base: f64 = match c.subclass {
        ContractSubclass::Http => 0.95,
        ContractSubclass::Cli => 0.90,
        ContractSubclass::Event => 0.85,
        ContractSubclass::Configuration => 0.80,
        ContractSubclass::PublicApi => 0.75,
        ContractSubclass::Rpc => 0.75,
        ContractSubclass::Message => 0.70,
        ContractSubclass::Plugin => 0.70,
        ContractSubclass::Extension => 0.70,
        ContractSubclass::Serialization => 0.65,
        ContractSubclass::CallContract => 0.60,
        ContractSubclass::Schema => 0.35, // schema/reactive: inferred, not observed
    };
    let producer_bonus = if c.producer.is_empty() || c.producer == c.id {
        0.0
    } else {
        0.10
    };
    let consumers_bonus = if c.consumers.is_empty() { 0.0 } else { 0.05 };
    let evidence_bonus = if c.evidence.is_empty() { 0.0 } else { 0.05 };
    (base + producer_bonus + consumers_bonus + evidence_bonus).min(1.0)
}

/// Order the startup atlas sections by evidence-backed confidence, highest
/// first. Every entry is kept — this is precision via ordering, never
/// deletion. Deterministic: ties break on the entry id (lexicographic; for
/// entrypoints, which carry no id, on name then kind then symbol).
pub fn rank_startup_atlas(atlas: &mut SystemAtlas) {
    atlas.components.sort_by(|a, b| {
        component_confidence(b)
            .partial_cmp(&component_confidence(a))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.name.cmp(&b.name))
    });
    atlas.entrypoints.sort_by(|a, b| {
        entrypoint_confidence(b)
            .partial_cmp(&entrypoint_confidence(a))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.symbol.cmp(&b.symbol))
    });
    atlas.contracts.sort_by(|a, b| {
        contract_confidence(b)
            .partial_cmp(&contract_confidence(a))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn term_tokenization() {
        let t = terms("change street-name normalization for transcripts");
        assert!(t.contains("street"));
        assert!(t.contains("name"));
        assert!(t.contains("transcripts"));
        assert!(!t.contains("for"));
        let t2 = terms("raw_text");
        assert!(t2.contains("raw"));
        assert!(t2.contains("text"));
    }

    #[test]
    fn prefix_match() {
        assert!(term_match("normalize", "normalizes"));
        assert!(term_match("transcript", "transcripts"));
        assert!(term_match("normalizer", "normalize"));
        assert!(!term_match("cat", "category"));
        assert!(!term_match("run", "running"));
    }

    struct BoostScorer;
    impl SemanticScorer for BoostScorer {
        fn score(&self, _goal: &str, e: &scc_core::Entity) -> f64 {
            if e.name.to_ascii_lowercase().contains("boost") {
                5.0
            } else {
                0.0
            }
        }
    }

    #[test]
    fn semantic_scorer_fuses_into_candidates() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = scc_store::Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let mut e1 = scc_core::Entity::new("repo://r/symbol/a.py/boosted", "symbol", "boosted");
        e1.attr("file", serde_json::json!("a.py"));
        store.insert_entity(&e1, &["a.py".into()]).unwrap();
        let mut e2 = scc_core::Entity::new("repo://r/symbol/a.py/plain", "symbol", "plain");
        e2.attr("file", serde_json::json!("a.py"));
        store.insert_entity(&e2, &["a.py".into()]).unwrap();
        let graph = scc_graph::RealityGraph::load(&store).unwrap();
        let view = scc_graph::TrustedGraphView::new(&graph, &store, &[], scc_graph::TrustPolicy::default());
        let plain = collect_lexical_candidates(&store, &view, "nothing matches", &[], 10);
        assert_eq!(plain.len(), 0, "lexical-only finds nothing");
        let fused = collect_lexical_candidates_with(
            &store, &view, "nothing matches", &[], 10, Some(&BoostScorer),
        );
        assert!(
            fused.iter().any(|c| c.name == "boosted" && c.score >= 5.0),
            "semantic signal must surface boosted: {fused:?}"
        );
        assert!(fused[0].name == "boosted", "boosted ranks first: {fused:?}");
    }

    #[test]
    fn similarity_weights_names() {
        let mut e = scc_core::Entity::new("repo://r/component/x", "component", "transcript-normalizer");
        e.attr("responsibility", serde_json::json!("normalizes radio transcripts"));
        let t = terms("normalize transcripts");
        let s = entity_similarity(&e, &t);
        assert!(s > 0.0, "got {s}");
        assert!(s >= 4.0, "expected name+attr hits, got {s}");
    }

    // ---- Wave 11: startup-atlas confidence ranking ----

    fn component(name: &str) -> AtlasComponent {
        AtlasComponent {
            name: name.to_string(),
            purpose: String::new(),
            implementation: Vec::new(),
            implementation_paths: Vec::new(),
            symbols: Vec::new(),
            consumes: Vec::new(),
            produces: Vec::new(),
            upstream: Vec::new(),
            downstream: Vec::new(),
            failure_behavior: Vec::new(),
            owns: Vec::new(),
            layer: "component".into(),
            parent: None,
        }
    }

    fn rich_component(name: &str) -> AtlasComponent {
        let mut c = component(name);
        c.purpose = "owns the billing pipeline".into();
        c.implementation = vec!["src/billing".into(), "BillingService".into()];
        c.symbols = vec!["BillingService".into()];
        c.consumes = vec!["db.ledger".into()];
        c.produces = vec!["Invoice".into()];
        c.upstream = vec!["api".into()];
        c.downstream = vec!["notifier".into()];
        c.owns = vec![scc_core::AtlasOwnershipClaim {
            target: "db.ledger".into(),
            provenance: "write-edge".into(),
        }];
        c.layer = "subsystem".into();
        c
    }

    fn empty_atlas() -> SystemAtlas {
        SystemAtlas {
            repository: String::new(),
            revision: String::new(),
            indexed_at: String::new(),
            freshness: String::new(),
            purpose: String::new(),
            components: Vec::new(),
            entrypoints: Vec::new(),
            contracts: Vec::new(),
            coverage: BTreeMap::new(),
            flows: Vec::new(),
            invariants: Vec::new(),
            deployment_units: Vec::new(),
            external_systems: Vec::new(),
            trust_boundaries: Vec::new(),
            async_boundaries: Vec::new(),
            implementation_map: BTreeMap::new(),
            data_stores: Vec::new(),
            archetype: None,
            state_authority: BTreeMap::new(),
            hierarchy: Vec::new(),
            evidence_summary: BTreeMap::new(),
            warnings: Vec::new(),
            public_api: BTreeMap::new(),
            framework_semantics: BTreeMap::new(),
            pipeline: Vec::new(),
            landmarks: Vec::new(),
        }
    }

    #[test]
    fn component_confidence_orders_evidence_over_heuristic() {
        // a bare-name component is the heuristic floor
        assert!((component_confidence(&component("zzz_bare")) - 0.2).abs() < 1e-9);
        // a fully-evidenced component is capped at 1.0, never above
        assert_eq!(component_confidence(&rich_component("billing")), 1.0);
        // every graph attribute contributes
        let mut mid = component("mid");
        mid.purpose = "p".into();
        mid.implementation = vec!["src/mid".into()];
        mid.owns = vec![scc_core::AtlasOwnershipClaim {
            target: "db.x".into(),
            provenance: "write-edge".into(),
        }];
        let bare = component_confidence(&component("bare"));
        let mid_score = component_confidence(&mid);
        let rich_score = component_confidence(&rich_component("billing"));
        assert!(mid_score > bare, "{mid_score} > {bare}");
        assert!(rich_score > mid_score, "{rich_score} > {mid_score}");
        assert!((0.0..=1.0).contains(&mid_score));
    }

    #[test]
    fn entrypoint_confidence_ranks_surface_kinds() {
        let ep = |kind: &str, symbol: &str| AtlasEntrypoint {
            name: "e".into(),
            kind: kind.into(),
            trigger: "t".into(),
            symbol: symbol.into(),
        };
        // http/cli/public_api evidence-backed > generic heuristic
        assert!(entrypoint_confidence(&ep("route", "s")) > entrypoint_confidence(&ep("entrypoint", "s")));
        assert!(entrypoint_confidence(&ep("http", "s")) > entrypoint_confidence(&ep("entrypoint", "s")));
        assert!(entrypoint_confidence(&ep("cli-subcommand", "s")) > entrypoint_confidence(&ep("entrypoint", "s")));
        assert!(entrypoint_confidence(&ep("public_api", "s")) > entrypoint_confidence(&ep("entrypoint", "s")));
        // unknown kind is the floor
        assert!(entrypoint_confidence(&ep("entrypoint", "s")) > entrypoint_confidence(&ep("", "s")));
        // a resolved symbol id is a weak positive signal (on a kind below
        // the 1.0 cap, so the bonus is observable)
        assert!(entrypoint_confidence(&ep("entrypoint", "repo://x/symbol/h")) > entrypoint_confidence(&ep("entrypoint", "")));
        assert!((0.0..=1.0).contains(&entrypoint_confidence(&ep("route", "s"))));
    }

    #[test]
    fn contract_confidence_ranks_surface_subclasses_over_schema() {
        let contract = |subclass: ContractSubclass, producer: &str, consumers: usize, evidence: usize| Contract {
            id: "id".into(),
            kind: subclass.as_str().into(),
            subclass,
            producer: producer.into(),
            consumers: (0..consumers).map(|i| format!("c{i}")).collect(),
            operations: vec!["op".into()],
            evidence: (0..evidence).map(|i| format!("ev{i}")).collect(),
        };
        let schema = contract(ContractSubclass::Schema, "", 0, 0);
        let schema_wired = contract(ContractSubclass::Schema, "producer", 2, 2);
        // http/cli/event/config with producer+consumers beat schema (even a
        // fully-wired schema): 0.55 max vs 0.95 bare http
        for subclass in [
            ContractSubclass::Http,
            ContractSubclass::Cli,
            ContractSubclass::Event,
            ContractSubclass::Configuration,
        ] {
            let surface = contract(subclass, "producer", 2, 2);
            assert!(
                contract_confidence(&surface) > contract_confidence(&schema_wired),
                "{:?} must beat schema",
                subclass
            );
        }
        // wiring raises a contract's confidence
        assert!(contract_confidence(&schema_wired) > contract_confidence(&schema));
        assert!((contract_confidence(&schema) - 0.35).abs() < 1e-9);
        assert!((0.0..=1.0).contains(&contract_confidence(&schema_wired)));
    }

    #[test]
    fn rank_startup_atlas_sorts_mixed_evidence_deterministically() {
        let mut atlas = empty_atlas();
        atlas.components = vec![
            rich_component("billing"),
            component("zzz_bare"),
            rich_component("aaa_billing_dup"), // same confidence as billing
            component("aaa_bare"),
        ];
        atlas.entrypoints = vec![
            AtlasEntrypoint { name: "zzz".into(), kind: "entrypoint".into(), trigger: "t".into(), symbol: "".into() },
            AtlasEntrypoint { name: "api".into(), kind: "route".into(), trigger: "GET /x".into(), symbol: "repo://r/symbol/h".into() },
            AtlasEntrypoint { name: "cli".into(), kind: "cli-subcommand".into(), trigger: "t".into(), symbol: "".into() },
        ];
        atlas.contracts = vec![
            Contract::new("c-schema", "schema", "").with_subclass(ContractSubclass::Schema),
            Contract::new("c-http", "http", "repo://r/symbol/h").with_subclass(ContractSubclass::Http),
            Contract::new("c-cli", "cli", "").with_subclass(ContractSubclass::Cli),
        ];

        rank_startup_atlas(&mut atlas);

        // components: evidence first, then bare; ties by name lexicographic
        assert_eq!(atlas.components[0].name, "aaa_billing_dup");
        assert_eq!(atlas.components[1].name, "billing");
        assert_eq!(atlas.components[2].name, "aaa_bare");
        assert_eq!(atlas.components[3].name, "zzz_bare");
        // every entry kept — precision via ordering, not deletion
        assert_eq!(atlas.components.len(), 4);

        // entrypoints: evidence-backed surfaces first, heuristic last
        assert_eq!(atlas.entrypoints[0].kind, "route");
        assert_eq!(atlas.entrypoints[1].kind, "cli-subcommand");
        assert_eq!(atlas.entrypoints[2].kind, "entrypoint");
        assert_eq!(atlas.entrypoints.len(), 3);

        // contracts: http > cli > schema, ties by id
        assert_eq!(atlas.contracts[0].id, "c-http");
        assert_eq!(atlas.contracts[1].id, "c-cli");
        assert_eq!(atlas.contracts[2].id, "c-schema");
        assert_eq!(atlas.contracts.len(), 3);

        // idempotent + deterministic: re-ranking changes nothing
        let snapshot = format!("{:?}", atlas);
        rank_startup_atlas(&mut atlas);
        assert_eq!(format!("{:?}", atlas), snapshot);
    }
}
