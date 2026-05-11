//! `coral test guarantee --can-i-deploy` — single-command deploy verdict.
//!
//! ★ Killer feature #1: aggregates test results, contract checks, and
//! lint into a green/yellow/red signal for CI gates.

use anyhow::Result;
use clap::Args;
use std::collections::HashSet;
use std::path::Path;
use std::process::ExitCode;

use crate::commands::common::resolve_project;

#[derive(Args, Debug)]
pub struct GuaranteeArgs {
    /// Run the full deployment safety check.
    #[arg(long)]
    pub can_i_deploy: bool,

    /// Strict mode: yellow (warnings) also fails the gate.
    #[arg(long)]
    pub strict: bool,

    /// Output format (human, json, github-actions).
    #[arg(long, default_value = "human")]
    pub format: String,
}

/// Verdict levels for the deployment gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// All checks pass. Safe to deploy.
    Green,
    /// Warnings exist but no blockers. Deploy with caution.
    Yellow,
    /// Critical failures. Do NOT deploy.
    Red,
}

impl Verdict {
    pub fn exit_code(self) -> ExitCode {
        match self {
            Verdict::Green => ExitCode::SUCCESS,
            Verdict::Yellow => ExitCode::SUCCESS,
            Verdict::Red => ExitCode::FAILURE,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Green => "GREEN",
            Verdict::Yellow => "YELLOW",
            Verdict::Red => "RED",
        }
    }
}

/// Result of one check category.
#[derive(Debug)]
struct CheckResult {
    name: &'static str,
    passed: usize,
    warnings: usize,
    failures: usize,
    detail: String,
}

pub fn run(args: GuaranteeArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    if !args.can_i_deploy {
        anyhow::bail!("usage: coral test guarantee --can-i-deploy");
    }

    let project = resolve_project(wiki_root)?;
    let wiki_path = project.root.join(".wiki");

    let mut checks: Vec<CheckResult> = Vec::new();

    // 1. Lint check
    let lint_result = run_lint_check(&wiki_path);
    checks.push(lint_result);

    // 2. Contract check (if project has repos with depends_on edges)
    if !project.is_legacy() && !project.repos.is_empty() {
        let repos: Vec<(String, Vec<String>)> = project
            .repos
            .iter()
            .filter(|r| r.enabled)
            .map(|r| (r.name.clone(), r.depends_on.clone()))
            .collect();
        if !repos.is_empty() {
            let contract_result = run_contract_check(&project.root, &repos);
            checks.push(contract_result);
        }
    }

    // Compute overall verdict
    let total_failures: usize = checks.iter().map(|c| c.failures).sum();
    let total_warnings: usize = checks.iter().map(|c| c.warnings).sum();

    let verdict = if total_failures > 0 {
        Verdict::Red
    } else if total_warnings > 0 {
        Verdict::Yellow
    } else {
        Verdict::Green
    };

    // Output based on format
    match args.format.as_str() {
        "json" => {
            let json = serde_json::json!({
                "verdict": verdict.as_str(),
                "checks": checks.iter().map(|c| serde_json::json!({
                    "name": c.name,
                    "passed": c.passed,
                    "warnings": c.warnings,
                    "failures": c.failures,
                    "detail": c.detail,
                })).collect::<Vec<_>>(),
            });
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
        "github-actions" => {
            // Emit GitHub Actions annotations
            for c in &checks {
                if c.failures > 0 {
                    println!("::error title={}::{}", c.name, c.detail);
                } else if c.warnings > 0 {
                    println!("::warning title={}::{}", c.name, c.detail);
                }
            }
            println!("::set-output name=verdict::{}", verdict.as_str());
        }
        _ => {
            // Human-readable
            eprintln!("+-----------------------------------------+");
            eprintln!("|  coral test guarantee --can-i-deploy    |");
            eprintln!("+-----------------------------------------+");
            eprintln!();
            for c in &checks {
                let icon = if c.failures > 0 {
                    "X"
                } else if c.warnings > 0 {
                    "!"
                } else {
                    "+"
                };
                eprintln!(
                    "  [{}] {} -- {} passed, {} warnings, {} failures",
                    icon, c.name, c.passed, c.warnings, c.failures
                );
                if !c.detail.is_empty() && (c.failures > 0 || c.warnings > 0) {
                    eprintln!("      {}", c.detail);
                }
            }
            eprintln!();
            eprintln!("  Verdict: {}", verdict.as_str());
        }
    }

    // In strict mode, Yellow also fails
    if args.strict && verdict == Verdict::Yellow {
        return Ok(ExitCode::FAILURE);
    }

    Ok(verdict.exit_code())
}

fn run_lint_check(wiki_path: &Path) -> CheckResult {
    if !wiki_path.exists() {
        return CheckResult {
            name: "lint",
            passed: 0,
            warnings: 0,
            failures: 0,
            detail: "no wiki found (skipped)".into(),
        };
    }
    let pages = match coral_core::walk::read_pages(wiki_path) {
        Ok(p) => p,
        Err(e) => {
            // v0.30.x audit #004: a failure to read the wiki must NOT
            // be a silent zero-failure check. Pre-fix, `failures: 0`
            // contributed nothing to the verdict and the deploy gate
            // went green on an unreadable wiki. Surface it as a hard
            // failure with the underlying error string so CI logs are
            // actionable.
            return CheckResult {
                name: "lint",
                passed: 0,
                warnings: 0,
                failures: 1,
                detail: format!("failed to read wiki pages: {e}"),
            };
        }
    };
    let report = coral_lint::run_structural(&pages);

    let failures = report
        .issues
        .iter()
        .filter(|i| i.severity == coral_lint::LintSeverity::Critical)
        .count();
    let warnings = report
        .issues
        .iter()
        .filter(|i| i.severity == coral_lint::LintSeverity::Warning)
        .count();
    let affected_pages: HashSet<_> = report
        .issues
        .iter()
        .filter_map(|i| i.page.as_ref())
        .collect();
    let passed = pages.len().saturating_sub(affected_pages.len());

    let detail = if failures > 0 {
        format!("{failures} critical lint issues")
    } else if warnings > 0 {
        format!("{warnings} lint warnings")
    } else {
        String::new()
    };

    CheckResult {
        name: "lint",
        passed,
        warnings,
        failures,
        detail,
    }
}

fn run_contract_check(project_root: &Path, repos: &[(String, Vec<String>)]) -> CheckResult {
    let report = match coral_test::check_contracts(project_root, repos) {
        Ok(r) => r,
        Err(e) => {
            // v0.30.x audit #004: same false-green pattern as the lint
            // check above — a contract-check tool crash must register
            // as a failure, not a silent zero-failure no-op.
            return CheckResult {
                name: "contracts",
                passed: 0,
                warnings: 0,
                failures: 1,
                detail: format!("failed to run contract check: {e}"),
            };
        }
    };
    let failures = report
        .findings
        .iter()
        .filter(|f| f.severity == coral_test::ContractSeverity::Error)
        .count();
    let warnings = report
        .findings
        .iter()
        .filter(|f| f.severity == coral_test::ContractSeverity::Warning)
        .count();
    let passed = report.findings.len().saturating_sub(failures + warnings);

    let detail = if failures > 0 {
        format!("{failures} contract violations")
    } else if warnings > 0 {
        format!("{warnings} contract drift warnings")
    } else {
        String::new()
    };

    CheckResult {
        name: "contracts",
        passed,
        warnings,
        failures,
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_green_exit_code_is_success() {
        assert_eq!(Verdict::Green.exit_code(), ExitCode::SUCCESS);
    }

    #[test]
    fn verdict_yellow_exit_code_is_success() {
        assert_eq!(Verdict::Yellow.exit_code(), ExitCode::SUCCESS);
    }

    #[test]
    fn verdict_red_exit_code_is_failure() {
        assert_eq!(Verdict::Red.exit_code(), ExitCode::FAILURE);
    }

    #[test]
    fn verdict_as_str_values() {
        assert_eq!(Verdict::Green.as_str(), "GREEN");
        assert_eq!(Verdict::Yellow.as_str(), "YELLOW");
        assert_eq!(Verdict::Red.as_str(), "RED");
    }

    #[test]
    fn lint_check_returns_skip_when_no_wiki() {
        let result = run_lint_check(Path::new("/nonexistent/path/.wiki"));
        assert_eq!(result.name, "lint");
        assert_eq!(result.passed, 0);
        assert_eq!(result.warnings, 0);
        assert_eq!(result.failures, 0);
        assert!(result.detail.contains("no wiki found"));
    }

    /// v0.30.x audit #004 regression: a wiki page that triggers a
    /// Critical lint issue (broken wikilink) must produce `failures >= 1`
    /// from `run_lint_check`, and `run(..)` must surface a non-zero
    /// exit code (Verdict::Red). Pre-fix the read-failure branch
    /// returned `failures: 0` and the gate went green.
    #[test]
    fn lint_check_failure_propagates_to_red_verdict() {
        let _guard = crate::commands::CWD_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = tempfile::TempDir::new().unwrap();
        let wiki = dir.path().join(".wiki");
        let modules = wiki.join("modules");
        std::fs::create_dir_all(&modules).unwrap();
        // A page whose body wikilinks a non-existent page — yields a
        // BrokenWikilink Critical, which the lint check must count as
        // a failure (>=1).
        let body = "---\nslug: orphan\ntype: module\nlast_updated_commit: aaa\nconfidence: 0.7\nstatus: reviewed\n---\n\n# orphan\n\nSee [[missing-target]].\n";
        std::fs::write(modules.join("orphan.md"), body).unwrap();

        let result = run_lint_check(&wiki);
        assert!(
            result.failures >= 1,
            "expected at least one failure, got: {result:?}"
        );

        // Drive the top-level entry point and assert a non-zero exit.
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let exit = run(
            GuaranteeArgs {
                can_i_deploy: true,
                strict: false,
                format: "json".into(),
            },
            Some(&wiki),
        );
        std::env::set_current_dir(original).unwrap();
        let exit = exit.expect("guarantee::run must not bail");
        assert_eq!(
            exit,
            ExitCode::FAILURE,
            "wiki with broken wikilink must produce Verdict::Red"
        );
    }
}
