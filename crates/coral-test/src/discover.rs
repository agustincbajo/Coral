//! `coral test discover` — auto-generate TestCases from on-disk specs
//! without invoking an LLM.
//!
//! v0.18 wave 3a covers OpenAPI 3.x (YAML or JSON). AsyncAPI and proto
//! follow in v0.19 with Testcontainers Kafka/Rabbit and gRPC reflection.
//!
//! The strategy is intentionally conservative: one TestCase per
//! `(path, method)` with a generated HTTP step that asserts the
//! response status code from the spec's `responses` block. We never
//! fabricate request bodies — endpoints that require non-empty bodies
//! get skipped (better a small honest test set than a flaky generated
//! one). Users who want richer cases run `coral test generate
//! --auto-validate` (LLM-augmented, v0.19).

use crate::error::{TestError, TestResult};
use crate::spec::{TestCase, TestKind, TestSource, TestSpec};
use crate::user_defined_runner::{HttpExpect, HttpStep, YamlStep, YamlSuite};
use std::path::{Path, PathBuf};

/// Walk `project_root` for OpenAPI specs (`openapi.yaml`,
/// `openapi.json`, `swagger.yaml`, `swagger.json`) and emit one
/// `TestCase` per `(path, method)`. The cases are tagged
/// `["smoke", "discovered"]` so `coral test --tag smoke` picks them up
/// alongside healthchecks.
pub fn discover_openapi_in_project(project_root: &Path) -> TestResult<Vec<DiscoveredCase>> {
    let mut out = Vec::new();
    for spec_path in find_openapi_specs(project_root)? {
        match parse_spec_file(&spec_path) {
            Ok(suites) => {
                for (case, suite) in suites {
                    out.push(DiscoveredCase {
                        case,
                        suite,
                        source_spec: spec_path.clone(),
                    });
                }
            }
            Err(e) => {
                tracing::warn!(
                    path = %spec_path.display(),
                    "failed to parse OpenAPI spec; skipping: {e}"
                );
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct DiscoveredCase {
    pub case: TestCase,
    pub suite: YamlSuite,
    pub source_spec: PathBuf,
}

/// Walk `project_root` for `openapi.{yaml,yml,json}` and
/// `swagger.{yaml,yml,json}`. Skips `.git/`, `.coral/`, `node_modules/`,
/// `target/` for performance — the repo we walk is typically a
/// multi-repo project root with checked-out service repos.
fn find_openapi_specs(project_root: &Path) -> TestResult<Vec<PathBuf>> {
    use std::collections::VecDeque;
    let mut out = Vec::new();
    let mut stack = VecDeque::new();
    stack.push_back(project_root.to_path_buf());
    while let Some(dir) = stack.pop_front() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = match path.file_name().and_then(|s| s.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if path.is_dir() {
                if matches!(
                    name.as_str(),
                    ".git" | ".coral" | "node_modules" | "target" | "vendor" | "dist" | "build"
                ) {
                    continue;
                }
                stack.push_back(path);
                continue;
            }
            if is_openapi_filename(&name) {
                out.push(path);
            }
        }
    }
    Ok(out)
}

fn is_openapi_filename(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(
        lower.as_str(),
        "openapi.yaml"
            | "openapi.yml"
            | "openapi.json"
            | "swagger.yaml"
            | "swagger.yml"
            | "swagger.json"
    )
}

/// Parse a single OpenAPI file. Returns one (TestCase, YamlSuite) pair
/// per (path, method) operation that has at least one declared
/// success response (2xx).
fn parse_spec_file(path: &Path) -> TestResult<Vec<(TestCase, YamlSuite)>> {
    let raw = std::fs::read_to_string(path).map_err(|source| TestError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let value: serde_json::Value = if path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_lowercase)
        .as_deref()
        == Some("json")
    {
        serde_json::from_str(&raw).map_err(|e| TestError::InvalidSpec {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?
    } else {
        serde_yaml_ng::from_str(&raw).map_err(|e| TestError::InvalidSpec {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?
    };

    let paths = match value.get("paths").and_then(|v| v.as_object()) {
        Some(p) => p,
        None => return Ok(Vec::new()),
    };

    let mut cases = Vec::new();
    for (path_str, ops_value) in paths {
        let ops = match ops_value.as_object() {
            Some(o) => o,
            None => continue,
        };
        for (method_str, op_value) in ops {
            let method = method_str.to_uppercase();
            if !is_http_method(&method) {
                continue;
            }
            // Only generate cases for operations that don't require a
            // body — we don't fabricate request bodies.
            if has_required_body(op_value) {
                continue;
            }
            let expect_status = pick_success_status(op_value);
            let suite = YamlSuite {
                name: format!("openapi {method} {path_str}"),
                service: extract_service_tag(op_value),
                tags: vec!["smoke".into(), "discovered".into()],
                steps: vec![YamlStep::Http(HttpStep {
                    http: format!("{method} {path_str}"),
                    headers: Default::default(),
                    body: None,
                    expect: HttpExpect {
                        status: Some(expect_status),
                        body_contains: None,
                        snapshot: None,
                    },
                    retry: None,
                    capture: Default::default(),
                    id: None,
                    depends_on: Vec::new(),
                })],
                retry: None,
            };
            let case = TestCase {
                id: format!("openapi:{method}:{path_str}"),
                name: suite.name.clone(),
                kind: TestKind::UserDefined,
                service: suite.service.clone(),
                tags: suite.tags.clone(),
                source: TestSource::Discovered {
                    from: path.to_string_lossy().into_owned(),
                },
                spec: TestSpec(serde_json::to_value(&suite).unwrap_or(serde_json::Value::Null)),
            };
            cases.push((case, suite));
        }
    }
    Ok(cases)
}

fn is_http_method(s: &str) -> bool {
    matches!(
        s,
        "GET" | "POST" | "PUT" | "DELETE" | "PATCH" | "HEAD" | "OPTIONS"
    )
}

fn has_required_body(op: &serde_json::Value) -> bool {
    op.get("requestBody")
        .and_then(|rb| rb.get("required"))
        .and_then(|r| r.as_bool())
        .unwrap_or(false)
}

/// Pick the smallest 2xx status code declared in `responses`. Falls
/// back to 200 when nothing's declared.
fn pick_success_status(op: &serde_json::Value) -> u16 {
    let responses = match op.get("responses").and_then(|v| v.as_object()) {
        Some(r) => r,
        None => return 200,
    };
    let mut candidates: Vec<u16> = responses
        .keys()
        .filter_map(|k| k.parse::<u16>().ok())
        .filter(|s| (200..300).contains(s))
        .collect();
    candidates.sort_unstable();
    candidates.first().copied().unwrap_or(200)
}

/// Extract the service name from an OpenAPI tag — by convention many
/// orgs put `tags: [<service>]` on every operation. Falls back to the
/// first tag, or None.
fn extract_service_tag(op: &serde_json::Value) -> Option<String> {
    op.get("tags")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_str())
        .map(str::to_lowercase)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn detects_openapi_yaml_filename_case_insensitive() {
        assert!(is_openapi_filename("openapi.yaml"));
        assert!(is_openapi_filename("OpenAPI.yml"));
        assert!(is_openapi_filename("swagger.json"));
        assert!(!is_openapi_filename("README.md"));
        assert!(!is_openapi_filename("Cargo.toml"));
    }

    #[test]
    fn pick_success_status_picks_lowest_2xx() {
        let op = serde_json::json!({
            "responses": {
                "200": {},
                "204": {},
                "400": {},
                "500": {}
            }
        });
        assert_eq!(pick_success_status(&op), 200);
    }

    #[test]
    fn pick_success_status_falls_back_to_200() {
        let op = serde_json::json!({});
        assert_eq!(pick_success_status(&op), 200);
    }

    #[test]
    fn pick_success_status_picks_201_when_no_200() {
        let op = serde_json::json!({"responses": {"201": {}, "401": {}}});
        assert_eq!(pick_success_status(&op), 201);
    }

    #[test]
    fn parse_spec_file_emits_one_case_per_get_endpoint() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("openapi.yaml");
        std::fs::write(
            &path,
            r#"openapi: 3.0.0
info:
  title: Demo
  version: 1.0.0
paths:
  /users:
    get:
      tags: [api]
      responses:
        '200':
          description: ok
  /users/{id}:
    get:
      tags: [api]
      responses:
        '200':
          description: ok
        '404':
          description: not found
"#,
        )
        .unwrap();
        let cases = parse_spec_file(&path).unwrap();
        assert_eq!(cases.len(), 2);
        assert!(cases.iter().any(|(c, _)| c.id == "openapi:GET:/users"));
        assert!(cases.iter().any(|(c, _)| c.id == "openapi:GET:/users/{id}"));
    }

    #[test]
    fn parse_spec_file_skips_endpoints_with_required_body() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("openapi.yaml");
        std::fs::write(
            &path,
            r#"openapi: 3.0.0
info:
  title: Demo
  version: 1.0.0
paths:
  /users:
    post:
      requestBody:
        required: true
        content:
          application/json: { schema: { type: object } }
      responses:
        '201':
          description: created
    get:
      responses:
        '200':
          description: ok
"#,
        )
        .unwrap();
        let cases = parse_spec_file(&path).unwrap();
        assert_eq!(cases.len(), 1, "POST with required body should be skipped");
        assert!(cases.iter().any(|(c, _)| c.id.contains("GET")));
    }

    #[test]
    fn parse_spec_file_handles_json_format() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("openapi.json");
        std::fs::write(
            &path,
            r#"{
                "openapi": "3.0.0",
                "info": {"title": "Demo", "version": "1.0.0"},
                "paths": {
                    "/health": {
                        "get": {"responses": {"200": {"description": "ok"}}}
                    }
                }
            }"#,
        )
        .unwrap();
        let cases = parse_spec_file(&path).unwrap();
        assert_eq!(cases.len(), 1);
    }

    #[test]
    fn discover_walks_subdirs_and_skips_excluded_dirs() {
        let dir = TempDir::new().unwrap();
        let api_dir = dir.path().join("repos/api");
        std::fs::create_dir_all(&api_dir).unwrap();
        std::fs::write(
            api_dir.join("openapi.yaml"),
            r#"openapi: 3.0.0
info: { title: x, version: 1.0 }
paths:
  /h:
    get:
      responses:
        '200':
          description: ok
"#,
        )
        .unwrap();
        // `node_modules` should be skipped.
        let nm = dir.path().join("node_modules/foo");
        std::fs::create_dir_all(&nm).unwrap();
        std::fs::write(
            nm.join("openapi.yaml"),
            r#"openapi: 3.0.0
info: { title: bad, version: 1.0 }
paths:
  /shadow:
    get:
      responses:
        '200':
          description: ok
"#,
        )
        .unwrap();
        let discovered = discover_openapi_in_project(dir.path()).unwrap();
        assert_eq!(discovered.len(), 1);
        assert!(discovered[0].case.id.contains("/h"));
    }
}
