//! Cross-language semantic bridges (Phase 6).
//!
//! Protobuf CONTRACT entities (`kind=rpc`) are linked to name-matched
//! symbols as IMPLEMENTS (server/svc/handler paths) or INVOKES
//! (client/web paths). The linker never emits CALLS between languages.

use crate::write::rel_id;
use scc_core::kinds;
use scc_core::{Evidence, Provenance, Relationship};
use scc_store::Store;

/// Attach INVOKES/IMPLEMENTS from symbols onto rpc CONTRACTs.
/// Returns the number of new relationships written.
// trace:v1 id=impl.scc.index.rpc-bridges work=WORK-ripwire-lessons-phase6 satisfies=REQ-cross-lang-semantic-bridges
pub fn link_rpc_bridges(store: &Store) -> Result<usize, scc_store::StoreError> {
    let contracts = store.entities_by_kind(kinds::CONTRACT)?;
    let mut rpcs: Vec<(String, String)> = Vec::new();
    for c in &contracts {
        if c.attributes.get("kind").and_then(|v| v.as_str()) != Some("rpc") {
            continue;
        }
        let Some(rpc) = c.attributes.get("rpc").and_then(|v| v.as_str()) else {
            continue;
        };
        rpcs.push((c.id.clone(), rpc.to_string()));
    }
    if rpcs.is_empty() {
        return Ok(0);
    }
    let symbols = store.entities_by_kind(kinds::SYMBOL)?;
    let mut written = 0usize;
    for sym in &symbols {
        let Some(path) = sym.attributes.get("file").and_then(|v| v.as_str()) else {
            continue;
        };
        if path.ends_with(".proto") {
            continue;
        }
        let Some(predicate) = bridge_role(path) else {
            continue;
        };
        let ident = last_ident(&sym.name);
        let key = normalize_ident(ident);
        if key.is_empty() {
            continue;
        }
        for (cid, rpc) in &rpcs {
            if normalize_ident(rpc) != key {
                continue;
            }
            let existing = store.relationships_between(&sym.id, predicate, cid)?;
            if !existing.is_empty() {
                continue;
            }
            let ev = Evidence {
                id: crate::write::evidence_id(path, "bridge", rpc, 0),
                r#type: scc_core::EvidenceType::Source,
                path: Some(path.to_string()),
                symbol: Some(sym.name.clone()),
                start_line: None,
                end_line: None,
                revision: None,
                content_hash: None,
                extractor: Some("scc-rpc-bridge".into()),
                extractor_version: Some("0.1.0".into()),
            };
            store.insert_evidence(&ev)?;
            let rel = Relationship::new(
                rel_id(&["bridge", predicate, &sym.id, cid]),
                sym.id.clone(),
                predicate,
                cid.clone(),
                Provenance::Extracted,
            )
            .with_confidence(0.8)
            .with_evidence(vec![ev.id.clone()]);
            store.insert_relationship(&rel, path)?;
            written += 1;
        }
    }
    Ok(written)
}

// trace:exempt reason=internal-detail
fn last_ident(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

// trace:exempt reason=internal-detail
fn normalize_ident(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

// trace:exempt reason=internal-detail
fn bridge_role(path: &str) -> Option<&'static str> {
    let p = path.replace('\\', "/").to_ascii_lowercase();
    let parts: Vec<&str> = p.split('/').collect();
    if parts.iter().any(|s| *s == "client" || *s == "web") {
        return Some(scc_core::predicates::INVOKES);
    }
    if parts
        .iter()
        .any(|s| *s == "server" || *s == "svc" || *s == "handler")
    {
        return Some(scc_core::predicates::IMPLEMENTS);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::Indexer;
    use scc_core::predicates;

    #[test]
    // trace:v1 id=test.scc.index.rpc-bridges verifies=REQ-cross-lang-semantic-bridges exercises=impl.scc.index.rpc-bridges
    fn rpc_bridge_is_invokes_implements_never_cross_lang_calls() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("server")).unwrap();
        std::fs::create_dir_all(root.join("client")).unwrap();
        std::fs::write(
            root.join("orders.proto"),
            r#"syntax = "proto3";
service Orders {
  rpc GetOrder (GetOrderRequest) returns (GetOrderResponse);
}
"#,
        )
        .unwrap();
        std::fs::write(
            root.join("server/orders.py"),
            "class OrdersServicer:\n    def GetOrder(self, request, context):\n        return {'id': '1'}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("client/orders.ts"),
            "export function getOrder(id: string) {\n  return id;\n}\n",
        )
        .unwrap();
        let tmp = tempfile::TempDir::new().unwrap();
        let store = scc_store::Store::open(&tmp.path().join("scc.db"), root).unwrap();
        let idx = Indexer::new(store, Config::default());
        idx.index().unwrap();

        let rels = idx.store.all_relationships().unwrap();
        let contracts = idx.store.entities_by_kind(kinds::CONTRACT).unwrap();
        assert!(
            contracts.iter().any(|c| c.name == "Orders.GetOrder"),
            "missing rpc contract: {contracts:?}"
        );
        let contract_id = contracts
            .iter()
            .find(|c| c.name == "Orders.GetOrder")
            .unwrap()
            .id
            .clone();

        assert!(
            rels.iter().any(|r| r.predicate == predicates::IMPLEMENTS
                && r.object == contract_id
                && r.provenance == Provenance::Extracted),
            "server must IMPLEMENT the contract: {rels:?}"
        );
        assert!(
            rels.iter().any(|r| r.predicate == predicates::INVOKES
                && r.object == contract_id
                && r.provenance == Provenance::Extracted),
            "client must INVOKES the contract: {rels:?}"
        );
        let cross_calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == predicates::CALLS)
            .filter(|r| {
                let subj = idx.store.get_entity(&r.subject).ok().flatten();
                let obj = idx.store.get_entity(&r.object).ok().flatten();
                match (subj, obj) {
                    (Some(s), Some(o)) => {
                        let sf = s
                            .attributes
                            .get("file")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let of = o
                            .attributes
                            .get("file")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        (sf.ends_with(".ts") && of.ends_with(".py"))
                            || (sf.ends_with(".py") && of.ends_with(".ts"))
                    }
                    _ => false,
                }
            })
            .collect();
        assert!(
            cross_calls.is_empty(),
            "must not emit CALLS across languages: {cross_calls:?}"
        );
    }

    #[test]
    // trace:exempt reason=internal-detail
    fn ambiguous_paths_are_not_guessed() {
        assert_eq!(bridge_role("lib/orders.py"), None);
        assert_eq!(bridge_role("client/orders.ts"), Some(predicates::INVOKES));
        assert_eq!(
            bridge_role("server/orders.py"),
            Some(predicates::IMPLEMENTS)
        );
        assert_eq!(normalize_ident("GetOrder"), normalize_ident("get_order"));
        assert_eq!(normalize_ident("getOrder"), "getorder");
        assert_ne!(normalize_ident("GetOrders"), normalize_ident("GetOrder"));
    }
}
