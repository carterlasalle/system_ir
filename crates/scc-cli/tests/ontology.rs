//! Ontology + hierarchy integration tests (the COMPILER-gap attack):
//! deterministic Archetype detection, STATE & DATA AUTHORITY compilation,
//! and the hierarchical architecture clusterer — asserted end-to-end
//! through `scc atlas` on fixtures across archetypes.

mod golden;

use golden::{copy_fixture, run_ok, workdir};
use std::path::Path;

#[test]
fn cli_service_is_cli_archetype_with_state_subsections() {
    // cli-service: clap + cobra + argparse subcommands, CLI flags, and an
    // axum router. CLI signals must win over the router signals; the atlas
    // must show the full STATE & DATA AUTHORITY section.
    let repo = copy_fixture("cli-service");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);

    let atlas = run_ok(&dir, &["atlas"]);
    assert!(atlas.contains("ARCHETYPE: cli"), "{atlas}");
    // section header (persistent claims present -> the five-subsection form)
    assert!(atlas.contains("STATE & DATA AUTHORITY"), "{atlas}");
    assert!(atlas.contains("DATA OWNERSHIP"), "{atlas}");
    // runtime state: mutable class field (ServerState.cache)
    assert!(atlas.contains("RUNTIME STATE"), "{atlas}");
    assert!(
        atlas.contains("ServerState.cache (mutable)"),
        "mutable field under runtime state: {atlas}"
    );
    // configuration: PORT env read
    assert!(atlas.contains("CONFIGURATION"), "{atlas}");
    assert!(atlas.contains("configured_by PORT"), "{atlas}");
    // derived: tower-http trace middleware registration
    assert!(atlas.contains("DERIVED / REGISTRIES"), "{atlas}");
    // persistent claims survive in the DATA OWNERSHIP subsection
    assert!(atlas.contains("root owns conn (EXTRACTED)"), "{atlas}");
    assert!(atlas.contains("DATA STORES"), "{atlas}");
}

#[test]
fn python_facts_service_is_web_framework() {
    // python-facts-service: fastapi + flask + celery, routes, middleware —
    // the framework archetype with runtime/config state subsections.
    let repo = copy_fixture("python-facts-service");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);

    let atlas = run_ok(&dir, &["atlas"]);
    assert!(atlas.contains("ARCHETYPE: web_framework"), "{atlas}");
    assert!(atlas.contains("STATE & DATA AUTHORITY"), "{atlas}");
    assert!(atlas.contains("RUNTIME STATE"), "{atlas}");
    assert!(atlas.contains("Cart.items (mutable)"), "{atlas}");
    assert!(atlas.contains("CONFIGURATION"), "{atlas}");
    assert!(atlas.contains("configured_by DEBUG"), "{atlas}");
}

#[test]
fn go_facts_service_is_web_framework() {
    // go-facts-service: gin + mux registrations, middleware chain — the
    // framework archetype (export ratio is high but route registrations +
    // framework imports dominate).
    let repo = copy_fixture("go-facts-service");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);

    let atlas = run_ok(&dir, &["atlas"]);
    assert!(atlas.contains("ARCHETYPE: web_framework"), "{atlas}");
    assert!(atlas.contains("STATE & DATA AUTHORITY"), "{atlas}");
    assert!(atlas.contains("RUNTIME STATE"), "{atlas}");
    assert!(atlas.contains("User.ID (mutable)"), "{atlas}");
}

#[test]
fn hierarchy_clusterer_groups_architecture_by_layer() {
    // Synthetic two-component repo: api and web write the same store
    // (shared state +4) and call across the boundary (+2) — the greedy
    // merge at MERGE_THRESHOLD=6 produces a SUBSYSTEM rendered as a
    // layer-grouped ARCHITECTURE with parent indentation; root stays a
    // bare code-region leaf.
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path().join("repo");
    write_hierarchy_fixture(&dir);

    run_ok(&dir, &["index", "--quiet"]);
    let atlas = run_ok(&dir, &["atlas"]);
    assert!(atlas.contains("ARCHETYPE:"), "archetype header: {atlas}");
    assert!(atlas.contains("SUBSYSTEM API+WEB"), "{atlas}");
    // members rendered under the container (parent indentation)
    assert!(atlas.contains("  API"), "component under subsystem: {atlas}");
    assert!(atlas.contains("  WEB"), "component under subsystem: {atlas}");
    // unmerged root stays a bare leaf after the containers
    assert!(atlas.contains("ROOT\nImplementation: root"), "{atlas}");
    // shared store ownership attributed to both members
    assert!(atlas.contains("api owns conn (EXTRACTED)"), "{atlas}");
    assert!(atlas.contains("web owns conn (EXTRACTED)"), "{atlas}");
}

/// Build the two-component repo (api + web share a sqlite store and call
/// across the boundary; README.md leaves a root leaf).
fn write_hierarchy_fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("api")).unwrap();
    std::fs::create_dir_all(dir.join("web")).unwrap();
    std::fs::write(
        dir.join("api/db.py"),
        r#"# api db helpers.
import sqlite3


def get_conn():
    return sqlite3.connect("app.db")


def save_event(kind: str) -> None:
    conn = get_conn()
    conn.execute("INSERT INTO events (kind) VALUES (?)", (kind,))
    conn.commit()
    conn.close()
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("api/handlers.py"),
        r#"# api handlers.
from api.db import save_event
from web.render import render_page


def handle(kind: str) -> str:
    save_event(kind)
    return render_page(kind)
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("web/render.py"),
        r#"# web rendering.
import sqlite3


def render_page(kind: str) -> str:
    conn = sqlite3.connect("app.db")
    conn.execute("INSERT INTO pages (kind) VALUES (?)", (kind,))
    conn.commit()
    conn.close()
    return kind
"#,
    )
    .unwrap();
    std::fs::write(dir.join("README.md"), "# demo\n").unwrap();
}
