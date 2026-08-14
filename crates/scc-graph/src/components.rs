//! Component Compiler (EPIC-040, docs/SYSTEM_DESIGN.md §7).
//!
//! Deterministic MVP signals:
//! 1. explicit intent (`.scc/intent.yaml` components with `paths`)
//! 2. package.json workspace members
//! 3. docker-compose service build contexts
//! 4. top-level directory boundaries
//!
//! Each component aggregates: responsibilities (routes owned, docstrings
//! INFERRED, intent DECLARED), ownership (store write edges), dependency
//! edges (cross-component calls), and implementation (paths/symbols).

use crate::{RealityGraph, Result};
use scc_core::kinds;
use scc_core::{entity_id, Provenance, Relationship};
use scc_store::Store;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

pub const RELPREFIX: &str = "rel:comp:";

/// Evidence class that created a component candidate (Wave 5, plan §27-30).
/// The `boundary_kind` attribute records it on every compiled component;
/// `code-region` is the bare top-level directory fallback and is never
/// authoritative architecture.
pub const BOUNDARY_DECLARED: &str = "declared";
pub const BOUNDARY_PACKAGE: &str = "package";
pub const BOUNDARY_DEPLOYMENT: &str = "deployment";
pub const BOUNDARY_CLI: &str = "cli";
pub const BOUNDARY_CODE_REGION: &str = "code-region";
pub const BOUNDARY_ROOT: &str = "root";

// ---- hierarchical architecture layers (Ontology phase) ----
pub const LAYER_CODE_REGION: &str = "code_region";
pub const LAYER_COMPONENT: &str = "component";
pub const LAYER_SUBSYSTEM: &str = "subsystem";
pub const LAYER_SERVICE: &str = "service";

/// Greedy merge threshold for the first (subsystem) pass.
pub const MERGE_THRESHOLD: i32 = 6;
/// Greedy merge threshold for the second (service) pass.
pub const SERVICE_THRESHOLD: i32 = 12;

/// Authority order for `boundary_kind` when one candidate is created by
/// several sources: declared intent > deployment units > workspace
/// packages > directory fallback. Deterministic (fixed precedence).
pub(crate) fn boundary_rank(kind: &str) -> u8 {
    match kind {
        BOUNDARY_DECLARED => 3,
        BOUNDARY_DEPLOYMENT => 2,
        BOUNDARY_CLI => 2,
        BOUNDARY_PACKAGE => 1,
        _ => 0,
    }
}

pub fn rel(parts: &[&str]) -> String {
    let mut h = blake3::Hasher::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update(b"|");
    }
    format!("{RELPREFIX}{}", &h.finalize().to_hex()[..12])
}

pub fn parse_prov(s: &str) -> Provenance {
    match s {
        "RESOLVED" => Provenance::Resolved,
        "EXTRACTED" => Provenance::Extracted,
        "DECLARED" => Provenance::Declared,
        "OBSERVED" => Provenance::Observed,
        "INFERRED" => Provenance::Inferred,
        _ => Provenance::Inferred,
    }
}

pub fn prov_rank(p: Provenance) -> u8 {
    match p {
        Provenance::Resolved => 4,
        Provenance::Observed => 4,
        Provenance::Extracted => 3,
        Provenance::Declared => 2,
        Provenance::Inferred => 1,
        Provenance::Stale => 0,
    }
}

#[derive(Debug, Clone)]
pub struct ComponentCandidate {
    pub name: String,
    pub dirs: Vec<String>,
    /// Evidence class that created this candidate (one of the
    /// `BOUNDARY_*` constants).
    pub boundary_kind: String,
}

/// Determine the component for a path: longest matching dir prefix wins.
/// Returns a `String` (candidate names are owned).
pub fn component_for_path(path: &str, candidates: &[ComponentCandidate]) -> String {
    let mut best: Option<(String, usize)> = None;
    for c in candidates {
        for d in &c.dirs {
            let d = d.trim_end_matches('/');
            if d.is_empty() {
                continue;
            }
            if path == d || path.starts_with(&format!("{d}/")) {
                let len = d.len();
                if best.as_ref().map(|(_, bl)| len > *bl).unwrap_or(true) {
                    best = Some((c.name.clone(), len));
                }
            }
        }
    }
    match best {
        Some((name, _)) => name,
        None => {
            // root-level file -> "root" component; nested -> first segment
            if path.contains('/') {
                let seg = path.split('/').next().unwrap_or("root");
                if seg == ".scc" {
                    "root".to_string()
                } else {
                    seg.to_string()
                }
            } else {
                "root".to_string()
            }
        }
    }
}

/// Normalize a manifest-declared directory: strip a leading `./` and any
/// trailing slashes. Deterministic.
fn norm_manifest_dir(p: &str) -> String {
    let t = p.trim();
    let t = t.strip_prefix("./").unwrap_or(t);
    t.trim_end_matches('/').to_string()
}

/// Collect the `"..."` string values of a (possibly multi-line) TOML array
/// assigned to `key` — e.g. `members = [\n "a",\n "b",\n]`. Stops at the
/// first `]` that is not inside a string literal. `key` must be a bare
/// token (`exclude_members` must not match `members`).
fn toml_string_array(table: &str, key: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = table;
    while let Some(pos) = rest.find(key) {
        let before_ok = pos == 0
            || !rest[..pos]
                .chars()
                .last()
                .map(|c| c.is_alphanumeric() || c == '_')
                .unwrap_or(false);
        let after_trim = rest[pos + key.len()..].trim_start();
        if before_ok && after_trim.starts_with('=') {
            let mut scan = &after_trim[1..];
            loop {
                let open = scan.find('"');
                let close = scan.find(']');
                match (open, close) {
                    (Some(q), Some(c)) if q < c => match scan[q + 1..].find('"') {
                        Some(e) => {
                            out.push(scan[q + 1..q + 1 + e].to_string());
                            scan = &scan[q + 1 + e + 1..];
                        }
                        None => break,
                    },
                    _ => break, // ']' first (or unterminated string): done
                }
            }
            break;
        }
        rest = &rest[pos + key.len()..];
    }
    out
}

/// Return the text of a TOML table (from its header line up to the next
/// top-level header or EOF). `header` must match exactly, e.g.
/// `[workspace]` (so `[workspace.dependencies]` is not a false match).
fn toml_table(text: &str, header: &str) -> Option<String> {
    let mut start: Option<usize> = None;
    let mut end = text.lines().count();
    for (i, line) in text.lines().enumerate() {
        let l = line.trim();
        if start.is_none() {
            if l == header {
                start = Some(i);
            }
            continue;
        }
        if l.starts_with('[') && l.ends_with(']') {
            end = i;
            break;
        }
    }
    let start = start?;
    Some(
        text.lines()
            .skip(start)
            .take(end.saturating_sub(start))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// `[package] name = "..."` from a crate manifest (only inside the
/// `[package]` table).
fn toml_package_name(text: &str) -> Option<String> {
    let table = toml_table(text, "[package]")?;
    for line in table.lines() {
        let l = line.trim();
        if let Some(eq) = l.find('=') {
            if l[..eq].trim() == "name" {
                let v = l[eq + 1..].trim();
                if let Some(s) = v.strip_prefix('"') {
                    if let Some(e) = s.find('"') {
                        return Some(s[..e].to_string());
                    }
                }
            }
        }
    }
    None
}

/// go.work `use` directives (block form `use ( ... )` and inline form
/// `use ./path`) as normalized module directories.
fn gowork_use_dirs(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut i = 0;
    while i < lines.len() {
        let toks: Vec<&str> = lines[i].split_whitespace().collect();
        if toks.first() == Some(&"use") {
            if toks.len() >= 2 && toks[1].starts_with('(') {
                // `use ( ./a ./b )` on one line, or `use (` + block
                for t in &toks[1..] {
                    let t = t.trim_matches(|c| c == '(' || c == ')');
                    if !t.is_empty() {
                        out.push(norm_manifest_dir(t));
                    }
                }
                i += 1;
                while i < lines.len() && lines[i] != ")" {
                    let l = lines[i];
                    if !l.is_empty() && !l.starts_with("//") {
                        out.push(norm_manifest_dir(l));
                    }
                    i += 1;
                }
            } else if toks.len() >= 2 {
                out.push(norm_manifest_dir(toks[1]));
            } else {
                // bare `use` + block on following lines
                i += 1;
                while i < lines.len() && lines[i] != ")" {
                    let l = lines[i];
                    if !l.is_empty() && !l.starts_with("//") {
                        out.push(norm_manifest_dir(l));
                    }
                    i += 1;
                }
            }
        }
        i += 1;
    }
    out
}

/// Resolve a cargo package's name from its own Cargo.toml `[package] name`;
/// falls back to the directory name when the manifest is missing or
/// unparseable.
fn crate_name(root: &std::path::Path, dir: &str) -> String {
    if let Ok(text) = std::fs::read_to_string(root.join(dir).join("Cargo.toml")) {
        if let Some(n) = toml_package_name(&text) {
            if !n.is_empty() {
                return n;
            }
        }
    }
    dir.rsplit('/').next().unwrap_or(dir).to_string()
}

/// Cargo workspace members from the root `Cargo.toml` `[workspace]` table:
/// `members = [...]` (expanding `dir/*` globs against the filesystem,
/// sorted) minus `exclude = [...]`. Returns (crate name, member dir) pairs.
fn cargo_workspace_members(root: &std::path::Path) -> Vec<(String, String)> {
    let text = match std::fs::read_to_string(root.join("Cargo.toml")) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let Some(ws) = toml_table(&text, "[workspace]") else {
        return Vec::new();
    };
    let members = toml_string_array(&ws, "members");
    if members.is_empty() {
        return Vec::new();
    }
    let excludes: BTreeSet<String> =
        toml_string_array(&ws, "exclude").into_iter().map(|e| norm_manifest_dir(&e)).collect();
    let mut out: Vec<(String, String)> = Vec::new();
    for m in members {
        let m = norm_manifest_dir(&m);
        if m.is_empty() || m == "." || excludes.contains(&m) {
            continue;
        }
        if let Some(glob_dir) = m.strip_suffix("/*") {
            let base = root.join(glob_dir);
            let mut dirs: Vec<String> = std::fs::read_dir(&base)
                .map(|rd| {
                    rd.filter_map(|e| e.ok())
                        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                        .filter_map(|e| e.file_name().into_string().ok())
                        .filter(|n| !n.starts_with('.'))
                        .filter(|n| {
                            root.join(format!("{glob_dir}/{n}")).join("Cargo.toml").is_file()
                        })
                        .collect()
                })
                .unwrap_or_default();
            dirs.sort();
            for d in dirs {
                let dir = format!("{glob_dir}/{d}");
                if excludes.contains(&dir) {
                    continue;
                }
                out.push((crate_name(root, &dir), dir));
            }
        } else if root.join(&m).is_dir() {
            out.push((crate_name(root, &m), m));
        }
        // stale (missing) members are skipped deterministically
    }
    out
}

fn merge_manifest_candidate(out: &mut Vec<ComponentCandidate>, name: String, dir: String) {
    if let Some(c) = out.iter_mut().find(|c| c.name == name) {
        if !c.dirs.contains(&dir) {
            c.dirs.push(dir);
        }
        if boundary_rank(BOUNDARY_PACKAGE) > boundary_rank(&c.boundary_kind) {
            c.boundary_kind = BOUNDARY_PACKAGE.to_string();
        }
    } else {
        out.push(ComponentCandidate {
            name,
            dirs: vec![dir],
            boundary_kind: BOUNDARY_PACKAGE.to_string(),
        });
    }
}

/// Workspace-member component candidates from manifests the config
/// extractors do not model: Cargo.toml `[workspace] members` (with
/// `[package] name` for crate names) and go.work `use (...)` modules.
/// File-based and deterministic; member dirs later out-prefix the top-dir
/// fallback via longest-prefix matching in [`component_for_path`].
fn manifest_package_candidates(root: &std::path::Path) -> Vec<ComponentCandidate> {
    let mut out: Vec<ComponentCandidate> = Vec::new();
    for (name, dir) in cargo_workspace_members(root) {
        merge_manifest_candidate(&mut out, name, dir);
    }
    for dir in gowork_use_dirs(&std::fs::read_to_string(root.join("go.work")).unwrap_or_default()) {
        if dir.is_empty() || dir == "." || !root.join(&dir).is_dir() {
            continue; // stale module: deterministic skip
        }
        let name = dir.rsplit('/').next().unwrap_or(&dir).to_string();
        merge_manifest_candidate(&mut out, name, dir);
    }
    out
}

/// Directories holding CLI registrations (clap/cobra/argparse/click):
/// the parent dir of every file with `cli-subcommand` entrypoints and of
/// every file whose symbols carry `cli-subcommand` entrypoints or
/// `cli_flags`. Each such directory becomes its own `boundary_kind=cli`
/// component (the CLI package), so the clusterer never folds it into a
/// generic top-level code-region component. Root-level registration files
/// map to the `root` dir. Deterministic (BTreeSet, sorted).
fn cli_package_dirs(graph: &RealityGraph) -> BTreeSet<String> {
    let mut dirs: BTreeSet<String> = BTreeSet::new();
    let note_file = |fname: &str, dirs: &mut BTreeSet<String>| {
        let dir = match fname.rsplit_once('/') {
            Some((parent, _)) if !parent.is_empty() => parent.to_string(),
            _ => "root".to_string(),
        };
        dirs.insert(dir);
    };
    // file-level cli entrypoints (no symbol, e.g. clap Subcommand enum or
    // cobra registrations landing on the file entity)
    for f in graph.entities_of_kind(kinds::FILE) {
        let has_cli = f
            .attributes
            .get("entrypoints")
            .and_then(|v| v.as_array())
            .map(|eps| eps.iter().any(|e| e.as_str() == Some("cli-subcommand")))
            .unwrap_or(false);
        if has_cli {
            note_file(&f.name, &mut dirs);
        }
    }
    // symbol-level cli evidence: `cli-subcommand` entrypoints or `cli_flags`
    for e in graph.entities_of_kind(kinds::SYMBOL) {
        let is_cli = e
            .attributes
            .get("entrypoints")
            .and_then(|v| v.as_array())
            .map(|eps| eps.iter().any(|e| e.as_str() == Some("cli-subcommand")))
            .unwrap_or(false)
            || e
                .attributes
                .get("cli_flags")
                .and_then(|v| v.as_array())
                .map(|fl| !fl.is_empty())
                .unwrap_or(false);
        if is_cli {
            if let Some(file) = e.attributes.get("file").and_then(|v| v.as_str()) {
                note_file(file, &mut dirs);
            }
        }
    }
    dirs
}

/// Build the component candidates from evidence (declared intent, workspace
/// packages, deployment-unit build contexts, CLI registration dirs, and the
/// bare top-level directory fallback). Shared by the component compiler and
/// the semantic clusterer so both see the same boundary priors.
// trace:v1 id=impl.scc.components work=WORK-SCC-001 satisfies=REQ-SCC-IR
pub(crate) fn build_candidates(
    graph: &RealityGraph,
    store: &Store,
    intent: &[(String, serde_json::Value)],
) -> Vec<ComponentCandidate> {
    // declared first so they win authority
    let mut candidates: Vec<ComponentCandidate> = Vec::new();
    let mut declared_names: HashSet<String> = HashSet::new();
    for (source, claim) in intent {
        if source == "component" {
            let name = claim["name"].as_str().unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }
            declared_names.insert(name.clone());
            let mut dirs: Vec<String> = Vec::new();
            if let Some(paths) = claim["paths"].as_array() {
                for p in paths {
                    if let Some(s) = p.as_str() {
                        dirs.push(s.to_string());
                    }
                }
            }
            dirs.push(name.clone()); // implicit: declared name == directory
            candidates.push(ComponentCandidate {
                name,
                dirs,
                boundary_kind: BOUNDARY_DECLARED.to_string(),
            });
        }
    }
    // workspace packages
    for pkg in graph.entities_of_kind(kinds::PACKAGE) {
        if let Some(path) = pkg.attributes.get("path").and_then(|v| v.as_str()) {
            let name = pkg.name.clone();
            if let Some(c) = candidates.iter_mut().find(|c| c.name == name) {
                if !c.dirs.contains(&path.to_string()) {
                    c.dirs.push(path.to_string());
                }
                if boundary_rank(BOUNDARY_PACKAGE) > boundary_rank(&c.boundary_kind) {
                    c.boundary_kind = BOUNDARY_PACKAGE.to_string();
                }
            } else {
                candidates.push(ComponentCandidate {
                    name,
                    dirs: vec![path.to_string()],
                    boundary_kind: BOUNDARY_PACKAGE.to_string(),
                });
            }
        }
    }
    // workspace members from manifests the config extractors don't model
    // (Cargo.toml `[workspace] members`, go.work `use (...)`): file-based,
    // deterministic; member dirs win over the top-dir fallback below via
    // longest-prefix matching in `component_for_path`.
    for cand in manifest_package_candidates(&store.root) {
        for dir in cand.dirs {
            merge_manifest_candidate(&mut candidates, cand.name.clone(), dir);
        }
    }
    // deployment units with build contexts
    for du in graph.entities_of_kind(kinds::DEPLOYMENT_UNIT) {
        if let Some(ctx) = du.attributes.get("build_context").and_then(|v| v.as_str()) {
            if ctx == "." || ctx == "./" {
                continue;
            }
            let ctx = ctx.trim_start_matches("./");
            let name = du.name.clone();
            if let Some(c) = candidates.iter_mut().find(|c| c.name == name) {
                if !c.dirs.contains(&ctx.to_string()) {
                    c.dirs.push(ctx.to_string());
                }
                if boundary_rank(BOUNDARY_DEPLOYMENT) > boundary_rank(&c.boundary_kind) {
                    c.boundary_kind = BOUNDARY_DEPLOYMENT.to_string();
                }
            } else {
                candidates.push(ComponentCandidate {
                    name,
                    dirs: vec![ctx.to_string()],
                    boundary_kind: BOUNDARY_DEPLOYMENT.to_string(),
                });
            }
        }
    }
    // CLI package dirs (clap/cobra/argparse/click registrations) become
    // their own `cli`-boundary components, so the top-dir fallback below
    // never demotes them to a generic code-region component
    for dir in cli_package_dirs(graph) {
        if let Some(c) = candidates.iter_mut().find(|c| c.name == dir) {
            if !c.dirs.contains(&dir) {
                c.dirs.push(dir);
            }
            if boundary_rank(BOUNDARY_CLI) > boundary_rank(&c.boundary_kind) {
                c.boundary_kind = BOUNDARY_CLI.to_string();
            }
        } else {
            candidates.push(ComponentCandidate {
                name: dir.clone(),
                dirs: vec![dir.clone()],
                boundary_kind: BOUNDARY_CLI.to_string(),
            });
        }
    }
    // top-level source dirs so nothing is orphaned; root-level files all
    // belong to the "root" component
    let mut top_dirs: HashSet<String> = HashSet::new();
    for f in graph.entities_of_kind(kinds::FILE) {
        if f.name.contains('/') {
            if let Some(seg) = f.name.split('/').next() {
                if !seg.is_empty() {
                    top_dirs.insert(seg.to_string());
                }
            }
        }
    }
    top_dirs.insert("root".to_string());
    for d in &top_dirs {
        if !candidates.iter().any(|c| c.name == *d) {
            candidates.push(ComponentCandidate {
                name: d.clone(),
                dirs: vec![d.clone()],
                boundary_kind: if d == "root" {
                    BOUNDARY_ROOT.to_string()
                } else {
                    BOUNDARY_CODE_REGION.to_string()
                },
            });
        }
    }
    candidates
}

// trace:v1 id=impl.scc.components.compile work=WORK-SCC-001 satisfies=REQ-SCC-IR
pub fn compile_components(
    graph: &RealityGraph,
    store: &Store,
    intent: &[(String, serde_json::Value)],
    pairs: &[crate::cochange::CochangePair],
) -> Result<Vec<scc_core::Entity>> {
    let repo_id = &store.repo_id;
    let candidates = build_candidates(graph, store, intent);

    // ---- semantic clustering over ATOMIC regions (generalization wave):
    // files belong to architecture because of BEHAVIOR, not directories.
    // The clusterer may SPLIT a top-level dir (its sub-regions land in
    // different clusters when intra-dir cohesion is low) and may MERGE
    // regions across dirs (high call+state weight) — the longest-prefix
    // path assignment is replaced by the clustering result. Path/package/
    // deployment stay as constraints and priors: authoritative boundaries
    // are atomic regions, and merging across deployment units requires
    // weight > SERVICE_THRESHOLD. ----
    let du_ctxs: Vec<(String, String)> = graph
        .entities_of_kind(kinds::DEPLOYMENT_UNIT)
        .into_iter()
        .filter_map(|du| {
            let ctx = du.attributes.get("build_context").and_then(|v| v.as_str())?;
            if ctx == "." || ctx == "./" {
                return None;
            }
            Some((du.name.clone(), ctx.trim_start_matches("./").to_string()))
        })
        .collect();
    let mut du_ctxs_sorted = du_ctxs;
    du_ctxs_sorted.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
    let clustering = crate::clustering::cluster_components(
        graph,
        store,
        intent,
        &candidates,
        pairs,
        &du_ctxs_sorted,
    )?;

    // ---- file/symbol assignment from the clustering result ----
    let files_in_component: BTreeMap<String, Vec<String>> = clustering.files_in_component;
    // symbol → component map (cluster membership)
    let symbol_component: HashMap<String, String> = clustering.symbol_component;
    let parent_per_comp: BTreeMap<String, String> = clustering.parent_per_comp;

    // ---- aggregation ----
    let mut responsibilities: BTreeMap<String, Vec<(String, Provenance, f64)>> = BTreeMap::new();
    // Ownership claims: (target entity id, provenance, confidence, evidence).
    // Write edges keep their own provenance; intent ownership stays DECLARED
    // — the compiler never promotes a claim's provenance (P0, §5).
    type OwnershipClaim = (String, Provenance, f64, Vec<String>);
    let mut owns: BTreeMap<String, Vec<OwnershipClaim>> = BTreeMap::new();
    let mut depends: BTreeMap<String, Vec<(String, Provenance, f64, u32)>> = BTreeMap::new();
    let mut symbols_per_comp: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut evidence_per_comp: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut retries_per_comp: BTreeMap<String, Vec<String>> = BTreeMap::new();

    // route ownership: handler handles route (RESOLVED responsibility)
    for (sym_id, comp) in &symbol_component {
        for r in graph.out_pred(sym_id, scc_core::predicates::HANDLES) {
            if let Some(route) = graph.entities.get(&r.object) {
                let method = route.attributes.get("method").and_then(|v| v.as_str()).unwrap_or("");
                let path = route.attributes.get("path").and_then(|v| v.as_str()).unwrap_or("");
                responsibilities.entry(comp.clone()).or_default().push((
                    format!("Handles {method} {path}"),
                    Provenance::Resolved,
                    1.0,
                ));
            }
        }
    }
    // store write ownership (RESOLVED, evidence = the write edges' evidence)
    for (sym_id, comp) in &symbol_component {
        for r in graph.out_pred(sym_id, scc_core::predicates::WRITES) {
            owns.entry(comp.clone()).or_default().push((
                r.object.clone(),
                r.provenance,
                r.confidence,
                r.evidence.clone(),
            ));
        }
    }
    // cross-component call dependencies
    for (sym_id, comp) in &symbol_component {
        for r in graph.out_pred(sym_id, scc_core::predicates::CALLS) {
            if let Some(target_comp) = symbol_component.get(&r.object) {
                if target_comp != comp {
                    let entry = depends.entry(comp.clone()).or_default();
                    if let Some((_, p, c, n)) = entry
                        .iter_mut()
                        .find(|(t, _, _, _)| t == target_comp)
                    {
                        *n += 1;
                        if prov_rank(r.provenance) > prov_rank(*p) {
                            *p = r.provenance;
                        }
                        *c = c.max(r.confidence);
                    } else {
                        entry.push((target_comp.clone(), r.provenance, r.confidence, 1));
                    }
                }
            }
        }
    }
    // symbols/evidence/retries per component (sorted for determinism —
    // aggregation iterates a HashMap)
    for (sym_id, comp) in &symbol_component {
        if let Some(e) = graph.entities.get(sym_id) {
            symbols_per_comp
                .entry(comp.clone())
                .or_default()
                .push(e.name.clone());
            evidence_per_comp
                .entry(comp.clone())
                .or_default()
                .extend(e.evidence.clone());
            if let Some(rp) = e.attributes.get("retry_policy").and_then(|v| v.as_str()) {
                retries_per_comp
                    .entry(comp.clone())
                    .or_default()
                    .push(format!("{} ({rp})", e.name));
            }
        }
    }
    for v in symbols_per_comp.values_mut() {
        v.sort();
        v.dedup();
    }
    for v in evidence_per_comp.values_mut() {
        v.sort();
        v.dedup();
    }
    for v in retries_per_comp.values_mut() {
        v.sort();
        v.dedup();
    }

    // ---- clustering evidence (Wave 5, plan §28): weighted per-candidate
    // signals feeding `clustering_score`. Deterministic by construction:
    // every loop below iterates a sorted collection (entities_of_kind,
    // files_in_component, or the sorted (symbol, component) list), never a
    // raw HashMap.
    let mut symbol_list: Vec<(String, String)> = symbol_component
        .iter()
        .map(|(s, c)| (s.clone(), c.clone()))
        .collect();
    symbol_list.sort();
    // (component, store) -> distinct symbols in the component writing it
    let mut shared_writes: BTreeMap<(String, String), HashSet<String>> = BTreeMap::new();
    // component -> HANDLES edges from its symbols (entrypoint ownership)
    let mut entrypoints: BTreeMap<String, usize> = BTreeMap::new();
    // component -> PUBLISHES/CONSUMES edges from its symbols (event ownership)
    let mut events: BTreeMap<String, usize> = BTreeMap::new();
    // component -> (internal calls, total calls) from its symbols
    let mut calls: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for (sym_id, comp) in &symbol_list {
        for r in graph.out_pred(sym_id, scc_core::predicates::WRITES) {
            // data entities (repo://r/data/store.entity) resolve to their
            // owning store so two symbols writing db.users and db.orders
            // count as shared ownership of the same store
            let target = if r.object.contains("/data/") {
                graph
                    .entities
                    .get(&r.object)
                    .and_then(|e| e.attributes.get("store"))
                    .and_then(|v| v.as_str())
                    .map(|s| entity_id(repo_id, kinds::DATA_STORE, s))
                    .unwrap_or_else(|| r.object.clone())
            } else {
                r.object.clone()
            };
            shared_writes
                .entry((comp.clone(), target))
                .or_default()
                .insert(sym_id.clone());
        }
        for _r in graph.out_pred(sym_id, scc_core::predicates::HANDLES) {
            *entrypoints.entry(comp.clone()).or_insert(0) += 1;
        }
        for _r in graph
            .out_pred(sym_id, scc_core::predicates::PUBLISHES)
            .into_iter()
            .chain(graph.out_pred(sym_id, scc_core::predicates::CONSUMES))
        {
            *events.entry(comp.clone()).or_insert(0) += 1;
        }
        for r in graph.out_pred(sym_id, scc_core::predicates::CALLS) {
            let e = calls.entry(comp.clone()).or_insert((0, 0));
            e.1 += 1;
            if symbol_component.get(&r.object) == Some(comp) {
                e.0 += 1;
            }
        }
    }
    // component -> route entities contained in its files (route ownership)
    let mut route_entities: BTreeMap<String, usize> = BTreeMap::new();
    for (comp, files) in &files_in_component {
        for fid in files {
            for r in graph.out_pred(fid, scc_core::predicates::CONTAINS) {
                if graph
                    .entities
                    .get(&r.object)
                    .map(|e| e.kind == kinds::ROUTE)
                    .unwrap_or(false)
                {
                    *route_entities.entry(comp.clone()).or_insert(0) += 1;
                }
            }
        }
    }
    // deployment units with build contexts, most specific (longest context)
    // first so `parent` picks the tightest unit; name tiebreak for
    // determinism — the per-component `parent` attribute comes from the
    // clusterer's parent_per_comp (cluster dirs vs unit build contexts).

    // intent responsibilities / ownership (DECLARED): attach to every
    // cluster whose name matches the claim or whose dirs contain the
    // claim's paths — a merged/split component keeps its members' declared
    // intent (deterministic: clusters are sorted by name).
    let mut intent_resp: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut intent_owns: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (source, claim) in intent {
        if source == "component" {
            let name = claim["name"].as_str().unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }
            let paths: Vec<String> = claim["paths"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|p| p.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let mut targets: Vec<String> = Vec::new();
            for c in &clustering.clusters {
                // the declared component's REGION is a member of the
                // cluster (a merged/split component keeps its members'
                // declared intent); also accept explicit dir containment
                // and an exact name match
                let member_region = c.member_regions.iter().any(|&m| {
                    clustering
                        .regions
                        .get(m)
                        .map(|r| r.name == name)
                        .unwrap_or(false)
                });
                let covered = !paths.is_empty()
                    && paths.iter().all(|p| {
                        c.dirs.iter().any(|d| {
                            let d = d.trim_end_matches('/');
                            p == d || p.starts_with(&format!("{d}/")) || d.starts_with(&format!("{p}/"))
                        })
                    });
                if member_region || covered || c.name == name {
                    targets.push(c.name.clone());
                }
            }
            if let Some(resp) = claim["responsibility"].as_array() {
                for t in &targets {
                    for r in resp {
                        if let Some(s) = r.as_str() {
                            intent_resp.entry(t.clone()).or_default().push(s.to_string());
                        }
                    }
                }
            }
            if let Some(o) = claim["owns"].as_array() {
                for t in &targets {
                    for ow in o {
                        if let Some(s) = ow.as_str() {
                            intent_owns.entry(t.clone()).or_default().push(s.to_string());
                        }
                    }
                }
            }
        }
    }

    // ---- build component entities (one per clustering result) ----
    let comp_names: Vec<String> = clustering
        .clusters
        .iter()
        .map(|c| c.name.clone())
        .collect();

    let mut out: Vec<scc_core::Entity> = Vec::new();
    for name in comp_names {
        let id = entity_id(repo_id, kinds::COMPONENT, &name);
        let mut e = scc_core::Entity::new(id.clone(), kinds::COMPONENT, name.clone());

        let mut resp: Vec<serde_json::Value> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let push_resp = |text: String, prov: Provenance, conf: f64,
                             resp: &mut Vec<serde_json::Value>, seen: &mut HashSet<String>| {
            if !seen.insert(text.clone()) {
                return;
            }
            resp.push(json!({
                "text": text,
                "provenance": prov.as_str(),
                "confidence": conf,
            }));
        };
        if let Some(irs) = intent_resp.get(&name) {
            for s in irs {
                push_resp(s.clone(), Provenance::Declared, 1.0, &mut resp, &mut seen);
            }
        }
        if let Some(rs) = responsibilities.get(&name) {
            let mut sorted = rs.clone();
            sorted.sort_by(|a, b| {
                prov_rank(b.1)
                    .cmp(&prov_rank(a.1))
                    .then_with(|| a.0.cmp(&b.0))
            });
            for (text, prov, conf) in sorted {
                push_resp(text, prov, conf, &mut resp, &mut seen);
            }
        }
        if resp.is_empty() {
            resp.push(json!({
                "text": format!("Hosts the {} code module", name),
                "provenance": Provenance::Inferred.as_str(),
                "confidence": 0.5,
            }));
        }
        e.attr("responsibility", json!(resp));

        let cluster = clustering
            .clusters
            .iter()
            .find(|c| c.name == name)
            .expect("every compiled component is a clustering result");
        let dirs = cluster.dirs.clone();
        e.attr(
            "implementation",
            json!({
                "paths": dirs,
                "symbols": symbols_per_comp.get(&name).cloned().unwrap_or_default(),
            }),
        );

        // ---- Wave 5: boundary kind + weighted clustering score ----
        let mut score: f64 = match cluster.boundary_kind.as_str() {
            BOUNDARY_DEPLOYMENT => 5.0,
            BOUNDARY_PACKAGE => 4.0,
            // CLI packages are real evidence-backed boundaries (like
            // workspace packages), not bare directory fallbacks
            BOUNDARY_CLI => 4.0,
            BOUNDARY_CODE_REGION | BOUNDARY_ROOT => 1.0,
            // declared intent carries its authority in `boundary_kind`;
            // the clustering score only counts graph evidence (plan §28)
            _ => 0.0,
        };
        if shared_writes
            .iter()
            .any(|((c, _), syms)| c == &name && syms.len() >= 2)
        {
            score += 4.0; // shared data ownership
        }
        if entrypoints.get(&name).copied().unwrap_or(0) > 0 {
            score += 4.0; // entrypoint ownership (route handlers)
        }
        if route_entities.get(&name).copied().unwrap_or(0) > 0 {
            score += 3.0; // route ownership
        }
        if events.get(&name).copied().unwrap_or(0) > 0 {
            score += 3.0; // event ownership
        }
        if let Some((internal, total)) = calls.get(&name) {
            if *total > 0 {
                score += 3.0 * (*internal as f64 / *total as f64); // cohesion
            }
        }
        let dir_refs: Vec<&str> = dirs.iter().map(|d| d.as_str()).collect();
        let co_pairs = pairs
            .iter()
            .filter(|p| {
                crate::cochange::file_in_paths(&p.a, &dir_refs)
                    && crate::cochange::file_in_paths(&p.b, &dir_refs)
            })
            .count();
        score += 2.0 * co_pairs as f64; // co-change (+2 per pair inside)
        score = (score * 1000.0).round() / 1000.0;
        e.attr("boundary_kind", json!(cluster.boundary_kind.clone()));
        e.attr("layer", json!(cluster.layer.clone()));
        e.attr("clustering_score", json!(score));
        if let Some(parent) = parent_per_comp.get(&name) {
            e.attr("parent", json!(parent));
        }

        // typed ownership claims: (target, provenance, confidence, evidence)
        // — intent stays DECLARED, write edges keep their own provenance
        let mut owned_claims: Vec<(String, Provenance, f64, Vec<String>)> = owns
            .get(&name)
            .cloned()
            .unwrap_or_default();
        if let Some(ios) = intent_owns.get(&name) {
            for target in ios {
                let target_l = target.to_ascii_lowercase();
                let matched = graph
                    .entities_of_kind(kinds::DATA_STORE)
                    .into_iter()
                    .chain(graph.entities_of_kind(kinds::DATA_ENTITY))
                    .find(|e| e.name.to_ascii_lowercase() == target_l)
                    .map(|e| e.id.clone());
                if let Some(mid) = matched {
                    owned_claims.push((mid, Provenance::Declared, 1.0, Vec::new()));
                }
            }
        }
        owned_claims.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| prov_rank(b.1).cmp(&prov_rank(a.1)))
        });
        let owned_json: Vec<serde_json::Value> = owned_claims
            .iter()
            .map(|(t, p, c, ev)| {
                json!({
                    "target": t,
                    "provenance": p.as_str(),
                    "confidence": c,
                    "evidence": ev,
                })
            })
            .collect();
        e.attr("owns", json!(owned_json));

        let deps: Vec<serde_json::Value> = depends
            .get(&name)
            .map(|v| {
                let mut sorted = v.clone();
                sorted.sort_by(|a, b| b.3.cmp(&a.3).then_with(|| a.0.cmp(&b.0)));
                sorted
                    .into_iter()
                    .map(|(t, p, c, n)| {
                        json!({"target": t, "provenance": p.as_str(), "confidence": c, "call_count": n})
                    })
                    .collect()
            })
            .unwrap_or_default();
        e.attr("depends_on", json!(deps));
        e.attr(
            "retries",
            json!(retries_per_comp.get(&name).cloned().unwrap_or_default()),
        );

        e.evidence = evidence_per_comp.get(&name).cloned().unwrap_or_default();
        out.push(e);
    }

    // ---- component-level relationships (derived; carry the evidence of
    // the underlying source facts) ----
    clear_component_relationships(store)?;
    let mut rels: Vec<(Relationship, String)> = Vec::new();

    // evidence aggregation helpers over the reality graph
    let sym_evidence_in_file = |fid: &str| -> Vec<String> {
        let mut ev: Vec<String> = Vec::new();
        for r in graph.out_pred(fid, scc_core::predicates::CONTAINS) {
            if let Some(e) = graph.entities.get(&r.object) {
                ev.extend(e.evidence.clone());
            }
        }
        ev
    };
    let write_evidence_to = |store_id: &str| -> Vec<String> {
        // data entities (repo://r/data/store.entity) resolve to their store
        let store_target = if store_id.contains("/data/") {
            graph
                .entities
                .get(store_id)
                .and_then(|e| e.attributes.get("store"))
                .and_then(|v| v.as_str())
                .map(|s| entity_id(repo_id, kinds::DATA_STORE, s))
                .unwrap_or_else(|| store_id.to_string())
        } else {
            store_id.to_string()
        };
        let mut ev: Vec<String> = Vec::new();
        for r in graph.in_pred(&store_target, scc_core::predicates::WRITES) {
            ev.extend(r.evidence.clone());
        }
        ev
    };
    let call_evidence_between = |from_comp: &str, to_comp: &str| -> Vec<String> {
        let mut ev: Vec<String> = Vec::new();
        for (sym_id, comp) in &symbol_component {
            if comp != from_comp {
                continue;
            }
            for r in graph.out_pred(sym_id, scc_core::predicates::CALLS) {
                if let Some(tc) = symbol_component.get(&r.object) {
                    if tc == to_comp {
                        ev.extend(r.evidence.clone());
                    }
                }
            }
        }
        ev
    };

    for e in &out {
        if let Some(files) = files_in_component.get(&e.name) {
            for fid in files {
                rels.push((
                    Relationship::new(
                        rel(&["component_contains", &e.id, fid]),
                        e.id.clone(),
                        scc_core::predicates::CONTAINS,
                        fid.clone(),
                        Provenance::Extracted,
                    )
                    .with_evidence(sym_evidence_in_file(fid)),
                    String::new(),
                ));
            }
        }
        if let Some(owned) = e.attributes.get("owns").and_then(|v| v.as_array()) {
            for o in owned {
                let target = o.get("target").and_then(|v| v.as_str());
                let prov = parse_prov(
                    o.get("provenance")
                        .and_then(|v| v.as_str())
                        .unwrap_or("INFERRED"),
                );
                let conf = o
                    .get("confidence")
                    .and_then(|v| v.as_f64())
                    .unwrap_or_else(|| prov.default_confidence());
                let claim_evidence: Vec<String> = o
                    .get("evidence")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(os) = target {
                    // provenance-preserving: the derived OWNS relationship
                    // carries the claim's own provenance (DECLARED intent
                    // never becomes a resolved ownership fact), and the rel
                    // id includes provenance so conflicting claims coexist
                    let prov_tag = prov.as_str().to_ascii_lowercase();
                    let mut evidence = claim_evidence;
                    if evidence.is_empty() {
                        evidence = write_evidence_to(os);
                    }
                    rels.push((
                        Relationship::new(
                            rel(&["component_owns", &e.id, os, &prov_tag]),
                            e.id.clone(),
                            scc_core::predicates::OWNS,
                            os.to_string(),
                            prov,
                        )
                        .with_confidence(conf)
                        .with_evidence(evidence),
                        String::new(),
                    ));
                }
            }
        }
        if let Some(deps) = e.attributes.get("depends_on").and_then(|v| v.as_array()) {
            for d in deps {
                if let Some(t) = d.get("target").and_then(|v| v.as_str()) {
                    let target_id = entity_id(repo_id, kinds::COMPONENT, t);
                    let prov = parse_prov(
                        d.get("provenance")
                            .and_then(|v| v.as_str())
                            .unwrap_or("INFERRED"),
                    );
                    rels.push((
                        Relationship::new(
                            rel(&["component_depends", &e.id, &target_id]),
                            e.id.clone(),
                            scc_core::predicates::DEPENDS_ON,
                            target_id,
                            prov,
                        )
                        .with_evidence(call_evidence_between(&e.name, t)),
                        String::new(),
                    ));
                }
            }
        }
    }
    for (r, src) in rels {
        store.insert_relationship(&r, &src)?;
    }

    // ---- per-component state attribution + pass-2 service compilation
    // (additive — the flat clustering list stays intact; services are extra
    // entities of kind SERVICE with CONTAINS edges to member components). ----
    let state_authority = crate::state::compile_state_authority(graph, &symbol_component);
    for c in out.iter_mut() {
        let mut mine: Vec<String> = Vec::new();
        for section in [
            crate::state::S_RUNTIME,
            crate::state::S_REACTIVE,
            crate::state::S_CONFIGURATION,
            crate::state::S_CACHES,
            crate::state::S_DERIVED,
        ] {
            if let Some(lines) = state_authority.get(section) {
                let prefix = format!("{} ", c.name);
                for l in lines {
                    if l.starts_with(&prefix) {
                        mine.push(l.clone());
                    }
                }
            }
        }
        c.attr("state_authority", json!(mine));
    }
    crate::clustering::compile_services(
        store,
        &out,
        &clustering.component_weights,
        &clustering.cross_unit,
        &parent_per_comp,
    )?;

    Ok(out)
}

fn clear_component_relationships(store: &Store) -> Result<()> {
    let rows = store.all_relationships()?;
    let ids: Vec<String> = rows
        .into_iter()
        .filter(|r| r.id.starts_with(RELPREFIX))
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

    #[test]
    fn intent_ownership_stays_declared() {
        // P0 provenance rule: DECLARED intent ownership must never be
        // promoted to a resolved OWNS relationship by the component compiler.
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();

        // a data store, a file, and a symbol in it that writes the store
        let repo = store.repo_id.clone();
        let store_ent = scc_core::entity_id(&repo, kinds::DATA_STORE, "db");
        store
            .insert_entity(
                    &scc_core::Entity::new(store_ent.clone(), kinds::DATA_STORE, "db"),
                &["main.py".into()],
            )
            .unwrap();
        let file = scc_core::entity_id(&repo, kinds::FILE, "main.py");
        store
            .insert_entity(
                    &scc_core::Entity::new(file.clone(), kinds::FILE, "main.py"),
                &["main.py".into()],
            )
            .unwrap();
        let sym = scc_core::symbol_id(&repo, "main.py", "save");
        store
            .insert_entity(
                    &scc_core::Entity::new(sym.clone(), kinds::SYMBOL, "save"),
                &["main.py".into()],
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:contains",
                    file,
                    scc_core::predicates::CONTAINS,
                    sym.clone(),
                    Provenance::Extracted,
                ),
                "main.py",
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:w",
                    sym.clone(),
                    scc_core::predicates::WRITES,
                    store_ent.clone(),
                    Provenance::Extracted,
                )
                .with_confidence(1.0),
                "main.py",
            )
            .unwrap();

        // intent: root component declares ownership of db too
        let intent = vec![(
            "component".to_string(),
            serde_json::json!({"name": "root", "owns": ["db"]}),
        )];
        let graph = RealityGraph::load(&store).unwrap();
        let comps = compile_components(&graph, &store, &intent, &[]).unwrap();
        let root_comp = comps.iter().find(|c| c.name == "root").unwrap();

        // the owns attribute carries typed claims with provenance
        let claims = root_comp.attributes.get("owns").unwrap().as_array().unwrap();
        assert_eq!(claims.len(), 2, "{claims:?}");
        let declared = claims
            .iter()
            .find(|c| c.get("provenance").and_then(|v| v.as_str()) == Some("DECLARED"))
            .expect("intent claim present");
        assert_eq!(declared["target"].as_str().unwrap(), store_ent);
        let extracted = claims
            .iter()
            .find(|c| c.get("provenance").and_then(|v| v.as_str()) == Some("EXTRACTED"))
            .expect("write-edge claim present");
        assert_eq!(extracted["target"].as_str().unwrap(), store_ent);

        // relationships: DECLARED claim stays DECLARED, never RESOLVED
        let rels = store.all_relationships().unwrap();
        let owns: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::OWNS)
            .collect();
        assert_eq!(owns.len(), 2, "{rels:?}");
        assert!(
            owns.iter().any(|r| r.provenance == Provenance::Declared),
            "declared ownership relationship must exist: {owns:?}"
        );
        assert!(
            !owns.iter().any(|r| r.provenance == Provenance::Resolved),
            "no provenance promotion allowed: {owns:?}"
        );
    }

    #[test]
    fn path_assignment() {
        let cands = vec![
            ComponentCandidate { name: "web".into(), dirs: vec!["src/web".into()], boundary_kind: BOUNDARY_PACKAGE.into() },
            ComponentCandidate { name: "api".into(), dirs: vec!["src/api".into()], boundary_kind: BOUNDARY_DECLARED.into() },
        ];
        assert_eq!(component_for_path("src/api/routes.py", &cands), "api");
        assert_eq!(component_for_path("src/web/app.ts", &cands), "web");
        assert_eq!(component_for_path("src/shared/util.py", &cands), "src");
        assert_eq!(component_for_path("README.md", &cands), "root");
    }

    /// Insert a FILE entity plus CONTAINS edges to its symbols; returns the
    /// file id and the symbol ids.
    fn insert_file_with_symbols(
        store: &Store,
        path: &str,
        symbols: &[&str],
    ) -> (String, Vec<String>) {
        let repo = store.repo_id.clone();
        let file_id = scc_core::entity_id(&repo, kinds::FILE, path);
        store
            .insert_entity(
                    &scc_core::Entity::new(file_id.clone(), kinds::FILE, path),
                &[path.into()],
            )
            .unwrap();
        let mut sym_ids = Vec::new();
        for s in symbols {
            let sid = scc_core::symbol_id(&repo, path, s);
            store
                .insert_entity(
                    &scc_core::Entity::new(sid.clone(), kinds::SYMBOL, *s),
                    &[path.into()],
                )
                .unwrap();
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:contains:{}:{s}", path.replace('/', "_")),
                        file_id.clone(),
                        scc_core::predicates::CONTAINS,
                        sid.clone(),
                        Provenance::Extracted,
                    ),
                    path,
                )
                .unwrap();
            sym_ids.push(sid);
        }
        (file_id, sym_ids)
    }

    #[test]
    fn boundary_kind_classification() {
        // Wave 5: every compiled component records the evidence class that
        // created it — declared intent, workspace package, deployment-unit
        // build context, bare top-level directory, or root-level files —
        // while the entity kind stays kinds::COMPONENT.
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let repo = store.repo_id.clone();

        for f in [
            "web/app.py",
            "packages/a/util.py",
            "services/api/main.py",
            "misc/util.py",
            "README.md",
        ] {
            store
                .insert_entity(
                    &scc_core::Entity::new(
                        scc_core::entity_id(&repo, kinds::FILE, f),
                        kinds::FILE,
                        f,
                    ),
                    &[f.into()],
                )
                .unwrap();
        }
        // workspace package member
        let mut pkg = scc_core::Entity::new(
            scc_core::entity_id(&repo, kinds::PACKAGE, "pkg_a"),
            kinds::PACKAGE,
            "pkg_a",
        );
        pkg.attr("path", serde_json::json!("packages/a"));
        store
            .insert_entity(&pkg, &["packages/a/util.py".into()])
            .unwrap();
        // deployment unit with a build context
        let mut du = scc_core::Entity::new(
            scc_core::entity_id(&repo, kinds::DEPLOYMENT_UNIT, "api"),
            kinds::DEPLOYMENT_UNIT,
            "api",
        );
        du.attr("build_context", serde_json::json!("services/api"));
        store
            .insert_entity(&du, &["services/api/main.py".into()])
            .unwrap();

        let intent = vec![(
            "component".to_string(),
            serde_json::json!({"name": "web", "paths": ["web"]}),
        )];
        let graph = RealityGraph::load(&store).unwrap();
        let comps = compile_components(&graph, &store, &intent, &[]).unwrap();
        let by_name: std::collections::BTreeMap<&str, &scc_core::Entity> =
            comps.iter().map(|c| (c.name.as_str(), c)).collect();
        let kind_of = |n: &str| by_name[n].attributes["boundary_kind"].as_str().unwrap();
        assert_eq!(kind_of("web"), BOUNDARY_DECLARED);
        assert_eq!(kind_of("pkg_a"), BOUNDARY_PACKAGE);
        assert_eq!(kind_of("api"), BOUNDARY_DEPLOYMENT);
        assert_eq!(kind_of("misc"), BOUNDARY_CODE_REGION);
        assert_eq!(kind_of("root"), BOUNDARY_ROOT);
        // semantic clustering: the bare `services` top dir holds NO shell
        // component — its only file belongs to the deployment region, so
        // no empty code-region shell survives (behavior-driven, not
        // directory-driven)
        assert!(
            !by_name.contains_key("services"),
            "empty dir shells are pruned by the clusterer: {comps:?}"
        );
        // entity kind is never renamed; candidates keep their names
        assert_eq!(by_name["api"].kind, kinds::COMPONENT);
        assert!(by_name.contains_key("api"), "candidate name unchanged");
        // components inside a deployment unit carry the additive parent attr
        assert_eq!(by_name["api"].attributes["parent"], serde_json::json!("api"));
        assert!(
            !by_name["web"].attributes.contains_key("parent"),
            "no parent outside a deployment unit"
        );
        // every compiled component carries both new attributes
        for c in &comps {
            assert!(c.attributes.contains_key("boundary_kind"), "{}", c.name);
            assert!(c.attributes.contains_key("clustering_score"), "{}", c.name);
        }
    }

    #[test]
    fn cli_package_dirs_become_cli_boundary_components() {
        // Wave 9: a directory holding CLI registrations (cli-subcommand
        // entrypoints / cli_flags on its symbols or file) becomes its own
        // `cli`-boundary component instead of a generic code-region dir —
        // and the CLI package dir out-prefixes the bare top-level dir for
        // path assignment.
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let _repo = store.repo_id.clone();

        // CLI registrations inside cmd/helm (cobra-style): a symbol owning
        // flags and a symbol with a subcommand entrypoint
        let (helm_file, helm_syms) =
            insert_file_with_symbols(&store, "cmd/helm/helm.go", &["rootCmd", "serveCmd"]);
        let mut flag_sym = store.get_entity(&helm_syms[0]).unwrap().unwrap();
        flag_sym
            .attributes
            .insert("file".into(), serde_json::json!("cmd/helm/helm.go"));
        flag_sym
            .attributes
            .insert("cli_flags".into(), serde_json::json!(["--port", "--env"]));
        store.insert_entity(&flag_sym, &["cmd/helm/helm.go".into()]).unwrap();
        let mut ep_sym = store.get_entity(&helm_syms[1]).unwrap().unwrap();
        ep_sym
            .attributes
            .insert("file".into(), serde_json::json!("cmd/helm/helm.go"));
        ep_sym.attributes
            .insert("entrypoints".into(), serde_json::json!(["cli-subcommand"]));
        store.insert_entity(&ep_sym, &["cmd/helm/helm.go".into()]).unwrap();

        // plain files elsewhere: pkg/util.go (generic) + root README
        let (pkg_file, _) = insert_file_with_symbols(&store, "pkg/util.go", &["Util"]);
        let _ = (helm_file, pkg_file);

        let graph = RealityGraph::load(&store).unwrap();
        let comps = compile_components(&graph, &store, &[], &[]).unwrap();
        let by_name: std::collections::BTreeMap<&str, &scc_core::Entity> =
            comps.iter().map(|c| (c.name.as_str(), c)).collect();

        let cli = by_name["cmd/helm"];
        assert_eq!(cli.attributes["boundary_kind"], serde_json::json!("cli"));
        assert_eq!(
            cli.attributes["layer"],
            serde_json::json!("component"),
            "cli components are authoritative components, not code regions"
        );
        // the cli dir owns its registration file, not the generic dir
        let impl_paths = cli.attributes["implementation"]["paths"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        assert_eq!(impl_paths, vec!["cmd/helm"]);
        // sibling dir stays a plain code-region component
        assert_eq!(
            by_name["pkg"].attributes["boundary_kind"],
            serde_json::json!("code-region")
        );
        assert_eq!(by_name["root"].attributes["boundary_kind"], serde_json::json!("root"));
    }

    #[test]
    fn cli_evidence_in_root_dir_promotes_root_boundary() {
        // Root-level CLI registrations (e.g. a single-package CLI repo)
        // promote the root component to the cli boundary: the whole dir is
        // the CLI package, so it must not read as a bare root fallback.
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let _repo = store.repo_id.clone();
        let (_, syms) = insert_file_with_symbols(&store, "cli.py", &["main"]);
        let mut sym = store.get_entity(&syms[0]).unwrap().unwrap();
        sym.attributes.insert("file".into(), serde_json::json!("cli.py"));
        sym.attributes
            .insert("cli_flags".into(), serde_json::json!(["--verbose"]));
        store.insert_entity(&sym, &["cli.py".into()]).unwrap();

        let graph = RealityGraph::load(&store).unwrap();
        let comps = compile_components(&graph, &store, &[], &[]).unwrap();
        let by_name: std::collections::BTreeMap<&str, &scc_core::Entity> =
            comps.iter().map(|c| (c.name.as_str(), c)).collect();
        assert_eq!(
            by_name["root"].attributes["boundary_kind"],
            serde_json::json!("cli")
        );
    }

    #[test]
    fn hierarchy_clusterer_builds_layer_stack() {
        // Ontology phase: 5-node synthetic graph. a<->b merge at 6
        // (shared store + call) into subsystem s1; c<->d merge at 6 into
        // subsystem s2; s1<->s2 merge at >= 12 (route ownership +3,
        // entrypoints +3, import cohesion +2, call +2, events +3 = 13)
        // into a service. e stays an unmerged leaf.
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let repo = store.repo_id.clone();

        let mut intent: Vec<(String, serde_json::Value)> = Vec::new();
        for name in ["a", "b", "c", "d", "e"] {
            intent.push((
                "component".to_string(),
                serde_json::json!({"name": name, "paths": [name]}),
            ));
        }

        // files + symbols per component
        let (_fa, sa) = insert_file_with_symbols(&store, "a/x.py", &["a_main"]);
        let (fb, sb) = insert_file_with_symbols(&store, "b/x.py", &["b_worker"]);
        let (fc, sc) = insert_file_with_symbols(&store, "c/x.py", &["c_main"]);
        let (_fd, sd) = insert_file_with_symbols(&store, "d/x.py", &["d_worker"]);
        let (_fe, _se) = insert_file_with_symbols(&store, "e/x.py", &["e_standalone"]);
        let _ = (sa[0].clone(), sb[0].clone(), sc[0].clone(), sd[0].clone());

        // shared stores: a+b write db; c+d write db2 (intra +4 each)
        for (i, (syms, store_name)) in [
            (sa.clone(), "db"),
            (sb.clone(), "db"),
            (sc.clone(), "db2"),
            (sd.clone(), "db2"),
        ]
        .into_iter()
        .enumerate()
        {
            let store_ent = scc_core::entity_id(&repo, kinds::DATA_STORE, store_name);
            store
                .insert_entity(
                    &scc_core::Entity::new(store_ent.clone(), kinds::DATA_STORE, store_name),
                    &["x.py".into()],
                )
                .unwrap();
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:w:{store_name}:{i}"),
                        syms[0].clone(),
                        scc_core::predicates::WRITES,
                        store_ent,
                        Provenance::Extracted,
                    ),
                    "x.py",
                )
                .unwrap();
        }
        // calls: a->b and c->d only (intra +2 each; NO cross calls, so no
        // cross pair can reach the pass-1 threshold of 6 on its own)
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:call_ab",
                    sa[0].clone(),
                    scc_core::predicates::CALLS,
                    sb[0].clone(),
                    Provenance::Extracted,
                ),
                "a/x.py",
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:call_cd",
                    sc[0].clone(),
                    scc_core::predicates::CALLS,
                    sd[0].clone(),
                    Provenance::Extracted,
                ),
                "c/x.py",
            )
            .unwrap();
        // routes owned by a and d only (cross a-d +3)
        for (i, sym) in [sa[0].clone(), sd[0].clone()].iter().enumerate() {
            let route = scc_core::entity_id(&repo, kinds::ROUTE, &format!("get-/r{i}"));
            store
                .insert_entity(
                    scc_core::Entity::new(route.clone(), kinds::ROUTE, format!("get-/r{i}"))
                    .attr("method", serde_json::json!("GET"))
                        .attr("path", serde_json::json!(format!("/r{i}"))),
                    &["x.py".into()],
                )
                .unwrap();
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:h{i}"),
                        sym.clone(),
                        scc_core::predicates::HANDLES,
                        route,
                        Provenance::Extracted,
                    ),
                    "x.py",
                )
                .unwrap();
        }
        // cli entrypoints on b and d only (cross b-d +3)
        for (i, sym) in [sb[0].clone(), sd[0].clone()].iter().enumerate() {
            let mut e = store.get_entity(sym).unwrap().unwrap();
            e.attributes
                .insert("entrypoints".into(), serde_json::json!(["cli-subcommand"]));
            store.insert_entity(&e, &["x.py".into()]).unwrap();
            let _ = i;
        }
        // events: c and d publish (intra c-d +3)
        for (i, sym) in [sc[0].clone(), sd[0].clone()].iter().enumerate() {
            let topic = scc_core::entity_id(&repo, kinds::TOPIC, &format!("topic{i}"));
            store
                .insert_entity(
                    &scc_core::Entity::new(topic.clone(), kinds::TOPIC, format!("topic{i}")),
                    &["x.py".into()],
                )
                .unwrap();
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:p{i}"),
                        sym.clone(),
                        scc_core::predicates::PUBLISHES,
                        topic,
                        Provenance::Extracted,
                    ),
                    "x.py",
                )
                .unwrap();
        }
        // shared configuration read: a and c (cross a-c +4)
        let cfg = scc_core::entity_id(&repo, kinds::CONFIGURATION, "MODE");
        store
            .insert_entity(
                    &scc_core::Entity::new(cfg.clone(), kinds::CONFIGURATION, "MODE"),
                &["x.py".into()],
            )
            .unwrap();
        for sym in [sa[0].clone(), sc[0].clone()] {
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:cfg:{}", sym),
                        cfg.clone(),
                        scc_core::predicates::CONFIGURED_BY,
                        sym,
                        Provenance::Extracted,
                    ),
                    "x.py",
                )
                .unwrap();
        }
        // import cohesion: b/x.py imports c/x.py (cross b-c +2)
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:imp_bc",
                    fb.clone(),
                    scc_core::predicates::IMPORTS,
                    fc.clone(),
                    Provenance::Extracted,
                ),
                "b/x.py",
            )
            .unwrap();

        let graph = RealityGraph::load(&store).unwrap();
        let comps = compile_components(&graph, &store, &intent, &[]).unwrap();

        // expected weights (semantic clustering signal set):
        //   intra: a-b = 4(shared store)+2(call) = 6 -> one component "a+b"
        //          c-d = 4(shared store)+2(call) = 6 -> one component "c+d"
        //          (c/d publish DIFFERENT topics — same-topic event edge
        //          does not fire)
        //   cross: a-c = 4(shared config); b-d = 2(cli prior — the repo
        //          classifies Cli on the cli-subcommand entrypoints, and
        //          command regions cohere +PRIOR_WEIGHT) — all < 6, so
        //          pass 1 keeps the pairs apart; pass-2 SUM = 4+2 = 6
        //          < 12 -> NO service
        //   e: isolated leaf; root: empty synthetic region

        // flat components ARE the clustering result: merged pairs, no
        // directory shells
        let names: std::collections::BTreeSet<&str> =
            comps.iter().map(|c| c.name.as_str()).collect();
        for n in ["a+b", "c+d", "e", "root"] {
            assert!(names.contains(n), "clustering result keeps {n}: {names:?}");
        }
        assert!(!names.contains("a"), "merged pair has no a shell: {names:?}");
        assert!(!names.contains("b"), "merged pair has no b shell: {names:?}");
        assert!(!names.contains("c"), "merged pair has no c shell: {names:?}");
        assert!(!names.contains("d"), "merged pair has no d shell: {names:?}");
        let by_name: std::collections::BTreeMap<&str, &scc_core::Entity> =
            comps.iter().map(|c| (c.name.as_str(), c)).collect();

        // no containers: cross evidence (6) is below SERVICE_THRESHOLD (12)
        let services = store.entities_by_kind(kinds::SERVICE).unwrap();
        assert_eq!(services.len(), 0, "no service at sum 6: {services:?}");
        let subsystems = store.entities_by_kind(kinds::SUBSYSTEM).unwrap();
        assert_eq!(subsystems.len(), 0, "no subsystem containers anymore: {subsystems:?}");

        // layers: multi-region merges are evidence-backed components; a
        // declared singleton keeps the component layer; root stays bare
        assert_eq!(by_name["a+b"].attributes["layer"], serde_json::json!("component"));
        assert_eq!(by_name["c+d"].attributes["layer"], serde_json::json!("component"));
        assert_eq!(by_name["e"].attributes["layer"], serde_json::json!("component"));
        assert_eq!(by_name["root"].attributes["layer"], serde_json::json!("code_region"));
        // merged clusters carry BOTH member dirs (path/package stay as
        // priors — the implementation paths are the union)
        let ab_paths = by_name["a+b"].attributes["implementation"]["paths"]
            .as_array()
            .unwrap();
        assert_eq!(
            ab_paths,
            &vec![serde_json::json!("a"), serde_json::json!("b")],
            "merged component paths: {ab_paths:?}"
        );

        // determinism: a second identical compile reproduces the clustering
        let graph2 = RealityGraph::load(&store).unwrap();
        let comps2 = compile_components(&graph2, &store, &intent, &[]).unwrap();
        assert_eq!(comps2.len(), comps.len());
        let names2: std::collections::BTreeSet<&str> =
            comps2.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names2, names, "cluster names stable across recompiles");
        for (a, b) in comps.iter().zip(comps2.iter()) {
            assert_eq!(a.attributes.get("layer"), b.attributes.get("layer"), "{}", a.name);
            assert_eq!(a.attributes.get("parent"), b.attributes.get("parent"), "{}", a.name);
        }
        let services2 = store.entities_by_kind(kinds::SERVICE).unwrap();
        assert_eq!(services2.len(), 0, "no container accumulation");
    }

    #[test]
    fn clustering_score_deterministic_and_ranked() {
        // Wave 5 §28 weights: shared data ownership +4, entrypoint
        // ownership +4, route ownership +3, internal call cohesion +3 —
        // and a bare directory fallback scores only +1.
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let repo = store.repo_id.clone();

        let (_f1, api_syms) =
            insert_file_with_symbols(&store, "api/routes.py", &["handle_a", "handle_b"]);
        let (_f2, api_helpers) = insert_file_with_symbols(&store, "api/helpers.py", &["helper"]);
        let (_f3, _web_syms) = insert_file_with_symbols(&store, "web/app.py", &["web_index"]);

        let store_ent = scc_core::entity_id(&repo, kinds::DATA_STORE, "db");
        store
            .insert_entity(
                    &scc_core::Entity::new(store_ent.clone(), kinds::DATA_STORE, "db"),
                &["api/routes.py".into()],
            )
            .unwrap();
        let routes_file = scc_core::entity_id(&repo, kinds::FILE, "api/routes.py");
        for (i, sym) in ["handle_a", "handle_b"].iter().enumerate() {
            let route = scc_core::entity_id(&repo, kinds::ROUTE, &format!("GET /api/{i}"));
            store
                .insert_entity(
                    scc_core::Entity::new(route.clone(), kinds::ROUTE, format!("GET /api/{i}"))
                    .attr("method", serde_json::json!("GET"))
                        .attr("path", serde_json::json!(format!("/api/{i}"))),
                    &["api/routes.py".into()],
                )
                .unwrap();
            // the route entity lives in the candidate's file (route ownership)
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:route_contains_{i}"),
                        routes_file.clone(),
                        scc_core::predicates::CONTAINS,
                        route.clone(),
                        Provenance::Extracted,
                    ),
                    "api/routes.py",
                )
                .unwrap();
            let sym_id = scc_core::symbol_id(&repo, "api/routes.py", sym);
            // the handler symbol owns the route (entrypoint ownership)
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:handles_{i}"),
                        sym_id.clone(),
                        scc_core::predicates::HANDLES,
                        route.clone(),
                        Provenance::Extracted,
                    ),
                    "api/routes.py",
                )
                .unwrap();
            // two distinct symbols write the same store (shared data ownership)
            store
                .insert_relationship(
                    &Relationship::new(
                        format!("rel:writes_{i}"),
                        sym_id,
                        scc_core::predicates::WRITES,
                        store_ent.clone(),
                        Provenance::Extracted,
                    ),
                    "api/routes.py",
                )
                .unwrap();
        }
        // internal call cohesion: handle_a -> helper, both inside api
        store
            .insert_relationship(
                &Relationship::new(
                    "rel:call_internal",
                    api_syms[0].clone(),
                    scc_core::predicates::CALLS,
                    api_helpers[0].clone(),
                    Provenance::Extracted,
                ),
                "api/routes.py",
            )
            .unwrap();

        let intent = vec![(
            "component".to_string(),
            serde_json::json!({"name": "api", "paths": ["api"]}),
        )];

        let graph = RealityGraph::load(&store).unwrap();
        let comps = compile_components(&graph, &store, &intent, &[]).unwrap();
        let graph2 = RealityGraph::load(&store).unwrap();
        let comps2 = compile_components(&graph2, &store, &intent, &[]).unwrap();
        let score = |c: &scc_core::Entity| c.attributes["clustering_score"].as_f64().unwrap();
        for (a, b) in comps.iter().zip(comps2.iter()) {
            assert_eq!(
                a.attributes["clustering_score"],
                b.attributes["clustering_score"],
                "scores must be deterministic for {}",
                a.name
            );
        }
        let api = comps.iter().find(|c| c.name == "api").unwrap();
        let web = comps.iter().find(|c| c.name == "web").unwrap();
        assert_eq!(score(api), 14.0, "{:?}", api.attributes);
        assert_eq!(score(web), 1.0, "bare directory: +1 only");
        assert!(score(api) > score(web), "evidence-rich candidate outranks a bare dir");
        assert_eq!(api.attributes["boundary_kind"], serde_json::json!("declared"));
        assert_eq!(web.attributes["boundary_kind"], serde_json::json!("code-region"));
    }

    #[test]
    fn manifest_parsing_is_deterministic() {
        // Cargo workspace member + exclude arrays, multi-line and inline
        let ws = "[workspace]\nmembers = [\n  \"crates/a\",\n  \"crates/b\",\n]\nexclude = [\"crates/a\"]\n";
        assert_eq!(
            toml_string_array(ws, "members"),
            vec!["crates/a", "crates/b"]
        );
        assert_eq!(toml_string_array(ws, "exclude"), vec!["crates/a"]);
        // `exclude_members` must not match the `members` key
        let trick = "[workspace]\nexclude_members = [\"x\"]\nmembers = [\"crates/a\"]\n";
        assert_eq!(toml_string_array(trick, "members"), vec!["crates/a"]);
        // [package] name inside a crate manifest
        let pkg = "[package]\nname = \"grep-cli\"\nversion = \"0.1.0\"\n";
        assert_eq!(toml_package_name(pkg).as_deref(), Some("grep-cli"));
        // `name` outside the [package] table is ignored
        let deps = "[dependencies]\nname = \"x\"\n";
        assert_eq!(toml_package_name(deps), None);
        // go.work: block + inline use directives, comments ignored
        let gow = "go 1.22.0\n\nuse (\n\t./cmd/app\n\t./internal/lib\n\t// a comment\n)\n\nuse ./third\n";
        assert_eq!(
            gowork_use_dirs(gow),
            vec!["cmd/app", "internal/lib", "third"]
        );
        // `user` must not parse as a `use` directive
        assert_eq!(gowork_use_dirs("user = \"x\"\nuse ./only\n"), vec!["only"]);
        assert!(gowork_use_dirs("go 1.22\n\nmodule = \"nouse\"\n").is_empty());
    }

    #[test]
    fn cargo_workspace_members_compile_to_package_components() {
        // EPIC-040 compiler gap: workspace crates become per-crate
        // components, not one top-dir blob. A synthetic Cargo workspace
        // (members via a `crates/*` glob, two crates) yields one
        // package-boundary component per crate and routes member files
        // into their crate — never into a single "crates" component.
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(root.join("crates/alpha/src")).unwrap();
        std::fs::create_dir_all(root.join("crates/beta/src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n\n[package]\nname = \"top\"\n",
        )
        .unwrap();
        std::fs::write(
            root.join("crates/alpha/Cargo.toml"),
            "[package]\nname = \"alpha\"\n",
        )
        .unwrap();
        std::fs::write(
            root.join("crates/beta/Cargo.toml"),
            "[package]\nname = \"beta\"\n",
        )
        .unwrap();

        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let (fa, sa) =
            insert_file_with_symbols(&store, "crates/alpha/src/lib.rs", &["alpha_run"]);
        let (fb, sb) =
            insert_file_with_symbols(&store, "crates/beta/src/lib.rs", &["beta_run"]);
        let (_fr, _sr) = insert_file_with_symbols(&store, "README.md", &["readme"]);
        let _ = (&fa, &fb, &sa, &sb);

        let graph = RealityGraph::load(&store).unwrap();
        let comps = compile_components(&graph, &store, &[], &[]).unwrap();
        let by_name: std::collections::BTreeMap<&str, &scc_core::Entity> =
            comps.iter().map(|c| (c.name.as_str(), c)).collect();

        assert!(by_name.contains_key("alpha"), "per-crate component: {comps:?}");
        assert!(by_name.contains_key("beta"), "per-crate component: {comps:?}");
        assert_eq!(
            by_name["alpha"].attributes["boundary_kind"].as_str(),
            Some(BOUNDARY_PACKAGE)
        );
        assert_eq!(
            by_name["beta"].attributes["boundary_kind"].as_str(),
            Some(BOUNDARY_PACKAGE)
        );
        assert_eq!(
            by_name["alpha"].attributes["implementation"]["paths"],
            json!(["crates/alpha"])
        );
        assert_eq!(
            by_name["beta"].attributes["implementation"]["paths"],
            json!(["crates/beta"])
        );
        // member symbols land in their crate's component
        let alpha_syms = by_name["alpha"].attributes["implementation"]["symbols"]
            .as_array()
            .unwrap();
        assert!(alpha_syms.iter().any(|s| s == "alpha_run"));
        let beta_syms = by_name["beta"].attributes["implementation"]["symbols"]
            .as_array()
            .unwrap();
        assert!(beta_syms.iter().any(|s| s == "beta_run"));
        // the top-level dir is NOT a single component: any "crates"
        // fallback exists only as an empty shell, never holding members
        if let Some(crates) = by_name.get("crates") {
            let syms = crates.attributes["implementation"]["symbols"]
                .as_array()
                .unwrap();
            assert!(syms.is_empty(), "'crates' must not swallow members: {syms:?}");
        }
        // longest-prefix matching routes member files to their crate
        let cands = vec![
            ComponentCandidate {
                name: "alpha".into(),
                dirs: vec!["crates/alpha".into()],
                boundary_kind: BOUNDARY_PACKAGE.into(),
            },
            ComponentCandidate {
                name: "beta".into(),
                dirs: vec!["crates/beta".into()],
                boundary_kind: BOUNDARY_PACKAGE.into(),
            },
            ComponentCandidate {
                name: "crates".into(),
                dirs: vec!["crates".into()],
                boundary_kind: BOUNDARY_CODE_REGION.into(),
            },
        ];
        assert_eq!(component_for_path("crates/alpha/src/lib.rs", &cands), "alpha");
        assert_eq!(component_for_path("crates/beta/src/lib.rs", &cands), "beta");
    }

    #[test]
    fn gowork_modules_compile_to_package_components() {
        // go.work `use (...)` modules become package-boundary components
        // too (go is not modeled by the config extractors; components.rs
        // parses the workspace file directly).
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        for d in ["cmd/app", "internal/lib", "third"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(
            root.join("go.work"),
            "go 1.22.0\n\nuse (\n\t./cmd/app\n\t./internal/lib\n)\n\nuse ./third\n",
        )
        .unwrap();

        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let (_fa, sa) = insert_file_with_symbols(&store, "cmd/app/main.go", &["app_main"]);
        let (_fb, sb) = insert_file_with_symbols(&store, "internal/lib/lib.go", &["lib_fn"]);
        let (_fc, sc) = insert_file_with_symbols(&store, "third/x.go", &["third_fn"]);
        let _ = (&sa, &sb, &sc);

        let graph = RealityGraph::load(&store).unwrap();
        let comps = compile_components(&graph, &store, &[], &[]).unwrap();
        let by_name: std::collections::BTreeMap<&str, &scc_core::Entity> =
            comps.iter().map(|c| (c.name.as_str(), c)).collect();

        for n in ["app", "lib", "third"] {
            assert!(by_name.contains_key(n), "module component missing: {comps:?}");
            assert_eq!(
                by_name[n].attributes["boundary_kind"].as_str(),
                Some(BOUNDARY_PACKAGE),
                "{n}"
            );
        }
        assert_eq!(
            by_name["app"].attributes["implementation"]["paths"],
            json!(["cmd/app"])
        );
        assert_eq!(
            by_name["lib"].attributes["implementation"]["paths"],
            json!(["internal/lib"])
        );
        let app_syms = by_name["app"].attributes["implementation"]["symbols"]
            .as_array()
            .unwrap();
        assert!(app_syms.iter().any(|s| s == "app_main"));
        let lib_syms = by_name["lib"].attributes["implementation"]["symbols"]
            .as_array()
            .unwrap();
        assert!(lib_syms.iter().any(|s| s == "lib_fn"));
        // no single top-level "cmd" blob holds the app module
        if let Some(cmd) = by_name.get("cmd") {
            let syms = cmd.attributes["implementation"]["symbols"]
                .as_array()
                .unwrap();
            assert!(syms.is_empty(), "'cmd' must not swallow modules: {syms:?}");
        }
    }
}
