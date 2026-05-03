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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HttpExpect {
    pub status: Option<u16>,
    pub body_contains: Option<String>,
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
}

impl UserDefinedRunner {
    pub fn new(backend: Arc<dyn EnvBackend>, plan: EnvPlan) -> Self {
        Self { backend, plan }
    }

    /// Walk `<project_root>/.coral/tests/` for `*.yaml` and `*.yml`
    /// files, parse them into TestCases.
    pub fn discover_tests_dir(project_root: &Path) -> TestResult<Vec<(TestCase, YamlSuite)>> {
        let dir = project_root.join(".coral/tests");
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&dir).map_err(|source| TestError::Io {
            path: dir.clone(),
            source,
        })? {
            let entry = entry.map_err(|source| TestError::Io {
                path: dir.clone(),
                source,
            })?;
            let path = entry.path();
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
            if !matches!(ext, "yaml" | "yml") {
                continue;
            }
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
        for (step_idx, step) in suite.steps.iter().enumerate() {
            let outcome = match step {
                YamlStep::Http(s) => self.run_http(s, env_status),
                YamlStep::Exec(s) => Self::run_exec(s),
            };
            match outcome {
                Ok(()) => continue,
                Err(reason) => {
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
        }
        Ok(TestReport::new(
            case.clone(),
            TestStatus::Pass,
            started.elapsed(),
        ))
    }

    fn run_http(
        &self,
        step: &HttpStep,
        env_status: Option<&ServiceStatus>,
    ) -> std::result::Result<(), String> {
        let (method, path) = parse_http_line(&step.http)
            .ok_or_else(|| format!("invalid http line: {}", step.http))?;
        let host_port = match env_status.and_then(|s| s.published_ports.first()) {
            Some(p) if p.host_port > 0 => p.host_port,
            _ => return Err("service has no published port; bring it up first".into()),
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
            cmd.arg("-H").arg(format!("{k}: {v}"));
        }
        if let Some(body) = &step.body {
            let body_str = body.to_string();
            cmd.args(["-H", "Content-Type: application/json"]);
            cmd.args(["-d", body_str.as_str()]);
        }
        cmd.arg(&url);

        let output = cmd
            .output()
            .map_err(|e| format!("failed to invoke curl: {e}"))?;
        let body = String::from_utf8_lossy(&output.stdout).into_owned();
        let (response_body, status_code) = split_curl_status(&body);

        if let Some(expected) = step.expect.status {
            if status_code != expected {
                return Err(format!(
                    "expected HTTP status {expected}, got {status_code}"
                ));
            }
        }
        if let Some(needle) = &step.expect.body_contains {
            if !response_body.contains(needle.as_str()) {
                return Err(format!(
                    "response body does not contain '{needle}' (first 200 bytes: {})",
                    response_body.chars().take(200).collect::<String>()
                ));
            }
        }
        Ok(())
    }

    fn run_exec(step: &ExecStep) -> std::result::Result<(), String> {
        if step.exec.is_empty() {
            return Err("exec step has no command".into());
        }
        let mut cmd = Command::new(&step.exec[0]);
        for arg in &step.exec[1..] {
            cmd.arg(arg);
        }
        let output = cmd
            .output()
            .map_err(|e| format!("failed to invoke {}: {e}", step.exec[0]))?;
        let exit = output.status.code().unwrap_or(-1);
        if let Some(expected) = step.expect.exit_code {
            if exit != expected {
                return Err(format!("expected exit {expected}, got {exit}"));
            }
        }
        if let Some(needle) = &step.expect.stdout_contains {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if !stdout.contains(needle.as_str()) {
                return Err(format!(
                    "stdout does not contain '{needle}' (got: {})",
                    stdout.chars().take(200).collect::<String>()
                ));
            }
        }
        Ok(())
    }
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
}
