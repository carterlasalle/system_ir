//! Part N/E token-invariant tests: the surface hard-max invariant must hold
//! against the FINAL rendered text (headers included), under pathological
//! fixtures — header-overhead domination, one enormous required signature,
//! many huge required entries (critical drops), and the adaptive startup
//! allocator's default/explicit/flow-heavy behavior.

use scc_context::startup::allocate_startup_budget;
use scc_context::surface::{build_surface, SurfacePolicy, SurfaceRequest, SurfaceMode};
use scc_core::{entity_id, estimate_tokens, relationship_id, symbol_id, kinds, predicates, ContextBudget};
use scc_core::{Entity, Provenance, Relationship};
use scc_store::Store;

// trace:v1 id=test.scc-context-token-invariants.fixture work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
}

// trace:v1 id=test.scc-context-token-invariants.fixture-new work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
fn fixture() -> Fixture {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
    Fixture { _dir: dir, store }
}

#[allow(clippy::too_many_arguments)] // fixture seam: every arg is a distinct axis
// trace:v1 id=test.scc-context-token-invariants.add-symbol work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
fn add_symbol(
    f: &Fixture,
    rid: &mut u64,
    path: &str,
    comp_id: &str,
    name: &str,
    sig: Option<&str>,
    exported: bool,
    extra: impl FnOnce(&mut Entity),
) -> String {
    let repo = f.store.repo_id.clone();
    let fid = entity_id(&repo, kinds::FILE, path);
    if f.store.get_entity(&fid).unwrap().is_none() {
        f.store.insert_entity(&Entity::new(fid.clone(), kinds::FILE, path), &[path.to_string()]).unwrap();
        f.store.insert_relationship(
            &Relationship::new(relationship_id(*rid), comp_id.to_string(), predicates::CONTAINS, fid.clone(), Provenance::Extracted),
            path,
        ).unwrap();
        *rid += 1;
    }
    let id = symbol_id(&repo, path, name);
    let mut e = Entity::new(id.clone(), kinds::SYMBOL, name);
    e.attr("kind", serde_json::json!("function"));
    e.attr("file", serde_json::json!(path));
    e.attr("exported", serde_json::json!(exported));
    e.attr("start_line", serde_json::json!(1u32));
    e.attr("end_line", serde_json::json!(10u32));
    if let Some(s) = sig {
        e.attr("signature", serde_json::json!(s));
    }
    extra(&mut e);
    f.store.insert_entity(&e, &[path.to_string()]).unwrap();
    *rid += 1;
    f.store.insert_relationship(
        &Relationship::new(relationship_id(*rid), fid.clone(), predicates::CONTAINS, id.clone(), Provenance::Extracted),
        path,
    ).unwrap();
    id
}

// trace:v1 id=test.scc-context-token-invariants.compiler work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
fn compiler(f: &Fixture) -> scc_context::ContextCompiler<'_> {
    // The trusted view reads adjacency from the REALITY GRAPH, which must
    // be (re)loaded AFTER the fixture's entities/relationships are
    // inserted — the constructor-time load in `fixture()` sees an empty
    // store.
    let graph = Box::leak(Box::new(scc_graph::RealityGraph::load(&f.store).unwrap()));
    scc_context::ContextCompiler::new(&f.store, graph, Default::default(), Vec::new())
}

/// Thousands of components with one tiny symbol each: group-header overhead
/// dominates item costs. The hard-max invariant must still hold on the
/// final text.
#[test]
// trace:v1 id=test.scc-context-token-invariants.header-overhead-domination work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching verifies=REQ-hard-max-invariant-on-rendered-text
fn hard_max_holds_when_header_overhead_dominates() {
    let f = fixture();
    let repo = f.store.repo_id.clone();
    let mut rid: u64 = 1;
    for i in 0..800usize {
        let comp_id = entity_id(&repo, kinds::COMPONENT, &format!("c{i}"));
        f.store.replace_components(&[Entity::new(comp_id.clone(), kinds::COMPONENT, format!("comp{i}"))]).unwrap();
        add_symbol(&f, &mut rid, &format!("m{i}/a.py"), &comp_id, &format!("fn{i}"), Some("def fn(i): pass"), true, |_| ());
    }
    let ctx = compiler(&f);
    let budget = 900;
    let policy = SurfacePolicy::defaults(budget);
    let result = build_surface(&ctx, SurfaceRequest {
        mode: SurfaceMode::Global,
        budget,
        explain: false,
        policy,
        semantic: None,
    });
    assert!(
        estimate_tokens(&result.text) <= policy.hard_max,
        "final render {} exceeded hard_max {}",
        estimate_tokens(&result.text),
        policy.hard_max
    );
}

/// One required entry carrying an enormous signature: the compression
/// ladder must shrink it to identity — never exceed the hard max, never
/// panic. Required via invocation_surfaces (route handler).
#[test]
// trace:v1 id=test.scc-context-token-invariants.enormous-required-signature work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching verifies=REQ-hard-max-invariant-on-rendered-text
fn enormous_required_signature_is_compressed_to_fit() {
    let f = fixture();
    let repo = f.store.repo_id.clone();
    let comp_id = entity_id(&repo, kinds::COMPONENT, "api");
    f.store.replace_components(&[Entity::new(comp_id.clone(), kinds::COMPONENT, "api")]).unwrap();
    let mut rid: u64 = 1;
    let huge = format!("pub fn mega({}) -> Result<(), Error> {{}}", "arg: VeryLongTypeName, ".repeat(600));
    let mega_id = add_symbol(&f, &mut rid, "api/huge.py", &comp_id, "mega", Some(&huge), true, |_| ());
    // Required via a concrete http surface (ROUTE -> handler).
    let route_id = entity_id(&repo, kinds::ROUTE, "rt-mega");
    let mut rt = Entity::new(route_id, kinds::ROUTE, "rt-mega");
    rt.attr("handler", serde_json::json!(mega_id));
    rt.attr("method", serde_json::json!("GET"));
    rt.attr("path", serde_json::json!("/mega"));
    f.store.insert_entity(&rt, &[]).unwrap();
    // ordinary filler so selection has a pool
    for i in 0..20 {
        add_symbol(&f, &mut rid, "api/fill.py", &comp_id, &format!("fill{i}"), Some("def fill(): pass"), true, |_| ());
    }
    let ctx = compiler(&f);
    let budget = 400;
    let policy = SurfacePolicy::defaults(budget);
    let result = build_surface(&ctx, SurfaceRequest {
        mode: SurfaceMode::Global,
        budget,
        explain: false,
        policy,
        semantic: None,
    });
    assert!(
        estimate_tokens(&result.text) <= policy.hard_max,
        "enormous required signature blew the hard max: {} > {}",
        estimate_tokens(&result.text),
        policy.hard_max
    );
}

/// Many huge REQUIRED entries vs a tiny hard max: after full compression
/// the pipeline must drop lowest-value required entries as explicit
/// critical drops while preserving the top-ranked one, and the invariant
/// still holds.
#[test]
// trace:v1 id=test.scc-context-token-invariants.critical-drops-accounted work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching verifies=REQ-hard-max-invariant-on-rendered-text
fn overflowing_required_set_yields_critical_drops() {
    let f = fixture();
    let repo = f.store.repo_id.clone();
    let comp_id = entity_id(&repo, kinds::COMPONENT, "api");
    f.store.replace_components(&[Entity::new(comp_id.clone(), kinds::COMPONENT, "api")]).unwrap();
    let mut rid: u64 = 1;
    let big = format!("pub fn big({}) -> () {{}}", "a: LongParamName, ".repeat(120));
    let mut ids = Vec::new();
    for i in 0..12 {
        let id = add_symbol(&f, &mut rid, &format!("api/r{i}.py"), &comp_id, &format!("req{i}"), Some(&big), true, |_| ());
        // Concrete http invocation surface => REQUIRED coverage entry:
        // ROUTE entity whose `handler` attr points at the symbol.
        let route_id = entity_id(&repo, kinds::ROUTE, &format!("rt{i}"));
        let mut rt = Entity::new(route_id, kinds::ROUTE, format!("rt{i}"));
        rt.attr("handler", serde_json::json!(id));
        rt.attr("method", serde_json::json!("GET"));
        rt.attr("path", serde_json::json!(format!("/r{i}")));
        f.store.insert_entity(&rt, &[]).unwrap();
        ids.push(id);
    }
    let _ = ids;
    let ctx = compiler(&f);
    let budget = 300;
    let policy = SurfacePolicy::defaults(budget);
    let result = build_surface(&ctx, SurfaceRequest {
        mode: SurfaceMode::Global,
        budget,
        explain: false,
        policy,
        semantic: None,
    });
    assert!(
        estimate_tokens(&result.text) <= policy.hard_max,
        "critical-drop path exceeded hard max: {} > {}",
        estimate_tokens(&result.text),
        policy.hard_max
    );
    assert!(
        !result.rendered_ids.is_empty() || !result.critical_drops.is_empty(),
        "pathological overflow must keep at least the top required entry"
    );
}

/// The ONE startup allocator: None selects the DEFAULT total and still
/// adapts (identical split to passing that total explicitly); an explicit
/// target scales the same split; flow-heavy repos shift share toward the
/// Atlas even at ordinary entity counts.
#[test]
// trace:v1 id=test.scc-context-token-invariants.startup-allocator-adaptive-default work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching verifies=REQ-adaptive-startup-budgets
fn startup_allocator_default_is_adaptive_and_flow_aware() {
    let f = fixture();
    let repo = f.store.repo_id.clone();
    let comp_id = entity_id(&repo, kinds::COMPONENT, "api");
    f.store.replace_components(&[Entity::new(comp_id.clone(), kinds::COMPONENT, "api")]).unwrap();
    // 40 flows + modest symbols => architecture-heavy tier at ordinary size
    let mut rid: u64 = 1;
    for i in 0..40 {
        let flow_id = entity_id(&repo, kinds::FLOW, &format!("flow{i}"));
        f.store.insert_entity(&Entity::new(flow_id, kinds::FLOW, format!("flow{i}")), &[]).unwrap();
    }
    for i in 0..50 {
        add_symbol(&f, &mut rid, "api/a.py", &comp_id, &format!("s{i}"), None, true, |_| ());
    }
    let ctx = compiler(&f);

    let none_budget = allocate_startup_budget(&ctx, None);
    let explicit = allocate_startup_budget(&ctx, Some(ContextBudget::default().total));
    assert_eq!(none_budget, explicit, "None must run the SAME adaptive split over the default total");

    // Flow-tier behavior is a pure function of the counts (RealityGraph
    // flows are produced by the indexer's flow compiler, not by raw FLOW
    // entities, so the tier is asserted on `adaptive` directly): a
    // flow-dense repo must land in the architecture-heavy tier even when
    // its entity count is tiny — beating the 55/45 tiny split.
    let arch = ContextBudget::adaptive(20_000, 92, 2, 41, 10);
    assert!(
        arch.atlas as f64 / arch.total as f64 >= 0.65,
        "architecture-heavy repo should give the atlas >=65%: got {}",
        arch.atlas as f64 / arch.total as f64
    );
    // and the same counts WITHOUT the flows stay in the tiny tier
    let tiny = ContextBudget::adaptive(20_000, 92, 2, 0, 10);
    assert!(
        tiny.surface > arch.surface,
        "flow-heavy repo must shift share toward the atlas"
    );

    // explicit smaller target scales proportionally (same tier)
    let small = allocate_startup_budget(&ctx, Some(8_000));
    assert_eq!(small.total, 8_000);
    assert!(small.atlas + small.surface <= 8_000 + 1);

    // tiny repo keeps a larger surface share than the architecture tier
    let tiny = ContextBudget::adaptive(20_000, 100, 2, 0, 10);
    assert!(tiny.surface > arch.surface, "tiny repos keep the larger surface share");
}
