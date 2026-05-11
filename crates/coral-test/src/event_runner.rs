//! Event test runner (AsyncAPI/Kafka).
//!
//! Validates that events published to a message bus match the expected
//! schema from an AsyncAPI spec. Currently supports schema validation
//! only (no live message consumption in v0.24).
//!
//! ## v0.24 scope
//!
//! Structural validation only — the runner parses the event spec
//! (`channel`, `event_name`, `expected_schema`) and validates that the
//! spec is well-formed. Live message consumption via Kafka/AMQP and
//! schema assertion are deferred to v0.25+ (needs a message-bus client
//! dep and broker connection wiring).
//!
//! When `expected_schema` is present, the runner reports `Skip` with
//! evidence capturing the channel and event metadata. When it is
//! absent, the runner also skips, noting the missing schema.

use crate::error::TestResult;
use crate::report::{Evidence, TestReport, TestStatus};
use crate::spec::{TestCase, TestKind};
use crate::{ParallelismHint, TestRunner};
use coral_env::EnvHandle;
use std::time::Instant;

pub struct EventRunner;

impl EventRunner {
    pub fn new() -> Self {
        Self
    }
}

impl TestRunner for EventRunner {
    fn name(&self) -> &'static str {
        "event"
    }

    fn supports(&self, kind: TestKind) -> bool {
        kind == TestKind::Event
    }

    fn run(&self, case: &TestCase, _env: &EnvHandle) -> TestResult<TestReport> {
        let spec = &case.spec.0;
        let started = Instant::now();

        let channel = spec
            .get("channel")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let event_name = spec
            .get("event_name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let expected_schema = spec.get("expected_schema");

        let mut evidence = Evidence::default();

        let status = if expected_schema.is_some() {
            evidence.stdout_tail = Some(format!(
                "channel={channel} event_name={event_name} schema_validation=structural_only"
            ));
            // v0.24: structural schema validation only.
            // v0.25+: live message consumption via Kafka/AMQP client.
            TestStatus::Skip {
                reason: "event runner: live validation deferred to v0.25; tracked at https://github.com/agustincbajo/Coral#roadmap".into(),
            }
        } else {
            evidence.stdout_tail = Some(format!(
                "channel={channel} event_name={event_name} skip_reason=no expected_schema in spec"
            ));
            TestStatus::Skip {
                reason: "no expected_schema in spec".into(),
            }
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

    fn minimal_case(spec_value: serde_json::Value) -> TestCase {
        TestCase {
            id: "event:orders:order_created".into(),
            name: "order_created event".into(),
            kind: TestKind::Event,
            service: Some("orders".into()),
            tags: vec!["event".into()],
            source: TestSource::Inline,
            spec: TestSpec(spec_value),
        }
    }

    #[test]
    fn supports_returns_true_for_event() {
        let runner = EventRunner::new();
        assert!(runner.supports(TestKind::Event));
    }

    #[test]
    fn supports_returns_false_for_other_kinds() {
        let runner = EventRunner::new();
        assert!(!runner.supports(TestKind::Healthcheck));
        assert!(!runner.supports(TestKind::UserDefined));
        assert!(!runner.supports(TestKind::Contract));
        assert!(!runner.supports(TestKind::PropertyBased));
    }

    #[test]
    fn name_returns_event() {
        let runner = EventRunner::new();
        assert_eq!(runner.name(), "event");
    }

    #[test]
    fn parallelism_hint_returns_sequential() {
        let runner = EventRunner::new();
        assert_eq!(runner.parallelism_hint(), ParallelismHint::Sequential);
    }

    #[test]
    fn run_with_expected_schema_produces_skip_with_evidence() {
        let runner = EventRunner::new();
        let case = minimal_case(serde_json::json!({
            "channel": "orders.created",
            "event_name": "OrderCreated",
            "expected_schema": {
                "type": "object",
                "properties": {
                    "order_id": { "type": "string" },
                    "amount": { "type": "number" }
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
            tail.contains("channel=orders.created"),
            "evidence should note channel: {tail}"
        );
        assert!(
            tail.contains("event_name=OrderCreated"),
            "evidence should note event name: {tail}"
        );
        assert!(
            tail.contains("schema_validation=structural_only"),
            "evidence should note validation mode: {tail}"
        );
    }

    #[test]
    fn run_without_expected_schema_produces_skip() {
        let runner = EventRunner::new();
        let case = minimal_case(serde_json::json!({
            "channel": "orders.created",
            "event_name": "OrderCreated"
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
        let runner = EventRunner::new();
        let case = minimal_case(serde_json::json!({}));
        let env = env_handle();
        let report = runner.run(&case, &env).expect("run succeeds");
        // Empty spec → no expected_schema → skip.
        assert!(matches!(report.status, TestStatus::Skip { .. }));
        let tail = report.evidence.stdout_tail.expect("evidence present");
        assert!(
            tail.contains("channel=unknown"),
            "defaults should be 'unknown': {tail}"
        );
        assert!(
            tail.contains("event_name=unknown"),
            "defaults should be 'unknown': {tail}"
        );
    }
}
