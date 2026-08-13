//! Archetype detection (Ontology phase — the COMPILER-gap attack).
//!
//! Deterministic evidence scoring over the reality graph + raw import
//! table classifies a repository into one of eight archetypes (plus the
//! honest `Unknown` fallback). Every signal contributes +1..+3; the
//! highest total wins; score ties are broken by the fixed precedence
//! order in [`scc_core::Archetype::PRECEDENCE`].
//!
//! Provenance discipline: all signals are counts/shape facts over
//! EXTRACTED entities and relationships — no heuristics are promoted to
//! facts, this function only classifies.

use crate::RealityGraph;
use scc_core::{kinds, predicates, Archetype};
use scc_store::Store;
use std::collections::HashSet;

/// CLI framework import segment names (module paths split on `/.`-`_`).
const CLI_IMPORTS: &[&str] = &[
    "clap", "cobra", "argparse", "commander", "yargs", "click", "typer", "docopt", "urfave",
];

/// Web-framework import segment names.
const FRAMEWORK_IMPORTS: &[&str] = &[
    "fastapi", "flask", "express", "nest", "nestjs", "gin", "axum", "django", "spring", "actix",
    "aiohttp", "starlette", "falcon", "rocket", "warp", "fiber", "chi", "tornado", "mux",
];

/// Compiler/tool phase verbs: symbols whose name contains one of these are
/// `parse/analyze/transform/generate`-style phase symbols.
const PHASE_VERBS: &[&str] = &[
    "parse", "parser", "compile", "compiler", "analyze", "analyse", "analysis", "transform",
    "generate", "generator", "lexer", "lex", "tokenize", "tokeniser", "emit", "optimize",
    "lower", "ast",
];
// trace:v1 id=impl.scc.archetype work=WORK-SCC-005 satisfies=REQ-SCC-IR

/// Deterministic evidence scores per archetype. Pure function of graph +
/// import-table shape; every signal is an integer count comparison.
pub fn detect_archetype(graph: &RealityGraph, store: &Store) -> Archetype {
    // ---- raw signals (deterministic; sorted iteration everywhere) ----
    let routes = graph.entities_of_kind(kinds::ROUTE).len();
    let mut route_handlers = 0usize;
    let mut registers = 0usize;
    let mut config_reads = 0usize;
    let mut injects = 0usize;
    let mut di_entities = 0usize;
    let mut middleware = 0usize;
    let mut annotations = 0usize;
    let mut exports = 0usize;
    let mut symbols_total = 0usize;
    let mut cli_entrypoints = 0usize;
    let mut entrypoints_total = 0usize;
    let mut cli_flags = 0usize;
    let mut main_guards = 0usize;
    let mut phase_symbols = 0usize;
    let mut infra_files = 0usize;
    let mut deployment_units = 0usize;
    let mut packages = 0usize;
    let mut cli_imports = 0usize;
    let mut framework_imports = 0usize;

    for e in graph.entities.values() {
        match e.kind.as_str() {
            kinds::SYMBOL => {
                symbols_total += 1;
                let name = e.name.to_ascii_lowercase();
                if PHASE_VERBS.iter().any(|v| name.contains(v)) {
                    phase_symbols += 1;
                }
                if let Some(eps) = e.attributes.get("entrypoints").and_then(|v| v.as_array()) {
                    if !eps.is_empty() {
                        entrypoints_total += 1;
                    }
                    for k in eps {
                        match k.as_str() {
                            Some("cli-subcommand") | Some("cli") => cli_entrypoints += 1,
                            Some("main-guard") => main_guards += 1,
                            _ => {}
                        }
                    }
                }
                if let Some(fl) = e.attributes.get("cli_flags").and_then(|v| v.as_array()) {
                    if !fl.is_empty() {
                        cli_flags += 1;
                    }
                }
            }
            kinds::ROUTE => route_handlers += graph.in_pred(&e.id, predicates::HANDLES).len(),
            kinds::MIDDLEWARE => middleware += 1,
            kinds::ANNOTATION => annotations += 1,
            kinds::EXPORT => exports += 1,
            kinds::DI_BINDING => di_entities += 1,
            kinds::DEPLOYMENT_UNIT => deployment_units += 1,
            kinds::PACKAGE => packages += 1,
            kinds::FILE => {
                let p = e.name.to_ascii_lowercase();
                let is_infra = p.contains("dockerfile")
                    || p.contains("docker-compose")
                    || p.contains("compose.yaml")
                    || p.contains("compose.yml")
                    || p.ends_with(".tf")
                    || p.contains("/k8s")
                    || p.contains("/helm")
                    || p.contains(".github/workflows");
                if is_infra {
                    infra_files += 1;
                }
            }
            _ => {}
        }
    }
    // relationship counts (deterministic totals)
    for rels in graph.out.values() {
        for r in rels {
            match r.predicate.as_str() {
                predicates::REGISTERS => registers += 1,
                predicates::CONFIGURED_BY => config_reads += 1,
                predicates::INJECTS => injects += 1,
                _ => {}
            }
        }
    }
    // raw import modules (store table: includes unresolved externals that
    // never became EXTERNAL_API entities) — segment-exact matching so
    // `click` never matches `clickhouse` and `github.com/spf13/cobra`
    // matches `cobra`.
    if let Ok(all) = store.all_imports() {
        let mut seen: HashSet<String> = HashSet::new();
        for (_path, module, _names, _line, _ty) in all {
            let lowered = module.to_ascii_lowercase();
            let segs: Vec<&str> = lowered
                .split(|c: char| !c.is_ascii_alphanumeric())
                .filter(|s| !s.is_empty())
                .collect();
            if CLI_IMPORTS.iter().any(|c| segs.contains(c)) && seen.insert(module.clone()) {
                cli_imports += 1;
            }
            if FRAMEWORK_IMPORTS.iter().any(|f| segs.contains(f)) && seen.insert(module) {
                framework_imports += 1;
            }
        }
    }

    let export_ratio = if symbols_total == 0 {
        0.0
    } else {
        exports as f64 / symbols_total as f64
    };

    // ---- scoring (+1..+3 per signal; highest total wins) ----
    let mut scores: Vec<(Archetype, i32)> = Vec::new();
    let mut push = |a: Archetype, s: i32| scores.push((a, s));

    // ServiceApplication: routes + deployment units, no lib-scale exports
    let mut s = 0;
    if routes > 0 {
        s += 2;
    }
    if route_handlers >= 3 {
        s += 1;
    }
    if deployment_units > 0 {
        s += 1;
    }
    if framework_imports > 0 {
        s += 1;
    }
    push(Archetype::ServiceApplication, s);

    // Cli: subcommand entrypoints + clap/cobra/argparse imports + flags
    s = 0;
    if cli_entrypoints > 0 {
        s += 3;
    }
    if cli_imports > 0 {
        s += 2;
    }
    if cli_flags > 0 {
        s += 2;
    }
    if main_guards > 0 {
        s += 1;
    }
    push(Archetype::Cli, s);

    // LibrarySdk: exported-symbol ratio, few/no routes or entrypoints.
    // Zero exports (empty or infra-only repos) never scores library.
    s = 0;
    if exports > 0 {
        if export_ratio > 0.5 {
            s += 3;
        } else if export_ratio > 0.3 {
            s += 2;
        }
        if routes == 0 {
            s += 1;
        }
        if entrypoints_total == 0 && route_handlers == 0 {
            s += 1;
        }
    }
    push(Archetype::LibrarySdk, s);

    // WebFramework: routes + framework registrations + middleware. The
    // framework-import bonus is strong (+3) only when accompanied by
    // route/registration/middleware evidence — a route-less repo that merely
    // imports a framework somewhere (docs/tests) is not a web framework.
    s = 0;
    if routes > 0 {
        s += 2;
    }
    let framework_evidence = routes > 0 || registers > 0 || middleware > 0;
    if framework_imports > 0 && framework_evidence {
        s += 3;
    } else if framework_imports > 0 {
        s += 1;
    }
    if middleware > 0 {
        s += 2;
    }
    if registers > 0 {
        s += 1;
    }
    if config_reads > 0 {
        s += 1;
    }
    push(Archetype::WebFramework, s);

    // CompilerLanguageTool: parse/analyze/transform/generate symbols
    s = 0;
    if phase_symbols >= 8 {
        s += 3;
    } else if phase_symbols >= 3 {
        s += 2;
    }
    if exports > 0 {
        s += 1;
    }
    push(Archetype::CompilerLanguageTool, s);

    // PluginFramework: registrations/DI/annotations dominating
    s = 0;
    if registers >= 5 {
        s += 3;
    }
    if di_entities > 0 || injects > 0 {
        s += 2;
    }
    if annotations >= 3 {
        s += 2;
    }
    if middleware > 0 {
        s += 1;
    }
    push(Archetype::PluginFramework, s);

    // InfrastructureProject: manifests + deployment units, few symbols.
    // The gate is manifest presence — a bare empty repo with few symbols
    // is never "infrastructure".
    s = 0;
    if infra_files > 0 {
        s += 3;
        if deployment_units > 0 {
            s += 1;
        }
        if symbols_total < 20 {
            s += 1;
        }
    }
    push(Archetype::InfrastructureProject, s);

    // MonorepoPlatform: workspace packages + multiple deployment units
    s = 0;
    if packages >= 3 {
        s += 3;
    } else if packages >= 2 {
        s += 2;
    }
    if deployment_units >= 2 {
        s += 2;
    }
    push(Archetype::MonorepoPlatform, s);

    // ---- deterministic winner: max score, precedence order on ties ----
    let mut best = Archetype::Unknown;
    let mut best_score = i32::MIN;
    for (a, sc) in scores {
        if sc > best_score || (sc == best_score && rank(a) < rank(best)) {
            best = a;
            best_score = sc;
        }
    }
    // scores are sums of positive signals: a max of 0 means nothing fired
    if best_score <= 0 {
        return Archetype::Unknown;
    }
    best
}

/// Fixed tie-break rank: lower rank wins (first in PRECEDENCE).
fn rank(a: Archetype) -> usize {
    Archetype::PRECEDENCE
        .iter()
        .position(|p| *p == a)
        .unwrap_or(usize::MAX)
}

/// Archetype emphasis (semantic clustering): for each archetype the
/// clustering priors change — one region trait coheres (+PRIOR_WEIGHT)
/// when the repository matches that archetype.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterPrior {
    /// CLI: command-region files cohere (cli-subcommand/cli_flags symbols).
    CliCommands,
    /// Framework: registration-emitting regions cohere (REGISTERS edges).
    FrameworkRegistrations,
    /// Service: entrypoint-owning regions cohere (route handlers /
    /// entrypoint-attributed symbols).
    ServiceEntrypoints,
    /// Compiler: phase-named regions cohere (parse/analyze/transform/...).
    CompilerPhases,
}

/// Extra weight applied between two regions sharing the prior trait.
pub const PRIOR_WEIGHT: i32 = 2;

/// The clustering prior for an archetype, or `None` when the archetype adds
/// no region-trait prior (LibrarySdk's emphasis is the doubled public-surface
/// cohesion weight in the clustering signal itself; Unknown/Monorepo/Infra/
/// Plugin rely on the base signal set).
pub fn cluster_prior(archetype: Archetype) -> Option<ClusterPrior> {
    match archetype {
        Archetype::Cli => Some(ClusterPrior::CliCommands),
        Archetype::WebFramework => Some(ClusterPrior::FrameworkRegistrations),
        Archetype::ServiceApplication => Some(ClusterPrior::ServiceEntrypoints),
        Archetype::CompilerLanguageTool => Some(ClusterPrior::CompilerPhases),
        _ => None,
    }
}

/// True when a symbol name matches the compiler/language-tool phase
/// vocabulary (`parse`/`analyze`/`transform`/`generate`/...). Shared with
/// archetype detection so the clustering prior uses the same vocabulary.
pub fn is_phase_symbol(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    PHASE_VERBS.iter().any(|v| n.contains(v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use scc_core::{entity_id, symbol_id, Entity, Provenance, Relationship};
    use scc_store::Store;

    /// Minimal harness: open a store, run a closure that inserts facts,
    /// then detect.
    fn detect(facts: impl FnOnce(&Store)) -> Archetype {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        facts(&store);
        let graph = RealityGraph::load(&store).unwrap();
        detect_archetype(&graph, &store)
    }

    fn file(store: &Store, path: &str) -> String {
        let id = entity_id(&store.repo_id, kinds::FILE, path);
        store
            .insert_entity(
                &Entity::new(id.clone(), kinds::FILE, path),
                &[path.into()],
            )
            .unwrap();
        id
    }

    fn sym(store: &Store, path: &str, name: &str) -> String {
        let id = symbol_id(&store.repo_id, path, name);
        store
            .insert_entity(&Entity::new(id.clone(), kinds::SYMBOL, name), &[path.into()])
            .unwrap();
        id
    }

    fn route(store: &Store, name: &str, handler: &str, sym: &str) {
        let id = entity_id(&store.repo_id, kinds::ROUTE, name);
        store
            .insert_entity(
                Entity::new(id.clone(), kinds::ROUTE, name)
                    .attr("method", serde_json::json!("GET"))
                    .attr("path", serde_json::json!("/x")),
                &["main.py".into()],
            )
            .unwrap();
        store
            .insert_relationship(
                &Relationship::new(
                    format!("rel:h:{name}"),
                    sym.to_string(),
                    predicates::HANDLES,
                    id,
                    Provenance::Extracted,
                ),
                "main.py",
            )
            .unwrap();
        let _ = handler;
    }

    fn import(store: &Store, path: &str, module: &str) {
        store
            .insert_imports(
                path,
                &[(module.to_string(), Vec::new(), 1, "module".to_string())],
            )
            .unwrap();
    }

    fn cli_symbol(store: &Store, path: &str, name: &str, with_flags: bool) -> String {
        let id = sym(store, path, name);
        let mut e = store.get_entity(&id).unwrap().unwrap();
        e.attributes.insert(
            "entrypoints".into(),
            serde_json::json!(["cli-subcommand"]),
        );
        if with_flags {
            e.attributes.insert("cli_flags".into(), serde_json::json!(["--port"]));
        }
        store.insert_entity(&e, &[path.into()]).unwrap();
        id
    }

    #[test]
    fn empty_repo_is_unknown() {
        let a = detect(|_| {});
        assert_eq!(a, Archetype::Unknown);
    }

    #[test]
    fn cli_archetype_from_subcommands_and_imports() {
        let a = detect(|s| {
            let f = file(s, "cli.py");
            let _ = f;
            cli_symbol(s, "cli.py", "serve", true);
            cli_symbol(s, "cli.py", "deploy", true);
            sym(s, "cli.py", "main");
            import(s, "cli.py", "argparse");
            import(s, "main.go", "github.com/spf13/cobra");
            import(s, "cli.rs", "clap");
        });
        assert_eq!(a, Archetype::Cli, "cli signals must win");
    }

    #[test]
    fn web_framework_beats_service_application_on_middleware() {
        let a = detect(|s| {
            let h1 = sym(s, "app.py", "ping");
            let h2 = sym(s, "app.py", "get_item");
            let h3 = sym(s, "app.py", "create_item");
            route(s, "get-/ping", "", &h1);
            route(s, "get-/items/{id}", "", &h2);
            route(s, "post-/items", "", &h3);
            sym(s, "app.py", "RequestLogger");
            let mw = entity_id(&s.repo_id, kinds::MIDDLEWARE, "RequestLogger");
            s.insert_entity(
                &Entity::new(mw, kinds::MIDDLEWARE, "RequestLogger"),
                &["app.py".into()],
            )
            .unwrap();
            import(s, "app.py", "fastapi");
            import(s, "app.py", "flask");
        });
        assert_eq!(a, Archetype::WebFramework, "framework+middleware must win");
    }

    #[test]
    fn library_sdk_wins_on_high_export_ratio() {
        let a = detect(|s| {
            for (i, name) in ["alpha", "beta", "gamma", "delta"].iter().enumerate() {
                let id = sym(s, "lib.rs", name);
                let exp = entity_id(&s.repo_id, kinds::EXPORT, name);
                s.insert_entity(
                &Entity::new(exp.clone(), kinds::EXPORT, *name),
                    &["lib.rs".into()],
                )
                .unwrap();
                s.insert_relationship(
                    &Relationship::new(
                        format!("rel:e{i}"),
                        id,
                        predicates::EXPORTS,
                        exp,
                        Provenance::Extracted,
                    ),
                    "lib.rs",
                )
                .unwrap();
            }
            sym(s, "lib.rs", "internal_helper");
            sym(s, "lib.rs", "another_helper");
        });
        assert_eq!(a, Archetype::LibrarySdk, "4/6 exported must read as library");
    }

    #[test]
    fn compiler_language_tool_from_phase_symbols() {
        let a = detect(|s| {
            for name in [
                "parse", "parse_expression", "tokenize", "lexer", "compile", "compile_module",
                "transform", "generate_code", "emit", "analyze",
            ] {
                sym(s, "compiler.rs", name);
            }
        });
        assert_eq!(a, Archetype::CompilerLanguageTool);
    }

    #[test]
    fn plugin_framework_from_registrations_and_di() {
        let a = detect(|s| {
            let owner = sym(s, "plugin.py", "register_plugin");
            for i in 0..6 {
                let target = entity_id(&s.repo_id, kinds::CONTRACT, &format!("plugin_{i}"));
                s.insert_entity(
                &Entity::new(target.clone(), kinds::CONTRACT, format!("plugin_{i}")),
                    &["plugin.py".into()],
                )
                .unwrap();
                s.insert_relationship(
                    &Relationship::new(
                        format!("rel:reg{i}"),
                        owner.clone(),
                        predicates::REGISTERS,
                        target,
                        Provenance::Extracted,
                    ),
                    "plugin.py",
                )
                .unwrap();
            }
            let binding = entity_id(&s.repo_id, kinds::DI_BINDING, "svc");
            s.insert_entity(
                &Entity::new(binding, kinds::DI_BINDING, "svc"),
                &["plugin.py".into()],
            )
            .unwrap();
        });
        assert_eq!(a, Archetype::PluginFramework);
    }

    #[test]
    fn infrastructure_project_from_manifests() {
        let a = detect(|s| {
            file(s, "Dockerfile");
            file(s, "docker-compose.yml");
            file(s, "terraform/main.tf");
            file(s, "k8s/deployment.yaml");
            let du = entity_id(&s.repo_id, kinds::DEPLOYMENT_UNIT, "web");
            s.insert_entity(
                &Entity::new(du, kinds::DEPLOYMENT_UNIT, "web"),
                &["Dockerfile".into()],
            )
            .unwrap();
        });
        assert_eq!(a, Archetype::InfrastructureProject);
    }

    #[test]
    fn monorepo_from_workspace_packages() {
        let a = detect(|s| {
            for p in ["pkg_a", "pkg_b", "pkg_c"] {
                let id = entity_id(&s.repo_id, kinds::PACKAGE, p);
                s.insert_entity(
                Entity::new(id, kinds::PACKAGE, p).attr("path", serde_json::json!(p)),
                    &["x".into()],
                )
                .unwrap();
            }
            for du in ["api", "web"] {
                let id = entity_id(&s.repo_id, kinds::DEPLOYMENT_UNIT, du);
                s.insert_entity(
                Entity::new(id, kinds::DEPLOYMENT_UNIT, du)
                    .attr("build_context", serde_json::json!(format!("services/{du}"))),
                    &["x".into()],
                )
                .unwrap();
            }
        });
        assert_eq!(a, Archetype::MonorepoPlatform);
    }

    #[test]
    fn ties_break_by_fixed_precedence() {
        // routes + framework imports but NO middleware/registrations:
        // ServiceApplication and WebFramework both score 3 — precedence
        // must pick WebFramework.
        let a = detect(|s| {
            let h1 = sym(s, "app.py", "ping");
            let h2 = sym(s, "app.py", "get_item");
            route(s, "get-/ping", "", &h1);
            route(s, "get-/items/{id}", "", &h2);
            import(s, "app.py", "fastapi");
        });
        // WebFramework: routes +2, framework +3 = 5
        // ServiceApplication: routes +2, handlers>=3? no (2) -> 2
        // LibrarySdk: ratio 0, routes>0 -> 0
        assert_eq!(a, Archetype::WebFramework);
    }

    #[test]
    fn cli_beats_web_framework_when_axum_router_also_present() {
        // cli-service fixture shape: clap + cobra + argparse subcommands
        // AND an axum router with routes. CLI signals must still win.
        let a = detect(|s| {
            let h1 = sym(s, "cli.rs", "health");
            let h2 = sym(s, "cli.rs", "list_users");
            route(s, "get-/health", "", &h1);
            route(s, "get-/users", "", &h2);
            cli_symbol(s, "cli.rs", "Cli", true);
            cli_symbol(s, "main.go", "serveCmd", true);
            cli_symbol(s, "cli.py", "serve", true);
            import(s, "cli.rs", "clap");
            import(s, "main.go", "github.com/spf13/cobra");
            import(s, "cli.py", "argparse");
            import(s, "cli.rs", "axum");
        });
        assert_eq!(a, Archetype::Cli, "cli entrypoints + 3 cli imports must dominate");
    }
}
