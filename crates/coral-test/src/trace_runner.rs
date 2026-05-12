//! OpenTelemetry trace assertion runner (M3.5).
//!
//! Validates assertions about OpenTelemetry spans — service name, span
//! name, expected attributes, and duration constraints.
//!
//! ## v0.25 scope
//!
//! Structural validation only — the runner parses the trace spec
//! (`service_name`, `span_name`, `expected_attributes`, `min_duration_ms`,
//! `max_duration_ms`, `otel_endpoint`) and validates that the spec is
//! well-formed. Live OTLP query execution against a collector is deferred
//! to a future release (needs an OTLP gRPC/HTTP client dep and collector
//! connection wiring).
//!
//! When `service_name` is present, the runner reports `Skip` with
//! evidence describing what would be asserted. When it is absent, the
//! runner reports `Error` noting the missing required field.

use crate::error::TestResult;
use crate::report::{Evidence, TestReport, TestStatus};
use crate::spec::{TestCase, TestKind};
use crate::{ParallelismHint, TestRunner};
use coral_env::EnvHandle;
use std::time::Instant;

pub struct TraceRunner;

impl Default for TraceRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl TraceRunner {
    pub fn new() -> Self {
        Self
    }
}

impl TestRunner for TraceRunner {
    fn name(&self) -> &'static str {
        "trace"
    }

    fn supports(&self, kind: TestKind) -> bool {
        kind == TestKind::Trace
    }

    fn run(&self, case: &TestCase, _env: &EnvHandle) -> TestResult<TestReport> {
        let spec = &case.spec.0;
        let started = Instant::now();

        let service_name = spec.get("service_name").and_then(|v| v.as_str());
        let span_name = spec
            .get("span_name")
            .and_then(|v| v.as_str())
            .unwrap_or("*");
        let otel_endpoint = spec
            .get("otel_endpoint")
            .and_then(|v| v.as_str())
            .unwrap_or("http://localhost:4318");
        let expected_attributes = spec.get("expected_attributes");
        let min_duration_ms = spec.get("min_duration_ms").and_then(|v| v.as_u64());
        let max_duration_ms = spec.get("max_duration_ms").and_then(|v| v.as_u64());

        let mut evidence = Evidence::default();

        let status = match service_name {
            Some(svc) => {
                let mut desc = format!(
                    "service_name={svc} span_name={span_name} otel_endpoint={otel_endpoint}"
                );
                if let Some(attrs) = expected_attributes {
                    if let Some(obj) = attrs.as_object() {
                        let keys: Vec<&String> = obj.keys().collect();
                        desc.push_str(&format!(
                            " expected_attrs=[{}]",
                            keys.iter()
                                .map(|k| k.as_str())
                                .collect::<Vec<_>>()
                                .join(",")
                        ));
                    }
                }
                if let Some(min) = min_duration_ms {
                    desc.push_str(&format!(" min_duration_ms={min}"));
                }
                if let Some(max) = max_duration_ms {
                    desc.push_str(&format!(" max_duration_ms={max}"));
                }
                evidence.stdout_tail = Some(desc);
                TestStatus::Skip {
                    reason: "trace runner: live OTLP query deferred to future release; tracked at https://github.com/agustincbajo/Coral#roadmap".into(),
                }
            }
            None => {
                evidence.stdout_tail = Some("error: service_name is required in trace spec".into());
                TestStatus::Error {
                    reason: "missing required field: service_name".into(),
                }
            }
        };

        let mut report = TestReport::new(case.clone(), status, started.elapsed());
        report.evidence = evidence;
        Ok(report)
    }

    fn parallelism_hint(&self) -> ParallelismHint {
        ParallelismHint::Isolated
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

    fn minimal_case(spec_value: serde_json::Value) -> TestCase {
        TestCase {
            id: "trace:payments:process_payment".into(),
            name: "process_payment span".into(),
            kind: TestKind::Trace,
            service: Some("payments".into()),
            tags: vec!["trace".into()],
            source: TestSource::Inline,
            spec: TestSpec(spec_value),
        }
    }

    #[test]
    fn supports_returns_true_for_trace() {
        let runner = TraceRunner::new();
        assert!(runner.supports(TestKind::Trace));
    }

    #[test]
    fn supports_returns_false_for_other_kinds() {
        let runner = TraceRunner::new();
        assert!(!runner.supports(TestKind::Healthcheck));
        assert!(!runner.supports(TestKind::UserDefined));
        assert!(!runner.supports(TestKind::Contract));
        assert!(!runner.supports(TestKind::Event));
        assert!(!runner.supports(TestKind::PropertyBased));
    }

    #[test]
    fn name_returns_trace() {
        let runner = TraceRunner::new();
        assert_eq!(runner.name(), "trace");
    }

    #[test]
    fn parallelism_hint_returns_isolated() {
        let runner = TraceRunner::new();
        assert_eq!(runner.parallelism_hint(), ParallelismHint::Isolated);
    }

    #[test]
    fn run_with_valid_spec_returns_skip_with_evidence() {
        let runner = TraceRunner::new();
        let case = minimal_case(serde_json::json!({
            "service_name": "payments",
            "span_name": "process_payment",
            "otel_endpoint": "http://localhost:4318",
            "expected_attributes": {
                "payment.method": "credit_card",
                "payment.currency": "USD"
            },
            "min_duration_ms": 10,
            "max_duration_ms": 5000
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
            tail.contains("service_name=payments"),
            "evidence should note service name: {tail}"
        );
        assert!(
            tail.contains("span_name=process_payment"),
            "evidence should note span name: {tail}"
        );
        assert!(
            tail.contains("otel_endpoint=http://localhost:4318"),
            "evidence should note endpoint: {tail}"
        );
        assert!(
            tail.contains("expected_attrs="),
            "evidence should note expected attributes: {tail}"
        );
        assert!(
            tail.contains("min_duration_ms=10"),
            "evidence should note min duration: {tail}"
        );
        assert!(
            tail.contains("max_duration_ms=5000"),
            "evidence should note max duration: {tail}"
        );
    }

    #[test]
    fn run_with_missing_service_name_reports_error() {
        let runner = TraceRunner::new();
        let case = minimal_case(serde_json::json!({
            "span_name": "process_payment",
            "otel_endpoint": "http://localhost:4318"
        }));
        let env = env_handle();
        let report = runner.run(&case, &env).expect("run succeeds");
        assert!(
            matches!(report.status, TestStatus::Error { .. }),
            "expected Error, got {:?}",
            report.status
        );
        let tail = report.evidence.stdout_tail.expect("evidence present");
        assert!(
            tail.contains("service_name is required"),
            "evidence should note missing field: {tail}"
        );
    }

    #[test]
    fn run_with_empty_spec_reports_error() {
        let runner = TraceRunner::new();
        let case = minimal_case(serde_json::json!({}));
        let env = env_handle();
        let report = runner.run(&case, &env).expect("run succeeds");
        assert!(
            matches!(report.status, TestStatus::Error { .. }),
            "expected Error for empty spec, got {:?}",
            report.status
        );
    }
}
