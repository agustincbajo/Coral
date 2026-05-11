//! `run_test_suite_filtered` — shared TestCase pipeline (v0.23.1).
//!
//! Both `coral test` (one-shot) and `coral monitor up` (cron-loop)
//! call this function. Pre-v0.23.1 the body lived inline in
//! `coral-cli::commands::test::run`; pulling it here makes the monitor
//! loop a thin wrapper over the same runner-build / discover / filter
//! / execute pipeline so the two surfaces can never drift on which
//! cases they pick or how they run.
//!
//! The function is **side-effect-light**: no stdout, no exit codes,
//! no file writes — it returns the per-case `TestReport`s and lets
//! the caller render / persist as it sees fit. `coral test` formats
//! to markdown / JSON / JUnit; `coral monitor up` collapses to a
//! `MonitorRun` summary and appends to JSONL.

use crate::error::TestResult;
use crate::healthcheck_runner::HealthcheckRunner;
use crate::hurl_runner::HurlRunner;
use crate::property_runner::{PropertyRunner, cases_from_property_specs};
use crate::recorded_runner::RecordedRunner;
use crate::report::{TestReport, TestStatus};
use crate::spec::{TestCase, TestKind, TestSource, TestSpec};
use crate::user_defined_runner::UserDefinedRunner;
use crate::{ParallelismHint, TestRunner, discover_openapi_in_project};
use coral_env::{EnvBackend, EnvHandle, EnvPlan, EnvironmentSpec};
use rayon::prelude::*;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

/// Filters applied to the discovered TestCase set before execution.
/// Mirrors the `coral test --service / --tag / --kind` flags so a
/// `[[environments.<env>.monitors]]` block can express the same set
/// of cases the user would have passed on the command line.
#[derive(Debug, Clone, Default)]
pub struct TestFilters {
    /// Services to include. Empty = include all.
    pub services: Vec<String>,
    /// Tags to include (any-of). Empty = include all.
    pub tags: Vec<String>,
    /// Kinds to include. Empty = include all (Healthcheck + UserDefined).
    pub kinds: Vec<TestKind>,
    /// When true, also discover from OpenAPI specs in repos.
    pub include_discovered: bool,
    /// v0.23.3: CLI override for `--iterations` on `coral test`. When
    /// `Some(N)`, every property-based TestCase runs N iterations
    /// regardless of `[[environments.<env>.property_tests]].iterations`.
    /// `None` = honor the manifest, fall back to the default (50).
    pub property_iterations: Option<u32>,
    /// v0.23.3: CLI override for `--seed` on `coral test`. Same
    /// precedence: `Some(N)` beats the manifest, `None` honors it.
    pub property_seed: Option<u64>,
}

/// Build runners, discover + filter + execute the resulting TestCase
/// set against the live env, return per-case reports.
///
/// Errors propagate from runner construction or per-case execution
/// (the report itself encodes pass/fail/skip/error — only catastrophic
/// failures bubble up here).
///
/// The `update_snapshots` flag flows into the `UserDefinedRunner` —
/// when set, the runner overwrites snapshot files instead of failing
/// on mismatch. Monitor loops should pass `false` (we never want a
/// monitor loop to silently rewrite snapshots).
pub fn run_test_suite_filtered(
    project_root: &Path,
    spec: &EnvironmentSpec,
    backend: Arc<dyn EnvBackend>,
    plan: &EnvPlan,
    env_handle: &EnvHandle,
    filters: &TestFilters,
    update_snapshots: bool,
) -> TestResult<Vec<TestReport>> {
    let want_healthcheck = filters.kinds.is_empty()
        || filters
            .kinds
            .iter()
            .any(|k| matches!(k, TestKind::Healthcheck));
    let want_user_defined = filters.kinds.is_empty()
        || filters
            .kinds
            .iter()
            .any(|k| matches!(k, TestKind::UserDefined));
    // v0.23.2: recorded replay. Empty kinds list does NOT include
    // recorded by default — replay is opt-in (the user has to commit
    // captured YAMLs first). Including it on `--kind recorded`
    // explicitly keeps `coral test` behavior backward-compatible:
    // pre-v0.23.2 invocations don't suddenly find new cases.
    let want_recorded = filters
        .kinds
        .iter()
        .any(|k| matches!(k, TestKind::Recorded));
    // v0.23.3: property-based fuzzing from OpenAPI specs. Same
    // opt-in stance as recorded — empty kinds list does NOT include
    // it. `coral test --kind property-based` is the explicit gate.
    let want_property_based = filters
        .kinds
        .iter()
        .any(|k| matches!(k, TestKind::PropertyBased));

    let hc_runner = HealthcheckRunner::new(backend.clone(), plan.clone(), spec.clone());
    let ud_runner = UserDefinedRunner::new(backend.clone(), plan.clone())
        .with_update_snapshots(update_snapshots);
    let rec_runner = RecordedRunner::new(backend.clone(), plan.clone(), spec.clone());
    let prop_runner = PropertyRunner::new(backend.clone(), plan.clone(), spec.clone());

    let mut all_cases: Vec<(TestCase, &dyn TestRunner)> = Vec::new();
    if want_healthcheck {
        for case in HealthcheckRunner::cases_from_spec(spec) {
            all_cases.push((case, &hc_runner));
        }
    }
    if want_user_defined {
        let yaml_pairs = UserDefinedRunner::discover_tests_dir(project_root)?;
        for (case, _suite) in yaml_pairs {
            all_cases.push((case, &ud_runner));
        }
        let hurl_pairs = HurlRunner::discover(project_root)?;
        for (case, _suite) in hurl_pairs {
            all_cases.push((case, &ud_runner));
        }
        if filters.include_discovered {
            let openapi_cases = discover_openapi_in_project(project_root)?;
            for d in openapi_cases {
                all_cases.push((d.case, &ud_runner));
            }
        }
    }
    if want_recorded {
        for case in RecordedRunner::cases_from_project(project_root)? {
            all_cases.push((case, &rec_runner));
        }
    }
    if want_property_based {
        for case in cases_from_property_specs(
            spec,
            project_root,
            filters.property_iterations,
            filters.property_seed,
        )? {
            all_cases.push((case, &prop_runner));
        }
    }

    // Apply --service / --tag filters.
    let services_filter: BTreeSet<&str> = filters.services.iter().map(String::as_str).collect();
    let tags_filter: BTreeSet<&str> = filters.tags.iter().map(String::as_str).collect();
    let filtered: Vec<(TestCase, &dyn TestRunner)> = all_cases
        .into_iter()
        .filter(|(case, _)| {
            if !services_filter.is_empty() {
                let svc = case.service.as_deref().unwrap_or("");
                if !services_filter.contains(svc) {
                    return false;
                }
            }
            if !tags_filter.is_empty()
                && !case.tags.iter().any(|t| tags_filter.contains(t.as_str()))
            {
                return false;
            }
            true
        })
        .collect();

    // Partition by parallelism hint.
    let (isolated, rest): (Vec<_>, Vec<_>) = filtered
        .into_iter()
        .partition(|(_, runner)| runner.parallelism_hint() == ParallelismHint::Isolated);
    let (sequential, per_service): (Vec<_>, Vec<_>) = rest
        .into_iter()
        .partition(|(_, runner)| runner.parallelism_hint() == ParallelismHint::Sequential);

    // Run isolated cases in parallel.
    let isolated_reports: Vec<TestReport> = isolated
        .par_iter()
        .map(|(case, runner)| runner.run(case, env_handle))
        .collect::<TestResult<Vec<_>>>()?;

    // Run sequential cases in order.
    let mut sequential_reports = Vec::with_capacity(sequential.len());
    for (case, runner) in &sequential {
        let report = runner.run(case, env_handle)?;
        sequential_reports.push(report);
    }

    // Run per-service cases: parallel across services, sequential within.
    let mut service_groups: HashMap<String, Vec<&(TestCase, &dyn TestRunner)>> = HashMap::new();
    for item in &per_service {
        let svc = item.0.service.clone().unwrap_or_default();
        service_groups.entry(svc).or_default().push(item);
    }
    let per_service_reports: Vec<TestReport> = service_groups
        .into_par_iter()
        .flat_map(|(_, cases)| {
            cases
                .into_iter()
                .map(|(case, runner)| runner.run(case, env_handle))
                .collect::<Vec<_>>()
        })
        .collect::<TestResult<Vec<_>>>()?;

    let mut reports = isolated_reports;
    reports.extend(sequential_reports);
    reports.extend(per_service_reports);

    // v0.31.1: reserved-kind transparency. The five kinds without a
    // discovery path (LlmGenerated; plus Contract/Event/Trace/E2eBrowser
    // whose runners exist but don't auto-discover cases yet) would
    // otherwise silently produce zero reports when explicitly
    // requested via `--kind <reserved>` — indistinguishable from "no
    // matching cases". Emit one synthetic Skip per requested reserved
    // kind so the user sees a clear "deferred" signal with a tracking
    // URL. Implemented kinds are skipped here (their normal runner
    // pipeline above already emitted their reports).
    for kind in filters.kinds.iter().copied() {
        if let Some(reason) = reserved_kind_skip_reason(kind) {
            // De-dup: only emit a synthetic skip when the runner
            // pipeline above produced zero reports for this kind.
            if !reports.iter().any(|r| r.case.kind == kind) {
                reports.push(synthetic_reserved_skip(kind, reason));
            }
        }
    }

    Ok(reports)
}

/// Returns `Some(reason)` for `TestKind` variants that are declared in
/// the schema but have no live execution path in v0.31.0. `None` for
/// kinds whose runner is wired (Healthcheck, UserDefined, PropertyBased,
/// Recorded). Resolution path for each reserved kind is tracked in the
/// README §Roadmap.
fn reserved_kind_skip_reason(kind: TestKind) -> Option<&'static str> {
    match kind {
        TestKind::Healthcheck
        | TestKind::UserDefined
        | TestKind::PropertyBased
        | TestKind::Recorded => None,
        TestKind::LlmGenerated => Some(
            "kind 'llm_generated' is reserved schema, no runner implementation; \
             tracked at https://github.com/agustincbajo/Coral#roadmap",
        ),
        TestKind::Contract => Some(
            "kind 'contract' runner: live validation deferred to v0.25; \
             tracked at https://github.com/agustincbajo/Coral#roadmap",
        ),
        TestKind::Event => Some(
            "kind 'event' runner: live validation deferred to v0.25; \
             tracked at https://github.com/agustincbajo/Coral#roadmap",
        ),
        TestKind::Trace => Some(
            "kind 'trace' runner: live OTLP query deferred to future release; \
             tracked at https://github.com/agustincbajo/Coral#roadmap",
        ),
        TestKind::E2eBrowser => Some(
            "kind 'e2e_browser' runner: Playwright execution deferred to v0.26; \
             tracked at https://github.com/agustincbajo/Coral#roadmap",
        ),
    }
}

/// Build a placeholder `TestReport` for a reserved kind that no runner
/// would otherwise emit. The synthesized TestCase carries enough
/// metadata (id, name, kind) for downstream formatters (markdown/JSON/
/// JUnit) to render a coherent row.
fn synthetic_reserved_skip(kind: TestKind, reason: &str) -> TestReport {
    let slug = match kind {
        TestKind::LlmGenerated => "llm_generated",
        TestKind::Contract => "contract",
        TestKind::Event => "event",
        TestKind::Trace => "trace",
        TestKind::E2eBrowser => "e2e_browser",
        _ => "reserved",
    };
    let case = TestCase {
        id: format!("reserved-kind:{slug}"),
        name: format!("[reserved] kind = {slug}"),
        kind,
        service: None,
        tags: vec!["reserved".to_string()],
        source: TestSource::Inline,
        spec: TestSpec::empty(),
    };
    TestReport::new(
        case,
        TestStatus::Skip {
            reason: reason.to_string(),
        },
        std::time::Duration::from_millis(0),
    )
}
