fn main() {
    let qw = std::env::var("QW").unwrap();
    let root = std::path::Path::new(&qw);
    let store = scc_store::Store::open(&root.join(".scc/scc.db"), root).unwrap();
    let graph = scc_graph::RealityGraph::load(&store).unwrap();
    let view = scc_graph::TrustedGraphView::new(&graph, &store, &[], scc_graph::TrustPolicy::default());
    let goal = "add retry handling to the asr transcription call";
    let cands = scc_context::rank::collect_lexical_candidates(&store, &view, goal, &[], 30);
    println!("candidates ({}):", cands.len());
    for c in cands.iter() {
        println!("  {} [{}] score={:.2} reason={}", c.name, c.kind, c.score, c.reason);
    }
    // check the consume -> transcribe edge
    let transcribe = scc_core::symbol_id("repo", "src/asr/client.ts", "transcribe");
    println!("in-edges of transcribe:");
    for r in view.in_pred(&transcribe, "calls") {
        println!("  {} calls {}", r.subject, r.object);
    }
}
