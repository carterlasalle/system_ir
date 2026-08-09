//! Infrastructure extractors (docs/EPICS_AND_TICKETS.md P1, system semantics):
//! Kubernetes manifests, Terraform (`.tf` / `.tf.json`), GitHub Actions
//! workflows, and Dockerfiles -> `ConfigExtraction`.
//!
//! Mirrors `configs::extract_config_file`: pure function from `(path, content,
//! repo_id)` to entities/relationships with `Provenance::Extracted`
//! (confidence 1.0). Entity output is sorted by name for determinism.
//!
//! Security invariant (docs/SECURITY.md §4): configuration *values* are never
//! persisted — ConfigMap/Secret data, `env` values, and Dockerfile `ENV`
//! values are dropped; only names/keys are emitted.

use crate::configs::ConfigExtraction;
use scc_core::kinds;
use scc_core::predicates;
use scc_core::{entity_id, relationship_id, Entity, Provenance, Relationship};
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// Extract infrastructure facts from a non-code file, or an empty extraction
/// for anything unrecognized. Deterministic: entities sorted by name.
pub fn extract_infra_file(path: &str, content: &str, repo_id: &str) -> ConfigExtraction {
    let mut out = ConfigExtraction::default();
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    if is_github_actions_path(path, &name) {
        extract_github_actions(path, content, repo_id, &mut out);
    } else if is_terraform_path(&name) {
        extract_terraform(path, content, repo_id, &mut out);
    } else if is_dockerfile_path(&name) {
        extract_dockerfile(path, content, repo_id, &mut out);
    } else if is_k8s_path(path, &name) {
        extract_k8s(path, content, repo_id, &mut out);
    }
    // Deterministic output: sort entities by name, then id.
    out.entities
        .sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    out
}

fn is_github_actions_path(path: &str, name: &str) -> bool {
    path.contains(".github/workflows/")
        && (name.ends_with(".yml") || name.ends_with(".yaml"))
}

fn is_terraform_path(name: &str) -> bool {
    name.ends_with(".tf") || name.ends_with(".tf.json")
}

fn is_dockerfile_path(name: &str) -> bool {
    name == "dockerfile" || name.starts_with("dockerfile.")
}

fn is_k8s_path(path: &str, name: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    if lower.contains("k8s") || lower.contains("kubernetes") || lower.contains("manifests") {
        return true;
    }
    name.ends_with(".k8s.yaml") || name.ends_with(".k8s.yml")
}

// ---------------------------------------------------------------------------
// Kubernetes
// ---------------------------------------------------------------------------

/// Split a (possibly multi-document) manifest into JSON values. Malformed or
/// non-document chunks are skipped — never panic.
fn k8s_documents(content: &str) -> Vec<Value> {
    let mut docs = Vec::new();
    let mut buf = String::new();
    for line in content.lines() {
        if line.trim() == "---" {
            push_k8s_doc(&mut docs, &buf);
            buf.clear();
        } else {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    push_k8s_doc(&mut docs, &buf);
    docs
}

fn push_k8s_doc(docs: &mut Vec<Value>, buf: &str) {
    if buf.trim().is_empty() {
        return;
    }
    if let Ok(v) = serde_yaml::from_str::<Value>(buf) {
        docs.push(v);
    }
}

fn extract_k8s(path: &str, content: &str, repo_id: &str, out: &mut ConfigExtraction) {
    let mut env_names: BTreeSet<String> = BTreeSet::new();
    for doc in k8s_documents(content) {
        // Tolerate a top-level list of objects (some tools emit one).
        let docs: Vec<Value> = match doc {
            Value::Array(items) => items,
            other => vec![other],
        };
        for d in docs {
            let Some(kind) = d.get("kind").and_then(|k| k.as_str()) else {
                continue;
            };
            let name = d
                .get("metadata")
                .and_then(|m| m.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            match kind {
                "Deployment" => {
                    let id = entity_id(repo_id, kinds::DEPLOYMENT_UNIT, name);
                    let mut e = Entity::new(id.clone(), kinds::DEPLOYMENT_UNIT, name);
                    if let Some(image) = first_container_image(&d) {
                        e.attr("image", json!(image));
                    }
                    if let Some(replicas) = d
                        .pointer("/spec/replicas")
                        .and_then(|r| match r {
                            Value::Number(n) => n.as_i64(),
                            Value::String(s) => s.parse::<i64>().ok(),
                            _ => None,
                        })
                    {
                        e.attr("replicas", json!(replicas));
                    }
                    let namespace = d
                        .get("metadata")
                        .and_then(|m| m.get("namespace"))
                        .and_then(|n| n.as_str());
                    if let Some(ns) = namespace {
                        e.attr("namespace", json!(ns));
                        // Namespace entity + (unit, deployed_in, namespace).
                        let ns_id = entity_id(repo_id, kinds::DEPLOYMENT_UNIT, ns);
                        let ne = Entity::new(ns_id.clone(), kinds::DEPLOYMENT_UNIT, ns);
                        push_entity(out, ne);
                        let rel = Relationship::new(
                            relationship_id(0), // id patched by writer
                            id.clone(),
                            predicates::DEPLOYED_IN,
                            ns_id,
                            Provenance::Extracted,
                        );
                        out.relationships.push((rel, path.to_string()));
                    }
                    push_entity(out, e);
                }
                "Service" => {
                    let id = entity_id(repo_id, kinds::DEPLOYMENT_UNIT, name);
                    let mut e = Entity::new(id, kinds::DEPLOYMENT_UNIT, name);
                    e.attr("service", json!(true));
                    let ports: Vec<Value> = d
                        .pointer("/spec/ports")
                        .and_then(|p| p.as_array())
                        .map(|arr| arr.iter().filter_map(port_value).collect())
                        .unwrap_or_default();
                    if !ports.is_empty() {
                        e.attr("ports", json!(ports));
                    }
                    push_entity(out, e);
                }
                "ConfigMap" => {
                    // Name only — data values are never stored (SECURITY.md §4).
                    let e = Entity::new(
                        entity_id(repo_id, kinds::CONFIGURATION, name),
                        kinds::CONFIGURATION,
                        name,
                    );
                    push_entity(out, e);
                }
                "Secret" => {
                    // Name only — values are never stored (SECURITY.md §4).
                    let e = Entity::new(
                        entity_id(repo_id, kinds::SECRET_REFERENCE, name),
                        kinds::SECRET_REFERENCE,
                        name,
                    );
                    push_entity(out, e);
                }
                _ => {}
            }
            collect_container_env(&d, &mut env_names);
        }
    }
    // `env:` entries in containers -> configuration entities (name only).
    for key in env_names {
        let mut e = Entity::new(
            entity_id(repo_id, kinds::CONFIGURATION, &key),
            kinds::CONFIGURATION,
            key,
        );
        e.attr("source", json!("env"));
        push_entity(out, e);
    }
}

fn first_container_image(doc: &Value) -> Option<String> {
    for p in ["/spec/template/spec/containers", "/spec/containers"] {
        if let Some(list) = doc.pointer(p).and_then(|c| c.as_array()) {
            if let Some(img) = list
                .first()
                .and_then(|c| c.get("image"))
                .and_then(|i| i.as_str())
            {
                return Some(img.to_string());
            }
        }
    }
    None
}

fn collect_container_env(doc: &Value, env_names: &mut BTreeSet<String>) {
    for p in ["/spec/template/spec/containers", "/spec/containers"] {
        let Some(containers) = doc.pointer(p).and_then(|c| c.as_array()) else {
            continue;
        };
        for c in containers {
            let Some(env) = c.get("env").and_then(|e| e.as_array()) else {
                continue;
            };
            for item in env {
                if let Some(s) = item.as_str() {
                    env_names.insert(s.to_string());
                } else if let Some(n) = item.get("name").and_then(|n| n.as_str()) {
                    env_names.insert(n.to_string());
                }
            }
        }
    }
}

/// A service port as a plain value: number/string as-is, object -> port or
/// targetPort. Never persists config beyond the port itself.
fn port_value(p: &Value) -> Option<Value> {
    match p {
        Value::Number(_) | Value::String(_) => Some(p.clone()),
        Value::Object(m) => m
            .get("port")
            .or_else(|| m.get("targetPort"))
            .cloned(),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Terraform
// ---------------------------------------------------------------------------

fn extract_terraform(path: &str, content: &str, repo_id: &str, out: &mut ConfigExtraction) {
    if path.to_ascii_lowercase().ends_with(".tf.json") {
        extract_tf_json(content, repo_id, out);
    } else {
        extract_tf_blocks(content, repo_id, out);
    }
}

/// Parse `"..."` at the start of `s`; returns (value, rest-after-quote).
fn take_quoted(s: &str) -> Option<(&str, &str)> {
    let s = s.strip_prefix('"')?;
    let end = s.find('"')?;
    Some((&s[..end], &s[end + 1..]))
}

/// `resource "TYPE" "NAME" {` — returns (type, name).
fn parse_resource_line(line: &str) -> Option<(String, String)> {
    let rest = line.trim_start().strip_prefix("resource")?.trim_start();
    let (ty, rest) = take_quoted(rest)?;
    let (name, _) = take_quoted(rest.trim_start())?;
    Some((ty.to_string(), name.to_string()))
}

/// `module "NAME" {` / `variable "NAME" {` — returns name.
fn parse_named_block(line: &str, keyword: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix(keyword)?.trim_start();
    let (name, _) = take_quoted(rest)?;
    Some(name.to_string())
}

fn extract_tf_blocks(content: &str, repo_id: &str, out: &mut ConfigExtraction) {
    for line in content.lines() {
        if let Some((ty, name)) = parse_resource_line(line) {
            terraform_resource(&ty, &name, repo_id, out);
        } else if let Some(name) = parse_named_block(line, "module") {
            let mut e = Entity::new(
                entity_id(repo_id, kinds::EXTERNAL_SYSTEM, &name),
                kinds::EXTERNAL_SYSTEM,
                name,
            );
            e.attr("terraform_module", json!(true));
            push_entity(out, e);
        } else if let Some(name) = parse_named_block(line, "variable") {
            let mut e = Entity::new(
                entity_id(repo_id, kinds::CONFIGURATION, &name),
                kinds::CONFIGURATION,
                name,
            );
            e.attr("terraform_variable", json!(true));
            push_entity(out, e);
        }
    }
}

fn extract_tf_json(content: &str, repo_id: &str, out: &mut ConfigExtraction) {
    let Ok(v) = serde_json::from_str::<Value>(content) else {
        return;
    };
    if let Some(resources) = v.get("resource").and_then(|r| r.as_object()) {
        for (ty, by_name) in resources {
            if let Some(by_name) = by_name.as_object() {
                for name in by_name.keys() {
                    terraform_resource(ty, name, repo_id, out);
                }
            }
        }
    }
    if let Some(modules) = v.get("module").and_then(|m| m.as_object()) {
        for name in modules.keys() {
            let mut e = Entity::new(
                entity_id(repo_id, kinds::EXTERNAL_SYSTEM, name),
                kinds::EXTERNAL_SYSTEM,
                name,
            );
            e.attr("terraform_module", json!(true));
            push_entity(out, e);
        }
    }
    if let Some(vars) = v.get("variable").and_then(|m| m.as_object()) {
        for name in vars.keys() {
            let mut e = Entity::new(
                entity_id(repo_id, kinds::CONFIGURATION, name),
                kinds::CONFIGURATION,
                name,
            );
            e.attr("terraform_variable", json!(true));
            push_entity(out, e);
        }
    }
}

fn terraform_resource(ty: &str, name: &str, repo_id: &str, out: &mut ConfigExtraction) {
    let (kind, technology) = if ty == "aws_db_instance" {
        (kinds::DATA_STORE, Some("postgres"))
    } else if ty == "aws_s3_bucket" {
        (kinds::DATA_STORE, Some("s3"))
    } else if ty == "aws_redis_cluster" || ty == "aws_elasticache_cluster" {
        (kinds::DATA_STORE, Some("redis"))
    } else if ty == "aws_lambda_function" || ty == "aws_ecs_service" || ty == "aws_ec2_instance" {
        (kinds::DEPLOYMENT_UNIT, None)
    } else {
        (kinds::RESOURCE, None)
    };
    let mut e = Entity::new(entity_id(repo_id, kind, name), kind, name);
    match kind {
        kinds::DATA_STORE => {
            e.attr("provider", json!("aws"));
            e.attr("type", json!(ty));
            if let Some(tech) = technology {
                e.attr("technology", json!(tech));
            }
        }
        kinds::DEPLOYMENT_UNIT => {
            e.attr("provider", json!("aws"));
            e.attr("type", json!(ty));
        }
        _ => {
            e.attr("provider_type", json!(ty));
        }
    }
    push_entity(out, e);
}

// ---------------------------------------------------------------------------
// GitHub Actions
// ---------------------------------------------------------------------------

fn extract_github_actions(path: &str, content: &str, repo_id: &str, out: &mut ConfigExtraction) {
    let Ok(v) = serde_yaml::from_str::<Value>(content) else {
        return;
    };
    // Workflow name: `name:` field or the file stem.
    let stem = {
        let base = path.rsplit('/').next().unwrap_or(path);
        match base.rfind('.') {
            Some(i) => &base[..i],
            None => base,
        }
    };
    let workflow_name = v
        .get("name")
        .and_then(|n| n.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| stem.to_string());
    let wf_id = entity_id(repo_id, kinds::WORKFLOW, &workflow_name);
    let mut wf = Entity::new(wf_id.clone(), kinds::WORKFLOW, workflow_name.clone());
    wf.attr("file", json!(path));
    let mut triggers: Vec<Value> = match v.get("on") {
        Some(Value::String(s)) => vec![json!(s)],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(|s| json!(s)))
            .collect(),
        Some(Value::Object(o)) => o.keys().map(|k| json!(k)).collect(),
        _ => Vec::new(),
    };
    triggers.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    wf.attr("on", json!(triggers));

    let mut env_names: BTreeSet<String> = BTreeSet::new();
    if let Some(env) = v.get("env").and_then(|e| e.as_object()) {
        env_names.extend(env.keys().cloned());
    }
    let mut jobs: Vec<Value> = Vec::new();
    if let Some(jobs_map) = v.get("jobs").and_then(|j| j.as_object()) {
        for (jname, jdef) in jobs_map {
            let runs_on: Vec<Value> = match jdef.get("runs-on") {
                Some(Value::String(s)) => vec![json!(s)],
                Some(Value::Array(a)) => a
                    .iter()
                    .filter_map(|x| x.as_str().map(|s| json!(s)))
                    .collect(),
                _ => Vec::new(),
            };
            let needs: Vec<Value> = match jdef.get("needs") {
                Some(Value::String(s)) => vec![json!(s)],
                Some(Value::Array(a)) => a
                    .iter()
                    .filter_map(|x| x.as_str().map(|s| json!(s)))
                    .collect(),
                _ => Vec::new(),
            };
            jobs.push(json!({
                "name": jname,
                "runs_on": runs_on,
                "needs": needs,
            }));
            // Job entity + (workflow, contains, job).
            let jname_full = format!("{}-{}", workflow_name, jname);
            let jid = entity_id(repo_id, kinds::WORKFLOW, &jname_full);
            let je = Entity::new(jid.clone(), kinds::WORKFLOW, jname_full);
            push_entity(out, je);
            let rel = Relationship::new(
                relationship_id(0), // id patched by writer
                wf_id.clone(),
                predicates::CONTAINS,
                jid,
                Provenance::Extracted,
            );
            out.relationships.push((rel, path.to_string()));
            // `env:` at job and step level -> configuration names only.
            if let Some(env) = jdef.get("env").and_then(|e| e.as_object()) {
                env_names.extend(env.keys().cloned());
            }
            if let Some(steps) = jdef.get("steps").and_then(|s| s.as_array()) {
                for step in steps {
                    if let Some(env) = step.get("env").and_then(|e| e.as_object()) {
                        env_names.extend(env.keys().cloned());
                    }
                }
            }
        }
    }
    jobs.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    wf.attr("jobs", json!(jobs));
    push_entity(out, wf);

    for key in env_names {
        let mut e = Entity::new(
            entity_id(repo_id, kinds::CONFIGURATION, &key),
            kinds::CONFIGURATION,
            key,
        );
        e.attr("source", json!("env"));
        push_entity(out, e);
    }
}

// ---------------------------------------------------------------------------
// Dockerfile
// ---------------------------------------------------------------------------

fn extract_dockerfile(path: &str, content: &str, repo_id: &str, out: &mut ConfigExtraction) {
    // Name the unit after the file's directory ("root" for the repo root).
    let mut parts = path.rsplit('/');
    parts.next(); // basename
    let dir = parts.next().unwrap_or("root");
    let id = entity_id(repo_id, kinds::DEPLOYMENT_UNIT, dir);
    let mut e = Entity::new(id, kinds::DEPLOYMENT_UNIT, dir);
    e.attr("dockerfile", json!(path));

    // Join backslash-continued lines so ENV/ FROM spanning lines parse.
    let mut logical: Vec<String> = Vec::new();
    for raw in content.lines() {
        let t = raw.trim();
        if let Some(last) = logical.last_mut() {
            if last.ends_with('\\') {
                last.truncate(last.len() - 1);
                last.push(' ');
                last.push_str(t);
                continue;
            }
        }
        logical.push(t.to_string());
    }

    let mut base_image: Option<String> = None;
    let mut env_names: BTreeSet<String> = BTreeSet::new();
    for line in &logical {
        if let Some(rest) = line.strip_prefix("FROM").or_else(|| line.strip_prefix("from")) {
            if base_image.is_none() {
                for tok in rest.split_whitespace() {
                    if tok.starts_with("--") {
                        continue;
                    }
                    if tok.eq_ignore_ascii_case("as") {
                        break;
                    }
                    base_image = Some(tok.to_string());
                    break;
                }
            }
        } else if let Some(rest) = line.strip_prefix("ENV").or_else(|| line.strip_prefix("env")) {
            let tokens: Vec<&str> = rest.split_whitespace().collect();
            if tokens.iter().any(|t| t.contains('=')) {
                for tok in tokens {
                    if let Some(eq) = tok.find('=') {
                        env_names.insert(tok[..eq].to_string());
                    }
                }
            } else if let Some(first) = tokens.first() {
                // Legacy `ENV KEY value` form.
                env_names.insert(first.to_string());
            }
        }
    }
    if let Some(img) = base_image {
        e.attr("base_image", json!(img));
    }
    push_entity(out, e);
    for key in env_names {
        let mut c = Entity::new(
            entity_id(repo_id, kinds::CONFIGURATION, &key),
            kinds::CONFIGURATION,
            key,
        );
        c.attr("source", json!("env"));
        push_entity(out, c);
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Push an entity, dropping duplicates by id (stable, deterministic).
fn push_entity(out: &mut ConfigExtraction, e: Entity) {
    if out.entities.iter().any(|x| x.id == e.id) {
        return;
    }
    out.entities.push(e);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn k8s_deployment_service_namespace_configmap() {
        let content = r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: api
  namespace: prod
spec:
  replicas: 2
  template:
    spec:
      containers:
        - name: api
          image: myorg/api:1.2.3
          env:
            - name: DATABASE_URL
              value: postgres://user:pass@host/db
            - name: PORT
              value: "8080"
---
apiVersion: v1
kind: Service
metadata:
  name: api-svc
  namespace: prod
spec:
  ports:
    - port: 80
      targetPort: 8080
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: app-config
data:
  FOO: bar
  SECRET_KEY: never-store-me
---
apiVersion: v1
kind: Secret
metadata:
  name: app-secret
stringData:
  PASSWORD: hunter2
"#;
        let out = extract_infra_file("k8s/deploy.yaml", content, "repo");
        let by_name: std::collections::BTreeMap<_, _> = out
            .entities
            .iter()
            .map(|e| (e.name.clone(), e))
            .collect();

        // Deployment -> deployment_unit with image/replicas/namespace.
        let api = by_name["api"];
        assert_eq!(api.kind, kinds::DEPLOYMENT_UNIT);
        assert_eq!(api.attributes["image"], json!("myorg/api:1.2.3"));
        assert_eq!(api.attributes["replicas"], json!(2));
        assert_eq!(api.attributes["namespace"], json!("prod"));

        // Namespace entity + deployed_in relationship.
        assert_eq!(by_name["prod"].kind, kinds::DEPLOYMENT_UNIT);
        assert_eq!(
            out.relationships
                .iter()
                .filter(|(r, _)| r.predicate == predicates::DEPLOYED_IN)
                .count(),
            1
        );

        // Service -> deployment_unit marked as service with ports.
        let svc = by_name["api-svc"];
        assert_eq!(svc.kind, kinds::DEPLOYMENT_UNIT);
        assert_eq!(svc.attributes["service"], json!(true));
        assert_eq!(svc.attributes["ports"], json!([80]));

        // ConfigMap/Secret -> name-only entities, never values.
        assert_eq!(by_name["app-config"].kind, kinds::CONFIGURATION);
        assert_eq!(by_name["app-secret"].kind, kinds::SECRET_REFERENCE);

        // env entries -> configuration, name only, source=env.
        let db = by_name["DATABASE_URL"];
        assert_eq!(db.kind, kinds::CONFIGURATION);
        assert_eq!(db.attributes["source"], json!("env"));

        // SECURITY.md §4: no values anywhere.
        let serialized = serde_json::to_string(&out.entities).unwrap();
        assert!(!serialized.contains("hunter2"));
        assert!(!serialized.contains("never-store-me"));
        assert!(!serialized.contains("postgres://"));
        assert!(!serialized.contains("8080\""));
    }

    #[test]
    fn k8s_detects_k8s_suffix_and_skips_non_manifests() {
        let content = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: cfg\n";
        let out = extract_infra_file("deploy/thing.k8s.yaml", content, "repo");
        assert_eq!(out.entities.len(), 1);
        assert_eq!(out.entities[0].name, "cfg");
        // A plain yaml outside infra paths is untouched.
        let out = extract_infra_file("docs/notes.yaml", content, "repo");
        assert!(out.entities.is_empty());
    }

    #[test]
    fn k8s_malformed_yaml_never_panics() {
        let out = extract_infra_file("k8s/bad.yaml", "kind: [unclosed", "repo");
        assert!(out.entities.is_empty());
        // One bad document doesn't poison the rest.
        let content = "kind: [unclosed\n---\napiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: good\n";
        let out = extract_infra_file("k8s/bad.yaml", content, "repo");
        assert_eq!(out.entities.len(), 1);
        assert_eq!(out.entities[0].name, "good");
    }

    #[test]
    fn k8s_output_sorted_by_name() {
        let content = r#"
kind: Deployment
metadata: { name: zeta }
spec: { template: { spec: { containers: [] } } }
---
kind: Deployment
metadata: { name: alpha }
spec: { template: { spec: { containers: [] } } }
---
kind: Deployment
metadata: { name: bravo }
spec: { template: { spec: { containers: [] } } }
"#;
        let out = extract_infra_file("manifests/all.yaml", content, "repo");
        let names: Vec<&str> = out.entities.iter().map(|e| e.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn terraform_blocks() {
        let content = r#"
resource "aws_db_instance" "main" {
  engine = "postgres"
}
resource "aws_s3_bucket" "assets" {}
resource "aws_elasticache_cluster" "cache" {}
resource "aws_lambda_function" "ingest" {}
resource "aws_ecs_service" "web" {}
resource "random_pet" "name" {}
module "networking" {
  source = "./modules/networking"
}
variable "environment" {
  type = string
}
"#;
        let out = extract_infra_file("infra/main.tf", content, "repo");
        let by_name: std::collections::BTreeMap<_, _> = out
            .entities
            .iter()
            .map(|e| (e.name.clone(), e))
            .collect();

        let main = by_name["main"];
        assert_eq!(main.kind, kinds::DATA_STORE);
        assert_eq!(main.attributes["provider"], json!("aws"));
        assert_eq!(main.attributes["type"], json!("aws_db_instance"));
        assert_eq!(main.attributes["technology"], json!("postgres"));
        assert_eq!(by_name["assets"].attributes["technology"], json!("s3"));
        assert_eq!(by_name["cache"].attributes["technology"], json!("redis"));

        assert_eq!(by_name["ingest"].kind, kinds::DEPLOYMENT_UNIT);
        assert_eq!(by_name["web"].kind, kinds::DEPLOYMENT_UNIT);

        let pet = by_name["name"];
        assert_eq!(pet.kind, kinds::RESOURCE);
        assert_eq!(pet.attributes["provider_type"], json!("random_pet"));

        let net = by_name["networking"];
        assert_eq!(net.kind, kinds::EXTERNAL_SYSTEM);
        assert_eq!(net.attributes["terraform_module"], json!(true));

        let env_var = by_name["environment"];
        assert_eq!(env_var.kind, kinds::CONFIGURATION);
        assert_eq!(env_var.attributes["terraform_variable"], json!(true));
    }

    #[test]
    fn terraform_json() {
        let content = r#"{
          "resource": {
            "aws_db_instance": { "db": {} },
            "aws_s3_bucket": { "files": {} }
          },
          "module": { "vpc": {} },
          "variable": { "region": {} }
        }"#;
        let out = extract_infra_file("infra/main.tf.json", content, "repo");
        let by_name: std::collections::BTreeMap<_, _> = out
            .entities
            .iter()
            .map(|e| (e.name.clone(), e))
            .collect();
        assert_eq!(by_name["db"].attributes["technology"], json!("postgres"));
        assert_eq!(by_name["files"].attributes["technology"], json!("s3"));
        assert_eq!(by_name["vpc"].kind, kinds::EXTERNAL_SYSTEM);
        assert_eq!(by_name["region"].kind, kinds::CONFIGURATION);
    }

    #[test]
    fn github_actions_workflow_jobs_env() {
        let content = r#"
name: CI
on:
  push:
    branches: [main]
  pull_request:
env:
  CI: "true"
  NODE_ENV: production
jobs:
  build:
    runs-on: ubuntu-latest
    env:
      TOKEN: secret-value
    steps:
      - uses: actions/checkout@v4
        env:
          STEP_KEY: x
  test:
    runs-on: [ubuntu-latest, macos-latest]
    needs: build
"#;
        let out = extract_infra_file(".github/workflows/ci.yml", content, "repo");
        let by_name: std::collections::BTreeMap<_, _> = out
            .entities
            .iter()
            .map(|e| (e.name.clone(), e))
            .collect();

        // The workflow entity ("CI") shares its name with the workflow-level
        // env var "CI" — disambiguate by kind.
        let wf = out
            .entities
            .iter()
            .find(|e| e.kind == kinds::WORKFLOW && e.attributes.contains_key("jobs"))
            .unwrap();
        assert_eq!(wf.name, "CI");
        assert_eq!(wf.attributes["file"], json!(".github/workflows/ci.yml"));
        assert_eq!(wf.attributes["on"], json!(["pull_request", "push"]));
        let jobs = wf.attributes["jobs"].as_array().unwrap();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0]["name"], json!("build"));
        assert_eq!(jobs[0]["runs_on"], json!(["ubuntu-latest"]));
        assert_eq!(jobs[1]["name"], json!("test"));
        assert_eq!(jobs[1]["runs_on"], json!(["ubuntu-latest", "macos-latest"]));
        assert_eq!(jobs[1]["needs"], json!(["build"]));

        // Job entities + contains relationships.
        assert_eq!(by_name["CI-build"].kind, kinds::WORKFLOW);
        assert_eq!(by_name["CI-test"].kind, kinds::WORKFLOW);
        assert_eq!(
            out.relationships
                .iter()
                .filter(|(r, _)| r.predicate == predicates::CONTAINS)
                .count(),
            2
        );

        // env at workflow/job/step levels -> configuration names only.
        for key in ["NODE_ENV", "TOKEN", "STEP_KEY"] {
            assert_eq!(by_name[key].kind, kinds::CONFIGURATION);
            assert_eq!(by_name[key].attributes["source"], json!("env"));
        }
        let ci_env = out
            .entities
            .iter()
            .find(|e| e.kind == kinds::CONFIGURATION && e.name == "CI")
            .unwrap();
        assert_eq!(ci_env.attributes["source"], json!("env"));
        // Values never stored.
        let serialized = serde_json::to_string(&out.entities).unwrap();
        assert!(!serialized.contains("secret-value"));
        assert!(!serialized.contains("production"));
    }

    #[test]
    fn github_actions_unnamed_uses_file_stem() {
        let content = "on: push\njobs: {}\n";
        let out = extract_infra_file(".github/workflows/deploy.yaml", content, "repo");
        assert_eq!(out.entities.len(), 1);
        assert_eq!(out.entities[0].name, "deploy");
        assert_eq!(out.entities[0].attributes["on"], json!(["push"]));
    }

    #[test]
    fn dockerfile_from_env() {
        let content = "FROM node:20-alpine AS base\nENV NODE_ENV=production \\\n    PORT=8080\nRUN echo hi\n";
        let out = extract_infra_file("services/api/Dockerfile", content, "repo");
        let by_name: std::collections::BTreeMap<_, _> = out
            .entities
            .iter()
            .map(|e| (e.name.clone(), e))
            .collect();

        let unit = by_name["api"];
        assert_eq!(unit.kind, kinds::DEPLOYMENT_UNIT);
        assert_eq!(unit.attributes["dockerfile"], json!("services/api/Dockerfile"));
        assert_eq!(unit.attributes["base_image"], json!("node:20-alpine"));

        // ENV names only — values never persisted.
        assert_eq!(by_name["NODE_ENV"].kind, kinds::CONFIGURATION);
        assert_eq!(by_name["PORT"].kind, kinds::CONFIGURATION);
        let serialized = serde_json::to_string(&out.entities).unwrap();
        assert!(!serialized.contains("production"));
        assert!(!serialized.contains("8080"));

        // Root-level Dockerfile -> "root" unit; Dockerfile.dev also matches.
        let out = extract_infra_file("Dockerfile", "FROM scratch\n", "repo");
        assert_eq!(out.entities[0].name, "root");
        assert_eq!(out.entities[0].attributes["base_image"], json!("scratch"));
        let out = extract_infra_file("ops/Dockerfile.prod", "FROM alpine\n", "repo");
        assert_eq!(out.entities[0].name, "ops");
    }

    #[test]
    fn unknown_files_extract_nothing() {
        let out = extract_infra_file("src/main.rs", "fn main() {}", "repo");
        assert!(out.entities.is_empty());
        assert!(out.relationships.is_empty());
        let out = extract_infra_file("docker-compose.yml", "services: {}", "repo");
        assert!(out.entities.is_empty());
    }
}
