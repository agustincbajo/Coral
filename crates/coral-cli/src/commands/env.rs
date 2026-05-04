//! `coral env <subcommand>` — environment introspection.
//!
//! v0.17 wave 2 ships `status`, `logs`, `exec`. The remaining
//! subcommands (`attach`, `reset`, `port-forward`, `open`, `prune`,
//! `devcontainer emit`) follow in v0.17.x as the CLI surface
//! stabilizes.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use coral_env::compose::{ComposeBackend, ComposeRuntime};
use coral_env::{
    EnvBackend, EnvPlan, EnvStatus, ExecOptions, HealthState, LogsOptions, ServiceState,
    ServiceStatus,
};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use crate::commands::common::resolve_project;
use crate::commands::env_resolve::{default_env_name, resolve_env};

#[derive(Args, Debug)]
pub struct EnvArgs {
    #[command(subcommand)]
    pub command: EnvCmd,
}

#[derive(Subcommand, Debug)]
pub enum EnvCmd {
    /// Show running services + health from the configured backend.
    Status(StatusArgs),
    /// Print recent logs for one service.
    Logs(LogsArgs),
    /// Run a command inside a service container.
    Exec(ExecArgs),
    /// Convert a `docker-compose.yml` into a `coral.toml`
    /// `[[environments]]` block. Output is advisory — review before
    /// committing.
    Import(ImportArgs),
}

#[derive(Args, Debug)]
pub struct ImportArgs {
    /// Path to the existing `docker-compose.yml` (or compatible).
    pub compose_path: std::path::PathBuf,
    /// Name to give the resulting environment block. Default: `dev`.
    #[arg(long, default_value = "dev")]
    pub env: String,
    /// Write the result to `<dir>/coral.env-import.toml` instead of
    /// stdout. Use `--out` to override the path. The flag exists so
    /// `coral env import compose.yml --write` is the obvious shape.
    #[arg(long)]
    pub write: bool,
    /// Override the destination path (only meaningful with `--write`).
    /// Default: `coral.env-import.toml` under the current dir.
    #[arg(long)]
    pub out: Option<std::path::PathBuf>,
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    #[arg(long)]
    pub env: Option<String>,
    /// Output format.
    #[arg(long, default_value = "markdown")]
    pub format: Format,
}

#[derive(Args, Debug)]
pub struct LogsArgs {
    #[arg(long)]
    pub env: Option<String>,
    pub service: String,
    /// Show only the last N lines.
    #[arg(long)]
    pub tail: Option<usize>,
}

#[derive(Args, Debug)]
pub struct ExecArgs {
    #[arg(long)]
    pub env: Option<String>,
    pub service: String,
    /// The command + arguments to run.
    #[arg(trailing_var_arg = true, num_args = 1..)]
    pub cmd: Vec<String>,
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum Format {
    Markdown,
    Json,
}

pub fn run(args: EnvArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    match args.command {
        EnvCmd::Status(a) => status(a, wiki_root),
        EnvCmd::Logs(a) => logs(a, wiki_root),
        EnvCmd::Exec(a) => exec(a, wiki_root),
        EnvCmd::Import(a) => import(a),
    }
}

fn status(args: StatusArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let (backend, plan, _env_name) = build_backend(wiki_root, args.env.as_deref())?;
    let report = backend
        .status(&plan)
        .context("querying environment status")?;
    match args.format {
        Format::Markdown => print_status_markdown(&report),
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&report).context("serializing status")?
        ),
    }
    Ok(ExitCode::SUCCESS)
}

fn logs(args: LogsArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let (backend, plan, _) = build_backend(wiki_root, args.env.as_deref())?;
    let opts = LogsOptions {
        follow: false,
        tail: args.tail,
    };
    let lines = backend
        .logs(&plan, &args.service, &opts)
        .with_context(|| format!("reading logs for service '{}'", args.service))?;
    for line in lines {
        println!("{}", line.line);
    }
    Ok(ExitCode::SUCCESS)
}

fn exec(args: ExecArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let (backend, plan, _) = build_backend(wiki_root, args.env.as_deref())?;
    let output = backend
        .exec(&plan, &args.service, &args.cmd, &ExecOptions::default())
        .with_context(|| format!("exec in service '{}'", args.service))?;
    if !output.stdout.is_empty() {
        print!("{}", output.stdout);
    }
    if !output.stderr.is_empty() {
        eprint!("{}", output.stderr);
    }
    if output.exit_code == 0 {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::FAILURE)
    }
}

fn import(args: ImportArgs) -> Result<ExitCode> {
    let yaml = std::fs::read_to_string(&args.compose_path)
        .with_context(|| format!("reading compose file at {}", args.compose_path.display()))?;
    let result = coral_env::import::import_compose_to_toml(&yaml, &args.env)
        .map_err(|e| anyhow::anyhow!("importing compose file: {e}"))?;

    if args.write {
        let dest = args
            .out
            .unwrap_or_else(|| std::path::PathBuf::from("coral.env-import.toml"));
        std::fs::write(&dest, &result.toml)
            .with_context(|| format!("writing {}", dest.display()))?;
        eprintln!("✔ wrote {}", dest.display());
        eprintln!(
            "  paste the contents into your `coral.toml` (top-level), then review the\n\
             `# TODO:` comments before running `coral up`."
        );
    } else {
        print!("{}", result.toml);
    }

    if !result.warnings.is_empty() {
        eprintln!();
        eprintln!(
            "⚠ {} warning(s) — fields Coral didn't translate cleanly:",
            result.warnings.len()
        );
        for w in &result.warnings {
            eprintln!("  - {w}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn build_backend(
    wiki_root: Option<&Path>,
    env_arg: Option<&str>,
) -> Result<(ComposeBackend, EnvPlan, String)> {
    let project = resolve_project(wiki_root)?;
    if project.environments_raw.is_empty() {
        anyhow::bail!("no [[environments]] declared in coral.toml");
    }
    let env_name = env_arg
        .map(str::to_string)
        .unwrap_or_else(|| default_env_name(&project));
    let spec = resolve_env(&project, &env_name)?;
    let mut repo_paths = BTreeMap::new();
    for repo in &project.repos {
        repo_paths.insert(repo.name.clone(), project.resolved_path(repo));
    }
    let plan = EnvPlan::from_spec(&spec, &project.root, &repo_paths)
        .map_err(|e| anyhow::anyhow!("building env plan: {}", e))?;
    let backend = ComposeBackend::new(ComposeRuntime::parse(&spec.compose_command));
    Ok((backend, plan, env_name))
}

fn print_status_markdown(report: &EnvStatus) {
    println!("| service | state | health | restarts | published ports |");
    println!("|---------|-------|--------|----------|-----------------|");
    for s in &report.services {
        let ports = if s.published_ports.is_empty() {
            "—".to_string()
        } else {
            s.published_ports
                .iter()
                .map(|p| format!("{}->{}", p.host_port, p.container_port))
                .collect::<Vec<_>>()
                .join(",")
        };
        println!(
            "| {} | {} | {} | {} | {} |",
            s.name,
            describe_state(&s.state),
            describe_health(&s.health),
            s.restarts,
            ports
        );
    }
}

fn describe_state(s: &ServiceState) -> &'static str {
    match s {
        ServiceState::Pending => "pending",
        ServiceState::Starting => "starting",
        ServiceState::Running => "running",
        ServiceState::Crashed => "crashed",
        ServiceState::Stopped => "stopped",
        ServiceState::Unknown => "unknown",
    }
}

fn describe_health(h: &HealthState) -> &'static str {
    match h {
        HealthState::Pass => "pass",
        HealthState::Fail => "fail",
        HealthState::Unknown => "unknown",
    }
}

#[allow(dead_code)]
fn _retain_servicestatus_referenced(_s: &ServiceStatus) {}
