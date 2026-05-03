//! `coral contract check` — cross-repo interface drift detection.
//!
//! Reads each `[[repos]]` cloned at `repos/<name>/`, parses each
//! `openapi.{yaml,yml,json}` as a provider interface, walks each
//! repo's `.coral/tests/*.{yaml,yml,hurl}` for HTTP step references,
//! then for every `depends_on` edge in the manifest it diffs the
//! consumer's expectations against the provider's declared interface.
//!
//! Use case: a developer changes `api`'s OpenAPI spec, removing
//! `/users/{id}`. `worker` still has `.coral/tests/*.yaml` referencing
//! `GET /users/42`. Without this command the test would fail at
//! runtime with a 404 — `coral contract check --strict` fails fast in
//! CI before `coral up` even runs, with an actionable message
//! pointing at the exact test file.

use anyhow::Result;
use clap::{Args, Subcommand};
use coral_test::ContractReport;
use std::path::Path;
use std::process::ExitCode;

use crate::commands::common::resolve_project;

#[derive(Args, Debug)]
pub struct ContractArgs {
    #[command(subcommand)]
    pub command: ContractCmd,
}

#[derive(Subcommand, Debug)]
pub enum ContractCmd {
    /// Check that consumers still match providers across the project.
    Check(CheckArgs),
}

#[derive(Args, Debug)]
pub struct CheckArgs {
    /// Output format.
    #[arg(long, default_value = "markdown")]
    pub format: Format,

    /// Exit non-zero if ANY finding (including warnings) is reported.
    /// Without `--strict`, warnings (e.g. status code drift) exit 0
    /// and only hard errors (unknown endpoint / method) exit non-zero.
    #[arg(long)]
    pub strict: bool,
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum Format {
    Markdown,
    Json,
}

pub fn run(args: ContractArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    match args.command {
        ContractCmd::Check(a) => run_check(a, wiki_root),
    }
}

fn run_check(args: CheckArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let project = resolve_project(wiki_root)?;
    if project.is_legacy() {
        anyhow::bail!(
            "`coral contract check` requires a coral.toml; this is a legacy single-repo project"
        );
    }
    if project.repos.is_empty() {
        println!("no [[repos]] declared; nothing to check");
        return Ok(ExitCode::SUCCESS);
    }

    let repos: Vec<(String, Vec<String>)> = project
        .repos
        .iter()
        .map(|r| (r.name.clone(), r.depends_on.clone()))
        .collect();

    let report = coral_test::check_contracts(&project.root, &repos)?;

    match args.format {
        Format::Markdown => print!("{}", coral_test::render_contract_markdown(&report)),
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&coral_test::render_contract_json(&report))?
        ),
    }

    let should_fail = if args.strict {
        report.has_findings()
    } else {
        report.has_errors()
    };
    if should_fail {
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

#[allow(dead_code)]
fn _ensure_contract_report_is_used(_r: ContractReport) {}
