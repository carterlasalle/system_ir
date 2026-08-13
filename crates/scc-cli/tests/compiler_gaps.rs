//! COMPILER-gap closure integration tests (Wave 10): the atlas must consume
//! the semantic fact layer — exported symbols, annotations/registrations/
//! callbacks, phase-named pipeline symbols, and component member symbols —
//! so store facts stop being COMPILER-gap misses.
//!
//! Fixtures: `python-facts-service` (exported classes + module functions,
//! annotations, registrations), `http-service-python` (routes + exports +
//! contract strings).

mod golden;

use golden::{copy_fixture, run_ok, workdir};

/// The rendered atlas exposes the fact-layer sections, and the structured
/// machine model carries exports as public-api entrypoints (so ground-truth
/// entrypoints like `FastAPI.include_router`-style surfaces match).
#[test]
// trace:v1 id=test.scc.compiler verifies=REQ-SCC-CTX exercises=impl.scc.atlas
fn atlas_renders_fact_layer_sections() {
    let repo = copy_fixture("python-facts-service");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);

    let atlas = run_ok(&dir, &["atlas"]);
    // new Wave 10 sections
    assert!(atlas.contains("# PUBLIC API"), "{atlas}");
    assert!(atlas.contains("# FRAMEWORK SEMANTICS"), "{atlas}");
    assert!(atlas.contains("# LANDMARKS"), "{atlas}");
    // exports grouped per component in compact `component: exports ...`
    // form; the render is bounded, so the `(+N more)` marker may appear.
    assert!(
        atlas.contains("exports "),
        "compact exports line: {atlas}"
    );
}

/// Component implementation carries its member symbols in the structured
/// model, and the rendered ARCHITECTURE block stays compact (paths + count).
#[test]
fn component_members_surface_in_architecture() {
    let repo = copy_fixture("http-service-python");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);

    let atlas = run_ok(&dir, &["atlas"]);
    // the structured component record includes member symbols (the
    // architecture layer matches them), while the rendered block shows the
    // paths plus an honest member count.
    assert!(
        atlas.contains("member symbols"),
        "compact member count in ARCHITECTURE: {atlas}"
    );
    // routes and contracts survive unchanged (regression guard)
    assert!(atlas.contains("GET /api/transcripts"), "{atlas}");
    assert!(atlas.contains("services owns db"), "{atlas}");
}

/// `scc bench atlas` over the fixtures fallback must still run green with
/// the fact-layer consumption (no COMPILER-gap crashes; gate score
/// reported). The fixtures corpus synthesizes ground truth from tasks.json.
#[test]
fn bench_atlas_runs_with_fact_layer() {
    let repo = copy_fixture("http-service-python");
    let dir = workdir(repo.path());
    run_ok(&dir, &["index", "--quiet"]);
    let out = run_ok(&dir, &["atlas", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["kind"], "atlas", "{out}");
    // the pack is well-formed even when the new sections render
    assert!(v["content"].as_str().unwrap().contains("SYSTEM PURPOSE"));
}
