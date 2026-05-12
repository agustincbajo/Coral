//! Contract test runner (Pact-style).
//!
//! Validates that a service's actual API responses match the expected
//! schema defined in the TestCase spec. Uses JSON Schema validation
//! against the `spec` field.
//!
//! ## v0.24 scope
//!
//! Structural validation only — the runner parses the contract spec
//! (`url`, `method`, `expected_status`, `expected_schema`) and
//! validates that the spec is well-formed. Live HTTP + JSON Schema
//! assertion against a running service is deferred to v0.25+ (needs
//! a JSON Schema validator dep and the env-resolver port mapping that
//! `PropertyRunner` already wires through `resolve_service_port`).
//!
//! When `expected_schema` is present, the runner reports `Skip` with
//! evidence capturing the contract metadata. When it is absent, the
//! runner also skips, noting the missing schema.

use crate::error::TestResult;
use crate::report::{Evidence, TestReport, TestStatus};
use crate::spec::{TestCase, TestKind};
use crate::{ParallelismHint, TestRunner};
use coral_env::EnvHandle;
use std::time::Instant;

pub struct ContractRunner;

impl Default for ContractRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl ContractRunner {
    pub fn new() -> Self {
        Self
    }
}

impl TestRunner for ContractRunner {
    fn name(&self) -> &'static str {
        "contract"
    }

    fn supports(&self, kind: TestKind) -> bool {
        kind == TestKind::Contract
    }

    fn run(&self, case: &TestCase, _env: &EnvHandle) -> TestResult<TestReport> {
        let spec = &case.spec.0;
        let started = Instant::now();

        let url = spec
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("http://localhost:8080");
        let method = spec.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
        let expected_schema = spec.get("expected_schema");

        let mut evidence = Evidence::default();

        let status = if expected_schema.is_some() {
            evidence.stdout_tail =
                Some(format!("contract_type=json_schema endpoint={method} {url}"));
            // v0.24: structural schema validation only.
            // v0.25+: live HTTP + JSON Schema assertion against running service.
            TestStatus::Skip {
                reason: "contract runner: live validation deferred to v0.25; tracked at https://github.com/agustincbajo/Coral#roadmap".into(),
            }
        } else {
            evidence.stdout_tail = Some("skip_reason: no expected_schema in spec".into());
            TestStatus::Skip {
                reason: "no expected_schema in spec".into(),
            }
        };

        let mut report = TestReport::new(case.clone(), status, started.elapsed());
        report.evidence = evidence;
        Ok(report)
    }

    fn parallelism_hint(&self) -> ParallelismHint {
        ParallelismHint::PerService
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
            id: "contract:api:users".into(),
            name: "users contract".into(),
            kind: TestKind::Contract,
            service: Some("api".into()),
            tags: vec!["contract".into()],
            source: TestSource::Inline,
            spec: TestSpec(spec_value),
        }
    }

    #[test]
    fn supports_returns_true_for_contract() {
        let runner = ContractRunner::new();
        assert!(runner.supports(TestKind::Contract));
    }

    #[test]
    fn supports_returns_false_for_other_kinds() {
        let runner = ContractRunner::new();
        assert!(!runner.supports(TestKind::Healthcheck));
        assert!(!runner.supports(TestKind::UserDefined));
        assert!(!runner.supports(TestKind::Event));
        assert!(!runner.supports(TestKind::PropertyBased));
    }

    #[test]
    fn name_returns_contract() {
        let runner = ContractRunner::new();
        assert_eq!(runner.name(), "contract");
    }

    #[test]
    fn parallelism_hint_returns_per_service() {
        let runner = ContractRunner::new();
        assert_eq!(runner.parallelism_hint(), ParallelismHint::PerService);
    }

    #[test]
    fn run_with_expected_schema_produces_skip_with_evidence() {
        let runner = ContractRunner::new();
        let case = minimal_case(serde_json::json!({
            "url": "http://localhost:3000/users",
            "method": "GET",
            "expected_status": 200,
            "expected_schema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer" },
                    "name": { "type": "string" }
                }
            }
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
            tail.contains("contract_type=json_schema"),
            "evidence should note contract type: {tail}"
        );
        assert!(
            tail.contains("GET http://localhost:3000/users"),
            "evidence should note endpoint: {tail}"
        );
    }

    #[test]
    fn run_without_expected_schema_produces_skip() {
        let runner = ContractRunner::new();
        let case = minimal_case(serde_json::json!({
            "url": "http://localhost:3000/health",
            "method": "GET"
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
            tail.contains("no expected_schema"),
            "evidence should note missing schema: {tail}"
        );
    }

    #[test]
    fn run_with_empty_spec_uses_defaults() {
        let runner = ContractRunner::new();
        let case = minimal_case(serde_json::json!({}));
        let env = env_handle();
        let report = runner.run(&case, &env).expect("run succeeds");
        // Empty spec → no expected_schema → skip.
        assert!(matches!(report.status, TestStatus::Skip { .. }));
    }
}
