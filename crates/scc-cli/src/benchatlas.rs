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
//! v3 metrics:
//! - `precision` (startup_required_precision): per startup-required layer,
//!   |atlas entries in that layer that match a ground-truth item| /
//!   |atlas entries in that layer| — a too-much-architecture detector: an
//!   atlas bloated with facts the ground truth never describes scores low.
//!   `RepoRecall.precision` is the equal-weighted mean over the five
//!   startup-required layers.
//! - `F2` per layer: (5 * P * R) / (4 * P + R) from that layer's precision
//!   P and recall R (zero when P + R == 0); the gate still uses recall.
//! - `density` (architecture_density): matched startup-required items per
//!   1000 atlas tokens; `atlas_tokens` per repo is reported too.
//!
//! The v1/v2 `--holdout` protocol is now labelled **development** vs
//! **validation** (the "holdout" corpus has been inspected and tuned
//! against — calling it blind would be dishonest; the on-disk dirs
//! `benchmarks/holdout` stay as they are).
//!
//! `--blind` scores the NEW frozen corpus (`benchmarks/blind-test` +
//! `benchmarks/blind-test-ground-truth`), never used by tuning, and prints
//! ONLY aggregates (overall, per-section means, the validation-vs-blind
//! generalization gap, precision, density) — no per-repo rows, no missed
//! keys, no filenames. blind-test failures are never shown to tuning agents.
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
use scc_core::Entity;
use scc_core::SystemAtlas;
use scc_indexer::scan::Language;
use scc_store::Store;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Quality gate: overall mean recall must be >= this floor (Wave 8 §57).
/// The floor is over the five startup-required layers ONLY.
pub const ATLAS_GATE: f64 = 0.5;

/// Holdout verdict tolerance: the validation corpus may lag the development
/// corpus by up to this much (overall recall, absolute) before the run is
/// called OVERFIT. The band absorbs corpus-difficulty, LOC-mix, and
/// ground-truth-strictness differences; a lag beyond it means the
/// development-tuned rules do not generalize to unseen repos.
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

/// Default per-section regression guard (`--guard-section-delta`): any
/// startup-required section dropping by more than this between two compared
/// runs fails the Wave-11 guard.
pub const DEFAULT_SECTION_GUARD: f64 = 0.05;

/// Minimal pure-Rust SHA-256 (FIPS 180-4) for the blind-test manifest hash.
/// Deterministic, dependency-free (scc-cli has no crypto dep), panic-free.
/// Public only so the roundtrip unit test can exercise it directly.
pub mod sha256 {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    const H0: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    /// SHA-256 digest of `data` as 32 raw bytes.
    pub fn digest(data: &[u8]) -> [u8; 32] {
        let mut h = H0;
        let bit_len: u64 = (data.len() as u64).wrapping_mul(8);
        // pad: 0x80, zeros to 56 mod 64, then the 64-bit big-endian bit length
        let mut buf: Vec<u8> = Vec::with_capacity(((data.len() + 72) / 64) * 64);
        buf.extend_from_slice(data);
        buf.push(0x80);
        while buf.len() % 64 != 56 {
            buf.push(0);
        }
        buf.extend_from_slice(&bit_len.to_be_bytes());

        let mut w = [0u32; 64];
        for chunk in buf.chunks_exact(64) {
            for (i, word) in w.iter_mut().enumerate().take(16) {
                let o = i * 4;
                *word = u32::from_be_bytes([
                    chunk[o],
                    chunk[o + 1],
                    chunk[o + 2],
                    chunk[o + 3],
                ]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }
            let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ (!e & g);
                let t1 = hh
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[i])
                    .wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let t2 = s0.wrapping_add(maj);
                hh = g;
                g = f;
                f = e;
                e = d.wrapping_add(t1);
                d = c;
                c = b;
                b = a;
                a = t1.wrapping_add(t2);
            }
            h[0] = h[0].wrapping_add(a);
            h[1] = h[1].wrapping_add(b);
            h[2] = h[2].wrapping_add(c);
            h[3] = h[3].wrapping_add(d);
            h[4] = h[4].wrapping_add(e);
            h[5] = h[5].wrapping_add(f);
            h[6] = h[6].wrapping_add(g);
            h[7] = h[7].wrapping_add(hh);
        }
        let mut out = [0u8; 32];
        for (i, v) in h.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
        }
        out
    }

    /// Lowercase hex of the SHA-256 digest.
    pub fn hex(data: &[u8]) -> String {
        let mut out = String::with_capacity(64);
        for b in digest(data) {
            out.push_str(&format!("{b:02x}"));
        }
        out
    }
}

// trace:exempt reason=internal-detail

/// Ground-truth sections parsed from `benchmarks/ground-truth/<name>.md`
/// (one `- <key string>` bullet per item). The v2 ontology; legacy section
/// names (components/flows/ownership) are accepted and normalized.
#[derive(Debug, Clone, Default)]
// trace:exempt reason=internal-detail
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

// trace:exempt reason=internal-detail

/// Gap-kind classification for a missed ground-truth item (`--diagnose`):
/// where the fact disappeared between source and the rendered atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
// trace:exempt reason=internal-detail
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapFinding {
    pub section: String,
    pub item: String,
    pub kind: GapKind,
    pub detail: String,
}

// trace:exempt reason=internal-detail

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
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
    /// Mean startup-required precision over the five startup-required
    /// layers: per layer, |atlas entries that match a ground-truth item| /
    /// |atlas entries in the layer| (v3 — a too-much-architecture
    /// detector; see `layer_precision`).
    pub precision: f64,
    /// Per-layer startup-required precision (the five startup layers only).
    pub layer_precision: BTreeMap<String, f64>,
    /// Per-layer F2 = (5*P*R)/(4*P+R) over the five startup-required
    /// layers (zero when P+R==0); `f2` is the equal-weighted mean.
    pub layer_f2: BTreeMap<String, f64>,
    /// Equal-weighted mean F2 over the five startup-required layers.
    pub f2: f64,
    /// Matched startup-required items per 1000 atlas tokens
    /// (architecture_density).
    pub density: f64,
    /// Rendered atlas token count.
    pub atlas_tokens: usize,
    /// Number of call edges upgraded to RESOLVED by the semantic backends
    /// (pyright/tsserver) before scoring; 0 when `--no-resolve`.
    pub resolved_calls: usize,
    /// Semantic backends that were available and used for resolution
    /// (pyright/tsserver); empty when unavailable or `--no-resolve`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backends_used: Vec<String>,
    /// Semantic backends unavailable (not installed) — resolution degraded,
    /// not fatal.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backends_missing: Vec<String>,
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

// trace:exempt reason=internal-detail
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
            layer_precision: BTreeMap::new(),
            layer_f2: BTreeMap::new(),
            f2: 0.0,
            density: 0.0,
            atlas_tokens: 0,
            resolved_calls: 0,
            backends_used: Vec::new(),
            backends_missing: Vec::new(),
            landmark_items: 0,
            skipped_reason: None,
            missed: Vec::new(),
            gaps: Vec::new(),
        }
    }
}

// trace:exempt reason=internal-detail

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
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
    pub mean_f2: f64,
    pub mean_density: f64,
    pub mean_atlas_tokens: f64,
    pub scored: usize,
    pub skipped: usize,
    pub gate_passed: bool,
    /// Gap-kind histogram over all diagnosed items (kind -> count).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub gap_histogram: BTreeMap<String, usize>,
}

impl AtlasRecallReport {
    /// Clone with all per-repo detail stripped (repos, gaps, histogram):
    /// only the aggregates survive. The blind protocol keeps this invariant
    /// end to end — blind-test failures are never shown to tuning agents,
    /// and the blind JSON / blind-v1.txt output is aggregates-only.
    pub fn aggregates_only(&self) -> Self {
        let mut c = self.clone();
        c.repos.clear();
        c.gap_histogram.clear();
        c
    }

    /// Mean per-layer precision over scored repos (the five startup layers).
    fn mean_layer_precision(&self) -> BTreeMap<String, f64> {
        self.mean_layer_map(|r| &r.layer_precision)
    }

    /// Mean per-layer F2 over scored repos (the five startup layers).
    fn mean_layer_f2(&self) -> BTreeMap<String, f64> {
        self.mean_layer_map(|r| &r.layer_f2)
    }

    fn mean_layer_map(
        &self,
        pick: impl Fn(&RepoRecall) -> &BTreeMap<String, f64>,
    ) -> BTreeMap<String, f64> {
        let mut sums: BTreeMap<String, f64> = BTreeMap::new();
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for r in &self.repos {
            if r.skipped_reason.is_some() {
                continue;
            }
            for (layer, v) in pick(r) {
                *sums.entry(layer.clone()).or_insert(0.0) += v;
                *counts.entry(layer.clone()).or_insert(0) += 1;
            }
        }
        sums.into_iter()
            .map(|(layer, sum)| {
                let n = counts.get(&layer).copied().unwrap_or(0).max(1) as f64;
                (layer, sum / n)
            })
            .collect()
    }
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

fn build_layers(
    ctx: &ContextCompiler<'_>,
    pack: &scc_context::ContextPack,
    atlas: &SystemAtlas,
) -> AtlasLayers {
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
/// Whether a ground-truth chain item (`A -> B -> C`) matches the layer's
/// haystack: each step must appear — in order — in the per-step lines.
/// Chain items never match a plain substring test (the haystack is one
/// step per line), so this is the honest interpretation of a chain.
fn chain_matches(chain: &str, haystack: &str) -> bool {
    let steps: Vec<String> = chain.split(" -> ").map(norm).collect();
    if steps.len() < 2 {
        return false;
    }
    let lines: Vec<&str> = haystack.lines().collect();
    let mut pos = 0usize;
    for step in steps {
        let mut found = false;
        while pos < lines.len() {
            if norm(lines[pos]).contains(&step) {
                found = true;
                pos += 1;
                break;
            }
            pos += 1;
        }
        if !found {
            return false;
        }
    }
    true
}

fn item_matches(item: &str, haystack: &str) -> bool {
    if item.contains(" -> ") {
        chain_matches(item, haystack)
    } else {
        haystack.contains(&norm(item))
    }
}

fn layer_recall(items: &[String], haystack: &str) -> (f64, usize, usize) {
    if items.is_empty() {
        return (1.0, 0, 0);
    }
    let mut hit = 0usize;
    for item in items {
        if item_matches(item, haystack) {
            hit += 1;
        }
    }
    (hit as f64 / items.len() as f64, hit, items.len())
}

/// Whether one ground-truth item matches its layer's structured haystack.
fn item_matched(section: &str, item: &str, layers: &AtlasLayers, text_norm: &str) -> bool {
    item_matches(item, layer_haystack(section, layers, text_norm))
}

/// Startup-required precision for one layer: |atlas entries in the layer
/// that match a ground-truth item| / |atlas entries in the layer|. The
/// haystack is one normalized entry per line, so entries are countable.
/// An entry matches when some ground-truth item (chain items included,
/// against a single line they virtually never match) is contained in it.
/// An empty layer haystack scores 1.0 (nothing spurious to report).
fn layer_precision(items: &[String], haystack: &str) -> f64 {
    let entries: Vec<&str> = haystack.lines().filter(|l| !l.is_empty()).collect();
    if entries.is_empty() {
        return 1.0;
    }
    let matched = entries
        .iter()
        .filter(|line| items.iter().any(|item| item_matches(item, line)))
        .count();
    matched as f64 / entries.len() as f64
}

/// F2 score from precision P and recall R: (5*P*R)/(4*P+R), zero when
/// P + R == 0. Recall-weighting (beta=2) rewards recall over precision,
/// matching the gate's recall-first stance while still penalizing bloat.
fn f2_score(p: f64, r: f64) -> f64 {
    if p + r == 0.0 {
        return 0.0;
    }
    (5.0 * p * r) / (4.0 * p + r)
}

/// Startup facts per 1000 atlas tokens (architecture_density): matched
/// startup-required items over the rendered pack's token count.
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
    let mut backends_used: Vec<String> = Vec::new();
    let mut backends_missing: Vec<String> = Vec::new();
    let resolved_calls = if resolve {
        match crate::resolve_and_recompile(repo_dir) {
            Ok(rep) => {
                backends_used = rep.backends_used;
                backends_missing = rep.backends_missing;
                rep.upgraded
            }
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
    // Wave 11: build the machine atlas once, order its sections by
    // evidence-backed confidence (highest first — precision via ordering,
    // no entries dropped), and render the agent-facing pack from the ranked
    // atlas so agents read the strongest facts first under the token budget.
    let mut atlas = atlas::build_atlas(&comp.ctx());
    scc_context::rank::rank_startup_atlas(&mut atlas);
    let pack = atlas::render_atlas(&comp.ctx(), &atlas, comp.ctx().settings.atlas_tokens);
    let layers = build_layers(&comp.ctx(), &pack, &atlas);
    let text_norm = &layers.text;

    let (architecture, arch_hit, _) = layer_recall(&gt.architecture, &layers.architecture);
    let (entrypoints, ep_hit, _) = layer_recall(&gt.entrypoints, &layers.entrypoints);
    let (behavior, bh_hit, _) = layer_recall(&gt.behavior, &layers.behavior);
    let (state_authority, sa_hit, _) = layer_recall(&gt.state_authority, &layers.state_authority);
    let (contracts, ct_hit, _) = layer_recall(&gt.contracts, &layers.contracts);
    let (landmarks, _, _) = layer_recall(&gt.landmarks, &layers.landmarks);
    let (tests, _, _) = layer_recall(&gt.tests, text_norm);
    let overall = (architecture + entrypoints + behavior + state_authority + contracts) / 5.0;

    // v3 startup-required precision + F2 per layer (the five startup layers
    // only — landmarks/tests are informational and excluded, mirroring the
    // recall gate's anti-bloat stance).
    let startup_layers = [
        ("architecture", architecture),
        ("entrypoints", entrypoints),
        ("behavior", behavior),
        ("state_authority", state_authority),
        ("contracts", contracts),
    ];
    let mut layer_precision_map: BTreeMap<String, f64> = BTreeMap::new();
    let mut layer_f2: BTreeMap<String, f64> = BTreeMap::new();
    let mut precision_sum = 0.0;
    let mut f2_sum = 0.0;
    for (name, recall) in startup_layers {
        let p = layer_precision(gt.section(name), layer_haystack(name, &layers, text_norm));
        let f2 = f2_score(p, recall);
        layer_precision_map.insert(name.to_string(), p);
        layer_f2.insert(name.to_string(), f2);
        precision_sum += p;
        f2_sum += f2;
    }
    let precision = precision_sum / startup_layers.len() as f64;
    let f2 = f2_sum / startup_layers.len() as f64;

    let matched_startup = arch_hit + ep_hit + bh_hit + sa_hit + ct_hit;
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
        layer_precision: layer_precision_map,
        layer_f2,
        f2,
        density,
        atlas_tokens: pack.tokens,
        resolved_calls,
        backends_used,
        backends_missing,
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
                report.mean_f2 += r.f2;
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
        report.mean_f2 /= n;
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
// trace:v1 id=impl.scc.bench.atlas work=WORK-SCC-003 verifies=REQ-SCC-TEST

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

// trace:exempt reason=internal-detail

/// Overfit verdict over the dev-vs-holdout overall gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
// trace:exempt reason=internal-detail
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

// trace:exempt reason=internal-detail

/// Dev-vs-holdout comparison for `scc bench atlas --holdout`.
#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct HoldoutComparison {
    pub dev: AtlasRecallReport,
    /// The validation corpus report (`benchmarks/holdout` — the inspected
    /// corpus that tuning has seen; the on-disk dir name is kept, only the
    /// output labels say "validation").
    pub holdout: AtlasRecallReport,
    /// Per-layer gap = validation mean - development mean (fraction,
    /// negative = lag).
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

// trace:exempt reason=internal-detail
impl HoldoutComparison {
    fn layer_gap(dev: f64, holdout: f64) -> f64 {
        (holdout - dev).clamp(-1.0, 1.0)
    }
}

/// Run the holdout protocol: score the development corpus and the
/// validation corpus with the same recall pipeline, compute per-layer gaps,
/// write `benchmarks/results/holdout-v3.txt`, and return the comparison.
///
/// `corpus`/`ground_truth` (when given) select the DEVELOPMENT corpus,
/// exactly as in `run_atlas_bench` (defaults: `benchmarks/corpus` +
/// `benchmarks/ground-truth`). The validation dirs are fixed protocol
/// paths: `benchmarks/holdout` + `benchmarks/holdout-ground-truth` (the
/// on-disk names are kept from v1; only the output labels say validation).
/// The validation corpus must exist — a missing dir is an error, not a
/// silent empty run.
///
/// `resolve` applies to BOTH corpora (the same pipeline must score
/// development and validation identically).
pub fn run_atlas_holdout(
    corpus: Option<&Path>,
    ground_truth: Option<&Path>,
    diagnose: bool,
    resolve: bool,
) -> Result<HoldoutComparison, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let root = crate::find_root(&cwd);
    let results_dir = root.join("benchmarks").join("results");
    let results_file = results_dir.join("holdout-v3.txt");

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
    holdout.mode = format!("validation: {}", holdout_corpus.display());

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
    /// Deterministic markdown text for `benchmarks/results/holdout-v3.txt`.
    fn to_results_text(&self) -> String {
        let mut out = String::new();
        out.push_str("# Holdout v3 — development corpus vs validation corpus\n");
        out.push_str(&format!("development corpus: {}\n", self.dev.mode));
        out.push_str(&format!("validation corpus:  {}\n", self.holdout.mode));
        out.push_str(&format!(
            "results:            {}\n",
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
            "{:<18} {:>12} {:>12} {:>10}\n",
            "layer", "development", "validation", "gap"
        ));
        for (layer, dev, ho) in rows {
            let gap = HoldoutComparison::layer_gap(dev, ho);
            out.push_str(&format!(
                "{:<18} {:>12.3} {:>12.3} {:>+10.3}\n",
                layer, dev, ho, gap
            ));
        }
        out.push('\n');
        out.push_str(&format!(
            "scored: development {} (skipped {}) | validation {} (skipped {})\n",
            self.dev.scored, self.dev.skipped, self.holdout.scored, self.holdout.skipped
        ));
        out.push_str(&format!(
            "precision: development {:.3} | validation {:.3}\n",
            self.dev.mean_precision, self.holdout.mean_precision
        ));
        out.push_str(&format!(
            "F2: development {:.3} | validation {:.3}\n",
            self.dev.mean_f2, self.holdout.mean_f2
        ));
        let dev_resolved: usize = self.dev.repos.iter().map(|r| r.resolved_calls).sum();
        let holdout_resolved: usize = self.holdout.repos.iter().map(|r| r.resolved_calls).sum();
        out.push_str(&format!(
            "resolved calls (upgraded): development {dev_resolved} | validation {holdout_resolved}\n"
        ));
        out.push_str(&format!(
            "density (facts/1k tokens): development {:.2} | validation {:.2}\n",
            self.dev.mean_density, self.holdout.mean_density
        ));
        out.push_str(&format!(
            "atlas tokens: development {:.0} | validation {:.0}\n",
            self.dev.mean_atlas_tokens, self.holdout.mean_atlas_tokens
        ));
        out.push('\n');
        out.push_str("## per-layer precision + F2 (startup-required layers)\n");
        out.push_str(&format!(
            "{:<18} {:>12} {:>12}   {:>12} {:>12}\n",
            "layer", "P:development", "P:validation", "F2:development", "F2:validation"
        ));
        for (layer, _, _) in rows {
            let p_dev = self.dev.mean_layer_precision().get(layer).copied().unwrap_or(0.0);
            let p_ho = self.holdout.mean_layer_precision().get(layer).copied().unwrap_or(0.0);
            let f_dev = self.dev.mean_layer_f2().get(layer).copied().unwrap_or(0.0);
            let f_ho = self.holdout.mean_layer_f2().get(layer).copied().unwrap_or(0.0);
            out.push_str(&format!(
                "{:<18} {:>12.3} {:>12.3}   {:>12.3} {:>12.3}\n",
                layer, p_dev, p_ho, f_dev, f_ho
            ));
        }
        out.push('\n');
        out.push_str(&format!(
            "## verdict: {} (gap = {:.3}; tolerance = {:.3})\n",
            self.verdict.as_str(),
            self.gap_overall,
            HOLDOUT_TOLERANCE
        ));
        match self.verdict {
            HoldoutVerdict::NoOverfit => out.push_str(
                "The validation corpus scores at least as well as the development \
                 corpus; the development-tuned rules generalize to unseen repos.\n",
            ),
            HoldoutVerdict::Borderline => out.push_str(
                "The validation corpus lags the development corpus, but by less than \
                 the tolerance band; the gap is consistent with \
                 corpus-difficulty/ground-truth noise, not demonstrated overfitting.\n",
            ),
            HoldoutVerdict::Overfit => out.push_str(
                "The validation corpus lags the development corpus by more than the \
                 tolerance band; rules tuned on the development corpus do not \
                 generalize to unseen repos.\n",
            ),
        }
        out.push('\n');
        out.push_str("## validation repo overall recall (sorted)\n");
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

/// Print the development and validation reports side by side plus the gap
/// summary.
pub fn print_holdout_report(c: &HoldoutComparison, diagnose: bool) {
    println!("scc bench atlas --holdout — development corpus vs validation corpus (v1)");
    println!("\n=== DEVELOPMENT corpus ===");
    print_report(&c.dev, diagnose);
    println!("\n=== VALIDATION corpus ===");
    print_report(&c.holdout, diagnose);
    println!("\n=== gap (validation - development) ===");
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
        "  verdict: {} (validation {:.3} vs development {:.3}; tolerance {:.3})",
        c.verdict.as_str(),
        c.holdout.mean_overall,
        c.dev.mean_overall,
        HOLDOUT_TOLERANCE
    );
    println!(
        "  F2: development {:.3} | validation {:.3}",
        c.dev.mean_f2, c.holdout.mean_f2
    );
    println!("  results written to: {}", c.results_file);
}

// ---------------------------------------------------------------------------
// Wave 11 — GENERALIZATION II gates (--compare OLD NEW)
// ---------------------------------------------------------------------------

/// Generalization efficiency (GE): how much of the development improvement
/// between two runs transferred to validation.
///
/// `GE = validation_delta / development_delta` over the overall recall
/// means. A positive GE means validation moved the same direction as
/// development (semantic waves generalize); negative GE means validation
/// regressed while development improved (overfit). When development did not
/// move (`dev_delta == 0`) the ratio is degenerate: a validation-only
/// improvement counts as pure generalization (`1.0`), anything else as
/// `0.0` — never NaN/inf.
pub fn generalization_efficiency(dev_delta: f64, validation_delta: f64) -> f64 {
    if dev_delta == 0.0 {
        return if validation_delta > 0.0 { 1.0 } else { 0.0 };
    }
    validation_delta / dev_delta
}

// trace:exempt reason=internal-detail

/// Wave-11 gate report over two saved holdout result files (JSON
/// `HoldoutComparison`s): the GE gate (`--gate-ge MIN`, default 0.0 — fails
/// when `GE <= MIN`; semantic waves must generalize) and the per-section
/// validation regression guard (`--guard-section-delta MAX`, default 0.05 —
/// fails when ANY startup-required section regresses by more than MAX
/// between the two runs, in development or validation).
#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct CompareReport {
    pub old_file: String,
    pub new_file: String,
    /// new.dev.mean_overall - old.dev.mean_overall.
    pub dev_delta_overall: f64,
    /// new.holdout.mean_overall - old.holdout.mean_overall.
    pub validation_delta_overall: f64,
    /// validation_delta_overall / dev_delta_overall (guarded).
    pub generalization_efficiency: f64,
    /// Per-section delta = new - old, development corpus.
    pub dev_deltas: BTreeMap<String, f64>,
    /// Per-section delta = new - old, validation corpus.
    pub validation_deltas: BTreeMap<String, f64>,
    /// Largest startup-required section drop (new - old) across both
    /// corpora; 0.0 when nothing regressed.
    pub max_section_regression: f64,
    /// The section with the largest drop (`section@corpus`), e.g.
    /// `contracts@validation`; deterministic (first at the max in
    /// `STARTUP_SECTIONS` order, development before validation).
    pub max_regression_section: String,
    pub gate_ge: f64,
    pub guard_section_delta: f64,
    pub ge_passed: bool,
    pub guard_passed: bool,
    /// Human-readable failure reasons; empty when the report passes.
    pub failures: Vec<String>,
}

/// The five startup-required sections the regression guard watches.
pub const STARTUP_SECTIONS: [&str; 5] = [
    "architecture",
    "entrypoints",
    "behavior",
    "state_authority",
    "contracts",
];

impl CompareReport {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Compare two saved holdout result files (JSON `HoldoutComparison`s, e.g.
/// `scc bench atlas --holdout --json` output) and apply the Wave-11 gates.
/// `old` is the earlier run (the pre-wave baseline), `new` the current one;
/// deltas are new - old.
pub fn compare_runs(
    old: &HoldoutComparison,
    new: &HoldoutComparison,
    gate_ge: f64,
    guard_section_delta: f64,
) -> CompareReport {
    let dev_delta_overall = new.dev.mean_overall - old.dev.mean_overall;
    let validation_delta_overall = new.holdout.mean_overall - old.holdout.mean_overall;
    let ge = generalization_efficiency(dev_delta_overall, validation_delta_overall);

    let section = |r: &AtlasRecallReport, name: &str| -> f64 {
        match name {
            "architecture" => r.mean_architecture,
            "entrypoints" => r.mean_entrypoints,
            "behavior" => r.mean_behavior,
            "state_authority" => r.mean_state_authority,
            "contracts" => r.mean_contracts,
            _ => 0.0,
        }
    };
    let mut dev_deltas: BTreeMap<String, f64> = BTreeMap::new();
    let mut validation_deltas: BTreeMap<String, f64> = BTreeMap::new();
    let mut max_section_regression: f64 = 0.0;
    let mut max_regression_section = String::new();
    for name in STARTUP_SECTIONS {
        let d_dev = section(&new.dev, name) - section(&old.dev, name);
        let d_val = section(&new.holdout, name) - section(&old.holdout, name);
        dev_deltas.insert(name.to_string(), d_dev);
        validation_deltas.insert(name.to_string(), d_val);
        for (d, corpus) in [(d_dev, "development"), (d_val, "validation")] {
            if -d > max_section_regression {
                max_section_regression = -d;
                max_regression_section = format!("{name}@{corpus}");
            }
        }
    }

    let ge_passed = ge > gate_ge;
    let guard_passed = max_section_regression <= guard_section_delta;
    let mut failures: Vec<String> = Vec::new();
    if !ge_passed {
        failures.push(format!(
            "generalization efficiency {ge:.3} <= gate {gate_ge:.3} \
             (validation delta {validation_delta_overall:+.3} vs development delta {dev_delta_overall:+.3}): \
             the semantic wave did not generalize"
        ));
    }
    if !guard_passed {
        failures.push(format!(
            "startup-required section regressed by {max_section_regression:.3} > guard {guard_section_delta:.3} \
             (worst: {max_regression_section}; new vs old run, development or validation)"
        ));
    }

    CompareReport {
        old_file: String::new(),
        new_file: String::new(),
        dev_delta_overall,
        validation_delta_overall,
        generalization_efficiency: ge,
        dev_deltas,
        validation_deltas,
        max_section_regression,
        max_regression_section,
        gate_ge,
        guard_section_delta,
        ge_passed,
        guard_passed,
        failures,
    }
}

/// Load a saved holdout JSON result file into a `HoldoutComparison`.
pub fn load_holdout_result(path: &Path) -> Result<HoldoutComparison, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|e| format!("cannot parse {} as a holdout JSON result: {e}", path.display()))
}

/// Print the Wave-11 compare report (deltas, GE, per-section guard).
pub fn print_compare_report(r: &CompareReport) {
    println!("scc bench atlas --compare — Wave-11 generalization gates");
    println!("  old: {}", r.old_file);
    println!("  new: {}", r.new_file);
    println!(
        "  development delta:  {:+.3} (new - old, overall)",
        r.dev_delta_overall
    );
    println!(
        "  validation delta:   {:+.3} (new - old, overall)",
        r.validation_delta_overall
    );
    println!(
        "  generalization efficiency: {:.3} (validation delta / development delta)",
        r.generalization_efficiency
    );
    println!(
        "  GE gate (--gate-ge {:.3}): {}",
        r.gate_ge,
        if r.ge_passed { "PASS" } else { "FAIL" }
    );
    println!("  per-section deltas (new - old):");
    println!(
        "  {:<18} {:>12} {:>12}",
        "section", "development", "validation"
    );
    for name in STARTUP_SECTIONS {
        println!(
            "  {:<18} {:>+12.3} {:>+12.3}",
            name,
            r.dev_deltas.get(name).copied().unwrap_or(0.0),
            r.validation_deltas.get(name).copied().unwrap_or(0.0)
        );
    }
    println!(
        "  max section regression: {:.3} (guard --guard-section-delta {:.3}): {}",
        r.max_section_regression,
        r.guard_section_delta,
        if r.guard_passed { "PASS" } else { "FAIL" }
    );
    if !r.max_regression_section.is_empty() {
        println!("    worst regression: {}", r.max_regression_section);
    }
    if r.passed() {
        println!("  verdict: PASS (all Wave-11 generalization gates)");
    } else {
        println!("  verdict: FAIL");
        for f in &r.failures {
            println!("    - {f}");
        }
    }
}

// trace:exempt reason=internal-detail

/// One pinned blind-test clone in `benchmarks/blind-lock.json`: the
/// upstream URL plus the exact commit the on-disk clone must sit at.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct BlindLockEntry {
    /// Upstream clone URL (informational — the commit is what is enforced).
    pub url: String,
    /// Pinned commit (full sha) the on-disk clone must be at.
    pub commit: String,
}

// trace:exempt reason=internal-detail

/// Load the committed blind commit lock (`benchmarks/blind-lock.json`,
/// shaped `{"blind-test": {<name>: {"url": ..., "commit": ...}}}`) as a
/// sorted name -> entry map. A missing or malformed lock is a hard error —
/// the lock is a protocol artifact, and skipping it would silently disable
/// the commit-pin guard.
// trace:exempt reason=internal-detail
fn load_blind_lock(root: &Path) -> Result<BTreeMap<String, BlindLockEntry>, String> {
    let path = root.join("benchmarks").join("blind-lock.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read blind lock {}: {e}", path.display()))?;
    #[derive(Deserialize)]
    struct LockFile {
        #[serde(rename = "blind-test")]
        blind_test: BTreeMap<String, BlindLockEntry>,
    }
    let lock: LockFile =
        serde_json::from_str(&text).map_err(|e| format!("parse blind lock {}: {e}", path.display()))?;
    Ok(lock.blind_test)
}

// trace:exempt reason=internal-detail

/// The on-disk HEAD commit of a clone dir (`git -C <dir> rev-parse HEAD`),
/// or an error when the dir is not a git checkout.
// trace:exempt reason=internal-detail
fn git_head(dir: &Path) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .map_err(|e| format!("cannot run git in {}: {e}", dir.display()))?;
    if !out.status.success() {
        return Err(format!(
            "git rev-parse HEAD failed in {}: {}",
            dir.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if sha.is_empty() {
        return Err(format!(
            "git rev-parse HEAD returned nothing in {}",
            dir.display()
        ));
    }
    Ok(sha)
}

// trace:exempt reason=internal-detail

/// Pure HEAD-vs-lock comparison (testable without git): Ok when the
/// on-disk clone HEAD equals the pinned commit, else a hard error naming
/// the repo, the actual commit, and the pin.
// trace:exempt reason=internal-detail
fn verify_head(name: &str, actual: &str, expected: &str) -> Result<(), String> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!("blind-test repo {name} at {actual}, lock pins {expected}"))
    }
}

/// Blind-test manifest (Wave 11 — GENERALIZATION II): a sha256 fingerprint
/// of the frozen blind set — the ground-truth answer keys
/// (`benchmarks/blind-test-ground-truth/**`), the clone list
/// (the committed `benchmarks/blind-test/README.md` manifest plus the
/// on-disk repo dirs — the git-ls-files equivalent for the gitignored
/// clones), and the commit pins from `benchmarks/blind-lock.json` (a
/// `lock <name> <sha>` line per repo, so the digest covers the pinned
/// commits). Written into the blind results header; `--blind` verifies the
/// hash matches the previous run before scoring and errors on mismatch, so
/// a changed blind set can never silently re-score different keys.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BlindManifest {
    /// sha256 hex of the deterministic manifest text.
    pub sha256: String,
    /// Number of ground-truth key files hashed.
    pub ground_truth_files: usize,
    /// Number of blind-test repo dirs in the clone list.
    pub repos: usize,
    /// The deterministic manifest text (paths + per-file hashes + clone
    /// list); the sha256 is over exactly this.
    pub text: String,
}

// trace:exempt reason=internal-detail

/// Deterministic manifest text + sha256 over the blind set under `root`:
/// every file in `benchmarks/blind-test-ground-truth/**` (path + content
/// hash), the clone list (the committed `benchmarks/blind-test/README.md`
/// content hash + the sorted top-level repo dir names — the git-ls-files
/// equivalent for the gitignored clones), and a `lock <name> <sha>` line
/// per repo from the committed `benchmarks/blind-lock.json` — the digest
/// covers the pinned commits. Missing ground-truth dir is an error (the
/// protocol requires it); a missing README is tolerated (the clone list
/// then reduces to the repo dirs); a missing lock is an error (the commit
/// pins are a protocol artifact).
// trace:exempt reason=internal-detail
pub fn blind_manifest(root: &Path) -> Result<BlindManifest, String> {
    let gt_dir = root.join("benchmarks").join("blind-test-ground-truth");
    if !gt_dir.is_dir() {
        return Err(format!(
            "blind-test ground-truth dir not found (run --blind from the workspace): {}",
            gt_dir.display()
        ));
    }
    let blind_dir = root.join("benchmarks").join("blind-test");
    let mut out = String::from("# scc blind-test manifest (deterministic)\n");

    let mut gt_files: Vec<PathBuf> = Vec::new();
    collect_files(&gt_dir, &mut gt_files);
    gt_files.sort();
    for f in &gt_files {
        let rel = f
            .strip_prefix(&gt_dir)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| f.display().to_string());
        let content = std::fs::read(f).map_err(|e| format!("read {}: {e}", f.display()))?;
        out.push_str(&format!("ground-truth {rel} {}\n", sha256::hex(&content)));
    }

    // clone list: the committed README manifest + the on-disk repo dirs
    // (git ls-files of benchmarks/blind-test is just README.md — the
    // clones are gitignored — so the dir names are the machine-level
    // clone-set fingerprint).
    let readme = blind_dir.join("README.md");
    if readme.is_file() {
        let content =
            std::fs::read(&readme).map_err(|e| format!("read {}: {e}", readme.display()))?;
        out.push_str(&format!("README.md {}\n", sha256::hex(&content)));
    }
    let dirs = repo_dirs(&blind_dir);
    for d in &dirs {
        out.push_str(&format!("clone {d}\n"));
    }
    // commit pins: the committed benchmarks/blind-lock.json (url + commit
    // per blind-test repo). The digest covers the pinned commits, so a
    // re-pin (or a reclone that updates the lock) changes the hash —
    // and run_atlas_blind additionally verifies each on-disk clone HEAD
    // against the pin before scoring.
    let lock = load_blind_lock(root)?;
    for (name, entry) in &lock {
        out.push_str(&format!("lock {name} {}\n", entry.commit));
    }

    let digest = sha256::hex(out.as_bytes());
    Ok(BlindManifest {
        sha256: digest,
        ground_truth_files: gt_files.len(),
        repos: dirs.len(),
        text: out,
    })
}

/// Recursively collect regular files under `dir` into `out`.
fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_files(&p, out);
        } else if p.is_file() {
            out.push(p);
        }
    }
}

/// The manifest hash recorded in a previous blind results-file header
/// (`blind manifest sha256: <hex>`), for the change-detection check.
/// The header line also carries a human summary after the hex
/// (`(20 ground-truth files, ...)`); only the first whitespace token is
/// the hash.
fn manifest_hash_from_header(header: &str) -> Option<String> {
    header.lines().find_map(|l| {
        l.trim()
            .strip_prefix("blind manifest sha256:")
            .map(str::trim)
            .and_then(|h| h.split_whitespace().next())
            .filter(|h| !h.is_empty())
            .map(|h| h.to_string())
    })
}

/// Guarded division for the blind transfer ratio: `numerator / denominator`,
/// 0.0 when the denominator is 0 (nothing transferred onto nothing).
fn safe_ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator == 0.0 {
        0.0
    } else {
        numerator / denominator
    }
}

/// Per-section blind transfer ratios (blind mean / validation mean) over
/// the seven layers plus overall — deterministic, 0.0-guarded.
fn blind_transfer_ratios(c: &BlindComparison) -> Vec<(&'static str, f64)> {
    let v = &c.validation;
    let b = &c.blind;
    vec![
        ("architecture", safe_ratio(b.mean_architecture, v.mean_architecture)),
        ("entrypoints", safe_ratio(b.mean_entrypoints, v.mean_entrypoints)),
        ("behavior", safe_ratio(b.mean_behavior, v.mean_behavior)),
        ("state_authority", safe_ratio(b.mean_state_authority, v.mean_state_authority)),
        ("contracts", safe_ratio(b.mean_contracts, v.mean_contracts)),
        ("landmarks", safe_ratio(b.mean_landmarks, v.mean_landmarks)),
        ("tests", safe_ratio(b.mean_tests, v.mean_tests)),
        ("overall", safe_ratio(b.mean_overall, v.mean_overall)),
    ]
}

// trace:exempt reason=internal-detail

/// Blind-test protocol comparison: validation-vs-blind generalization.
#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct BlindComparison {
    /// Validation corpus aggregates (`benchmarks/holdout`) — per-repo
    /// detail stripped.
    pub validation: AtlasRecallReport,
    /// Blind corpus aggregates (`benchmarks/blind-test`) — per-repo detail
    /// stripped: blind-test failures are never shown to tuning agents.
    pub blind: AtlasRecallReport,
    /// Per-layer gap = blind mean - validation mean (fraction, negative = lag).
    pub gap_architecture: f64,
    pub gap_entrypoints: f64,
    pub gap_behavior: f64,
    pub gap_state_authority: f64,
    pub gap_contracts: f64,
    pub gap_landmarks: f64,
    pub gap_tests: f64,
    pub gap_overall: f64,
    /// The sha256 manifest fingerprint of the frozen blind set scored in
    /// this run (ground-truth keys + clone list); verified against the
    /// previous run before scoring.
    pub manifest: BlindManifest,
    /// Path of the written results file.
    pub results_file: String,
}

/// Score one fixed-protocol corpus with the same recall pipeline; missing
/// or empty dirs are errors, not silent empty runs.
fn score_protocol_corpus(
    corpus: &Path,
    ground_truth: &Path,
    resolve: bool,
    mode_label: &str,
) -> Result<AtlasRecallReport, String> {
    if !corpus.is_dir() {
        return Err(format!(
            "corpus dir not found (run from the workspace): {}",
            corpus.display()
        ));
    }
    let names = repo_dirs(corpus);
    if names.is_empty() {
        return Err(format!("corpus dir is empty: {}", corpus.display()));
    }
    let mut report = run_atlas_recall(corpus, ground_truth, &names, false, resolve)?;
    report.mode = format!("{mode_label}: {}", corpus.display());
    Ok(report)
}

// trace:exempt reason=internal-detail

/// Run the blind protocol: verify the frozen blind clones are at their
/// pinned commits (`benchmarks/blind-lock.json`), score the validation
/// corpus (`benchmarks/holdout`) and the blind-test corpus
/// (`benchmarks/blind-test`) with the same recall pipeline, keep ONLY
/// aggregates (per-repo rows, missed keys, and filenames are stripped —
/// blind-test failures are never shown to tuning agents), compute the
/// validation-vs-blind generalization gap, write
/// `benchmarks/results/blind-v1.txt`, and return the comparison.
///
/// `--diagnose` is refused: diagnosis prints per-repo miss lines, which
/// would leak the blind misses ("blind corpus is not diagnosable").
// trace:exempt reason=internal-detail
pub fn run_atlas_blind(diagnose: bool, resolve: bool) -> Result<BlindComparison, String> {
    if diagnose {
        return Err("blind corpus is not diagnosable".into());
    }
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let root = crate::find_root(&cwd);
    let results_dir = root.join("benchmarks").join("results");
    let results_file = results_dir.join("blind-v1.txt");

    // Wave 11: verify the frozen blind set did not change since the last
    // recorded run BEFORE scoring — a mismatched manifest is a protocol
    // error, not a silent re-score of different keys.
    let manifest = blind_manifest(&root)?;
    if results_file.is_file() {
        let previous = std::fs::read_to_string(&results_file).map_err(|e| e.to_string())?;
        if let Some(prev_hash) = manifest_hash_from_header(&previous) {
            if prev_hash != manifest.sha256 {
                return Err(format!(
                    "blind-test set changed: manifest sha256 {prev_hash} (previous run) != {} (current); \
                     the blind ground truth or clone list was modified — refusing to score",
                    manifest.sha256
                ));
            }
        }
    }

    let validation_corpus = root.join("benchmarks").join("holdout");
    let validation_gt = root.join("benchmarks").join("holdout-ground-truth");
    let blind_corpus = root.join("benchmarks").join("blind-test");
    let blind_gt = root.join("benchmarks").join("blind-test-ground-truth");

    // Wave 13: the blind corpus is frozen at pinned commits — verify every
    // clone's on-disk HEAD against benchmarks/blind-lock.json BEFORE
    // scoring. A reclone at a different commit (or a missing clone) is a
    // protocol error, not a silent re-score of different code.
    let lock = load_blind_lock(&root)?;
    for (name, entry) in &lock {
        let clone = blind_corpus.join(name);
        let actual = git_head(&clone)
            .map_err(|e| format!("blind-test repo {name} missing or not a git checkout: {e}"))?;
        verify_head(name, &actual, &entry.commit)?;
    }

    let validation = score_protocol_corpus(&validation_corpus, &validation_gt, resolve, "validation")?;
    let blind = score_protocol_corpus(&blind_corpus, &blind_gt, resolve, "blind-test")?;
    // Aggregates only, end to end: drop every per-repo row, missed key,
    // and filename before the comparison leaves this function.
    let validation = validation.aggregates_only();
    let blind = blind.aggregates_only();

    let c = BlindComparison {
        gap_architecture: HoldoutComparison::layer_gap(
            validation.mean_architecture,
            blind.mean_architecture,
        ),
        gap_entrypoints: HoldoutComparison::layer_gap(
            validation.mean_entrypoints,
            blind.mean_entrypoints,
        ),
        gap_behavior: HoldoutComparison::layer_gap(validation.mean_behavior, blind.mean_behavior),
        gap_state_authority: HoldoutComparison::layer_gap(
            validation.mean_state_authority,
            blind.mean_state_authority,
        ),
        gap_contracts: HoldoutComparison::layer_gap(
            validation.mean_contracts,
            blind.mean_contracts,
        ),
        gap_landmarks: HoldoutComparison::layer_gap(validation.mean_landmarks, blind.mean_landmarks),
        gap_tests: HoldoutComparison::layer_gap(validation.mean_tests, blind.mean_tests),
        gap_overall: HoldoutComparison::layer_gap(validation.mean_overall, blind.mean_overall),
        manifest,
        results_file: results_file.display().to_string(),
        validation,
        blind,
    };

    std::fs::create_dir_all(&results_dir).map_err(|e| e.to_string())?;
    std::fs::write(&results_file, c.to_blind_text()).map_err(|e| e.to_string())?;
    Ok(c)
}

impl BlindComparison {
    /// Deterministic aggregates-only text for
    /// `benchmarks/results/blind-v1.txt` (aggregates + gap only).
    fn to_blind_text(&self) -> String {
        let mut out = String::new();
        out.push_str("# Blind v1 — validation vs blind (aggregates only)\n");
        out.push_str(&format!(
            "blind manifest sha256: {} ({} ground-truth files, {} blind-test repos)\n",
            self.manifest.sha256, self.manifest.ground_truth_files, self.manifest.repos
        ));
        out.push_str(&format!("validation corpus: {}\n", self.validation.mode));
        out.push_str(&format!("blind corpus:      {}\n", self.blind.mode));
        out.push_str(&format!("results:           {}\n", self.results_file));
        out.push_str(
            "# blind-test failures are never shown to tuning agents: no per-repo rows, no filenames, no missed keys\n",
        );
        out.push('\n');

        let rows: [(&str, f64, f64); 8] = [
            ("architecture", self.validation.mean_architecture, self.blind.mean_architecture),
            ("entrypoints", self.validation.mean_entrypoints, self.blind.mean_entrypoints),
            ("behavior", self.validation.mean_behavior, self.blind.mean_behavior),
            ("state_authority", self.validation.mean_state_authority, self.blind.mean_state_authority),
            ("contracts", self.validation.mean_contracts, self.blind.mean_contracts),
            ("landmarks", self.validation.mean_landmarks, self.blind.mean_landmarks),
            ("tests", self.validation.mean_tests, self.blind.mean_tests),
            ("overall (gate)", self.validation.mean_overall, self.blind.mean_overall),
        ];
        out.push_str(&format!(
            "{:<18} {:>12} {:>12} {:>10}\n",
            "layer", "validation", "blind", "gap"
        ));
        for (layer, v, b) in rows {
            let gap = HoldoutComparison::layer_gap(v, b);
            out.push_str(&format!(
                "{:<18} {:>12.3} {:>12.3} {:>+10.3}\n",
                layer, v, b, gap
            ));
        }
        out.push('\n');
        out.push_str(&format!(
            "scored: validation {} (skipped {}) | blind {} (skipped {})\n",
            self.validation.scored,
            self.validation.skipped,
            self.blind.scored,
            self.blind.skipped
        ));
        out.push_str(&format!(
            "precision: validation {:.3} | blind {:.3}\n",
            self.validation.mean_precision, self.blind.mean_precision
        ));
        out.push_str(&format!(
            "F2: validation {:.3} | blind {:.3}\n",
            self.validation.mean_f2, self.blind.mean_f2
        ));
        out.push_str(&format!(
            "density (facts/1k tokens): validation {:.2} | blind {:.2}\n",
            self.validation.mean_density, self.blind.mean_density
        ));
        out.push_str(&format!(
            "atlas tokens: validation {:.0} | blind {:.0}\n",
            self.validation.mean_atlas_tokens, self.blind.mean_atlas_tokens
        ));
        out.push('\n');
        out.push_str("## blind transfer ratio (blind / validation) — how much of the validation recall transfers to the unseen blind corpus\n");
        for (layer, ratio) in blind_transfer_ratios(self) {
            out.push_str(&format!("  {:<18} {:>10.3}\n", layer, ratio));
        }
        out.push('\n');
        out.push_str("## generalization gap (blind - validation) — informational, not gating\n");
        for (layer, gap) in [
            ("architecture", self.gap_architecture),
            ("entrypoints", self.gap_entrypoints),
            ("behavior", self.gap_behavior),
            ("state_authority", self.gap_state_authority),
            ("contracts", self.gap_contracts),
            ("landmarks", self.gap_landmarks),
            ("tests", self.gap_tests),
            ("overall", self.gap_overall),
        ] {
            out.push_str(&format!("  {:<18} {:>+10.3}\n", layer, gap));
        }
        out.push_str(&format!(
            "  gate (recall >= {ATLAS_GATE}): blind {} | validation {}\n",
            if self.blind.gate_passed { "PASS" } else { "FAIL" },
            if self.validation.gate_passed { "PASS" } else { "FAIL" }
        ));
        out
    }
}

/// Print the blind protocol: aggregates ONLY (no per-repo rows, no missed
/// keys, no filenames) plus the validation-vs-blind generalization gap.
pub fn print_blind_report(c: &BlindComparison) {
    println!("scc bench atlas --blind — validation vs blind (aggregates only)");
    println!("  validation corpus: {}", c.validation.mode);
    println!("  blind corpus:      {}", c.blind.mode);
    println!(
        "  blind manifest sha256: {} ({} ground-truth files, {} blind-test repos)",
        c.manifest.sha256, c.manifest.ground_truth_files, c.manifest.repos
    );
    println!("  blind-test failures are never shown to tuning agents.");
    println!("\n=== per-section means ===");
    println!(
        "  {:<18} {:>12} {:>12} {:>10}",
        "layer", "validation", "blind", "gap"
    );
    let rows: [(&str, f64, f64); 8] = [
        ("architecture", c.validation.mean_architecture, c.blind.mean_architecture),
        ("entrypoints", c.validation.mean_entrypoints, c.blind.mean_entrypoints),
        ("behavior", c.validation.mean_behavior, c.blind.mean_behavior),
        ("state_authority", c.validation.mean_state_authority, c.blind.mean_state_authority),
        ("contracts", c.validation.mean_contracts, c.blind.mean_contracts),
        ("landmarks", c.validation.mean_landmarks, c.blind.mean_landmarks),
        ("tests", c.validation.mean_tests, c.blind.mean_tests),
        ("overall (gate)", c.validation.mean_overall, c.blind.mean_overall),
    ];
    for (layer, v, b) in rows {
        let gap = HoldoutComparison::layer_gap(v, b);
        println!("  {:<18} {:>12.3} {:>12.3} {:>+10.3}", layer, v, b, gap);
    }
    println!(
        "  scored: validation {} (skipped {}) | blind {} (skipped {})",
        c.validation.scored, c.validation.skipped, c.blind.scored, c.blind.skipped
    );
    println!(
        "  precision: validation {:.3} | blind {:.3}   F2: validation {:.3} | blind {:.3}",
        c.validation.mean_precision, c.blind.mean_precision, c.validation.mean_f2, c.blind.mean_f2
    );
    println!(
        "  density (facts/1k tokens): validation {:.2} | blind {:.2}   atlas tokens: validation {:.0} | blind {:.0}",
        c.validation.mean_density, c.blind.mean_density,
        c.validation.mean_atlas_tokens, c.blind.mean_atlas_tokens
    );
    println!("\n=== blind transfer ratio (blind / validation) ===");
    for (layer, ratio) in blind_transfer_ratios(c) {
        println!("  {:<18} {:>10.3}", layer, ratio);
    }
    println!("\n=== generalization gap (blind - validation) — informational, not gating ===");
    for (layer, gap) in [
        ("architecture", c.gap_architecture),
        ("entrypoints", c.gap_entrypoints),
        ("behavior", c.gap_behavior),
        ("state_authority", c.gap_state_authority),
        ("contracts", c.gap_contracts),
        ("landmarks", c.gap_landmarks),
        ("tests", c.gap_tests),
        ("overall", c.gap_overall),
    ] {
        println!("  {:<18} {:>+10.3}", layer, gap);
    }
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
    println!("scc bench atlas — startup-atlas recall vs independent ground truth (Wave 8 §57, v3)");
    println!("  mode: {}", r.mode);
    println!(
        "  gate: overall mean recall (architecture+entrypoints+behavior+state_authority+contracts) >= {ATLAS_GATE}"
    );
    println!(
        "  {:<24} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6} {:>8} {:>6} {:>6} {:>6} {:>5}  note",
        "repo", "arch", "entry", "behav", "state", "contr", "landm", "tests", "overall",
        "prec", "f2", "f/1k", "toks"
    );
    for repo in &r.repos {
        match &repo.skipped_reason {
            Some(reason) => println!(
                "  {:<24} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6} {:>8} {:>6} {:>6} {:>6} {:>5}  skipped: {reason}",
                repo.repo, "-", "-", "-", "-", "-", "-", "-", "-", "-", "-", "-", "-"
            ),
            None => {
                let note = if repo.resolved_calls > 0 {
                    format!("resolved:{}", repo.resolved_calls)
                } else {
                    String::new()
                };
                println!(
                    "  {:<24} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>8.3} {:>6.3} {:>6.3} {:>6.2} {:>5}  {}",
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
                    repo.f2,
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
        "  {:<24} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>8.3} {:>6.3} {:>6.3} {:>6.2} {:>5}",
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
        r.mean_f2,
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
    fn layer_precision_counts_atlas_entries_matching_ground_truth() {
        // One haystack entry per line; an entry matches when a ground-truth
        // item is contained in it. 2 of 3 entries match -> 2/3.
        let items = vec!["services".to_string(), "db.items".to_string()];
        let hay = "services\ndb.items\nzzz_extra_fact";
        let p = layer_precision(&items, hay);
        assert!((p - 2.0 / 3.0).abs() < 1e-9, "2/3 entries match: {p}");

        // non-matching entries drag precision down
        let p2 = layer_precision(&items, "services\nzzz_extra_fact\nzzz_extra_fact2");
        assert!((p2 - 1.0 / 3.0).abs() < 1e-9, "1/3 entries match: {p2}");

        // empty layer haystack -> 1.0 (nothing spurious)
        assert_eq!(layer_precision(&items, ""), 1.0);
        // empty ground truth -> no entry can match -> 0.0
        assert_eq!(layer_precision(&[], hay), 0.0);
        // normalization applies to both sides
        let p3 = layer_precision(&["Controller::run".to_string()], "controller.run\nother");
        assert!((p3 - 0.5).abs() < 1e-9, ":: alias applied: {p3}");
    }

    #[test]
    fn f2_score_weights_recall_over_precision() {
        // F2 = 5PR/(4P+R): recall-favoring harmonic mean.
        let a = f2_score(0.9, 0.5);
        let b = f2_score(0.5, 0.9);
        assert!(b > a, "recall-weighted: {a} vs {b}");
        // exact values: P=0.5 R=0.5 -> 5*0.25/(2+0.5) = 1.25/2.5 = 0.5
        assert!((f2_score(0.5, 0.5) - 0.5).abs() < 1e-9);
        // zero when P+R==0
        assert_eq!(f2_score(0.0, 0.0), 0.0);
        assert_eq!(f2_score(0.0, 0.5), 0.0);
        assert_eq!(f2_score(0.5, 0.0), 0.0);
        // perfect P and R -> 1.0
        assert!((f2_score(1.0, 1.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn aggregates_only_strips_per_repo_detail() {
        let r = AtlasRecallReport {
            mean_overall: 0.42,
            repos: vec![RepoRecall {
                repo: "secret-repo".into(),
                missed: vec!["architecture:secret_item".into()],
                ..Default::default()
            }],
            ..Default::default()
        };
        let a = r.aggregates_only();
        assert_eq!(a.repos.len(), 0);
        assert!((a.mean_overall - 0.42).abs() < 1e-9, "aggregates survive");
    }

    #[test]
    fn run_atlas_blind_refuses_diagnose() {
        let err = run_atlas_blind(true, false).unwrap_err();
        assert!(err.contains("blind corpus is not diagnosable"), "{err}");
    }

    #[test]
    fn blind_results_text_is_aggregates_only_and_deterministic() {
        let mut validation = AtlasRecallReport {
            mean_architecture: 0.3,
            mean_overall: 0.27,
            mean_precision: 0.6,
            mean_f2: 0.4,
            scored: 20,
            ..Default::default()
        };
        validation.mean_entrypoints = 0.2;
        validation.mean_behavior = 0.3;
        validation.mean_state_authority = 0.3;
        validation.mean_contracts = 0.2;
        validation.mean_landmarks = 0.1;
        validation.mean_tests = 0.1;
        validation.mean_density = 0.5;
        validation.mean_atlas_tokens = 46657.0;
        let mut blind = validation.clone();
        blind.mean_overall = 0.31;
        blind.mean_architecture = 0.34;
        blind.mean_contracts = 0.25;

        let c = BlindComparison {
            gap_architecture: HoldoutComparison::layer_gap(validation.mean_architecture, blind.mean_architecture),
            gap_entrypoints: HoldoutComparison::layer_gap(validation.mean_entrypoints, blind.mean_entrypoints),
            gap_behavior: HoldoutComparison::layer_gap(validation.mean_behavior, blind.mean_behavior),
            gap_state_authority: HoldoutComparison::layer_gap(validation.mean_state_authority, blind.mean_state_authority),
            gap_contracts: HoldoutComparison::layer_gap(validation.mean_contracts, blind.mean_contracts),
            gap_landmarks: HoldoutComparison::layer_gap(validation.mean_landmarks, blind.mean_landmarks),
            gap_tests: HoldoutComparison::layer_gap(validation.mean_tests, blind.mean_tests),
            gap_overall: HoldoutComparison::layer_gap(validation.mean_overall, blind.mean_overall),
            manifest: BlindManifest {
                sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
                ground_truth_files: 20,
                repos: 20,
                text: String::new(),
            },
            results_file: "benchmarks/results/blind-v1.txt".to_string(),
            validation,
            blind,
        };
        let text = c.to_blind_text();
        let text2 = c.to_blind_text();
        assert_eq!(text, text2, "deterministic output");
        assert!(text.contains("aggregates only"), "{text}");
        assert!(text.contains("never shown to tuning agents"), "{text}");
        assert!(text.contains("overall (gate)"), "{text}");
        assert!(text.contains("generalization gap"), "{text}");
        assert!(text.contains("+0.040"), "gap +0.04 rendered: {text}");
        // Wave 11: manifest header + blind transfer ratios are printed
        assert!(
            text.contains("blind manifest sha256: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
            "manifest hash in header: {text}"
        );
        assert!(text.contains("blind transfer ratio"), "{text}");
        assert!(
            text.contains("overall"),
            "overall row present: {text}"
        );
        assert!(
            text.contains("     1.148"),
            "overall transfer ratio 0.31/0.27 = 1.148 rendered: {text}"
        );
        assert!(
            !text.contains("missed:"),
            "no per-repo miss lines in blind output: {text}"
        );
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
        // v3 metrics are finite and in range
        assert!((0.0..=1.0).contains(&r.precision), "startup precision: {}", r.precision);
        assert!((0.0..=1.0).contains(&r.f2), "F2: {}", r.f2);
        assert_eq!(r.layer_precision.len(), 5, "five startup layers");
        assert_eq!(r.layer_f2.len(), 5);
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
            results_file: "benchmarks/results/holdout-v3.txt".to_string(),
            dev,
            holdout,
        };
        let text = c.to_results_text();
        let text2 = c.to_results_text();
        assert_eq!(text, text2, "deterministic output");
        assert!(text.contains("development corpus vs validation corpus"), "{text}");
        assert!(text.contains("overall (gate)"));
        assert!(text.contains("per-layer precision + F2"), "{text}");
        assert!(text.contains("verdict: BORDERLINE"));
        assert!(text.contains("-0.040"), "gap -0.04 rendered: {text}");
    }
    #[test]
    fn behavior_chains_match_in_order() {
        // P1: ground-truth chains (A -> B -> C) match the per-step haystack
        // as an in-order subsequence, not as a substring.
        let hay = "src: Command.main\n-> src: Command.parse_args\n-> src: Command.invoke";
        assert!(chain_matches("Command.main -> Command.parse_args -> Command.invoke", hay));
        // gaps allowed (other steps between), order required
        let hay2 = "a: X\nb: main\nc: mid\nd: parse_args\ne: end\nf: invoke";
        assert!(chain_matches("main -> parse_args -> invoke", hay2));
        // wrong order must NOT match
        assert!(!chain_matches("invoke -> main", hay));
        assert!(!chain_matches("main -> invoke -> parse_args", hay2));
        // plain items still substring-match
        assert!(item_matches("parse_args", hay));
    }

    // ---- Wave 11: generalization gates + blind manifest ----

    #[test]
    fn sha256_matches_standard_test_vectors() {
        assert_eq!(
            sha256::hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256::hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256::hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let million: Vec<u8> = vec![b'a'; 1_000_000];
        assert_eq!(
            sha256::hex(&million),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn generalization_efficiency_gate_fails_negative_passes_positive() {
        // dev improved, validation regressed -> negative GE -> the default
        // gate (MIN = 0.0) fails: the semantic wave overfit
        let ge = generalization_efficiency(0.10, -0.05);
        assert!((ge - (-0.5)).abs() < 1e-9);
        assert!(ge <= 0.0, "negative GE must fail the default gate");
        // both moved up -> positive GE -> passes
        let ge2 = generalization_efficiency(0.10, 0.04);
        assert!((ge2 - 0.4).abs() < 1e-9);
        assert!(ge2 > 0.0, "positive GE passes the default gate");
        // zero dev delta: validation-only improvement is pure generalization
        // (1.0); anything else is 0.0 — never NaN/inf
        assert_eq!(generalization_efficiency(0.0, 0.05), 1.0);
        assert_eq!(generalization_efficiency(0.0, 0.0), 0.0);
        assert_eq!(generalization_efficiency(0.0, -0.05), 0.0);
        for d in [0.0, -0.0, 1e-9, -1e-9] {
            assert!(generalization_efficiency(0.0, d).is_finite());
        }
    }

    fn holdout_with(
        dev_overall: f64,
        dev_sections: [f64; 5],
        val_overall: f64,
        val_sections: [f64; 5],
    ) -> HoldoutComparison {
        let dev = AtlasRecallReport {
            mean_overall: dev_overall,
            mean_architecture: dev_sections[0],
            mean_entrypoints: dev_sections[1],
            mean_behavior: dev_sections[2],
            mean_state_authority: dev_sections[3],
            mean_contracts: dev_sections[4],
            ..Default::default()
        };
        let holdout = AtlasRecallReport {
            mean_overall: val_overall,
            mean_architecture: val_sections[0],
            mean_entrypoints: val_sections[1],
            mean_behavior: val_sections[2],
            mean_state_authority: val_sections[3],
            mean_contracts: val_sections[4],
            ..Default::default()
        };
        HoldoutComparison {
            gap_architecture: 0.0,
            gap_entrypoints: 0.0,
            gap_behavior: 0.0,
            gap_state_authority: 0.0,
            gap_contracts: 0.0,
            gap_overall: 0.0,
            verdict: HoldoutVerdict::NoOverfit,
            results_file: String::new(),
            dev,
            holdout,
        }
    }

    #[test]
    fn compare_runs_ge_gate_and_section_guard() {
        let old = holdout_with(0.50, [0.5; 5], 0.50, [0.5; 5]);
        // new run: dev improved everywhere; validation improved overall but
        // contracts regressed 0.50 -> 0.30 (beyond the 0.05 guard)
        let mut new = holdout_with(0.55, [0.55; 5], 0.51, [0.55, 0.55, 0.55, 0.55, 0.30]);
        new.dev.mean_contracts = 0.56;
        let r = compare_runs(&old, &new, 0.0, 0.05);
        // GE = (0.51 - 0.50) / (0.55 - 0.50) = 0.2 > 0.0 -> passes
        assert!((r.generalization_efficiency - 0.2).abs() < 1e-9);
        assert!(r.ge_passed, "{:?}", r.failures);
        // contracts validation delta = 0.30 - 0.50 = -0.20 -> guard fails
        assert!((r.max_section_regression - 0.20).abs() < 1e-9);
        assert!(!r.guard_passed);
        assert!(!r.passed());
        assert!(r.failures.iter().any(|f| f.contains("contracts")));
        assert!((r.validation_deltas["contracts"] - (-0.20)).abs() < 1e-9);

        // everything improved -> both gates pass
        let good = holdout_with(0.56, [0.56; 5], 0.53, [0.53; 5]);
        let r2 = compare_runs(&old, &good, 0.0, 0.05);
        assert!((r2.generalization_efficiency - 0.5).abs() < 1e-9);
        assert!(r2.ge_passed, "{:?}", r2.failures);
        assert!(r2.guard_passed, "{:?}", r2.failures);
        assert!(r2.passed());
        assert!(r2.failures.is_empty());
        assert_eq!(r2.max_section_regression, 0.0);

        // a tight GE floor: ge 0.5 <= MIN 0.6 -> fails
        let r3 = compare_runs(&old, &good, 0.6, 0.05);
        assert!(!r3.ge_passed);
        assert!(r3.failures.iter().any(|f| f.contains("generalization efficiency")));
    }

// trace:exempt reason=internal-detail

    #[test]
// trace:exempt reason=internal-detail
    fn blind_manifest_hash_roundtrip_detects_change() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let gt = root.join("benchmarks/blind-test-ground-truth");
        let blind = root.join("benchmarks/blind-test");
        std::fs::create_dir_all(&gt).unwrap();
        std::fs::create_dir_all(&blind).unwrap();
        std::fs::write(gt.join("axum.md"), "## architecture\n- axum\n").unwrap();
        std::fs::write(gt.join("echo.md"), "## architecture\n- echo\n").unwrap();
        std::fs::write(blind.join("README.md"), "# manifest\n| axum | url |\n").unwrap();
        std::fs::create_dir_all(blind.join("axum")).unwrap();
        std::fs::create_dir_all(blind.join("echo")).unwrap();
        std::fs::write(
            root.join("benchmarks/blind-lock.json"),
            r#"{"blind-test": {"axum": {"url": "https://github.com/tokio-rs/axum", "commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}, "echo": {"url": "https://github.com/labstack/echo", "commit": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}}}"#,
        )
        .unwrap();

        let m1 = blind_manifest(root).unwrap();
        assert_eq!(m1.ground_truth_files, 2);
        assert_eq!(m1.repos, 2);
        assert_eq!(m1.sha256.len(), 64);
        assert!(m1.text.contains("ground-truth axum.md"), "{}", m1.text);
        assert!(m1.text.contains("clone axum"), "{}", m1.text);
        // Wave 13: the manifest carries the pinned commits from the lock
        assert!(
            m1.text.contains("lock axum aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            "{}",
            m1.text
        );
        assert!(
            m1.text.contains("lock echo bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            "{}",
            m1.text
        );
        // deterministic roundtrip
        let m2 = blind_manifest(root).unwrap();
        assert_eq!(m1.sha256, m2.sha256);
        // tamper with a ground-truth key -> hash changes
        std::fs::write(gt.join("axum.md"), "## architecture\n- axum-changed\n").unwrap();
        assert_ne!(m1.sha256, blind_manifest(root).unwrap().sha256);
        // restore, then tamper with the clone list (drop a repo dir)
        std::fs::write(gt.join("axum.md"), "## architecture\n- axum\n").unwrap();
        std::fs::remove_dir_all(blind.join("echo")).unwrap();
        let m4 = blind_manifest(root).unwrap();
        assert_ne!(m1.sha256, m4.sha256);
        assert_eq!(m4.repos, 1);
        // re-pin one commit -> the digest covers the pinned commits
        std::fs::write(
            root.join("benchmarks/blind-lock.json"),
            r#"{"blind-test": {"axum": {"url": "https://github.com/tokio-rs/axum", "commit": "cccccccccccccccccccccccccccccccccccccccc"}, "echo": {"url": "https://github.com/labstack/echo", "commit": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}}}"#,
        )
        .unwrap();
        assert_ne!(m1.sha256, blind_manifest(root).unwrap().sha256);
        // header roundtrip: the recorded hash is exactly what verification reads
        let header = format!("blind manifest sha256: {}\n", m1.sha256);
        assert_eq!(manifest_hash_from_header(&header).as_deref(), Some(m1.sha256.as_str()));
        assert_eq!(manifest_hash_from_header("no hash here"), None);
        // missing ground-truth dir is an error, not a silent empty manifest
        std::fs::remove_dir_all(&gt).unwrap();
        assert!(blind_manifest(root).is_err());
        // a missing lock is an error too — skipping it would silently
        // disable the commit-pin guard
        std::fs::create_dir_all(&gt).unwrap();
        std::fs::write(gt.join("axum.md"), "## architecture\n- axum\n").unwrap();
        std::fs::remove_file(root.join("benchmarks/blind-lock.json")).unwrap();
        let err = blind_manifest(root).unwrap_err();
        assert!(err.contains("blind-lock.json"), "{err}");
    }

// trace:exempt reason=internal-detail

    #[test]
// trace:exempt reason=internal-detail
    fn blind_head_lock_verifies_pinned_commit() {
        // Pure HEAD-vs-lock check (no git needed): a matching commit passes,
        // a mismatch is a hard error naming the repo, the actual commit, and
        // the pin.
        assert!(verify_head("axum", "abc123", "abc123").is_ok());
        let err = verify_head("axum", "abc123", "def456").unwrap_err();
        assert!(
            err.contains("blind-test repo axum at abc123, lock pins def456"),
            "{err}"
        );
    }

    #[test]
    fn blind_transfer_ratio_guards_zero_denominator() {
        assert_eq!(safe_ratio(1.0, 2.0), 0.5);
        assert_eq!(safe_ratio(0.0, 0.0), 0.0);
        assert_eq!(safe_ratio(5.0, 0.0), 0.0);
    }
}



