//! `coral project doctor` — drift / configuration health check.
//!
//! Replaces the originally proposed `coral project healthcheck` (which
//! collided with the `service.healthcheck` concept introduced in v0.17).
//! The name `doctor` mirrors the `npm doctor` / `cargo doctor` /
//! `rustup doctor` convention.

use anyhow::Result;
use clap::Args;
use coral_core::project::Project;
use std::path::Path;
use std::process::ExitCode;

use crate::commands::common::resolve_project;

#[derive(Args, Debug)]
pub struct DoctorArgs {
    /// Exit non-zero if any issue is reported (use in CI gates).
    #[arg(long)]
    pub strict: bool,
}

pub fn run(args: DoctorArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let project = resolve_project(wiki_root)?;
    let mut findings = Vec::new();

    findings.extend(check_apiversion(&project));
    findings.extend(check_repo_clones(&project));
    findings.extend(check_lockfile(&project));
    findings.extend(check_unique_paths(&project));

    print_report(&project, &findings);
    let has_errors = findings.iter().any(|f| f.severity == Severity::Error);
    if (args.strict && !findings.is_empty()) || has_errors {
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Severity {
    Info,
    Warn,
    Error,
}

#[derive(Debug)]
struct Finding {
    severity: Severity,
    message: String,
}

fn check_apiversion(project: &Project) -> Vec<Finding> {
    if project.is_legacy() {
        return vec![Finding {
            severity: Severity::Info,
            message: "legacy single-repo project (no coral.toml found)".to_string(),
        }];
    }
    if project.api_version != coral_core::project::manifest::CURRENT_API_VERSION {
        return vec![Finding {
            severity: Severity::Error,
            message: format!(
                "apiVersion '{}' is not supported by this binary ('{}')",
                project.api_version,
                coral_core::project::manifest::CURRENT_API_VERSION
            ),
        }];
    }
    Vec::new()
}

fn check_repo_clones(project: &Project) -> Vec<Finding> {
    if project.is_legacy() {
        return Vec::new();
    }
    let mut findings = Vec::new();
    for repo in &project.repos {
        if !repo.enabled {
            continue;
        }
        let path = project.resolved_path(repo);
        let in_place = repo
            .path
            .as_ref()
            .map(|p| p == Path::new("."))
            .unwrap_or(false);
        if in_place {
            continue;
        }
        if !path.exists() {
            findings.push(Finding {
                severity: Severity::Warn,
                message: format!(
                    "repo '{}' is not yet cloned (expected at {}). Run `coral project sync` once available.",
                    repo.name,
                    path.display()
                ),
            });
        } else if !path.join(".git").exists() {
            findings.push(Finding {
                severity: Severity::Warn,
                message: format!(
                    "{} exists but is not a git repository (no .git/)",
                    path.display()
                ),
            });
        }
    }
    findings
}

fn check_lockfile(project: &Project) -> Vec<Finding> {
    if project.is_legacy() {
        return Vec::new();
    }
    let lock_path = project.lockfile_path();
    let mut findings = Vec::new();
    if !lock_path.exists() {
        findings.push(Finding {
            severity: Severity::Warn,
            message: format!(
                "{} not found. `coral project lock` will create it.",
                lock_path.display()
            ),
        });
        return findings;
    }
    match coral_core::project::Lockfile::load_or_default(&lock_path) {
        Ok(lock) => {
            let manifest_repos: std::collections::BTreeSet<&str> =
                project.repos.iter().map(|r| r.name.as_str()).collect();
            for name in lock.repos.keys() {
                if !manifest_repos.contains(name.as_str()) {
                    findings.push(Finding {
                        severity: Severity::Warn,
                        message: format!(
                            "lockfile entry '{}' is not declared in coral.toml (stale)",
                            name
                        ),
                    });
                }
            }
            for repo in &project.repos {
                if !repo.enabled {
                    continue;
                }
                let in_place = repo
                    .path
                    .as_ref()
                    .map(|p| p == Path::new("."))
                    .unwrap_or(false);
                if in_place {
                    continue;
                }
                if !lock.repos.contains_key(&repo.name) {
                    findings.push(Finding {
                        severity: Severity::Warn,
                        message: format!("repo '{}' has no lockfile entry yet", repo.name),
                    });
                }
            }
        }
        Err(e) => findings.push(Finding {
            severity: Severity::Error,
            message: format!("failed to parse {}: {}", lock_path.display(), e),
        }),
    }
    findings
}

fn check_unique_paths(project: &Project) -> Vec<Finding> {
    if project.is_legacy() {
        return Vec::new();
    }
    let mut seen = std::collections::HashMap::new();
    let mut findings = Vec::new();
    for repo in &project.repos {
        let resolved = project.resolved_path(repo);
        if let Some(other) = seen.insert(resolved.clone(), repo.name.clone()) {
            findings.push(Finding {
                severity: Severity::Error,
                message: format!(
                    "repos '{}' and '{}' resolve to the same path {}",
                    other,
                    repo.name,
                    resolved.display()
                ),
            });
        }
    }
    findings
}

fn print_report(project: &Project, findings: &[Finding]) {
    if project.is_legacy() {
        println!("project: {} (legacy single-repo)", project.name);
    } else {
        println!("project: {} ({} repos)", project.name, project.repos.len());
        println!("manifest: {}", project.manifest_path.display());
    }
    if findings.is_empty() {
        println!();
        println!("✔ no issues found");
        return;
    }
    println!();
    for f in findings {
        let prefix = match f.severity {
            Severity::Info => "ℹ",
            Severity::Warn => "⚠",
            Severity::Error => "✘",
        };
        println!("{} {}", prefix, f.message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn doctor_clean_on_legacy_project() {
        let _guard = crate::commands::CWD_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = TempDir::new().unwrap();
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = run(DoctorArgs { strict: false }, None);
        std::env::set_current_dir(original).unwrap();
        assert!(result.is_ok());
    }

    #[test]
    fn doctor_warns_when_clones_missing() {
        let _guard = crate::commands::CWD_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("coral.toml"),
            r#"apiVersion = "coral.dev/v1"
[project]
name = "demo"

[[repos]]
name = "api"
url = "git@example.com:acme/api.git"
"#,
        )
        .unwrap();
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        // Strict mode → exit FAILURE because the clone is missing.
        let result = run(DoctorArgs { strict: true }, None);
        std::env::set_current_dir(original).unwrap();
        let exit = result.unwrap();
        assert_eq!(exit, ExitCode::FAILURE);
    }
}
