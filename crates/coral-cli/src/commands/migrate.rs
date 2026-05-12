//! `coral migrate-consumers` — draft migration PRs for consumer repos
//! when a breaking change is introduced.
//!
//! Part of M3.6 (FR-KILLER-5): auto-PRs draft en consumer repos cuando
//! hay breaking change. Opt-in — requires `.coral/consumers.json` or
//! explicit `--consumers` flag.
//!
//! v0.25 scope: writes draft PR specs to `.coral/migrations/<timestamp>/`
//! as JSON files. Does NOT actually create GitHub PRs (future integration).

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Args;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Schema for `.coral/consumers.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsumersConfig {
    pub consumers: Vec<Consumer>,
}

/// A single consumer entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Consumer {
    /// Repository identifier, e.g. `org/repo-name`.
    pub repo: String,
    /// Interface slugs this consumer depends on.
    pub depends_on: Vec<String>,
}

/// A draft PR spec written to disk for a consumer repo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationDraft {
    /// Target consumer repository.
    pub repo: String,
    /// Draft PR title.
    pub title: String,
    /// Draft PR body (markdown).
    pub body: String,
    /// Breaking change description.
    pub breaking_change: String,
    /// Suggested file changes (paths relative to consumer repo root).
    pub suggested_file_changes: Vec<String>,
    /// Timestamp of generation.
    pub generated_at: String,
}

#[derive(Args, Debug)]
pub struct MigrateArgs {
    /// Description of the breaking change.
    #[arg(long, required = true)]
    pub breaking_change: String,

    /// Comma-separated list of consumer repos (e.g. `org/repo1,org/repo2`).
    /// If not provided, reads from `.coral/consumers.json`.
    #[arg(long)]
    pub consumers: Option<String>,

    /// Print what PRs would be created without writing to disk.
    /// This is the default mode.
    #[arg(long)]
    pub dry_run: bool,

    /// Write draft PR specs to `.coral/migrations/<timestamp>/`.
    #[arg(long, conflicts_with = "dry_run")]
    pub apply: bool,
}

pub fn run(args: MigrateArgs, _wiki_root: Option<&Path>) -> Result<ExitCode> {
    let consumers = resolve_consumers(&args.consumers)?;

    if consumers.is_empty() {
        println!("No consumers found. Nothing to migrate.");
        return Ok(ExitCode::SUCCESS);
    }

    let apply = args.apply;
    let dry_run = args.dry_run || !apply;
    if !args.dry_run && !apply {
        eprintln!(
            "No --dry-run / --apply flag passed; defaulting to --dry-run. \
             Pass --apply to write draft PR specs to disk."
        );
    }

    let timestamp = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let drafts = build_drafts(&consumers, &args.breaking_change, &timestamp);

    if dry_run {
        println!("=== migrate-consumers (dry-run) ===");
        println!("Breaking change: {}", args.breaking_change);
        println!("Consumers: {}", consumers.len());
        println!();
        for draft in &drafts {
            println!("--- {} ---", draft.repo);
            println!("  Title: {}", draft.title);
            println!("  Body:");
            for line in draft.body.lines() {
                println!("    {line}");
            }
            println!("  Suggested changes: {:?}", draft.suggested_file_changes);
            println!();
        }
        println!("(pass --apply to write these to .coral/migrations/{timestamp}/)");
    } else {
        let migrations_dir = PathBuf::from(".coral").join("migrations").join(&timestamp);
        fs::create_dir_all(&migrations_dir)
            .with_context(|| format!("creating migrations dir: {}", migrations_dir.display()))?;

        for draft in &drafts {
            let filename = draft.repo.replace('/', "__") + ".json";
            let path = migrations_dir.join(&filename);
            let json =
                serde_json::to_string_pretty(draft).context("serializing migration draft")?;
            fs::write(&path, &json).with_context(|| format!("writing {}", path.display()))?;
            println!("wrote: {}", path.display());
        }
        println!(
            "\n{} draft PR spec(s) written to {}",
            drafts.len(),
            migrations_dir.display()
        );
    }

    Ok(ExitCode::SUCCESS)
}

/// Resolve consumers from `--consumers` flag or `.coral/consumers.json`.
pub fn resolve_consumers(consumers_arg: &Option<String>) -> Result<Vec<Consumer>> {
    if let Some(csv) = consumers_arg {
        let consumers = csv
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(|repo| Consumer {
                repo,
                depends_on: vec![],
            })
            .collect();
        return Ok(consumers);
    }

    let config_path = PathBuf::from(".coral").join("consumers.json");
    if !config_path.exists() {
        return Ok(vec![]);
    }

    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("reading {}", config_path.display()))?;
    let config: ConsumersConfig = serde_json::from_str(&content)
        .with_context(|| format!("parsing {}", config_path.display()))?;
    Ok(config.consumers)
}

/// Build draft PR specs for each consumer.
pub fn build_drafts(
    consumers: &[Consumer],
    breaking_change: &str,
    timestamp: &str,
) -> Vec<MigrationDraft> {
    consumers
        .iter()
        .map(|c| {
            let title = format!(
                "chore: migrate for breaking change — {}",
                truncate(breaking_change, 50)
            );
            let body = format!(
                "## Breaking Change Migration\n\n\
                 A breaking change has been introduced upstream:\n\n\
                 > {breaking_change}\n\n\
                 ### What to do\n\n\
                 Review the suggested file changes below and update your \
                 integration accordingly.\n\n\
                 ### Affected interfaces\n\n\
                 {interfaces}\n\n\
                 ---\n\
                 *Auto-generated by `coral migrate-consumers`*",
                interfaces = if c.depends_on.is_empty() {
                    "(not specified)".to_string()
                } else {
                    c.depends_on
                        .iter()
                        .map(|s| format!("- `{s}`"))
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            );
            let suggested = c
                .depends_on
                .iter()
                .map(|slug| format!("src/{slug}.rs"))
                .collect::<Vec<_>>();
            MigrationDraft {
                repo: c.repo.clone(),
                title,
                body,
                breaking_change: breaking_change.to_string(),
                suggested_file_changes: if suggested.is_empty() {
                    vec!["(review manually)".to_string()]
                } else {
                    suggested
                },
                generated_at: timestamp.to_string(),
            }
        })
        .collect()
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { s } else { &s[..max] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_generates_expected_output() {
        let consumers = vec![
            Consumer {
                repo: "acme/frontend".to_string(),
                depends_on: vec!["auth-api".to_string()],
            },
            Consumer {
                repo: "acme/mobile".to_string(),
                depends_on: vec!["auth-api".to_string(), "user-api".to_string()],
            },
        ];

        let drafts = build_drafts(
            &consumers,
            "removed /users/{id} endpoint",
            "20260511T120000Z",
        );

        assert_eq!(drafts.len(), 2);
        assert_eq!(drafts[0].repo, "acme/frontend");
        assert!(drafts[0].title.contains("removed /users/{id} endpoint"));
        assert!(drafts[0].body.contains("removed /users/{id} endpoint"));
        assert_eq!(drafts[0].suggested_file_changes, vec!["src/auth-api.rs"]);
        assert_eq!(drafts[1].repo, "acme/mobile");
        assert_eq!(
            drafts[1].suggested_file_changes,
            vec!["src/auth-api.rs", "src/user-api.rs"]
        );
    }

    #[test]
    fn reads_consumers_json_correctly() {
        let _guard = super::super::CWD_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::TempDir::new().unwrap();
        let coral_dir = tmp.path().join(".coral");
        fs::create_dir_all(&coral_dir).unwrap();
        let consumers_json = serde_json::json!({
            "consumers": [
                { "repo": "org/service-a", "depends_on": ["payment-api"] },
                { "repo": "org/service-b", "depends_on": [] }
            ]
        });
        fs::write(
            coral_dir.join("consumers.json"),
            serde_json::to_string_pretty(&consumers_json).unwrap(),
        )
        .unwrap();

        let orig_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        let result = resolve_consumers(&None);
        std::env::set_current_dir(orig_dir).unwrap();

        let consumers = result.unwrap();
        assert_eq!(consumers.len(), 2);
        assert_eq!(consumers[0].repo, "org/service-a");
        assert_eq!(consumers[0].depends_on, vec!["payment-api"]);
        assert_eq!(consumers[1].repo, "org/service-b");
        assert!(consumers[1].depends_on.is_empty());
    }

    #[test]
    fn arg_parsing_consumers_csv() {
        let consumers = resolve_consumers(&Some("org/a, org/b, org/c".to_string())).unwrap();
        assert_eq!(consumers.len(), 3);
        assert_eq!(consumers[0].repo, "org/a");
        assert_eq!(consumers[1].repo, "org/b");
        assert_eq!(consumers[2].repo, "org/c");
        // When passed via --consumers flag, depends_on is empty
        assert!(consumers[0].depends_on.is_empty());
    }
}
