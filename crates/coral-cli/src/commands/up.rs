//! `coral up [--service NAME]... [--env dev|ci] [--detach] [--build]`
//!
//! Brings up the selected environment via the configured backend
//! (compose in v0.17). Single-repo legacy users can still declare
//! `[[environments]]` in a `coral.toml` placed in their cwd; this
//! command always requires a manifest because environments are
//! manifest-only.

use anyhow::{Context, Result};
use clap::Args;
use coral_env::compose::{ComposeBackend, ComposeRuntime};
use coral_env::{EnvBackend, EnvPlan, UpOptions};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use crate::commands::common::resolve_project;
use crate::commands::env_resolve::{default_env_name, resolve_env};

#[derive(Args, Debug)]
pub struct UpArgs {
    /// Environment name (e.g. `dev`, `ci`). Defaults to the first
    /// declared `[[environments]]`.
    #[arg(long)]
    pub env: Option<String>,

    /// Limit `up` to specific services (repeatable).
    #[arg(long = "service", num_args = 1..)]
    pub services: Vec<String>,

    /// Run detached (compose `-d`). Default is true (matches the
    /// expectation of `coral up && coral verify`).
    #[arg(long, default_value_t = true)]
    pub detach: bool,

    /// Force rebuild before bringing up.
    #[arg(long)]
    pub build: bool,
}

pub fn run(args: UpArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let project = resolve_project(wiki_root)?;
    if project.environments_raw.is_empty() {
        anyhow::bail!(
            "no [[environments]] declared in coral.toml; add one before running `coral up`"
        );
    }
    let env_name = args.env.unwrap_or_else(|| default_env_name(&project));
    let spec = resolve_env(&project, &env_name)?;

    // Repo path map for `EnvPlan::from_spec` so `repo = "..."` resolves
    // to the actual checkout directory.
    let mut repo_paths = BTreeMap::new();
    for repo in &project.repos {
        repo_paths.insert(repo.name.clone(), project.resolved_path(repo));
    }
    let plan = EnvPlan::from_spec(&spec, &project.root, &repo_paths)
        .map_err(|e| anyhow::anyhow!("building env plan: {}", e))?;

    let backend = ComposeBackend::new(ComposeRuntime::parse(&spec.compose_command));
    let opts = UpOptions {
        services: args.services,
        detach: args.detach,
        build: args.build,
        watch: false,
    };
    let handle = backend
        .up(&plan, &opts)
        .context("bringing environment up")?;

    println!(
        "✔ environment '{}' up; project name '{}'",
        env_name, plan.project_name
    );
    println!("  artifact: {}", handle.artifact_path.display());
    println!("  hash: {}", handle.artifact_hash);
    Ok(ExitCode::SUCCESS)
}
