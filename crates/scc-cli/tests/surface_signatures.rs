//! Golden end-to-end tests for exact declaration headers (audit-fix Item
//! 5/6): source -> index -> store -> Surface must preserve the EXACT
//! declaration header as written (byte-for-byte, multi-line, untruncated)
//! for all five extractor languages, plus same-name overload support
//! (distinct store entities, `overload_index` attrs, separate surface
//! entries). The store assertions read real extractor output — not
//! handcrafted signatures — via `scc index` + `scc export` + `scc surface`.

use std::collections::BTreeMap;

mod golden;

// trace:v1 id=test.scc.surface-signatures verifies=REQ-SCC-IR exercises=impl.scc.extract.rust,impl.scc.extract.python,impl.scc.extract.typescript,impl.scc.extract.go,impl.scc.extract.java,impl.scc.write,impl.scc.surface
#[test]
// trace:exempt reason=unit-test
fn rust_header_survives_index_and_surface() {
    let (repo, dir, attrs) = index_and_attrs();
    // Exact header of `pub async fn process<T: Event>(...` in
    // rust_surface.rs, with the where clause and line breaks preserved.
    let expect = "pub async fn process<T: Event>(\n        &self,\n        event: T,\n    ) -> Result<Incident, Box<dyn Error>>\n    where\n        T: Send + Sync,";
    assert_eq!(header_of(&attrs, "rust_surface.rs", "Handler.process"), expect);

    // Class/interface/enum headers too.
    assert_eq!(header_of(&attrs, "rust_surface.rs", "Incident"), "pub struct Incident");
    assert_eq!(header_of(&attrs, "rust_surface.rs", "Severity"), "pub enum Severity");
    assert_eq!(header_of(&attrs, "rust_surface.rs", "Event"), "pub trait Event: Send + Sync");

    // The surface renders the exact header (not a truncated/one-line
    // reconstruction): each source line appears indented 4 spaces.
    let out = golden::run_ok(&dir, &["surface"]);
    assert!(
        out.contains(&rendered(expect)),
        "surface must render the full multi-line rust header:\n{out}"
    );
    let _ = repo;
}

#[test]
// trace:exempt reason=unit-test
fn python_header_survives_index_and_surface() {
    let (repo, dir, attrs) = index_and_attrs();
    let expect = "async def fetch_all(\n    endpoint: str,\n    limit: int = 20,\n    retries: Optional[int] = None,\n) -> List[dict]";
    assert_eq!(header_of(&attrs, "python_surface.py", "fetch_all"), expect);
    assert_eq!(header_of(&attrs, "python_surface.py", "QueryBuilder"), "class QueryBuilder");
    assert_eq!(
        header_of(&attrs, "python_surface.py", "QueryBuilder.build"),
        "def build(\n        self,\n        fields: List[str],\n        where: Optional[str] = None,\n        order_by: str = \"id\",\n    ) -> \"QueryBuilder\""
    );

    let out = golden::run_ok(&dir, &["surface"]);
    assert!(
        out.contains(&rendered(expect)),
        "surface must render the full multi-line python header:\n{out}"
    );
    let _ = repo;
}

#[test]
// trace:exempt reason=unit-test
fn typescript_header_survives_index_and_surface() {
    let (repo, dir, attrs) = index_and_attrs();
    assert_eq!(header_of(&attrs, "ts_surface.ts", "Repo"), "interface Repo<T>");
    assert_eq!(
        header_of(&attrs, "ts_surface.ts", "IncidentRepo"),
        "class IncidentRepo implements Repo<Incident>"
    );
    let expect = "async findByOwner(\n    owner: string,\n    opts?: { limit?: number },\n  ): Promise<Incident[]>";
    assert_eq!(header_of(&attrs, "ts_surface.ts", "IncidentRepo.findByOwner"), expect);

    let out = golden::run_ok(&dir, &["surface"]);
    assert!(
        out.contains(&rendered(expect)),
        "surface must render the full multi-line typescript header:\n{out}"
    );
    let _ = repo;
}

#[test]
// trace:exempt reason=unit-test
fn go_header_survives_index_and_surface() {
    let (repo, dir, attrs) = index_and_attrs();
    assert_eq!(header_of(&attrs, "go_surface.go", "Incident"), "type Incident struct");
    assert_eq!(header_of(&attrs, "go_surface.go", "Reporter"), "type Reporter struct");
    assert_eq!(header_of(&attrs, "go_surface.go", "Notifier"), "type Notifier interface");
    // Receiver + multi-return + tabs preserved verbatim.
    let expect = "func (r *Reporter) Summarize(\n\tincidents []*Incident,\n\tlimit int,\n) ([]string, error)";
    assert_eq!(header_of(&attrs, "go_surface.go", "Reporter.Summarize"), expect);
    assert_eq!(
        header_of(&attrs, "go_surface.go", "Reporter.Merge"),
        "func (r *Reporter) Merge(values ...string) string"
    );

    let out = golden::run_ok(&dir, &["surface"]);
    assert!(
        out.contains(&rendered(expect)),
        "surface must render the full multi-line go header:\n{out}"
    );
    let _ = repo;
}

#[test]
// trace:exempt reason=unit-test
fn java_header_survives_index_and_surface() {
    let (repo, dir, attrs) = index_and_attrs();
    assert_eq!(header_of(&attrs, "java_surface.java", "Repository"), "public interface Repository<T>");
    assert_eq!(
        header_of(&attrs, "java_surface.java", "IncidentService"),
        "public class IncidentService<T> implements Repository<T>"
    );
    // throws clause + line breaks preserved.
    let expect = "public List<T> findIncidents(\n        String owner,\n        int limit\n    ) throws IOException";
    assert_eq!(header_of(&attrs, "java_surface.java", "IncidentService.findIncidents"), expect);

    let out = golden::run_ok(&dir, &["surface"]);
    assert!(
        out.contains(&rendered(expect)),
        "surface must render the full multi-line java header:\n{out}"
    );
    let _ = repo;
}

#[test]
// trace:exempt reason=unit-test
fn overload_symbols_are_separate_entries_with_distinct_indexes() {
    let (_repo, dir, _attrs) = index_and_attrs();
    // Two same-name `Calc.foo` methods with different parameter lists are
    // two separate store entities with distinct overload_index values.
    let ir = golden::run_ok(&dir, &["export", "system-ir.json"]);
    let v: serde_json::Value = serde_json::from_str(&ir).expect("system-ir.json parses");
    let mut foos: Vec<serde_json::Value> = Vec::new();
    for e in v["entities"].as_array().expect("entities array") {
        if e["kind"].as_str() == Some("symbol")
            && e["name"].as_str() == Some("Calc.foo")
        {
            foos.push(e.clone());
        }
    }
    assert_eq!(foos.len(), 2, "two separate Calc.foo entities: {foos:?}");
    assert_ne!(
        foos[0]["id"].as_str(),
        foos[1]["id"].as_str(),
        "overload entity ids are distinct"
    );
    let mut by_idx: BTreeMap<u64, &serde_json::Value> = BTreeMap::new();
    for e in &foos {
        let idx = e["attributes"]["overload_index"]
            .as_u64()
            .expect("overload_index attr");
        by_idx.insert(idx, e);
    }
    assert_eq!(by_idx.keys().copied().collect::<Vec<_>>(), vec![0, 1]);
    assert_eq!(
        by_idx[&0]["attributes"]["decl_header"].as_str(),
        Some("public int foo(int a)")
    );
    assert_eq!(
        by_idx[&1]["attributes"]["decl_header"].as_str(),
        Some("public int foo(int a, int b)")
    );

    // The surface renders both overloads as separate entries carrying
    // their exact headers.
    let out = golden::run_ok(&dir, &["surface"]);
    assert!(
        out.contains(&rendered("public int foo(int a)")),
        "surface renders overload 0 header:\n{out}"
    );
    assert!(
        out.contains(&rendered("public int foo(int a, int b)")),
        "surface renders overload 1 header:\n{out}"
    );
}

// trace:exempt reason=internal-detail
/// Index the copied fixture and return (file, name) -> attributes for
/// symbol entities.
// trace:exempt reason=unit-test
fn index_and_attrs(
) -> (tempfile::TempDir, std::path::PathBuf, BTreeMap<(String, String), serde_json::Value>) {
    let repo = golden::copy_fixture("surface-signatures");
    let dir = golden::workdir(repo.path());
    golden::run_ok(&dir, &["index", "--quiet"]);
    let ir = golden::run_ok(&dir, &["export", "system-ir.json"]);
    let v: serde_json::Value = serde_json::from_str(&ir).expect("system-ir.json parses");
    let mut attrs = BTreeMap::new();
    for e in v["entities"].as_array().expect("entities array") {
        if e["kind"].as_str() != Some("symbol") {
            continue;
        }
        let file = e["attributes"]["file"].as_str().unwrap_or("").to_string();
        let name = e["name"].as_str().unwrap_or("").to_string();
        attrs.insert((file, name), e["attributes"].clone());
    }
    (repo, dir, attrs)
}

/// `decl_header` attr of a symbol entity in `file`.
// trace:exempt reason=unit-test
fn header_of(
    attrs: &BTreeMap<(String, String), serde_json::Value>,
    file: &str,
    name: &str,
) -> String {
    attrs
        .get(&(file.to_string(), name.to_string()))
        .and_then(|a| a.get("decl_header"))
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| {
            panic!(
                "no decl_header attr for {file} {name}: {:?}",
                attrs.get(&(file.to_string(), name.to_string()))
            )
        })
        .to_string()
}

/// The exact fixture source header, indented 4 spaces per line exactly as
/// the surface renderer emits source_signature lines.
// trace:exempt reason=unit-test
fn rendered(header: &str) -> String {
    header
        .split('\n')
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}
