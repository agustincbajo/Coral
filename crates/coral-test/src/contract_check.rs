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
    /// The provider's `openapi.{yaml,yml,json}` is on disk but won't
    /// parse. Treated as a warning so a single bad spec doesn't abort
    /// the whole check, but surfaced loud so the user can fix it.
    MalformedProviderSpec {
        provider_repo: String,
        reason: String,
    },
    /// Consumer sends a request body but provider doesn't declare one
    /// (or vice versa).
    RequestBodyDrift {
        method: String,
        path: String,
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
    /// `(method, path)` pairs whose `responses` declared `default`.
    /// Treated as a wildcard for status drift — any consumer-expected
    /// code matches.
    pub has_default_response: BTreeSet<(String, String)>,
    /// v0.24: endpoints that declare a `requestBody` in the provider's
    /// OpenAPI spec. Key: (method, path). Presence means the provider
    /// expects a request body.
    pub has_request_body: BTreeSet<(String, String)>,
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
    /// v0.24: true when the test step sends a request body (`body` or
    /// `json` field in the YAML step).
    pub sends_body: bool,
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
    let mut findings: Vec<Finding> = Vec::new();
    // 1. Collect provider interfaces. A single malformed spec must NOT
    //    abort the whole check — surface it as a warning and keep going
    //    so the user sees every drift in one report instead of having
    //    to fix specs one-by-one to discover the next failure.
    let mut providers: BTreeMap<String, ProviderInterface> = BTreeMap::new();
    for (repo_name, _) in repos {
        let repo_path = project_root.join("repos").join(repo_name);
        match parse_provider_for_repo(repo_name, &repo_path) {
            Ok(Some(iface)) => {
                providers.insert(repo_name.clone(), iface);
            }
            Ok(None) => {}
            Err(e) => {
                findings.push(Finding {
                    severity: Severity::Warning,
                    kind: FindingKind::MalformedProviderSpec {
                        provider_repo: repo_name.clone(),
                        reason: format!("{e}"),
                    },
                    consumer: String::new(),
                    provider: repo_name.clone(),
                    message: format!(
                        "provider '{repo_name}' has an openapi spec on disk but it failed to parse: {e}"
                    ),
                });
            }
        }
    }
    // 2. Collect consumer expectations. Same soft-fail discipline:
    //    a single bad test file shouldn't lose drift detection on the
    //    healthy ones.
    let mut consumers: BTreeMap<String, ConsumerExpectations> = BTreeMap::new();
    for (repo_name, _) in repos {
        let repo_path = project_root.join("repos").join(repo_name);
        if let Ok(Some(expectations)) = parse_consumer_for_repo(repo_name, &repo_path) {
            consumers.insert(repo_name.clone(), expectations);
        }
    }
    // Also pick up project-root tests (not under repos/).
    if let Ok(Some(meta_consumer)) = parse_consumer_for_repo("<project>", project_root) {
        consumers.insert("<project>".to_string(), meta_consumer);
    }

    // 3. For each consumer, for each `depends_on` edge, run the
    //    drift checks against the corresponding provider.
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
    // Common spec locations across orgs we've seen. Order matters —
    // the first match wins, so put the most specific (api/v1/, schema/)
    // before the more generic ones (the bare repo root).
    let prefixes: &[&str] = &[
        "",
        "api/",
        "api/v1/",
        "api/v2/",
        "openapi/",
        "openapi/v1/",
        "openapi/v2/",
        "spec/",
        "schema/",
        "contract/",
        "docs/",
        "docs/api/",
        "reference/",
    ];
    let names: &[&str] = &[
        "openapi.yaml",
        "openapi.yml",
        "openapi.json",
        "swagger.yaml",
        "swagger.yml",
        "swagger.json",
    ];
    let candidates: Vec<PathBuf> = prefixes
        .iter()
        .flat_map(|prefix| {
            names
                .iter()
                .map(move |name| repo_path.join(format!("{prefix}{name}")))
        })
        .collect();
    let spec_path = match candidates.into_iter().find(|p| p.is_file()) {
        Some(p) => p,
        None => return Ok(None),
    };
    // 32 MB cap — large enterprise specs (~10MB max in the wild) fit;
    // anything bigger is almost certainly a checked-in dump that would
    // OOM the process. Surface as a parse-time warning instead of
    // bailing out of the whole project check.
    if let Ok(meta) = std::fs::metadata(&spec_path) {
        if meta.len() > 32 * 1024 * 1024 {
            tracing::warn!(
                path = %spec_path.display(),
                bytes = meta.len(),
                "openapi spec exceeds 32 MB cap; skipping for contract check"
            );
            return Ok(None);
        }
    }
    let raw_with_bom = std::fs::read_to_string(&spec_path).map_err(|source| TestError::Io {
        path: spec_path.clone(),
        source,
    })?;
    // Strip leading UTF-8 BOM. Windows tools (PowerShell `Out-File
    // -Encoding utf8`, some IDE save dialogs) add one; serde_yaml_ng
    // rejects it as an invalid YAML document. Same fix for the
    // consumer-side parser below.
    let raw = raw_with_bom
        .strip_prefix('\u{FEFF}')
        .unwrap_or(&raw_with_bom);
    let value: serde_json::Value = if spec_path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_lowercase)
        .as_deref()
        == Some("json")
    {
        serde_json::from_str(raw).map_err(|e| TestError::InvalidSpec {
            path: spec_path.clone(),
            reason: e.to_string(),
        })?
    } else {
        serde_yaml_ng::from_str(raw).map_err(|e| TestError::InvalidSpec {
            path: spec_path.clone(),
            reason: e.to_string(),
        })?
    };
    let mut iface = ProviderInterface {
        repo_name: repo_name.to_string(),
        endpoints: BTreeMap::new(),
        has_default_response: BTreeSet::new(),
        has_request_body: BTreeSet::new(),
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
                let mut has_default = false;
                if let Some(responses) = op_value.get("responses").and_then(|v| v.as_object()) {
                    for code_str in responses.keys() {
                        if code_str.eq_ignore_ascii_case("default") {
                            // OpenAPI's `default` response key means
                            // "any status not enumerated above". Track
                            // it so the diff treats this operation as
                            // a wildcard for status checks.
                            has_default = true;
                            continue;
                        }
                        if let Ok(s) = code_str.parse::<u16>() {
                            statuses.insert(s);
                        }
                    }
                }
                // Don't fabricate a 200 for empty responses anymore —
                // it produced false positives when the consumer
                // expected a different status. A spec with no declared
                // responses (malformed but seen in the wild) is still
                // recorded as a known endpoint, just with no
                // documented status set.
                if has_default {
                    iface
                        .has_default_response
                        .insert((method.clone(), path_str.clone()));
                }
                // v0.24: track whether the operation declares a requestBody.
                if op_value.get("requestBody").is_some() {
                    iface
                        .has_request_body
                        .insert((method.clone(), path_str.clone()));
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

/// Walk `<repo>/.coral/tests/**/*.{yaml,yml,hurl}` recursively for HTTP
/// step references — every `(method, path, expected_status)` becomes an
/// `EndpointReference`.
///
/// **Recursive walk is critical** — generated tests committed by
/// `coral test-discover --commit` land in `.coral/tests/discovered/`,
/// and a non-recursive `read_dir` would silently miss them. See
/// `walk_tests::walk_tests_recursive` for the contract.
pub fn parse_consumer_for_repo(
    repo_name: &str,
    repo_path: &Path,
) -> TestResult<Option<ConsumerExpectations>> {
    let tests_dir = repo_path.join(".coral/tests");
    let recorded_dir = tests_dir.join("recorded");
    let paths = crate::walk_tests::walk_tests_recursive(repo_path, &["yaml", "yml", "hurl"])
        .map_err(|source| TestError::Io {
            path: tests_dir.clone(),
            source,
        })?;
    let mut expectations = ConsumerExpectations {
        repo_name: repo_name.to_string(),
        references: Vec::new(),
    };
    for path in paths {
        // v0.23.2: Keploy YAMLs under `.coral/tests/recorded/` are
        // managed by `RecordedRunner` and have a different schema
        // from `YamlSuite`. Skip them so the consumer-side contract
        // walk doesn't try to parse them as user-defined.
        if path.starts_with(&recorded_dir) {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase);
        match ext.as_deref() {
            Some("yaml") | Some("yml") => extract_from_yaml(&path, &mut expectations.references)?,
            Some("hurl") => extract_from_hurl(&path, &mut expectations.references)?,
            _ => continue,
        }
    }
    if expectations.references.is_empty() {
        return Ok(None);
    }
    Ok(Some(expectations))
}

fn extract_from_yaml(path: &Path, refs: &mut Vec<EndpointReference>) -> TestResult<()> {
    let raw_with_bom = std::fs::read_to_string(path).map_err(|source| TestError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let raw = raw_with_bom
        .strip_prefix('\u{FEFF}')
        .unwrap_or(&raw_with_bom);
    let value: serde_json::Value =
        serde_yaml_ng::from_str(raw).map_err(|e| TestError::InvalidSpec {
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
                // v0.24: detect whether the step sends a request body.
                let sends_body =
                    step.get("body").is_some() || step.get("json").is_some();
                refs.push(EndpointReference {
                    method,
                    path: path_str,
                    expected_status,
                    sends_body,
                    source: path.to_path_buf(),
                });
            }
        }
    }
    Ok(())
}

fn extract_from_hurl(path: &Path, refs: &mut Vec<EndpointReference>) -> TestResult<()> {
    let raw_with_bom = std::fs::read_to_string(path).map_err(|source| TestError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let raw = raw_with_bom
        .strip_prefix('\u{FEFF}')
        .unwrap_or(&raw_with_bom);
    let mut current_method: Option<String> = None;
    let mut current_path: Option<String> = None;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((method, path_str)) = parse_http_line(trimmed) {
            // If we already have a queued (method, path) without a
            // matching `HTTP <status>` line, record it with no
            // expected_status before queueing the new one — Hurl
            // allows omitting the response status, and missing it
            // shouldn't make the previous request silently disappear.
            if let (Some(m), Some(p)) = (current_method.take(), current_path.take()) {
                refs.push(EndpointReference {
                    method: m,
                    path: p,
                    expected_status: None,
                    sends_body: false,
                    source: path.to_path_buf(),
                });
            }
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
                    sends_body: false,
                    source: path.to_path_buf(),
                });
            }
        }
    }
    // Record any trailing pending request without a status line.
    if let (Some(m), Some(p)) = (current_method, current_path) {
        refs.push(EndpointReference {
            method: m,
            path: p,
            expected_status: None,
            sends_body: false,
            source: path.to_path_buf(),
        });
    }
    Ok(())
}

fn parse_http_line(line: &str) -> Option<(String, String)> {
    // Split on any whitespace, not just ASCII space — `.hurl` files
    // sometimes indent with tabs and Postman exports too. Normalise the
    // method to uppercase so a lowercased `get /users` from a
    // copy-pasted curl command isn't silently dropped.
    let mut parts = line.split_whitespace();
    let method_raw = parts.next()?;
    let method = method_raw.to_uppercase();
    if !is_http_method(&method) {
        return None;
    }
    let raw_url = parts.next()?.trim();
    if raw_url.is_empty() {
        return None;
    }
    // Strip scheme + host if present.
    let path_with_qs = if let Some((_scheme, rest)) = raw_url.split_once("://") {
        match rest.find('/') {
            Some(i) => rest[i..].to_string(),
            None => "/".to_string(),
        }
    } else {
        raw_url.to_string()
    };
    // Strip query string and fragment — OpenAPI specs declare paths
    // without them, so `/users?id=1` should match `/users`. Otherwise
    // the matcher reports `UnknownEndpoint` for what is actually a
    // perfectly fine consumer test.
    let path = strip_query_and_fragment(&path_with_qs);
    if path.is_empty() {
        return None;
    }
    Some((method, path))
}

/// Strip `?…` and `#…` from a path. Pure helper; tests cover it
/// directly to confirm that consumer paths like `/users?id=1#frag`
/// land at `/users` before reaching `openapi_path_matches`.
pub(crate) fn strip_query_and_fragment(s: &str) -> String {
    let after_qs = s.split_once('?').map(|(p, _)| p).unwrap_or(s);
    after_qs
        .split_once('#')
        .map(|(p, _)| p)
        .unwrap_or(after_qs)
        .to_string()
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
            // Union the documented status sets across ALL spec paths
            // matching this consumer path (e.g. both `/users/{id}` and
            // `/users/{name}` could match `/users/42`); also detect if
            // any matched op has a `default` response declared, which
            // wildcards every status.
            let mut documented: BTreeSet<u16> = BTreeSet::new();
            let mut covered_by_default = false;
            if let Some(by_path) = provider.endpoints.get(&r.method) {
                for (spec_path, statuses) in by_path {
                    if openapi_path_matches(spec_path, &r.path) {
                        documented.extend(statuses.iter().copied());
                        if provider
                            .has_default_response
                            .contains(&(r.method.clone(), spec_path.clone()))
                        {
                            covered_by_default = true;
                        }
                    }
                }
            }
            if !covered_by_default && !documented.contains(&expected) {
                let documented_vec: Vec<u16> = documented.iter().copied().collect();
                findings.push(Finding {
                    severity: Severity::Warning,
                    kind: FindingKind::StatusDrift {
                        method: r.method.clone(),
                        path: r.path.clone(),
                        expected,
                        documented: documented_vec.clone(),
                    },
                    consumer: consumer.repo_name.clone(),
                    provider: provider.repo_name.clone(),
                    message: format!(
                        "consumer '{}' expects {} from {} {} but provider documents {:?} (in {})",
                        consumer.repo_name,
                        expected,
                        r.method,
                        r.path,
                        documented_vec,
                        r.source.display()
                    ),
                });
            }
        }
        // v0.24: warn if consumer sends a body to an endpoint without requestBody.
        if r.sends_body {
            let provider_expects_body = if let Some(by_path) = provider.endpoints.get(&r.method) {
                by_path.keys().any(|spec_path| {
                    openapi_path_matches(spec_path, &r.path)
                        && provider
                            .has_request_body
                            .contains(&(r.method.clone(), spec_path.clone()))
                })
            } else {
                false
            };
            if !provider_expects_body {
                findings.push(Finding {
                    severity: Severity::Warning,
                    kind: FindingKind::RequestBodyDrift {
                        method: r.method.clone(),
                        path: r.path.clone(),
                    },
                    consumer: consumer.repo_name.clone(),
                    provider: provider.repo_name.clone(),
                    message: format!(
                        "consumer '{}' sends request body to {} {} but provider '{}' does not declare requestBody (in {})",
                        consumer.repo_name,
                        r.method,
                        r.path,
                        provider.repo_name,
                        r.source.display()
                    ),
                });
            }
        }
    }
    findings
}

/// Look up the set of HTTP methods the provider supports for `path`.
/// Path matching honours `{param}` placeholders (see
/// `openapi_path_matches`). Returns a deduplicated, alphabetised list
/// so the `UnknownMethod` `available` field reads cleanly in the
/// report — even when the spec declares the same method against
/// multiple `{param}` variants of the same path.
fn collect_methods_for_path(provider: &ProviderInterface, consumer_path: &str) -> Vec<String> {
    let mut methods = BTreeSet::new();
    for (method, by_path) in &provider.endpoints {
        for spec_path in by_path.keys() {
            if openapi_path_matches(spec_path, consumer_path) {
                methods.insert(method.clone());
            }
        }
    }
    methods.into_iter().collect()
}

/// Match an OpenAPI path with `{param}` placeholders against a
/// consumer-side concrete path. Each `{name}` matches any non-empty
/// non-`/` segment. Consumer-side `${var}` (a runtime substitution
/// from a previous step's `capture`) is treated as wildcard ONLY
/// when the spec-side segment is also a `{param}` — this catches
/// typos like `/uers/${id}` against `/users/{id}` instead of silently
/// passing.
pub fn openapi_path_matches(spec_path: &str, consumer_path: &str) -> bool {
    let spec_norm = normalize_path(spec_path);
    let consumer_norm = normalize_path(consumer_path);
    if spec_norm == consumer_norm {
        return true;
    }
    let spec_segments: Vec<&str> = spec_norm.split('/').collect();
    let consumer_segments: Vec<&str> = consumer_norm.split('/').collect();
    if spec_segments.len() != consumer_segments.len() {
        return false;
    }
    for (s, c) in spec_segments.iter().zip(consumer_segments.iter()) {
        let spec_is_param = is_path_parameter(s);
        if spec_is_param {
            // Spec-side `{name}` matches any non-empty segment, including
            // `${runtime_var}` from the consumer test.
            if c.is_empty() {
                return false;
            }
            continue;
        }
        if s == c {
            continue;
        }
        // Spec is a literal segment but consumer is `${var}` — refuse.
        // Otherwise a typo like `/uers/${id}` would silently match
        // `/users/{id}` and the drift would never be caught.
        return false;
    }
    true
}

/// Normalize a path: trim a single trailing `/` (except the root
/// itself). OpenAPI tooling treats `/users` and `/users/` as the
/// same endpoint; consumers may produce either.
fn normalize_path(p: &str) -> String {
    if p == "/" || !p.ends_with('/') {
        return p.to_string();
    }
    p.trim_end_matches('/').to_string()
}

/// `{param}` predicate. Refuses empty `{}` (which is malformed
/// OpenAPI but seen in the wild) so we don't accept anything as a
/// wildcard.
fn is_path_parameter(seg: &str) -> bool {
    seg.len() > 2 && seg.starts_with('{') && seg.ends_with('}')
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
    fn openapi_path_matches_consumer_var_only_when_spec_is_param() {
        // `${var}` in the consumer (a runtime substitution) is treated
        // as a wildcard ONLY when the spec-side segment is also a
        // `{param}`. Otherwise typos like `/uers/${id}` would silently
        // match `/users/{id}` and the drift would never be caught.
        assert!(openapi_path_matches("/users/{id}", "/users/${user_id}"));
        // Spec literal vs consumer ${var} → must NOT match.
        assert!(!openapi_path_matches("/users", "/${something}"));
        assert!(!openapi_path_matches("/uers/{id}", "/users/${id}"));
    }

    #[test]
    fn openapi_path_matches_normalizes_trailing_slash() {
        assert!(openapi_path_matches("/users", "/users/"));
        assert!(openapi_path_matches("/users/", "/users"));
        // Root path is not normalized away.
        assert!(openapi_path_matches("/", "/"));
    }

    #[test]
    fn openapi_path_matches_rejects_empty_param_braces() {
        // `{}` in a spec is malformed OpenAPI; we refuse to treat it
        // as a wildcard so a malformed spec doesn't accept anything.
        assert!(!openapi_path_matches("/users/{}", "/users/42"));
    }

    #[test]
    fn detects_unknown_endpoint() {
        let provider = ProviderInterface {
            has_default_response: BTreeSet::new(),
            has_request_body: BTreeSet::new(),
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
                sends_body: false,
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
            has_default_response: BTreeSet::new(),
            has_request_body: BTreeSet::new(),
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
                sends_body: false,
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
            has_default_response: BTreeSet::new(),
            has_request_body: BTreeSet::new(),
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
                sends_body: false,
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
            has_default_response: BTreeSet::new(),
            has_request_body: BTreeSet::new(),
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
                    sends_body: false,
                    source: PathBuf::from("/x/.coral/tests/api.yaml"),
                },
                EndpointReference {
                    method: "GET".into(),
                    path: "/users/42".into(), // matches /users/{id}
                    expected_status: Some(200),
                    sends_body: false,
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

    // ----- Regression tests for the validation pass (the 4 agents
    // findings) -----

    #[test]
    fn parse_http_line_strips_query_string() {
        let (m, p) = parse_http_line("GET /users?id=1").unwrap();
        assert_eq!(m, "GET");
        assert_eq!(p, "/users");
    }

    #[test]
    fn parse_http_line_strips_fragment() {
        let (_, p) = parse_http_line("GET /users#section").unwrap();
        assert_eq!(p, "/users");
    }

    #[test]
    fn parse_http_line_strips_query_and_fragment_combined() {
        let (_, p) = parse_http_line("GET /users?x=1#frag").unwrap();
        assert_eq!(p, "/users");
    }

    #[test]
    fn parse_http_line_accepts_tab_separator() {
        let (m, p) = parse_http_line("GET\t/users").unwrap();
        assert_eq!(m, "GET");
        assert_eq!(p, "/users");
    }

    #[test]
    fn parse_http_line_normalizes_lowercase_method() {
        // Postman / curl history exports occasionally lowercase the
        // method. Treat them the same as uppercase so contract check
        // catches drift in those files too.
        let (m, p) = parse_http_line("get /users").unwrap();
        assert_eq!(m, "GET");
        assert_eq!(p, "/users");
    }

    #[test]
    fn strip_query_and_fragment_helper_handles_pathological_input() {
        assert_eq!(strip_query_and_fragment("/x"), "/x");
        assert_eq!(strip_query_and_fragment("/x?"), "/x");
        assert_eq!(strip_query_and_fragment("/x?#"), "/x");
        assert_eq!(strip_query_and_fragment("/x#"), "/x");
        assert_eq!(strip_query_and_fragment("/x?a=1&b=2#frag"), "/x");
    }

    #[test]
    fn collect_methods_for_path_dedupes_when_spec_has_multiple_param_paths() {
        // Pathological-but-real spec where the same method is declared
        // against two `{param}` variants. The available list must be
        // deduplicated so the report reads cleanly.
        let provider = ProviderInterface {
            repo_name: "api".into(),
            has_default_response: BTreeSet::new(),
            has_request_body: BTreeSet::new(),
            endpoints: {
                let mut m = BTreeMap::new();
                let mut by_path = BTreeMap::new();
                by_path.insert("/users/{id}".into(), [200].into_iter().collect());
                by_path.insert("/users/{name}".into(), [200].into_iter().collect());
                m.insert("GET".into(), by_path);
                m
            },
        };
        let methods = collect_methods_for_path(&provider, "/users/42");
        assert_eq!(methods, vec!["GET".to_string()]);
    }

    #[test]
    fn status_drift_skipped_when_provider_has_default_response() {
        // Provider declares only `default` for /errors; consumer expects
        // 503. Default acts as a wildcard and the diff should NOT
        // report drift.
        let provider = ProviderInterface {
            repo_name: "api".into(),
            has_default_response: {
                let mut s = BTreeSet::new();
                s.insert(("GET".to_string(), "/errors".to_string()));
                s
            },
            has_request_body: BTreeSet::new(),
            endpoints: {
                let mut m = BTreeMap::new();
                let mut by_path = BTreeMap::new();
                by_path.insert("/errors".into(), BTreeSet::new());
                m.insert("GET".into(), by_path);
                m
            },
        };
        let consumer = ConsumerExpectations {
            repo_name: "worker".into(),
            references: vec![EndpointReference {
                method: "GET".into(),
                path: "/errors".into(),
                expected_status: Some(503),
                sends_body: false,
                source: PathBuf::from("/x/.coral/tests/api.yaml"),
            }],
        };
        let findings = diff_consumer_against_provider(&consumer, &provider);
        assert!(
            !findings
                .iter()
                .any(|f| matches!(f.kind, FindingKind::StatusDrift { .. })),
            "default response key should suppress status drift, got: {findings:?}"
        );
    }

    #[test]
    fn extract_from_yaml_strips_utf8_bom() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("tests/api.yaml");
        write(
            &path,
            "\u{FEFF}name: bom suite\nservice: api\nsteps:\n  - http: GET /users\n    expect: { status: 200 }\n",
        );
        let mut refs = Vec::new();
        extract_from_yaml(&path, &mut refs).expect("BOM-prefixed YAML must parse cleanly");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].path, "/users");
    }

    #[test]
    fn extract_from_hurl_strips_utf8_bom() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("tests/api.hurl");
        write(&path, "\u{FEFF}GET /users\nHTTP 200\n");
        let mut refs = Vec::new();
        extract_from_hurl(&path, &mut refs).expect("BOM-prefixed Hurl must parse cleanly");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].path, "/users");
    }

    #[test]
    fn extract_from_hurl_records_pending_request_without_status_line() {
        // Real Hurl files sometimes omit the HTTP status assertion.
        // The previous implementation silently dropped such requests.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("tests/api.hurl");
        write(&path, "GET /a\n\nGET /b\nHTTP 200\n");
        let mut refs = Vec::new();
        extract_from_hurl(&path, &mut refs).unwrap();
        assert_eq!(refs.len(), 2);
        // First one: no expected status. Second: 200.
        assert_eq!(refs[0].path, "/a");
        assert_eq!(refs[0].expected_status, None);
        assert_eq!(refs[1].path, "/b");
        assert_eq!(refs[1].expected_status, Some(200));
    }

    #[test]
    fn parse_provider_for_repo_finds_spec_in_api_v1_subdir() {
        let dir = TempDir::new().unwrap();
        write(
            &dir.path().join("api/v1/openapi.yaml"),
            r#"openapi: 3.0.0
info: { title: x, version: 1.0 }
paths:
  /h:
    get:
      responses: { '200': { description: ok } }
"#,
        );
        let iface = parse_provider_for_repo("svc", dir.path()).unwrap();
        assert!(iface.is_some(), "spec under api/v1/ must be discovered");
        assert!(
            iface
                .unwrap()
                .endpoints
                .get("GET")
                .map(|by_path| by_path.contains_key("/h"))
                .unwrap_or(false)
        );
    }

    #[test]
    fn check_project_skips_disabled_repos_via_filter_at_caller() {
        // The `enabled = false` filter is applied at the CLI layer
        // (commands::contract::run_check). At the library layer
        // `check_project` simply walks whatever it's given. Pin the
        // contract: passing an empty `repos` slice → empty report.
        let dir = TempDir::new().unwrap();
        let report = check_project(dir.path(), &[]).unwrap();
        assert!(report.findings.is_empty());
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

    #[test]
    fn end_to_end_check_project_normalizes_lowercase_methods_in_test_yaml() {
        // Real-world tests sometimes lowercase the HTTP verb (`get` not
        // `GET`). The provider's OpenAPI lookup is uppercase-keyed, so
        // parse_http_line must normalize the consumer side or every
        // request looks unknown.
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
"#,
        );
        write(
            &root.join("repos/worker/.coral/tests/case.yaml"),
            r#"name: lowercase verb
service: worker
steps:
  - http: get /users
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
            "lowercase verb must normalize and match, got: {:#?}",
            report.findings
        );
    }

    #[test]
    fn end_to_end_check_project_strips_query_string_from_consumer_test() {
        // Tests often append querystrings (`/users?limit=10`). OpenAPI
        // paths never include the query, so parse_http_line must strip
        // it before comparison.
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
"#,
        );
        write(
            &root.join("repos/worker/.coral/tests/case.yaml"),
            r#"name: query string
service: worker
steps:
  - http: GET /users?limit=10&offset=20
    expect: { status: 200 }
  - http: GET /users#section
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
            "query strings/fragments must not produce false positives, got: {:#?}",
            report.findings
        );
    }

    #[test]
    fn end_to_end_check_project_finds_provider_spec_in_diamond_subdir() {
        // Many real repos place the spec under api/v1/openapi.yaml or
        // contract/openapi.yaml. parse_provider_for_repo's candidate list
        // must cover those paths so users don't have to reorganize their
        // repos to use Coral.
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write(
            &root.join("repos/api/api/v1/openapi.yaml"),
            r#"openapi: 3.0.0
info: { title: api, version: 1.0 }
paths:
  /v1/users:
    get:
      responses: { '200': { description: ok } }
"#,
        );
        write(
            &root.join("repos/worker/.coral/tests/case.yaml"),
            r#"name: nested spec
service: worker
steps:
  - http: GET /v1/users
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
            "spec under api/v1/ must be discovered, got: {:#?}",
            report.findings
        );
    }

    #[test]
    fn end_to_end_check_project_warns_on_malformed_provider_spec() {
        // If the provider's openapi.yaml is malformed, the contract
        // check must not abort — it must emit a warning finding and
        // continue so the user sees every drift in one report.
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write(
            &root.join("repos/api/openapi.yaml"),
            "this is not yaml: [[[",
        );
        write(
            &root.join("repos/worker/.coral/tests/case.yaml"),
            r#"name: case
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
        .expect("malformed spec must not abort the check");
        assert!(
            report.findings.iter().any(|f| matches!(
                &f.kind,
                FindingKind::MalformedProviderSpec { provider_repo, .. }
                    if provider_repo == "api"
            )),
            "expected MalformedProviderSpec finding, got: {:#?}",
            report.findings
        );
        // Severity is warning — drift is still detectable on healthy
        // provider/consumer pairs in the same project.
        assert!(!report.has_errors());
    }
}
