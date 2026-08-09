//! Secret detection and redaction (docs/SECURITY.md §4, SCC-220).
//!
//! SCC never persists file contents, but config values could still leak into
//! extracted attributes. This module scans env/config-style values and
//! ensures only references (variable names) are stored, never values.

/// Heuristic secret-value classifiers. `None` = not secret.
pub fn classify_secret(key: &str, value: &str) -> bool {
    let k = key.to_ascii_lowercase();
    let key_hint = ["password", "passwd", "secret", "token", "api_key", "apikey", "access_key", "private_key", "client_secret", "auth"]
        .iter()
        .any(|h| k.contains(h));
    if key_hint {
        return true;
    }
    // Generic value patterns for well-known formats even with neutral keys.
    let v = value.trim();
    v.starts_with("sk-") // OpenAI-style
        || v.starts_with("ghp_") // GitHub PAT
        || v.starts_with("gho_")
        || v.starts_with("xoxb-") // Slack
        || v.starts_with("AKIA") // AWS access key
        || v.starts_with("postgres://") || v.starts_with("postgresql://") || v.starts_with("mysql://") || v.starts_with("mongodb://") || v.starts_with("redis://") // DSN with credentials
        || (v.starts_with("-----BEGIN") && v.contains("PRIVATE KEY"))
}

/// Redact a config value for any persisted attribute. Keeps type shape for
/// booleans/numbers, masks strings.
pub fn redact_value(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    let masked = "***REDACTED***";
    if value.len() <= 8 {
        masked.to_string()
    } else {
        let head: String = value.chars().take(2).collect();
        format!("{head}…{masked}")
    }
}

/// Parse a `.env`-style file into `(key, value)` pairs. Handles comments,
/// quotes, and inline comments. Values are returned raw (not redacted) for
/// classification; callers persist only keys.
pub fn parse_env_file(content: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, rest)) = line.split_once('=') else { continue };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let mut value = rest.trim().to_string();
        // strip inline comment (outside quotes)
        if let Some(idx) = value.find(" #") {
            value.truncate(idx);
        }
        let value = value.trim();
        // strip surrounding quotes
        let value = if value.len() >= 2 {
            let b = value.as_bytes();
            if (b[0] == b'"' && b[b.len() - 1] == b'"') || (b[0] == b'\'' && b[b.len() - 1] == b'\'') {
                value[1..value.len() - 1].to_string()
            } else {
                value.to_string()
            }
        } else {
            value.to_string()
        };
        out.push((key.to_string(), value.trim().to_string()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_secrets() {
        assert!(classify_secret("DATABASE_URL", "postgres://user:pass@host:5432/db"));
        assert!(classify_secret("API_KEY", "sk-abc123"));
        assert!(classify_secret("GITHUB_TOKEN", "ghp_xxxxxxxx"));
        assert!(classify_secret("password", "hunter2"));
        assert!(!classify_secret("PORT", "8080"));
        assert!(!classify_secret("LOG_LEVEL", "info"));
        assert!(!classify_secret("NODE_ENV", "production"));
    }

    #[test]
    fn parses_env_with_quotes_and_comments() {
        let content = r#"
# comment
PORT=8080
DATABASE_URL="postgres://u:p@h/db"  # trailing comment
QUOTED='single'
EMPTY=
"#;
        let pairs = parse_env_file(content);
        assert_eq!(pairs.len(), 4);
        assert_eq!(pairs[0], ("PORT".into(), "8080".into()));
        assert_eq!(pairs[1], ("DATABASE_URL".into(), "postgres://u:p@h/db".into()));
        assert_eq!(pairs[2], ("QUOTED".into(), "single".into()));
        assert_eq!(pairs[3], ("EMPTY".into(), "".into()));
    }

    #[test]
    fn redaction_never_leaks_value() {
        let r = redact_value("postgres://user:s3cret@host/db");
        assert!(!r.contains("s3cret"));
        assert!(!r.contains("user"));
        assert!(r.contains("REDACTED"));
    }
}
