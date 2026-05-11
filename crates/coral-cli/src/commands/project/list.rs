//! `coral project list` — tabular view of the repos in the manifest.

use anyhow::Result;
use clap::Args;
use coral_core::project::Project;
use std::path::Path;
use std::process::ExitCode;

use crate::commands::common::resolve_project;

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Output format. Default is a Markdown table; `json` emits a stable
    /// JSON array suitable for piping to `jq`.
    #[arg(long, default_value = "markdown")]
    pub format: Format,
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum Format {
    Markdown,
    Json,
}

pub fn run(args: ListArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let project = resolve_project(wiki_root)?;
    match args.format {
        Format::Markdown => print_markdown(&project),
        Format::Json => print_json(&project)?,
    }
    Ok(ExitCode::SUCCESS)
}

fn print_markdown(project: &Project) {
    println!("# {} — repos", project.name);
    if project.is_legacy() {
        println!();
        println!("_(legacy single-repo project; no `coral.toml` present)_");
        println!();
        println!("| name | path |");
        println!("|------|------|");
        for r in &project.repos {
            println!(
                "| {} | {} |",
                r.name,
                r.path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| ".".to_string()),
            );
        }
        return;
    }
    println!();
    println!("| name | ref | tags | path | url |");
    println!("|------|-----|------|------|-----|");
    for r in &project.repos {
        let url = project
            .resolved_url(r)
            .unwrap_or_else(|| "<unresolved>".to_string());
        let path = project.resolved_path(r);
        let r_ref = r
            .r#ref
            .clone()
            .unwrap_or_else(|| project.defaults.r#ref.clone());
        let tags = if r.tags.is_empty() {
            "—".to_string()
        } else {
            r.tags.join(",")
        };
        println!(
            "| {} | {} | {} | {} | {} |",
            r.name,
            r_ref,
            tags,
            path.display(),
            url,
        );
    }
}

fn print_json(project: &Project) -> Result<()> {
    let entries: Vec<_> = project
        .repos
        .iter()
        .map(|r| {
            serde_json::json!({
                "name": r.name,
                "ref": r.r#ref.clone().unwrap_or_else(|| project.defaults.r#ref.clone()),
                "tags": r.tags,
                "path": project.resolved_path(r).display().to_string(),
                "url": project.resolved_url(r),
                "depends_on": r.depends_on,
                "enabled": r.enabled,
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "project": project.name,
            "legacy": project.is_legacy(),
            "repos": entries,
        }))?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn list_legacy_project_runs_clean() {
        let _guard = crate::commands::CWD_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = TempDir::new().unwrap();
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = run(
            ListArgs {
                format: Format::Json,
            },
            None,
        );
        std::env::set_current_dir(original).unwrap();
        result.expect("project list --format json must succeed against a valid coral.toml");
    }
}
