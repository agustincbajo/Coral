//! `RecordedRunner` — replay Keploy-captured HTTP exchanges as
//! deterministic TestCases (v0.23.2).
//!
//! ## What this is
//!
//! A test runner that reads `.coral/tests/recorded/<service>/*.yaml`
//! files in the [Keploy v1beta1] schema, sends each captured request
//! against the live env, and asserts the response matches the
//! captured one. Pairs with the `coral test record` capture-side
//! subcommand (Linux-only, gated behind the `recorded` Cargo feature)
//! that wraps `keploy record` to produce these YAMLs in the first
//! place.
//!
//! [Keploy v1beta1]: https://github.com/keploy/keploy
//!
//! ## Why parser+replay are always-on (D4)
//!
//! Capture needs eBPF + libraries that only exist on Linux, but
//! parsing + replay are pure I/O against any HTTP endpoint — no
//! kernel hooks. Keeping the parser available on macOS lets a Mac
//! contributor commit a YAML captured on Linux CI and `coral test
//! --kind recorded` will still replay it locally. Only `coral test
//! record` (the capture side) is Linux+feature-gated.
//!
//! ## Assertion ladder (D7)
//!
//! Per captured exchange, in order:
//!
//! 1. **Status code** — must match exactly. Mismatch → Fail.
//! 2. **Response headers** — only `Content-Type` is asserted. Other
//!    headers are deliberately ignored: server-set `Date`, `Server`,
//!    `X-Request-Id`, etc. are noise that varies per replay.
//! 3. **Response body** — JSON deep-equal AFTER recursively stripping
//!    every key listed in `[environments.<env>.recorded].ignore_response_fields`.
//!    The strip is recursive so a nested `id` field at any depth
//!    drops out of both sides before comparison.
//!
//! Non-JSON responses (text, binary) are compared byte-for-byte
//! after the header check. A future v0.23.x can add `ignore_body_pattern`
//! once we hit a real-world need.
//!
//! ## Why curl, not reqwest (consistency with the rest of the workspace)
//!
//! `coral_runner::http`, `commands::notion_push`, and `commands::chaos`
//! all subprocess curl rather than dragging `reqwest` + `tokio` into
//! the sync CLI. Recorded replay follows the same pattern so the
//! workspace stays single-async-runtime-free.

use crate::error::{TestError, TestResult};
use crate::report::{Evidence, HttpEvidence, TestReport, TestStatus};
use crate::spec::{TestCase, TestKind, TestSource, TestSpec};
use crate::{ParallelismHint, TestRunner};
use coral_env::{EnvBackend, EnvHandle, EnvPlan, EnvironmentSpec, RecordedConfig};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

/// On-disk Keploy v1beta1 test-case schema.
///
/// We deliberately `#[serde(default)]` every optional field so a Keploy
/// minor-version bump that adds new fields (or omits ours) doesn't
/// hard-fail the parser — we ignore anything we don't recognize. The
/// fields we DO use are pinned by `recorded_runner_parses_keploy_yaml`.
///
/// Real-world Keploy YAMLs look like:
/// ```yaml
/// version: api.keploy.io/v1beta1
/// kind: Http
/// name: test-1
/// spec:
///   metadata:
///     type: HTTP
///   req:
///     method: GET
///     proto_major: 1
///     proto_minor: 1
///     url: http://localhost:3000/users/42
///     header:
///       Accept: application/json
///     body: ""
///   resp:
///     status_code: 200
///     header:
///       Content-Type: application/json
///     body: '{"id":42,"name":"alice"}'
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeployTestCase {
    /// `api.keploy.io/v1beta1` — pinned but we don't error on drift.
    #[serde(default)]
    pub version: String,
    /// `Http` — the only kind we replay in v0.23.2.
    #[serde(default)]
    pub kind: String,
    /// Human-readable case name (also lifted into the `TestCase.name`).
    #[serde(default)]
    pub name: String,
    /// The captured request + response.
    pub spec: KeploySpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeploySpec {
    pub req: KeployRequest,
    pub resp: KeployResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeployRequest {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub header: BTreeMap<String, String>,
    #[serde(default)]
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeployResponse {
    pub status_code: u16,
    #[serde(default)]
    pub header: BTreeMap<String, String>,
    #[serde(default)]
    pub body: String,
}

impl KeployTestCase {
    /// Parse a Keploy YAML string into our internal representation.
    pub fn from_yaml(raw: &str) -> TestResult<Self> {
        serde_yaml_ng::from_str(raw).map_err(TestError::Yaml)
    }

    /// Read + parse a single Keploy YAML file from disk.
    pub fn from_path(path: &Path) -> TestResult<Self> {
        let raw = std::fs::read_to_string(path).map_err(|source| TestError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_yaml(&raw).map_err(|e| match e {
            TestError::Yaml(yaml_err) => TestError::InvalidSpec {
                path: path.to_path_buf(),
                reason: yaml_err.to_string(),
            },
            other => other,
        })
    }
}

/// Walk `.coral/tests/recorded/<service>/*.yaml` and return one
/// (TestCase, KeployTestCase) pair per file. The directory layout
/// is:
///
/// ```text
/// .coral/tests/recorded/
///   api/
///     test-1.yaml
///     test-2.yaml
///   worker/
///     test-1.yaml
/// ```
///
/// The service name is taken from the parent directory; YAML files
/// directly under `recorded/` (without a service subdir) are ignored
/// — they have no service to target.
pub fn discover_recorded(project_root: &Path) -> TestResult<Vec<(TestCase, KeployTestCase)>> {
    let root = project_root.join(".coral/tests/recorded");
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut out: Vec<(TestCase, KeployTestCase)> = Vec::new();
    let services = match std::fs::read_dir(&root) {
        Ok(it) => it,
        Err(source) => return Err(TestError::Io { path: root, source }),
    };
    let mut service_dirs: Vec<(String, PathBuf)> = Vec::new();
    for entry in services.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) if !n.starts_with('.') => n.to_string(),
            _ => continue,
        };
        service_dirs.push((name, path));
    }
    // Deterministic order across platforms — required for snapshot
    // tests + CI parity.
    service_dirs.sort();
    for (service_name, service_dir) in service_dirs {
        let mut yamls: Vec<PathBuf> = Vec::new();
        let entries = match std::fs::read_dir(&service_dir) {
            Ok(it) => it,
            Err(source) => {
                return Err(TestError::Io {
                    path: service_dir,
                    source,
                });
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = match path.file_name().and_then(|s| s.to_str()) {
                Some(n) if !n.starts_with('.') => n.to_string(),
                _ => continue,
            };
            let lower = name.to_ascii_lowercase();
            if lower.ends_with(".yaml") || lower.ends_with(".yml") {
                yamls.push(path);
            }
        }
        yamls.sort();
        for path in yamls {
            let kc = KeployTestCase::from_path(&path)?;
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("recorded")
                .to_string();
            let case_id = format!("recorded:{service_name}:{stem}");
            let case_name = if kc.name.is_empty() {
                stem.clone()
            } else {
                kc.name.clone()
            };
            let case = TestCase {
                id: case_id,
                name: case_name,
                kind: TestKind::Recorded,
                service: Some(service_name.clone()),
                tags: vec!["recorded".into()],
                source: TestSource::File { path: path.clone() },
                spec: TestSpec(serde_json::to_value(&kc).unwrap_or(serde_json::Value::Null)),
            };
            out.push((case, kc));
        }
    }
    Ok(out)
}

/// `RecordedRunner` — replay captured Keploy YAMLs against a live env.
///
/// The runner holds the env spec so it can pick up
/// `[environments.<env>.recorded].ignore_response_fields`. `backend`
/// and `plan` are stashed for symmetry with `HealthcheckRunner` and
/// `UserDefinedRunner`; v0.23.2 doesn't need them at runtime, but
/// they're free and future-compat (e.g. resolving `${SVC_API_URL}` in
/// captured URLs against the actual published port).
pub struct RecordedRunner {
    #[allow(dead_code)] // reserved for future port-rewriting use
    backend: Arc<dyn EnvBackend>,
    #[allow(dead_code)]
    plan: EnvPlan,
    spec: EnvironmentSpec,
}

impl RecordedRunner {
    pub fn new(backend: Arc<dyn EnvBackend>, plan: EnvPlan, spec: EnvironmentSpec) -> Self {
        Self {
            backend,
            plan,
            spec,
        }
    }

    /// Discover + return TestCases without running them. Used by the
    /// orchestrator's discover step.
    pub fn cases_from_project(project_root: &Path) -> TestResult<Vec<TestCase>> {
        let pairs = discover_recorded(project_root)?;
        Ok(pairs.into_iter().map(|(c, _)| c).collect())
    }

    /// The configured `ignore_response_fields` list, or empty if no
    /// `[environments.<env>.recorded]` block was declared.
    fn ignore_fields(&self) -> &[String] {
        match &self.spec.recorded {
            Some(RecordedConfig {
                ignore_response_fields,
            }) => ignore_response_fields.as_slice(),
            None => &[],
        }
    }

    /// Run a single Keploy exchange and return its `TestStatus`. Pure
    /// over `(KeployTestCase, ignore_fields, http_invoker)` so the
    /// unit tests don't have to spawn curl — they pass a mock
    /// invoker that returns canned responses.
    pub(crate) fn assert_exchange(
        captured: &KeployTestCase,
        ignore_fields: &[String],
        actual_status: u16,
        actual_headers: &BTreeMap<String, String>,
        actual_body: &str,
    ) -> TestStatus {
        // 1. Status check.
        if actual_status != captured.spec.resp.status_code {
            return TestStatus::Fail {
                reason: format!(
                    "status mismatch: expected {}, got {}",
                    captured.spec.resp.status_code, actual_status
                ),
            };
        }
        // 2. Content-Type check (only this header).
        let expected_ct = lookup_ci(&captured.spec.resp.header, "Content-Type");
        let actual_ct = lookup_ci(actual_headers, "Content-Type");
        match (expected_ct, actual_ct) {
            (Some(e), Some(a)) => {
                // Strip parameters (`; charset=...`) before comparison
                // — they vary per server config.
                let e_main = e
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                let a_main = a
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                if e_main != a_main {
                    return TestStatus::Fail {
                        reason: format!("Content-Type mismatch: expected '{e}', got '{a}'"),
                    };
                }
            }
            (Some(e), None) => {
                return TestStatus::Fail {
                    reason: format!("Content-Type mismatch: expected '{e}', got (none)"),
                };
            }
            // Captured had no Content-Type to assert — be lenient.
            (None, _) => {}
        }
        // 3. Body check.
        let expected_body = &captured.spec.resp.body;
        let body_matches =
            if is_json_response(actual_headers) || is_json_response(&captured.spec.resp.header) {
                json_bodies_match(expected_body, actual_body, ignore_fields)
            } else {
                expected_body == actual_body
            };
        if !body_matches {
            return TestStatus::Fail {
                reason: format!(
                    "body mismatch (ignore_fields={:?}): expected {} bytes, got {} bytes",
                    ignore_fields,
                    expected_body.len(),
                    actual_body.len()
                ),
            };
        }
        TestStatus::Pass
    }
}

impl TestRunner for RecordedRunner {
    fn name(&self) -> &'static str {
        "recorded"
    }

    fn supports(&self, kind: TestKind) -> bool {
        matches!(kind, TestKind::Recorded)
    }

    fn run(&self, case: &TestCase, _env: &EnvHandle) -> TestResult<TestReport> {
        let started = Instant::now();
        // Re-parse from the recorded file so the runner is a clean
        // function of disk state — same as UserDefinedRunner.
        let path = match &case.source {
            TestSource::File { path } => path.clone(),
            _ => {
                return Ok(TestReport::new(
                    case.clone(),
                    TestStatus::Skip {
                        reason: "recorded TestCase is missing a File source path".into(),
                    },
                    started.elapsed(),
                ));
            }
        };
        let captured = KeployTestCase::from_path(&path)?;
        let resp = invoke_http(&captured.spec.req)?;
        let status = Self::assert_exchange(
            &captured,
            self.ignore_fields(),
            resp.status_code,
            &resp.headers,
            &resp.body,
        );
        let mut report = TestReport::new(case.clone(), status, started.elapsed());
        report.evidence = Evidence {
            http: Some(HttpEvidence {
                method: captured.spec.req.method.clone(),
                url: captured.spec.req.url.clone(),
                status: resp.status_code,
                body_tail: Some(truncate_for_evidence(&resp.body)),
            }),
            ..Evidence::default()
        };
        Ok(report)
    }

    fn discover(&self, project_root: &Path) -> TestResult<Vec<TestCase>> {
        Self::cases_from_project(project_root)
    }

    fn parallelism_hint(&self) -> ParallelismHint {
        // Each replay hits one URL with one method; no shared state
        // across cases, so they're independently parallelizable.
        ParallelismHint::Isolated
    }
}

// ---- helpers ---------------------------------------------------------------

fn lookup_ci<'a>(map: &'a BTreeMap<String, String>, key: &str) -> Option<&'a String> {
    map.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

fn is_json_response(headers: &BTreeMap<String, String>) -> bool {
    lookup_ci(headers, "Content-Type")
        .map(|v| v.to_ascii_lowercase().contains("json"))
        .unwrap_or(false)
}

/// Compare two JSON-encoded bodies after recursively stripping every
/// key listed in `ignore_fields`. Pure — exposed crate-public for the
/// unit tests.
pub(crate) fn json_bodies_match(expected: &str, actual: &str, ignore_fields: &[String]) -> bool {
    let mut e = match serde_json::from_str::<serde_json::Value>(expected) {
        Ok(v) => v,
        Err(_) => return expected == actual,
    };
    let mut a = match serde_json::from_str::<serde_json::Value>(actual) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let ignore: std::collections::BTreeSet<&str> =
        ignore_fields.iter().map(String::as_str).collect();
    strip_keys_recursive(&mut e, &ignore);
    strip_keys_recursive(&mut a, &ignore);
    e == a
}

/// Recursively strip every JSON object key that appears in `keys`
/// (anywhere in the tree). Mutates in place.
pub(crate) fn strip_keys_recursive(
    value: &mut serde_json::Value,
    keys: &std::collections::BTreeSet<&str>,
) {
    match value {
        serde_json::Value::Object(obj) => {
            obj.retain(|k, _| !keys.contains(k.as_str()));
            for v in obj.values_mut() {
                strip_keys_recursive(v, keys);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                strip_keys_recursive(v, keys);
            }
        }
        _ => {}
    }
}

fn truncate_for_evidence(body: &str) -> String {
    const MAX: usize = 512;
    if body.len() <= MAX {
        body.to_string()
    } else {
        format!("{}…", &body[..MAX])
    }
}

#[derive(Debug, Clone)]
pub(crate) struct InvokedResponse {
    pub(crate) status_code: u16,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: String,
}

/// Drive curl with the captured request, return parsed status +
/// headers + body. Body lands on stdin via `--data-binary @-` when
/// non-empty, mirroring the rest of the workspace's curl pattern.
fn invoke_http(req: &KeployRequest) -> TestResult<InvokedResponse> {
    let cmd = build_invoke_curl_command(req);
    run_curl(cmd, &req.body)
}

/// Build the curl command for replay. Public to the crate so unit
/// tests can assert the argv shape without spawning a process.
pub(crate) fn build_invoke_curl_command(req: &KeployRequest) -> Command {
    let mut cmd = Command::new("curl");
    cmd.args([
        "-s",
        "-i", // include response headers in stdout
        "-X",
        req.method.as_str(),
        req.url.as_str(),
    ]);
    for (k, v) in &req.header {
        cmd.arg("-H");
        cmd.arg(format!("{k}: {v}"));
    }
    if !req.body.is_empty() {
        cmd.arg("--data-binary");
        cmd.arg("@-");
    }
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd
}

fn run_curl(mut cmd: Command, body: &str) -> TestResult<InvokedResponse> {
    let mut child = cmd.spawn().map_err(|source| TestError::Io {
        path: PathBuf::from("curl"),
        source,
    })?;
    if !body.is_empty()
        && let Some(mut stdin) = child.stdin.take()
    {
        use std::io::Write as _;
        let _ = stdin.write_all(body.as_bytes());
        drop(stdin);
    } else {
        drop(child.stdin.take());
    }
    let output = child.wait_with_output().map_err(|source| TestError::Io {
        path: PathBuf::from("curl"),
        source,
    })?;
    let combined = String::from_utf8_lossy(&output.stdout);
    Ok(parse_curl_response(&combined))
}

/// Parse `curl -i` stdout (status line + headers + blank + body).
/// Public to the crate so the unit tests can verify parsing without
/// hitting the network.
pub(crate) fn parse_curl_response(raw: &str) -> InvokedResponse {
    let mut status_code = 0u16;
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    let mut body = String::new();

    // Walk past any 1xx informational responses (e.g., `100 Continue`)
    // — curl emits them too. The "real" response is the LAST
    // status block.
    let mut blocks: Vec<&str> = raw.split("\r\n\r\n").collect();
    if blocks.len() == 1 {
        // No CRLF separator? Try LF-only (some toy servers / mocks).
        blocks = raw.split("\n\n").collect();
    }
    if blocks.is_empty() {
        return InvokedResponse {
            status_code,
            headers,
            body,
        };
    }
    // The last block is the body; the second-to-last is the final
    // header block. (For a normal response, blocks = [headers, body],
    // and the first block holds the status line + headers.)
    let body_str = blocks.last().copied().unwrap_or("");
    body = body_str.to_string();
    // The header block we want is the LAST block whose first line
    // looks like `HTTP/<version> <status>` — that is, the trailing
    // headers, not any 100-continue precursor.
    let mut header_block = "";
    for block in blocks.iter().take(blocks.len().saturating_sub(1)).rev() {
        if block
            .lines()
            .next()
            .map(|l| l.starts_with("HTTP/"))
            .unwrap_or(false)
        {
            header_block = block;
            break;
        }
    }
    if header_block.is_empty() && blocks.len() >= 2 {
        header_block = blocks[blocks.len() - 2];
    }
    let mut lines = header_block.lines();
    if let Some(status_line) = lines.next() {
        // Format: `HTTP/1.1 200 OK`
        let mut parts = status_line.split_whitespace();
        let _ = parts.next();
        if let Some(code) = parts.next() {
            status_code = code.parse::<u16>().unwrap_or(0);
        }
    }
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    InvokedResponse {
        status_code,
        headers,
        body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_env::spec::EnvMode;
    use coral_env::{RealService, ServiceKind};
    use std::collections::BTreeMap as Map;

    fn spec_with_recorded(ignore: Vec<String>) -> EnvironmentSpec {
        let mut services = Map::new();
        services.insert(
            "api".to_string(),
            ServiceKind::Real(Box::new(RealService {
                repo: None,
                image: Some("api:dev".into()),
                build: None,
                ports: vec![3000],
                env: Map::new(),
                depends_on: vec![],
                healthcheck: None,
                watch: None,
            })),
        );
        EnvironmentSpec {
            name: "dev".into(),
            backend: "compose".into(),
            mode: EnvMode::Managed,
            compose_command: "auto".into(),
            production: false,
            env_file: None,
            services,
            chaos: None,
            chaos_scenarios: Vec::new(),
            monitors: Vec::new(),
            recorded: Some(RecordedConfig {
                ignore_response_fields: ignore,
            }),
        }
    }

    /// **T2 — parser pin (acceptance #3).**
    /// A canonical Keploy v1beta1 YAML deserializes into our internal
    /// representation with all fields populated correctly. Runs on
    /// macOS (parser is always-on per D4).
    #[test]
    fn recorded_runner_parses_keploy_yaml() {
        let yaml = r#"
version: api.keploy.io/v1beta1
kind: Http
name: get-user
spec:
  metadata:
    type: HTTP
  req:
    method: GET
    url: http://localhost:3000/users/42
    header:
      Accept: application/json
    body: ""
  resp:
    status_code: 200
    header:
      Content-Type: application/json
    body: '{"id":42,"name":"alice"}'
"#;
        let parsed = KeployTestCase::from_yaml(yaml).expect("parses");
        assert_eq!(parsed.version, "api.keploy.io/v1beta1");
        assert_eq!(parsed.kind, "Http");
        assert_eq!(parsed.name, "get-user");
        assert_eq!(parsed.spec.req.method, "GET");
        assert_eq!(parsed.spec.req.url, "http://localhost:3000/users/42");
        assert_eq!(
            parsed.spec.req.header.get("Accept").map(String::as_str),
            Some("application/json")
        );
        assert_eq!(parsed.spec.resp.status_code, 200);
        assert_eq!(
            parsed
                .spec
                .resp
                .header
                .get("Content-Type")
                .map(String::as_str),
            Some("application/json")
        );
        assert!(parsed.spec.resp.body.contains("alice"));
    }

    /// **T3 — status mismatch fails (acceptance #5).**
    /// Captured response is 200, live response is 500 → Fail with a
    /// status-mismatch reason.
    #[test]
    fn recorded_runner_status_mismatch_fails() {
        let captured = KeployTestCase {
            version: "api.keploy.io/v1beta1".into(),
            kind: "Http".into(),
            name: "test".into(),
            spec: KeploySpec {
                req: KeployRequest {
                    method: "GET".into(),
                    url: "http://localhost/x".into(),
                    header: Map::new(),
                    body: String::new(),
                },
                resp: KeployResponse {
                    status_code: 200,
                    header: Map::from([("Content-Type".into(), "application/json".into())]),
                    body: r#"{"ok":true}"#.into(),
                },
            },
        };
        let actual_headers: BTreeMap<String, String> =
            BTreeMap::from([("Content-Type".into(), "application/json".into())]);
        let status =
            RecordedRunner::assert_exchange(&captured, &[], 500, &actual_headers, r#"{"ok":true}"#);
        match status {
            TestStatus::Fail { reason } => assert!(
                reason.contains("status mismatch")
                    && reason.contains("200")
                    && reason.contains("500"),
                "wrong reason: {reason}"
            ),
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    /// **T4 — body diff with ignore_fields passes (acceptance #6).**
    /// Captured `id=42`, live `id=43`; with `ignore_response_fields =
    /// ["id"]` the deep-equal comparison passes after both sides are
    /// stripped.
    #[test]
    fn recorded_runner_body_diff_with_ignore_fields_passes() {
        let captured = KeployTestCase {
            version: "v1beta1".into(),
            kind: "Http".into(),
            name: "t".into(),
            spec: KeploySpec {
                req: KeployRequest {
                    method: "GET".into(),
                    url: "http://localhost/users/42".into(),
                    header: Map::new(),
                    body: String::new(),
                },
                resp: KeployResponse {
                    status_code: 200,
                    header: Map::from([("Content-Type".into(), "application/json".into())]),
                    body: r#"{"id":42,"name":"alice"}"#.into(),
                },
            },
        };
        let actual_headers: BTreeMap<String, String> =
            BTreeMap::from([("Content-Type".into(), "application/json".into())]);
        let ignore = vec!["id".to_string()];
        let status = RecordedRunner::assert_exchange(
            &captured,
            &ignore,
            200,
            &actual_headers,
            r#"{"id":99,"name":"alice"}"#,
        );
        assert!(matches!(status, TestStatus::Pass), "got: {status:?}");
    }

    /// **T5 — body diff WITHOUT ignore_fields fails.**
    /// Same shape but no ignore list → Fail with body-mismatch.
    #[test]
    fn recorded_runner_body_diff_without_ignore_fields_fails() {
        let captured = KeployTestCase {
            version: "v1beta1".into(),
            kind: "Http".into(),
            name: "t".into(),
            spec: KeploySpec {
                req: KeployRequest {
                    method: "GET".into(),
                    url: "http://localhost/u".into(),
                    header: Map::new(),
                    body: String::new(),
                },
                resp: KeployResponse {
                    status_code: 200,
                    header: Map::from([("Content-Type".into(), "application/json".into())]),
                    body: r#"{"id":42,"name":"alice"}"#.into(),
                },
            },
        };
        let actual_headers: BTreeMap<String, String> =
            BTreeMap::from([("Content-Type".into(), "application/json".into())]);
        let status = RecordedRunner::assert_exchange(
            &captured,
            &[], // no ignore list
            200,
            &actual_headers,
            r#"{"id":99,"name":"alice"}"#,
        );
        assert!(
            matches!(status, TestStatus::Fail { .. }),
            "expected Fail, got {status:?}"
        );
    }

    /// Recursive strip — `id` at top level AND inside an array AND
    /// inside a nested object all drop out.
    #[test]
    fn strip_keys_recursive_walks_arrays_and_nested_objects() {
        let mut v: serde_json::Value = serde_json::json!({
            "id": 42,
            "name": "alice",
            "items": [
                {"id": 1, "x": "a"},
                {"id": 2, "x": "b"}
            ],
            "meta": { "id": 999, "type": "user" }
        });
        let mut keys: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        keys.insert("id");
        strip_keys_recursive(&mut v, &keys);
        let expected = serde_json::json!({
            "name": "alice",
            "items": [{"x":"a"}, {"x":"b"}],
            "meta": {"type":"user"}
        });
        assert_eq!(v, expected);
    }

    /// Curl `-i` output parsing: status line, headers, body block.
    #[test]
    fn parse_curl_response_separates_headers_and_body() {
        let raw = "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nX-Other: yes\r\n\r\n{\"id\":1}";
        let parsed = parse_curl_response(raw);
        assert_eq!(parsed.status_code, 201);
        assert_eq!(
            parsed.headers.get("Content-Type").map(String::as_str),
            Some("application/json")
        );
        assert_eq!(parsed.body, "{\"id\":1}");
    }

    #[test]
    fn parse_curl_response_handles_100_continue_precursor() {
        let raw =
            "HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nhello";
        let parsed = parse_curl_response(raw);
        assert_eq!(parsed.status_code, 200);
        assert_eq!(parsed.body, "hello");
    }

    /// **T6 — supports() is wired correctly.**
    /// `RecordedRunner::supports(TestKind::Recorded)` is true; other
    /// kinds are false.
    #[test]
    fn recorded_runner_supports_only_recorded_kind() {
        let spec = spec_with_recorded(vec![]);
        let backend: Arc<dyn EnvBackend> = Arc::new(coral_env::MockBackend::new());
        let plan =
            EnvPlan::from_spec(&spec, Path::new("/tmp/x"), &Map::new()).expect("plan from spec");
        let runner = RecordedRunner::new(backend, plan, spec);
        assert!(runner.supports(TestKind::Recorded));
        assert!(!runner.supports(TestKind::Healthcheck));
        assert!(!runner.supports(TestKind::UserDefined));
    }

    /// `discover_recorded` walks `<root>/.coral/tests/recorded/<svc>/*.yaml`
    /// and emits one TestCase per file with the right service +
    /// source path.
    #[test]
    fn discover_recorded_walks_service_directories() {
        let tmp = tempfile::TempDir::new().unwrap();
        let api_dir = tmp.path().join(".coral/tests/recorded/api");
        std::fs::create_dir_all(&api_dir).unwrap();
        let yaml = r#"
version: api.keploy.io/v1beta1
kind: Http
name: t1
spec:
  req:
    method: GET
    url: http://localhost/x
    header: {}
    body: ""
  resp:
    status_code: 200
    header:
      Content-Type: application/json
    body: '{}'
"#;
        std::fs::write(api_dir.join("test-1.yaml"), yaml).unwrap();
        std::fs::write(api_dir.join("test-2.yml"), yaml).unwrap();
        let pairs = discover_recorded(tmp.path()).expect("discover");
        assert_eq!(pairs.len(), 2);
        for (case, _) in &pairs {
            assert_eq!(case.kind, TestKind::Recorded);
            assert_eq!(case.service.as_deref(), Some("api"));
            assert!(case.tags.iter().any(|t| t == "recorded"));
        }
        // Sorted order is deterministic.
        assert_eq!(pairs[0].0.id, "recorded:api:test-1");
        assert_eq!(pairs[1].0.id, "recorded:api:test-2");
    }

    #[test]
    fn discover_recorded_returns_empty_when_dir_missing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pairs = discover_recorded(tmp.path()).expect("ok");
        assert!(pairs.is_empty());
    }

    #[test]
    fn build_invoke_curl_command_uses_method_and_url() {
        let req = KeployRequest {
            method: "POST".into(),
            url: "http://localhost:3000/users".into(),
            header: BTreeMap::from([("Accept".into(), "application/json".into())]),
            body: r#"{"name":"x"}"#.into(),
        };
        let cmd = build_invoke_curl_command(&req);
        let argv: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(argv.contains(&"POST".to_string()));
        assert!(
            argv.iter()
                .any(|a| a.contains("http://localhost:3000/users")),
            "argv: {argv:?}"
        );
        // Body must travel via stdin (`@-`), not argv.
        assert!(argv.iter().any(|a| a == "@-"), "argv: {argv:?}");
        // Header was emitted.
        assert!(
            argv.iter().any(|a| a.contains("Accept: application/json")),
            "argv: {argv:?}"
        );
    }

    /// Content-Type with charset parameters compares main type only.
    #[test]
    fn content_type_charset_parameters_are_stripped_before_compare() {
        let captured = KeployTestCase {
            version: "v1beta1".into(),
            kind: "Http".into(),
            name: "t".into(),
            spec: KeploySpec {
                req: KeployRequest {
                    method: "GET".into(),
                    url: "http://localhost/x".into(),
                    header: Map::new(),
                    body: String::new(),
                },
                resp: KeployResponse {
                    status_code: 200,
                    header: Map::from([("Content-Type".into(), "application/json".into())]),
                    body: r#"{}"#.into(),
                },
            },
        };
        let actual_headers: BTreeMap<String, String> = BTreeMap::from([(
            "Content-Type".into(),
            "application/json; charset=utf-8".into(),
        )]);
        let status = RecordedRunner::assert_exchange(&captured, &[], 200, &actual_headers, r#"{}"#);
        assert!(matches!(status, TestStatus::Pass), "got: {status:?}");
    }
}
