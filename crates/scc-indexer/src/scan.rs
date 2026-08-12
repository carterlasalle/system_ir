//! Repository scanning: file discovery, classification, hashing, and the
//! filesystem sandbox (docs/SECURITY.md §5).

use crate::config::IndexConfig;
use blake3::Hash;
use ignore::WalkBuilder;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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
    Other,
}

impl Language {
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
            Language::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("path escapes repository root: {0}")]
    Escape(PathBuf),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Filesystem sandbox: canonicalize and verify a candidate path stays inside
/// the repository root. Symlinks pointing outside the root are rejected.
pub fn sandbox_path(root: &Path, candidate: &Path) -> Result<PathBuf, ScanError> {
    let root_c = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root_c.join(candidate)
    };
    let canon = joined.canonicalize().map_err(|_| ScanError::Escape(joined.clone()))?;
    if !canon.starts_with(&root_c) {
        return Err(ScanError::Escape(canon));
    }
    Ok(canon)
}

fn is_test_path(path: &Path, language: Language) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let dir = path.parent().and_then(|p| p.to_str()).unwrap_or("");
    let dirs = dir.split('/').collect::<Vec<_>>();
    match language {
        Language::Python => {
            name.starts_with("test_") || name.ends_with("_test.py")
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

fn classify(path: &Path) -> Option<(Language, FileKind)> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let lname = name.to_ascii_lowercase();
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
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
            && (lname.starts_with("compose") || parent.contains(".github") || lname.starts_with("helm")))
        || lname == "docker-compose.yml"
        || lname == "docker-compose.yaml";

    let kind = if is_test_path(path, language) {
        FileKind::Test
    } else if matches!(language, Language::Markdown) {
        FileKind::Docs
    } else if is_infra_file {
        FileKind::Infra
    } else if matches!(language, Language::Json | Language::Yaml | Language::Toml | Language::Env | Language::Dockerfile | Language::Sql) {
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
pub fn scan_repo(root: &Path, config: &IndexConfig) -> Result<Vec<ScannedFile>, ScanError> {
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

    for entry in builder.build() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let abs = entry.path();
        // Resolve symlinks defensively through the sandbox.
        let canon = match abs.canonicalize() {
            Ok(c) => c,
            Err(_) => continue,
        };
        if !canon.starts_with(root.canonicalize().unwrap_or_else(|_| root.to_path_buf())) {
            continue; // symlink escape: ignore
        }
        let rel = match abs.strip_prefix(root) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if rel_str.is_empty() {
            continue;
        }
        if is_ignored(&rel_str, config) {
            continue;
        }
        let Some((language, kind)) = classify(rel) else {
            continue;
        };
        let bytes = match std::fs::read(abs) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if bytes.len() > 5 * 1024 * 1024 {
            continue; // skip oversized files
        }
        let size = bytes.len() as u64;
        let hash = hash_bytes(&bytes);
        out.push(ScannedFile {
            path: rel_str,
            hash,
            language,
            kind,
            size,
        });
    }
    Ok(out)
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
pub fn relative_of(root: &Path, abs: &Path) -> Option<String> {
    let root_c = root.canonicalize().ok()?;
    let abs_c = abs.canonicalize().ok()?;
    let rel = abs_c.strip_prefix(&root_c).ok()?;
    if rel.as_os_str().is_empty() {
        return None;
    }
    let s = rel.to_string_lossy().replace('\\', "/");
    if Path::new(&s).components().any(|c| matches!(c, Component::ParentDir)) {
        return None;
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
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
        assert_eq!(
            classify(Path::new("README.md")).unwrap().1,
            FileKind::Docs
        );
        assert_eq!(classify(Path::new("logo.png")), None);
        assert_eq!(
            classify(Path::new(".env.example")).unwrap().0,
            Language::Env
        );
        assert_eq!(
            classify(Path::new("Dockerfile")).unwrap().0,
            Language::Dockerfile
        );
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
