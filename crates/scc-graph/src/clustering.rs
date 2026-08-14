//! Semantic hierarchical clustering (generalization wave).
//!
//! Files belong to architecture because of BEHAVIOR, not directories. The
//! clustering graph is built over **atomic regions** — the finest
//! evidence-backed file groups: authoritative candidate dirs (declared /
//! package / deployment / cli boundaries — never split) and the immediate
//! sub-directories of bare code-region dirs. Weighted edges from graph
//! evidence drive a greedy agglomerative merge:
//!
//! | signal | weight |
//! |---|---|
//! | semantic calls | +2 |
//! | shared state authority | +4 |
//! | public surface cohesion (exports under one interface / extension point) | +4 |
//! | flow participation | +3 |
//! | event ownership (shared topic) | +3 |
//! | invocation-surface cohesion (same framework family) | +2 |
//! | type hierarchy (shared base) | +2 |
//! | co-change | +1 (capped at 5) |
//! | deployment boundary | +5 (constraint, not decoration) |
//! | archetype prior (CLI/Framework/Service/Compiler) | +2 |
//!
//! The merge may SPLIT a top-level directory into several components (its
//! sub-regions land in different clusters when intra-dir cohesion is low)
//! and may MERGE regions across directories when behavior says so — the
//! longest-prefix path assignment is replaced by the clustering result.
//! Path/package/deployment remain as constraints and priors: authoritative
//! boundaries are atomic, and merging ACROSS deployment units requires
//! weight > [`SERVICE_THRESHOLD`].
//!
//! Layers: atomic region → component (merge ≥ `MERGE_THRESHOLD`) → service
//! (pass-2 sum-linkage ≥ `SERVICE_THRESHOLD`). Determinism is the contract:
//! every iteration is over sorted collections and ties break on the
//! smallest index pair.

use crate::{RealityGraph, Result};
use scc_core::{kinds, predicates, Archetype};
use scc_store::Store;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::components::{
    boundary_rank, component_for_path, BOUNDARY_CODE_REGION, BOUNDARY_ROOT, ComponentCandidate,
    LAYER_CODE_REGION, LAYER_COMPONENT, LAYER_SERVICE, MERGE_THRESHOLD, SERVICE_THRESHOLD,
};

// ---- signal weights (the clustering graph edge list) ----
/// Semantic calls between regions (+2).
pub const W_CALL: i32 = 2;
/// Shared state authority: same store written or same config read (+4).
pub const W_STATE: i32 = 4;
/// Public surface cohesion: exports consumed by the same facade/interface,
/// or symbols registered into the same extension point (+4).
pub const W_PUBLIC_SURFACE: i32 = 4;
/// LibrarySdk archetype: public API cohesion weight doubles (+8).
pub const W_PUBLIC_SURFACE_LIBRARY: i32 = 8;
/// Flow participation: regions whose symbols share a flow walk (+3).
pub const W_FLOW: i32 = 3;
/// Event ownership: regions sharing a topic via PUBLISHES/SUBSCRIBES/
/// CONSUMES (+3).
pub const W_EVENT: i32 = 3;
/// Invocation-surface cohesion: regions whose symbols expose invocation
/// surfaces of the same framework family (queue consumers, schedulers,
/// plugin registrations; framework_callback+lifecycle count as one
/// family) (+2, capped per pair).
pub const W_SURFACE_FAMILY: i32 = 2;
/// Type hierarchy: regions implementing/inheriting the same base (+2).
pub const W_TYPE_HIERARCHY: i32 = 2;
/// Deployment boundary containment: same deployment unit (+5).
pub const W_DEPLOYMENT: i32 = 5;
/// Co-change cap: a change-heavy region pair never dominates the graph.
pub const COCHANGE_CAP: i32 = 5;

/// One clustered component: the merge result over a set of atomic regions.
#[derive(Debug, Clone)]
pub struct ClusterComponent {
    pub name: String,
    /// Union of the member regions' path prefixes (sorted, deduped).
    pub dirs: Vec<String>,
    /// Highest-authority boundary kind among the members.
    pub boundary_kind: String,
    /// `LAYER_*` constant: `code_region` for a single bare dir region,
    /// `component` for anything with evidence (authoritative boundary or a
    /// multi-region merge).
    pub layer: String,
    /// File entity ids owned by the cluster (sorted).
    pub files: Vec<String>,
    /// Member region indices (sorted).
    pub member_regions: Vec<usize>,
}

/// The structural output of the clusterer: what `compile_components` turns
/// into component entities. All maps are name/order aligned and sorted.
pub struct ClusteringResult {
    /// Clusters sorted by name (the order component entities are built in).
    pub clusters: Vec<ClusterComponent>,
    /// The pruned atomic regions (authoritative boundaries + code-region
    /// dirs/subdirs), aligned with each cluster's `member_regions` indices.
    pub regions: Vec<ComponentCandidate>,
    /// symbol id -> component NAME (the cluster the symbol's region merged
    /// into).
    pub symbol_component: HashMap<String, String>,
    /// component name -> file entity ids.
    pub files_in_component: BTreeMap<String, Vec<String>>,
    /// component name -> deployment unit name (tightest build context).
    pub parent_per_comp: BTreeMap<String, String>,
    /// Cross-component weight sums (aligned with `clusters`) for the
    /// pass-2 service merge.
    pub component_weights: Vec<Vec<i32>>,
    /// Cross-component deployment-unit constraint (aligned with
    /// `clusters`): true when both sides sit in DIFFERENT deployment
    /// units (such pairs merge only at weight > SERVICE_THRESHOLD).
    pub cross_unit: Vec<Vec<bool>>,
}

/// Build the atomic regions from the path candidates:
/// 1. every authoritative boundary (declared / package / deployment / cli /
///    root) is ONE region — these never split internally;
/// 2. each code-region top-level dir becomes one region per immediate
///    sub-directory that holds files, plus one region for the dir's own
///    direct files;
/// 3. the synthetic `root` region always exists (root-level files).
///
/// Regions that end up with zero assigned files are pruned afterwards by
/// the caller (a code-region dir whose every file belongs to an
/// authoritative boundary is not a region). Deterministic: candidates are
/// processed in sorted name order and every file list is sorted.
// trace:v1 id=impl.scc.clustering work=WORK-SCC-005 satisfies=REQ-SCC-IR
pub fn build_regions(
    graph: &RealityGraph,
    candidates: &[ComponentCandidate],
) -> Vec<ComponentCandidate> {
    let mut regions: Vec<ComponentCandidate> = Vec::new();
    let mut by_name: HashSet<String> = HashSet::new();

    // 1. authoritative boundaries (sorted for determinism)
    let mut auth: Vec<ComponentCandidate> = candidates
        .iter()
        .filter(|c| c.boundary_kind != BOUNDARY_CODE_REGION)
        .cloned()
        .collect();
    auth.sort_by(|a, b| a.name.cmp(&b.name));
    for c in auth {
        if by_name.insert(c.name.clone()) {
            regions.push(c);
        }
    }

    // 2. code-region top-level dirs: direct files -> one dir region;
    //    sub-directories with files -> one region each
    let mut code: Vec<ComponentCandidate> = candidates
        .iter()
        .filter(|c| c.boundary_kind == BOUNDARY_CODE_REGION)
        .cloned()
        .collect();
    code.sort_by(|a, b| a.name.cmp(&b.name));
    for c in &code {
        if c.name == "root" || by_name.contains(&c.name) {
            continue;
        }
        let dir = &c.name;
        let prefix = format!("{dir}/");
        let mut direct = false;
        let mut subs: BTreeSet<String> = BTreeSet::new();
        for f in graph.entities_of_kind(kinds::FILE) {
            if f.name == *dir || f.name.starts_with(&prefix) {
                let rest = &f.name[dir.len() + 1..];
                if let Some(slash) = rest.find('/') {
                    subs.insert(format!("{dir}/{}", &rest[..slash]));
                } else {
                    direct = true;
                }
            }
        }
        if direct && !by_name.contains(dir) {
            by_name.insert(dir.clone());
            regions.push(ComponentCandidate {
                name: dir.clone(),
                dirs: vec![dir.clone()],
                boundary_kind: BOUNDARY_CODE_REGION.to_string(),
            });
        }
        for sub in subs {
            if by_name.contains(&sub) {
                continue;
            }
            by_name.insert(sub.clone());
            regions.push(ComponentCandidate {
                name: sub.clone(),
                dirs: vec![sub.clone()],
                boundary_kind: BOUNDARY_CODE_REGION.to_string(),
            });
        }
    }

    // 3. root region always exists (root-level files; empty repos too)
    if !by_name.contains("root") {
        regions.push(ComponentCandidate {
            name: "root".to_string(),
            dirs: vec!["root".to_string()],
            boundary_kind: BOUNDARY_ROOT.to_string(),
        });
    }
    regions
}

/// Cluster the atomic regions into components (and record the pass-2
/// service weights). See the module docs for the signal list.
// trace:v1 id=impl.scc.clustering.merge work=WORK-SCC-005 satisfies=REQ-SCC-IR
pub fn cluster_components(
    graph: &RealityGraph,
    store: &Store,
    intent: &[(String, serde_json::Value)],
    candidates: &[ComponentCandidate],
    pairs: &[crate::cochange::CochangePair],
    du_ctxs: &[(String, String)],
) -> Result<ClusteringResult> {
    let mut regions = build_regions(graph, candidates);

    // ---- file -> region assignment (longest prefix over region dirs) ----
    let mut files_in_region: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for f in graph.entities_of_kind(kinds::FILE) {
        let region = component_for_path(&f.name, &regions);
        files_in_region
            .entry(region)
            .or_default()
            .push(f.id.clone());
    }
    for v in files_in_region.values_mut() {
        v.sort();
    }
    // prune code-region regions with zero assigned files (their files were
    // captured by an authoritative boundary — no region shell remains)
    regions.retain(|r| {
        r.boundary_kind != BOUNDARY_CODE_REGION
            || files_in_region
                .get(&r.name)
                .map(|v| !v.is_empty())
                .unwrap_or(false)
    });

    let n = regions.len();
    let idx: HashMap<&str, usize> = regions
        .iter()
        .enumerate()
        .map(|(i, r)| (r.name.as_str(), i))
        .collect();
    let mut w: Vec<Vec<i32>> = vec![vec![0i32; n]; n];

    // ---- symbol -> region ----
    let mut symbol_region: HashMap<String, usize> = HashMap::new();
    for (region, files) in &files_in_region {
        for fid in files {
            for r in graph.out_pred(fid, scc_core::predicates::CONTAINS) {
                if let Some(&ri) = idx.get(region.as_str()) {
                    symbol_region.insert(r.object.clone(), ri);
                }
            }
        }
    }
    // file path -> region (co-change pairs reference paths, not ids)
    let mut path_region: BTreeMap<String, usize> = BTreeMap::new();
    for f in graph.entities_of_kind(kinds::FILE) {
        if let Some(&ri) = idx.get(component_for_path(&f.name, &regions).as_str()) {
            path_region.insert(f.name.clone(), ri);
        }
    }
    let mut syms: Vec<(&String, usize)> = symbol_region
        .iter()
        .map(|(s, r)| (s, *r))
        .collect();
    syms.sort();

    // ---- deployment unit per region (tightest build context) ----
    let mut parent_per_region: Vec<Option<String>> = vec![None; n];
    for (i, r) in regions.iter().enumerate() {
        for (du_name, ctx) in du_ctxs {
            let inside = r.dirs.iter().any(|d| {
                let d = d.trim_end_matches('/');
                d == ctx.as_str() || d.starts_with(&format!("{ctx}/"))
            });
            if inside {
                parent_per_region[i] = Some(du_name.clone());
                break;
            }
        }
    }

    // test files (files referenced by TEST entities): verification code.
    // Its coupling to the code under test — calls, flows — is expected and
    // must NOT drive architecture; a test suite never merges into the
    // component it verifies.
    let mut test_files: HashSet<String> = HashSet::new();
    for t in graph.entities_of_kind(kinds::TEST) {
        if let Some(f) = t.attributes.get("file").and_then(|v| v.as_str()) {
            test_files.insert(f.to_string());
        }
    }
    let is_test_symbol = |sym: &str| -> bool {
        graph
            .entities
            .get(sym)
            .and_then(|e| e.attributes.get("file"))
            .and_then(|v| v.as_str())
            .map(|f| test_files.contains(f))
            .unwrap_or(false)
    };

    let archetype = crate::archetype::detect_archetype(graph, store);

    // ---- signal (a): semantic calls (+2) ----
    for (sym, ra) in &syms {
        if is_test_symbol(sym) {
            continue;
        }
        for r in graph.out_pred(sym, scc_core::predicates::CALLS) {
            if let Some(&rb) = symbol_region.get(&r.object) {
                if ra != &rb {
                    w[*ra][rb] += W_CALL;
                    w[rb][*ra] += W_CALL;
                }
            }
        }
    }

    // ---- signal (b): shared state authority (+4) ----
    for group in crate::state::state_authority_groups(graph) {
        let set: BTreeSet<String> = group
            .iter()
            .filter_map(|s| symbol_region.get(s).map(|&r| regions[r].name.clone()))
            .collect();
        if set.len() >= 2 {
            add_pair_weight(&mut w, &idx, &set, W_STATE);
        }
    }

    // ---- signals (c) public surface cohesion (+4/+8) and (d) type
    // hierarchy (+2): shared IMPLEMENTS/INHERITS targets and shared
    // extension-point (REGISTERS) targets ----
    let mut public_targets: HashSet<String> = HashSet::new();
    for e in graph.entities_of_kind(kinds::EXPORT) {
        public_targets.insert(e.id.clone());
    }
    for r in graph.all_rels() {
        if r.predicate == predicates::EXPORTS {
            // both the export entity AND the exporting symbol are public
            // API — an IMPLEMENTS edge into an exported symbol is an
            // interface implementation
            public_targets.insert(r.object.clone());
            public_targets.insert(r.subject.clone());
        }
    }
    let mut hier_groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut reg_groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for &(sym, ra) in &syms {
        for pred in [predicates::IMPLEMENTS, predicates::INHERITS] {
            for r in graph.out_pred(sym, pred) {
                let group = hier_groups.entry(r.object.clone()).or_default();
                group.insert(regions[ra].name.clone());
                // the facade/interface's own home region coheres with its
                // implementors (exports consumed by the same interface)
                if let Some(&rt) = symbol_region.get(&r.object) {
                    group.insert(regions[rt].name.clone());
                }
            }
        }
        for r in graph.out_pred(sym, predicates::REGISTERS) {
            let group = reg_groups.entry(r.object.clone()).or_default();
            group.insert(regions[ra].name.clone());
            // the extension point's own home region coheres with its
            // registrants
            if let Some(&rt) = symbol_region.get(&r.object) {
                group.insert(regions[rt].name.clone());
            }
        }
    }
    let ps_w = if archetype == Archetype::LibrarySdk {
        W_PUBLIC_SURFACE_LIBRARY
    } else {
        W_PUBLIC_SURFACE
    };
    for (target, set) in &hier_groups {
        if set.len() < 2 {
            continue;
        }
        if public_targets.contains(target) {
            // public API cohesion: exports under one exported interface
            add_pair_weight(&mut w, &idx, set, ps_w);
        } else {
            // plain shared base class
            add_pair_weight(&mut w, &idx, set, W_TYPE_HIERARCHY);
        }
    }
    for set in reg_groups.values() {
        if set.len() >= 2 {
            // extension points: symbols registered into the same
            // plugin/extension/interface registry
            add_pair_weight(&mut w, &idx, set, ps_w);
        }
    }

    // ---- signal (e): flow participation (+3, capped per region pair) ----
    // "Regions whose symbols share a flow cohere": a categorical signal —
    // one shared flow chain (possibly seeded by several entrypoints, e.g.
    // a route and its exported handler) counts once, never stacked.
    // Verification-seeded flows are excluded (see `test_files` above).
    let mut flow_pairs: BTreeSet<(usize, usize)> = BTreeSet::new();
    for (entry, group) in crate::flows::flow_participant_groups(graph, store, intent) {
        if is_test_symbol(&entry) {
            continue;
        }
        let set: BTreeSet<String> = group
            .iter()
            .filter_map(|s| symbol_region.get(s).map(|&r| regions[r].name.clone()))
            .collect();
        if set.len() < 2 {
            continue;
        }
        let cs: Vec<&String> = set.iter().collect();
        for (k, a) in cs.iter().enumerate() {
            let Some(&ia) = idx.get(a.as_str()) else { continue };
            for b in cs.iter().skip(k + 1) {
                let Some(&ib) = idx.get(b.as_str()) else { continue };
                flow_pairs.insert(if ia < ib { (ia, ib) } else { (ib, ia) });
            }
        }
    }
    for (ia, ib) in flow_pairs {
        w[ia][ib] += W_FLOW;
        w[ib][ia] += W_FLOW;
    }

    // ---- signal (f): event ownership (+3, same topic) ----
    let mut topic_groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for &(sym, ra) in &syms {
        for pred in [
            predicates::PUBLISHES,
            predicates::CONSUMES,
            predicates::SUBSCRIBES,
        ] {
            for r in graph.out_pred(sym, pred) {
                topic_groups
                    .entry(r.object.clone())
                    .or_default()
                    .insert(regions[ra].name.clone());
            }
        }
    }
    for set in topic_groups.values() {
        if set.len() >= 2 {
            add_pair_weight(&mut w, &idx, set, W_EVENT);
        }
    }

    // ---- signal (g): invocation-surface cohesion (+2, capped per pair) ----
    // Regions whose symbols own invocation surfaces of the SAME framework
    // family cohere: queue consumers with queue consumers, schedulers with
    // schedulers, plugin registrations with plugin registrations;
    // framework_callback and lifecycle symbols count as one family. The
    // family group is a set of region names, so a shared family adds +2
    // once per pair — never stacked per surface.
    let mut surface_families: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
    for s in crate::flows::invocation_surfaces(graph) {
        if is_test_symbol(&s.symbol) {
            continue;
        }
        let family: &'static str = match s.kind {
            scc_core::InvocationSurfaceKind::Queue => "queue",
            scc_core::InvocationSurfaceKind::Schedule => "schedule",
            scc_core::InvocationSurfaceKind::Plugin => "plugin",
            scc_core::InvocationSurfaceKind::FrameworkCallback
            | scc_core::InvocationSurfaceKind::Lifecycle => "lifecycle",
            _ => continue,
        };
        let Some(&ra) = symbol_region.get(&s.symbol) else { continue };
        surface_families
            .entry(family)
            .or_default()
            .insert(regions[ra].name.clone());
    }
    for set in surface_families.values() {
        if set.len() >= 2 {
            add_pair_weight(&mut w, &idx, set, W_SURFACE_FAMILY);
        }
    }

    // ---- signal (h): co-change (+1 per spanning pair, capped) ----
    let mut cc: BTreeMap<(usize, usize), i32> = BTreeMap::new();
    for p in pairs {
        if let (Some(&ra), Some(&rb)) = (path_region.get(&p.a), path_region.get(&p.b)) {
            if ra != rb {
                let (x, y) = if ra < rb { (ra, rb) } else { (rb, ra) };
                *cc.entry((x, y)).or_insert(0) += 1;
            }
        }
    }
    for ((x, y), count) in cc {
        let weight = count.min(COCHANGE_CAP);
        w[x][y] += weight;
        w[y][x] += weight;
    }

    // ---- signal (i): deployment boundary containment (+5) ----
    let mut du_groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (i, du) in parent_per_region.iter().enumerate() {
        if let Some(du) = du {
            du_groups
                .entry(du.clone())
                .or_default()
                .insert(regions[i].name.clone());
        }
    }
    for set in du_groups.values() {
        if set.len() >= 2 {
            add_pair_weight(&mut w, &idx, set, W_DEPLOYMENT);
        }
    }

    // ---- signal (j): archetype emphasis (+2, one region trait) ----
    if let Some(prior) = crate::archetype::cluster_prior(archetype) {
        let mut set: BTreeSet<String> = BTreeSet::new();
        for &(sym, ra) in &syms {
            let Some(e) = graph.entities.get(sym) else { continue };
            let name = regions[ra].name.clone();
            let has = match prior {
                crate::archetype::ClusterPrior::CliCommands => {
                    let cli_ep = e
                        .attributes
                        .get("entrypoints")
                        .and_then(|v| v.as_array())
                        .map(|eps| {
                            eps.iter().any(|k| {
                                matches!(k.as_str(), Some("cli-subcommand") | Some("cli"))
                            })
                        })
                        .unwrap_or(false);
                    let cli_flags = e
                        .attributes
                        .get("cli_flags")
                        .and_then(|v| v.as_array())
                        .map(|fl| !fl.is_empty())
                        .unwrap_or(false);
                    cli_ep || cli_flags
                }
                crate::archetype::ClusterPrior::FrameworkRegistrations => {
                    !graph.out_pred(sym, predicates::REGISTERS).is_empty()
                }
                crate::archetype::ClusterPrior::ServiceEntrypoints => {
                    !graph.out_pred(sym, predicates::HANDLES).is_empty()
                        || e.attributes
                            .get("entrypoints")
                            .and_then(|v| v.as_array())
                            .map(|eps| !eps.is_empty())
                            .unwrap_or(false)
                }
                crate::archetype::ClusterPrior::CompilerPhases => {
                    crate::archetype::is_phase_symbol(&e.name)
                }
            };
            if has {
                set.insert(name);
            }
        }
        if set.len() >= 2 {
            add_pair_weight(&mut w, &idx, &set, crate::archetype::PRIOR_WEIGHT);
        }
    }

    // ---- cross-unit constraint: pairs in DIFFERENT deployment units may
    // only merge at weight > SERVICE_THRESHOLD ----
    let mut cross_unit: Vec<Vec<bool>> = vec![vec![false; n]; n];
    for i in 0..n {
        for j in (i + 1)..n {
            if let (Some(a), Some(b)) = (&parent_per_region[i], &parent_per_region[j]) {
                if a != b {
                    cross_unit[i][j] = true;
                    cross_unit[j][i] = true;
                }
            }
        }
    }

    // ---- pass 1: greedy merge at MERGE_THRESHOLD (max-linkage) ----
    let mut dsu = Dsu::new(n);
    greedy_merge(&mut dsu, &w, n, MERGE_THRESHOLD, &cross_unit);
    let mut clusters: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..n {
        clusters.entry(dsu.find(i)).or_default().push(i);
    }
    let mut comps: Vec<ClusterComponent> = clusters
        .values()
        .map(|members| build_cluster(&regions, members))
        .collect();
    comps.sort_by(|a, b| a.name.cmp(&b.name));

    // ---- file/symbol maps over the clusters ----
    let mut files_in_component: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for c in &comps {
        let mut files: Vec<String> = Vec::new();
        for &m in &c.member_regions {
            if let Some(fs) = files_in_region.get(&regions[m].name) {
                files.extend(fs.iter().cloned());
            }
        }
        files.sort();
        files.dedup();
        files_in_component.insert(c.name.clone(), files);
    }
    let mut symbol_component: HashMap<String, String> = HashMap::new();
    for (sym, &ri) in &symbol_region {
        let root = dsu.find(ri);
        if let Some(members) = clusters.get(&root) {
            let comp_name = &comps
                .iter()
                .find(|c| c.member_regions == *members)
                .map(|c| c.name.clone())
                .unwrap_or_default();
            symbol_component.insert(sym.clone(), comp_name.clone());
        }
    }

    // ---- parent per component (deployment unit of the cluster dirs) ----
    let mut parent_per_comp: BTreeMap<String, String> = BTreeMap::new();
    for c in &comps {
        for (du_name, ctx) in du_ctxs {
            let inside = c.dirs.iter().any(|d| {
                let d = d.trim_end_matches('/');
                d == ctx.as_str() || d.starts_with(&format!("{ctx}/"))
            });
            if inside {
                parent_per_comp.insert(c.name.clone(), du_name.clone());
                break;
            }
        }
    }

    // ---- pass-2 weights: sum-linkage over the component clusters ----
    let m = comps.len();
    let mut component_weights: Vec<Vec<i32>> = vec![vec![0i32; m]; m];
    let mut cunit: Vec<Vec<bool>> = vec![vec![false; m]; m];
    for (i, ci) in comps.iter().enumerate() {
        for (j, cj) in comps.iter().enumerate() {
            if i == j {
                continue;
            }
            let mut sum = 0;
            for &ra in &ci.member_regions {
                for &rb in &cj.member_regions {
                    sum += w[ra][rb];
                }
            }
            component_weights[i][j] = sum;
        }
    }
    for (i, ci) in comps.iter().enumerate() {
        for (j, cj) in comps.iter().enumerate() {
            if i == j {
                continue;
            }
            let cross = ci.member_regions.iter().any(|&ra| {
                cj.member_regions.iter().any(|&rb| cross_unit[ra][rb])
            });
            cunit[i][j] = cross;
        }
    }

    Ok(ClusteringResult {
        clusters: comps,
        regions,
        symbol_component,
        files_in_component,
        parent_per_comp,
        component_weights,
        cross_unit: cunit,
    })
}

/// Build one cluster record from its member region indices: name (longest
/// common dir prefix, or sorted names joined with `+` when the regions
/// share no directory), highest-authority boundary kind, layer, and the
/// union of member dirs. Deterministic.
fn build_cluster(regions: &[ComponentCandidate], members: &[usize]) -> ClusterComponent {
    let name = cluster_name(regions, members);
    let mut dirs: BTreeSet<String> = BTreeSet::new();
    let mut boundary: Option<(u8, String)> = None;
    // deterministic boundary pick: members in sorted-name order, highest
    // authority wins, ties keep the first (lowest name)
    let mut sorted: Vec<usize> = members.to_vec();
    sorted.sort_by(|a, b| regions[*a].name.cmp(&regions[*b].name));
    for &m in &sorted {
        let r = &regions[m];
        for d in &r.dirs {
            dirs.insert(d.clone());
        }
        let rank = boundary_rank(&r.boundary_kind);
        match &boundary {
            Some((br, _)) if *br >= rank => {}
            _ => boundary = Some((rank, r.boundary_kind.clone())),
        }
    }
    let boundary_kind = boundary.map(|(_, k)| k).unwrap_or_else(|| BOUNDARY_CODE_REGION.to_string());
    let layer = if members.len() == 1
        && (boundary_kind == BOUNDARY_CODE_REGION || boundary_kind == BOUNDARY_ROOT)
    {
        LAYER_CODE_REGION.to_string()
    } else {
        LAYER_COMPONENT.to_string()
    };
    let mut member_regions = members.to_vec();
    member_regions.sort();
    ClusterComponent {
        name,
        dirs: dirs.into_iter().collect(),
        boundary_kind,
        layer,
        files: Vec::new(),
        member_regions,
    }
}

/// Deterministic cluster name: the longest common directory prefix of the
/// member region names when they share one (and it is not another region's
/// name — that would collide), otherwise the sorted member names joined
/// with `+`.
fn cluster_name(regions: &[ComponentCandidate], members: &[usize]) -> String {
    if members.len() == 1 {
        return regions[members[0]].name.clone();
    }
    let segs: Vec<Vec<&str>> = members
        .iter()
        .map(|&i| regions[i].name.split('/').collect())
        .collect();
    let mut common: Vec<&str> = Vec::new();
    'outer: for k in 0..segs[0].len() {
        let seg = segs[0][k];
        for other in segs.iter().skip(1) {
            if other.get(k) != Some(&seg) {
                break 'outer;
            }
        }
        common.push(seg);
    }
    if !common.is_empty() {
        let lcp = common.join("/");
        // the LCP must not be a *different* region's name (id collision)
        let taken_by_other = regions.iter().enumerate().any(|(i, r)| {
            !members.contains(&i) && r.name == lcp
        });
        if !taken_by_other {
            return lcp;
        }
    }
    let mut names: Vec<String> = members.iter().map(|&i| regions[i].name.clone()).collect();
    names.sort();
    names.join("+")
}

/// Add `weight` to every pair inside `set` (deterministic: sorted names).
fn add_pair_weight(w: &mut [Vec<i32>], idx: &HashMap<&str, usize>, set: &BTreeSet<String>, weight: i32) {
    let cs: Vec<&String> = set.iter().collect();
    for (k, a) in cs.iter().enumerate() {
        let Some(&ia) = idx.get(a.as_str()) else { continue };
        for b in cs.iter().skip(k + 1) {
            let Some(&ib) = idx.get(b.as_str()) else { continue };
            w[ia][ib] += weight;
            w[ib][ia] += weight;
        }
    }
}

/// Disjoint-set union-find with path compression.
// # trace:exempt — union-find helper, data container with no behavior
pub(crate) struct Dsu {
    parent: Vec<usize>,
}

impl Dsu {
    pub(crate) fn new(n: usize) -> Dsu {
        Dsu {
            parent: (0..n).collect(),
        }
    }
    pub(crate) fn find(&mut self, mut x: usize) -> usize {
        let mut root = x;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        while self.parent[x] != root {
            let next = self.parent[x];
            self.parent[x] = root;
            x = next;
        }
        root
    }
    pub(crate) fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[rb] = ra;
        }
    }
}

/// Max-linkage weight between two DSU clusters. Pairs that cross deployment
/// units contribute their raw weight only when it exceeds
/// `SERVICE_THRESHOLD` (the ">12" cross-unit rule); otherwise they count 0.
fn cluster_weight(w: &[Vec<i32>], dsu: &mut Dsu, a: usize, b: usize, cross_unit: &[Vec<bool>]) -> i32 {
    let (ra, rb) = (dsu.find(a), dsu.find(b));
    let mut m = 0;
    for (i, row) in w.iter().enumerate() {
        if dsu.find(i) != ra {
            continue;
        }
        for (j, cell) in row.iter().enumerate() {
            if dsu.find(j) != rb {
                continue;
            }
            let cell = if cross_unit[i][j] && *cell <= SERVICE_THRESHOLD {
                0
            } else {
                *cell
            };
            m = m.max(cell);
        }
    }
    m
}

/// Greedy merge: repeatedly union the highest-weight pair while the weight
/// is >= threshold. Deterministic: on weight ties the smallest (i, j) wins.
fn greedy_merge(
    dsu: &mut Dsu,
    w: &[Vec<i32>],
    n: usize,
    threshold: i32,
    cross_unit: &[Vec<bool>],
) {
    loop {
        let mut best: Option<(i32, usize, usize)> = None;
        for i in 0..n {
            for j in (i + 1)..n {
                if dsu.find(i) == dsu.find(j) {
                    continue;
                }
                let wi = cluster_weight(w, dsu, i, j, cross_unit);
                match best {
                    Some((bw, bi, bj)) => {
                        if wi > bw || (wi == bw && (i < bi || (i == bi && j < bj))) {
                            best = Some((wi, i, j));
                        }
                    }
                    None => best = Some((wi, i, j)),
                }
            }
        }
        match best {
            Some((wi, i, j)) if wi >= threshold => dsu.union(i, j),
            _ => break,
        }
    }
}

/// Pass-2 (service) merge: sum-linkage. Pass 1's max-linkage already
/// absorbed every pair >= MERGE_THRESHOLD, so a component pair can only
/// reach SERVICE_THRESHOLD by *accumulated* cross evidence (e.g. four weak
/// signals of 3 each) — the "merged again at >= 12" step. The cross-unit
/// constraint still applies per region pair.
fn cluster_weight_sum(
    w: &[Vec<i32>],
    dsu: &mut Dsu,
    a: usize,
    b: usize,
    cross_unit: &[Vec<bool>],
) -> i32 {
    let (ra, rb) = (dsu.find(a), dsu.find(b));
    let mut sum = 0;
    for (i, row) in w.iter().enumerate() {
        if dsu.find(i) != ra {
            continue;
        }
        for (j, cell) in row.iter().enumerate() {
            if dsu.find(j) != rb {
                continue;
            }
            if cross_unit[i][j] && *cell <= SERVICE_THRESHOLD {
                continue;
            }
            sum += *cell;
        }
    }
    sum
}

/// Greedy sum-linkage merge (see [`cluster_weight_sum`]).
fn greedy_merge_sum(
    dsu: &mut Dsu,
    w: &[Vec<i32>],
    n: usize,
    threshold: i32,
    cross_unit: &[Vec<bool>],
) {
    loop {
        let mut best: Option<(i32, usize, usize)> = None;
        for i in 0..n {
            for j in (i + 1)..n {
                if dsu.find(i) == dsu.find(j) {
                    continue;
                }
                let wi = cluster_weight_sum(w, dsu, i, j, cross_unit);
                match best {
                    Some((bw, bi, bj)) => {
                        if wi > bw || (wi == bw && (i < bi || (i == bi && j < bj))) {
                            best = Some((wi, i, j));
                        }
                    }
                    None => best = Some((wi, i, j)),
                }
            }
        }
        match best {
            Some((wi, i, j)) if wi >= threshold => dsu.union(i, j),
            _ => break,
        }
    }
}

/// Relationship-id prefix for hierarchy CONTAINS edges (kept distinct from
/// the component rel prefix so the two clears never cross).
const HIER_RELPREFIX: &str = "rel:hier:";

fn hier_rel(parts: &[&str]) -> String {
    let mut h = blake3::Hasher::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update(b"|");
    }
    format!("{HIER_RELPREFIX}{}", &h.finalize().to_hex()[..12])
}

/// Pass-2 service compilation: components (pass-1 clusters) merge again by
/// sum-linkage at SERVICE_THRESHOLD into SERVICE containers. The flat
/// component list is untouched — services are extra entities of kind
/// SERVICE with CONTAINS edges to their member component ids. Idempotent:
/// stale SERVICE/SUBSYSTEM entities and hierarchy rels are cleared first.
pub fn compile_services(
    store: &Store,
    comps: &[scc_core::Entity],
    component_weights: &[Vec<i32>],
    cross_unit: &[Vec<bool>],
    parent_per_comp: &BTreeMap<String, String>,
) -> Result<()> {
    clear_hierarchy(store)?;
    let n = comps.len();
    if n < 2 {
        return Ok(());
    }
    let mut dsu = Dsu::new(n);
    greedy_merge_sum(&mut dsu, component_weights, n, SERVICE_THRESHOLD, cross_unit);

    let mut unions: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..n {
        unions.entry(dsu.find(i)).or_default().push(i);
    }
    let repo = &store.repo_id;
    for members in unions.values() {
        if members.len() < 2 {
            continue;
        }
        // shared deployment unit -> the unit's name; else sorted names
        let dus: BTreeSet<&String> = members
            .iter()
            .filter_map(|i| parent_per_comp.get(comps[*i].name.as_str()))
            .collect();
        let name = if dus.len() == 1 {
            dus.into_iter().next().unwrap().clone()
        } else {
            let mut names: Vec<String> =
                members.iter().map(|i| comps[*i].name.clone()).collect();
            names.sort();
            names.join("+")
        };
        let id = scc_core::entity_id(repo, kinds::SERVICE, &name);
        let mut e = scc_core::Entity::new(id.clone(), kinds::SERVICE, name.clone());
        e.attr("layer", json!(LAYER_SERVICE));
        let member_names: Vec<String> =
            members.iter().map(|i| comps[*i].name.clone()).collect();
        e.attr("members", json!(member_names));
        store.insert_entity(&e, &[])?;
        for m in members {
            let target = scc_core::entity_id(repo, kinds::COMPONENT, &comps[*m].name);
            let r = scc_core::Relationship::new(
                hier_rel(&["hier_contains", &id, &target]),
                id.clone(),
                scc_core::predicates::CONTAINS,
                target,
                scc_core::Provenance::Extracted,
            );
            store.insert_relationship(&r, "")?;
        }
    }
    Ok(())
}

/// Delete stale SUBSYSTEM/SERVICE entities and hierarchy CONTAINS edges so
/// a recompile never accumulates containers.
pub(crate) fn clear_hierarchy(store: &Store) -> Result<()> {
    for kind in [kinds::SUBSYSTEM, kinds::SERVICE] {
        let ids: Vec<String> = store
            .entities_by_kind(kind)?
            .into_iter()
            .map(|e| e.id)
            .collect();
        if !ids.is_empty() {
            store.delete_entities(&ids)?;
        }
    }
    let rows = store.all_relationships()?;
    let ids: Vec<String> = rows
        .into_iter()
        .filter(|r| r.id.starts_with(HIER_RELPREFIX))
        .map(|r| r.id)
        .collect();
    for id in ids {
        store.delete_relationship(&id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scc_core::{entity_id, kinds, predicates, symbol_id, Entity, Provenance, Relationship};
    use scc_store::Store;

    /// Insert a FILE entity plus CONTAINS edges to its symbols; returns the
    /// symbol ids. Mirrors the components.rs test helper.
    fn insert_file_with_symbols(
        store: &Store,
        path: &str,
        symbols: &[&str],
    ) -> Vec<String> {
        let repo = store.repo_id.clone();
        let file_id = entity_id(&repo, kinds::FILE, path);
        store
            .insert_entity(&Entity::new(file_id.clone(), kinds::FILE, path), &[path.into()])
            .unwrap();
        let mut sym_ids = Vec::new();
        for s in symbols {
            let sid = symbol_id(&repo, path, s);
            store
                .insert_entity(&Entity::new(sid.clone(), kinds::SYMBOL, *s), &[path.into()])
                .unwrap();
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:contains:{}:{s}", path.replace('/', "_")),
                        file_id.clone(),
                        predicates::CONTAINS,
                        sid.clone(),
                        Provenance::Extracted,
                    ),
                    path,
                )
                .unwrap();
            sym_ids.push(sid);
        }
        sym_ids
    }

    fn store_for() -> (Store, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&tmp.path().join("scc.db"), &root).unwrap();
        (store, tmp)
    }

    fn compile(store: &Store) -> Vec<Entity> {
        let graph = RealityGraph::load(store).unwrap();
        crate::components::compile_components(&graph, store, &[], &[]).unwrap()
    }

    #[test]
    fn code_region_dir_splits_into_low_cohesion_modules() {
        // Two modules in the SAME top-level directory with no behavioral
        // evidence between them: the clusterer must SPLIT the dir into two
        // components (the longest-prefix assignment would have fused them
        // into one `src` blob).
        let (store, _t) = store_for();
        let repo = store.repo_id.clone();
        let _ = insert_file_with_symbols(&store, "src/checkout/cart.py", &["add_item"]);
        let _ = insert_file_with_symbols(&store, "src/pricing/tax.py", &["compute_tax"]);
        // no calls, no shared state, no shared exports — zero cross weight
        let _ = repo;

        let comps = compile(&store);
        let names: Vec<&str> = comps.iter().map(|c| c.name.as_str()).collect();
        assert!(
            names.contains(&"src/checkout"),
            "checkout module split out: {names:?}"
        );
        assert!(names.contains(&"src/pricing"), "pricing module split out: {names:?}");
        assert!(!names.contains(&"src"), "no fused src blob: {names:?}");
        assert!(names.contains(&"root"), "root shell stays: {names:?}");
        // both modules carry the code-region boundary (bare dirs)
        for n in ["src/checkout", "src/pricing"] {
            let c = comps.iter().find(|c| c.name == n).unwrap();
            assert_eq!(
                c.attributes["boundary_kind"],
                serde_json::json!(crate::components::BOUNDARY_CODE_REGION),
                "{n}"
            );
            assert_eq!(c.attributes["layer"], serde_json::json!(LAYER_CODE_REGION), "{n}");
        }
    }

    #[test]
    fn cross_dir_regions_merge_on_call_and_state_weight() {
        // Two modules in DIFFERENT directories with a call (+2) and shared
        // store writes (+4) = 6 >= MERGE_THRESHOLD: one component spanning
        // both dirs — behavior beats directory.
        let (store, _t) = store_for();
        let repo = store.repo_id.clone();
        let sa = insert_file_with_symbols(&store, "auth/session.py", &["create_session"]);
        let sb = insert_file_with_symbols(&store, "users/api.py", &["get_user"]);

        let db = entity_id(&repo, kinds::DATA_STORE, "db");
        store
            .insert_entity(&Entity::new(db.clone(), kinds::DATA_STORE, "db"), &["auth/session.py".into()])
            .unwrap();
        for (i, sym) in [sa[0].clone(), sb[0].clone()].iter().enumerate() {
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:w:{i}"),
                        sym.clone(),
                        predicates::WRITES,
                        db.clone(),
                        Provenance::Extracted,
                    ),
                    "auth/session.py",
                )
                .unwrap();
        }
        // create_session -> get_user (cross-dir semantic call)
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:call",
                    sa[0].clone(),
                    predicates::CALLS,
                    sb[0].clone(),
                    Provenance::Extracted,
                ),
                "auth/session.py",
            )
            .unwrap();

        let comps = compile(&store);
        let names: Vec<&str> = comps.iter().map(|c| c.name.as_str()).collect();
        assert!(
            names.contains(&"auth+users"),
            "cross-dir merge into one component: {names:?}"
        );
        assert!(!names.contains(&"auth"), "no auth shell: {names:?}");
        assert!(!names.contains(&"users"), "no users shell: {names:?}");
        let merged = comps.iter().find(|c| c.name == "auth+users").unwrap();
        let paths = merged.attributes["implementation"]["paths"]
            .as_array()
            .unwrap();
        assert_eq!(
            paths,
            &vec![serde_json::json!("auth"), serde_json::json!("users")],
            "merged component keeps both dirs as priors"
        );
        assert_eq!(merged.attributes["layer"], serde_json::json!(LAYER_COMPONENT));
    }

    #[test]
    fn library_architecture_comes_from_exports() {
        // A library whose modules share no calls: the public surface graph
        // (EXPORT entities + IMPLEMENTS hierarchy + LibrarySdk archetype
        // doubling) drives the merge. Three exported classes implementing
        // one exported interface across three modules -> one component.
        let (store, _t) = store_for();
        let repo = store.repo_id.clone();
        let si = insert_file_with_symbols(&store, "lib/contracts/base.py", &["iface"]);
        let sa = insert_file_with_symbols(&store, "lib/impl_a/a.py", &["ImplA"]);
        let sb = insert_file_with_symbols(&store, "lib/impl_b/b.py", &["ImplB"]);

        // exported interface + two exported implementations
        for (sym, name) in [
            (si[0].clone(), "iface"),
            (sa[0].clone(), "ImplA"),
            (sb[0].clone(), "ImplB"),
        ] {
            let exp = entity_id(&repo, kinds::EXPORT, name);
            store
                .insert_entity(&Entity::new(exp.clone(), kinds::EXPORT, name), &["lib/contracts/base.py".into()])
                .unwrap();
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:exp:{name}"),
                        sym.clone(),
                        predicates::EXPORTS,
                        exp,
                        Provenance::Extracted,
                    ),
                    "lib/contracts/base.py",
                )
                .unwrap();
        }
        // impls implement the exported interface (same facade target)
        for (i, sym) in [sa[0].clone(), sb[0].clone()].iter().enumerate() {
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:impl:{i}"),
                        sym.clone(),
                        predicates::IMPLEMENTS,
                        si[0].clone(),
                        Provenance::Extracted,
                    ),
                    "lib/impl_a/a.py",
                )
                .unwrap();
        }

        let comps = compile(&store);
        let names: Vec<&str> = comps.iter().map(|c| c.name.as_str()).collect();
        // 3 exports / 3 symbols = ratio 1.0 -> LibrarySdk -> public API
        // cohesion +8 per pair: all three modules merge into `lib`
        assert!(
            names.contains(&"lib"),
            "export-driven merge into one component: {names:?}"
        );
        assert!(!names.contains(&"lib/contracts"), "no contracts shell: {names:?}");
        assert!(!names.contains(&"lib/impl_a"), "no impl_a shell: {names:?}");
        assert!(!names.contains(&"lib/impl_b"), "no impl_b shell: {names:?}");
        assert!(names.contains(&"root"), "root shell stays: {names:?}");
        let lib = comps.iter().find(|c| c.name == "lib").unwrap();
        assert_eq!(lib.attributes["layer"], serde_json::json!(LAYER_COMPONENT));
    }

    /// Wave 11: two regions whose symbols are BOTH queue consumers (of
    /// DIFFERENT topics — no shared-topic event signal) cohere: cross-region
    /// call (2) + flow participation (3) + the invocation-surface cohesion
    /// signal (2) = 7 >= MERGE_THRESHOLD. The surface signal is decisive —
    /// without it the pair sits at 5 < 6 and stays split.
    #[test]
    // # trace:exempt — unit test (tests are not trace-worthy behavior)
    fn queue_consumer_regions_cohere_on_surface_family() {
        let (store, _t) = store_for();
        let repo = store.repo_id.clone();
        let sa = insert_file_with_symbols(&store, "jobs-a/consumer.py", &["consume_a"]);
        let sb = insert_file_with_symbols(&store, "jobs-b/consumer.py", &["consume_b"]);
        for (i, (sym, topic)) in [
            (sa[0].clone(), "orders".to_string()),
            (sb[0].clone(), "shipments".to_string()),
        ]
        .iter()
        .enumerate()
        {
            let topic_id = entity_id(&repo, kinds::TOPIC, topic);
            store
                .insert_entity(
                    &Entity::new(topic_id.clone(), kinds::TOPIC, topic),
                    &["jobs-a/consumer.py".into()],
                )
                .unwrap();
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:sub:{i}"),
                        sym.clone(),
                        predicates::SUBSCRIBES,
                        topic_id,
                        Provenance::Extracted,
                    ),
                    "jobs-a/consumer.py",
                )
                .unwrap();
        }
        // one cross-region call: 2 (call) + 3 (flow — the queue surfaces
        // seed flows) = 5; the surface-family signal (+2) pushes to 7
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:call",
                    sa[0].clone(),
                    predicates::CALLS,
                    sb[0].clone(),
                    Provenance::Extracted,
                ),
                "jobs-a/consumer.py",
            )
            .unwrap();

        let comps = compile(&store);
        let names: Vec<&str> = comps.iter().map(|c| c.name.as_str()).collect();
        assert!(
            names.contains(&"jobs-a+jobs-b"),
            "queue-consumer regions cohere into one component: {names:?}"
        );
        assert!(!names.contains(&"jobs-a"), "no jobs-a shell: {names:?}");
        assert!(!names.contains(&"jobs-b"), "no jobs-b shell: {names:?}");
    }

    #[test]
    fn deployment_unit_boundary_is_a_constraint() {
        // Two regions in DIFFERENT deployment units with call+state weight
        // 6 (>= MERGE_THRESHOLD but <= SERVICE_THRESHOLD): the constraint
        // blocks the merge — a split must respect deployment boundaries.
        let (store, _t) = store_for();
        let repo = store.repo_id.clone();
        let sa = insert_file_with_symbols(&store, "svc-a/worker.py", &["a_main"]);
        let sb = insert_file_with_symbols(&store, "svc-b/worker.py", &["b_main"]);

        for (name, ctx, file) in [
            ("du-a", "svc-a", "svc-a/worker.py"),
            ("du-b", "svc-b", "svc-b/worker.py"),
        ] {
            let mut du = Entity::new(entity_id(&repo, kinds::DEPLOYMENT_UNIT, name), kinds::DEPLOYMENT_UNIT, name);
            du.attr("build_context", serde_json::json!(ctx));
            store.insert_entity(&du, &[file.into()]).unwrap();
        }
        let db = entity_id(&repo, kinds::DATA_STORE, "db");
        store
            .insert_entity(&Entity::new(db.clone(), kinds::DATA_STORE, "db"), &["svc-a/worker.py".into()])
            .unwrap();
        for (i, sym) in [sa[0].clone(), sb[0].clone()].iter().enumerate() {
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:w:{i}"),
                        sym.clone(),
                        predicates::WRITES,
                        db.clone(),
                        Provenance::Extracted,
                    ),
                    "svc-a/worker.py",
                )
                .unwrap();
        }
        // one cross-unit call: 4 (state) + 2 (call) = 6 — blocked
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:call",
                    sa[0].clone(),
                    predicates::CALLS,
                    sb[0].clone(),
                    Provenance::Extracted,
                ),
                "svc-a/worker.py",
            )
            .unwrap();

        let comps = compile(&store);
        let names: Vec<&str> = comps.iter().map(|c| c.name.as_str()).collect();
        assert!(
            names.contains(&"du-a") && names.contains(&"du-b"),
            "cross-unit weight 6 must NOT merge: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n.contains('+')),
            "no cross-unit merge at 6: {names:?}"
        );
        // each unit member carries the unit as parent (deployment-unit
        // candidate names are the unit names)
        for n in ["du-a", "du-b"] {
            let c = comps.iter().find(|c| c.name == n).unwrap();
            assert_eq!(c.attributes["parent"].as_str().unwrap(), n, "{n}");
        }

        // ---- escalate to >12: four more cross-unit calls (2 each) make
        // the pair weight 4(state) + 10(five calls) = 14 > 12: the merge
        // is now ALLOWED and produces one cross-unit component ----
        for i in 0..4 {
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:call2:{i}"),
                        sa[0].clone(),
                        predicates::CALLS,
                        sb[0].clone(),
                        Provenance::Extracted,
                    ),
                    "svc-a/worker.py",
                )
                .unwrap();
        }
        let comps2 = compile(&store);
        let names2: Vec<&str> = comps2.iter().map(|c| c.name.as_str()).collect();
        assert!(
            names2.iter().any(|n| n.contains('+')),
            "cross-unit weight >12 merges: {names2:?}"
        );
    }

    #[test]
    fn archetype_prior_makes_cli_command_regions_cohere() {
        // CLI archetype: two command regions whose combined call+flow
        // weight (5) stays below the threshold — the CLI command prior
        // (+2) pushes them over 6, so command-region files cohere.
        let (store, _t) = store_for();
        let sa = insert_file_with_symbols(&store, "cmd/serve/serve.go", &["serve_root"]);
        let sb = insert_file_with_symbols(&store, "cmd/version/version.go", &["version_root"]);

        // cli-subcommand entrypoints + file attr on both (cli boundary +
        // Cli archetype + prior trait)
        for (sym, file) in [(sa[0].clone(), "cmd/serve/serve.go"), (sb[0].clone(), "cmd/version/version.go")] {
            let mut e = store.get_entity(&sym).unwrap().unwrap();
            e.attributes.insert("entrypoints".into(), serde_json::json!(["cli-subcommand"]));
            e.attributes.insert("file".into(), serde_json::json!(file));
            store.insert_entity(&e, &[file.into()]).unwrap();
        }
        // one cross-region call: 2 (call) + 3 (flow participation — the
        // cli entrypoints seed flows) = 5 < 6 without the prior
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:call",
                    sa[0].clone(),
                    predicates::CALLS,
                    sb[0].clone(),
                    Provenance::Extracted,
                ),
                "cmd/serve/serve.go",
            )
            .unwrap();

        let comps = compile(&store);
        let names: Vec<&str> = comps.iter().map(|c| c.name.as_str()).collect();
        // LCP("cmd/serve", "cmd/version") = "cmd" — the merged commands
        // component
        assert!(
            names.contains(&"cmd"),
            "cli command regions cohere into one component: {names:?}"
        );
        assert!(!names.contains(&"cmd/serve"), "no serve shell: {names:?}");
        assert!(!names.contains(&"cmd/version"), "no version shell: {names:?}");
        let cmd = comps.iter().find(|c| c.name == "cmd").unwrap();
        assert_eq!(
            cmd.attributes["boundary_kind"],
            serde_json::json!(crate::components::BOUNDARY_CLI),
            "merged cli regions keep the cli boundary"
        );
    }










    #[test]
    fn clustering_is_deterministic() {
        // Same graph compiled twice -> identical component names, layers,
        // parents, boundary kinds, and clustering scores.
        let (store, _t) = store_for();
        let repo = store.repo_id.clone();
        let sa = insert_file_with_symbols(&store, "auth/session.py", &["create_session"]);
        let sb = insert_file_with_symbols(&store, "users/api.py", &["get_user"]);
        let _ = insert_file_with_symbols(&store, "billing/invoice.py", &["make_invoice"]);
        let db = entity_id(&repo, kinds::DATA_STORE, "db");
        store
            .insert_entity(&Entity::new(db.clone(), kinds::DATA_STORE, "db"), &["auth/session.py".into()])
            .unwrap();
        for (i, sym) in [sa[0].clone(), sb[0].clone()].iter().enumerate() {
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:w:{i}"),
                        sym.clone(),
                        predicates::WRITES,
                        db.clone(),
                        Provenance::Extracted,
                    ),
                    "auth/session.py",
                )
                .unwrap();
        }
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:call",
                    sa[0].clone(),
                    predicates::CALLS,
                    sb[0].clone(),
                    Provenance::Extracted,
                ),
                "auth/session.py",
            )
            .unwrap();

        let c1 = compile(&store);
        let c2 = compile(&store);
        assert_eq!(c1.len(), c2.len());
        for (a, b) in c1.iter().zip(c2.iter()) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.attributes.get("layer"), b.attributes.get("layer"), "{}", a.name);
            assert_eq!(a.attributes.get("parent"), b.attributes.get("parent"), "{}", a.name);
            assert_eq!(
                a.attributes.get("boundary_kind"),
                b.attributes.get("boundary_kind"),
                "{}",
                a.name
            );
            assert_eq!(
                a.attributes.get("clustering_score"),
                b.attributes.get("clustering_score"),
                "{}",
                a.name
            );
        }
        // the merged auth+users component appears in both runs
        assert!(c1.iter().any(|c| c.name == "auth+users"));
        assert!(c2.iter().any(|c| c.name == "auth+users"));
    }
}
