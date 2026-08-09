//! Optional semantic ranker E2E (SCC-071): with a local Ollama serving an
//! embedding model, `scc embed` + task context must surface entities the
//! lexical pass misses. Skips gracefully when Ollama is unreachable.

mod golden;

fn ollama_available() -> bool {
    let cfg = scc_indexer::embed::EmbedConfig {
        base_url: "http://127.0.0.1:11434/v1".into(),
        model: "all-minilm".into(),
        api_key: None,
        rerank_model: None,
    };
    scc_indexer::embed::embed_texts(&cfg, &["ping"]).is_ok()
}

#[test]
fn embeddings_surface_semantic_candidates() {
    if !ollama_available() {
        eprintln!("ollama not running — skipping semantic ranker E2E");
        return;
    }
    let repo = tempfile::TempDir::new().unwrap();
    let root = golden::workdir(repo.path());
    std::fs::create_dir_all(root.join("svc")).unwrap();
    std::fs::create_dir_all(root.join(".scc")).unwrap();
    std::fs::write(
        root.join("svc/payments.py"),
        "class PaymentDb:\n    def insert(self, amount):\n        return amount\n\ndef handle_payment(a):\n    return PaymentDb().insert(a)\n",
    )
    .unwrap();
    std::fs::write(
        root.join(".scc/config.yaml"),
        "schema: 1\ninference:\n  enabled: true\n  provider: ollama\n  embedding_model: all-minilm\n",
    )
    .unwrap();
    golden::run_ok(&root, &["index", "--quiet"]);
    golden::run_ok(&root, &["embed"]);

    // goal with zero surface overlap: "cash movement handling"
    let out = golden::run(
        &root,
        &["context", "task", "cash movement handling", "--json"],
    );
    assert!(out.status.success());
    let pack: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let ids: Vec<String> = pack["entity_ids"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    assert!(
        ids.iter().any(|i| i.contains("PaymentDb")),
        "semantic pass must surface PaymentDb: {ids:?}"
    );
}

#[test]
fn inference_disabled_keeps_lexical_behavior() {
    let repo = tempfile::TempDir::new().unwrap();
    let root = golden::workdir(repo.path());
    std::fs::create_dir_all(root.join("svc")).unwrap();
    std::fs::write(
        root.join("svc/a.py"),
        "def helper():\n    return 1\n",
    )
    .unwrap();
    golden::run_ok(&root, &["index", "--quiet"]);
    let out = golden::run_ok(&root, &["context", "task", "helper", "--json"]);
    assert!(out.contains("helper"), "{out}");
}
