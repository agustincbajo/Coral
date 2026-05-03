//! `TestReport` — the runner output. Includes JUnit XML emission for
//! CI integration.

use crate::spec::TestCase;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestReport {
    pub case: TestCase,
    pub status: TestStatus,
    pub started_at: DateTime<Utc>,
    pub duration_ms: u64,
    pub evidence: Evidence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum TestStatus {
    Pass,
    Fail { reason: String },
    Skip { reason: String },
    Error { reason: String },
}

impl TestStatus {
    pub fn is_pass(&self) -> bool {
        matches!(self, TestStatus::Pass)
    }
    pub fn is_fail(&self) -> bool {
        matches!(self, TestStatus::Fail { .. })
    }
}

/// Per-case evidence — request/response, exit codes, captured spans
/// for trace tests, etc. `Generic` is the catch-all for the v0.18
/// wave 1 scaffold; concrete shapes evolve with each runner.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Evidence {
    pub stdout_tail: Option<String>,
    pub stderr_tail: Option<String>,
    pub http: Option<HttpEvidence>,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpEvidence {
    pub method: String,
    pub url: String,
    pub status: u16,
    pub body_tail: Option<String>,
}

impl TestReport {
    pub fn new(case: TestCase, status: TestStatus, duration: Duration) -> Self {
        Self {
            case,
            status,
            started_at: Utc::now(),
            duration_ms: duration.as_millis() as u64,
            evidence: Evidence::default(),
        }
    }
}

/// JUnit XML emitter — minimal but compliant with the GitHub Actions
/// reporter plugin and most CI dashboards.
pub struct JunitOutput;

impl JunitOutput {
    /// Render `reports` as a single `<testsuites>` document.
    pub fn render(reports: &[TestReport]) -> String {
        let total = reports.len();
        let failures = reports
            .iter()
            .filter(|r| matches!(r.status, TestStatus::Fail { .. }))
            .count();
        let errors = reports
            .iter()
            .filter(|r| matches!(r.status, TestStatus::Error { .. }))
            .count();
        let skipped = reports
            .iter()
            .filter(|r| matches!(r.status, TestStatus::Skip { .. }))
            .count();
        let total_ms: u64 = reports.iter().map(|r| r.duration_ms).sum();

        let mut out = String::new();
        out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        out.push_str(&format!(
            "<testsuites name=\"coral\" tests=\"{}\" failures=\"{}\" errors=\"{}\" skipped=\"{}\" time=\"{:.3}\">\n",
            total,
            failures,
            errors,
            skipped,
            total_ms as f64 / 1000.0
        ));
        out.push_str("  <testsuite name=\"coral.tests\">\n");
        for r in reports {
            out.push_str(&format!(
                "    <testcase name=\"{}\" classname=\"{}\" time=\"{:.3}\">\n",
                xml_escape(&r.case.name),
                r.case
                    .service
                    .as_deref()
                    .map(xml_escape)
                    .unwrap_or_else(|| "unspecified".to_string()),
                r.duration_ms as f64 / 1000.0
            ));
            match &r.status {
                TestStatus::Pass => {}
                TestStatus::Fail { reason } => out.push_str(&format!(
                    "      <failure message=\"{}\"/>\n",
                    xml_escape(reason)
                )),
                TestStatus::Error { reason } => out.push_str(&format!(
                    "      <error message=\"{}\"/>\n",
                    xml_escape(reason)
                )),
                TestStatus::Skip { reason } => out.push_str(&format!(
                    "      <skipped message=\"{}\"/>\n",
                    xml_escape(reason)
                )),
            }
            out.push_str("    </testcase>\n");
        }
        out.push_str("  </testsuite>\n");
        out.push_str("</testsuites>\n");
        out
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{TestCase, TestKind, TestSource, TestSpec};
    use std::time::Duration;

    fn case(name: &str) -> TestCase {
        TestCase {
            id: name.to_string(),
            name: name.to_string(),
            kind: TestKind::UserDefined,
            service: Some("api".into()),
            tags: vec![],
            source: TestSource::Inline,
            spec: TestSpec::empty(),
        }
    }

    #[test]
    fn junit_renders_passing_suite() {
        let r = TestReport::new(case("smoke"), TestStatus::Pass, Duration::from_millis(123));
        let xml = JunitOutput::render(&[r]);
        assert!(xml.contains("tests=\"1\""));
        assert!(xml.contains("failures=\"0\""));
        assert!(xml.contains("name=\"smoke\""));
    }

    #[test]
    fn junit_renders_failure_with_message() {
        let r = TestReport::new(
            case("smoke"),
            TestStatus::Fail {
                reason: "expected 200, got 500".into(),
            },
            Duration::from_millis(8),
        );
        let xml = JunitOutput::render(&[r]);
        assert!(xml.contains("failures=\"1\""));
        assert!(xml.contains("expected 200"));
    }

    #[test]
    fn xml_escape_handles_ampersand_and_quotes() {
        assert_eq!(xml_escape("a & b"), "a &amp; b");
        assert_eq!(xml_escape("\"x\""), "&quot;x&quot;");
        assert_eq!(xml_escape("<a>"), "&lt;a&gt;");
    }
}
