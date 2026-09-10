//! Repository scanning: file discovery, classification, hashing, and the
//! filesystem sandbox (docs/SECURITY.md §5).

use crate::config::IndexConfig;
use blake3::Hash;
use ignore::WalkBuilder;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
// trace:exempt reason=internal-detail
pub enum Language {
    Python,
    TypeScript,
    JavaScript,
    Go,
    Rust,
    Java,
    Json,
    Yaml,
    Toml,
    Env,
    Dockerfile,
    Compose,
    Terraform,
    Markdown,
    Shell,
    Sql,
    C,
    Cpp,
    Objc,
    Csharp,
    Ruby,
    Php,
    Lua,
    Swift,
    Kotlin,
    Protobuf,
    Other,
}

// trace:exempt reason=internal-detail
impl Language {
    // trace:exempt reason=internal-detail
    pub fn as_str(&self) -> &'static str {
        match self {
            Language::Python => "python",
            Language::TypeScript => "typescript",
            Language::JavaScript => "javascript",
            Language::Go => "go",
            Language::Rust => "rust",
            Language::Java => "java",
            Language::Json => "json",
            Language::Yaml => "yaml",
            Language::Toml => "toml",
            Language::Env => "env",
            Language::Dockerfile => "dockerfile",
            Language::Compose => "compose",
            Language::Terraform => "terraform",
            Language::Markdown => "markdown",
            Language::Shell => "shell",
            Language::Sql => "sql",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::Objc => "objc",
            Language::Csharp => "csharp",
            Language::Ruby => "ruby",
            Language::Php => "php",
            Language::Lua => "lua",
            Language::Swift => "swift",
            Language::Kotlin => "kotlin",
            Language::Protobuf => "protobuf",
            Language::Other => "other",
        }
    }

    /// Config/infra extractors run for these languages (not AST walkers).
    // trace:exempt reason=internal-detail
    pub fn is_config_extract(self) -> bool {
        matches!(
            self,
            Language::Env
                | Language::Json
                | Language::Yaml
                | Language::Dockerfile
                | Language::Terraform
                | Language::Protobuf
        )
    }

    /// Every classified language. Tests bind this to LANGUAGE_REGISTRY.
    pub const ALL: &[Language] = &[
        Language::Python,
        Language::TypeScript,
        Language::JavaScript,
        Language::Go,
        Language::Rust,
        Language::Java,
        Language::Json,
        Language::Yaml,
        Language::Toml,
        Language::Env,
        Language::Dockerfile,
        Language::Compose,
        Language::Terraform,
        Language::Markdown,
        Language::Shell,
        Language::Sql,
        Language::C,
        Language::Cpp,
        Language::Objc,
        Language::Csharp,
        Language::Ruby,
        Language::Php,
        Language::Lua,
        Language::Swift,
        Language::Kotlin,
        Language::Protobuf,
        Language::Other,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
// trace:exempt reason=internal-detail
pub enum FileKind {
    Source,
    Test,
    Config,
    Infra,
    Docs,
    Other,
}

impl FileKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            FileKind::Source => "source",
            FileKind::Test => "test",
            FileKind::Config => "config",
            FileKind::Infra => "infra",
            FileKind::Docs => "docs",
            FileKind::Other => "other",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScannedFile {
    /// Repo-relative path, `/`-separated.
    pub path: String,
    pub hash: String,
    pub language: Language,
    pub kind: FileKind,
    pub size: u64,
}

/// Skip accounting for one repository scan. Every met file with a resolved
/// repo-relative path lands in exactly one bucket, so `discovered ==
/// indexed + ignored + unsupported + oversized + unreadable`. Pre-path
/// walker errors and symlink escapes are counted in their own buckets
/// without a path. Counts (never paths) surface in `scc status`,
/// `scc verify`, and analysis quality.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
// trace:v1 id=impl.crates-scc-indexer-src-scan.scan-stats work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-NX53P4B7
pub struct ScanStats {
    pub discovered: u64,
    pub indexed: u64,
    pub ignored: u64,
    pub unsupported: u64,
    pub oversized: u64,
    pub unreadable: u64,
    pub symlink_escape: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("path escapes repository root: {0}")]
    Escape(PathBuf),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Filesystem sandbox: canonicalize and verify a candidate path stays inside
/// the repository root. Symlinks pointing outside the root are rejected.
// trace:exempt reason=internal-detail
pub fn sandbox_path(root: &Path, candidate: &Path) -> Result<PathBuf, ScanError> {
    let root_c = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root_c.join(candidate)
    };
    let canon = joined
        .canonicalize()
        .map_err(|_| ScanError::Escape(joined.clone()))?;
    if !canon.starts_with(&root_c) {
        return Err(ScanError::Escape(canon));
    }
    Ok(canon)
}

// trace:exempt reason=internal-detail
fn is_test_path(path: &Path, language: Language) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let dir = path.parent().and_then(|p| p.to_str()).unwrap_or("");
    let dirs = dir.split('/').collect::<Vec<_>>();
    match language {
        Language::Python => {
            name.starts_with("test_")
                || name.ends_with("_test.py")
                || dirs.contains(&"tests")
                || dirs.contains(&"test")
        }
        Language::TypeScript | Language::JavaScript => {
            name.ends_with(".test.ts")
                || name.ends_with(".spec.ts")
                || name.ends_with(".test.tsx")
                || name.ends_with(".spec.tsx")
                || name.ends_with(".test.js")
                || name.ends_with(".spec.js")
                || dirs.contains(&"__tests__")
        }
        Language::Java => {
            name.ends_with("Test.java")
                || name.ends_with("Tests.java")
                || dirs.contains(&"test")
                || dirs.contains(&"src/test")
                || dirs.iter().any(|d| d.starts_with("test"))
        }
        _ => false,
    }
}

// trace:exempt reason=internal-detail
fn classify(path: &Path) -> Option<(Language, FileKind)> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let lname = name.to_ascii_lowercase();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let parent = path.parent().and_then(|p| p.to_str()).unwrap_or("");

    let language = if lname.starts_with(".env") {
        Language::Env
    } else {
        match ext.as_str() {
            "py" | "pyi" => Language::Python,
            "ts" | "tsx" | "mts" | "cts" => Language::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => Language::JavaScript,
            "go" => Language::Go,
            "rs" => Language::Rust,
            "java" => Language::Java,
            "json" | "jsonc" => Language::Json,
            "yaml" | "yml" => Language::Yaml,
            "toml" => Language::Toml,
            "tf" => Language::Terraform,
            "md" | "mdx" | "rst" => Language::Markdown,
            "sh" | "bash" | "zsh" => Language::Shell,
            "sql" => Language::Sql,
            "c" | "h" => Language::C,
            "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Language::Cpp,
            "m" | "mm" => Language::Objc,
            "cs" => Language::Csharp,
            "rb" => Language::Ruby,
            "php" => Language::Php,
            "lua" => Language::Lua,
            "swift" => Language::Swift,
            "kt" | "kts" => Language::Kotlin,
            "proto" => Language::Protobuf,
            "env" => Language::Env,
            "" => {
                if lname == "dockerfile" {
                    Language::Dockerfile
                } else {
                    Language::Other
                }
            }
            _ => Language::Other,
        }
    };

    if language == Language::Other && ext.as_str() != "" {
        return None;
    }

    let is_infra_file = matches!(language, Language::Terraform)
        || lname == "dockerfile"
        || (language == Language::Yaml
            && (lname.starts_with("compose")
                || parent.contains(".github")
                || lname.starts_with("helm")))
        || lname == "docker-compose.yml"
        || lname == "docker-compose.yaml";

    let kind = if is_test_path(path, language) {
        FileKind::Test
    } else if matches!(language, Language::Markdown) {
        FileKind::Docs
    } else if is_infra_file {
        FileKind::Infra
    } else if matches!(
        language,
        Language::Json
            | Language::Yaml
            | Language::Toml
            | Language::Env
            | Language::Dockerfile
            | Language::Sql
            | Language::Protobuf
    ) {
        FileKind::Config
    } else {
        FileKind::Source
    };

    Some((language, kind))
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    let h: Hash = blake3::hash(bytes);
    h.to_hex().to_string()
}

/// Walk the repository, honoring `.gitignore` and the configured ignore
/// globs, and classify every file.
// trace:exempt reason=internal-detail
pub fn scan_repo(root: &Path, config: &IndexConfig) -> Result<Vec<ScannedFile>, ScanError> {
    Ok(scan_repo_with_stats(root, config)?.0)
}

/// [`scan_repo`] plus per-category skip counts. Every met file lands in
/// exactly one [`ScanStats`] bucket.
// trace:v1 id=impl.crates-scc-indexer-src-scan.scan-repo-with-stats work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-NX53P4B7
pub fn scan_repo_with_stats(
    root: &Path,
    config: &IndexConfig,
) -> Result<(Vec<ScannedFile>, ScanStats), ScanError> {
    let mut stats = ScanStats::default();
    let mut out = Vec::new();
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false) // include dotfiles (.env, .scc/intent.yaml); .git is
        // excluded by git_ignore and our ignore globs
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .parents(true)
        .follow_links(false)
        .require_git(false);

    // One canonical root for the whole walk: canonicalization is a
    // syscall per file when left inside the loop below.
    let root_c = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    for entry in builder.build() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                stats.unreadable += 1;
                continue;
            }
        };
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let abs = entry.path();
        // Resolve symlinks defensively through the sandbox.
        let canon = match abs.canonicalize() {
            Ok(c) => c,
            Err(_) => {
                stats.unreadable += 1;
                continue;
            }
        };
        if !canon.starts_with(&root_c) {
            stats.symlink_escape += 1;
            continue; // symlink escape: ignore
        }
        let rel = match abs.strip_prefix(root) {
            Ok(r) => r,
            Err(_) => {
                stats.unreadable += 1;
                continue;
            }
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if rel_str.is_empty() {
            continue;
        }
        stats.discovered += 1;
        if is_ignored(&rel_str, config) {
            stats.ignored += 1;
            continue;
        }
        let Some((language, kind)) = classify(rel) else {
            stats.unsupported += 1;
            continue;
        };
        let bytes = match std::fs::read(abs) {
            Ok(b) => b,
            Err(_) => {
                stats.unreadable += 1;
                continue;
            }
        };
        if bytes.len() > 5 * 1024 * 1024 {
            stats.oversized += 1;
            continue; // skip oversized files
        }
        let size = bytes.len() as u64;
        let hash = hash_bytes(&bytes);
        stats.indexed += 1;
        out.push(ScannedFile {
            path: rel_str,
            hash,
            language,
            kind,
            size,
        });
    }
    Ok((out, stats))
}

/// Check ignore globs: `**/node_modules/**` style patterns relative to the
/// repo root. `.scc/intent.yaml` is repository intent, not SCC state, so it
/// is always scanned.
pub fn is_ignored(rel: &str, config: &IndexConfig) -> bool {
    if rel == ".scc/intent.yaml" {
        return false;
    }
    if rel == ".scc" || rel.starts_with(".scc/") {
        return true;
    }
    for pat in config.compile_ignore() {
        if pat.is_match(rel) {
            return true;
        }
        // also match directory-prefix semantics
        if rel.contains('/') {
            if let Some(dir) = rel.rsplit_once('/') {
                if pat.is_match(dir.0) {
                    return true;
                }
            }
        }
    }
    false
}

/// Group scanned files by language for stats.
pub fn language_histogram(files: &[ScannedFile]) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for f in files {
        *m.entry(f.language.as_str().to_string()).or_insert(0) += 1;
    }
    m
}

/// Repo-relative path of a file under root, or None if it escapes.
// trace:exempt reason=internal-detail
pub fn relative_of(root: &Path, abs: &Path) -> Option<String> {
    let root_c = root.canonicalize().ok()?;
    let abs_c = abs.canonicalize().ok()?;
    let rel = abs_c.strip_prefix(&root_c).ok()?;
    if rel.as_os_str().is_empty() {
        return None;
    }
    let s = rel.to_string_lossy().replace('\\', "/");
    if Path::new(&s)
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return None;
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // trace:v1 id=test.scc.scan.registry-covers-classified verifies=REQ-language-support-matrix exercises=impl.scc.core.language-registry
    fn classified_languages_are_in_the_support_registry() {
        for lang in Language::ALL {
            if *lang == Language::Other {
                continue;
            }
            assert!(
                scc_core::language_registry()
                    .iter()
                    .any(|c| c.id == lang.as_str()),
                "scan Language::{} missing from LANGUAGE_REGISTRY",
                lang.as_str()
            );
        }
        for id in scc_core::extracted_language_ids() {
            assert!(
                Language::ALL.iter().any(|l| l.as_str() == id),
                "extracted language {id} is not classified by scan"
            );
            let cap = scc_core::language_by_id(id).unwrap();
            assert_eq!(
                cap.tier,
                scc_core::LanguageTier::SemanticDeep,
                "{id} extractor is not tier A"
            );
        }
        assert!(scc_core::language_by_id("c").is_some());
        assert!(scc_core::language_by_id("c").unwrap().extractor);
        assert_eq!(classify(Path::new("src/foo.c")).unwrap().0, Language::C);
        assert_eq!(classify(Path::new("src/foo.cpp")).unwrap().0, Language::Cpp);
        assert_eq!(classify(Path::new("src/foo.hxx")).unwrap().0, Language::Cpp);
        assert_eq!(classify(Path::new("src/foo.rb")).unwrap().0, Language::Ruby);
        assert!(!scc_core::language_by_id("objc").unwrap().extractor);
        assert_eq!(
            scc_core::language_by_id("objc").unwrap().tier,
            scc_core::LanguageTier::IndexSearch
        );
    }

    #[test]
    // trace:exempt reason=internal-detail
    fn classify_various() {
        assert_eq!(
            classify(Path::new("src/app.py")).unwrap().1,
            FileKind::Source
        );
        assert_eq!(
            classify(Path::new("tests/test_app.py")).unwrap().1,
            FileKind::Test
        );
        assert_eq!(
            classify(Path::new("src/app.test.ts")).unwrap().1,
            FileKind::Test
        );
        assert_eq!(
            classify(Path::new("docker-compose.yml")),
            Some((Language::Yaml, FileKind::Infra))
        );
        assert_eq!(classify(Path::new("README.md")).unwrap().1, FileKind::Docs);
        assert_eq!(classify(Path::new("logo.png")), None);
        assert_eq!(
            classify(Path::new(".env.example")).unwrap().0,
            Language::Env
        );
        assert_eq!(
            classify(Path::new("Dockerfile")).unwrap().0,
            Language::Dockerfile
        );
        assert_eq!(
            classify(Path::new("contracts/orders.proto")),
            Some((Language::Protobuf, FileKind::Config))
        );
        let proto = scc_core::language_by_id("protobuf").unwrap();
        assert_eq!(proto.tier, scc_core::LanguageTier::DataConfig);
        assert!(!proto.extractor);
        assert!(proto.contracts);
    }

    #[test]
    // trace:v1 id=test.scc.scan.indexsearch-classified verifies=REQ-scan-classifies-registry-languages exercises=impl.scc.core.language-registry
    fn indexsearch_registry_languages_are_scan_classified() {
        for cap in scc_core::language_registry() {
            if cap.tier != scc_core::LanguageTier::IndexSearch {
                continue;
            }
            let ext = cap
                .extensions
                .first()
                .expect("index/search languages list an extension");
            let (lang, _) = classify(Path::new(&format!("src/sample.{ext}"))).expect("classified");
            assert_eq!(lang.as_str(), cap.id, "scan vs registry id for .{ext}");
            assert!(!cap.extractor, "{} must not claim an extractor", cap.id);
        }
    }

    #[test]
    fn sandbox_rejects_escape() {
        let root = Path::new("/tmp");
        assert!(sandbox_path(root, Path::new("/etc/passwd")).is_err());
        assert!(sandbox_path(root, Path::new("..")).is_err());
    }

    #[test]
    fn ignore_globs() {
        let cfg = IndexConfig {
            ignore: vec!["vendor/**".into(), "generated/**".into()],
            watch: true,
            auto_resolve: false,
        };
        assert!(is_ignored("vendor/foo/bar.py", &cfg));
        assert!(is_ignored("generated/x.ts", &cfg));
        assert!(!is_ignored("src/main.py", &cfg));
        assert!(is_ignored(".scc/data.db", &cfg));
    }

    #[test]
    // trace:v1 id=test.scc.scan.default-ignore-skips-os-metadata verifies=REQ-SI-NX53P4B7
    fn default_ignore_skips_os_metadata() {
        // macOS/Windows metadata must never enter the inventory: Finder
        // mutates .DS_Store spontaneously, which would otherwise flake
        // freshness (a file the user never touched reads as changed).
        let cfg = IndexConfig::default();
        assert!(is_ignored(".DS_Store", &cfg));
        assert!(is_ignored("services/.DS_Store", &cfg));
        assert!(is_ignored("Thumbs.db", &cfg));
        assert!(!is_ignored("services/app.py", &cfg));
    }

    #[test]
    fn scan_respects_gitignore() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.py"), "def main(): pass\n").unwrap();
        std::fs::write(root.join(".gitignore"), "ignored.py\n").unwrap();
        std::fs::write(root.join("ignored.py"), "x = 1\n").unwrap();
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        std::fs::write(root.join("node_modules/dep.js"), "//x\n").unwrap();
        let cfg = IndexConfig::default();
        let files = scan_repo(root, &cfg).unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"src/main.py"));
        assert!(!paths.contains(&"ignored.py"));
        assert!(!paths.iter().any(|p| p.contains("node_modules")));
    }
}
