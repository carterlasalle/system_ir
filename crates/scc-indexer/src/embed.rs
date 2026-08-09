//! Optional semantic ranker (SCC-071): embeddings via any OpenAI-compatible
//! endpoint (Ollama's `/v1/embeddings`, OpenAI, or a self-hosted gateway),
//! plus an optional separate rerank model (Cohere/Jina-style `/rerank`).
//!
//! Everything here is opt-in: `inference.enabled` must be true and the
//! provider must be reachable, otherwise callers degrade gracefully to the
//! lexical/graph ranker.

use scc_store::Store;

#[derive(Debug, Clone)]
pub struct EmbedConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub rerank_model: Option<String>,
}

impl EmbedConfig {
    /// Resolve the config from the SCC inference config. `local` maps to the
    /// default Ollama OpenAI-compatible endpoint.
    pub fn from_config(cfg: &crate::config::InferenceConfig) -> EmbedConfig {
        let provider = cfg.provider.as_str();
        let base_url = if !cfg.base_url.is_empty() {
            cfg.base_url.trim_end_matches('/').to_string()
        } else if provider == "openai" {
            "https://api.openai.com/v1".to_string()
        } else {
            "http://127.0.0.1:11434/v1".to_string()
        };
        let api_key = if cfg.api_key_env.is_empty() {
            None
        } else {
            std::env::var(&cfg.api_key_env).ok().filter(|k| !k.is_empty())
        };
        EmbedConfig {
            base_url,
            model: if cfg.embedding_model.is_empty() {
                "nomic-embed-text".to_string()
            } else {
                cfg.embedding_model.clone()
            },
            api_key,
            rerank_model: if cfg.rerank_model.is_empty() {
                None
            } else {
                Some(cfg.rerank_model.clone())
            },
        }
    }
}

/// Embed one or more texts via POST {base}/embeddings (OpenAI shape).
pub fn embed_texts(cfg: &EmbedConfig, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let url = format!("{}/embeddings", cfg.base_url);
    let mut req = ureq::post(&url).timeout(std::time::Duration::from_secs(60));
    if let Some(key) = &cfg.api_key {
        req = req.set("Authorization", &format!("Bearer {key}"));
    }
    let body = serde_json::json!({ "model": cfg.model, "input": texts });
    let resp = req
        .send_json(body)
        .map_err(|e| format!("embedding request failed: {e}"))?;
    let v: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("embedding response parse failed: {e}"))?;
    let data = v
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| format!("embedding response missing data: {v}"))?;
    let mut out = Vec::with_capacity(data.len());
    for item in data {
        let emb = item
            .get("embedding")
            .and_then(|e| e.as_array())
            .ok_or_else(|| "embedding item missing vector".to_string())?;
        out.push(
            emb.iter()
                .filter_map(|x| x.as_f64().map(|f| f as f32))
                .collect(),
        );
    }
    Ok(out)
}

/// Rerank documents against a query via POST {base}/rerank (Cohere/Jina
/// shape). Returns relevance scores aligned with `documents`. Any failure
/// returns Err — callers treat that as "no reranking".
pub fn rerank(
    cfg: &EmbedConfig,
    query: &str,
    documents: &[String],
) -> Result<Vec<f64>, String> {
    let model = cfg
        .rerank_model
        .as_ref()
        .ok_or_else(|| "no rerank model configured".to_string())?;
    let url = format!("{}/rerank", cfg.base_url);
    let mut req = ureq::post(&url).timeout(std::time::Duration::from_secs(60));
    if let Some(key) = &cfg.api_key {
        req = req.set("Authorization", &format!("Bearer {key}"));
    }
    let body = serde_json::json!({
        "model": model,
        "query": query,
        "documents": documents,
    });
    let resp = req
        .send_json(body)
        .map_err(|e| format!("rerank request failed: {e}"))?;
    let v: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("rerank response parse failed: {e}"))?;
    let results = v
        .get("results")
        .and_then(|r| r.as_array())
        .ok_or_else(|| format!("rerank response missing results: {v}"))?;
    let mut scores = vec![0.0f64; documents.len()];
    for r in results {
        let idx = r.get("index").and_then(|i| i.as_u64()).unwrap_or(u64::MAX) as usize;
        let score = r.get("relevance_score").and_then(|s| s.as_f64()).unwrap_or(0.0);
        if idx < scores.len() {
            scores[idx] = score;
        }
    }
    Ok(scores)
}

pub fn cosine(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut na = 0.0f64;
    let mut nb = 0.0f64;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += (*x as f64) * (*y as f64);
        na += (*x as f64) * (*x as f64);
        nb += (*y as f64) * (*y as f64);
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// The set of entity kinds the ranker embeds.
pub const EMBED_KINDS: &[&str] = &[
    scc_core::kinds::SYMBOL,
    scc_core::kinds::COMPONENT,
    scc_core::kinds::ROUTE,
    scc_core::kinds::DATA_ENTITY,
    scc_core::kinds::DATA_STORE,
    scc_core::kinds::TEST,
];

/// Compute and store embeddings for every embeddable entity. Returns the
/// number of vectors stored.
pub fn embed_repository(store: &Store, cfg: &EmbedConfig) -> Result<usize, String> {
    let mut texts: Vec<(String, String)> = Vec::new(); // (entity_id, text)
    for kind in EMBED_KINDS {
        for e in store.entities_by_kind(kind).map_err(|e| e.to_string())? {
            let mut text = e.name.clone();
            if let Some(doc) = e.attributes.get("docstring").and_then(|v| v.as_str()) {
                text.push(' ');
                text.push_str(doc);
            }
            if let Some(resp) = e.attributes.get("responsibility").and_then(|v| v.as_str()) {
                text.push(' ');
                text.push_str(resp);
            }
            texts.push((e.id.clone(), text));
        }
    }
    // batch in chunks of 32
    let mut stored = 0usize;
    for chunk in texts.chunks(32) {
        let refs: Vec<&str> = chunk.iter().map(|(_, t)| t.as_str()).collect();
        let vectors = embed_texts(cfg, &refs)?;
        for ((id, _), vec) in chunk.iter().zip(vectors.iter()) {
            store
                .put_embedding(id, vec, &cfg.model)
                .map_err(|e| e.to_string())?;
            stored += 1;
        }
    }
    Ok(stored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Tiny in-process OpenAI-compatible /embeddings + /rerank server with
    /// deterministic fixed vectors.
    fn mock_server() -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buf = Vec::new();
                let mut tmp = [0u8; 8192];
                let header_end = loop {
                    match stream.read(&mut tmp) {
                        Ok(0) => break None,
                        Ok(n) => {
                            buf.extend_from_slice(&tmp[..n]);
                            match buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                Some(i) => break Some(i),
                                None => continue,
                            }
                        }
                        Err(_) => break None,
                    }
                };
                let Some(hdr_end) = header_end else { break };
                let headers = String::from_utf8_lossy(&buf[..hdr_end]);
                let content_length: usize = headers
                    .lines()
                    .find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.trim().eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                while buf.len() < hdr_end + 4 + content_length {
                    match stream.read(&mut tmp) {
                        Ok(0) => break,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                        Err(_) => break,
                    }
                }
                let body_start = hdr_end + 4;
                let body: serde_json::Value =
                    serde_json::from_slice(&buf[body_start..]).unwrap_or(serde_json::json!({}));
                let is_rerank = buf
                    .windows(7)
                    .any(|w| w == b"/rerank");
                let empty_arr: Vec<serde_json::Value> = Vec::new();
                let response = if is_rerank {
                    let docs = body
                        .get("documents")
                        .and_then(|d| d.as_array())
                        .unwrap_or(&empty_arr);
                    let results: Vec<serde_json::Value> = docs
                        .iter()
                        .enumerate()
                        .map(|(i, d)| {
                            let text = d.as_str().unwrap_or("");
                            let score = if text.contains("match") { 0.95 - i as f64 * 0.1 } else { 0.1 };
                            serde_json::json!({"index": i, "relevance_score": score})
                        })
                        .collect();
                    serde_json::json!({"results": results}).to_string()
                } else {
                    let inputs = body
                        .get("input")
                        .and_then(|i| i.as_array())
                        .unwrap_or(&empty_arr);
                    let data: Vec<serde_json::Value> = inputs
                        .iter()
                        .enumerate()
                        .map(|(i, t)| {
                            let text = t.as_str().unwrap_or("");
                            // deterministic pseudo-vector: unit-ish vector where
                            // "match" texts get a distinct direction
                            let base = if text.contains("match") { 0.9f32 } else { 0.1f32 };
                            let vec: Vec<f32> = (0..8).map(|d| if d == i % 8 { base } else { 0.0 }).collect();
                            serde_json::json!({"embedding": vec, "index": i})
                        })
                        .collect();
                    serde_json::json!({"data": data, "model": body.get("model")}).to_string()
                };
                let payload = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.len(),
                    response
                );
                let _ = stream.write_all(payload.as_bytes());
            }
        });
        (format!("http://{addr}"), handle)
    }

    fn test_cfg(url: &str) -> EmbedConfig {
        EmbedConfig {
            base_url: url.to_string(),
            model: "test-embed".into(),
            api_key: None,
            rerank_model: Some("test-rerank".into()),
        }
    }

    #[test]
    fn embed_texts_parses_openai_shape() {
        let (url, handle) = mock_server();
        let cfg = test_cfg(&url);
        let vectors = embed_texts(&cfg, &["match me", "other"]).unwrap();
        assert_eq!(vectors.len(), 2);
        assert_eq!(vectors[0].len(), 8);
        assert!(vectors[0][0] > vectors[1][0], "first text is the 'match'");
        drop(handle);
    }

    #[test]
    fn rerank_parses_scores() {
        let (url, handle) = mock_server();
        let cfg = test_cfg(&url);
        let scores = rerank(
            &cfg,
            "query",
            &["a match doc".to_string(), "unrelated".to_string()],
        )
        .unwrap();
        assert!(scores[0] > scores[1], "{scores:?}");
        drop(handle);
    }

    #[test]
    fn cosine_similarity() {
        let a = vec![1.0f32, 0.0];
        let b = vec![1.0f32, 0.0];
        let c = vec![0.0f32, 1.0];
        assert!((cosine(&a, &b) - 1.0).abs() < 1e-6);
        assert!(cosine(&a, &c).abs() < 1e-6);
        assert_eq!(cosine(&[], &[]), 0.0);
    }

    #[test]
    fn unreachable_provider_errors_gracefully() {
        let cfg = EmbedConfig {
            base_url: "http://127.0.0.1:1".into(), // nothing listens
            model: "x".into(),
            api_key: None,
            rerank_model: None,
        };
        assert!(embed_texts(&cfg, &["hi"]).is_err());
        assert!(rerank(&cfg, "q", &["d".into()]).is_err());
    }

    #[test]
    fn config_resolution() {
        use crate::config::InferenceConfig;
        let c = InferenceConfig {
            enabled: true,
            provider: "ollama".into(),
            embedding_model: "all-minilm".into(),
            rerank_model: "x-rerank".into(),
            base_url: String::new(),
            api_key_env: String::new(),
        };
        let cfg = EmbedConfig::from_config(&c);
        assert_eq!(cfg.base_url, "http://127.0.0.1:11434/v1");
        assert_eq!(cfg.model, "all-minilm");
        assert_eq!(cfg.rerank_model.as_deref(), Some("x-rerank"));

        let c2 = InferenceConfig {
            provider: "openai".into(),
            base_url: "https://gateway.example/v1/".into(),
            api_key_env: "SCC_API_KEY".into(),
            ..c.clone()
        };
        let cfg2 = EmbedConfig::from_config(&c2);
        assert_eq!(cfg2.base_url, "https://gateway.example/v1");
        assert_eq!(cfg2.api_key, None, "key read from env at construction");
        std::env::set_var("SCC_API_KEY", "k");
        let cfg3 = EmbedConfig::from_config(&c2);
        assert_eq!(cfg3.api_key.as_deref(), Some("k"));
        std::env::remove_var("SCC_API_KEY");
    }
}
