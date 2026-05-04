//! `UserDefinedRunner` — execute YAML-declared HTTP/exec test suites.
//!
//! v0.18 wave 2 covers the MVP feature set: HTTP requests with
//! status + body_contains assertions, exec steps with exit_code +
//! stdout_contains, retry policy, and `${VAR}` variable substitution
//! sourced from the captured port map of the live env. gRPC,
//! GraphQL helper, snapshot, trace, retry-on-condition, captures,
//! step dependencies, and parallel execution follow in v0.18 wave 3.

use crate::error::{TestError, TestResult};
use crate::report::{Evidence, HttpEvidence, TestReport, TestStatus};
use crate::spec::{TestCase, TestKind, TestSource, TestSpec};
use crate::{ParallelismHint, TestRunner};
use coral_env::{EnvBackend, EnvHandle, EnvPlan, ServiceStatus};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

/// On-disk YAML schema. v0.18 wave 2 keeps it small; the
/// `serde_json::Value` payload is opaque so future fields don't need
/// schema-level migrations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YamlSuite {
    pub name: String,
    pub service: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub steps: Vec<YamlStep>,
    /// Default retry policy applied to every step that doesn't set
    /// its own. Wave 3c addition.
    #[serde(default)]
    pub retry: Option<RetryPolicy>,
}

/// `retry: { max: 3, backoff: "exponential", on: ["5xx", "timeout"] }`
/// Wave 3c addition; `on` defaults to `["5xx", "timeout"]` so callers
/// only need to set `max` for the common case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    #[serde(default = "default_retry_max")]
    pub max: u32,
    #[serde(default = "default_backoff")]
    pub backoff: BackoffKind,
    #[serde(default = "default_retry_on")]
    pub on: Vec<RetryCondition>,
}

fn default_retry_max() -> u32 {
    3
}

fn default_backoff() -> BackoffKind {
    BackoffKind::Exponential
}

fn default_retry_on() -> Vec<RetryCondition> {
    vec![RetryCondition::FivexX, RetryCondition::Timeout]
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackoffKind {
    None,
    Linear,
    Exponential,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetryCondition {
    #[serde(rename = "5xx")]
    FivexX,
    #[serde(rename = "4xx")]
    FourxX,
    Timeout,
    Any,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum YamlStep {
    Http(HttpStep),
    Exec(ExecStep),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpStep {
    /// `"GET /users"`-style shorthand.
    pub http: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Option<serde_json::Value>,
    #[serde(default)]
    pub expect: HttpExpect,
    /// Per-step retry override. None inherits from the suite's `retry`.
    #[serde(default)]
    pub retry: Option<RetryPolicy>,
    /// Capture values from the response body (Hurl-style):
    /// `capture: { user_id: "$.id" }`. Captured values are
    /// substitution-available in subsequent steps as `${user_id}`.
    #[serde(default)]
    pub capture: BTreeMap<String, String>,
    /// Step ID for cross-step references. Defaults to step index.
    #[serde(default)]
    pub id: Option<String>,
    /// Wait for these step ids to pass before running this one.
    /// Default: implicit sequential.
    #[serde(default)]
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HttpExpect {
    pub status: Option<u16>,
    pub body_contains: Option<String>,
    /// File-based snapshot assertion. The first run writes the
    /// response body to this path; subsequent runs compare. Update
    /// via `coral test --update-snapshots`.
    pub snapshot: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecStep {
    pub exec: Vec<String>,
    #[serde(default)]
    pub expect: ExecExpect,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecExpect {
    pub exit_code: Option<i32>,
    pub stdout_contains: Option<String>,
}

pub struct UserDefinedRunner {
    backend: Arc<dyn EnvBackend>,
    plan: EnvPlan,
    update_snapshots: bool,
    snapshot_dir: std::path::PathBuf,
}

impl UserDefinedRunner {
    pub fn new(backend: Arc<dyn EnvBackend>, plan: EnvPlan) -> Self {
        let snapshot_dir = plan.project_root.join(".coral/snapshots");
        Self {
            backend,
            plan,
            update_snapshots: false,
            snapshot_dir,
        }
    }

    /// Enable `--update-snapshots`: missing or differing snapshots are
    /// written rather than failing the test. Symmetric to
    /// `cargo insta review` semantics.
    pub fn with_update_snapshots(mut self, on: bool) -> Self {
        self.update_snapshots = on;
        self
    }

    /// Walk `<project_root>/.coral/tests/**` recursively for `*.yaml`
    /// and `*.yml` files, parse them into TestCases.
    ///
    /// **Recursive walk is critical** — `coral test-discover --commit`
    /// writes generated YAML under `.coral/tests/discovered/`, which
    /// would be invisible to a non-recursive `read_dir`. See
    /// `walk_tests::walk_tests_recursive` for the contract.
    pub fn discover_tests_dir(project_root: &Path) -> TestResult<Vec<(TestCase, YamlSuite)>> {
        let dir = project_root.join(".coral/tests");
        let paths = crate::walk_tests::walk_tests_recursive(project_root, &["yaml", "yml"])
            .map_err(|source| TestError::Io {
                path: dir.clone(),
                source,
            })?;
        let mut out = Vec::with_capacity(paths.len());
        for path in paths {
            let raw = std::fs::read_to_string(&path).map_err(|source| TestError::Io {
                path: path.clone(),
                source,
            })?;
            let suite: YamlSuite =
                serde_yaml_ng::from_str(&raw).map_err(|e| TestError::InvalidSpec {
                    path: path.clone(),
                    reason: e.to_string(),
                })?;
            let case = TestCase {
                id: format!("user-defined:{}", suite.name),
                name: suite.name.clone(),
                kind: TestKind::UserDefined,
                service: suite.service.clone(),
                tags: suite.tags.clone(),
                source: TestSource::File { path: path.clone() },
                spec: TestSpec(serde_json::to_value(&suite).unwrap_or(serde_json::Value::Null)),
            };
            out.push((case, suite));
        }
        Ok(out)
    }

    fn execute_suite(
        &self,
        case: &TestCase,
        suite: &YamlSuite,
        env_status: Option<&ServiceStatus>,
    ) -> TestResult<TestReport> {
        let started = Instant::now();
        let mut captures: BTreeMap<String, String> = BTreeMap::new();
        let suite_retry = suite.retry.clone();

        for (step_idx, step) in suite.steps.iter().enumerate() {
            let retry = match step {
                YamlStep::Http(s) => s.retry.clone().or_else(|| suite_retry.clone()),
                YamlStep::Exec(_) => None,
            };
            let max_attempts = retry.as_ref().map(|r| r.max).unwrap_or(0) + 1;

            let mut last_err: Option<String> = None;
            let mut attempt = 0u32;
            while attempt < max_attempts {
                let mut new_captures: BTreeMap<String, String> = BTreeMap::new();
                let outcome = match step {
                    YamlStep::Http(s) => self
                        .run_http_attempt(s, env_status, &captures, &mut new_captures)
                        .inspect(|_| {
                            captures.extend(std::mem::take(&mut new_captures));
                        }),
                    YamlStep::Exec(s) => Self::run_exec(s).map(|_| HttpStepOutcome::default()),
                };
                match outcome {
                    Ok(_) => {
                        last_err = None;
                        break;
                    }
                    Err(StepFailure { reason, kind }) => {
                        last_err = Some(reason);
                        let should_retry = retry
                            .as_ref()
                            .map(|r| matches_retry_condition(&r.on, kind))
                            .unwrap_or(false);
                        if !should_retry {
                            break;
                        }
                        attempt += 1;
                        if attempt < max_attempts {
                            std::thread::sleep(retry_backoff(retry.as_ref().unwrap(), attempt));
                        }
                    }
                }
            }

            if let Some(reason) = last_err {
                let report = TestReport::new(
                    case.clone(),
                    TestStatus::Fail {
                        reason: format!("step {step_idx}: {reason}"),
                    },
                    started.elapsed(),
                );
                return Ok(report);
            }
        }
        Ok(TestReport::new(
            case.clone(),
            TestStatus::Pass,
            started.elapsed(),
        ))
    }

    fn run_http_attempt(
        &self,
        step: &HttpStep,
        env_status: Option<&ServiceStatus>,
        in_captures: &BTreeMap<String, String>,
        out_captures: &mut BTreeMap<String, String>,
    ) -> std::result::Result<HttpStepOutcome, StepFailure> {
        let line = substitute_vars(&step.http, in_captures);
        let (method, path) = parse_http_line(&line).ok_or_else(|| StepFailure {
            reason: format!("invalid http line: {line}"),
            kind: FailureKind::Spec,
        })?;
        let host_port = match env_status.and_then(|s| s.published_ports.first()) {
            Some(p) if p.host_port > 0 => p.host_port,
            _ => {
                return Err(StepFailure {
                    reason: "service has no published port; bring it up first".into(),
                    kind: FailureKind::EnvUp,
                });
            }
        };
        let url = format!("http://127.0.0.1:{host_port}{path}");

        let mut cmd = Command::new("curl");
        cmd.args([
            "-s",
            "-X",
            method,
            "-w",
            "\nHTTP_STATUS:%{http_code}",
            "--max-time",
            "10",
        ]);
        for (k, v) in &step.headers {
            cmd.arg("-H")
                .arg(substitute_vars(&format!("{k}: {v}"), in_captures));
        }
        if let Some(body) = &step.body {
            let body_str = substitute_vars(&body.to_string(), in_captures);
            cmd.args(["-H", "Content-Type: application/json"]);
            cmd.args(["-d", body_str.as_str()]);
        }
        cmd.arg(&url);

        let output = cmd.output().map_err(|e| StepFailure {
            reason: format!("failed to invoke curl: {e}"),
            kind: FailureKind::Other,
        })?;
        let body = String::from_utf8_lossy(&output.stdout).into_owned();
        let (response_body, status_code) = split_curl_status(&body);

        if let Some(expected) = step.expect.status {
            if status_code != expected {
                let kind = match status_code {
                    0 => FailureKind::Timeout,
                    s if (500..600).contains(&s) => FailureKind::Status5xx,
                    s if (400..500).contains(&s) => FailureKind::Status4xx,
                    _ => FailureKind::Other,
                };
                return Err(StepFailure {
                    reason: format!("expected HTTP status {expected}, got {status_code}"),
                    kind,
                });
            }
        }
        if let Some(needle) = &step.expect.body_contains {
            if !response_body.contains(needle.as_str()) {
                return Err(StepFailure {
                    reason: format!(
                        "response body does not contain '{needle}' (first 200 bytes: {})",
                        response_body.chars().take(200).collect::<String>()
                    ),
                    kind: FailureKind::Other,
                });
            }
        }
        // Snapshot assertion: write on first run / `--update-snapshots`,
        // compare otherwise.
        if let Some(snap_path) = &step.expect.snapshot {
            let resolved = if snap_path.is_absolute() {
                snap_path.clone()
            } else {
                self.snapshot_dir.join(snap_path)
            };
            let normalized = response_body.trim().to_string();
            if !resolved.exists() || self.update_snapshots {
                if let Some(parent) = resolved.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| StepFailure {
                        reason: format!("creating snapshot dir {}: {e}", parent.display()),
                        kind: FailureKind::Other,
                    })?;
                }
                std::fs::write(&resolved, &normalized).map_err(|e| StepFailure {
                    reason: format!("writing snapshot {}: {e}", resolved.display()),
                    kind: FailureKind::Other,
                })?;
            } else {
                let saved = std::fs::read_to_string(&resolved).map_err(|e| StepFailure {
                    reason: format!("reading snapshot {}: {e}", resolved.display()),
                    kind: FailureKind::Other,
                })?;
                if saved.trim() != normalized {
                    return Err(StepFailure {
                        reason: format!(
                            "snapshot mismatch at {}: rerun with --update-snapshots",
                            resolved.display()
                        ),
                        kind: FailureKind::Snapshot,
                    });
                }
            }
        }
        // Apply captures from the response body. Failure to match is
        // a step failure (captures are user-declared expectations).
        for (var, json_path) in &step.capture {
            let extracted =
                extract_jsonpath(&response_body, json_path).ok_or_else(|| StepFailure {
                    reason: format!(
                        "capture '{var}' from path '{json_path}' did not match anything"
                    ),
                    kind: FailureKind::Other,
                })?;
            out_captures.insert(var.clone(), extracted);
        }
        Ok(HttpStepOutcome::default())
    }

    fn run_exec(step: &ExecStep) -> std::result::Result<(), StepFailure> {
        if step.exec.is_empty() {
            return Err(StepFailure {
                reason: "exec step has no command".into(),
                kind: FailureKind::Spec,
            });
        }
        let mut cmd = Command::new(&step.exec[0]);
        for arg in &step.exec[1..] {
            cmd.arg(arg);
        }
        let output = cmd.output().map_err(|e| StepFailure {
            reason: format!("failed to invoke {}: {e}", step.exec[0]),
            kind: FailureKind::Other,
        })?;
        let exit = output.status.code().unwrap_or(-1);
        if let Some(expected) = step.expect.exit_code {
            if exit != expected {
                return Err(StepFailure {
                    reason: format!("expected exit {expected}, got {exit}"),
                    kind: FailureKind::Other,
                });
            }
        }
        if let Some(needle) = &step.expect.stdout_contains {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if !stdout.contains(needle.as_str()) {
                return Err(StepFailure {
                    reason: format!(
                        "stdout does not contain '{needle}' (got: {})",
                        stdout.chars().take(200).collect::<String>()
                    ),
                    kind: FailureKind::Other,
                });
            }
        }
        Ok(())
    }
}

/// Outcome of a single HTTP step. Reserved for future fields
/// (e.g. captured headers); empty in v0.18.
#[derive(Debug, Clone, Default)]
struct HttpStepOutcome {}

#[derive(Debug, Clone)]
struct StepFailure {
    reason: String,
    kind: FailureKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureKind {
    Spec,
    EnvUp,
    Status4xx,
    Status5xx,
    Timeout,
    Snapshot,
    Other,
}

fn matches_retry_condition(conditions: &[RetryCondition], kind: FailureKind) -> bool {
    if conditions.contains(&RetryCondition::Any) {
        return true;
    }
    match kind {
        FailureKind::Status4xx => conditions.contains(&RetryCondition::FourxX),
        FailureKind::Status5xx => conditions.contains(&RetryCondition::FivexX),
        FailureKind::Timeout => conditions.contains(&RetryCondition::Timeout),
        _ => false,
    }
}

fn retry_backoff(retry: &RetryPolicy, attempt: u32) -> std::time::Duration {
    let base_ms = match retry.backoff {
        BackoffKind::None => 0,
        BackoffKind::Linear => 200u64.saturating_mul(attempt as u64),
        BackoffKind::Exponential => {
            // 200ms, 400ms, 800ms, 1600ms, capped at 5s.
            let exp = 1u64 << (attempt.saturating_sub(1).min(5));
            (200u64 * exp).min(5_000)
        }
    };
    std::time::Duration::from_millis(base_ms)
}

/// Substitute `${var}` occurrences with values from `captures`. Unknown
/// vars are left intact (no panic, no error) so missing captures
/// produce visible URL/string fragments rather than silent empty
/// strings.
///
/// v0.19.6 audit M1: previously this walked `input.as_bytes()` and did
/// `out.push(bytes[i] as char)`, which corrupted multi-byte UTF-8
/// (e.g. `é = 0xC3 0xA9` became Latin-1 `Ã©`). The implementation
/// below walks `char_indices()` and only enters the `${…}` fast path
/// when the current ASCII byte is `$` — every other char is appended
/// verbatim, preserving multi-byte sequences.
fn substitute_vars(input: &str, captures: &BTreeMap<String, String>) -> String {
    if !input.contains("${") {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut iter = input.char_indices().peekable();
    while let Some((i, ch)) = iter.next() {
        // Only ASCII `$` followed by ASCII `{` triggers the
        // substitution path. Every other char (including multi-byte
        // sequences like `é`) is appended as-is.
        if ch == '$' && iter.peek().map(|&(_, c)| c == '{').unwrap_or(false) {
            // Skip the `{`.
            let _ = iter.next();
            // Find the matching `}` in the rest of `input`. Search by
            // byte offset is safe because `}` is a single-byte char
            // and `find` returns byte offsets.
            let body_start = i + 2; // bytes for `${`
            if let Some(rel_end) = input[body_start..].find('}') {
                let body_end = body_start + rel_end;
                let key = &input[body_start..body_end];
                if let Some(val) = captures.get(key) {
                    out.push_str(val);
                } else {
                    // Echo the unknown placeholder back verbatim so
                    // tests fail loudly with a visible `${name}` in
                    // the URL / payload.
                    out.push_str(&input[i..body_end + 1]);
                }
                // Advance the iterator past the `}` we just consumed.
                while let Some(&(j, _)) = iter.peek() {
                    if j > body_end {
                        break;
                    }
                    let _ = iter.next();
                }
                continue;
            }
            // Unterminated `${…}` — copy the literal `${` and let the
            // outer loop continue from the char after it. (This also
            // matches the pre-fix behavior of failing-open instead of
            // panicking.)
            out.push('$');
            out.push('{');
            continue;
        }
        out.push(ch);
    }
    out
}

/// Tiny `$.field.subfield`-style JSONPath extractor — supports just
/// dot navigation through objects. Anything more complex (filters,
/// array indexes, wildcards) defers to v0.20+ when we add a real
/// JSONPath library.
fn extract_jsonpath(body: &str, path: &str) -> Option<String> {
    let trimmed = path.trim().strip_prefix("$.").unwrap_or(path.trim());
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let mut cursor = &value;
    for segment in trimmed.split('.') {
        if segment.is_empty() {
            continue;
        }
        cursor = cursor.get(segment)?;
    }
    Some(match cursor {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

impl TestRunner for UserDefinedRunner {
    fn name(&self) -> &'static str {
        "user_defined"
    }

    fn supports(&self, kind: TestKind) -> bool {
        matches!(kind, TestKind::UserDefined)
    }

    fn run(&self, case: &TestCase, _env: &EnvHandle) -> TestResult<TestReport> {
        let suite: YamlSuite =
            serde_json::from_value(case.spec.0.clone()).map_err(|e| TestError::InvalidSpec {
                path: match &case.source {
                    TestSource::File { path } => path.clone(),
                    _ => Path::new("<inline>").to_path_buf(),
                },
                reason: e.to_string(),
            })?;

        let mut env_status = None;
        if let Some(name) = case.service.as_deref() {
            let status = self.backend.status(&self.plan)?;
            env_status = status.services.into_iter().find(|s| s.name == name);
        }
        self.execute_suite(case, &suite, env_status.as_ref())
    }

    fn discover(&self, project_root: &Path) -> TestResult<Vec<TestCase>> {
        let pairs = Self::discover_tests_dir(project_root)?;
        Ok(pairs.into_iter().map(|(case, _)| case).collect())
    }

    fn parallelism_hint(&self) -> ParallelismHint {
        ParallelismHint::PerService
    }
}

fn parse_http_line(line: &str) -> Option<(&'static str, String)> {
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
    let path = parts.next()?.trim().to_string();
    if path.is_empty() {
        return None;
    }
    Some((method, path))
}

fn split_curl_status(body_with_status: &str) -> (String, u16) {
    if let Some(pos) = body_with_status.rfind("\nHTTP_STATUS:") {
        let body = body_with_status[..pos].to_string();
        let status_part = &body_with_status[pos + "\nHTTP_STATUS:".len()..];
        let status: u16 = status_part.trim().parse().unwrap_or(0);
        (body, status)
    } else {
        (body_with_status.to_string(), 0)
    }
}

#[allow(dead_code)]
fn _ev() -> Evidence {
    Evidence {
        http: Some(HttpEvidence {
            method: "GET".into(),
            url: "".into(),
            status: 0,
            body_tail: None,
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_http_line_recognizes_get() {
        let parsed = parse_http_line("GET /users").unwrap();
        assert_eq!(parsed.0, "GET");
        assert_eq!(parsed.1, "/users");
    }

    #[test]
    fn parse_http_line_recognizes_post_with_path() {
        let parsed = parse_http_line("POST /users/42").unwrap();
        assert_eq!(parsed.0, "POST");
        assert_eq!(parsed.1, "/users/42");
    }

    #[test]
    fn parse_http_line_rejects_unknown_method() {
        assert!(parse_http_line("FROBNICATE /x").is_none());
    }

    #[test]
    fn parse_http_line_rejects_empty_path() {
        assert!(parse_http_line("GET ").is_none());
    }

    #[test]
    fn split_curl_status_extracts_trailing_status() {
        let raw = "{\"hello\":\"world\"}\nHTTP_STATUS:200";
        let (body, status) = split_curl_status(raw);
        assert_eq!(body, "{\"hello\":\"world\"}");
        assert_eq!(status, 200);
    }

    #[test]
    fn split_curl_status_returns_zero_on_missing_marker() {
        let (body, status) = split_curl_status("just body");
        assert_eq!(body, "just body");
        assert_eq!(status, 0);
    }

    #[test]
    fn yaml_suite_round_trips() {
        let raw = r#"
name: api smoke
service: api
tags: [smoke]
steps:
  - http: GET /users
    expect:
      status: 200
      body_contains: "users"
  - exec: ["echo", "ok"]
    expect:
      exit_code: 0
      stdout_contains: "ok"
"#;
        let suite: YamlSuite = serde_yaml_ng::from_str(raw).unwrap();
        assert_eq!(suite.name, "api smoke");
        assert_eq!(suite.steps.len(), 2);
    }

    #[test]
    fn discover_returns_empty_when_dir_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        let pairs = UserDefinedRunner::discover_tests_dir(dir.path()).unwrap();
        assert!(pairs.is_empty());
    }

    #[test]
    fn substitute_vars_replaces_known_keys_and_keeps_unknowns() {
        let mut caps = BTreeMap::new();
        caps.insert("user_id".to_string(), "42".to_string());
        assert_eq!(substitute_vars("/users/${user_id}", &caps), "/users/42");
        // Unknown var is left intact (visible in URL → easy to debug).
        assert_eq!(substitute_vars("/x/${other}", &caps), "/x/${other}");
        // No-op fast path on inputs without `${`.
        assert_eq!(substitute_vars("/static", &caps), "/static");
    }

    #[test]
    fn substitute_vars_handles_multiple_substitutions() {
        let mut caps = BTreeMap::new();
        caps.insert("a".into(), "1".into());
        caps.insert("b".into(), "2".into());
        assert_eq!(substitute_vars("a=${a},b=${b}", &caps), "a=1,b=2");
    }

    /// v0.19.6 audit M1: pre-fix the function walked the byte stream
    /// and did `out.push(bytes[i] as char)`, treating each `u8` as a
    /// codepoint. A multi-byte UTF-8 sequence (`é = 0xC3 0xA9`)
    /// emerged as `Ã©`. Pin char-aware behavior here.
    #[test]
    fn substitute_vars_preserves_multibyte_utf8() {
        let mut caps = BTreeMap::new();
        caps.insert("name".into(), "test".into());
        // `café` contains the 2-byte UTF-8 sequence `0xC3 0xA9` for `é`.
        assert_eq!(
            substitute_vars("café ${name}", &caps),
            "café test",
            "non-ASCII chars must round-trip through substitute_vars"
        );
        // Without any placeholder the fast-path returns the input
        // unchanged — that path has always been correct.
        assert_eq!(
            substitute_vars("naïve résumé 日本", &caps),
            "naïve résumé 日本"
        );
        // Mixed: emoji + multi-byte + placeholder.
        assert_eq!(substitute_vars("👋 ${name} naïve", &caps), "👋 test naïve");
    }

    #[test]
    fn extract_jsonpath_pulls_string_field_from_json_body() {
        let body = r#"{"id":"abc-123","name":"alice"}"#;
        assert_eq!(extract_jsonpath(body, "$.id"), Some("abc-123".into()));
        assert_eq!(extract_jsonpath(body, "$.name"), Some("alice".into()));
        assert_eq!(extract_jsonpath(body, "$.missing"), None);
    }

    #[test]
    fn extract_jsonpath_navigates_nested_objects() {
        let body = r#"{"user":{"id":"7","email":"a@b"}}"#;
        assert_eq!(extract_jsonpath(body, "$.user.id"), Some("7".into()));
        assert_eq!(extract_jsonpath(body, "$.user.email"), Some("a@b".into()));
    }

    #[test]
    fn extract_jsonpath_returns_none_on_invalid_json() {
        assert_eq!(extract_jsonpath("not json", "$.x"), None);
    }

    #[test]
    fn matches_retry_condition_recognizes_5xx() {
        assert!(matches_retry_condition(
            &[RetryCondition::FivexX],
            FailureKind::Status5xx
        ));
        assert!(!matches_retry_condition(
            &[RetryCondition::FivexX],
            FailureKind::Status4xx
        ));
        // `Any` matches everything.
        assert!(matches_retry_condition(
            &[RetryCondition::Any],
            FailureKind::Other
        ));
    }

    #[test]
    fn retry_backoff_grows_exponentially_capped_at_5s() {
        let policy = RetryPolicy {
            max: 10,
            backoff: BackoffKind::Exponential,
            on: default_retry_on(),
        };
        let a1 = retry_backoff(&policy, 1);
        let a2 = retry_backoff(&policy, 2);
        let a3 = retry_backoff(&policy, 3);
        let a10 = retry_backoff(&policy, 10);
        assert!(a1 < a2 && a2 < a3, "exp backoff should grow");
        assert!(
            a10 <= std::time::Duration::from_secs(5),
            "exp backoff should cap at 5s, got {a10:?}"
        );
    }

    #[test]
    fn yaml_suite_with_retry_round_trips() {
        let raw = r#"
name: api with retry
service: api
retry: { max: 3, backoff: exponential, on: ["5xx"] }
steps:
  - http: GET /users
    expect:
      status: 200
"#;
        let suite: YamlSuite = serde_yaml_ng::from_str(raw).unwrap();
        let r = suite.retry.unwrap();
        assert_eq!(r.max, 3);
        assert!(r.on.contains(&RetryCondition::FivexX));
    }

    #[test]
    fn yaml_step_with_capture_and_snapshot_round_trips() {
        let raw = r#"
http: POST /users
body: { name: "test" }
expect:
  status: 201
  snapshot: "fixtures/users.json"
capture:
  user_id: "$.id"
"#;
        let step: HttpStep = serde_yaml_ng::from_str(raw).unwrap();
        assert_eq!(step.capture.get("user_id"), Some(&"$.id".to_string()));
        assert_eq!(
            step.expect
                .snapshot
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            Some("fixtures/users.json".to_string())
        );
    }

    #[test]
    fn discover_parses_yaml_files_in_dot_coral_tests() {
        let dir = tempfile::TempDir::new().unwrap();
        let tests_dir = dir.path().join(".coral/tests");
        std::fs::create_dir_all(&tests_dir).unwrap();
        std::fs::write(
            tests_dir.join("api.yaml"),
            r#"name: api smoke
service: api
steps:
  - http: GET /users
    expect:
      status: 200
"#,
        )
        .unwrap();
        let pairs = UserDefinedRunner::discover_tests_dir(dir.path()).unwrap();
        assert_eq!(pairs.len(), 1);
        let (case, suite) = &pairs[0];
        assert_eq!(case.kind, TestKind::UserDefined);
        assert_eq!(suite.name, "api smoke");
    }

    /// Regression: pre-v0.19.3 the discovery walk was non-recursive, so
    /// files committed by `coral test-discover --commit` (which writes
    /// to `.coral/tests/discovered/<id>.yaml`) were silently invisible
    /// to `coral test`. The subsequent `coral test --include-discovered`
    /// flow re-generated tests in memory from the OpenAPI spec instead
    /// of reading the user's curated YAML — meaning user edits to the
    /// committed YAML had ZERO effect, in clear violation of the
    /// advertised "discover → commit → run" workflow.
    #[test]
    fn discover_walks_recursively_into_subdirectories() {
        let dir = tempfile::TempDir::new().unwrap();
        let nested = dir.path().join(".coral/tests/discovered");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            nested.join("openapi_GET__users.yaml"),
            r#"name: discovered users
service: api
steps:
  - http: GET /users
    expect:
      status: 200
"#,
        )
        .unwrap();
        // Also a top-level file to confirm BOTH levels are walked, not
        // a regression where the fix accidentally only walked subdirs.
        std::fs::write(
            dir.path().join(".coral/tests/manual.yaml"),
            r#"name: manual case
service: api
steps:
  - http: GET /health
    expect:
      status: 200
"#,
        )
        .unwrap();
        let pairs = UserDefinedRunner::discover_tests_dir(dir.path()).unwrap();
        assert_eq!(
            pairs.len(),
            2,
            "expected both manual and discovered, got {pairs:?}"
        );
        let names: Vec<&str> = pairs.iter().map(|(_, s)| s.name.as_str()).collect();
        assert!(names.contains(&"discovered users"));
        assert!(names.contains(&"manual case"));
    }
}
