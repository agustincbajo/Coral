//! `MockTestRunner` — scripted FIFO outputs + call recorder for tests.
//!
//! Same shape as `coral_runner::MockRunner` and `coral_env::MockBackend`.
//!
//! v0.36 clippy: see `coral_runner::mock` for the test-fixture allow.
#![allow(clippy::unwrap_used)]

use crate::error::TestResult;
use crate::report::{TestReport, TestStatus};
use crate::spec::{TestCase, TestKind};
use crate::{ParallelismHint, TestRunner};
use coral_env::EnvHandle;
use std::sync::Mutex;
use std::time::Duration;

pub struct MockTestRunner {
    inner: Mutex<MockState>,
}

#[derive(Default)]
struct MockState {
    pub statuses: std::collections::VecDeque<TestStatus>,
    pub recorded: Vec<TestCase>,
}

impl MockTestRunner {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(MockState::default()),
        }
    }

    pub fn push_status(&self, status: TestStatus) {
        self.inner.lock().unwrap().statuses.push_back(status);
    }

    pub fn recorded(&self) -> Vec<TestCase> {
        self.inner.lock().unwrap().recorded.clone()
    }
}

impl Default for MockTestRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl TestRunner for MockTestRunner {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn supports(&self, _kind: TestKind) -> bool {
        true
    }

    fn run(&self, case: &TestCase, _env: &EnvHandle) -> TestResult<TestReport> {
        let mut state = self.inner.lock().unwrap();
        state.recorded.push(case.clone());
        let status = state.statuses.pop_front().unwrap_or(TestStatus::Pass);
        drop(state);
        Ok(TestReport::new(
            case.clone(),
            status,
            Duration::from_millis(0),
        ))
    }

    fn parallelism_hint(&self) -> ParallelismHint {
        ParallelismHint::Isolated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{TestCase, TestSource, TestSpec};
    use std::collections::BTreeMap;

    fn case() -> TestCase {
        TestCase {
            id: "x".into(),
            name: "x".into(),
            kind: TestKind::UserDefined,
            service: None,
            tags: vec![],
            source: TestSource::Inline,
            spec: TestSpec::empty(),
        }
    }

    fn env_handle() -> EnvHandle {
        EnvHandle {
            backend: "mock".into(),
            artifact_hash: "x".into(),
            artifact_path: std::path::PathBuf::from("/tmp"),
            state: BTreeMap::new(),
        }
    }

    #[test]
    fn mock_returns_pass_by_default() {
        let r = MockTestRunner::new();
        let report = r.run(&case(), &env_handle()).unwrap();
        assert!(matches!(report.status, TestStatus::Pass));
    }

    #[test]
    fn mock_returns_scripted_statuses_in_order() {
        let r = MockTestRunner::new();
        r.push_status(TestStatus::Fail {
            reason: "first".into(),
        });
        r.push_status(TestStatus::Pass);
        let report1 = r.run(&case(), &env_handle()).unwrap();
        let report2 = r.run(&case(), &env_handle()).unwrap();
        assert!(matches!(report1.status, TestStatus::Fail { .. }));
        assert!(matches!(report2.status, TestStatus::Pass));
    }

    #[test]
    fn mock_records_invocations() {
        let r = MockTestRunner::new();
        r.run(&case(), &env_handle()).unwrap();
        r.run(&case(), &env_handle()).unwrap();
        assert_eq!(r.recorded().len(), 2);
    }
}
