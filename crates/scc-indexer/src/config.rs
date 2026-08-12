//! SCC configuration (`config.yaml` + `.scc/config.yaml`), per
//! docs/DEPLOYMENT_AND_INFRA.md and docs/config.example.yaml.

use globset::{Glob, GlobMatcher};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub schema: u32,
    pub index: IndexConfig,
    pub languages: LanguagesConfig,
    pub context: ContextConfig,
    pub inference: InferenceConfig,
    pub runtime: RuntimeConfig,
    pub integrations: IntegrationsConfig,
    pub security: SecurityConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IndexConfig {
    pub ignore: Vec<String>,
    pub watch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LanguagesConfig {
    pub typescript: bool,
    pub python: bool,
    pub go: bool,
    pub rust: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextConfig {
    pub startup_tokens: usize,
    pub task_tokens: usize,
    pub include_low_confidence_inference: bool,
    /// Full System Atlas token budget (agent startup architecture, 15–20k
    /// default; small repos naturally produce less).
    pub atlas_tokens: usize,
    /// UserPromptSubmit behavior: `false` (default) injects nothing — the
    /// atlas is already in context and the agent calls SCC on demand;
    /// `true` injects a small task focus (<= 1500 tokens) for
    /// repository-changing prompts.
    pub inject_task_focus: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct InferenceConfig {
    pub enabled: bool,
    /// ollama | openai | endpoint (any OpenAI-compatible API)
    pub provider: String,
    /// Embedding model name (ollama: nomic-embed-text / all-minilm; openai:
    /// text-embedding-3-small; endpoint: provider-specific).
    pub embedding_model: String,
    /// Separate rerank model (cross-encoder style). Empty = no reranking.
    /// Only providers exposing a Cohere/Jina-style `/rerank` endpoint use it.
    pub rerank_model: String,
    /// Base URL of an OpenAI-compatible API. Defaults per provider:
    /// ollama -> http://127.0.0.1:11434/v1, openai -> https://api.openai.com/v1.
    pub base_url: String,
    /// Environment variable name holding the API key (never stored in config
    /// or the database; empty for local providers).
    pub api_key_env: String,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        InferenceConfig {
            enabled: false,
            provider: "local".into(),
            embedding_model: "nomic-embed-text".into(),
            rerank_model: String::new(),
            base_url: String::new(),
            api_key_env: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeConfig {
    pub opentelemetry: OtelConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct OtelConfig {
    pub enabled: bool,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IntegrationsConfig {
    pub serena: bool,
    pub beads: bool,
    pub hindsight: bool,
    pub gitnexus: bool,
    pub narsil: bool,
    /// Shell command launching the Context7 MCP server. Empty = disabled.
    pub context7_command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SecurityConfig {
    pub redact_secrets: bool,
    pub allow_remote_models: bool,
    pub listen: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            schema: 1,
            index: IndexConfig::default(),
            languages: LanguagesConfig::default(),
            context: ContextConfig::default(),
            inference: InferenceConfig::default(),
            runtime: RuntimeConfig::default(),
            integrations: IntegrationsConfig::default(),
            security: SecurityConfig::default(),
        }
    }
}

impl Default for IndexConfig {
    fn default() -> Self {
        IndexConfig {
            ignore: vec![
                ".git/**".into(),
                "vendor/**".into(),
                "generated/**".into(),
                "node_modules/**".into(),
                "dist/**".into(),
                "build/**".into(),
                "target/**".into(),
                ".next/**".into(),
                "venv/**".into(),
                ".venv/**".into(),
                "__pycache__/**".into(),
                "*.lock".into(),
                "*.min.js".into(),
                "*.map".into(),
                "coverage/**".into(),
            ],
            watch: true,
        }
    }
}

impl Default for LanguagesConfig {
    fn default() -> Self {
        LanguagesConfig {
            typescript: true,
            python: true,
            go: false,
            rust: false,
        }
    }
}

impl Default for ContextConfig {
    fn default() -> Self {
        ContextConfig {
            startup_tokens: 6000,
            task_tokens: 10000,
            include_low_confidence_inference: false,
            atlas_tokens: 15000,
            inject_task_focus: false,
        }
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        RuntimeConfig { opentelemetry: OtelConfig { enabled: false } }
    }
}

impl Default for IntegrationsConfig {
    fn default() -> Self {
        IntegrationsConfig {
            serena: true,
            beads: false,
            hindsight: false,
            gitnexus: false,
            narsil: false,
            context7_command: String::new(),
        }
    }
}

impl Default for SecurityConfig {
    fn default() -> Self {
        SecurityConfig {
            redact_secrets: true,
            allow_remote_models: false,
            listen: "127.0.0.1:7777".into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("invalid config yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

impl Config {
    pub fn load(path: &std::path::Path) -> Result<Config, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let cfg: Config = serde_yaml::from_str(&text)?;
        Ok(cfg)
    }

    pub fn default_yaml() -> String {
        serde_yaml::to_string(&Config::default()).unwrap()
    }

    pub fn language_enabled(&self, lang: crate::scan::Language) -> bool {
        match lang {
            crate::scan::Language::Python => self.languages.python,
            crate::scan::Language::TypeScript | crate::scan::Language::JavaScript => {
                self.languages.typescript
            }
            crate::scan::Language::Go => self.languages.go,
            crate::scan::Language::Rust => self.languages.rust,
            _ => true, // config/infra/docs always processed
        }
    }
}

impl IndexConfig {
    pub fn compile_ignore(&self) -> Vec<GlobMatcher> {
        self.ignore
            .iter()
            .filter_map(|p| Glob::new(p).ok().map(|g| g.compile_matcher()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_roundtrip() {
        let cfg = Config::default();
        let yaml = serde_yaml::to_string(&cfg).unwrap();
        let back: Config = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(back.context.task_tokens, 10000);
        assert_eq!(back.security.listen, "127.0.0.1:7777");
        assert!(back.index.ignore.len() >= 5);
    }

    #[test]
    fn parses_example_config() {
        let text = r#"
schema: 1
index:
  ignore:
    - vendor/**
  watch: true
languages:
  typescript: true
  python: true
context:
  startup_tokens: 6000
  task_tokens: 10000
security:
  redact_secrets: true
  listen: 127.0.0.1:7777
"#;
        let cfg: Config = serde_yaml::from_str(text).unwrap();
        assert_eq!(cfg.index.ignore, vec!["vendor/**"]);
        assert_eq!(cfg.context.startup_tokens, 6000);
    }
}
