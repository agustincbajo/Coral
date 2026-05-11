//! `coral scaffold module --like <slug>` — wiki-driven scaffolding.
//!
//! Part of M3.6 (FR-KILLER-8): wiki-driven scaffolding. Opt-in.
//!
//! Reads an existing wiki page's frontmatter + structure and generates
//! a new page with the same structure but placeholder content.

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
use coral_core::walk;

#[derive(Args, Debug)]
pub struct ScaffoldArgs {
    #[command(subcommand)]
    pub command: ScaffoldCmd,
}

#[derive(Subcommand, Debug)]
pub enum ScaffoldCmd {
    /// Scaffold a new module page from an existing page template.
    Module(ModuleArgs),
}

#[derive(Args, Debug)]
pub struct ModuleArgs {
    /// Slug of an existing wiki page to use as a structural template.
    #[arg(long)]
    pub like: String,

    /// Slug for the new page being scaffolded.
    #[arg(long)]
    pub name: String,

    /// Output directory (default: `.wiki/`).
    #[arg(long)]
    pub output: Option<PathBuf>,
}

pub fn run(args: ScaffoldArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    match args.command {
        ScaffoldCmd::Module(module_args) => run_module(module_args, wiki_root),
    }
}

fn run_module(args: ModuleArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let root = wiki_root
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".wiki"));

    if !root.exists() {
        bail!(
            "wiki root not found: {}. Run `coral init` first.",
            root.display()
        );
    }

    // Find the template page by slug
    let pages = walk::read_pages(&root).context("reading wiki pages")?;
    let template = pages
        .iter()
        .find(|p| p.frontmatter.slug == args.like);

    let template = match template {
        Some(p) => p,
        None => bail!(
            "no wiki page with slug '{}' found in {}",
            args.like,
            root.display()
        ),
    };

    // Build the scaffolded page
    let scaffolded = scaffold_from_template(&template.frontmatter, &template.body, &args.name);

    // Determine output path
    let output_dir = args.output.unwrap_or(root);
    let subdir = page_type_subdir(template.frontmatter.page_type);
    let target_dir = output_dir.join(subdir);
    fs::create_dir_all(&target_dir)
        .with_context(|| format!("creating dir {}", target_dir.display()))?;

    let target_file = target_dir.join(format!("{}.md", args.name));
    fs::write(&target_file, &scaffolded)
        .with_context(|| format!("writing {}", target_file.display()))?;

    println!("scaffolded: {}", target_file.display());
    println!("  template: {} (type={:?})", args.like, template.frontmatter.page_type);
    println!("  new slug: {}", args.name);

    Ok(ExitCode::SUCCESS)
}

/// Generate a scaffolded page from a template's frontmatter and body.
pub fn scaffold_from_template(fm: &Frontmatter, body: &str, new_slug: &str) -> String {
    // Build new frontmatter preserving the structure
    let new_fm = Frontmatter {
        slug: new_slug.to_string(),
        page_type: fm.page_type,
        last_updated_commit: "0000000".to_string(),
        confidence: Confidence::try_new(0.3).unwrap(),
        sources: vec![],
        backlinks: vec![],
        status: Status::Draft,
        generated_at: Some(chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()),
        valid_from: None,
        valid_to: None,
        superseded_by: None,
        extra: fm.extra.clone(),
    };

    let yaml = serde_yaml_ng::to_string(&new_fm).unwrap_or_default();

    // Build placeholder body preserving heading structure from template
    let placeholder_body = extract_heading_structure(body, new_slug);

    format!("---\n{yaml}---\n\n{placeholder_body}")
}

/// Extract heading structure from a template body, replacing content
/// with placeholders but keeping the headings intact.
fn extract_heading_structure(body: &str, new_slug: &str) -> String {
    let mut result = String::new();
    let mut last_was_heading = false;

    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            if !result.is_empty() && !last_was_heading {
                result.push('\n');
            }
            result.push_str(line);
            result.push('\n');
            result.push_str(&format!("\n<!-- TODO: fill in content for {new_slug} -->\n\n"));
            last_was_heading = true;
        } else {
            last_was_heading = false;
        }
    }

    if result.is_empty() {
        format!(
            "# {new_slug}\n\n<!-- TODO: fill in content for {new_slug} -->\n"
        )
    } else {
        result
    }
}

/// Map page type to its conventional subdirectory under `.wiki/`.
fn page_type_subdir(pt: PageType) -> &'static str {
    match pt {
        PageType::Module => "modules",
        PageType::Concept => "concepts",
        PageType::Entity => "entities",
        PageType::Flow => "flows",
        PageType::Decision => "decisions",
        PageType::Synthesis => "synthesis",
        PageType::Operation => "operations",
        PageType::Source => "sources",
        PageType::Gap => "gaps",
        PageType::Index => ".",
        PageType::Log => ".",
        PageType::Schema => "schemas",
        PageType::Readme => ".",
        PageType::Reference => "references",
        PageType::Interface => "interfaces",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn make_test_frontmatter(slug: &str) -> Frontmatter {
        Frontmatter {
            slug: slug.to_string(),
            page_type: PageType::Module,
            last_updated_commit: "abc1234".to_string(),
            confidence: Confidence::try_new(0.8).unwrap(),
            sources: vec!["src/auth.rs".to_string()],
            backlinks: vec!["user-management".to_string()],
            status: Status::Reviewed,
            generated_at: Some("2026-01-01T00:00:00Z".to_string()),
            valid_from: None,
            valid_to: None,
            superseded_by: None,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn scaffold_produces_correct_frontmatter_from_template() {
        let fm = make_test_frontmatter("auth-module");
        let body = "# Auth Module\n\nHandles authentication.\n\n## Endpoints\n\nGET /login\n";
        let result = scaffold_from_template(&fm, body, "payment-module");

        // Should contain the new slug
        assert!(result.contains("slug: payment-module"));
        // Should preserve page type
        assert!(result.contains("type: module"));
        // Should have draft status
        assert!(result.contains("status: draft"));
        // Should have low confidence
        assert!(result.contains("confidence: 0.3"));
        // Should NOT carry over the template's sources/backlinks
        assert!(!result.contains("src/auth.rs"));
        assert!(!result.contains("user-management"));
        // Should have frontmatter delimiters
        assert!(result.starts_with("---\n"));
        assert!(result.contains("\n---\n"));
    }

    #[test]
    fn scaffold_rejects_nonexistent_like_slug() {
        let _guard = super::super::CWD_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::TempDir::new().unwrap();
        let wiki_dir = tmp.path().join(".wiki");
        fs::create_dir_all(wiki_dir.join("modules")).unwrap();

        // Write a page with slug "existing-page"
        let page_content = r#"---
slug: existing-page
type: module
last_updated_commit: "abc1234"
confidence: 0.8
sources: []
backlinks: []
status: draft
---

# Existing Page

Content here.
"#;
        fs::write(wiki_dir.join("modules/existing-page.md"), page_content).unwrap();

        let args = ModuleArgs {
            like: "nonexistent-slug".to_string(),
            name: "new-page".to_string(),
            output: Some(tmp.path().join("output")),
        };

        let result = run_module(args, Some(&wiki_dir));
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("nonexistent-slug"));
    }

    #[test]
    fn heading_structure_preserved_as_placeholders() {
        let body = "# Main Title\n\nSome text.\n\n## Section A\n\nDetails.\n\n## Section B\n\nMore details.\n";
        let result = extract_heading_structure(body, "new-mod");

        assert!(result.contains("# Main Title"));
        assert!(result.contains("## Section A"));
        assert!(result.contains("## Section B"));
        // Should have placeholder comments
        assert!(result.contains("<!-- TODO: fill in content for new-mod -->"));
        // Should NOT contain the original content
        assert!(!result.contains("Some text."));
        assert!(!result.contains("Details."));
        assert!(!result.contains("More details."));
    }
}
