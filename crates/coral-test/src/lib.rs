//! Coral testing layer (v0.18+).
//!
//! Pluggable trait family for functional smoke / contract / property /
//! recorded / event / trace / browser tests against a running
//! environment. v0.18 wave 1 ships the trait, the type model, and a
//! `MockRunner`; the four MVP runners (`Healthcheck`, `UserDefined`,
//! `Hurl`, `Discovery`) follow in v0.18 wave 2 with their own feature
//! flags.
//!
//! Mirrors the shape of `coral-env::EnvBackend` and `coral-runner::Runner`
//! deliberately — same `Send + Sync`, `thiserror` errors, `Mock*` for
//! tests.

pub mod browser_runner;
pub mod contract_check;
pub mod contract_runner;
pub mod coverage;
pub mod discover;
pub mod emit_k6;
pub mod error;
pub mod event_runner;
pub mod healthcheck_runner;
pub mod history;
pub mod hurl_runner;
pub mod mock;
pub mod orchestrator;
pub mod perf;
pub mod probe;
pub mod property_runner;
pub mod recorded_runner;
pub mod report;
pub mod semantic_diff;
pub mod spec;
pub mod trace_runner;
pub mod user_defined_runner;
pub mod walk_tests;

pub use browser_runner::BrowserRunner;
pub use contract_check::{
    ContractReport, Finding as ContractFinding, FindingKind as ContractFindingKind,
    Severity as ContractSeverity, check_project as check_contracts,
    render_report_json as render_contract_json, render_report_markdown as render_contract_markdown,
};
pub use contract_runner::ContractRunner;
pub use coverage::{
    CoverageReport, Endpoint as CoverageEndpoint, compute_coverage,
    render_json as render_coverage_json, render_markdown as render_coverage_markdown,
};
pub use discover::{DiscoveredCase, discover_openapi_in_project};
pub use emit_k6::{EmitOutput, SkipNote, SkipReason, emit_k6};
pub use error::{TestError, TestResult};
pub use event_runner::EventRunner;
pub use healthcheck_runner::HealthcheckRunner;
pub use history::{
    FlakeEntry, TestRecord, append_records, compute_flakes, read_history,
    render_json as render_flakes_json, render_markdown as render_flakes_markdown,
};
pub use hurl_runner::HurlRunner;
pub use mock::MockTestRunner;
pub use orchestrator::{TestFilters, run_test_suite_filtered};
pub use perf::{
    LatencyStats, PerfBaseline, PerfRegression, PerfReport, detect_regressions, load_baseline,
    render_json as render_perf_json, render_markdown as render_perf_markdown, save_baseline,
    update_baseline,
};
pub use property_runner::{
    DEFAULT_ITERATIONS as PROPERTY_DEFAULT_ITERATIONS, PropertyRunner, PropertyTestCaseSpec,
    cases_from_property_specs, json_schema_strategy,
};
pub use recorded_runner::{KeployTestCase, RecordedRunner, discover_recorded};
pub use report::{Evidence, JunitOutput, TestReport, TestStatus};
pub use semantic_diff::{
    BreakingChange, DiffSeverity, SemanticDiffResult, diff_openapi, diff_protobuf, diff_schema,
};
pub use spec::{TestCase, TestKind, TestSource, TestSpec};
pub use trace_runner::TraceRunner;
pub use user_defined_runner::UserDefinedRunner;

use coral_env::EnvHandle;
use std::path::PathBuf;
use std::time::Duration;

/// The pluggable trait. Each concrete runner declares which
/// `TestKind`s it supports and exposes a `run()` method that produces
/// a `TestReport`. Every runner is `Send + Sync` so the orchestration
/// layer can fan out across threads / rayon.
pub trait TestRunner: Send + Sync {
    fn name(&self) -> &'static str;
    fn supports(&self, kind: TestKind) -> bool;
    fn run(&self, case: &TestCase, env: &EnvHandle) -> TestResult<TestReport>;

    /// Auto-discover `TestCase`s without LLM. Default impl returns
    /// nothing; runners that read OpenAPI / proto / asyncapi specs
    /// override this. Used by `coral test discover` (v0.18 wave 2).
    fn discover(&self, _project_root: &std::path::Path) -> TestResult<Vec<TestCase>> {
        Ok(Vec::new())
    }

    /// Hint to the orchestration layer about how to schedule cases
    /// from this runner. Healthcheck is `Isolated` (parallel-safe);
    /// stateful flows use `Sequential`; a UserDefined suite that
    /// targets one service uses `PerService`.
    fn parallelism_hint(&self) -> ParallelismHint {
        ParallelismHint::Isolated
    }

    /// Where to read/write snapshots for `expect.snapshot` assertions.
    /// `None` = snapshots not supported.
    fn snapshot_dir(&self) -> Option<PathBuf> {
        None
    }

    /// `true` when this runner can capture live traffic to author new
    /// `TestCase`s (e.g. Keploy-style). v0.20+ feature.
    fn supports_record(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelismHint {
    Isolated,
    Sequential,
    PerService,
}

/// Run-level configuration shared across all runners.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    pub services: Vec<String>,
    pub tags: Vec<String>,
    pub kinds: Vec<TestKind>,
    pub update_snapshots: bool,
    pub parallelism: Option<usize>,
    pub timeout: Option<Duration>,
}
