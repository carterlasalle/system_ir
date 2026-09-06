//! Authoritative language-support registry.
//!
//! Scan classification, extractor wiring, CLI `scc languages`, and tests
//! all derive from [`LANGUAGE_REGISTRY`]. Do not hand-maintain a second
//! list in docs or MCP copy.

use serde::{Deserialize, Serialize};

/// Honest support tier. Parsing syntax is not "supported".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
// trace:exempt reason=internal-detail
pub enum LanguageTier {
    /// Semantic/deep: symbols, refs, routes/state/contracts when present.
    SemanticDeep,
    /// Structural: definitions + calls/imports, limited semantics.
    Structural,
    /// Index/search only: classified and hashed, no extractor.
    IndexSearch,
    /// Data/config/docs: specialized non-AST extractors or classification.
    DataConfig,
}

impl LanguageTier {
    pub fn as_str(self) -> &'static str {
        match self {
            LanguageTier::SemanticDeep => "A",
            LanguageTier::Structural => "B",
            LanguageTier::IndexSearch => "C",
            LanguageTier::DataConfig => "D",
        }
    }
}

/// Capability flags for one language. Generated surfaces must read these
/// rather than assuming "we have a parser, therefore we extract X".
// trace:exempt reason=internal-detail
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LanguageCapability {
    pub id: &'static str,
    pub display: &'static str,
    pub tier: LanguageTier,
    pub extensions: &'static [&'static str],
    pub filenames: &'static [&'static str],
    pub extractor: bool,
    pub definitions: bool,
    pub methods: bool,
    pub classes: bool,
    pub fields: bool,
    pub calls: bool,
    pub reads_writes: bool,
    pub imports: bool,
    pub inheritance: bool,
    pub type_refs: bool,
    pub macros: bool,
    pub routes: bool,
    pub state: bool,
    pub contracts: bool,
    pub tests: bool,
    pub notes: &'static str,
}

/// Single source of truth for language support.
///
/// Keep `id` identical to [`crate` scan `Language::as_str`] for languages
/// the indexer classifies. Languages listed here but not classified are
/// honest "not indexed" rows.
pub const LANGUAGE_REGISTRY: &[LanguageCapability] = &[
    cap("python", "Python", LanguageTier::SemanticDeep, &["py", "pyi"], &[], true, true, true, true, true, true, true, true, false, false, false, true, true, true, true, "procedural tree-sitter; store R/W not SSA"),
    cap("typescript", "TypeScript", LanguageTier::SemanticDeep, &["ts", "tsx", "mts", "cts"], &[], true, true, true, true, true, true, true, true, false, true, false, true, true, true, true, "TS/TSX; implements→REGISTERS not INHERITS"),
    cap("javascript", "JavaScript", LanguageTier::SemanticDeep, &["js", "jsx", "mjs", "cjs"], &[], true, true, true, true, true, true, true, true, false, false, false, true, true, true, true, "shares TypeScript extractor"),
    cap("go", "Go", LanguageTier::SemanticDeep, &["go"], &[], true, true, true, true, true, true, true, true, false, false, false, true, true, false, false, "framework-gated routes; tests via path heuristics"),
    cap("rust", "Rust", LanguageTier::SemanticDeep, &["rs"], &[], true, true, true, true, true, true, true, true, false, false, true, true, true, true, true, "macros as invocations; no INHERITS edges"),
    cap("java", "Java", LanguageTier::SemanticDeep, &["java"], &[], true, true, true, true, true, true, true, true, false, true, false, true, true, true, true, "Spring routes import-gated; extends→schema composition"),
    cap("json", "JSON", LanguageTier::DataConfig, &["json", "jsonc"], &[], false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, "named config files only (package.json)"),
    cap("yaml", "YAML", LanguageTier::DataConfig, &["yaml", "yml"], &[], false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, "compose/helm/github workflows"),
    cap("toml", "TOML", LanguageTier::DataConfig, &["toml"], &[], false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, "classified; Cargo.toml not a code extractor"),
    cap("env", "Env", LanguageTier::DataConfig, &["env"], &[".env"], false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, "secret redaction"),
    cap("dockerfile", "Dockerfile", LanguageTier::DataConfig, &[], &["dockerfile", "Dockerfile"], false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, "infra extractor"),
    cap("compose", "Compose", LanguageTier::DataConfig, &[], &[], false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, "detected via filename, not extension"),
    cap("terraform", "Terraform", LanguageTier::DataConfig, &["tf"], &[], false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, "infra extractor"),
    cap("markdown", "Markdown", LanguageTier::DataConfig, &["md", "mdx", "rst"], &[], false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, "docs; DECLARED_AS mention index"),
    cap("shell", "Shell", LanguageTier::IndexSearch, &["sh", "bash", "zsh"], &[], false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, "classified; no AST extractor"),
    cap("sql", "SQL", LanguageTier::IndexSearch, &["sql"], &[], false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, "classified; no AST extractor"),
    cap("c", "C", LanguageTier::IndexSearch, &["c", "h"], &[], false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, "classified; no AST extractor"),
    cap("cpp", "C++", LanguageTier::IndexSearch, &["cc", "cpp", "cxx", "hpp", "hh"], &[], false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, "classified; no AST extractor"),
    cap("objc", "Objective-C", LanguageTier::IndexSearch, &["m", "mm"], &[], false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, "classified; no AST extractor"),
    cap("csharp", "C#", LanguageTier::IndexSearch, &["cs"], &[], false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, "classified; no AST extractor"),
    cap("ruby", "Ruby", LanguageTier::IndexSearch, &["rb"], &[], false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, "classified; no AST extractor"),
    cap("php", "PHP", LanguageTier::IndexSearch, &["php"], &[], false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, "classified; no AST extractor"),
    cap("lua", "Lua", LanguageTier::IndexSearch, &["lua"], &[], false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, "classified; no AST extractor"),
    cap("swift", "Swift", LanguageTier::IndexSearch, &["swift"], &[], false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, "classified; no AST extractor"),
    cap("kotlin", "Kotlin", LanguageTier::IndexSearch, &["kt", "kts"], &[], false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, "classified; no AST extractor"),
];

/// Authoritative language-support rows. Scan, CLI `scc languages`, and
/// tests must read this rather than a second hand-maintained list.
// trace:v1 id=impl.scc.core.language-registry work=WORK-ripwire-lessons-phase1 satisfies=REQ-language-support-matrix
pub fn language_registry() -> &'static [LanguageCapability] {
    LANGUAGE_REGISTRY
}

#[allow(clippy::too_many_arguments)]
// trace:exempt reason=internal-detail
const fn cap(
    id: &'static str,
    display: &'static str,
    tier: LanguageTier,
    extensions: &'static [&'static str],
    filenames: &'static [&'static str],
    extractor: bool,
    definitions: bool,
    methods: bool,
    classes: bool,
    fields: bool,
    calls: bool,
    reads_writes: bool,
    imports: bool,
    inheritance: bool,
    type_refs: bool,
    macros: bool,
    routes: bool,
    state: bool,
    contracts: bool,
    tests: bool,
    notes: &'static str,
) -> LanguageCapability {
    LanguageCapability {
        id,
        display,
        tier,
        extensions,
        filenames,
        extractor,
        definitions,
        methods,
        classes,
        fields,
        calls,
        reads_writes,
        imports,
        inheritance,
        type_refs,
        macros,
        routes,
        state,
        contracts,
        tests,
        notes,
    }
}

// trace:exempt reason=internal-detail
pub fn language_by_id(id: &str) -> Option<&'static LanguageCapability> {
    language_registry().iter().find(|c| c.id == id)
}

// trace:exempt reason=internal-detail
pub fn extracted_language_ids() -> Vec<&'static str> {
    language_registry()
        .iter()
        .filter(|c| c.extractor)
        .map(|c| c.id)
        .collect()
}

/// Markdown table generated from the registry (docs/CLI must not duplicate).
// trace:exempt reason=internal-detail
pub fn support_matrix_markdown() -> String {
    let mut out = String::from(
        "| Language | Tier | Extractor | Defs | Calls | Fields | Imports | Routes | Tests | Notes |\n|---|---|---|---|---|---|---|---|---|---|\n",
    );
    for c in language_registry() {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            c.display,
            c.tier.as_str(),
            yn(c.extractor),
            yn(c.definitions),
            yn(c.calls),
            yn(c.fields),
            yn(c.imports),
            yn(c.routes),
            yn(c.tests),
            c.notes
        ));
    }
    out
}

fn yn(v: bool) -> &'static str {
    if v {
        "yes"
    } else {
        "no"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    // trace:v1 id=test.scc.core.language-registry-unique verifies=REQ-language-support-matrix exercises=impl.scc.core.language-registry
    fn registry_ids_are_unique_and_extracted_are_tier_a() {
        let mut ids = BTreeSet::new();
        for c in language_registry() {
            assert!(ids.insert(c.id), "duplicate language id {}", c.id);
            if c.extractor {
                assert_eq!(
                    c.tier,
                    LanguageTier::SemanticDeep,
                    "{} claims an extractor but is not tier A",
                    c.id
                );
                assert!(c.definitions && c.calls, "{} extractor without defs/calls", c.id);
            }
        }
        assert!(extracted_language_ids().contains(&"python"));
        assert!(extracted_language_ids().contains(&"rust"));
        assert!(!extracted_language_ids().contains(&"c"));
    }

    #[test]
    fn matrix_is_generated_not_empty() {
        let md = support_matrix_markdown();
        assert!(md.contains("| Python |"));
        assert!(md.contains("| C | C | no |"));
    }
}
