//! Push wiki pages to a Notion database via the Notion REST API.
//!
//! Thin wrapper over `coral export --format notion-json` that reads the
//! `NOTION_TOKEN` + `CORAL_NOTION_DB` env vars (or flags), substitutes the
//! database id, and shells out to `curl` to POST each body.
//!
//! v0.2.1: shells to `curl` to keep the binary footprint small. v0.3 may
//! switch to `reqwest` if richer error handling is needed.

use anyhow::{Context, Result};
use clap::Args;
use coral_core::page::Page;
use coral_core::walk;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Args, Debug, Default)]
pub struct NotionPushArgs {
    /// Notion integration token. Falls back to `NOTION_TOKEN` env var.
    #[arg(long)]
    pub token: Option<String>,
    /// Notion database id (the parent the pages get pushed into). Falls back
    /// to `CORAL_NOTION_DB` env var.
    #[arg(long)]
    pub database: Option<String>,
    /// Filter by page type (repeatable).
    #[arg(long = "type", value_name = "TYPE")]
    pub types: Vec<String>,
    /// Print what would be pushed without actually calling Notion.
    #[arg(long)]
    pub dry_run: bool,
}

pub fn run(args: NotionPushArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let token = args
        .token
        .or_else(|| std::env::var("NOTION_TOKEN").ok())
        .context("NOTION_TOKEN env var (or --token) required")?;
    let database = args
        .database
        .or_else(|| std::env::var("CORAL_NOTION_DB").ok())
        .context("CORAL_NOTION_DB env var (or --database) required")?;

    let root: PathBuf = wiki_root
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".wiki"));
    if !root.exists() {
        anyhow::bail!(
            "wiki root not found: {}. Run `coral init` first.",
            root.display()
        );
    }
    let pages = walk::read_pages(&root)
        .with_context(|| format!("reading pages from {}", root.display()))?;
    let pages = filter_by_types(pages, &args.types);
    let bodies = build_notion_bodies(&pages, &database);

    if args.dry_run {
        println!(
            "Would POST {} page(s) to Notion database {database}",
            bodies.len()
        );
        for (i, _b) in bodies.iter().enumerate() {
            println!("  - {i}: {}", pages[i].frontmatter.slug);
        }
        return Ok(ExitCode::SUCCESS);
    }

    let mut ok = 0usize;
    let mut fail = 0usize;
    for (i, body) in bodies.iter().enumerate() {
        let slug = &pages[i].frontmatter.slug;
        let json_string = serde_json::to_string(body)?;
        let output = std::process::Command::new("curl")
            .args([
                "-s",
                "-o",
                "/dev/null",
                "-w",
                "%{http_code}",
                "-X",
                "POST",
                "https://api.notion.com/v1/pages",
                "-H",
                &format!("Authorization: Bearer {token}"),
                "-H",
                "Notion-Version: 2022-06-28",
                "-H",
                "Content-Type: application/json",
                "-d",
                &json_string,
            ])
            .output()
            .context("invoking curl (is it in PATH?)")?;
        let http_code = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if http_code.starts_with('2') {
            ok += 1;
            tracing::info!(slug = %slug, http = %http_code, "notion push ok");
        } else {
            fail += 1;
            eprintln!("FAIL {slug}: HTTP {http_code}");
        }
    }
    println!("Pushed: {ok} ok, {fail} failed.");
    Ok(if fail == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn filter_by_types(pages: Vec<Page>, types: &[String]) -> Vec<Page> {
    if types.is_empty() {
        return pages;
    }
    let allow: std::collections::HashSet<&str> = types.iter().map(String::as_str).collect();
    pages
        .into_iter()
        .filter(|p| allow.contains(super::export::page_type_name_pub(&p.frontmatter)))
        .collect()
}

/// Build Notion `POST /v1/pages` request bodies, one per page, with the
/// supplied database id substituted into `parent.database_id`.
pub fn build_notion_bodies(pages: &[Page], database_id: &str) -> Vec<Value> {
    pages
        .iter()
        .map(|p| {
            serde_json::json!({
                "parent": { "database_id": database_id },
                "properties": {
                    "Name": {
                        "title": [{ "text": { "content": p.frontmatter.slug } }]
                    },
                    "Type": {
                        "select": { "name": super::export::page_type_name_pub(&p.frontmatter) }
                    },
                    "Status": {
                        "select": { "name": super::export::status_name_pub(&p.frontmatter) }
                    },
                    "Confidence": {
                        "number": p.frontmatter.confidence.as_f64()
                    }
                },
                "children": [{
                    "object": "block",
                    "type": "paragraph",
                    "paragraph": {
                        "rich_text": [{
                            "type": "text",
                            "text": { "content": p.body.chars().take(2000).collect::<String>() }
                        }]
                    }
                }]
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
    use std::path::PathBuf;

    fn page(slug: &str, t: PageType) -> Page {
        Page {
            path: PathBuf::from(format!(".wiki/x/{slug}.md")),
            frontmatter: Frontmatter {
                slug: slug.into(),
                page_type: t,
                last_updated_commit: "abc".into(),
                confidence: Confidence::try_new(0.7).unwrap(),
                sources: vec![],
                backlinks: vec![],
                status: Status::Reviewed,
                generated_at: None,
                extra: Default::default(),
            },
            body: "body".into(),
        }
    }

    #[test]
    fn build_substitutes_database_id() {
        let pages = vec![page("a", PageType::Module)];
        let bodies = build_notion_bodies(&pages, "db-abc-123");
        assert_eq!(bodies[0]["parent"]["database_id"], "db-abc-123");
    }

    #[test]
    fn build_includes_slug_in_title() {
        let pages = vec![page("order", PageType::Entity)];
        let bodies = build_notion_bodies(&pages, "db");
        assert_eq!(
            bodies[0]["properties"]["Name"]["title"][0]["text"]["content"],
            "order"
        );
    }

    #[test]
    fn build_includes_type_select_name() {
        let pages = vec![page("x", PageType::Concept)];
        let bodies = build_notion_bodies(&pages, "db");
        assert_eq!(bodies[0]["properties"]["Type"]["select"]["name"], "concept");
    }

    #[test]
    fn filter_by_types_keeps_matching_only() {
        let pages = vec![
            page("a", PageType::Module),
            page("b", PageType::Concept),
            page("c", PageType::Entity),
        ];
        let kept = filter_by_types(pages, &["concept".into()]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].frontmatter.slug, "b");
    }
}
