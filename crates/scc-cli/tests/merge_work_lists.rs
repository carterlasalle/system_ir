//! Merge invariant: origin/main work items and this branch's Ripwire
//! work list both remain in `.trace/work.toml`.

// trace:exempt reason=internal-detail
fn work_toml() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../.trace/work.toml");
    std::fs::read_to_string(path).expect("read .trace/work.toml")
}

// trace:v1 id=impl.scc-merge-origin-main-work-lists work=WORK-merge-origin-main-into-the-ripwire-extraction-pr-branch-and-resolve-conf satisfies=REQ-implement-merge-origin-main-into-the-ripwire-extraction-pr-branch-and implements=PLAN-merge-origin-main-into-the-ripwire-extraction-pr-branch-and-resolve-conf
fn has_work(text: &str, id: &str) -> bool {
    text.contains(&format!("[work.\"{id}\"]"))
}

#[test]
// trace:v1 id=test.scc-merge-keeps-ripwire-and-main-work verifies=REQ-implement-merge-origin-main-into-the-ripwire-extraction-pr-branch-and exercises=impl.scc-merge-origin-main-work-lists
fn work_toml_keeps_ripwire_and_main_entries() {
    let text = work_toml();
    assert!(
        has_work(
            &text,
            "WORK-phase-30-of-scc-x-ripwire-lessons-absorb-c-and-c-extractors-with-path"
        ),
        "Phase 30 Ripwire work dropped during merge"
    );
    assert!(
        has_work(
            &text,
            "WORK-merge-origin-main-into-the-ripwire-extraction-pr-branch-and-resolve-conf"
        ),
        "this-branch merge work dropped"
    );
    assert!(
        has_work(
            &text,
            "WORK-resolve-merge-conflicts-with-origin-main-and-address-remaining-code-rabbi"
        ),
        "main's merge-conflict work dropped"
    );
    assert!(
        has_work(&text, "WORK-ripwire-lessons-phase1"),
        "Ripwire Phase 1 work dropped"
    );
}
