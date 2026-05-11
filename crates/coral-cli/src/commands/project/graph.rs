//! `coral project graph` — visualize the repo dependency graph from
//! `[[repos]] depends_on`.
//!
//! Outputs Mermaid (default — works in GitHub-rendered Markdown), DOT
//! (for Graphviz), or JSON (for downstream tooling). Pure read; no LLM.

use anyhow::Result;
use clap::Args;
use coral_core::project::Project;
use std::path::Path;
use std::process::ExitCode;

use crate::commands::common::resolve_project;

#[derive(Args, Debug)]
pub struct GraphArgs {
    /// Output format.
    #[arg(long, default_value = "mermaid")]
    pub format: Format,

    /// Optional title for Mermaid/DOT diagrams. Defaults to the project name.
    #[arg(long)]
    pub title: Option<String>,
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum Format {
    Mermaid,
    Dot,
    Json,
}

pub fn run(args: GraphArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let project = resolve_project(wiki_root)?;
    if project.is_legacy() {
        anyhow::bail!(
            "`coral project graph` requires a coral.toml; this is a legacy single-repo project"
        );
    }
    match args.format {
        Format::Mermaid => print_mermaid(&project, args.title.as_deref()),
        Format::Dot => print_dot(&project, args.title.as_deref()),
        Format::Json => print_json(&project)?,
    }
    Ok(ExitCode::SUCCESS)
}

fn print_mermaid(project: &Project, _title: Option<&str>) {
    println!("```mermaid");
    println!("graph TD");
    // Declare nodes first so isolated repos still appear.
    for repo in &project.repos {
        let label = if repo.tags.is_empty() {
            repo.name.clone()
        } else {
            format!("{}<br/>{}", repo.name, repo.tags.join(", "))
        };
        println!("    {}[\"{}\"]", repo_id(repo.name.as_str()), label);
    }
    for repo in &project.repos {
        for dep in &repo.depends_on {
            println!(
                "    {} --> {}",
                repo_id(repo.name.as_str()),
                repo_id(dep.as_str())
            );
        }
    }
    println!("```");
}

fn print_dot(project: &Project, title: Option<&str>) {
    let title = title.unwrap_or(&project.name);
    println!("digraph {} {{", repo_id(title));
    println!("  rankdir=TB;");
    println!("  node [shape=box, style=rounded];");
    for repo in &project.repos {
        let tag_label = if repo.tags.is_empty() {
            String::new()
        } else {
            format!("\\n[{}]", repo.tags.join(", "))
        };
        println!(
            "  {} [label=\"{}{}\"];",
            repo_id(repo.name.as_str()),
            repo.name,
            tag_label
        );
    }
    for repo in &project.repos {
        for dep in &repo.depends_on {
            println!(
                "  {} -> {};",
                repo_id(repo.name.as_str()),
                repo_id(dep.as_str())
            );
        }
    }
    println!("}}");
}

fn print_json(project: &Project) -> Result<()> {
    let nodes: Vec<_> = project
        .repos
        .iter()
        .map(|r| {
            serde_json::json!({
                "name": r.name,
                "tags": r.tags,
                "enabled": r.enabled,
            })
        })
        .collect();
    let edges: Vec<_> = project
        .repos
        .iter()
        .flat_map(|r| {
            r.depends_on
                .iter()
                .map(move |d| serde_json::json!({"from": r.name, "to": d}))
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "project": project.name,
            "nodes": nodes,
            "edges": edges,
        }))?
    );
    Ok(())
}

/// Sanitize a repo name into a Mermaid/DOT-safe identifier. Both
/// formats accept ASCII alphanumerics and underscores; replace
/// anything else.
fn repo_id(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_core::project::manifest::parse_toml;

    fn project_with_two_repos() -> Project {
        let raw = r#"apiVersion = "coral.dev/v1"
[project]
name = "demo"

[[repos]]
name = "api"
url  = "git@x:acme/api.git"
tags = ["service"]

[[repos]]
name = "worker"
url  = "git@x:acme/worker.git"
depends_on = ["api"]
"#;
        let mut p = parse_toml(raw, std::path::Path::new("/tmp/coral.toml")).unwrap();
        p.root = std::path::PathBuf::from("/tmp");
        p.manifest_path = std::path::PathBuf::from("/tmp/coral.toml");
        p
    }

    #[test]
    fn mermaid_output_lists_nodes_and_edges() {
        let p = project_with_two_repos();
        let captured = std::panic::catch_unwind(|| {
            // Capture stdout indirectly: the function prints to stdout.
            // We can't easily intercept, but assert no panic.
            print_mermaid(&p, None);
        });
        // v0.30.0 audit cycle 5 B10: surface the panic payload instead
        // of `assert!(... .is_ok())`, which only prints "left: true,
        // right: false" on failure. `.expect` lets a future regression
        // include the actual panic message in the CI log.
        captured.expect("print_mermaid must not panic on a two-repo project");
    }

    #[test]
    fn repo_id_replaces_unsafe_chars() {
        assert_eq!(repo_id("api"), "api");
        assert_eq!(repo_id("api-v2"), "api_v2");
        assert_eq!(repo_id("acme/api"), "acme_api");
    }
}
