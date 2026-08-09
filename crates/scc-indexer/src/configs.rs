//! Config, infrastructure, and intent extraction (docs/FLOW_COMPILER.md §2,
//! EPIC-140-lite for MVP): package.json workspaces, docker-compose services,
//! env files, `.scc/intent.yaml`, README purpose.

use crate::model::{Entrypoint, ExtractedFile};
use crate::redact::{classify_secret, parse_env_file};
use scc_core::kinds;
use scc_core::{Entity, Provenance, Relationship};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A declared-intent document (`.scc/intent.yaml`), per docs §33 and
/// EPIC-180. Fields beyond the docs (paths, stores) are optional extensions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Intent {
    #[serde(default)]
    pub components: BTreeMap<String, IntentComponent>,
    #[serde(default)]
    pub invariants: BTreeMap<String, IntentInvariant>,
    #[serde(default)]
    pub flows: BTreeMap<String, IntentFlow>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IntentComponent {
    #[serde(default)]
    pub responsibility: Vec<String>,
    #[serde(default)]
    pub owns: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentInvariant {
    pub statement: String,
    #[serde(default = "default_severity")]
    pub severity: String,
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default)]
    pub enforced_by: Vec<String>,
}

fn default_severity() -> String {
    "critical".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IntentFlow {
    pub entrypoint: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub trigger: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Non-code extraction result for one file.
#[derive(Debug, Clone, Default)]
pub struct ConfigExtraction {
    pub entities: Vec<Entity>,
    pub relationships: Vec<(Relationship, String)>, // (rel, source_path)
    pub entrypoints: Vec<Entrypoint>,
    /// (file, extracted facts) — e.g. package.json's scripts are not modeled.
    pub intent: Option<Intent>,
    /// README purpose paragraph.
    pub readme_purpose: Option<String>,
}

pub fn extract_config_file(path: &str, content: &str, repo_id: &str) -> ConfigExtraction {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    let mut out = ConfigExtraction::default();
    if name == "package.json" {
        extract_package_json(path, content, repo_id, &mut out);
    } else if name.starts_with("docker-compose") || name.starts_with("compose.") {
        extract_compose(path, content, repo_id, &mut out);
    } else if name.starts_with(".env") {
        extract_env(path, content, repo_id, &mut out);
    } else if path == ".scc/intent.yaml" {
        out.intent = serde_yaml::from_str(content).ok();
    } else if path.eq_ignore_ascii_case("readme.md") {
        out.readme_purpose = readme_purpose(content);
    }
    out
}

fn extract_package_json(path: &str, content: &str, repo_id: &str, out: &mut ConfigExtraction) {
    let Ok(v): Result<serde_json::Value, _> = serde_json::from_str(content) else {
        return;
    };
    // Workspace members -> package entities
    let mut members: Vec<String> = Vec::new();
    if let Some(ws) = v.get("workspaces") {
        if let Some(arr) = ws.as_array() {
            for m in arr {
                if let Some(s) = m.as_str() {
                    members.push(s.trim_end_matches('/').to_string());
                }
            }
        } else if let Some(obj) = ws.as_object() {
            if let Some(pkgs) = obj.get("packages").and_then(|p| p.as_array()) {
                for m in pkgs {
                    if let Some(s) = m.as_str() {
                        members.push(s.trim_end_matches('/').to_string());
                    }
                }
            }
        }
    }
    for m in members {
        let id = scc_core::entity_id(repo_id, kinds::PACKAGE, &m);
        let mut e = Entity::new(id.clone(), kinds::PACKAGE, m.clone());
        e.attr("path", serde_json::json!(m));
        out.entities.push(e);
        let rel = Relationship::new(
            scc_core::relationship_id(0), // id patched by writer
            format!("repo://{repo_id}"),
            scc_core::predicates::CONTAINS,
            id,
            Provenance::Extracted,
        );
        out.relationships.push((rel, path.to_string()));
    }
    // bin/main -> entrypoints
    for key in ["bin", "main"] {
        if let Some(bin) = v.get(key) {
            let target = match bin {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Object(o) => o
                    .values()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .next()
                    .unwrap_or_default(),
                _ => String::new(),
            };
            if !target.is_empty() {
                let entry = Entrypoint {
                    symbol: format!("{key}:{target}"),
                    kind: key.to_string(),
                    line: 1,
                };
                out.entrypoints.push(entry);
            }
        }
    }
}

fn extract_compose(path: &str, content: &str, repo_id: &str, out: &mut ConfigExtraction) {
    let Ok(v): Result<serde_json::Value, _> = serde_yaml::from_str(content) else {
        return;
    };
    let Some(services) = v.get("services").and_then(|s| s.as_object()) else {
        return;
    };
    for (name, spec) in services {
        let id = scc_core::entity_id(repo_id, kinds::DEPLOYMENT_UNIT, name);
        let mut e = Entity::new(id.clone(), kinds::DEPLOYMENT_UNIT, name.clone());
        if let Some(image) = spec.get("image").and_then(|i| i.as_str()) {
            e.attr("image", serde_json::json!(image));
        }
        if let Some(build) = spec.get("build") {
            let ctx = build
                .get("context")
                .or(Some(build))
                .and_then(|b| b.as_str())
                .unwrap_or(".");
            e.attr("build_context", serde_json::json!(ctx));
        }
        if let Some(ports) = spec.get("ports").and_then(|p| p.as_array()) {
            let ps: Vec<String> = ports
                .iter()
                .filter_map(|p| p.as_str().map(|s| s.to_string()))
                .collect();
            if !ps.is_empty() {
                e.attr("ports", serde_json::json!(ps));
            }
        }
        out.entities.push(e);
        // depends_on -> deployed_with
        if let Some(deps) = spec.get("depends_on") {
            let dep_names: Vec<String> = match deps {
                serde_json::Value::Array(a) => a
                    .iter()
                    .filter_map(|d| d.as_str().map(|s| s.to_string()))
                    .collect(),
                serde_json::Value::Object(o) => o.keys().cloned().collect(),
                _ => Vec::new(),
            };
            for d in dep_names {
                let dep_id = scc_core::entity_id(repo_id, kinds::DEPLOYMENT_UNIT, &d);
                let rel = Relationship::new(
                    scc_core::relationship_id(0),
                    id.clone(),
                    scc_core::predicates::DEPENDS_ON,
                    dep_id,
                    Provenance::Extracted,
                );
                out.relationships.push((rel, path.to_string()));
            }
        }
    }
}

fn extract_env(_path: &str, content: &str, repo_id: &str, out: &mut ConfigExtraction) {
    for (key, value) in parse_env_file(content) {
        let secret = classify_secret(&key, &value);
        let kind = if secret {
            kinds::SECRET_REFERENCE
        } else {
            kinds::CONFIGURATION
        };
        let id = scc_core::entity_id(repo_id, kind, &key);
        let mut e = Entity::new(id, kind, key.clone());
        // Persist only references — never values (docs/SECURITY.md §4).
        if secret {
            e.attr("secret", serde_json::json!(true));
        }
        out.entities.push(e);
    }
}

/// First non-heading paragraph of the README as repository purpose.
pub fn readme_purpose(content: &str) -> Option<String> {
    let mut in_code = false;
    let mut paragraphs: Vec<String> = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        if t.is_empty() {
            if let Some(last) = paragraphs.last() {
                if !last.is_empty() {
                    paragraphs.push(String::new());
                }
            }
            continue;
        }
        if t.starts_with('#') || t.starts_with("![]") || t.starts_with("<img") {
            continue;
        }
        if paragraphs.is_empty() || paragraphs.last().map(|p| p.is_empty()).unwrap_or(false) {
            paragraphs.push(t.to_string());
        } else {
            let last = paragraphs.last_mut().unwrap();
            last.push(' ');
            last.push_str(t);
        }
    }
    let joined = paragraphs.join("\n");
    let joined = joined.trim();
    if joined.is_empty() {
        return None;
    }
    Some(joined.chars().take(600).collect())
}

/// Materialize intent.yaml into DECLARED entities/claims consumed by the
/// graph layer. Returns (entities, relationships, invariants-as-claims).
pub fn intent_claims(intent: &Intent, _repo_id: &str) -> Vec<(String, serde_json::Value)> {
    let mut claims = Vec::new();
    for (name, comp) in &intent.components {
        claims.push((
            "component".into(),
            serde_json::json!({
                "name": name,
                "responsibility": comp.responsibility,
                "owns": comp.owns,
                "paths": comp.paths,
            }),
        ));
    }
    for (name, inv) in &intent.invariants {
        claims.push((
            "invariant".into(),
            serde_json::json!({
                "name": name,
                "statement": inv.statement,
                "severity": inv.severity,
                "scope": inv.scope,
                "enforced_by": inv.enforced_by,
            }),
        ));
    }
    for (name, flow) in &intent.flows {
        claims.push((
            "flow".into(),
            serde_json::json!({
                "name": name,
                "entrypoint": flow.entrypoint,
                "kind": flow.kind,
                "trigger": flow.trigger,
                "description": flow.description,
            }),
        ));
    }
    claims
}

/// Language-level extractions that live in config files (e.g. package.json
/// entrypoints) merged into an `ExtractedFile`-like shape for the writer.
pub fn config_as_extracted(out: &ConfigExtraction) -> ExtractedFile {
    let mut ef = ExtractedFile::default();
    for e in &out.entrypoints {
        ef.entrypoints.push(e.clone());
    }
    ef
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_json_workspaces() {
        let content = r#"{
            "name": "mono",
            "workspaces": ["packages/*"],
            "main": "dist/index.js"
        }"#;
        let out = extract_config_file("package.json", content, "mono");
        assert_eq!(out.entities.len(), 1);
        assert_eq!(out.entities[0].kind, kinds::PACKAGE);
        assert_eq!(out.entrypoints.len(), 1);
    }

    #[test]
    fn compose_services() {
        let content = r#"
services:
  api:
    image: my/api
    build: { context: ./services/api }
    depends_on: [db, queue]
  db:
    image: postgres:16
  queue:
    image: redis:7
"#;
        let out = extract_config_file("docker-compose.yml", content, "repo");
        let units: Vec<&Entity> = out
            .entities
            .iter()
            .filter(|e| e.kind == kinds::DEPLOYMENT_UNIT)
            .collect();
        assert_eq!(units.len(), 3);
        let deps = out
            .relationships
            .iter()
            .filter(|(r, _)| r.predicate == scc_core::predicates::DEPENDS_ON)
            .count();
        assert_eq!(deps, 2);
    }

    #[test]
    fn env_only_references() {
        let content = "DATABASE_URL=postgres://u:p@h/db\nPORT=8080\n";
        let out = extract_config_file(".env", content, "repo");
        let kinds_map: BTreeMap<_, _> = out
            .entities
            .iter()
            .map(|e| (e.name.clone(), e.kind.clone()))
            .collect();
        assert_eq!(kinds_map.get("DATABASE_URL").unwrap(), kinds::SECRET_REFERENCE);
        assert_eq!(kinds_map.get("PORT").unwrap(), kinds::CONFIGURATION);
        // values never persisted
        assert!(out
            .entities
            .iter()
            .all(|e| !serde_json::to_string(&e.attributes).unwrap().contains("postgres://")));
    }

    #[test]
    fn intent_parses() {
        let content = r#"
components:
  incident-engine:
    responsibility:
      - extract incidents from transcripts
    owns: [Incident]
invariants:
  raw-immutable:
    statement: raw output cannot be modified
    severity: critical
flows:
  live-radio:
    entrypoint: RadioReceiver.handle
"#;
        let intent: Intent = serde_yaml::from_str(content).unwrap();
        assert!(intent.components.contains_key("incident-engine"));
        assert_eq!(intent.invariants["raw-immutable"].severity, "critical");
        let claims = intent_claims(&intent, "repo");
        assert_eq!(claims.len(), 3);
    }

    #[test]
    fn readme_purpose_extracted() {
        let content = "# My App\n\nThis app processes radio\naudio into incidents.\n\n## Install\n...";
        let purpose = readme_purpose(content).unwrap();
        assert!(purpose.contains("processes radio audio"));
        assert!(!purpose.contains("## Install"));
    }
}
