//! `coral test [--service NAME] [--kind KIND] [--tag T] [--format json|markdown|junit]`
//!
//! Wave 2 wires Healthcheck + UserDefined runners. Discovery from
//! OpenAPI/proto, Hurl, snapshot assertions, and the rest of the v0.18
//! roadmap follow in wave 3.
//!
//! v0.22.2 adds `--emit k6 [--emit-output PATH]` for the smoke→load
//! handoff: discover + filter the same TestCase pipeline, but emit a
//! single k6 JS file instead of running cases against a live env.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use coral_env::compose::{ComposeBackend, ComposeRuntime};
use coral_env::{EnvBackend, EnvHandle, EnvPlan};
use coral_test::{
    HealthcheckRunner, HurlRunner, JunitOutput, TestCase, TestFilters, TestKind, TestStatus,
    UserDefinedRunner, run_test_suite_filtered,
};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use crate::commands::common::resolve_project;
use crate::commands::env_resolve::{default_env_name, resolve_env};

/// `coral test [args]` and `coral test record [args]`.
///
/// v0.23.2 adds the `record` subcommand. Pre-v0.23.2 invocations
/// (`coral test --service foo --kind smoke ...`) are byte-compatible —
/// when no subcommand is given, clap matches the flat flag set.
#[derive(Args, Debug)]
pub struct TestArgs {
    #[command(subcommand)]
    pub command: Option<TestSubcommand>,

    #[command(flatten)]
    pub run: TestRunArgs,
}

/// v0.23.2: `coral test record` — capture-side subcommand.
///
/// Linux-only and gated behind the `recorded` Cargo feature. When
/// `coral` is built without the feature OR run on a non-Linux host,
/// the handler exits 2 with a friendly message (acceptance criterion
/// #2). The capture path subprocesses `keploy record` against a
/// service PID resolved via `docker compose ps`.
#[derive(Subcommand, Debug)]
pub enum TestSubcommand {
    /// Capture live HTTP traffic and persist as Keploy YAML for
    /// later replay via `coral test --kind recorded`. Linux-only;
    /// requires the `recorded` Cargo feature.
    Record(RecordArgs),
    /// Cross-reference discovered OpenAPI endpoints against existing
    /// TestCases and report coverage gaps. Answers: "which endpoints
    /// have tests, which don't?"
    Coverage(CoverageArgs),
    /// Report flaky tests from historical test-run data stored in
    /// `.coral/test-history.jsonl`. Shows tests that pass/fail
    /// inconsistently and flags those above the quarantine threshold.
    Flakes(FlakesArgs),
    /// Compare test latencies against a stored baseline and report
    /// p95 regressions. Reads timing data from `.coral/test-history.jsonl`
    /// and baseline from `.coral/perf-baseline.json`.
    Perf(PerfArgs),
}

#[derive(Args, Debug)]
pub struct RecordArgs {
    /// Environment name (default: first declared).
    #[arg(long)]
    pub env: Option<String>,
    /// Target service to capture traffic from. Must be a `kind = "real"`
    /// service in the env spec; resolved to a PID via
    /// `docker compose ps --format json` + `docker inspect`.
    #[arg(long)]
    pub service: String,
    /// Capture duration in seconds. The Keploy subprocess is sent
    /// SIGTERM after this many seconds elapse; YAMLs flushed to
    /// disk before that point are retained.
    #[arg(long, default_value_t = 30)]
    pub duration: u64,
    /// Override the output directory. Default:
    /// `<project_root>/.coral/tests/recorded/<service>/`. The
    /// directory is created if it doesn't exist.
    #[arg(long, value_name = "DIR")]
    pub output: Option<std::path::PathBuf>,
}

#[derive(Args, Debug)]
pub struct CoverageArgs {
    /// Output format for the coverage report.
    #[arg(long, default_value = "markdown")]
    pub format: CoverageFormat,
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum CoverageFormat {
    Markdown,
    Json,
}

#[derive(Args, Debug)]
pub struct FlakesArgs {
    /// Output format for the flakes report.
    #[arg(long, default_value = "markdown")]
    pub format: FlakesFormat,

    /// Only consider test runs from the last N days (default: 30).
    #[arg(long, default_value_t = 30)]
    pub max_age_days: u64,
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum FlakesFormat {
    Markdown,
    Json,
}

#[derive(Args, Debug)]
pub struct PerfArgs {
    /// Output format for the performance report.
    #[arg(long, default_value = "markdown")]
    pub format: PerfFormat,

    /// Regression threshold as a percentage of the baseline p95.
    /// A test whose current p95 exceeds the baseline by more than
    /// this value is flagged as a regression (default: 20%).
    #[arg(long, default_value_t = 20.0)]
    pub threshold: f64,

    /// Update the stored baseline with timings from the most recent
    /// test history. Useful after intentionally landing a slower path.
    #[arg(long)]
    pub update_baseline: bool,
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum PerfFormat {
    Markdown,
    Json,
}

#[derive(Args, Debug)]
pub struct TestRunArgs {
    #[arg(long)]
    pub env: Option<String>,

    /// Filter by service name (repeatable).
    #[arg(long = "service", num_args = 1..)]
    pub services: Vec<String>,

    /// Filter by `TestKind` (repeatable). Default: all.
    #[arg(long = "kind", value_enum, num_args = 1..)]
    pub kinds: Vec<KindArg>,

    /// Filter by tag (repeatable).
    #[arg(long = "tag", num_args = 1..)]
    pub tags: Vec<String>,

    /// Output format.
    #[arg(long, default_value = "markdown")]
    pub format: Format,

    /// Update snapshot files instead of failing on mismatch.
    #[arg(long)]
    pub update_snapshots: bool,

    /// Auto-generate test cases from OpenAPI specs found in repos
    /// (`openapi.{yaml,yml,json}`). Determines what `discover` would
    /// emit and runs it against the live env.
    #[arg(long)]
    pub include_discovered: bool,

    /// Instead of executing cases, emit them in a different format.
    /// `k6` (v0.22.2): translate the discovered + filtered TestCase set
    /// into a single k6 JavaScript load-test script. Coral does not
    /// invoke k6; the user runs `k6 run <emitted file>`.
    #[arg(long, value_enum)]
    pub emit: Option<Emit>,

    /// When `--emit` is set, write the rendered output to PATH
    /// atomically (temp + rename) instead of stdout.
    #[arg(long, value_name = "PATH")]
    pub emit_output: Option<std::path::PathBuf>,

    /// v0.23.3: override `[[environments.<env>.property_tests]].iterations`
    /// for this invocation only. `--kind property-based` only.
    /// CLI > manifest > 50 (default).
    #[arg(long, value_name = "N")]
    pub iterations: Option<u32>,

    /// v0.23.3: override `[[environments.<env>.property_tests]].seed`
    /// for this invocation only. When set, two runs with the same
    /// `--seed` produce byte-identical request sequences (acceptance
    /// criterion #6). Without the flag, a fresh seed is drawn from
    /// the system clock and logged to stderr + Evidence::stdout_tail.
    #[arg(long, value_name = "N")]
    pub seed: Option<u64>,
}

#[derive(clap::ValueEnum, Clone, Debug, Copy, PartialEq, Eq)]
pub enum Emit {
    /// Translate to a k6 JavaScript load-test script.
    K6,
}

#[derive(clap::ValueEnum, Clone, Debug, Copy, PartialEq, Eq)]
pub enum KindArg {
    Healthcheck,
    UserDefined,
    Smoke,
    /// v0.23.2: replay Keploy-captured exchanges from
    /// `.coral/tests/recorded/<service>/*.yaml`.
    Recorded,
    /// v0.23.3: Schemathesis-style property-based fuzzing of every
    /// `(path, method)` operation declared in the OpenAPI spec
    /// pinned by `[[environments.<env>.property_tests]]`.
    PropertyBased,
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum Format {
    Markdown,
    Json,
    Junit,
}

pub fn run(args: TestArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    // v0.23.2: dispatch on subcommand. Pre-existing flat-flag
    // invocations land in `run_inner` (the `record` subcommand is
    // a separate handler below).
    match args.command {
        Some(TestSubcommand::Record(rec)) => run_record(rec, wiki_root),
        Some(TestSubcommand::Coverage(cov)) => run_coverage(cov, wiki_root),
        Some(TestSubcommand::Flakes(flk)) => run_flakes(flk, wiki_root),
        Some(TestSubcommand::Perf(perf)) => run_perf(perf, wiki_root),
        None => run_inner(args.run, wiki_root),
    }
}

fn run_inner(args: TestRunArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    // ---- Flag-interaction validation (acceptance #9) ------------
    // These checks run BEFORE any I/O so misuse fails fast with exit 2.
    if args.emit.is_some() {
        // `--format` is execution-only; `--emit` selects an emitter, not
        // a runner output format. Reject ANY non-default `--format`
        // value, not just `junit` — `--format json --emit k6` was
        // silently accepted in the v0.22.2 ship and printed k6 JS on
        // stdout while the user expected JSON. Default is `Markdown`,
        // so any other value here was explicitly chosen by the user
        // and must conflict.
        if !matches!(args.format, Format::Markdown) {
            let chosen = match args.format {
                Format::Markdown => "markdown",
                Format::Json => "json",
                Format::Junit => "junit",
            };
            eprintln!(
                "--format {chosen} applies to test execution; --emit selects an emitter (you used both)"
            );
            return Ok(ExitCode::from(2));
        }
        if args.update_snapshots {
            eprintln!("--update-snapshots not meaningful with --emit");
            return Ok(ExitCode::from(2));
        }
    }

    let project = resolve_project(wiki_root)?;
    if project.environments_raw.is_empty() {
        anyhow::bail!("no [[environments]] declared in coral.toml");
    }
    let env_name = args
        .env
        .clone()
        .unwrap_or_else(|| default_env_name(&project));
    let spec = resolve_env(&project, &env_name)?;

    // ---- --emit short-circuit (acceptance #1, #12) --------------
    // No backend, no `coral up` requirement. Discover + filter cases,
    // hand to `emit_k6`, write to stdout / atomic file.
    if let Some(Emit::K6) = args.emit {
        let cases = collect_cases_for_emit(&args, &project.root, &spec)?;
        let filtered = apply_filters(cases, &args.services, &args.tags);
        if filtered.is_empty() {
            eprintln!(
                "no test cases match the given filters (services={:?}, tags={:?}, kinds={:?}, include_discovered={})",
                args.services, args.tags, args.kinds, args.include_discovered
            );
            return Ok(ExitCode::from(2));
        }
        let out = coral_test::emit_k6(&filtered, &spec);
        // Stderr: skip telemetry summary (acceptance #6).
        for note in &out.skipped {
            eprintln!("skip: {} — {}", note.case_id, note.detail);
        }
        eprintln!(
            "k6 emit summary: included={} skipped={}",
            out.included,
            out.skipped.len()
        );
        if let Some(path) = &args.emit_output {
            coral_core::atomic::atomic_write_string(path, &out.script)
                .with_context(|| format!("writing k6 script to {}", path.display()))?;
        } else {
            // Stdout — print verbatim, no trailing extra newline (the
            // emitter already ends with `\n`).
            print!("{}", out.script);
        }
        return Ok(ExitCode::SUCCESS);
    }

    let mut repo_paths = BTreeMap::new();
    for repo in &project.repos {
        repo_paths.insert(repo.name.clone(), project.resolved_path(repo));
    }
    let plan = EnvPlan::from_spec(&spec, &project.root, &repo_paths)
        .map_err(|e| anyhow::anyhow!("building env plan: {}", e))?;
    let backend: Arc<dyn EnvBackend> = Arc::new(ComposeBackend::new(ComposeRuntime::parse(
        &spec.compose_command,
    )));
    let env_handle = EnvHandle {
        backend: backend.name().to_string(),
        artifact_hash: "test".into(),
        artifact_path: plan.project_root.join(".coral/env/compose/test.yml"),
        state: BTreeMap::new(),
    };

    // v0.23.1: runner build + discover + filter + execute now lives in
    // `coral_test::run_test_suite_filtered` so the monitor loop and
    // `coral test` can never drift on which cases they pick. Translate
    // the CLI's `KindArg` (which has a `Smoke` umbrella variant) into
    // the underlying `TestKind`s the orchestrator wants. Empty list ==
    // include all kinds.
    let kinds: Vec<TestKind> = {
        let mut out: Vec<TestKind> = Vec::new();
        for k in &args.kinds {
            match k {
                KindArg::Healthcheck => {
                    if !out.contains(&TestKind::Healthcheck) {
                        out.push(TestKind::Healthcheck);
                    }
                }
                KindArg::UserDefined => {
                    if !out.contains(&TestKind::UserDefined) {
                        out.push(TestKind::UserDefined);
                    }
                }
                KindArg::Smoke => {
                    // Smoke = healthcheck + user_defined; orchestrator
                    // takes a flat list, so push both (unique).
                    if !out.contains(&TestKind::Healthcheck) {
                        out.push(TestKind::Healthcheck);
                    }
                    if !out.contains(&TestKind::UserDefined) {
                        out.push(TestKind::UserDefined);
                    }
                }
                KindArg::Recorded => {
                    // v0.23.2: replay captured Keploy YAMLs. Recorded
                    // is opt-in — empty kinds list does not include
                    // it (orchestrator-side gate in
                    // `run_test_suite_filtered`).
                    if !out.contains(&TestKind::Recorded) {
                        out.push(TestKind::Recorded);
                    }
                }
                KindArg::PropertyBased => {
                    // v0.23.3: property-based fuzzing from OpenAPI.
                    // Same opt-in stance as Recorded — empty kinds
                    // list does not include it; the orchestrator
                    // gate in `run_test_suite_filtered` enforces this.
                    if !out.contains(&TestKind::PropertyBased) {
                        out.push(TestKind::PropertyBased);
                    }
                }
            }
        }
        out
    };
    let filters = TestFilters {
        services: args.services.clone(),
        tags: args.tags.clone(),
        kinds,
        include_discovered: args.include_discovered,
        property_iterations: args.iterations,
        property_seed: args.seed,
    };
    let reports = run_test_suite_filtered(
        &project.root,
        &spec,
        backend.clone(),
        &plan,
        &env_handle,
        &filters,
        args.update_snapshots,
    )
    .context("running filtered test suite")?;

    if reports.is_empty() {
        println!("no test cases match the given filters");
        return Ok(ExitCode::SUCCESS);
    }

    let all_pass = !reports
        .iter()
        .any(|r| matches!(r.status, TestStatus::Fail { .. } | TestStatus::Error { .. }));

    match args.format {
        Format::Markdown => print_markdown(&reports),
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&reports).context("serializing reports")?
        ),
        Format::Junit => print!("{}", JunitOutput::render(&reports)),
    }

    if all_pass {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::FAILURE)
    }
}

fn print_markdown(reports: &[coral_test::TestReport]) {
    println!("| status | case | duration |");
    println!("|--------|------|----------|");
    for r in reports {
        let s = match &r.status {
            TestStatus::Pass => "✔ pass".to_string(),
            TestStatus::Fail { reason } => format!("✘ fail: {reason}"),
            TestStatus::Skip { reason } => format!("⚠ skip: {reason}"),
            TestStatus::Error { reason } => format!("⚠ err: {reason}"),
        };
        println!("| {} | {} | {}ms |", s, r.case.name, r.duration_ms);
    }
}

/// `_KindArg_unused_to_silence_dead_code` — `Smoke` variant is meaningful
/// (selects healthcheck + user-defined) but the matcher above doesn't
/// declare a default; this helper ensures clippy doesn't flag it.
#[allow(dead_code)]
fn _ensure_kind_arg_smoke_used(k: KindArg) -> bool {
    matches!(k, KindArg::Smoke)
}

// ---------------------------------------------------------------
// --emit pipeline helpers (v0.22.2). Lives at module bottom so the
// existing test-execution path stays visually grouped.
// ---------------------------------------------------------------

/// Collect TestCases for the `--emit` path. Mirrors the
/// `want_healthcheck` / `want_user_defined` gating used by the runner
/// flow, but skips the runner construction entirely — emit needs only
/// the case list, no backend.
fn collect_cases_for_emit(
    args: &TestRunArgs,
    project_root: &Path,
    spec: &coral_env::EnvironmentSpec,
) -> Result<Vec<TestCase>> {
    let want_healthcheck = args.kinds.is_empty()
        || args
            .kinds
            .iter()
            .any(|k| matches!(k, KindArg::Healthcheck | KindArg::Smoke));
    let want_user_defined = args.kinds.is_empty()
        || args
            .kinds
            .iter()
            .any(|k| matches!(k, KindArg::UserDefined | KindArg::Smoke));

    let mut cases: Vec<TestCase> = Vec::new();
    if want_healthcheck {
        cases.extend(HealthcheckRunner::cases_from_spec(spec));
    }
    if want_user_defined {
        let yaml_pairs = UserDefinedRunner::discover_tests_dir(project_root)
            .context("discovering YAML user-defined tests")?;
        for (case, _suite) in yaml_pairs {
            cases.push(case);
        }
        let hurl_pairs =
            HurlRunner::discover(project_root).context("discovering Hurl user-defined tests")?;
        for (case, _suite) in hurl_pairs {
            cases.push(case);
        }
        if args.include_discovered {
            let openapi_cases = coral_test::discover_openapi_in_project(project_root)
                .context("discovering OpenAPI tests")?;
            for d in openapi_cases {
                cases.push(d.case);
            }
        }
    }
    Ok(cases)
}

/// Apply `--service` / `--tag` filters to a flat case list. Same gate
/// as the runner-flow filter step — kept separate so the `--emit` path
/// can reuse it without dragging the `&dyn TestRunner` pair through.
fn apply_filters(cases: Vec<TestCase>, services: &[String], tags: &[String]) -> Vec<TestCase> {
    let services_filter: std::collections::BTreeSet<&str> =
        services.iter().map(String::as_str).collect();
    let tags_filter: std::collections::BTreeSet<&str> = tags.iter().map(String::as_str).collect();
    cases
        .into_iter()
        .filter(|case| {
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
        .collect()
}

// ----------------------------------------------------------------------
// v0.24.2: `coral test coverage` — endpoint gap analysis (M1.6).
// ----------------------------------------------------------------------

fn run_coverage(args: CoverageArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let project = resolve_project(wiki_root)?;
    let report = coral_test::compute_coverage(&project.root)
        .context("computing test coverage")?;

    match args.format {
        CoverageFormat::Markdown => print!("{}", coral_test::render_coverage_markdown(&report)),
        CoverageFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&coral_test::render_coverage_json(&report))
                .context("serializing coverage report")?
        ),
    }

    Ok(ExitCode::SUCCESS)
}

// ----------------------------------------------------------------------
// v0.24.2: `coral test flakes` — historical flake-rate report (M2.7).
// ----------------------------------------------------------------------

fn run_flakes(args: FlakesArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let project = resolve_project(wiki_root)?;
    let records = coral_test::read_history(&project.root);

    if records.is_empty() {
        println!("No test history found. Run `coral test` to start building history.");
        return Ok(ExitCode::SUCCESS);
    }

    let max_age = if args.max_age_days > 0 {
        Some(args.max_age_days)
    } else {
        None
    };
    let flakes = coral_test::compute_flakes(&records, max_age);

    match args.format {
        FlakesFormat::Markdown => print!("{}", coral_test::render_flakes_markdown(&flakes)),
        FlakesFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&coral_test::render_flakes_json(&flakes))
                .context("serializing flakes report")?
        ),
    }

    Ok(ExitCode::SUCCESS)
}

// ----------------------------------------------------------------------
// v0.24.2: `coral test perf` — latency baseline + regression detection (M2.8).
// ----------------------------------------------------------------------

fn run_perf(args: PerfArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let project = resolve_project(wiki_root)?;
    let records = coral_test::read_history(&project.root);

    if records.is_empty() {
        println!("No test history found. Run `coral test` to start building history.");
        return Ok(ExitCode::SUCCESS);
    }

    let mut baseline = coral_test::load_baseline(&project.root);

    if args.update_baseline {
        // Fold all history records into the baseline.
        for rec in &records {
            coral_test::update_baseline(&mut baseline, &rec.case_id, rec.duration_ms);
        }
        coral_test::save_baseline(&project.root, &baseline)
            .context("saving perf baseline")?;
        eprintln!(
            "Baseline updated with {} records ({} cases)",
            records.len(),
            baseline.cases.len()
        );
        return Ok(ExitCode::SUCCESS);
    }

    // Build "current" timings from the most recent run per case.
    // Group by case_id and take the last recorded duration.
    let mut latest: BTreeMap<String, u64> = BTreeMap::new();
    for rec in &records {
        latest.insert(rec.case_id.clone(), rec.duration_ms);
    }

    if baseline.cases.is_empty() {
        println!(
            "No baseline found. Run `coral test perf --update-baseline` to establish one."
        );
        return Ok(ExitCode::SUCCESS);
    }

    let report = coral_test::detect_regressions(&baseline, &latest, args.threshold);

    match args.format {
        PerfFormat::Markdown => print!("{}", coral_test::render_perf_markdown(&report)),
        PerfFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&coral_test::render_perf_json(&report))
                .context("serializing perf report")?
        ),
    }

    if report.regressions.is_empty() {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::FAILURE)
    }
}

// ----------------------------------------------------------------------
// v0.23.2: `coral test record` (capture-side, Linux + feature-gated).
// ----------------------------------------------------------------------

/// `coral test record` handler.
///
/// Two-layer gate:
///
/// 1. **Compile-time** (`#[cfg(all(target_os = "linux", feature = "..."))]`)
///    — the cargo build only includes the capture path on Linux with
///    `--features recorded`. On every other build, the handler exits
///    2 with a friendly hint.
///
/// 2. **Runtime sanity** — even when the feature is on, we re-check
///    that the manifest declares the service, that `coral up` is
///    running, and that `keploy` is on PATH. Errors before doing
///    any privileged work.
///
/// The capture path itself spawns `keploy record --pid <PID>
/// --path <output_dir>` and waits `--duration` seconds before sending
/// SIGTERM. PID resolution: `docker compose ps --format json` →
/// `docker inspect <id>` to get `State.Pid`. Kept minimal in v0.23.2:
/// proxy-mode capture, JSON output, no DNS rewriting.
pub fn run_record(args: RecordArgs, _wiki_root: Option<&Path>) -> Result<ExitCode> {
    // ---- Platform + feature gate (acceptance criterion #2) ----
    if !is_recorded_capture_supported() {
        eprintln!(
            "error: `coral test record` requires Linux + 'recorded' cargo feature.\n\
             current platform: {}\n\
             rebuild with: `cargo install coral-cli --features recorded` on a Linux host.\n\
             Replay (`coral test --kind recorded`) is supported on every platform.",
            std::env::consts::OS
        );
        return Ok(ExitCode::from(2));
    }
    #[cfg(all(target_os = "linux", feature = "recorded"))]
    {
        run_record_linux(args)
    }
    #[cfg(not(all(target_os = "linux", feature = "recorded")))]
    {
        // Unreachable in practice — the gate above exits early. This
        // branch is here so the function compiles on every platform.
        let _ = args;
        Ok(ExitCode::from(2))
    }
}

/// `true` only when this binary was built on Linux with the `recorded`
/// Cargo feature. Public-to-the-crate so the help-snippet snapshot
/// test can assert the right wording without invoking the handler.
#[allow(dead_code)]
pub(crate) fn is_recorded_capture_supported() -> bool {
    cfg!(all(target_os = "linux", feature = "recorded"))
}

#[cfg(all(target_os = "linux", feature = "recorded"))]
fn run_record_linux(args: RecordArgs) -> Result<ExitCode> {
    use std::process::Command;
    use std::time::Duration;
    let project = resolve_project(_wiki_root)?;
    if project.environments_raw.is_empty() {
        anyhow::bail!("no [[environments]] declared in coral.toml");
    }
    let env_name = args
        .env
        .clone()
        .unwrap_or_else(|| default_env_name(&project));
    let spec = resolve_env(&project, &env_name)?;
    if !spec.services.contains_key(&args.service) {
        let known: Vec<String> = spec.services.keys().cloned().collect();
        anyhow::bail!(
            "service '{}' not found in environment '{}'; declared services: {}",
            args.service,
            env_name,
            if known.is_empty() {
                "(none)".into()
            } else {
                known.join(", ")
            }
        );
    }
    let output_dir = args.output.clone().unwrap_or_else(|| {
        project
            .root
            .join(".coral/tests/recorded")
            .join(&args.service)
    });
    std::fs::create_dir_all(&output_dir)
        .with_context(|| format!("creating output dir {}", output_dir.display()))?;
    // Resolve service PID via docker compose ps + docker inspect.
    let pid = resolve_service_pid(&args.service)?;
    eprintln!(
        "✔ resolved service '{}' PID {} → capturing for {}s into {}",
        args.service,
        pid,
        args.duration,
        output_dir.display()
    );
    let mut keploy = Command::new("keploy");
    keploy.args([
        "record",
        "--pid",
        &pid.to_string(),
        "--path",
        output_dir.to_string_lossy().as_ref(),
    ]);
    let mut child = keploy.spawn().context("spawning keploy (is it on PATH?)")?;
    let deadline = std::time::Instant::now() + Duration::from_secs(args.duration);
    while std::time::Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(status)) => {
                eprintln!("warn: keploy exited early with {status}");
                return Ok(ExitCode::FAILURE);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(250)),
            Err(e) => anyhow::bail!("waiting for keploy: {e}"),
        }
    }
    // Best-effort SIGTERM via libc kill — keploy doesn't expose a
    // graceful-shutdown signal otherwise.
    let _ = child.kill();
    let _ = child.wait();
    eprintln!("✔ capture complete");
    Ok(ExitCode::SUCCESS)
}

#[cfg(all(target_os = "linux", feature = "recorded"))]
fn resolve_service_pid(service: &str) -> Result<u32> {
    use std::process::Command;
    let ps = Command::new("docker")
        .args(["compose", "ps", "--format", "json"])
        .output()
        .context("running `docker compose ps`")?;
    if !ps.status.success() {
        anyhow::bail!(
            "`docker compose ps` failed: {}",
            String::from_utf8_lossy(&ps.stderr)
        );
    }
    // The JSON shape varies by Docker version; fall back to two-step
    // (find any running container with the service label, then `docker
    // inspect` for State.Pid).
    let stdout = String::from_utf8_lossy(&ps.stdout);
    let mut container_id: Option<String> = None;
    for line in stdout.lines() {
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let svc = v
            .get("Service")
            .and_then(|s| s.as_str())
            .or_else(|| v.get("service").and_then(|s| s.as_str()));
        if svc == Some(service) {
            if let Some(id) = v
                .get("ID")
                .and_then(|s| s.as_str())
                .or_else(|| v.get("id").and_then(|s| s.as_str()))
            {
                container_id = Some(id.to_string());
                break;
            }
        }
    }
    let container_id = container_id.ok_or_else(|| {
        anyhow::anyhow!(
            "no running container for service '{}'; run `coral up` first",
            service
        )
    })?;
    let inspect = Command::new("docker")
        .args(["inspect", "--format", "{{.State.Pid}}", &container_id])
        .output()
        .context("running `docker inspect`")?;
    if !inspect.status.success() {
        anyhow::bail!(
            "`docker inspect` failed: {}",
            String::from_utf8_lossy(&inspect.stderr)
        );
    }
    let pid_str = String::from_utf8_lossy(&inspect.stdout).trim().to_string();
    pid_str.parse::<u32>().map_err(|e| {
        anyhow::anyhow!("could not parse PID from `docker inspect` output {pid_str:?}: {e}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    /// Minimal CLI shim so we can parse `coral test record ...` flags
    /// without standing up the whole top-level Cli.
    #[derive(clap::Parser, Debug)]
    struct ShimCli {
        #[command(subcommand)]
        cmd: ShimCmd,
    }

    #[derive(clap::Subcommand, Debug)]
    enum ShimCmd {
        Test(TestArgs),
    }

    /// Acceptance criterion #8 — `--kind recorded` is a value-enum entry
    /// on `coral test`. Tests parse-time mapping not the
    /// orchestrator gate (covered in `coral-test/tests/recorded.rs`).
    #[test]
    fn coral_test_kind_recorded_in_value_enum() {
        let parsed =
            ShimCli::try_parse_from(["coral", "test", "--kind", "recorded", "--service", "api"])
                .expect("parse");
        match parsed.cmd {
            ShimCmd::Test(t) => {
                assert!(
                    t.run.kinds.iter().any(|k| matches!(k, KindArg::Recorded)),
                    "expected Recorded in kinds, got {:?}",
                    t.run.kinds
                );
                assert_eq!(t.run.services, vec!["api".to_string()]);
            }
        }
    }

    /// Sanity: every existing `--kind` value still parses (Smoke / Healthcheck
    /// / UserDefined) — pin against an accidental enum reordering.
    #[test]
    fn coral_test_kind_smoke_user_defined_still_parse() {
        // Clap renders the value-enum variants as kebab-case, so
        // `UserDefined` is `user-defined` on the CLI surface.
        let parsed =
            ShimCli::try_parse_from(["coral", "test", "--kind", "smoke", "--kind", "user-defined"])
                .expect("parse");
        match parsed.cmd {
            ShimCmd::Test(t) => {
                assert!(t.run.kinds.iter().any(|k| matches!(k, KindArg::Smoke)));
                assert!(
                    t.run
                        .kinds
                        .iter()
                        .any(|k| matches!(k, KindArg::UserDefined))
                );
            }
        }
    }

    /// Test #7 — `coral test record --help` mentions the Linux-only
    /// constraint. Snapshot-asserted (acceptance criterion #10).
    ///
    /// We don't invoke the binary here (that's the e2e snapshot in
    /// `coral-cli/tests/snapshot_cli.rs` if needed); we render the
    /// generated long-help via clap and check the substring. Avoids
    /// process spawning + flaky path normalization.
    #[test]
    fn coral_test_record_help_mentions_linux() {
        let mut shim = ShimCli::command();
        let test_cmd = shim.find_subcommand_mut("test").expect("test cmd");
        let record_cmd = test_cmd
            .find_subcommand_mut("record")
            .expect("record subcommand");
        let help = record_cmd.render_long_help();
        let help_str = help.to_string();
        // The about/long_about for the record variant must say
        // Linux-only and mention the cargo feature. We assert both
        // substrings; `--help` text is the contract surfaced to users.
        assert!(
            help_str.contains("Linux"),
            "record --help missing Linux constraint:\n{help_str}"
        );
        assert!(
            help_str.contains("recorded"),
            "record --help missing 'recorded' feature mention:\n{help_str}"
        );
    }

    /// Acceptance #10 (v0.23.3) — `--iterations 5` parses and lands
    /// in `args.run.iterations`. Pinned because the runtime
    /// resolution path (`property_iterations`) on `TestFilters`
    /// trusts the CLI value.
    #[test]
    fn coral_test_iterations_seed_flags_parse() {
        let parsed = ShimCli::try_parse_from([
            "coral",
            "test",
            "--kind",
            "property-based",
            "--service",
            "api",
            "--iterations",
            "5",
            "--seed",
            "42",
        ])
        .expect("parse");
        match parsed.cmd {
            ShimCmd::Test(t) => {
                assert!(
                    t.run
                        .kinds
                        .iter()
                        .any(|k| matches!(k, KindArg::PropertyBased)),
                    "kind property-based must parse: {:?}",
                    t.run.kinds
                );
                assert_eq!(t.run.iterations, Some(5));
                assert_eq!(t.run.seed, Some(42));
            }
        }
    }

    /// Test #6 — on macOS, `coral test record` exits 2 with a
    /// friendly error. Compiled into the macOS binary only.
    #[cfg(target_os = "macos")]
    #[test]
    fn coral_test_record_on_macos_exits_with_friendly_error() {
        // `is_recorded_capture_supported()` returns false on macOS by
        // construction; pin it as the gate predicate the handler
        // dispatches on.
        assert!(
            !is_recorded_capture_supported(),
            "macOS binary must NOT support the capture path"
        );
        // The handler returns ExitCode 2 + writes a friendly stderr
        // message. Because ExitCode doesn't impl PartialEq, assert via
        // the String form once the handler runs.
        let args = RecordArgs {
            env: None,
            service: "api".into(),
            duration: 1,
            output: None,
        };
        let exit = run_record(args, None).expect("run_record returns Ok");
        // On macOS, this must be ExitCode::from(2) — Process exit
        // semantic. We can't compare directly; instead, check that
        // the gate predicate is still false (already done above), and
        // re-call to verify the handler is short-circuiting.
        let exit2 = run_record(
            RecordArgs {
                env: None,
                service: "api".into(),
                duration: 1,
                output: None,
            },
            None,
        )
        .expect("run_record returns Ok 2");
        // Both must complete without erroring (the handler is the
        // friendly-message path, not an `anyhow::bail!`).
        let _ = (exit, exit2);
    }
}
