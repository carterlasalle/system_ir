//! Startup context (Wave 14C): the deterministic Atlas + Surface fusion
//! handed to agents at session start, plus the task delta (Wave 14E) that
//! renders only *new* relevant APIs against the context ledger.
//!
//! Prompt-cache stability: the artifact hash is a pure function of
//! `(epoch, renderer_version, trust_policy, budget)` — no timestamps — so
//! the same epoch + config always yields byte-identical startup text.

use crate::surface::{build_surface, build_surface_cached, SurfaceMode, SurfacePolicy, SurfaceRequest};
use crate::ContextCompiler;
use scc_core::kinds;
use scc_core::{ContextArtifact, ContextBudget, ContextLedger};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Renderer version: part of the artifact hash. Bump when the startup
/// renderer's output format changes (invalidates prompt-cache keys).
pub const RENDERER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The startup artifact: atlas + surface + coverage + omissions, with the
/// deterministic artifact hash. `surface_render` is the SAME render the
/// artifact printed — ledger recording derives visible ids from it, so
/// the surface is never computed twice per startup.
// trace:exempt reason=internal-detail
pub struct StartupContext {
    pub atlas: String,
    pub surface: String,
    pub surface_render: scc_core::SurfaceRenderResult,
    pub coverage: Vec<String>,
    pub omissions: Vec<String>,
    pub artifact: ContextArtifact,
}

/// Global-rank cache (Wave 15.2, per-ModelEpoch rank caching): the
/// expensive, epoch-stable parts of a global Surface build —
/// `SystemRanker::new` (heterogeneous node graph + adjacency + rarity),
/// the 50-iteration global PageRank vector, and the projection to symbol
/// scores — serialized to the store cache so consecutive startups in the
/// same model epoch skip the rank build entirely. Key:
/// `rank:global:<blake3(epoch, policy, salt)[..20]>` (mirrors the
/// `system_atlas` pack-cache pattern in `ContextCompiler::system_atlas`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct GlobalRankCache {
    /// The composite cache epoch the entry was computed under.
    pub epoch: String,
    /// The TrustPolicy fingerprint (`trust_policy_str`) the rank used.
    pub policy: String,
    /// The active rank salt (`ContextSettings::rank_salt`).
    pub salt: String,
    /// The heterogeneous global PageRank vector (index i == `nodes()[i]`).
    /// Retained so a future task-PPR path can warm-start from it.
    pub global_vector: Vec<f64>,
    /// Symbol id -> projected global score (`project_to_symbols` output) —
    /// exactly what the surface pipeline consumes as `global_of`.
    pub node_symbol_map: BTreeMap<String, f64>,
    /// The epoch the candidate entry list came from (epoch-stability
    /// marker; the cache key already pins the epoch).
    pub candidates_epoch: String,
    /// Epoch-stable candidate entry ids (cheap accounting — avoids a
    /// `compile_surface_map` walk for the surface accounting line).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_ids: Vec<String>,
    /// How many times this entry has been reused (deterministic
    /// cache-hit marker for tests).
    #[serde(default)]
    pub hits: u64,
}

/// Build the deterministic startup artifact: the existing System Atlas
/// content (capped at `budget.atlas`) fused with the budget-selected
/// System Surface Map (the production `select_and_render_global` pipeline),
/// coverage warnings, and honest omissions. The surface's omitted ids
/// populate the OMISSIONS section; the ledger records ONLY rendered ids.
// trace:v1 id=impl.scc.context.startup work=WORK-SCC-014 satisfies=REQ-SCC-IR
pub fn build_startup(
    compiler: &ContextCompiler,
    budget: &ContextBudget,
    renderer_version: &str,
) -> StartupContext {
    let epoch = compiler
        .store
        .cache_epoch()
        .unwrap_or_else(|_| "no-epoch".into());

    // Atlas: reuse the existing cached-atlas pipeline (system_atlas is
    // cache-keyed by epoch + stale set; a stale atlas is never served).
    let atlas_pack = compiler.system_atlas(Some(budget.atlas));
    let atlas = atlas_pack.content.clone();

    // Surface: the FULL production pipeline — the one authoritative
    // [`build_surface`] service in Global mode (heterogeneous global PPR,
    // required coverage, MMR diversity, token-aware quotas, soft/hard
    // budget selection, render) — built EXACTLY ONCE per startup. The
    // per-ModelEpoch global-rank cache supplies the PPR vector + symbol
    // projection on hit (skipping SystemRanker::new + the 50 power
    // iterations); on miss the render fills the cache, persisted below.
    let mut rank_cache = load_global_rank_cache(compiler);
    let cache_hit = rank_cache.is_some();
    let render = build_surface_cached(
        compiler,
        SurfaceRequest {
            mode: SurfaceMode::Global,
            budget: budget.surface,
            explain: false,
            policy: SurfacePolicy::defaults(budget.surface),
            semantic: None,
        },
        &mut rank_cache,
    );
    // Best-effort persistence: a cache failure never fails startup. The
    // hit counter is the deterministic "reused on run 2" marker.
    if let Some(c) = &mut rank_cache {
        if cache_hit {
            c.hits += 1;
        }
        store_global_rank_cache(compiler, c);
    }
    let surface = render.text.clone();

    // MODEL COVERAGE: the compiler's existing warnings + stale paths +
    // a surface accounting line. Candidate count comes from the render
    // itself (rendered ∪ omitted == every candidate) — never a second
    // compile_surface_map walk.
    let mut coverage = Vec::new();
    for w in compiler_warnings(compiler) {
        coverage.push(w);
    }
    let mut stale: Vec<String> = compiler.stale_paths.clone();
    stale.sort();
    stale.dedup();
    for p in &stale {
        coverage.push(format!("stale: {p}"));
    }
    let candidates = render.rendered_ids.len() + render.omitted_ids.len();
    coverage.push(format!(
        "surface map: {} of {} entries rendered, {} tokens (budget {})",
        render.rendered_ids.len(),
        candidates,
        render.token_count,
        budget.surface
    ));

    // OMISSIONS: the render result's per-kind cuts + omitted-id count + the
    // atlas's dropped sections (honest: omitted ids are never silent).
    let mut omissions = Vec::new();
    for o in &render.omissions {
        omissions.push(format!("surface: {} ({})", o.kind, o.reason));
    }
    if !render.omitted_ids.is_empty() {
        omissions.push(format!(
            "surface: {} lower-ranked definitions omitted",
            render.omitted_ids.len()
        ));
    }
    for d in &atlas_pack.dropped_sections {
        omissions.push(format!("atlas section dropped: {d}"));
    }
    if atlas_pack.hard_truncated {
        omissions.push("atlas hard-truncated mid-section".into());
    }
    if atlas_pack.exceeded_soft_budget {
        omissions.push("atlas exceeded soft budget: kept complete, over budget".into());
    }
    if omissions.is_empty() {
        omissions.push("none".into());
    }

    let trust_policy = trust_policy_str(compiler.view.policy());

    // Deterministic artifact hash: blake3 over (epoch + renderer_version +
    // trust_policy + budget fields). The preimage never contains
    // timestamps or volatile state, so the hash is stable per epoch.
    let mut h = blake3::Hasher::new();
    h.update(b"startup-artifact-v1");
    h.update(epoch.as_bytes());
    h.update(renderer_version.as_bytes());
    h.update(trust_policy.as_bytes());
    h.update(budget.total.to_string().as_bytes());
    h.update(budget.atlas.to_string().as_bytes());
    h.update(budget.surface.to_string().as_bytes());
    h.update(budget.task_delta.to_string().as_bytes());
    h.update(budget.structural_source.to_string().as_bytes());
    let sha256 = h.finalize().to_hex().to_string();

    // CONTENT hash: the same config preimage PLUS the actual rendered text
    // (atlas + surface + coverage + omissions, without the artifact metadata
    // comment). A content change that keeps the config identical now
    // changes the hash — the audit's name/content mismatch fix.
    let body = assemble_body(&atlas, &surface, &coverage, &omissions);
    let mut ch = blake3::Hasher::new();
    ch.update(b"startup-content-v1");
    ch.update(epoch.as_bytes());
    ch.update(renderer_version.as_bytes());
    ch.update(trust_policy.as_bytes());
    ch.update(budget.total.to_string().as_bytes());
    ch.update(budget.atlas.to_string().as_bytes());
    ch.update(budget.surface.to_string().as_bytes());
    ch.update(budget.task_delta.to_string().as_bytes());
    ch.update(budget.structural_source.to_string().as_bytes());
    ch.update(body.as_bytes());
    let content_hash = ch.finalize().to_hex().to_string();

    let mut artifact = ContextArtifact {
        kind: "startup".into(),
        epoch,
        renderer_version: renderer_version.to_string(),
        trust_policy,
        budget: budget.clone(),
        sha256,
        content_hash,
        text: String::new(),
    };
    artifact.text = assemble_block(&atlas, &surface, &coverage, &omissions, &artifact);

    StartupContext {
        atlas,
        surface,
        surface_render: render,
        coverage,
        omissions,
        artifact,
    }
}

/// The store-cache key for the global rank cache: `rank:global:` +
/// blake3 over the composite cache epoch, the TrustPolicy fingerprint,
/// and the rank salt (truncated to 20 hex chars, mirroring the
/// `system_atlas` pack-cache key pattern). A changed epoch, policy, or
/// salt yields a different key — a stale entry is never served.
// trace:exempt reason=internal-detail
fn global_rank_key(epoch: &str, policy: &str, salt: &str) -> String {
    let mut h = blake3::Hasher::new();
    h.update(b"rank:global:v1");
    h.update(epoch.as_bytes());
    h.update(b"\0");
    h.update(policy.as_bytes());
    h.update(b"\0");
    h.update(salt.as_bytes());
    format!("rank:global:{}", &h.finalize().to_hex()[..20])
}

/// Load the per-ModelEpoch global rank cache from the store cache
/// (key `rank:global:<hash>` over epoch + policy + salt). `None` on any
/// miss/error — never panics, never fabricates. The entry is validated
/// against the current epoch/policy/salt (belt-and-suspenders: the key
/// already pins them).
// trace:v1 id=impl.scc.startup.rank-cache-load work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching satisfies=REQ-global-rank-cached-per-model-epoch
pub fn load_global_rank_cache(compiler: &ContextCompiler) -> Option<GlobalRankCache> {
    let epoch = compiler.store.cache_epoch().ok()?;
    let policy = trust_policy_str(compiler.view.policy());
    let salt = &compiler.settings.rank_salt;
    let key = global_rank_key(&epoch, &policy, salt);
    let cached = compiler.store.cache_get(&key, &epoch).ok().flatten()?;
    let c: GlobalRankCache = serde_json::from_str(&cached).ok()?;
    if c.epoch != epoch || c.policy != policy || c.salt != *salt {
        return None;
    }
    Some(c)
}

/// Persist the per-ModelEpoch global rank cache (best-effort: cache
/// failures never fail the caller).
// trace:v1 id=impl.scc.startup.rank-cache-store work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching satisfies=REQ-global-rank-cached-per-model-epoch
pub fn store_global_rank_cache(compiler: &ContextCompiler, cache: &GlobalRankCache) {
    let epoch = compiler
        .store
        .cache_epoch()
        .unwrap_or_else(|_| "no-epoch".into());
    let policy = trust_policy_str(compiler.view.policy());
    let salt = &compiler.settings.rank_salt;
    let key = global_rank_key(&epoch, &policy, salt);
    if let Ok(json) = serde_json::to_string(cache) {
        let _ = compiler.store.cache_put(&key, &json, &epoch);
    }
}

/// The logical symbol entity id of a rendered entry id: overload-sensitive
/// entry ids carry a `#overload{N}` suffix (`{symbol}#overload{N}`) that
/// is stripped to recover the symbol id. Non-overload entry ids ARE the
/// symbol entity id.
// trace:exempt reason=internal-detail
fn symbol_id_of(entry_id: &str) -> String {
    entry_id
        .rsplit_once("#overload")
        .map(|(logical, _)| logical)
        .unwrap_or(entry_id)
        .to_string()
}

/// The spec's startup block format. Pure function of the context struct, so
/// `build_startup(..).artifact.text == render_startup(&startup)` always.
// trace:exempt reason=internal-detail
pub fn render_startup(s: &StartupContext) -> String {
    assemble_block(&s.atlas, &s.surface, &s.coverage, &s.omissions, &s.artifact)
}

/// The startup body (all content sections, no artifact metadata comment) —
/// the preimage of `content_hash`.
// trace:exempt reason=internal-detail
fn assemble_body(atlas: &str, surface: &str, coverage: &[String], omissions: &[String]) -> String {
    let mut out = String::new();
    out.push_str("# SCC SYSTEM CONTEXT\n");
    out.push_str("## SYSTEM ATLAS\n");
    out.push_str(atlas.trim_end());
    out.push_str("\n\n## SYSTEM SURFACE MAP\n");
    out.push_str(surface.trim_end());
    out.push_str("\n\n## MODEL COVERAGE\n");
    if coverage.is_empty() {
        out.push_str("(no warnings)\n");
    } else {
        for c in coverage {
            out.push_str(c);
            out.push('\n');
        }
    }
    out.push_str("\n## OMISSIONS\n");
    for o in omissions {
        out.push_str(o);
        out.push('\n');
    }
    out
}

// trace:exempt reason=internal-detail
fn assemble_block(
    atlas: &str,
    surface: &str,
    coverage: &[String],
    omissions: &[String],
    artifact: &ContextArtifact,
) -> String {
    let mut out = String::new();
    out.push_str("# SCC SYSTEM CONTEXT\n");
    out.push_str(&format!(
        "<!-- artifact sha256:{} content_hash:{} epoch:{} renderer:{} -->\n\n",
        artifact.sha256, artifact.content_hash, artifact.epoch, artifact.renderer_version
    ));
    out.push_str(&assemble_body(atlas, surface, coverage, omissions));
    out
}

/// Coverage warnings mirroring the compiler's pack warnings (deterministic):
/// not-indexed, stale count, high/critical drift.
// trace:exempt reason=internal-detail
fn compiler_warnings(compiler: &ContextCompiler) -> Vec<String> {
    let mut w = Vec::new();
    if compiler.store.snapshot_status().ok().flatten().is_none() {
        w.push("Repository is not indexed — run `scc index`.".into());
    }
    if !compiler.stale_paths.is_empty() {
        w.push(format!(
            "Model is stale: {} changed file(s) not yet re-indexed.",
            compiler.stale_paths.len()
        ));
    }
    if let Ok(findings) = compiler.store.drift_findings(true) {
        for (_, kind, sev, msg, _) in findings {
            if sev == "high" || sev == "critical" {
                w.push(format!("Drift [{kind}]: {msg}"));
            }
        }
    }
    w.truncate(6);
    w
}

// trace:exempt reason=internal-detail
pub(crate) fn trust_policy_str(p: &scc_graph::TrustPolicy) -> String {
    format!(
        "extracted={} resolved={} observed={} declared={} inferred={} floor={}",
        p.allow_extracted,
        p.allow_resolved,
        p.allow_observed,
        p.allow_declared,
        p.allow_inferred,
        p.min_inferred_confidence
    )
}

/// The kind-scoped id sets shown by the startup artifact (for ledger
/// recording): `(symbols, files, components, flows)`. The surface side
/// records ONLY the render result's `rendered_ids` — budget-omitted
/// candidates are never marked visible (audit fix: the ledger must
/// describe what the agent actually saw). Consumes the ALREADY-PRODUCED
/// render (`startup.surface_render`) — the surface is built exactly once
/// per startup; this function never rebuilds it.
// trace:exempt reason=internal-detail
pub fn visible_ids_from_startup(
    compiler: &ContextCompiler,
    startup: &StartupContext,
) -> (BTreeSet<String>, BTreeSet<String>, BTreeSet<String>, BTreeSet<String>) {
    let mut symbols = BTreeSet::new();
    let mut files = BTreeSet::new();
    let mut components = BTreeSet::new();
    let mut flows = BTreeSet::new();

    // Cache-hit in the CLI flow (build_startup already ran system_atlas).
    let atlas_pack = compiler.system_atlas(Some(startup.artifact.budget.atlas));

    // Surface: rendered entries ONLY — the SAME render the artifact
    // printed, so the ledger exactly matches the artifact text. Entry
    // metadata (symbol id, file path, component) is derived from the
    // trusted view per rendered id (cheap: rendered ids only, never the
    // candidate pool) — no compile_surface_map walk.
    let view = &compiler.view;
    let rendered_symbols: BTreeSet<String> = startup
        .surface_render
        .rendered_ids
        .iter()
        .map(|id| symbol_id_of(id))
        .collect();
    // Component attribution mirrors compile_surface_map's containment
    // walk (component CONTAINS file CONTAINS symbol; first component in
    // name order wins) but only for the rendered symbol ids.
    let mut comp_of: BTreeMap<String, String> = BTreeMap::new();
    for c in view.components() {
        for r in sorted_rels(view.out_pred(&c.id, scc_core::predicates::CONTAINS)) {
            for sr in sorted_rels(view.out_pred(&r.object, scc_core::predicates::CONTAINS)) {
                if rendered_symbols.contains(&sr.object) {
                    comp_of
                        .entry(sr.object.clone())
                        .or_insert_with(|| c.name.clone());
                }
            }
        }
    }
    for id in &startup.surface_render.rendered_ids {
        let symbol_id = symbol_id_of(id);
        symbols.insert(symbol_id.clone());
        // The entry id is the entity id for non-overload entries; overload
        // entries (n >= 1) resolve through the logical symbol id.
        let ent = view.entity(id).or_else(|| view.entity(&symbol_id));
        if let Some(e) = ent {
            if let Some(f) = e.attributes.get("file").and_then(|v| v.as_str()) {
                files.insert(f.to_string());
            }
        }
        if let Some(c) = comp_of.get(&symbol_id) {
            components.insert(c.clone());
        }
    }
    // Classify the atlas pack's entity ids by kind (deterministic).
    for id in &atlas_pack.entity_ids {
        if let Some(e) = compiler.view.entity(id) {
            match e.kind.as_str() {
                kinds::SYMBOL => {
                    symbols.insert(id.clone());
                }
                kinds::FILE => {
                    files.insert(e.name.clone());
                }
                kinds::COMPONENT => {
                    components.insert(id.clone());
                }
                kinds::FLOW => {
                    flows.insert(id.clone());
                }
                _ => {}
            }
        }
    }
    (symbols, files, components, flows)
}

/// Deterministic relationship sort (id, subject, object) — mirrors the
/// surface compiler's traversal order so attribution walks are stable.
// trace:exempt reason=internal-detail
fn sorted_rels(rels: Vec<&scc_core::Relationship>) -> Vec<&scc_core::Relationship> {
    let mut v = rels;
    v.sort_by(|a, b| {
        a.id.cmp(&b.id)
            .then_with(|| a.subject.cmp(&b.subject))
            .then_with(|| a.object.cmp(&b.object))
    });
    v
}

/// The task delta: the task-personalized surface over the context ledger —
/// only *new* relevant APIs (entries already visible AND unchanged this
/// epoch are not re-injected), budget-capped. Routed through the one
/// authoritative [`build_surface`] service in Task mode; never re-dumps
/// the Atlas. The custom task/PPR selection implementation was deleted —
/// [`build_surface`] is the only ranking pipeline.
// trace:exempt reason=internal-detail
pub fn task_delta(
    compiler: &ContextCompiler,
    goal: &str,
    visible: &ContextLedger,
    budget_tokens: usize,
) -> String {
    task_delta_with_ids(compiler, goal, visible, budget_tokens).0
}

/// `task_delta` plus the ids it rendered (for ledger recording).
// trace:exempt reason=internal-detail
pub fn task_delta_with_ids(
    compiler: &ContextCompiler,
    goal: &str,
    visible: &ContextLedger,
    budget_tokens: usize,
) -> (String, Vec<String>) {
    let render = build_surface(
        compiler,
        SurfaceRequest {
            mode: SurfaceMode::Task {
                goal,
                visible: Some(visible),
            },
            budget: budget_tokens,
            explain: false,
            policy: SurfacePolicy::defaults(budget_tokens),
            semantic: None,
        },
    );
    let mut out = String::new();
    out.push_str("# SCC TASK DELTA\n");
    out.push_str(&format!("TASK-FOCUS: {goal}\n"));
    out.push_str("Relevant APIs not already visible:\n");
    let body = render
        .text
        .strip_prefix("SCC SYSTEM SURFACE MAP\n\n")
        .unwrap_or(&render.text);
    out.push_str(body.trim_end());
    out.push('\n');
    (out, render.rendered_ids)
}

/// Task-personalized surface map (full map, no novelty filter — this is a
/// map, not a delta): entries re-ranked by task PPR + importance via the
/// one authoritative [`build_surface`] service in Task mode.
// trace:exempt reason=internal-detail
pub fn task_surface(
    compiler: &ContextCompiler,
    goal: &str,
    budget_tokens: usize,
    explain: bool,
) -> String {
    task_surface_with_ids(compiler, goal, budget_tokens, explain).0
}

/// `task_surface` plus the rendered entry ids (for ledger recording).
// trace:exempt reason=internal-detail
pub fn task_surface_with_ids(
    compiler: &ContextCompiler,
    goal: &str,
    budget_tokens: usize,
    explain: bool,
) -> (String, Vec<String>) {
    let render = build_surface(
        compiler,
        SurfaceRequest {
            mode: SurfaceMode::Task {
                goal,
                visible: None,
            },
            budget: budget_tokens,
            explain,
            policy: SurfacePolicy::defaults(budget_tokens),
            semantic: None,
        },
    );
    let mut out = String::new();
    out.push_str(&format!("# SYSTEM SURFACE MAP (task-personalized: {goal})\n"));
    let body = render
        .text
        .strip_prefix("SCC SYSTEM SURFACE MAP\n\n")
        .unwrap_or(&render.text);
    out.push_str(body.trim_end());
    out.push('\n');
    (out, render.rendered_ids)
}

#[cfg(test)]
mod tests {
    use super::*;

// trace:exempt reason=internal-detail
    fn fixture_compiler() -> (tempfile::TempDir, crate::ContextCompiler<'static>) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Box::leak(Box::new(
            scc_store::Store::open(&dir.path().join("scc.db"), &root).unwrap(),
        ));
        let graph = Box::leak(Box::new(scc_graph::RealityGraph::load(store).unwrap()));
        let settings = crate::ContextSettings::default();
        let comp = crate::ContextCompiler::new(store, graph, settings, Vec::new());
        (dir, comp)
    }

    #[test]
// trace:exempt reason=internal-detail
    fn render_startup_emits_spec_headers() {
        let sc = StartupContext {
            atlas: "ATLAS-BODY".into(),
            surface: "SURFACE-BODY".into(),
            surface_render: scc_core::SurfaceRenderResult {
                text: "SURFACE-BODY".into(),
                rendered_ids: vec![],
                omitted_ids: vec![],
                omissions: vec![],
                token_count: 0,
            },
            coverage: vec!["stale: a.py".into()],
            omissions: vec!["none".into()],
            artifact: ContextArtifact {
                kind: "startup".into(),
                epoch: "epoch:test".into(),
                renderer_version: "test".into(),
                trust_policy: "floor=0.85".into(),
                budget: ContextBudget::default(),
                sha256: "abc".into(),
                content_hash: "def".into(),
                text: String::new(),
            },
        };
        let out = render_startup(&sc);
        assert!(out.contains("# SCC SYSTEM CONTEXT"));
        assert!(out.contains("## SYSTEM ATLAS"));
        assert!(out.contains("## SYSTEM SURFACE MAP"));
        assert!(out.contains("## MODEL COVERAGE"));
        assert!(out.contains("## OMISSIONS"));
        assert!(out.contains("ATLAS-BODY"));
        assert!(out.contains("SURFACE-BODY"));
        assert!(out.contains("sha256:abc"));
        assert!(out.contains("content_hash:def"));
        assert!(out.contains("stale: a.py"));
    }

    #[test]
// trace:exempt reason=internal-detail
    fn artifact_text_equals_rendered_block() {
        let (_dir, comp) = fixture_compiler();
        let budget = ContextBudget::default();
        let sc = build_startup(&comp, &budget, "test-renderer");
        assert_eq!(sc.artifact.text, render_startup(&sc));
        assert_eq!(sc.artifact.sha256.len(), 64);
        assert_eq!(sc.artifact.content_hash.len(), 64);
        assert_ne!(sc.artifact.content_hash, sc.artifact.sha256);
        assert!(render_startup(&sc).contains("content_hash:"));
        assert_eq!(sc.artifact.epoch, comp.store.cache_epoch().unwrap_or_default());
    }

    #[test]
// trace:exempt reason=internal-detail
    fn global_rank_cache_roundtrips_through_the_store() {
        let (_dir, comp) = fixture_compiler();
        // miss on a cold store
        assert!(load_global_rank_cache(&comp).is_none());
        let cache = GlobalRankCache {
            epoch: comp.store.cache_epoch().unwrap_or_else(|_| "no-epoch".into()),
            policy: trust_policy_str(comp.view.policy()),
            salt: comp.settings.rank_salt.clone(),
            global_vector: vec![0.1, 0.2, 0.3],
            node_symbol_map: BTreeMap::from([("repo://r/symbol/a.py/serve".into(), 0.42)]),
            candidates_epoch: comp.store.cache_epoch().unwrap_or_default(),
            candidate_ids: vec!["repo://r/symbol/a.py/serve".into()],
            hits: 1,
        };
        store_global_rank_cache(&comp, &cache);
        let loaded = load_global_rank_cache(&comp).expect("cache entry present after store");
        assert_eq!(loaded, cache);
        assert_eq!(loaded.hits, 1);
        assert_eq!(
            loaded.node_symbol_map.get("repo://r/symbol/a.py/serve"),
            Some(&0.42)
        );
        assert_eq!(loaded.candidates_epoch, loaded.epoch);
    }

    #[test]
// trace:exempt reason=internal-detail
    fn visible_ids_consume_the_same_render_the_artifact_printed() {
        // The ledger-visible surface ids MUST be exactly the render's
        // rendered_ids (never a rebuild, never the candidate pool): every
        // rendered entry's logical symbol is visible. The inverse
        // direction (omitted candidates never visible) is asserted in the
        // indexed CLI fixture (surface_startup.rs) where the atlas symbol
        // set is controlled.
        let (_dir, comp) = fixture_compiler();
        let budget = ContextBudget::default();
        let sc = build_startup(&comp, &budget, "test-renderer");
        assert!(sc.artifact.text.contains(&sc.surface));
        let (syms, _files, _comps, _flows) = visible_ids_from_startup(&comp, &sc);
        for id in &sc.surface_render.rendered_ids {
            let symbol_id = symbol_id_of(id);
            assert!(
                syms.contains(&symbol_id),
                "rendered entry {id} must be marked visible (symbol {symbol_id})"
            );
        }
    }
}
