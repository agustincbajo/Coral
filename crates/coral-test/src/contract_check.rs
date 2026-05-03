//! `coral contract check` — cross-repo interface drift detection.
//!
//! In a multi-repo project where `worker` declares `depends_on = ["api"]`
//! and references HTTP endpoints from api's `openapi.yaml`, this module
//! answers: **does worker still match what api exposes?**
//!
//! v0.19.0 wave 4 covers the most common drift patterns:
//!
//! 1. **Endpoint removed**: worker's `.coral/tests/*.yaml` references
//!    `GET /users/{id}` but api's openapi.yaml no longer has it.
//! 2. **Method changed**: worker tests `POST /users` but api's spec
//!    only declares `PUT /users`.
//! 3. **Status code drift**: worker expects `status: 200` but api now
//!    only documents `201` and `400` for that path.
//! 4. **Cycle inconsistency**: api `depends_on` is declared but the
//!    upstream's openapi can't be located on disk (un-synced repo).
//!
//! **Scope**: deterministic (no LLM). Heuristic (path-prefix match,
//! method exact, status containment). Designed to give a `coral
//! contract check --strict` CI gate that fails fast when an interface
//! breaks before the test environment is even brought up.
//!
//! Pact-style consumer-driven contracts with `--can-i-deploy` land
//! in v0.20+. This module is the lighter sibling.

use crate::error::{TestError, TestResult};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Outcome of a contract check run. Each finding documents one drift.
#[derive(Debug, Clone, PartialEq)]
pub struct ContractReport {
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub severity: Severity,
    pub kind: FindingKind,
    pub consumer: String,
    pub provider: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FindingKind {
    /// The consumer references a path that the provider no longer
    /// declares.
    UnknownEndpoint { method: String, path: String },
    /// The consumer uses a method that the provider doesn't declare
    /// for that path.
    UnknownMethod {
        method: String,
        path: String,
        available: Vec<String>,
    },
    /// The consumer expects a status code the provider doesn't
    /// document.
    StatusDrift {
        method: String,
        path: String,
        expected: u16,
        documented: Vec<u16>,
    },
    /// `depends_on` declares an upstream repo that has no
    /// `openapi.yaml` on disk (un-synced or not API-shaped).
    MissingProviderSpec { provider_repo: String },
    /// Lockfile says the upstream repo is at SHA X but disk has SHA Y
    /// — surface so the user re-syncs before trusting the contract
    /// check.
    StaleLockfile {
        provider_repo: String,
        locked: String,
        on_disk: String,
    },
}

impl ContractReport {
    pub fn has_errors(&self) -> bool {
        self.findings.iter().any(|f| f.severity == Severity::Error)
    }
    pub fn has_findings(&self) -> bool {
        !self.findings.is_empty()
    }
}

/// Provider-side: every (method, path, declared statuses) extracted
/// from a single OpenAPI spec.
#[derive(Debug, Clone, Default)]
pub struct ProviderInterface {
    pub repo_name: String,
    /// `method → path → set of declared status codes`
    pub endpoints: BTreeMap<String, BTreeMap<String, BTreeSet<u16>>>,
}

/// Consumer-side: every (method, path, expected status) referenced by
/// the consumer's test suites.
#[derive(Debug, Clone, Default)]
pub struct ConsumerExpectations {
    pub repo_name: String,
    /// `method → path → expected status (or None for "any 2xx")`
    pub references: Vec<EndpointReference>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EndpointReference {
    pub method: String,
    pub path: String,
    pub expected_status: Option<u16>,
    /// Test file that produced this reference (for actionable error
    /// messages).
    pub source: PathBuf,
}

/// Top-level entry: walk the project, parse every repo's
/// `openapi.{yaml,yml,json}` as a `ProviderInterface`, walk every
/// repo's `.coral/tests/*.{yaml,yml,hurl}` as `ConsumerExpectations`,
/// then for each `[[repos]] depends_on` edge check the consumer
/// against each declared provider.
pub fn check_project(
    project_root: &Path,
    repos: &[(String, Vec<String>)], // (repo_name, depends_on)
) -> TestResult<ContractReport> {
    // 1. Collect provider interfaces.
    let mut providers: BTreeMap<String, ProviderInterface> = BTreeMap::new();
    for (repo_name, _) in repos {
        let repo_path = project_root.join("repos").join(repo_name);
        if let Some(iface) = parse_provider_for_repo(repo_name, &repo_path)? {
            providers.insert(repo_name.clone(), iface);
        }
    }
    // 2. Collect consumer expectations.
    let mut consumers: BTreeMap<String, ConsumerExpectations> = BTreeMap::new();
    for (repo_name, _) in repos {
        let repo_path = project_root.join("repos").join(repo_name);
        if let Some(expectations) = parse_consumer_for_repo(repo_name, &repo_path)? {
            consumers.insert(repo_name.clone(), expectations);
        }
    }
    // Also pick up project-root tests (not under repos/).
    if let Some(meta_consumer) = parse_consumer_for_repo("<project>", project_root)? {
        consumers.insert("<project>".to_string(), meta_consumer);
    }

    // 3. For each consumer, for each `depends_on` edge, run the
    //    drift checks against the corresponding provider.
    let mut findings = Vec::new();
    for (consumer_repo, depends_on) in repos {
        let consumer = match consumers.get(consumer_repo) {
            Some(c) if !c.references.is_empty() => c,
            _ => continue,
        };
        for provider_repo in depends_on {
            let provider = match providers.get(provider_repo) {
                Some(p) => p,
                None => {
                    findings.push(Finding {
                        severity: Severity::Warning,
                        kind: FindingKind::MissingProviderSpec {
                            provider_repo: provider_repo.clone(),
                        },
                        consumer: consumer_repo.clone(),
                        provider: provider_repo.clone(),
                        message: format!(
                            "consumer '{consumer_repo}' depends on '{provider_repo}' but no openapi spec found at repos/{provider_repo}/"
                        ),
                    });
                    continue;
                }
            };
            findings.extend(diff_consumer_against_provider(consumer, provider));
        }
    }

    Ok(ContractReport { findings })
}

/// Look for `openapi.{yaml,yml,json}` or `swagger.{yaml,yml,json}` at
/// the repo root or one level under `api/` / `spec/`. We don't walk
/// the entire tree because real openapi files live at predictable
/// paths and we want fast-fail behavior.
fn parse_provider_for_repo(
    repo_name: &str,
    repo_path: &Path,
) -> TestResult<Option<ProviderInterface>> {
    if !repo_path.exists() {
        return Ok(None);
    }
    let candidates: Vec<PathBuf> = ["", "api/", "spec/", "openapi/", "docs/"]
        .iter()
        .flat_map(|prefix| {
            [
                "openapi.yaml",
                "openapi.yml",
                "openapi.json",
                "swagger.yaml",
                "swagger.yml",
                "swagger.json",
            ]
            .iter()
            .map(move |name| repo_path.join(format!("{prefix}{name}")))
        })
        .collect();
    let spec_path = match candidates.into_iter().find(|p| p.is_file()) {
        Some(p) => p,
        None => return Ok(None),
    };
    let raw = std::fs::read_to_string(&spec_path).map_err(|source| TestError::Io {
        path: spec_path.clone(),
        source,
    })?;
    let value: serde_json::Value = if spec_path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_lowercase)
        .as_deref()
        == Some("json")
    {
        serde_json::from_str(&raw).map_err(|e| TestError::InvalidSpec {
            path: spec_path.clone(),
            reason: e.to_string(),
        })?
    } else {
        serde_yaml_ng::from_str(&raw).map_err(|e| TestError::InvalidSpec {
            path: spec_path.clone(),
            reason: e.to_string(),
        })?
    };
    let mut iface = ProviderInterface {
        repo_name: repo_name.to_string(),
        endpoints: BTreeMap::new(),
    };
    if let Some(paths) = value.get("paths").and_then(|v| v.as_object()) {
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
                let mut statuses = BTreeSet::new();
                if let Some(responses) = op_value.get("responses").and_then(|v| v.as_object()) {
                    for code_str in responses.keys() {
                        if let Ok(s) = code_str.parse::<u16>() {
                            statuses.insert(s);
                        }
                    }
                }
                if statuses.is_empty() {
                    statuses.insert(200);
                }
                iface
                    .endpoints
                    .entry(method)
                    .or_default()
                    .insert(path_str.clone(), statuses);
            }
        }
    }
    Ok(Some(iface))
}

fn is_http_method(s: &str) -> bool {
    matches!(
        s,
        "GET" | "POST" | "PUT" | "DELETE" | "PATCH" | "HEAD" | "OPTIONS"
    )
}

/// Walk `<repo>/.coral/tests/*.{yaml,yml,hurl}` for HTTP step
/// references — every `(method, path, expected_status)` becomes an
/// `EndpointReference`.
pub fn parse_consumer_for_repo(
    repo_name: &str,
    repo_path: &Path,
) -> TestResult<Option<ConsumerExpectations>> {
    let tests_dir = repo_path.join(".coral/tests");
    if !tests_dir.is_dir() {
        return Ok(None);
    }
    let mut expectations = ConsumerExpectations {
        repo_name: repo_name.to_string(),
        references: Vec::new(),
    };
    for entry in std::fs::read_dir(&tests_dir).map_err(|source| TestError::Io {
        path: tests_dir.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| TestError::Io {
            path: tests_dir.clone(),
            source,
        })?;
        let path = entry.path();
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        match ext {
            "yaml" | "yml" => extract_from_yaml(&path, &mut expectations.references)?,
            "hurl" => extract_from_hurl(&path, &mut expectations.references)?,
            _ => continue,
        }
    }
    if expectations.references.is_empty() {
        return Ok(None);
    }
    Ok(Some(expectations))
}

fn extract_from_yaml(path: &Path, refs: &mut Vec<EndpointReference>) -> TestResult<()> {
    let raw = std::fs::read_to_string(path).map_err(|source| TestError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let value: serde_json::Value =
        serde_yaml_ng::from_str(&raw).map_err(|e| TestError::InvalidSpec {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;
    let steps = match value.get("steps").and_then(|v| v.as_array()) {
        Some(s) => s,
        None => return Ok(()),
    };
    for step in steps {
        if let Some(http_line) = step.get("http").and_then(|v| v.as_str()) {
            if let Some((method, path_str)) = parse_http_line(http_line) {
                let expected_status = step
                    .get("expect")
                    .and_then(|e| e.get("status"))
                    .and_then(|s| s.as_u64())
                    .map(|s| s as u16);
                refs.push(EndpointReference {
                    method,
                    path: path_str,
                    expected_status,
                    source: path.to_path_buf(),
                });
            }
        }
    }
    Ok(())
}

fn extract_from_hurl(path: &Path, refs: &mut Vec<EndpointReference>) -> TestResult<()> {
    let raw = std::fs::read_to_string(path).map_err(|source| TestError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut current_method: Option<String> = None;
    let mut current_path: Option<String> = None;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((method, path_str)) = parse_http_line(trimmed) {
            current_method = Some(method);
            current_path = Some(path_str);
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("HTTP ") {
            if let (Some(m), Some(p)) = (current_method.take(), current_path.take()) {
                let expected_status = rest.trim().parse::<u16>().ok();
                refs.push(EndpointReference {
                    method: m,
                    path: p,
                    expected_status,
                    source: path.to_path_buf(),
                });
            }
        }
    }
    Ok(())
}

fn parse_http_line(line: &str) -> Option<(String, String)> {
    let mut parts = line.splitn(2, ' ');
    let method = match parts.next()?.trim() {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "DELETE" => "DELETE",
        "PATCH" => "PATCH",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        _ => return None,
    };
    let raw_url = parts.next()?.trim();
    let path = if let Some((_scheme, rest)) = raw_url.split_once("://") {
        match rest.find('/') {
            Some(i) => rest[i..].to_string(),
            None => "/".to_string(),
        }
    } else {
        raw_url.to_string()
    };
    if path.is_empty() {
        return None;
    }
    Some((method.to_string(), path))
}

/// Core diff: for each consumer reference, look up the path in the
/// provider's endpoint table and emit findings for any mismatch.
pub fn diff_consumer_against_provider(
    consumer: &ConsumerExpectations,
    provider: &ProviderInterface,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    for r in &consumer.references {
        let methods_for_path = collect_methods_for_path(provider, &r.path);
        if methods_for_path.is_empty() {
            findings.push(Finding {
                severity: Severity::Error,
                kind: FindingKind::UnknownEndpoint {
                    method: r.method.clone(),
                    path: r.path.clone(),
                },
                consumer: consumer.repo_name.clone(),
                provider: provider.repo_name.clone(),
                message: format!(
                    "consumer '{}' references {} {} but provider '{}' does not declare it (in {})",
                    consumer.repo_name,
                    r.method,
                    r.path,
                    provider.repo_name,
                    r.source.display()
                ),
            });
            continue;
        }
        if !methods_for_path.contains(&r.method) {
            findings.push(Finding {
                severity: Severity::Error,
                kind: FindingKind::UnknownMethod {
                    method: r.method.clone(),
                    path: r.path.clone(),
                    available: methods_for_path.to_vec(),
                },
                consumer: consumer.repo_name.clone(),
                provider: provider.repo_name.clone(),
                message: format!(
                    "consumer '{}' uses {} on {} but provider '{}' only declares {:?} (in {})",
                    consumer.repo_name,
                    r.method,
                    r.path,
                    provider.repo_name,
                    methods_for_path,
                    r.source.display()
                ),
            });
            continue;
        }
        if let Some(expected) = r.expected_status {
            // Use openapi_path_matches to find the right by-path entry
            // when the consumer uses a concrete path against an
            // OpenAPI path with `{param}` placeholders.
            let documented: Vec<u16> = provider
                .endpoints
                .get(&r.method)
                .map(|by_path| {
                    by_path
                        .iter()
                        .find(|(spec_path, _)| openapi_path_matches(spec_path, &r.path))
                        .map(|(_, s)| s.iter().copied().collect::<Vec<_>>())
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            if !documented.contains(&expected) {
                findings.push(Finding {
                    severity: Severity::Warning,
                    kind: FindingKind::StatusDrift {
                        method: r.method.clone(),
                        path: r.path.clone(),
                        expected,
                        documented: documented.clone(),
                    },
                    consumer: consumer.repo_name.clone(),
                    provider: provider.repo_name.clone(),
                    message: format!(
                        "consumer '{}' expects {} from {} {} but provider documents {:?} (in {})",
                        consumer.repo_name,
                        expected,
                        r.method,
                        r.path,
                        documented,
                        r.source.display()
                    ),
                });
            }
        }
    }
    findings
}

/// Look up the set of HTTP methods the provider supports for `path`.
/// Path matching is exact + prefix-aware: `/users/{id}` in the spec
/// matches `/users/42` in the consumer (parameter substitution),
/// `/users` in the spec matches `/users` literal in the consumer.
fn collect_methods_for_path(provider: &ProviderInterface, consumer_path: &str) -> Vec<String> {
    let mut methods = Vec::new();
    for (method, by_path) in &provider.endpoints {
        for spec_path in by_path.keys() {
            if openapi_path_matches(spec_path, consumer_path) {
                methods.push(method.clone());
            }
        }
    }
    methods
}

/// Match an OpenAPI path with `{param}` placeholders against a
/// consumer-side concrete path. Each `{name}` matches any non-`/`
/// segment.
pub fn openapi_path_matches(spec_path: &str, consumer_path: &str) -> bool {
    if spec_path == consumer_path {
        return true;
    }
    let spec_segments: Vec<&str> = spec_path.split('/').collect();
    let consumer_segments: Vec<&str> = consumer_path.split('/').collect();
    if spec_segments.len() != consumer_segments.len() {
        return false;
    }
    for (s, c) in spec_segments.iter().zip(consumer_segments.iter()) {
        if s.starts_with('{') && s.ends_with('}') {
            // Parameter — accept any non-empty segment.
            if c.is_empty() {
                return false;
            }
            // ${var} substitution from coral test runner: also a wildcard.
            continue;
        }
        if s != c {
            // A consumer-side `${var}` placeholder also wildcards.
            if c.starts_with("${") && c.ends_with('}') {
                continue;
            }
            return false;
        }
    }
    true
}

/// Render the report as a human-readable Markdown table.
pub fn render_report_markdown(report: &ContractReport) -> String {
    if report.findings.is_empty() {
        return "✔ no contract drift detected\n".to_string();
    }
    let mut out = String::new();
    out.push_str("# Contract drift report\n\n");
    out.push_str(&format!(
        "Found **{}** finding(s) ({} error(s), {} warning(s)):\n\n",
        report.findings.len(),
        report
            .findings
            .iter()
            .filter(|f| f.severity == Severity::Error)
            .count(),
        report
            .findings
            .iter()
            .filter(|f| f.severity == Severity::Warning)
            .count(),
    ));
    out.push_str("| severity | consumer | provider | message |\n");
    out.push_str("|----------|----------|----------|---------|\n");
    for f in &report.findings {
        let prefix = match f.severity {
            Severity::Error => "✘",
            Severity::Warning => "⚠",
            Severity::Info => "ℹ",
        };
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            prefix, f.consumer, f.provider, f.message
        ));
    }
    out
}

/// Render the report as JSON for CI consumption.
pub fn render_report_json(report: &ContractReport) -> serde_json::Value {
    let findings: Vec<serde_json::Value> = report
        .findings
        .iter()
        .map(|f| {
            serde_json::json!({
                "severity": match f.severity {
                    Severity::Error => "error",
                    Severity::Warning => "warning",
                    Severity::Info => "info",
                },
                "consumer": f.consumer,
                "provider": f.provider,
                "message": f.message,
            })
        })
        .collect();
    serde_json::json!({
        "findings": findings,
        "has_errors": report.has_errors(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn openapi_path_matches_exact_equality() {
        assert!(openapi_path_matches("/users", "/users"));
        assert!(!openapi_path_matches("/users", "/posts"));
    }

    #[test]
    fn openapi_path_matches_substitutes_braces() {
        assert!(openapi_path_matches("/users/{id}", "/users/42"));
        assert!(openapi_path_matches("/users/{id}/posts", "/users/42/posts"));
        assert!(!openapi_path_matches("/users/{id}", "/users"));
        assert!(!openapi_path_matches("/users", "/users/42"));
    }

    #[test]
    fn openapi_path_matches_consumer_var_wildcard() {
        // `${var}` in the consumer (substituted at runtime) also acts
        // as a wildcard segment when checking for drift statically.
        assert!(openapi_path_matches("/users/{id}", "/users/${user_id}"));
        assert!(openapi_path_matches("/users/42", "/users/${id}"));
    }

    #[test]
    fn detects_unknown_endpoint() {
        let provider = ProviderInterface {
            repo_name: "api".into(),
            endpoints: {
                let mut m = BTreeMap::new();
                let mut by_path = BTreeMap::new();
                by_path.insert("/users".into(), [200].into_iter().collect());
                m.insert("GET".into(), by_path);
                m
            },
        };
        let consumer = ConsumerExpectations {
            repo_name: "worker".into(),
            references: vec![EndpointReference {
                method: "GET".into(),
                path: "/orders".into(), // not declared!
                expected_status: Some(200),
                source: PathBuf::from("/x/.coral/tests/api.yaml"),
            }],
        };
        let findings = diff_consumer_against_provider(&consumer, &provider);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Error);
        assert!(matches!(
            findings[0].kind,
            FindingKind::UnknownEndpoint { .. }
        ));
    }

    #[test]
    fn detects_unknown_method() {
        let provider = ProviderInterface {
            repo_name: "api".into(),
            endpoints: {
                let mut m = BTreeMap::new();
                let mut by_path = BTreeMap::new();
                by_path.insert("/users".into(), [200].into_iter().collect());
                m.insert("GET".into(), by_path);
                m
            },
        };
        let consumer = ConsumerExpectations {
            repo_name: "worker".into(),
            references: vec![EndpointReference {
                method: "POST".into(),
                path: "/users".into(),
                expected_status: Some(201),
                source: PathBuf::from("/x/.coral/tests/api.yaml"),
            }],
        };
        let findings = diff_consumer_against_provider(&consumer, &provider);
        assert_eq!(findings.len(), 1);
        assert!(matches!(
            findings[0].kind,
            FindingKind::UnknownMethod { ref available, .. } if available == &["GET".to_string()]
        ));
    }

    #[test]
    fn detects_status_drift() {
        let provider = ProviderInterface {
            repo_name: "api".into(),
            endpoints: {
                let mut m = BTreeMap::new();
                let mut by_path = BTreeMap::new();
                by_path.insert("/users".into(), [201, 400].into_iter().collect());
                m.insert("POST".into(), by_path);
                m
            },
        };
        let consumer = ConsumerExpectations {
            repo_name: "worker".into(),
            references: vec![EndpointReference {
                method: "POST".into(),
                path: "/users".into(),
                expected_status: Some(200), // not in {201, 400}
                source: PathBuf::from("/x/.coral/tests/api.yaml"),
            }],
        };
        let findings = diff_consumer_against_provider(&consumer, &provider);
        assert_eq!(findings.len(), 1);
        assert!(matches!(
            findings[0].kind,
            FindingKind::StatusDrift { expected: 200, .. }
        ));
        assert_eq!(findings[0].severity, Severity::Warning);
    }

    #[test]
    fn happy_path_no_drift() {
        let provider = ProviderInterface {
            repo_name: "api".into(),
            endpoints: {
                let mut m = BTreeMap::new();
                let mut by_path_get = BTreeMap::new();
                by_path_get.insert("/users".into(), [200].into_iter().collect());
                by_path_get.insert("/users/{id}".into(), [200, 404].into_iter().collect());
                m.insert("GET".into(), by_path_get);
                m
            },
        };
        let consumer = ConsumerExpectations {
            repo_name: "worker".into(),
            references: vec![
                EndpointReference {
                    method: "GET".into(),
                    path: "/users".into(),
                    expected_status: Some(200),
                    source: PathBuf::from("/x/.coral/tests/api.yaml"),
                },
                EndpointReference {
                    method: "GET".into(),
                    path: "/users/42".into(), // matches /users/{id}
                    expected_status: Some(200),
                    source: PathBuf::from("/x/.coral/tests/api.yaml"),
                },
            ],
        };
        assert!(diff_consumer_against_provider(&consumer, &provider).is_empty());
    }

    #[test]
    fn end_to_end_check_project_detects_removed_endpoint() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        // Provider repo with /users only.
        write(
            &root.join("repos/api/openapi.yaml"),
            r#"openapi: 3.0.0
info: { title: api, version: 1.0 }
paths:
  /users:
    get:
      responses: { '200': { description: ok } }
"#,
        );
        // Consumer repo with tests against /users + /users/{id}.
        write(
            &root.join("repos/worker/.coral/tests/integration.yaml"),
            r#"name: worker integration
service: worker
steps:
  - http: GET /users
    expect: { status: 200 }
  - http: GET /users/42
    expect: { status: 200 }
"#,
        );

        let report = check_project(
            root,
            &[
                ("api".to_string(), vec![]),
                ("worker".to_string(), vec!["api".to_string()]),
            ],
        )
        .unwrap();
        assert!(report.has_errors());
        // The /users/42 reference is missing from api's spec.
        let unknown = report
            .findings
            .iter()
            .filter(|f| matches!(f.kind, FindingKind::UnknownEndpoint { .. }))
            .count();
        assert_eq!(
            unknown, 1,
            "expected 1 unknown-endpoint finding, got: {:?}",
            report.findings
        );
    }

    #[test]
    fn end_to_end_check_project_passes_when_in_sync() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write(
            &root.join("repos/api/openapi.yaml"),
            r#"openapi: 3.0.0
info: { title: api, version: 1.0 }
paths:
  /users:
    get:
      responses: { '200': { description: ok } }
  /users/{id}:
    get:
      responses: { '200': { description: ok } }
"#,
        );
        write(
            &root.join("repos/worker/.coral/tests/integration.yaml"),
            r#"name: worker integration
service: worker
steps:
  - http: GET /users
    expect: { status: 200 }
  - http: GET /users/42
    expect: { status: 200 }
"#,
        );
        let report = check_project(
            root,
            &[
                ("api".to_string(), vec![]),
                ("worker".to_string(), vec!["api".to_string()]),
            ],
        )
        .unwrap();
        assert!(
            !report.has_errors(),
            "expected no errors, got: {:?}",
            report.findings
        );
    }

    #[test]
    fn end_to_end_check_project_warns_on_status_drift() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write(
            &root.join("repos/api/openapi.yaml"),
            r#"openapi: 3.0.0
info: { title: api, version: 1.0 }
paths:
  /users:
    post:
      responses:
        '201': { description: created }
        '400': { description: invalid }
"#,
        );
        write(
            &root.join("repos/worker/.coral/tests/integration.yaml"),
            r#"name: worker integration
service: worker
steps:
  - http: POST /users
    expect: { status: 200 }
"#,
        );
        let report = check_project(
            root,
            &[
                ("api".to_string(), vec![]),
                ("worker".to_string(), vec!["api".to_string()]),
            ],
        )
        .unwrap();
        let drifts: Vec<_> = report
            .findings
            .iter()
            .filter(|f| matches!(f.kind, FindingKind::StatusDrift { .. }))
            .collect();
        assert_eq!(drifts.len(), 1);
        assert_eq!(drifts[0].severity, Severity::Warning);
    }

    #[test]
    fn end_to_end_check_project_warns_when_provider_repo_missing() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        // Worker has tests but api repo isn't synced.
        write(
            &root.join("repos/worker/.coral/tests/integration.yaml"),
            r#"name: worker
service: worker
steps:
  - http: GET /users
    expect: { status: 200 }
"#,
        );
        let report = check_project(
            root,
            &[
                ("api".to_string(), vec![]),
                ("worker".to_string(), vec!["api".to_string()]),
            ],
        )
        .unwrap();
        assert!(
            report
                .findings
                .iter()
                .any(|f| matches!(f.kind, FindingKind::MissingProviderSpec { .. }))
        );
    }

    #[test]
    fn render_markdown_says_no_drift_when_empty() {
        let report = ContractReport { findings: vec![] };
        assert!(render_report_markdown(&report).contains("no contract drift"));
    }

    #[test]
    fn render_markdown_includes_severity_and_message() {
        let report = ContractReport {
            findings: vec![Finding {
                severity: Severity::Error,
                kind: FindingKind::UnknownEndpoint {
                    method: "GET".into(),
                    path: "/x".into(),
                },
                consumer: "worker".into(),
                provider: "api".into(),
                message: "test message".into(),
            }],
        };
        let md = render_report_markdown(&report);
        assert!(md.contains("✘"));
        assert!(md.contains("test message"));
    }
}
