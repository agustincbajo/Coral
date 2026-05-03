//! `coral verify [--env dev]` — sugar for "run all healthchecks".
//!
//! Returns exit-0 if every service with a declared healthcheck passes,
//! exit-1 otherwise. Liveness only (<30s budget). Distinct from
//! `coral test` (functional smoke / regression).

use anyhow::{Context, Result};
use clap::Args;
use coral_env::compose::{ComposeBackend, ComposeRuntime};
use coral_env::{EnvBackend, EnvHandle, EnvPlan};
use coral_test::{HealthcheckRunner, TestRunner, TestStatus};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use crate::commands::common::resolve_project;
use crate::commands::env_resolve::{default_env_name, resolve_env};

#[derive(Args, Debug)]
pub struct VerifyArgs {
    #[arg(long)]
    pub env: Option<String>,
}

pub fn run(args: VerifyArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let project = resolve_project(wiki_root)?;
    if project.environments_raw.is_empty() {
        anyhow::bail!("no [[environments]] declared in coral.toml");
    }
    let env_name = args.env.unwrap_or_else(|| default_env_name(&project));
    let spec = resolve_env(&project, &env_name)?;

    let mut repo_paths = BTreeMap::new();
    for repo in &project.repos {
        repo_paths.insert(repo.name.clone(), project.resolved_path(repo));
    }
    let plan = EnvPlan::from_spec(&spec, &project.root, &repo_paths)
        .map_err(|e| anyhow::anyhow!("building env plan: {}", e))?;

    let backend: Arc<dyn EnvBackend> = Arc::new(ComposeBackend::new(ComposeRuntime::parse(
        &spec.compose_command,
    )));
    let env_handle = synth_handle(&backend, &plan)?;
    let runner = HealthcheckRunner::new(backend.clone(), plan.clone(), spec.clone());

    let cases = HealthcheckRunner::cases_from_spec(&spec);
    if cases.is_empty() {
        println!("no [services.*.healthcheck] declared in environment '{env_name}'");
        return Ok(ExitCode::SUCCESS);
    }

    let mut all_pass = true;
    for case in &cases {
        let report = runner
            .run(case, &env_handle)
            .context("running healthcheck")?;
        match &report.status {
            TestStatus::Pass => println!("✔ {} ({}ms)", case.name, report.duration_ms),
            TestStatus::Skip { reason } => println!("⚠ {} skipped: {reason}", case.name),
            TestStatus::Fail { reason } => {
                all_pass = false;
                println!("✘ {}: {reason}", case.name);
            }
            TestStatus::Error { reason } => {
                all_pass = false;
                println!("⚠ {} errored: {reason}", case.name);
            }
        }
    }

    if all_pass {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::FAILURE)
    }
}

/// Build a synthetic `EnvHandle` for `verify`. Reuses the artifact
/// path the backend would write on `up`, but doesn't require an
/// actual `up` to have run — `status()` works on a stale or
/// never-up'd plan and returns the right state per service.
fn synth_handle(backend: &Arc<dyn EnvBackend>, plan: &EnvPlan) -> Result<EnvHandle> {
    Ok(EnvHandle {
        backend: backend.name().to_string(),
        artifact_hash: "verify".into(),
        artifact_path: plan.project_root.join(".coral/env/compose/verify.yml"),
        state: BTreeMap::new(),
    })
}
