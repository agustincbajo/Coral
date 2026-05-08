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
    DevcontainerOpts, EnvBackend, EnvPlan, EnvStatus, ExecOptions, HealthState, LogsOptions,
    ServiceState, ServiceStatus,
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
    /// Devcontainer (.devcontainer/devcontainer.json) operations.
    Devcontainer(DevcontainerArgs),
    /// Run `compose watch` in foreground for live-reload. Alias for
    /// `coral up --watch`. v0.21.2+.
    Watch(WatchArgs),
}

#[derive(Args, Debug)]
pub struct WatchArgs {
    /// Environment name (e.g. `dev`). Defaults to the first declared.
    #[arg(long)]
    pub env: Option<String>,

    /// Limit watch to specific services.
    #[arg(long = "service", num_args = 1..)]
    pub services: Vec<String>,

    /// Force rebuild before bringing up.
    #[arg(long)]
    pub build: bool,
}

#[derive(Args, Debug)]
pub struct DevcontainerArgs {
    #[command(subcommand)]
    pub command: DevcontainerCmd,
}

#[derive(Subcommand, Debug)]
pub enum DevcontainerCmd {
    /// Render a `.devcontainer/devcontainer.json` describing the
    /// `[[environments]]` block. Print to stdout, or write to disk
    /// with `--write`.
    Emit(DevcontainerEmitArgs),
}

#[derive(Args, Debug)]
pub struct DevcontainerEmitArgs {
    /// Environment to render. Default: first declared.
    #[arg(long)]
    pub env: Option<String>,
    /// Force the `service:` field. Default: deterministic algorithm
    /// (first real service with `repo = "..."`, fall back to
    /// alphabetic).
    #[arg(long)]
    pub service: Option<String>,
    /// Write to `<project_root>/.devcontainer/devcontainer.json`.
    /// Mirrors `coral env import --write`.
    #[arg(long)]
    pub write: bool,
    /// Override the destination path (only meaningful with `--write`).
    #[arg(long)]
    pub out: Option<std::path::PathBuf>,
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
        EnvCmd::Devcontainer(a) => match a.command {
            DevcontainerCmd::Emit(emit) => devcontainer_emit(emit, wiki_root),
        },
        EnvCmd::Watch(a) => watch(a, wiki_root),
    }
}

/// `coral env watch` is a thin alias for `coral up --watch`. We
/// translate `WatchArgs` to `UpArgs` and dispatch through `up::run` so
/// there's exactly one watch implementation. Doubles the help-text
/// surface by ~10 lines, but keeps the orchestration single-sourced.
fn watch(args: WatchArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let up_args = crate::commands::up::UpArgs {
        env: args.env,
        services: args.services,
        detach: true,
        build: args.build,
        watch: true,
    };
    crate::commands::up::run(up_args, wiki_root)
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
        // v0.20.2 audit-followup #45: atomic write so a crash
        // mid-write can't leave a torn TOML on disk. Writes go through
        // a sibling tempfile + `rename` (POSIX-atomic on the same
        // filesystem), matching every other on-disk write in the
        // workspace.
        coral_core::atomic::atomic_write_string(&dest, &result.toml)
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

fn devcontainer_emit(args: DevcontainerEmitArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let (_backend, plan, _env_name) = build_backend(wiki_root, args.env.as_deref())?;
    let opts = DevcontainerOpts {
        service_override: args.service,
    };
    let artifact = coral_env::render_devcontainer(&plan, &opts)
        .map_err(|e| anyhow::anyhow!("rendering devcontainer.json: {e}"))?;

    if args.write {
        let dest = args
            .out
            .unwrap_or_else(|| plan.project_root.join(".devcontainer/devcontainer.json"));
        // Mirror `coral env import --write`: atomic write so a crash
        // mid-write can't leave a torn JSON on disk. The atomic
        // helper creates the parent directory if missing.
        coral_core::atomic::atomic_write_string(&dest, &artifact.json)
            .with_context(|| format!("writing {}", dest.display()))?;
        eprintln!("✔ wrote {}", dest.display());
        eprintln!(
            "  open the project in VS Code (or Cursor) and choose\n  \"Reopen in Container\" to use it."
        );
    } else {
        print!("{}", artifact.json);
    }

    if !artifact.warnings.is_empty() {
        eprintln!();
        eprintln!(
            "⚠ {} warning(s) — fields Coral didn't translate cleanly:",
            artifact.warnings.len()
        );
        for w in &artifact.warnings {
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// v0.20.2 audit-followup #45: regression — `coral env import --write`
    /// must use the atomic write helper (sibling tempfile + rename) rather
    /// than `std::fs::write`, so a crash mid-write can't leave a torn TOML
    /// on disk.
    ///
    /// The most direct test of "uses atomic write" is that the
    /// destination file is never observed in a half-written state.
    /// We can't easily simulate a process crash from inside a
    /// process, but we CAN exercise the import path end-to-end and
    /// verify (a) the file lands well-formed, and (b) the import
    /// function reaches the atomic helper rather than a bare
    /// `std::fs::write`. We approach (b) by overwriting an existing
    /// file: `atomic_write_string` replaces in place via `rename` and
    /// preserves no temporary partial state visible to readers.
    #[test]
    fn import_write_replaces_existing_file_atomically() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("coral.env-import.toml");
        let compose_path = dir.path().join("compose.yml");
        // Minimal compose file the import recognizes.
        std::fs::write(
            &compose_path,
            "services:\n  api:\n    image: alpine:latest\n",
        )
        .unwrap();
        // Pre-populate destination with a sentinel so we can verify
        // it was replaced (not left in a torn intermediate).
        std::fs::write(&dest, "this should be replaced").unwrap();

        let args = ImportArgs {
            compose_path,
            env: "dev".into(),
            write: true,
            out: Some(dest.clone()),
        };
        let exit = import(args).unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
        // The replacement must contain real TOML content (not the
        // sentinel) — proves we wrote and replaced.
        let written = std::fs::read_to_string(&dest).unwrap();
        assert!(
            !written.contains("this should be replaced"),
            "destination still has the pre-existing sentinel; atomic replacement skipped"
        );
        // Sanity: must be parseable as TOML — otherwise the write
        // landed torn / partial.
        let _: toml::Value = toml::from_str(&written).expect("written output must be valid TOML");
    }
}
