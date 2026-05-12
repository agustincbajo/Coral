//! E2E browser test runner (Playwright).
//!
//! Executes Playwright scripts for cross-service UI flows. The spec
//! carries: `url` (base URL), `script_path` (path to .js/.ts
//! Playwright script), `expected_selector` (CSS selector that must
//! exist after the script runs), and `timeout_ms`.
//!
//! ## v0.25 scope
//!
//! Structural validation only — the runner parses the browser spec
//! fields and verifies the script file exists on disk. Actual
//! Playwright execution (npx playwright test) is deferred to v0.26+.
//!
//! When the spec is well-formed and the script exists, the runner
//! reports `Skip` with evidence capturing the parsed metadata.
//! When required fields are missing or the script file does not exist,
//! the runner reports `Fail` with a descriptive reason.

use crate::error::TestResult;
use crate::report::{Evidence, TestReport, TestStatus};
use crate::spec::{TestCase, TestKind};
use crate::{ParallelismHint, TestRunner};
use coral_env::EnvHandle;
use std::path::Path;
use std::time::Instant;

pub struct BrowserRunner;

impl Default for BrowserRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserRunner {
    pub fn new() -> Self {
        Self
    }
}

impl TestRunner for BrowserRunner {
    fn name(&self) -> &'static str {
        "browser"
    }

    fn supports(&self, kind: TestKind) -> bool {
        kind == TestKind::E2eBrowser
    }

    fn run(&self, case: &TestCase, _env: &EnvHandle) -> TestResult<TestReport> {
        let spec = &case.spec.0;
        let started = Instant::now();

        // Parse required fields from the spec.
        let url = match spec.get("url").and_then(|v| v.as_str()) {
            Some(u) => u,
            None => {
                let evidence = Evidence {
                    stdout_tail: Some("missing required field: url".into()),
                    ..Default::default()
                };
                let status = TestStatus::Fail {
                    reason: "browser spec missing required field: url".into(),
                };
                let mut report = TestReport::new(case.clone(), status, started.elapsed());
                report.evidence = evidence;
                return Ok(report);
            }
        };

        let script_path = match spec.get("script_path").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                let evidence = Evidence {
                    stdout_tail: Some("missing required field: script_path".into()),
                    ..Default::default()
                };
                let status = TestStatus::Fail {
                    reason: "browser spec missing required field: script_path".into(),
                };
                let mut report = TestReport::new(case.clone(), status, started.elapsed());
                report.evidence = evidence;
                return Ok(report);
            }
        };

        let expected_selector = spec
            .get("expected_selector")
            .and_then(|v| v.as_str())
            .unwrap_or("body");
        let timeout_ms = spec
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(30_000);

        // Structural check: verify the script file exists.
        if !Path::new(script_path).exists() {
            let evidence = Evidence {
                stdout_tail: Some(format!("script_path={script_path} error=file_not_found")),
                ..Default::default()
            };
            let status = TestStatus::Fail {
                reason: format!("playwright script not found: {script_path}"),
            };
            let mut report = TestReport::new(case.clone(), status, started.elapsed());
            report.evidence = evidence;
            return Ok(report);
        }

        // All structural checks passed — report Skip (execution deferred).
        let evidence = Evidence {
            stdout_tail: Some(format!(
                "url={url} script_path={script_path} expected_selector={expected_selector} timeout_ms={timeout_ms} validation=structural_only"
            )),
            ..Default::default()
        };

        let status = TestStatus::Skip {
            reason: "browser runner: Playwright execution deferred to v0.26; tracked at https://github.com/agustincbajo/Coral#roadmap".into(),
        };

        let mut report = TestReport::new(case.clone(), status, started.elapsed());
        report.evidence = evidence;
        Ok(report)
    }

    fn parallelism_hint(&self) -> ParallelismHint {
        ParallelismHint::Sequential
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{TestCase, TestKind, TestSource, TestSpec};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn env_handle() -> EnvHandle {
        EnvHandle {
            backend: "mock".into(),
            artifact_hash: "x".into(),
            artifact_path: PathBuf::from("/tmp"),
            state: BTreeMap::new(),
        }
    }

    fn browser_case(spec_value: serde_json::Value) -> TestCase {
        TestCase {
            id: "browser:checkout:happy_path".into(),
            name: "checkout happy path".into(),
            kind: TestKind::E2eBrowser,
            service: Some("frontend".into()),
            tags: vec!["e2e".into(), "browser".into()],
            source: TestSource::Inline,
            spec: TestSpec(spec_value),
        }
    }

    #[test]
    fn supports_returns_true_for_e2e_browser() {
        let runner = BrowserRunner::new();
        assert!(runner.supports(TestKind::E2eBrowser));
    }

    #[test]
    fn supports_returns_false_for_other_kinds() {
        let runner = BrowserRunner::new();
        assert!(!runner.supports(TestKind::Healthcheck));
        assert!(!runner.supports(TestKind::UserDefined));
        assert!(!runner.supports(TestKind::Contract));
        assert!(!runner.supports(TestKind::Event));
        assert!(!runner.supports(TestKind::PropertyBased));
        assert!(!runner.supports(TestKind::Trace));
    }

    #[test]
    fn name_returns_browser() {
        let runner = BrowserRunner::new();
        assert_eq!(runner.name(), "browser");
    }

    #[test]
    fn parallelism_hint_returns_sequential() {
        let runner = BrowserRunner::new();
        assert_eq!(runner.parallelism_hint(), ParallelismHint::Sequential);
    }

    #[test]
    fn run_with_valid_spec_and_existing_script_returns_skip() {
        let runner = BrowserRunner::new();
        // Use a file we know exists (Cargo.toml in the crate root).
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let script_path = format!("{}/Cargo.toml", manifest_dir);

        let case = browser_case(serde_json::json!({
            "url": "http://localhost:3000",
            "script_path": script_path,
            "expected_selector": "#app",
            "timeout_ms": 10000
        }));
        let env = env_handle();
        let report = runner.run(&case, &env).expect("run succeeds");
        assert!(
            matches!(report.status, TestStatus::Skip { .. }),
            "expected Skip, got {:?}",
            report.status
        );
        let tail = report.evidence.stdout_tail.expect("evidence present");
        assert!(
            tail.contains("url=http://localhost:3000"),
            "evidence should note url: {tail}"
        );
        assert!(
            tail.contains("expected_selector=#app"),
            "evidence should note selector: {tail}"
        );
        assert!(
            tail.contains("timeout_ms=10000"),
            "evidence should note timeout: {tail}"
        );
        assert!(
            tail.contains("validation=structural_only"),
            "evidence should note validation mode: {tail}"
        );
    }

    #[test]
    fn run_with_missing_url_reports_fail() {
        let runner = BrowserRunner::new();
        let case = browser_case(serde_json::json!({
            "script_path": "/tmp/test.ts",
            "expected_selector": "#app"
        }));
        let env = env_handle();
        let report = runner.run(&case, &env).expect("run succeeds");
        match &report.status {
            TestStatus::Fail { reason } => {
                assert!(
                    reason.contains("url"),
                    "fail reason should mention url: {reason}"
                );
            }
            other => panic!("expected Fail, got {:?}", other),
        }
        let tail = report.evidence.stdout_tail.expect("evidence present");
        assert!(tail.contains("missing required field: url"));
    }

    #[test]
    fn run_with_missing_script_path_reports_fail() {
        let runner = BrowserRunner::new();
        let case = browser_case(serde_json::json!({
            "url": "http://localhost:3000",
            "expected_selector": "#app"
        }));
        let env = env_handle();
        let report = runner.run(&case, &env).expect("run succeeds");
        match &report.status {
            TestStatus::Fail { reason } => {
                assert!(
                    reason.contains("script_path"),
                    "fail reason should mention script_path: {reason}"
                );
            }
            other => panic!("expected Fail, got {:?}", other),
        }
    }

    #[test]
    fn run_with_nonexistent_script_reports_fail() {
        let runner = BrowserRunner::new();
        let case = browser_case(serde_json::json!({
            "url": "http://localhost:3000",
            "script_path": "/nonexistent/path/to/test.spec.ts",
            "expected_selector": "#app"
        }));
        let env = env_handle();
        let report = runner.run(&case, &env).expect("run succeeds");
        match &report.status {
            TestStatus::Fail { reason } => {
                assert!(
                    reason.contains("not found"),
                    "fail reason should mention not found: {reason}"
                );
            }
            other => panic!("expected Fail, got {:?}", other),
        }
        let tail = report.evidence.stdout_tail.expect("evidence present");
        assert!(tail.contains("file_not_found"), "evidence: {tail}");
    }

    #[test]
    fn run_with_empty_spec_reports_fail() {
        let runner = BrowserRunner::new();
        let case = browser_case(serde_json::json!({}));
        let env = env_handle();
        let report = runner.run(&case, &env).expect("run succeeds");
        // Empty spec has no `url` → Fail.
        assert!(
            matches!(report.status, TestStatus::Fail { .. }),
            "expected Fail for empty spec, got {:?}",
            report.status
        );
    }
}
