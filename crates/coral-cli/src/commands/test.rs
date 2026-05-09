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
use clap::Args;
use coral_env::compose::{ComposeBackend, ComposeRuntime};
use coral_env::{EnvBackend, EnvHandle, EnvPlan};
use coral_test::{
    HealthcheckRunner, HurlRunner, JunitOutput, TestCase, TestRunner, TestStatus, UserDefinedRunner,
};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use crate::commands::common::resolve_project;
use crate::commands::env_resolve::{default_env_name, resolve_env};

#[derive(Args, Debug)]
pub struct TestArgs {
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
}

#[derive(clap::ValueEnum, Clone, Debug, Copy, PartialEq, Eq)]
pub enum Emit {
    /// Translate to a k6 JavaScript load-test script.
    K6,
}

#[derive(clap::ValueEnum, Clone, Debug, Copy)]
pub enum KindArg {
    Healthcheck,
    UserDefined,
    Smoke,
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum Format {
    Markdown,
    Json,
    Junit,
}

pub fn run(args: TestArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
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

    let hc_runner = HealthcheckRunner::new(backend.clone(), plan.clone(), spec.clone());
    let ud_runner = UserDefinedRunner::new(backend.clone(), plan.clone())
        .with_update_snapshots(args.update_snapshots);

    let mut all_cases: Vec<(TestCase, &dyn TestRunner)> = Vec::new();
    if want_healthcheck {
        for case in HealthcheckRunner::cases_from_spec(&spec) {
            all_cases.push((case, &hc_runner));
        }
    }
    if want_user_defined {
        let yaml_pairs = UserDefinedRunner::discover_tests_dir(&project.root)
            .context("discovering YAML user-defined tests")?;
        for (case, _suite) in yaml_pairs {
            all_cases.push((case, &ud_runner));
        }
        let hurl_pairs =
            HurlRunner::discover(&project.root).context("discovering Hurl user-defined tests")?;
        for (case, _suite) in hurl_pairs {
            all_cases.push((case, &ud_runner));
        }
        if args.include_discovered {
            let openapi_cases = coral_test::discover_openapi_in_project(&project.root)
                .context("discovering OpenAPI tests")?;
            for d in openapi_cases {
                all_cases.push((d.case, &ud_runner));
            }
        }
    }

    // Apply --service / --tag filters.
    let services_filter: std::collections::BTreeSet<&str> =
        args.services.iter().map(String::as_str).collect();
    let tags_filter: std::collections::BTreeSet<&str> =
        args.tags.iter().map(String::as_str).collect();
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

    if filtered.is_empty() {
        println!("no test cases match the given filters");
        return Ok(ExitCode::SUCCESS);
    }

    let mut reports = Vec::with_capacity(filtered.len());
    let mut all_pass = true;
    for (case, runner) in filtered {
        let report = runner.run(&case, &env_handle).with_context(|| {
            format!(
                "running case '{}' via runner '{}'",
                case.name,
                runner.name()
            )
        })?;
        if matches!(
            report.status,
            TestStatus::Fail { .. } | TestStatus::Error { .. }
        ) {
            all_pass = false;
        }
        reports.push(report);
    }

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
    args: &TestArgs,
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
