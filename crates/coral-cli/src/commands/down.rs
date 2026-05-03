//! `coral down [--volumes] [--env]`
//!
//! Tears down the named environment. Refuses to act on any environment
//! marked `production = true` unless `--yes` is passed (PRD §3.10
//! safety stance).

use anyhow::{Context, Result};
use clap::Args;
use coral_env::compose::{ComposeBackend, ComposeRuntime};
use coral_env::{DownOptions, EnvBackend, EnvPlan};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use crate::commands::common::resolve_project;
use crate::commands::env_resolve::{default_env_name, resolve_env};

#[derive(Args, Debug)]
pub struct DownArgs {
    #[arg(long)]
    pub env: Option<String>,

    /// Also remove named volumes.
    #[arg(long)]
    pub volumes: bool,

    /// Required to bring down a `production = true` environment.
    #[arg(long)]
    pub yes: bool,
}

pub fn run(args: DownArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let project = resolve_project(wiki_root)?;
    if project.environments_raw.is_empty() {
        anyhow::bail!("no [[environments]] declared in coral.toml");
    }
    let env_name = args.env.unwrap_or_else(|| default_env_name(&project));
    let spec = resolve_env(&project, &env_name)?;

    if spec.production && !args.yes {
        anyhow::bail!(
            "environment '{}' is marked production = true; pass --yes to confirm tear-down",
            env_name
        );
    }

    let mut repo_paths = BTreeMap::new();
    for repo in &project.repos {
        repo_paths.insert(repo.name.clone(), project.resolved_path(repo));
    }
    let plan = EnvPlan::from_spec(&spec, &project.root, &repo_paths)
        .map_err(|e| anyhow::anyhow!("building env plan: {}", e))?;

    let backend = ComposeBackend::new(ComposeRuntime::parse(&spec.compose_command));
    let opts = DownOptions {
        volumes: args.volumes,
    };
    backend
        .down(&plan, &opts)
        .context("bringing environment down")?;

    println!("✔ environment '{}' down", env_name);
    Ok(ExitCode::SUCCESS)
}
