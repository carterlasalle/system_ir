//! First-class multi-repo systems (mission §XVI): semantic stitching across
//! member stores with strong keys and recorded match quality (§XVIII).
//!
//! A system never concatenates graphs. Each member keeps its own store and
//! its own canonical entity ids (stable repository identity, §XIV-XV); the
//! system only adds cross-repo [`Stitch`] edges bound through contracts
//! (HTTP verb + normalized route), topics, and public-export identity —
//! never ordinary static `CALLS`, never bare string similarity.
//!
//! Match quality is explicit on every stitch: `Exact` (strong key, all
//! sides evidenced), `Declared` (importer names a member repo AND the
//! imported symbol), `Inferred` (symbol match, repo binding unresolved),
//! `Ambiguous` (key claimed incompatibly — recorded, never joined).

use crate::{Entity, Store, StoreError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One member repository: its checkout root and its own index database.
/// Entity ids inside are the member's canonical ids (§XV).
#[derive(Debug)]
// trace:v1 id=impl.scc.store.system.member work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct Member {
    /// Canonical member identity (stable repo id, §XIV).
    pub repo_id: String,
    /// Checkout root (provenance path, not identity).
    pub root: PathBuf,
    /// Member's own store. Opened normally; never rewritten by stitching.
    pub store: Store,
}

/// A stitched system: members plus the cross-repo edges between them.
#[derive(Debug)]
// trace:v1 id=impl.scc.store.system work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct System {
    /// Member repositories in open order (deterministic: caller order).
    pub members: Vec<Member>,
}

/// How a stitch was evidenced. Never stronger than the keys behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
// trace:v1 id=impl.crates-scc-store-src-system.match-kind work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub enum MatchKind {
    /// Strong key, every side evidenced (verb+route, shared topic name).
    Exact,
    /// Importer names the member repo AND the imported symbol.
    Declared,
    /// Symbol match; which repo provides the module is unresolved.
    Inferred,
    /// Same key claimed incompatibly — recorded, never joined.
    Ambiguous,
}

/// What kind of cross-repo binding a stitch represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
// trace:v1 id=impl.crates-scc-store-src-system.stitch-kind work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub enum StitchKind {
    /// Same HTTP verb + normalized route in ≥2 members.
    Route,
    /// Same topic name produced/consumed in ≥2 members.
    Topic,
    /// One member imports a symbol another member exports.
    PackageExport,
}

/// One end of a stitch: the member entity plus where it was observed.
#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:v1 id=impl.crates-scc-store-src-system.stitch-end work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct StitchEnd {
    /// Member repo id (semantic scope of the entity).
    pub repo_id: String,
    /// Canonical entity id inside the member (§XV: unchanged by stitching).
    pub entity_id: String,
    /// Files observing this entity (provenance survives stitching).
    pub sources: Vec<String>,
}

/// A cross-repo semantic edge. Ambiguous stitches carry all claimants and
/// must not be treated as a join.
#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:v1 id=impl.crates-scc-store-src-system.stitch work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct Stitch {
    /// Binding family.
    pub kind: StitchKind,
    /// Strong key: route `VERB /path`, topic name, or `module::symbol`.
    pub key: String,
    /// Evidenced quality (§XVIII).
    pub match_kind: MatchKind,
    /// One end per claiming member, ordered by (repo_id, entity_id).
    pub ends: Vec<StitchEnd>,
}

// trace:exempt reason=internal-detail
impl System {
    /// Open a system over member checkout roots. Each member database is
    /// `<root>/.scc/scc.db` (the default state dir); members must already
    /// be indexed. Stitching is read-only over member graphs.
    // trace:v1 id=impl.crates-scc-store-src-system-system.open work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    pub fn open(roots: &[&Path]) -> Result<System, StoreError> {
        let mut members = Vec::with_capacity(roots.len());
        for root in roots {
            let db = root.join(".scc").join("scc.db");
            // Read-only command surface: never conjure an empty database for
            // an unindexed member (that would silently omit it from stitches).
            if !db.is_file() {
                return Err(StoreError::Corrupt(format!(
                    "system member not indexed (no {}): run `scc index` there first",
                    db.display()
                )));
            }
            let store = Store::open(&db, root)?;
            members.push(Member {
                repo_id: store.repo_id.clone(),
                root: store.root.clone(),
                store,
            });
        }
        Ok(System { members })
    }

    /// Canonical entity ids of one member (standalone-vs-system stability
    /// reads these through [`System`] without rewriting them).
    // trace:v1 id=impl.crates-scc-store-src-system-system.member-entity-ids work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    pub fn member_entity_ids(&self, repo_id: &str) -> Result<Vec<String>, StoreError> {
        let m = self
            .members
            .iter()
            .find(|m| m.repo_id == repo_id)
            .ok_or_else(|| StoreError::Corrupt(format!("unknown member {repo_id}")))?;
        Ok(m
            .store
            .all_entities()?
            .into_iter()
            .map(|e| e.id)
            .collect())
    }

    // trace:v1 id=impl.crates-scc-store-src-system-system.end work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    fn end(&self, m: &Member, e: &Entity) -> Result<StitchEnd, StoreError> {
        Ok(StitchEnd {
            repo_id: m.repo_id.clone(),
            entity_id: e.id.clone(),
            sources: m.store.entity_sources(&e.id)?,
        })
    }

    /// Stitch HTTP contracts: same `VERB /path` route name in ≥2 members
    /// is an exact cross-repo contract (strong key: verb + normalized
    /// route). Same path with different verbs across members is ambiguous:
    /// recorded with all claimants, never joined.
    // trace:v1 id=impl.scc.store.system.stitch-routes work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    pub fn stitch_routes(&self) -> Result<Vec<Stitch>, StoreError> {
        // path -> [(member_idx, verb, entity)]
        let mut by_path: BTreeMap<String, Vec<(usize, String, Entity)>> = BTreeMap::new();
        for (i, m) in self.members.iter().enumerate() {
            for e in m.store.entities_by_kind(scc_core::kinds::ROUTE)? {
                if let Some((verb, path)) = e.name.split_once(' ') {
                    by_path
                        .entry(path.to_string())
                        .or_default()
                        .push((i, verb.to_string(), e));
                }
            }
        }
        let mut out = Vec::new();
        for (path, claims) in &by_path {
            let repos: std::collections::BTreeSet<&str> = claims
                .iter()
                .map(|(i, _, _)| self.members[*i].repo_id.as_str())
                .collect();
            if repos.len() < 2 {
                continue; // single-repo path: internal, not a stitch
            }
            let mut verbs: Vec<&str> = claims.iter().map(|(_, v, _)| v.as_str()).collect();
            verbs.sort();
            verbs.dedup();
            let mut stitch_ends = Vec::new();
            for (i, _, e) in claims {
                stitch_ends.push(self.end(&self.members[*i], e)?);
            }
            stitch_ends.sort_by(|a, b| {
                (a.repo_id.clone(), a.entity_id.clone())
                    .cmp(&(b.repo_id.clone(), b.entity_id.clone()))
            });
            stitch_ends.dedup_by(|a, b| a.entity_id == b.entity_id);
            if verbs.len() == 1 {
                out.push(Stitch {
                    kind: StitchKind::Route,
                    key: format!("{} {path}", verbs[0]),
                    match_kind: MatchKind::Exact,
                    ends: stitch_ends,
                });
            } else {
                // same path, different verbs across members: ambiguous —
                // recorded with all claimants, never joined.
                out.push(Stitch {
                    kind: StitchKind::Route,
                    key: path.clone(),
                    match_kind: MatchKind::Ambiguous,
                    ends: stitch_ends,
                });
            }
        }
        out.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(out)
    }

    /// Stitch event topics: same topic name evidenced in ≥2 members is an
    /// exact semantic stitch (brokers namespace globally). Publish and
    /// subscribe sides are distinguished by each end's relationships, not
    /// by the stitch itself.
    // trace:v1 id=impl.scc.store.system.stitch-topics work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    pub fn stitch_topics(&self) -> Result<Vec<Stitch>, StoreError> {
        let mut by_name: BTreeMap<String, Vec<(usize, Entity)>> = BTreeMap::new();
        for (i, m) in self.members.iter().enumerate() {
            for e in m.store.entities_by_kind(scc_core::kinds::TOPIC)? {
                by_name.entry(e.name.clone()).or_default().push((i, e));
            }
        }
        let mut out = Vec::new();
        for (name, ends) in &by_name {
            let mut stitch_ends = Vec::new();
            for (i, e) in ends {
                stitch_ends.push(self.end(&self.members[*i], e)?);
            }
            let repos: std::collections::BTreeSet<&str> =
                stitch_ends.iter().map(|e| e.repo_id.as_str()).collect();
            if repos.len() < 2 {
                continue;
            }
            stitch_ends.sort_by(|a, b| {
                (a.repo_id.clone(), a.entity_id.clone())
                    .cmp(&(b.repo_id.clone(), b.entity_id.clone()))
            });
            out.push(Stitch {
                kind: StitchKind::Topic,
                key: name.clone(),
                match_kind: MatchKind::Exact,
                ends: stitch_ends,
            });
        }
        out.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(out)
    }

    /// Stitch public API: member A imports module M with symbol names N
    /// while member B exports n ∈ N. When M names B's repo (normalized),
    /// the repo binding is declared; otherwise the symbol match is
    /// inferred and the module→repo binding stays unresolved. A symbol
    /// exported by ≥2 members is ambiguous.
    // trace:v1 id=impl.scc.store.system.stitch-package-exports work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    pub fn stitch_package_exports(&self) -> Result<Vec<Stitch>, StoreError> {
        let mut exports: BTreeMap<String, Vec<(usize, Entity)>> = BTreeMap::new();
        for (i, m) in self.members.iter().enumerate() {
            for e in m.store.entities_by_kind(scc_core::kinds::EXPORT)? {
                exports
                    .entry(norm_export(&e.name))
                    .or_default()
                    .push((i, e));
            }
        }
        let mut imports: Vec<(usize, String, Vec<String>)> = Vec::new();
        for (i, m) in self.members.iter().enumerate() {
            for (_file, module, names, _line, _typ) in m.store.all_imports()? {
                let syms: Vec<String> = names.into_iter().map(|(_, alias)| norm_export(&alias)).collect();
                if !syms.is_empty() {
                    imports.push((i, module, syms));
                }
            }
        }
        let mut out = Vec::new();
        for (ai, module, names) in &imports {
            let am = &self.members[*ai];
            for wanted in names {
                let Some(providers) = exports.get(wanted) else {
                    continue;
                };
                let others: Vec<(usize, Entity)> = providers
                    .iter()
                    .filter(|(bi, _)| *bi != *ai)
                    .cloned()
                    .collect();
                if others.is_empty() {
                    continue;
                }
                let declared_here: Vec<(usize, Entity)> = others
                    .iter()
                    .filter(|(bi, _)| {
                        norm_export(&self.members[*bi].repo_id) == norm_export(module)
                    })
                    .cloned()
                    .collect();
                let (match_kind, ends_src): (MatchKind, Vec<(usize, Entity)>) =
                    if others.len() > 1 && declared_here.is_empty() {
                        (MatchKind::Ambiguous, others)
                    } else if !declared_here.is_empty() {
                        (MatchKind::Declared, declared_here)
                    } else {
                        (MatchKind::Inferred, others)
                    };
                // importer end: the importing file. The external_api edge
                // target is not always materialized as an entity row, so
                // provenance comes from the imports table itself.
                let mut stitch_ends = Vec::new();
                if let Ok(rows) = am.store.all_imports() {
                    for (file, mod_name, _, _, _) in &rows {
                        if mod_name == module {
                            stitch_ends.push(StitchEnd {
                                repo_id: am.repo_id.clone(),
                                entity_id: format!(
                                    "repo://{}/external_api/{}",
                                    am.repo_id, module
                                ),
                                sources: vec![file.clone()],
                            });
                            break;
                        }
                    }
                }
                for (bi, e) in &ends_src {
                    stitch_ends.push(self.end(&self.members[*bi], e)?);
                }
                stitch_ends.sort_by(|a, b| {
                    (a.repo_id.clone(), a.entity_id.clone())
                        .cmp(&(b.repo_id.clone(), b.entity_id.clone()))
                });
                out.push(Stitch {
                    kind: StitchKind::PackageExport,
                    key: format!("{module}::{wanted}"),
                    match_kind,
                    ends: stitch_ends,
                });
            }
        }
        out.sort_by(|a, b| a.key.cmp(&b.key));
        out.dedup_by(|a, b| a.key == b.key && a.match_kind == b.match_kind);
        Ok(out)
    }
}

/// Normalize an export/symbol/module name for cross-repo comparison:
/// lowercase, `-`/`_` unified.
// trace:exempt reason=internal-detail
fn norm_export(s: &str) -> String {
    s.to_ascii_lowercase().replace('_', "-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use scc_core::Entity;
    use tempfile::TempDir;

    // trace:exempt reason=internal-detail
    fn member_with(
        dir: &TempDir,
        name: &str,
        routes: &[&str],
        topics: &[&str],
        exports: &[&str],
        imports: &[(String, Vec<(String, String)>)],
    ) -> PathBuf {
        let root = dir.path().join(name);
        std::fs::create_dir_all(&root).unwrap();
        let db = root.join(".scc").join("scc.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let store = Store::open(&db, &root).unwrap();
        for r in routes {
            store
                .insert_entity(
                    &Entity::new(format!("repo://{}/route/{}", store.repo_id, r.replace(' ', "-")), "route", r.to_string()),
                    &[format!("{name}/app.py")],
                )
                .unwrap();
        }
        for t in topics {
            store
                .insert_entity(
                    &Entity::new(format!("repo://{}/topic/{t}", store.repo_id), "topic", t.to_string()),
                    &[format!("{name}/bus.py")],
                )
                .unwrap();
        }
        for e in exports {
            store
                .insert_entity(
                    &Entity::new(format!("repo://{}/export/{e}", store.repo_id), "export", e.to_string()),
                    &[format!("{name}/lib.py")],
                )
                .unwrap();
        }
        for (module, names) in imports {
            store
                .insert_imports(&format!("{name}/app.py"), &[(module.clone(), names.clone(), 1, "member".into())])
                .unwrap();
            // unresolved modules surface as external_api, like the extractor
            let ext = Entity::new(
                format!("repo://{}/external_api/{}", store.repo_id, module),
                "external_api",
                module.clone(),
            );
            let _ = store.insert_entity(&ext, &[format!("{name}/app.py")]);
        }
        root
    }

    #[test]
    // trace:v1 id=test.scc.store.system-route-exact verifies=REQ-SI-503JSBGP exercises=impl.scc.store.system.stitch-routes
    fn shared_verb_and_path_is_an_exact_contract() {
        let dir = TempDir::new().unwrap();
        let a = member_with(&dir, "svc-a", &["GET /health"], &[], &[], &[]);
        let b = member_with(&dir, "svc-b", &["GET /health", "POST /users"], &[], &[], &[]);
        let sys = System::open(&[a.as_path(), b.as_path()]).unwrap();
        let routes = sys.stitch_routes().unwrap();
        assert_eq!(routes.len(), 1, "{routes:?}");
        assert_eq!(routes[0].key, "GET /health");
        assert_eq!(routes[0].match_kind, MatchKind::Exact);
        assert_eq!(routes[0].ends.len(), 2);
        // canonical ids untouched by stitching
        for e in &routes[0].ends {
            assert!(e.entity_id.starts_with("repo://"), "{}", e.entity_id);
            assert!(!e.sources.is_empty());
        }
    }

    #[test]
    // trace:v1 id=test.scc.store.system-route-ambiguous verifies=REQ-SI-503JSBGP exercises=impl.scc.store.system.stitch-routes
    fn same_path_different_verbs_stays_ambiguous() {
        let dir = TempDir::new().unwrap();
        let a = member_with(&dir, "svc-a", &["GET /health"], &[], &[], &[]);
        let b = member_with(&dir, "svc-b", &["POST /health"], &[], &[], &[]);
        let sys = System::open(&[a.as_path(), b.as_path()]).unwrap();
        let routes = sys.stitch_routes().unwrap();
        assert_eq!(routes.len(), 1, "{routes:?}");
        assert_eq!(routes[0].key, "/health");
        assert_eq!(routes[0].match_kind, MatchKind::Ambiguous);
        assert_eq!(routes[0].ends.len(), 2);
    }

    #[test]
    // trace:v1 id=test.scc.store.system-unindexed-refused verifies=REQ-SI-503JSBGP exercises=impl.crates-scc-store-src-system-system.open
    fn unindexed_member_is_refused_not_conjured() {
        let dir = TempDir::new().unwrap();
        let bare = dir.path().join("bare");
        std::fs::create_dir_all(&bare).unwrap();
        let err = System::open(&[bare.as_path()]).unwrap_err();
        assert!(err.to_string().contains("not indexed"), "{err}");
        assert!(!bare.join(".scc").exists(), "no side-effect database");
    }

    #[test]
    // trace:v1 id=test.scc.store.system-topic-exact verifies=REQ-SI-503JSBGP exercises=impl.scc.store.system.stitch-topics
    fn shared_topic_is_an_exact_stitch() {
        let dir = TempDir::new().unwrap();
        let a = member_with(&dir, "prod", &[], &["orders.created"], &[], &[]);
        let b = member_with(&dir, "cons", &[], &["orders.created"], &[], &[]);
        let sys = System::open(&[a.as_path(), b.as_path()]).unwrap();
        let topics = sys.stitch_topics().unwrap();
        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0].match_kind, MatchKind::Exact);
        assert_eq!(topics[0].ends.len(), 2);
    }

    #[test]
    // trace:v1 id=test.scc.store.system-package-declared verifies=REQ-SI-503JSBGP exercises=impl.scc.store.system.stitch-package-exports
    fn named_repo_plus_symbol_is_declared() {
        let dir = TempDir::new().unwrap();
        let hub = member_with(&dir, "stitch_hub", &[], &[], &["normalize_email"], &[]);
        let spoke = member_with(
            &dir,
            "spoke",
            &[],
            &[],
            &[],
            &[("stitch_hub".into(), vec![("normalize_email".into(), "normalize_email".into())])],
        );
        let sys = System::open(&[hub.as_path(), spoke.as_path()]).unwrap();
        let stitches = sys.stitch_package_exports().unwrap();
        assert_eq!(stitches.len(), 1, "{stitches:?}");
        assert_eq!(stitches[0].match_kind, MatchKind::Declared);
    }

    #[test]
    // trace:v1 id=test.scc.store.system-package-inferred verifies=REQ-SI-503JSBGP exercises=impl.scc.store.system.stitch-package-exports
    fn symbol_only_match_is_inferred_not_joined() {
        let dir = TempDir::new().unwrap();
        let hub = member_with(&dir, "other-lib", &[], &[], &["normalize_email"], &[]);
        let spoke = member_with(
            &dir,
            "spoke",
            &[],
            &[],
            &[],
            &[("stitch_hub".into(), vec![("normalize_email".into(), "normalize_email".into())])],
        );
        let sys = System::open(&[hub.as_path(), spoke.as_path()]).unwrap();
        let stitches = sys.stitch_package_exports().unwrap();
        assert_eq!(stitches.len(), 1, "{stitches:?}");
        assert_eq!(stitches[0].match_kind, MatchKind::Inferred);
    }
}
