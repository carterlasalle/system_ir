//! Atlas recall benchmark v2 (Wave 8 §57): structured recall of
//! independently documented ground truth against the startup System Atlas,
//! with precision and token-density metrics and per-gap diagnosis.
//!
//! Ground truth is organized into seven layers (the v2 ontology):
//! `architecture` (components/subsystems the agent must know at startup),
//! `entrypoints` (invokable surfaces), `behavior` (flows/lifecycles),
//! `state_authority` (state owners), `contracts` (HTTP/CLI/API contracts),
//! `landmarks` (symbols one zoom level deeper — informational), and
//! `tests` (informational). The quality gate is the equal-weighted mean of
//! the FIVE startup-required layers only (architecture, entrypoints,
//! behavior, state_authority, contracts) — landmarks/tests are excluded,
//! which is the anti-bloat guarantee: dumping implementation symbols into
//! the atlas can no longer inflate the score.
//!
//! Scoring is STRUCTURED, not text-substring: each layer is matched against
//! the machine model (`scc_context::atlas::build_atlas`), not the rendered
//! text. Item/haystack normalization applies the documented aliases
//! (`::` -> `.`, `fn X` -> `X`, `./p` -> `p`) so e.g. `Controller::run`
//! matches a flow step rendered as `Controller.run`.
//!
//! v2 metrics:
//! - `precision`: fraction of canonical flow-graph edges whose (from-op,
//!   to-op) pair appears — in order — inside some ground-truth `behavior`
//!   item (a same-item chain step). A pragmatic false-causal proxy: an edge
//!   the ground truth never describes may be a spurious causal link.
//! - `density` (startup_facts_per_1k_tokens): matched startup-required
//!   items per 1000 atlas tokens; `atlas_tokens` per repo is reported too.
//!
//! `--diagnose` classifies every missed item by WHERE it disappeared
//! (PARSER/EXTRACTOR/WRITER/RESOLUTION/COMPILER/PROJECTION/ALIAS) via a
//! deterministic store->flows->components->text ladder, and prints a
//! per-kind histogram plus per-repo gap lines (the regeneration source for
//! `benchmarks/results/ground-truth-gaps.md`).
//!
//! When `benchmarks/corpus/` is absent (or empty), the harness falls back to
//! the golden `fixtures/`: ground truth is synthesized from
//! `benchmarks/tasks.json`, fixture copies are indexed in a temp dir (the
//! golden fixtures are never written into), and the same recall pipeline runs.

use crate::benchctx::{BenchmarkCorpus, BenchTask};
use crate::Compiler;
use scc_context::atlas;
use scc_context::ContextCompiler;
use scc_core::{Entity, FlowGraph};
use scc_indexer::scan::Language;
use scc_store::Store;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Quality gate: overall mean recall must be >= this floor (Wave 8 §57).
/// The floor is over the five startup-required layers ONLY.
pub const ATLAS_GATE: f64 = 0.5;

/// Holdout verdict tolerance: the blind holdout corpus may lag the dev
/// corpus by up to this much (overall recall, absolute) before the run is
/// called OVERFIT. The band absorbs corpus-difficulty, LOC-mix, and
/// ground-truth-strictness differences; a lag beyond it means the dev-tuned
/// rules do not generalize to unseen repos.
pub const HOLDOUT_TOLERANCE: f64 = 0.05;

/// The five startup-required layers that count toward the overall score
/// (architecture, entrypoints, behavior, state_authority, contracts — see
/// `ALL_SECTIONS`; landmarks + tests are informational).
const ALL_SECTIONS: [&str; 7] = [
    "architecture",
    "entrypoints",
    "behavior",
    "state_authority",
    "contracts",
    "landmarks",
    "tests",
];

/// Ground-truth sections parsed from `benchmarks/ground-truth/<name>.md`
/// (one `- <key string>` bullet per item). The v2 ontology; legacy section
/// names (components/flows/ownership) are accepted and normalized.
#[derive(Debug, Clone, Default)]
pub struct GroundTruthDoc {
    pub architecture: Vec<String>,
    pub entrypoints: Vec<String>,
    pub behavior: Vec<String>,
    pub state_authority: Vec<String>,
    pub contracts: Vec<String>,
    pub landmarks: Vec<String>,
    pub tests: Vec<String>,
}

impl GroundTruthDoc {
    pub fn section(&self, name: &str) -> &Vec<String> {
        match name {
            "architecture" => &self.architecture,
            "entrypoints" => &self.entrypoints,
            "behavior" => &self.behavior,
            "state_authority" => &self.state_authority,
            "contracts" => &self.contracts,
            "landmarks" => &self.landmarks,
            "tests" => &self.tests,
            _ => unreachable!("unknown section {name}"),
        }
    }

    fn section_mut(&mut self, name: &str) -> &mut Vec<String> {
        match name {
            "architecture" => &mut self.architecture,
            "entrypoints" => &mut self.entrypoints,
            "behavior" => &mut self.behavior,
            "state_authority" => &mut self.state_authority,
            "contracts" => &mut self.contracts,
            "landmarks" => &mut self.landmarks,
            "tests" => &mut self.tests,
            _ => unreachable!("unknown section {name}"),
        }
    }

    /// Remove duplicates, preserving first-seen order.
    fn dedupe(&mut self) {
        for name in ALL_SECTIONS {
            let mut seen: BTreeSet<String> = BTreeSet::new();
            self.section_mut(name).retain(|item| seen.insert(item.clone()));
        }
    }

    fn to_markdown(&self) -> String {
        let mut out = String::from("# fixtures fallback (synthesized from benchmarks/tasks.json)\n");
        for name in ALL_SECTIONS {
            out.push_str(&format!("## {name}\n"));
            for item in self.section(name) {
                out.push_str(&format!("- {item}\n"));
            }
        }
        out
    }
}

/// Gap-kind classification for a missed ground-truth item (`--diagnose`):
/// where the fact disappeared between source and the rendered atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GapKind {
    /// The symbol/string is not parseable by any enabled extractor
    /// (language disabled, ignored path, or a format no extractor reads).
    Parser,
    /// Parsed, but no semantic fact was emitted for it (e.g. no route /
    /// registration / export extraction for the construct).
    Extractor,
    /// The fact exists in the ExtractedFile but was not written to the
    /// store. Not observable from the store side (the heuristic ladder
    /// below maps parsed-but-absent facts to `Extractor`); the kind exists
    /// so the taxonomy is complete.
    Writer,
    /// Exists in the store but was never wired into the graph (the matched
    /// symbol has zero relationships — resolution/compilation never reached
    /// it, so no flow or component can carry it).
    Resolution,
    /// Exists in the store but was not compiled into components/flows.
    Compiler,
    /// Compiled into a component but dropped by budget/policy/rendering.
    Projection,
    /// Likely present under a different spelling; the aliases did not match.
    Alias,
}

impl GapKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            GapKind::Parser => "PARSER",
            GapKind::Extractor => "EXTRACTOR",
            GapKind::Writer => "WRITER",
            GapKind::Resolution => "RESOLUTION",
            GapKind::Compiler => "COMPILER",
            GapKind::Projection => "PROJECTION",
            GapKind::Alias => "ALIAS",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GapFinding {
    pub section: String,
    pub item: String,
    pub kind: GapKind,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoRecall {
    pub repo: String,
    pub architecture: f64,
    pub entrypoints: f64,
    pub behavior: f64,
    pub state_authority: f64,
    pub contracts: f64,
    pub landmarks: f64,
    pub tests: f64,
    /// Equal-weighted mean of the five startup-required layers (the gate).
    pub overall: f64,
    /// Fraction of canonical flow-graph edges whose (from-op, to-op) pair
    /// appears in order inside some ground-truth behavior item.
    pub precision: f64,
    /// Matched startup-required items per 1000 atlas tokens.
    pub density: f64,
    /// Rendered atlas token count.
    pub atlas_tokens: usize,
    /// Number of call edges upgraded to RESOLVED by the semantic backends
    /// (pyright/tsserver) before scoring; 0 when `--no-resolve`.
    pub resolved_calls: usize,
    /// Number of ground-truth items in the (informational) landmarks layer.
    pub landmark_items: usize,
    /// When set, the repo was not scored (missing dir / missing ground
    /// truth / index failure) and the recall fields are meaningless.
    pub skipped_reason: Option<String>,
    /// Ground-truth key strings the structured matcher missed
    /// (`section:key`), for diagnosing misses.
    pub missed: Vec<String>,
    /// Per-item gap classification (populated when `--diagnose`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gaps: Vec<GapFinding>,
}

impl RepoRecall {
    fn skipped(repo: &str, reason: impl Into<String>) -> Self {
        RepoRecall {
            repo: repo.to_string(),
            skipped_reason: Some(reason.into()),
            ..Default::default()
        }
    }
}

impl Default for RepoRecall {
    fn default() -> Self {
        RepoRecall {
            repo: String::new(),
            architecture: 0.0,
            entrypoints: 0.0,
            behavior: 0.0,
            state_authority: 0.0,
            contracts: 0.0,
            landmarks: 0.0,
            tests: 0.0,
            overall: 0.0,
            precision: 0.0,
            density: 0.0,
            atlas_tokens: 0,
            resolved_calls: 0,
            landmark_items: 0,
            skipped_reason: None,
            missed: Vec::new(),
            gaps: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AtlasRecallReport {
    /// One row per requested repo, in sorted order; skipped repos carry
    /// `skipped_reason`.
    pub repos: Vec<RepoRecall>,
    /// Where the run came from: "benchmarks/corpus" or "fixtures fallback".
    pub mode: String,
    pub mean_architecture: f64,
    pub mean_entrypoints: f64,
    pub mean_behavior: f64,
    pub mean_state_authority: f64,
    pub mean_contracts: f64,
    pub mean_landmarks: f64,
    pub mean_tests: f64,
    /// Equal-weighted mean of the five startup-required layers over scored
    /// repos (the gate).
    pub mean_overall: f64,
    pub mean_precision: f64,
    pub mean_density: f64,
    pub mean_atlas_tokens: f64,
    pub scored: usize,
    pub skipped: usize,
    pub gate_passed: bool,
    /// Gap-kind histogram over all diagnosed items (kind -> count).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub gap_histogram: BTreeMap<String, usize>,
}

/// Parse a ground-truth markdown doc into per-section key strings.
///
/// Accepts the Wave 8 corpus format (`## section` heading + `- item` bullets)
/// for both the v2 ontology (architecture/entrypoints/behavior/
/// state_authority/contracts/landmarks/tests) and the legacy names
/// (components -> architecture, flows -> behavior, ownership ->
/// state_authority). A bullet is either the bare key string or
/// `<key string> — explanation`; the explanation is not expected in atlas
/// output, so only the key string (before ` — `) is kept. Inline-code
/// backticks are stripped.
pub fn parse_ground_truth(md: &str) -> GroundTruthDoc {
    let mut doc = GroundTruthDoc::default();
    let mut current: Option<&'static str> = None;
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for raw in md.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("## ") {
            current = match rest.trim().to_ascii_lowercase().as_str() {
                "architecture" | "components" => Some("architecture"),
                "entrypoints" => Some("entrypoints"),
                "behavior" | "flows" => Some("behavior"),
                "state_authority" | "ownership" => Some("state_authority"),
                "contracts" => Some("contracts"),
                "landmarks" => Some("landmarks"),
                "tests" => Some("tests"),
                _ => None,
            };
            continue;
        }
        let Some(section) = current else { continue };
        let Some(item) = line
            .strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
        else {
            continue;
        };
        let item = item.trim().trim_matches('`').trim();
        if item.is_empty() {
            continue;
        }
        let key = match item.split_once(" — ") {
            Some((k, _)) => k.trim().trim_matches('`').trim(),
            None => item,
        };
        if key.is_empty() {
            continue;
        }
        // duplicates (same key bulleted twice in one section, e.g. a route
        // listed both as decorator and as canonical form) would skew the
        // denominator — keep the first occurrence only.
        if !seen.insert(format!("{section}:{key}")) {
            continue;
        }
        doc.section_mut(section).push(key.to_string());
    }
    doc
}

/// Normalize a ground-truth item / atlas string for matching, applying the
/// documented aliases: `::` -> `.` (so `Controller::run` matches
/// `Controller.run`), `fn X` -> `X`, and `./p` -> `p` (path prefix).
fn norm(s: &str) -> String {
    let mut out = s.to_ascii_lowercase();
    out = out.replace("::", ".");
    if let Some(rest) = out.strip_prefix("fn ") {
        out = rest.to_string();
    }
    if let Some(rest) = out.strip_prefix("./") {
        out = rest.to_string();
    }
    out
}

/// Normalize and join parts into one haystack (newline-separated).
fn norm_join(parts: impl IntoIterator<Item = String>) -> String {
    let mut out: Vec<String> = parts.into_iter().map(|p| norm(&p)).collect();
    out.sort();
    out.dedup();
    out.join("\n")
}

/// Structured atlas haystacks, one per ontology layer, built from the
/// machine model (`SystemAtlas`) plus the rendered pack (for the
/// informational `tests` layer and the ALIAS gap check).
struct AtlasLayers {
    architecture: String,
    entrypoints: String,
    behavior: String,
    state_authority: String,
    contracts: String,
    landmarks: String,
    /// Normalized rendered atlas content (tests layer + alias check).
    text: String,
    /// Normalized flow inventory: derived flows + canonical flow graphs
    /// (names, triggers, step/node operations) — used by gap diagnosis.
    flows: String,
    /// Normalized component inventory (names, purposes, implementations,
    /// owns targets) — used by gap diagnosis.
    components: String,
}

fn build_layers(ctx: &ContextCompiler<'_>, pack: &scc_context::ContextPack) -> AtlasLayers {
    let atlas = atlas::build_atlas(ctx);

    let mut arch_parts: Vec<String> = Vec::new();
    let mut sa_parts: Vec<String> = Vec::new();
    let mut land_parts: Vec<String> = Vec::new();
    let mut comp_parts: Vec<String> = Vec::new();
    for c in &atlas.components {
        comp_parts.push(c.name.clone());
        comp_parts.push(c.purpose.clone());
        comp_parts.extend(c.implementation.iter().cloned());
        comp_parts.extend(c.owns.iter().map(|o| o.target.clone()));
        arch_parts.push(c.name.clone());
        if !c.purpose.is_empty() {
            arch_parts.push(c.purpose.clone());
        }
        arch_parts.extend(c.implementation.iter().cloned());
        for o in &c.owns {
            sa_parts.push(o.target.clone());
        }
        land_parts.extend(c.implementation.iter().cloned());
    }
    sa_parts.extend(atlas.data_stores.iter().cloned());

    let mut ep_parts: Vec<String> = Vec::new();
    for e in &atlas.entrypoints {
        ep_parts.push(e.name.clone());
        ep_parts.push(e.trigger.clone());
        // the symbol id's last segment is the symbol name (e.g.
        // repo://x/symbol/fastapi/applications.py/FastAPI -> FastAPI)
        if let Some(seg) = e.symbol.rsplit('/').next() {
            if !seg.is_empty() {
                ep_parts.push(seg.to_string());
            }
        }
    }

    let mut bh_parts: Vec<String> = Vec::new();
    let mut flow_parts: Vec<String> = Vec::new();
    for f in &atlas.flows {
        bh_parts.push(f.name.clone());
        flow_parts.push(f.name.clone());
        if let Some(t) = &f.trigger {
            bh_parts.push(t.clone());
            flow_parts.push(t.clone());
        }
        for s in &f.steps {
            bh_parts.push(s.clone());
            flow_parts.push(s.clone());
            land_parts.push(s.clone());
        }
    }
    // canonical flow graphs (the flow edge source) — ops join the flow
    // inventory for diagnosis even when the flow was not rendered
    if let Ok(graphs) = ctx.store.flow_graphs() {
        for g in &graphs {
            flow_parts.push(g.name.clone());
            if let Some(t) = &g.trigger {
                flow_parts.push(t.clone());
            }
            for n in &g.nodes {
                flow_parts.push(n.operation.clone());
                flow_parts.push(n.actor.clone());
            }
        }
    }
    // derived (non-sequence) flows: view flows carry name/trigger/steps
    for f in ctx.view.flows() {
        flow_parts.push(f.name.clone());
        if let Some(t) = &f.trigger {
            flow_parts.push(t.clone());
        }
        for s in &f.steps {
            flow_parts.push(s.actor.clone());
            flow_parts.push(s.operation.clone());
        }
    }

    // Wave 9: contracts are first-class `Contract` records; the layer
    // haystack keeps the contract strings (operations) exactly as before.
    let contracts: Vec<String> = atlas
        .contracts
        .iter()
        .flat_map(|c| c.operations.iter().cloned())
        .collect();

    AtlasLayers {
        architecture: norm_join(arch_parts),
        entrypoints: norm_join(ep_parts),
        behavior: norm_join(bh_parts),
        state_authority: norm_join(sa_parts),
        contracts: norm_join(contracts),
        landmarks: norm_join(land_parts),
        text: norm(&pack.content),
        flows: norm_join(flow_parts),
        components: norm_join(comp_parts),
    }
}

/// The haystack a layer matches against.
fn layer_haystack<'a>(section: &str, layers: &'a AtlasLayers, text_norm: &'a str) -> &'a str {
    match section {
        "architecture" => &layers.architecture,
        "entrypoints" => &layers.entrypoints,
        "behavior" => &layers.behavior,
        "state_authority" => &layers.state_authority,
        "contracts" => &layers.contracts,
        "landmarks" => &layers.landmarks,
        "tests" => text_norm,
        _ => unreachable!("unknown section {section}"),
    }
}

/// Recall for one layer: fraction of ground-truth key strings found
/// (case-insensitive, aliases applied) in the layer's structured haystack.
/// An empty ground truth scores 1.0 (nothing to miss).
fn layer_recall(items: &[String], haystack: &str) -> (f64, usize, usize) {
    if items.is_empty() {
        return (1.0, 0, 0);
    }
    let mut hit = 0usize;
    for item in items {
        if haystack.contains(&norm(item)) {
            hit += 1;
        }
    }
    (hit as f64 / items.len() as f64, hit, items.len())
}

/// Whether one ground-truth item matches its layer's structured haystack.
fn item_matched(section: &str, item: &str, layers: &AtlasLayers, text_norm: &str) -> bool {
    layer_haystack(section, layers, text_norm).contains(&norm(item))
}

/// Flow-edge precision: the fraction of canonical flow-graph edges whose
/// (from-op, to-op) pair appears — in order — inside some ground-truth
/// `## behavior` item. An edge the ground truth never describes may be a
/// spurious causal link; a same-item chain step (e.g. `worker consumer ->
/// task.run`) supports exactly its consecutive pairs. Empty ground truth
/// (nothing to contradict any edge) or an empty graph scores 1.0.
fn flow_edge_precision(graphs: &[FlowGraph], behavior: &[String]) -> f64 {
    if behavior.is_empty() {
        return 1.0;
    }
    let mut total = 0usize;
    let mut supported = 0usize;
    for g in graphs {
        for e in &g.edges {
            let from = g
                .nodes
                .get(e.from as usize)
                .map(|n| norm(&n.operation))
                .unwrap_or_default();
            let to = g
                .nodes
                .get(e.to as usize)
                .map(|n| norm(&n.operation))
                .unwrap_or_default();
            if from.is_empty() || to.is_empty() {
                continue;
            }
            total += 1;
            let ok = behavior.iter().any(|item| {
                let it = norm(item);
                match (it.find(&from), it.find(&to)) {
                    (Some(a), Some(b)) => a < b,
                    _ => false,
                }
            });
            if ok {
                supported += 1;
            }
        }
    }
    if total == 0 {
        return 1.0;
    }
    supported as f64 / total as f64
}

/// Startup facts per 1000 atlas tokens: matched startup-required items over
/// the rendered pack's token count.
fn token_density(matched_startup: usize, tokens: usize) -> f64 {
    if tokens == 0 {
        return 0.0;
    }
    matched_startup as f64 / (tokens as f64 / 1000.0)
}

/// Index one repo in place and score its ground truth against the atlas.
///
/// v2 scoring is structured: each layer is matched against the machine
/// `SystemAtlas` (component name/purpose/implementation for architecture;
/// entrypoint name/trigger/symbol; flow name/trigger/step ops; owns claims +
/// data stores; contracts), except the informational `tests` layer which is
/// matched against the rendered text. `diagnose` additionally classifies
/// every missed item by gap kind.
///
/// `resolve` runs the language-aware semantic backends (pyright + tsserver)
/// on the freshly indexed repo before the atlas is built, so call chains
/// upgrade from EXTRACTED candidates to RESOLVED edges and the behavior
/// flows are seeded from resolved paths; the per-repo `resolved_calls`
/// reports how many edges were upgraded (0 with `--no-resolve` or when the
/// backends are unavailable — resolution degrades, never fails the run).
pub fn score_repo(
    repo_dir: &Path,
    gt: &GroundTruthDoc,
    diagnose: bool,
    resolve: bool,
) -> Result<RepoRecall, String> {
    crate::commands::cmd_index(repo_dir, true).map_err(|e| format!("index failed: {e}"))?;
    let resolved_calls = if resolve {
        match crate::resolve_and_recompile(repo_dir) {
            Ok(rep) => rep.upgraded,
            Err(e) => {
                eprintln!(
                    "benchatlas: semantic resolution skipped for {}: {e}",
                    repo_dir.display()
                );
                0
            }
        }
    } else {
        0
    };
    let store = crate::open_store(repo_dir).map_err(|e| format!("store: {e}"))?;
    let config = crate::load_config(repo_dir).map_err(|e| format!("config: {e}"))?;
    let stale = crate::stale_paths(&store).map_err(|e| format!("stale: {e}"))?;
    let comp = crate::compiler(&store, &config, stale).map_err(|e| format!("compiler: {e}"))?;
    let pack = comp.ctx().system_atlas(None);
    let layers = build_layers(&comp.ctx(), &pack);
    let text_norm = &layers.text;

    let (architecture, arch_hit, _) = layer_recall(&gt.architecture, &layers.architecture);
    let (entrypoints, ep_hit, _) = layer_recall(&gt.entrypoints, &layers.entrypoints);
    let (behavior, bh_hit, _) = layer_recall(&gt.behavior, &layers.behavior);
    let (state_authority, sa_hit, _) = layer_recall(&gt.state_authority, &layers.state_authority);
    let (contracts, ct_hit, _) = layer_recall(&gt.contracts, &layers.contracts);
    let (landmarks, _, _) = layer_recall(&gt.landmarks, &layers.landmarks);
    let (tests, _, _) = layer_recall(&gt.tests, text_norm);
    let overall = (architecture + entrypoints + behavior + state_authority + contracts) / 5.0;

    let matched_startup = arch_hit + ep_hit + bh_hit + sa_hit + ct_hit;
    let graphs = store.flow_graphs().unwrap_or_default();
    let precision = flow_edge_precision(&graphs, &gt.behavior);
    let density = token_density(matched_startup, pack.tokens);

    let mut missed: Vec<String> = Vec::new();
    for section in ALL_SECTIONS {
        for item in gt.section(section) {
            if !item_matched(section, item, &layers, text_norm) {
                missed.push(format!("{section}:{item}"));
            }
        }
    }

    let gaps = if diagnose {
        let store_cands = store_candidates(&comp);
        missed
            .iter()
            .map(|m| {
                let (section, item) = m
                    .split_once(':')
                    .map(|(s, i)| (s.to_string(), i.to_string()))
                    .unwrap_or_else(|| ("?".into(), m.clone()));
                classify_gap(
                    &section,
                    &item,
                    repo_dir,
                    &store,
                    &config,
                    &comp,
                    &layers,
                    &store_cands,
                )
            })
            .collect()
    } else {
        Vec::new()
    };

    let repo = repo_dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| repo_dir.display().to_string());
    Ok(RepoRecall {
        repo,
        architecture,
        entrypoints,
        behavior,
        state_authority,
        contracts,
        landmarks,
        tests,
        overall,
        precision,
        density,
        atlas_tokens: pack.tokens,
        resolved_calls,
        landmark_items: gt.landmarks.len(),
        skipped_reason: None,
        missed,
        gaps,
    })
}

/// Run the recall benchmark over `repo_names` (sorted for deterministic
/// output). Repos whose corpus dir is missing, whose ground-truth doc is
/// missing, or whose index/atlas fails are recorded with `skipped_reason` —
/// this function never panics on missing dirs.
pub fn run_atlas_recall(
    corpus_dir: &Path,
    ground_truth_dir: &Path,
    repo_names: &[String],
    diagnose: bool,
    resolve: bool,
) -> Result<AtlasRecallReport, String> {
    let mut names: Vec<&String> = repo_names.iter().collect();
    names.sort();

    let mut report = AtlasRecallReport {
        mode: format!("corpus: {}", corpus_dir.display()),
        ..Default::default()
    };
    for name in names {
        let repo_dir = corpus_dir.join(name);
        if !repo_dir.is_dir() {
            report.skipped += 1;
            report
                .repos
                .push(RepoRecall::skipped(name, "corpus dir missing"));
            continue;
        }
        let gt_path = ground_truth_dir.join(format!("{name}.md"));
        let gt = match std::fs::read_to_string(&gt_path) {
            Ok(text) => parse_ground_truth(&text),
            Err(_) => {
                report.skipped += 1;
                report.repos.push(RepoRecall::skipped(
                    name,
                    format!("ground truth missing: {}", gt_path.display()),
                ));
                continue;
            }
        };
        match score_repo(&repo_dir, &gt, diagnose, resolve) {
            Ok(r) => {
                report.mean_architecture += r.architecture;
                report.mean_entrypoints += r.entrypoints;
                report.mean_behavior += r.behavior;
                report.mean_state_authority += r.state_authority;
                report.mean_contracts += r.contracts;
                report.mean_landmarks += r.landmarks;
                report.mean_tests += r.tests;
                report.mean_precision += r.precision;
                report.mean_density += r.density;
                report.mean_atlas_tokens += r.atlas_tokens as f64;
                if diagnose {
                    for g in &r.gaps {
                        *report
                            .gap_histogram
                            .entry(g.kind.as_str().to_string())
                            .or_insert(0) += 1;
                    }
                }
                report.scored += 1;
                report.repos.push(r);
            }
            Err(e) => {
                report.skipped += 1;
                report.repos.push(RepoRecall::skipped(name, e));
            }
        }
    }

    if report.scored > 0 {
        let n = report.scored as f64;
        report.mean_architecture /= n;
        report.mean_entrypoints /= n;
        report.mean_behavior /= n;
        report.mean_state_authority /= n;
        report.mean_contracts /= n;
        report.mean_landmarks /= n;
        report.mean_tests /= n;
        report.mean_precision /= n;
        report.mean_density /= n;
        report.mean_atlas_tokens /= n;
        report.mean_overall = (report.mean_architecture
            + report.mean_entrypoints
            + report.mean_behavior
            + report.mean_state_authority
            + report.mean_contracts)
            / 5.0;
    }
    report.gate_passed = report.mean_overall >= ATLAS_GATE;
    Ok(report)
}

/// Top-level entry for `scc bench atlas`: locate the workspace, resolve the
/// corpus/ground-truth directories (or the fixtures fallback), and run.
pub fn run_atlas_bench(
    corpus: Option<&Path>,
    ground_truth: Option<&Path>,
    diagnose: bool,
    resolve: bool,
) -> Result<AtlasRecallReport, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let root = crate::find_root(&cwd);
    let default_corpus = root.join("benchmarks").join("corpus");

    // Fixtures fallback: no corpus dir (or an empty one) -> run over the
    // golden fixtures with ground truth synthesized from tasks.json.
    if corpus.is_none() && repo_dirs(&default_corpus).is_empty() {
        return fixtures_fallback(&root, diagnose, resolve);
    }
    if let Some(p) = corpus {
        if !p.is_dir() {
            return Err(format!("corpus dir not found: {}", p.display()));
        }
    }

    let corpus_dir = corpus.map(Path::to_path_buf).unwrap_or(default_corpus);
    let gt_dir = match ground_truth {
        Some(p) => p.to_path_buf(),
        None => root.join("benchmarks").join("ground-truth"),
    };
    let names = repo_dirs(&corpus_dir);
    run_atlas_recall(&corpus_dir, &gt_dir, &names, diagnose, resolve)
}

/// Overfit verdict over the dev-vs-holdout overall gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HoldoutVerdict {
    /// holdout >= dev: the dev-tuned rules generalize at least as well.
    NoOverfit,
    /// dev - tolerance <= holdout < dev: lag inside the noise band.
    Borderline,
    /// holdout < dev - tolerance: dev-tuned rules do not generalize.
    Overfit,
}

impl HoldoutVerdict {
    pub fn as_str(&self) -> &'static str {
        match self {
            HoldoutVerdict::NoOverfit => "NO OVERFIT",
            HoldoutVerdict::Borderline => "BORDERLINE",
            HoldoutVerdict::Overfit => "OVERFIT",
        }
    }
}

/// Verdict over the dev-vs-holdout overall gap (`dev`, `holdout` are the
/// equal-weighted mean recalls of the five startup-required layers).
pub fn holdout_verdict(dev: f64, holdout: f64) -> HoldoutVerdict {
    if holdout >= dev {
        HoldoutVerdict::NoOverfit
    } else if holdout >= dev - HOLDOUT_TOLERANCE {
        HoldoutVerdict::Borderline
    } else {
        HoldoutVerdict::Overfit
    }
}

/// Dev-vs-holdout comparison for `scc bench atlas --holdout`.
#[derive(Debug, Clone, Serialize)]
pub struct HoldoutComparison {
    pub dev: AtlasRecallReport,
    pub holdout: AtlasRecallReport,
    /// Per-layer gap = holdout mean - dev mean (fraction, negative = lag).
    pub gap_architecture: f64,
    pub gap_entrypoints: f64,
    pub gap_behavior: f64,
    pub gap_state_authority: f64,
    pub gap_contracts: f64,
    pub gap_overall: f64,
    pub verdict: HoldoutVerdict,
    /// Path of the written comparison file.
    pub results_file: String,
}

impl HoldoutComparison {
    fn layer_gap(dev: f64, holdout: f64) -> f64 {
        (holdout - dev).clamp(-1.0, 1.0)
    }
}

/// Run the holdout protocol: score the dev corpus and the blind holdout
/// corpus with the same recall pipeline, compute per-layer gaps, write
/// `benchmarks/results/holdout-v2.txt`, and return the comparison.
///
/// `corpus`/`ground_truth` (when given) select the DEV corpus, exactly as in
/// `run_atlas_bench` (defaults: `benchmarks/corpus` +
/// `benchmarks/ground-truth`). The holdout dirs are fixed protocol paths:
/// `benchmarks/holdout` + `benchmarks/holdout-ground-truth`. The holdout
/// corpus must exist — a missing dir is an error, not a silent empty run.
///
/// `resolve` applies to BOTH corpora (the same pipeline must score dev and
/// holdout identically).
pub fn run_atlas_holdout(
    corpus: Option<&Path>,
    ground_truth: Option<&Path>,
    diagnose: bool,
    resolve: bool,
) -> Result<HoldoutComparison, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let root = crate::find_root(&cwd);
    let results_dir = root.join("benchmarks").join("results");
    let results_file = results_dir.join("holdout-v2.txt");

    let dev = run_atlas_bench(corpus, ground_truth, diagnose, resolve)?;

    let holdout_corpus = root.join("benchmarks").join("holdout");
    let holdout_gt = root.join("benchmarks").join("holdout-ground-truth");
    if !holdout_corpus.is_dir() {
        return Err(format!(
            "holdout corpus dir not found (run --holdout from the workspace): {}",
            holdout_corpus.display()
        ));
    }
    let names = repo_dirs(&holdout_corpus);
    if names.is_empty() {
        return Err(format!(
            "holdout corpus dir is empty: {}",
            holdout_corpus.display()
        ));
    }
    let mut holdout = run_atlas_recall(&holdout_corpus, &holdout_gt, &names, diagnose, resolve)?;
    holdout.mode = format!("holdout: {}", holdout_corpus.display());

    let c = HoldoutComparison {
        gap_architecture: HoldoutComparison::layer_gap(dev.mean_architecture, holdout.mean_architecture),
        gap_entrypoints: HoldoutComparison::layer_gap(dev.mean_entrypoints, holdout.mean_entrypoints),
        gap_behavior: HoldoutComparison::layer_gap(dev.mean_behavior, holdout.mean_behavior),
        gap_state_authority: HoldoutComparison::layer_gap(
            dev.mean_state_authority,
            holdout.mean_state_authority,
        ),
        gap_contracts: HoldoutComparison::layer_gap(dev.mean_contracts, holdout.mean_contracts),
        gap_overall: HoldoutComparison::layer_gap(dev.mean_overall, holdout.mean_overall),
        verdict: holdout_verdict(dev.mean_overall, holdout.mean_overall),
        results_file: results_file.display().to_string(),
        dev,
        holdout,
    };

    std::fs::create_dir_all(&results_dir).map_err(|e| e.to_string())?;
    std::fs::write(&results_file, c.to_results_text()).map_err(|e| e.to_string())?;
    Ok(c)
}

impl HoldoutComparison {
    /// Deterministic markdown text for `benchmarks/results/holdout-v2.txt`.
    fn to_results_text(&self) -> String {
        let mut out = String::new();
        out.push_str("# Holdout v2 — dev corpus vs blind holdout\n");
        out.push_str(&format!("dev corpus:     {}\n", self.dev.mode));
        out.push_str(&format!("holdout corpus: {}\n", self.holdout.mode));
        out.push_str(&format!(
            "results:        {}\n",
            self.results_file
        ));
        out.push('\n');

        let rows: [(&str, f64, f64); 6] = [
            ("architecture", self.dev.mean_architecture, self.holdout.mean_architecture),
            ("entrypoints", self.dev.mean_entrypoints, self.holdout.mean_entrypoints),
            ("behavior", self.dev.mean_behavior, self.holdout.mean_behavior),
            ("state_authority", self.dev.mean_state_authority, self.holdout.mean_state_authority),
            ("contracts", self.dev.mean_contracts, self.holdout.mean_contracts),
            ("overall (gate)", self.dev.mean_overall, self.holdout.mean_overall),
        ];
        out.push_str(&format!(
            "{:<18} {:>10} {:>10} {:>10}\n",
            "layer", "dev", "holdout", "gap"
        ));
        for (layer, dev, ho) in rows {
            let gap = HoldoutComparison::layer_gap(dev, ho);
            out.push_str(&format!(
                "{:<18} {:>10.3} {:>10.3} {:>+10.3}\n",
                layer, dev, ho, gap
            ));
        }
        out.push('\n');
        out.push_str(&format!(
            "scored: dev {} (skipped {}) | holdout {} (skipped {})\n",
            self.dev.scored, self.dev.skipped, self.holdout.scored, self.holdout.skipped
        ));
        out.push_str(&format!(
            "precision: dev {:.3} | holdout {:.3}\n",
            self.dev.mean_precision, self.holdout.mean_precision
        ));
        let dev_resolved: usize = self.dev.repos.iter().map(|r| r.resolved_calls).sum();
        let holdout_resolved: usize = self.holdout.repos.iter().map(|r| r.resolved_calls).sum();
        out.push_str(&format!(
            "resolved calls (upgraded): dev {dev_resolved} | holdout {holdout_resolved}\n"
        ));
        out.push_str(&format!(
            "density (facts/1k tokens): dev {:.2} | holdout {:.2}\n",
            self.dev.mean_density, self.holdout.mean_density
        ));
        out.push_str(&format!(
            "atlas tokens: dev {:.0} | holdout {:.0}\n",
            self.dev.mean_atlas_tokens, self.holdout.mean_atlas_tokens
        ));
        out.push('\n');
        out.push_str(&format!(
            "## verdict: {} (gap = {:.3}; tolerance = {:.3})\n",
            self.verdict.as_str(),
            self.gap_overall,
            HOLDOUT_TOLERANCE
        ));
        match self.verdict {
            HoldoutVerdict::NoOverfit => out.push_str(
                "The blind holdout corpus scores at least as well as the dev corpus; \
                 the dev-tuned rules generalize to unseen repos.\n",
            ),
            HoldoutVerdict::Borderline => out.push_str(
                "The holdout corpus lags the dev corpus, but by less than the tolerance \
                 band; the gap is consistent with corpus-difficulty/ground-truth noise, \
                 not demonstrated overfitting.\n",
            ),
            HoldoutVerdict::Overfit => out.push_str(
                "The holdout corpus lags the dev corpus by more than the tolerance band; \
                 rules tuned on the dev corpus do not generalize to unseen repos.\n",
            ),
        }
        out.push('\n');
        out.push_str("## holdout repo overall recall (sorted)\n");
        for r in &self.holdout.repos {
            match &r.skipped_reason {
                Some(reason) => out.push_str(&format!("  {:<24} skipped: {reason}\n", r.repo)),
                None => out.push_str(&format!(
                    "  {:<24} {:>8.3}\n",
                    r.repo, r.overall
                )),
            }
        }
        out
    }
}

/// Print the dev and holdout reports side by side plus the gap summary.
pub fn print_holdout_report(c: &HoldoutComparison, diagnose: bool) {
    println!("scc bench atlas --holdout — dev corpus vs blind holdout (v1)");
    println!("\n=== DEV corpus ===");
    print_report(&c.dev, diagnose);
    println!("\n=== HOLDOUT corpus ===");
    print_report(&c.holdout, diagnose);
    println!("\n=== gap (holdout - dev) ===");
    println!(
        "  {:<18} {:>10}\n  {:<18} {:>+10.3}\n  {:<18} {:>+10.3}\n  {:<18} {:>+10.3}\n  {:<18} {:>+10.3}\n  {:<18} {:>+10.3}\n  {:<18} {:>+10.3}",
        "layer", "gap",
        "architecture", c.gap_architecture,
        "entrypoints", c.gap_entrypoints,
        "behavior", c.gap_behavior,
        "state_authority", c.gap_state_authority,
        "contracts", c.gap_contracts,
        "overall", c.gap_overall,
    );
    println!(
        "  verdict: {} (holdout {:.3} vs dev {:.3}; tolerance {:.3})",
        c.verdict.as_str(),
        c.holdout.mean_overall,
        c.dev.mean_overall,
        HOLDOUT_TOLERANCE
    );
    println!("  results written to: {}", c.results_file);
}

/// Sorted non-hidden subdirectory names of `dir` (the corpus listing).
fn repo_dirs(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if entry.path().is_dir() {
            out.push(name);
        }
    }
    out.sort();
    out
}

/// Hermetic fixtures run: copy the golden fixtures into a temp corpus (the
/// real fixtures are never indexed into) and synthesize ground-truth docs
/// from `benchmarks/tasks.json`.
fn fixtures_fallback(root: &Path, diagnose: bool, resolve: bool) -> Result<AtlasRecallReport, String> {
    let fixtures = locate_fixtures_dir().ok_or("cannot locate fixtures/ directory")?;
    let tasks_path = root.join("benchmarks").join("tasks.json");
    let text = std::fs::read_to_string(&tasks_path)
        .map_err(|e| format!("cannot read {}: {e}", tasks_path.display()))?;
    let corpus: BenchmarkCorpus =
        serde_json::from_str(&text).map_err(|e| format!("tasks.json parse: {e}"))?;

    let names: Vec<String> = corpus
        .tasks
        .iter()
        .map(|t| t.repo.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let tmp = tempfile::TempDir::new().map_err(|e| e.to_string())?;
    let tmp_corpus = tmp.path().join("corpus");
    let tmp_gt = tmp.path().join("ground-truth");
    std::fs::create_dir_all(&tmp_corpus).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&tmp_gt).map_err(|e| e.to_string())?;
    for name in &names {
        let src = fixtures.join(name);
        if src.is_dir() {
            copy_tree_skip_scc(&src, &tmp_corpus.join(name));
        }
        let doc = ground_truth_from_tasks(&corpus.tasks, name);
        std::fs::write(tmp_gt.join(format!("{name}.md")), doc.to_markdown())
            .map_err(|e| e.to_string())?;
    }

    let mut report = run_atlas_recall(&tmp_corpus, &tmp_gt, &names, diagnose, resolve)?;
    report.mode = "fixtures fallback (ground truth from benchmarks/tasks.json)".to_string();
    Ok(report)
}

/// Fixtures-fallback ground truth per repo, synthesized from the task
/// corpus. Mapping: components -> architecture; routes -> entrypoints AND
/// contracts (HTTP routes are both); symbols proxy flow steps (behavior);
/// stores + data -> state_authority (who owns the store/DB); tests -> tests
/// (informational).
fn ground_truth_from_tasks(tasks: &[BenchTask], repo: &str) -> GroundTruthDoc {
    let mut doc = GroundTruthDoc::default();
    for t in tasks {
        if t.repo != repo {
            continue;
        }
        doc.architecture
            .extend(t.ground_truth.components.iter().cloned());
        for r in &t.ground_truth.routes {
            doc.entrypoints.push(r.clone());
            doc.contracts.push(r.clone());
        }
        doc.behavior
            .extend(t.ground_truth.symbols.iter().cloned());
        doc.state_authority
            .extend(t.ground_truth.stores.iter().cloned());
        doc.state_authority
            .extend(t.ground_truth.data.iter().cloned());
        doc.tests.extend(t.ground_truth.tests.iter().cloned());
    }
    doc.dedupe();
    doc
}

/// Locate the fixtures directory: walk up from cwd; fall back to the
/// workspace-relative path (dev tooling).
pub fn locate_fixtures_dir() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join("fixtures").join("http-service-python").is_dir() {
            return Some(dir.join("fixtures"));
        }
        if !dir.pop() {
            break;
        }
    }
    let manifest: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidate = manifest
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("fixtures"))
        .filter(|p| p.join("http-service-python").is_dir());
    candidate
}

/// Copy a tree, skipping `.scc` state dirs (mirrors golden::copy_tree).
fn copy_tree_skip_scc(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == ".scc" {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        if from.is_dir() {
            std::fs::create_dir_all(&to).unwrap();
            copy_tree_skip_scc(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
}

// ---------------------------------------------------------------------------
// Gap diagnosis (--diagnose)
// ---------------------------------------------------------------------------

/// Normalized presence haystack of one entity: id + name + attribute JSON.
fn entity_norm(e: &Entity) -> String {
    let attrs = serde_json::to_string(&e.attributes).unwrap_or_default();
    norm(&format!("{} {} {}", e.id, e.name, attrs))
}

/// Precompute the store-presence candidates for one repo (entity haystacks +
/// derived route strings), so gap diagnosis does not re-serialize every
/// entity per missed item.
fn store_candidates(comp: &Compiler<'_>) -> Vec<String> {
    let mut cands: Vec<String> = Vec::new();
    for e in comp.graph.entities.values() {
        cands.push(entity_norm(e));
    }
    for e in comp.graph.entities_of_kind(scc_core::kinds::ROUTE) {
        let method = e
            .attributes
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let path = e
            .attributes
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !path.is_empty() {
            cands.push(norm(&format!("{method} {path}")));
        }
    }
    cands
}

/// Classify a missed ground-truth item by where it disappeared, via the
/// deterministic ladder of the v2 spec: store presence -> flows ->
/// components -> rendered atlas text.
///
/// - nothing in the store: file-level parseability decides PARSER (disabled
///   language / no extractor for the format) vs EXTRACTOR (parsed but no
///   semantic fact emitted). WRITER is not observable from the store side
///   (it needs extractor output); parsed-but-absent facts map to EXTRACTOR.
/// - in the store as an isolated symbol (zero graph relationships):
///   RESOLUTION (resolution never connected it to a flow or component).
/// - in the store but never compiled into a component or flow: COMPILER.
/// - compiled into a flow but no component: COMPILER (flow-level only).
/// - compiled into a component but absent from the rendered atlas:
///   PROJECTION (dropped by budget/policy/rendering).
/// - present in the rendered atlas text only under a spelling the
///   structured layers do not carry (README prose, format variants):
///   ALIAS (the aliases did not reconcile it).
#[allow(clippy::too_many_arguments)]
fn classify_gap(
    section: &str,
    item: &str,
    repo_dir: &Path,
    store: &Store,
    config: &scc_indexer::Config,
    comp: &Compiler<'_>,
    layers: &AtlasLayers,
    store_cands: &[String],
) -> GapFinding {
    let n = norm(item);
    let section = section.to_string();
    let item = item.to_string();

    // 1. Store presence.
    if !store_cands.iter().any(|c| c.contains(&n)) {
        // 2. Not in the store at all: file-level parseability.
        return match file_language(item.as_str(), repo_dir, store) {
            Some(lang) if config.language_enabled(lang) => GapFinding {
                section,
                item,
                kind: GapKind::Extractor,
                detail: format!(
                    "file exists and {} is enabled, but no semantic fact reached the store",
                    lang.as_str()
                ),
            },
            Some(lang) => GapFinding {
                section,
                item,
                kind: GapKind::Parser,
                detail: format!(
                    "file is {} but the extractor is disabled/ignored",
                    lang.as_str()
                ),
            },
            None => GapFinding {
                section,
                item,
                kind: GapKind::Extractor,
                detail: "nothing in the store and no extractor emits this fact".into(),
            },
        };
    }

    // 3. In the store: a symbol with zero graph relationships was never
    // wired by resolution/compilation.
    let sym_matches: Vec<&Entity> = comp
        .graph
        .entities
        .values()
        .filter(|e| e.kind == scc_core::kinds::SYMBOL && entity_norm(e).contains(&n))
        .collect();
    let isolated = !sym_matches.is_empty()
        && sym_matches.iter().all(|e| {
            comp.graph
                .out
                .get(&e.id)
                .map(|r| r.is_empty())
                .unwrap_or(true)
                && comp
                    .graph
                    .inn
                    .get(&e.id)
                    .map(|r| r.is_empty())
                    .unwrap_or(true)
        });
    if isolated {
        return GapFinding {
            section,
            item,
            kind: GapKind::Resolution,
            detail: "symbol exists in the store but has zero graph relationships; resolution never connected it to a flow or component".into(),
        };
    }

    // 4. Compiled into a component? Then only rendering could drop it.
    if layers.components.contains(&n) {
        return if layers.text.contains(&n) {
            GapFinding {
                section,
                item,
                kind: GapKind::Alias,
                detail: "compiled into a component and present in the rendered atlas; the structured layer's spelling/aliases did not match".into(),
            }
        } else {
            GapFinding {
                section,
                item,
                kind: GapKind::Projection,
                detail: "compiled into a component but dropped from the rendered atlas by budget/policy/rendering".into(),
            }
        };
    }

    // 5. Reached a flow but never a component, or neither.
    if layers.flows.contains(&n) {
        return GapFinding {
            section,
            item,
            kind: GapKind::Compiler,
            detail: "reached a flow but was never compiled into a component".into(),
        };
    }
    if layers.text.contains(&n) {
        GapFinding {
            section,
            item,
            kind: GapKind::Alias,
            detail: "present in the rendered atlas text (e.g. README purpose or trust boundaries) under a spelling the structured layers do not carry".into(),
        }
    } else {
        GapFinding {
            section,
            item,
            kind: GapKind::Compiler,
            detail: "present in the store but not compiled into components or flows".into(),
        }
    }
}

/// If `item` names a repo file, return the language it would be scanned
/// with (language from the file registry when scanned; otherwise inferred
/// from the extension when the file exists on disk).
fn file_language(item: &str, repo_dir: &Path, store: &Store) -> Option<Language> {
    if !item.contains('/') {
        return None;
    }
    let rel = item.trim_start_matches("./");
    if let Ok(files) = store.all_files() {
        for (path, _hash, lang, _kind, _size) in files {
            if path == rel {
                return lang_from_str(&lang);
            }
        }
    }
    let full = repo_dir.join(rel);
    if full.is_file() {
        lang_from_ext(rel)
    } else {
        None
    }
}

fn lang_from_str(s: &str) -> Option<Language> {
    match s {
        "python" => Some(Language::Python),
        "typescript" | "javascript" => Some(Language::TypeScript),
        "go" => Some(Language::Go),
        "rust" => Some(Language::Rust),
        "java" => Some(Language::Java),
        _ => None,
    }
}

fn lang_from_ext(path: &str) -> Option<Language> {
    let ext = path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "py" | "pyi" => Some(Language::Python),
        "ts" | "tsx" | "mts" | "cts" => Some(Language::TypeScript),
        "js" | "jsx" | "mjs" | "cjs" => Some(Language::TypeScript),
        "go" => Some(Language::Go),
        "rs" => Some(Language::Rust),
        "java" => Some(Language::Java),
        _ => None,
    }
}

pub fn print_report(r: &AtlasRecallReport, diagnose: bool) {
    println!("scc bench atlas — startup-atlas recall vs independent ground truth (Wave 8 §57, v2)");
    println!("  mode: {}", r.mode);
    println!(
        "  gate: overall mean recall (architecture+entrypoints+behavior+state_authority+contracts) >= {ATLAS_GATE}"
    );
    println!(
        "  {:<24} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6} {:>8} {:>6} {:>6} {:>5}  note",
        "repo", "arch", "entry", "behav", "state", "contr", "landm", "tests", "overall",
        "prec", "f/1k", "toks"
    );
    for repo in &r.repos {
        match &repo.skipped_reason {
            Some(reason) => println!(
                "  {:<24} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6} {:>8} {:>6} {:>6} {:>5}  skipped: {reason}",
                repo.repo, "-", "-", "-", "-", "-", "-", "-", "-", "-", "-", "-"
            ),
            None => {
                let note = if repo.resolved_calls > 0 {
                    format!("resolved:{}", repo.resolved_calls)
                } else {
                    String::new()
                };
                println!(
                    "  {:<24} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>8.3} {:>6.3} {:>6.2} {:>5}  {}",
                    repo.repo,
                    repo.architecture,
                    repo.entrypoints,
                    repo.behavior,
                    repo.state_authority,
                    repo.contracts,
                    repo.landmarks,
                    repo.tests,
                    repo.overall,
                    repo.precision,
                    repo.density,
                    repo.atlas_tokens,
                    note
                );
                for m in &repo.missed {
                    println!("      missed: {m}");
                }
            }
        }
    }
    println!(
        "  {:<24} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>8.3} {:>6.3} {:>6.2} {:>5}",
        "mean",
        r.mean_architecture,
        r.mean_entrypoints,
        r.mean_behavior,
        r.mean_state_authority,
        r.mean_contracts,
        r.mean_landmarks,
        r.mean_tests,
        r.mean_overall,
        r.mean_precision,
        r.mean_density,
        r.mean_atlas_tokens.round() as usize
    );
    println!("  scored: {}   skipped: {}", r.scored, r.skipped);
    let resolved_total: usize = r.repos.iter().map(|repo| repo.resolved_calls).sum();
    if resolved_total > 0 {
        println!("  resolved calls (semantic backends upgraded): {resolved_total}");
    }
    println!(
        "  gate: {} (overall mean recall {:.3} >= {ATLAS_GATE})",
        if r.gate_passed { "PASS" } else { "FAIL" },
        r.mean_overall
    );

    if !diagnose {
        return;
    }
    println!("\ngap-kind histogram (all sections):");
    if r.gap_histogram.is_empty() {
        println!("  (none — no missed items)");
    }
    for (kind, count) in &r.gap_histogram {
        println!("  {kind:<12} {count}");
    }
    println!("\nPer-repo gap lines (regenerated into benchmarks/results/ground-truth-gaps.md):");
    for repo in &r.repos {
        if repo.gaps.is_empty() {
            continue;
        }
        println!("\n## {}", repo.repo);
        for g in &repo.gaps {
            println!("- `{}:{}` — {} GAP: {}", g.section, g.item, g.kind.as_str(), g.detail);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GT_MD: &str = r#"# synth repo
> synthetic | python | service

## architecture
- root — the app root component
- services — business logic

## entrypoints
- GET /api/items — fetch items

## behavior
- `handle_items` — entry handler

## state_authority
- db.items — owned by services

## contracts
- POST /api/items

## landmarks
- `ItemStore` — one zoom level deeper

## tests
- test_create_item — creation test
"#;

    #[test]
    fn parses_ground_truth_sections() {
        let doc = parse_ground_truth(GT_MD);
        assert_eq!(doc.architecture, ["root", "services"]);
        assert_eq!(doc.entrypoints, ["GET /api/items"]);
        assert_eq!(doc.behavior, ["handle_items"], "inline-code backticks stripped");
        assert_eq!(doc.state_authority, ["db.items"]);
        assert_eq!(doc.contracts, ["POST /api/items"]);
        assert_eq!(doc.landmarks, ["ItemStore"]);
        assert_eq!(doc.tests, ["test_create_item"]);
    }

    #[test]
    fn parse_ground_truth_accepts_legacy_section_names() {
        let md = "## components\n- root\n## flows\n- handle\n## ownership\n- db.x\n## entrypoints\n- e\n## contracts\n- c\n## tests\n- t\n";
        let doc = parse_ground_truth(md);
        assert_eq!(doc.architecture, ["root"], "components -> architecture");
        assert_eq!(doc.behavior, ["handle"], "flows -> behavior");
        assert_eq!(doc.state_authority, ["db.x"], "ownership -> state_authority");
        assert_eq!(doc.entrypoints, ["e"]);
        assert_eq!(doc.contracts, ["c"]);
        assert_eq!(doc.tests, ["t"]);
        assert!(doc.landmarks.is_empty());
    }

    #[test]
    fn parse_ground_truth_dedupes_keys_per_section() {
        let md = "## contracts\n- GET /api/items\n- GET /api/items — duplicate bullet\n- POST /api/items\n";
        let doc = parse_ground_truth(md);
        assert_eq!(doc.contracts, ["GET /api/items", "POST /api/items"]);
        // same key in two sections is not a duplicate
        let md2 = "## entrypoints\n- GET /api/items\n## contracts\n- GET /api/items\n";
        let doc2 = parse_ground_truth(md2);
        assert_eq!(doc2.entrypoints, ["GET /api/items"]);
        assert_eq!(doc2.contracts, ["GET /api/items"]);
    }

    #[test]
    fn norm_applies_documented_aliases() {
        assert_eq!(norm("Controller::run"), "controller.run");
        assert_eq!(norm("fn main"), "main");
        assert_eq!(norm("./src/index-client.js"), "src/index-client.js");
        assert_eq!(norm("ArgMatches"), "argmatches");
        assert_eq!(norm("GET /api/items"), "get /api/items");
    }

    #[test]
    fn recall_counts_all_hit_partial_and_zero() {
        // haystack already normalized: layer_recall applies norm() to items
        let hay = "root\nservices\nget /api/items\nhandle_items\ndb.items\npost /api/items\nitemstore\ntest_create_item";
        let doc = parse_ground_truth(GT_MD);
        let (a, _, _) = layer_recall(&doc.architecture, hay);
        assert_eq!(a, 1.0);
        let (e, _, _) = layer_recall(&doc.entrypoints, hay);
        assert_eq!(e, 1.0);
        let (b, _, _) = layer_recall(&doc.behavior, hay);
        assert_eq!(b, 1.0);
        let (s, _, _) = layer_recall(&doc.state_authority, hay);
        assert_eq!(s, 1.0);
        let (c, _, _) = layer_recall(&doc.contracts, hay);
        assert_eq!(c, 1.0);
        let (l, _, _) = layer_recall(&doc.landmarks, hay);
        assert_eq!(l, 1.0);
        let (t, _, _) = layer_recall(&doc.tests, hay);
        assert_eq!(t, 1.0);

        // zero: no ground-truth item is a substring of empty haystack
        let (z, _, _) = layer_recall(&doc.architecture, "");
        assert_eq!(z, 0.0);

        // partial: "root" hits, "services" does not
        let (p, hit, total) = layer_recall(&doc.architecture, "root only");
        assert_eq!((p, hit, total), (0.5, 1, 2));

        // empty ground truth scores 1.0 (nothing to miss)
        let empty = GroundTruthDoc::default();
        let (x, hit0, total0) = layer_recall(&empty.architecture, "anything");
        assert_eq!((x, hit0, total0), (1.0, 0, 0));
    }

    #[test]
    fn flow_edge_precision_counts_ordered_pairs_in_behavior_items() {
        // two graphs: g1 has a supported chain, g2 has an unsupported edge
        let g1 = FlowGraph {
            id: "g1".into(),
            kind: scc_core::FlowKind::Sequence,
            name: "g1".into(),
            trigger: None,
            nodes: vec![
                scc_core::FlowNode { id: 0, actor: "a".into(), operation: "worker consumer".into(), evidence: vec![] },
                scc_core::FlowNode { id: 1, actor: "b".into(), operation: "task.run".into(), evidence: vec![] },
                scc_core::FlowNode { id: 2, actor: "c".into(), operation: "task.retry".into(), evidence: vec![] },
            ],
            edges: vec![
                scc_core::FlowEdge { from: 0, to: 1, kind: scc_core::FlowEdgeKind::Next, condition: None, provenance: None, confidence: 1.0, evidence: vec![] },
                scc_core::FlowEdge { from: 1, to: 2, kind: scc_core::FlowEdgeKind::Next, condition: None, provenance: None, confidence: 1.0, evidence: vec![] },
            ],
            entrypoints: vec![0],
            exits: vec![2],
            provenance_summary: Default::default(),
        };
        // "worker consumer -> task.run" supports (0,1) but not (1,2)
        let behavior = vec!["worker consumer -> task.run".to_string()];
        let prec = flow_edge_precision(&[g1], &behavior);
        assert!((prec - 0.5).abs() < 1e-9, "1 of 2 edges supported: {prec}");

        // no behavior ground truth -> 1.0 (nothing to contradict)
        assert_eq!(flow_edge_precision(&[], &behavior), 1.0);
        let g_empty = FlowGraph {
            id: "g".into(),
            kind: scc_core::FlowKind::Sequence,
            name: "g".into(),
            trigger: None,
            nodes: vec![
                scc_core::FlowNode { id: 0, actor: "a".into(), operation: "x".into(), evidence: vec![] },
                scc_core::FlowNode { id: 1, actor: "b".into(), operation: "y".into(), evidence: vec![] },
            ],
            edges: vec![],
            entrypoints: vec![0],
            exits: vec![1],
            provenance_summary: Default::default(),
        };
        assert_eq!(flow_edge_precision(std::slice::from_ref(&g_empty), &behavior), 1.0, "no edges -> 1.0");
        assert_eq!(flow_edge_precision(&[g_empty], &[]), 1.0);
    }

    #[test]
    fn token_density_guards_zero_tokens() {
        assert_eq!(token_density(5, 0), 0.0);
        assert!((token_density(5, 5000) - 1.0).abs() < 1e-9, "5 facts / 5k tokens = 1 per 1k");
    }

    #[test]
    fn run_atlas_recall_skips_missing_dirs_without_panicking() {
        let tmp = tempfile::TempDir::new().unwrap();
        let corpus = tmp.path().join("corpus");
        let gt = tmp.path().join("ground-truth");
        std::fs::create_dir_all(&corpus).unwrap();
        std::fs::create_dir_all(&gt).unwrap();
        std::fs::create_dir_all(corpus.join("repo-a")).unwrap();

        let names = ["repo-b".to_string(), "repo-a".to_string()];
        let report = run_atlas_recall(&corpus, &gt, &names, false, false).unwrap();
        assert_eq!(report.scored, 0);
        assert_eq!(report.skipped, 2);
        assert!(!report.gate_passed);
        assert_eq!(report.mean_overall, 0.0);
        // deterministic order regardless of input order
        assert_eq!(report.repos[0].repo, "repo-a");
        assert!(report.repos[0]
            .skipped_reason
            .as_deref()
            .unwrap()
            .contains("ground truth missing"));
        assert_eq!(report.repos[1].repo, "repo-b");
        assert!(report.repos[1]
            .skipped_reason
            .as_deref()
            .unwrap()
            .contains("corpus dir missing"));
    }

    fn synth_repo(tmp: &tempfile::TempDir, gt_md: &str) {
        let corpus = tmp.path().join("corpus");
        let gt = tmp.path().join("ground-truth");
        std::fs::create_dir_all(&corpus).unwrap();
        std::fs::create_dir_all(&gt).unwrap();
        let repo_dir = corpus.join("synth");
        std::fs::create_dir_all(repo_dir.join("services")).unwrap();
        std::fs::write(
            repo_dir.join("main.py"),
            "from fastapi import FastAPI\nfrom services.items import ItemRepository\n\napp = FastAPI()\n\n\n@app.get(\"/api/items\")\ndef handle_items() -> list:\n    \"\"\"List all items.\"\"\"\n    repo = ItemRepository()\n    return repo.find_all()\n",
        )
        .unwrap();
        std::fs::write(
            repo_dir.join("services/items.py"),
            "class ItemRepository:\n    def find_all(self):\n        return []\n",
        )
        .unwrap();
        std::fs::write(gt.join("synth.md"), gt_md).unwrap();
    }

    #[test]
    fn run_atlas_recall_scores_synthetic_repo_structurally() {
        let tmp = tempfile::TempDir::new().unwrap();
        synth_repo(
            &tmp,
            "## architecture\n- root\n- services\n## entrypoints\n- handle_items\n## behavior\n- ItemRepository\n## state_authority\n- zzz_nonexistent_store\n## contracts\n- GET /api/items\n- GET /api/zzz_nonexistent\n## landmarks\n- ItemRepository\n## tests\n- test_nonexistent\n",
        );

        let report =
            run_atlas_recall(&tmp.path().join("corpus"), &tmp.path().join("ground-truth"), &["synth".to_string()], false, false)
                .unwrap();
        assert_eq!(report.scored, 1);
        assert_eq!(report.skipped, 0);
        assert!(report.gate_passed == (report.mean_overall >= ATLAS_GATE));
        let r = &report.repos[0];
        assert!(r.skipped_reason.is_none());
        // without the semantic pass the repo reports zero resolved calls
        assert_eq!(r.resolved_calls, 0);
        // overall is the equal-weighted mean of the five startup layers
        let expect = (r.architecture + r.entrypoints + r.behavior + r.state_authority + r.contracts) / 5.0;
        assert!((r.overall - expect).abs() < 1e-9);
        // the deliberately nonexistent items must be missed
        assert_eq!(r.state_authority, 0.0);
        assert_eq!(r.contracts, 0.5, "GET /api/items hits, zzz misses");
        assert!(r.missed.contains(&"state_authority:zzz_nonexistent_store".to_string()));
        assert!(r.missed.contains(&"contracts:GET /api/zzz_nonexistent".to_string()));
        assert!(!r.missed.contains(&"entrypoints:handle_items".to_string()));
        // v2 metrics are finite and in range
        assert!((0.0..=1.0).contains(&r.precision));
        assert!(r.density >= 0.0);
        assert!(r.atlas_tokens > 0, "rendered atlas has tokens");
        assert_eq!(r.landmark_items, 1);
    }

    #[test]
    fn diagnose_classifies_missed_items_deterministically() {
        let tmp = tempfile::TempDir::new().unwrap();
        synth_repo(
            &tmp,
            "## architecture\n- root\n## state_authority\n- zzz_nonexistent_store\n## contracts\n- GET /api/zzz_nonexistent\n## tests\n- test_nonexistent\n",
        );

        let report =
            run_atlas_recall(&tmp.path().join("corpus"), &tmp.path().join("ground-truth"), &["synth".to_string()], true, false)
                .unwrap();
        let r = &report.repos[0];
        assert!(!r.gaps.is_empty(), "diagnose produced gap findings");
        // nothing in the store, not a repo file -> EXTRACTOR
        for g in &r.gaps {
            assert_eq!(g.kind, GapKind::Extractor, "{:?}", g);
            assert!(!g.detail.is_empty());
        }
        // histogram covers exactly the diagnosed items
        let total: usize = report.gap_histogram.values().sum();
        assert_eq!(total, r.gaps.len());
    }

    #[test]
    fn fallback_ground_truth_maps_tasks_and_dedupes() {
        let task = serde_json::from_str::<BenchTask>(
            r#"{"id":"t1","repo":"r","goal":"g",
               "ground_truth":{"files":["a.py"],"symbols":["s1","s2"],"components":["root"],
                               "data":["db.x"],"routes":["GET /api/x"],"stores":["redis"],
                               "tests":["test_a"]},
               "hallucinations":[]}"#,
        )
        .unwrap();
        let doc = ground_truth_from_tasks(&[task.clone(), task], "r");
        assert_eq!(doc.architecture, ["root"]);
        assert_eq!(doc.entrypoints, ["GET /api/x"]);
        assert_eq!(doc.contracts, ["GET /api/x"]);
        assert_eq!(doc.behavior, ["s1", "s2"]);
        assert_eq!(doc.state_authority, ["redis", "db.x"]);
        assert_eq!(doc.tests, ["test_a"]);
    }

    #[test]
    fn holdout_verdict_matches_gap_bands() {
        // holdout >= dev -> NO OVERFIT
        assert_eq!(holdout_verdict(0.20, 0.25), HoldoutVerdict::NoOverfit);
        assert_eq!(holdout_verdict(0.20, 0.20), HoldoutVerdict::NoOverfit);
        // lag inside the tolerance band -> BORDERLINE
        assert_eq!(holdout_verdict(0.20, 0.17), HoldoutVerdict::Borderline);
        assert_eq!(holdout_verdict(0.20, 0.20 - 0.05 + 1e-9), HoldoutVerdict::Borderline);
        // lag beyond the tolerance band -> OVERFIT
        assert_eq!(holdout_verdict(0.20, 0.14), HoldoutVerdict::Overfit);
        assert_eq!(holdout_verdict(0.20, 0.0), HoldoutVerdict::Overfit);
        assert_eq!(holdout_verdict(0.20, 0.20 - 0.05 - 1e-9), HoldoutVerdict::Overfit);
    }

    #[test]
    fn layer_gap_is_clamped_and_signed() {
        assert!((HoldoutComparison::layer_gap(0.1, 0.3) - 0.2).abs() < 1e-9);
        assert!((HoldoutComparison::layer_gap(0.3, 0.1) - (-0.2)).abs() < 1e-9);
        assert_eq!(HoldoutComparison::layer_gap(0.0, 2.0), 1.0);
        assert_eq!(HoldoutComparison::layer_gap(2.0, 0.0), -1.0);
        assert_eq!(HoldoutComparison::layer_gap(0.5, 0.5), 0.0);
    }

    #[test]
    fn holdout_results_text_is_deterministic_and_contains_gap() {
        let dev = AtlasRecallReport {
            mean_architecture: 0.3,
            ..Default::default()
        };
        let mut dev = dev;
        dev.mean_entrypoints = 0.2;
        dev.mean_behavior = 0.4;
        dev.mean_state_authority = 0.1;
        dev.mean_contracts = 0.5;
        dev.mean_overall = 0.3;
        dev.scored = 20;
        let mut holdout = dev.clone();
        holdout.mean_overall = 0.26; // lag 0.04 -> BORDERLINE (inside 0.05 band)
        holdout.mean_contracts = 0.45;

        let c = HoldoutComparison {
            gap_architecture: HoldoutComparison::layer_gap(dev.mean_architecture, holdout.mean_architecture),
            gap_entrypoints: HoldoutComparison::layer_gap(dev.mean_entrypoints, holdout.mean_entrypoints),
            gap_behavior: HoldoutComparison::layer_gap(dev.mean_behavior, holdout.mean_behavior),
            gap_state_authority: HoldoutComparison::layer_gap(
                dev.mean_state_authority,
                holdout.mean_state_authority,
            ),
            gap_contracts: HoldoutComparison::layer_gap(dev.mean_contracts, holdout.mean_contracts),
            gap_overall: HoldoutComparison::layer_gap(dev.mean_overall, holdout.mean_overall),
            verdict: holdout_verdict(dev.mean_overall, holdout.mean_overall),
            results_file: "benchmarks/results/holdout-v2.txt".to_string(),
            dev,
            holdout,
        };
        let text = c.to_results_text();
        let text2 = c.to_results_text();
        assert_eq!(text, text2, "deterministic output");
        assert!(text.contains("overall (gate)"));
        assert!(text.contains("verdict: BORDERLINE"));
        assert!(text.contains("-0.040"), "gap -0.04 rendered: {text}");
    }
}



