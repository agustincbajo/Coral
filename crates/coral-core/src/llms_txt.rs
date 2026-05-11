//! `llms.txt` generator — machine-readable wiki summary for AI agents.
//!
//! Produces a structured text file that any LLM can ingest as context.
//! Format follows the llms.txt convention: title, description, then
//! a list of pages with their slug, type, and one-line summary.

use crate::page::Page;

/// Generate llms.txt content from wiki pages.
pub fn generate(pages: &[Page], project_name: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {project_name}\n\n"));
    out.push_str("> Auto-generated wiki summary for AI agent consumption.\n\n");

    // Group by type
    let mut by_type: std::collections::BTreeMap<String, Vec<&Page>> =
        std::collections::BTreeMap::new();
    for p in pages {
        let type_str = format!("{:?}", p.frontmatter.page_type).to_lowercase();
        by_type.entry(type_str).or_default().push(p);
    }

    for (page_type, type_pages) in &by_type {
        out.push_str(&format!("## {page_type}\n\n"));
        for p in type_pages {
            let summary = p
                .body
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("(no description)")
                .chars()
                .take(100)
                .collect::<String>();
            out.push_str(&format!("- [{}]: {}\n", p.frontmatter.slug, summary));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter::{Confidence, Frontmatter, PageType, Status};
    use crate::page::Page;
    use std::path::PathBuf;

    fn make_page(slug: &str, page_type: PageType, body: &str) -> Page {
        Page {
            path: PathBuf::from(format!("{slug}.md")),
            frontmatter: Frontmatter {
                slug: slug.to_string(),
                page_type,
                last_updated_commit: "abc123".into(),
                confidence: Confidence::try_new(0.8).unwrap(),
                sources: Vec::new(),
                backlinks: Vec::new(),
                status: Status::Draft,
                generated_at: None,
                valid_from: None,
                valid_to: None,
                extra: std::collections::BTreeMap::new(),
            },
            body: body.to_string(),
        }
    }

    #[test]
    fn generate_basic_output() {
        let pages = vec![
            make_page("auth", PageType::Module, "# Authentication\n\nHandles user login."),
            make_page("payment", PageType::Flow, "# Payment Flow\n\nProcesses transactions."),
            make_page("user", PageType::Entity, "# User\n\nRepresents a system user."),
        ];

        let output = generate(&pages, "TestProject");

        assert!(output.starts_with("# TestProject\n"));
        assert!(output.contains("> Auto-generated wiki summary for AI agent consumption."));
        assert!(output.contains("## module"));
        assert!(output.contains("## flow"));
        assert!(output.contains("## entity"));
        assert!(output.contains("- [auth]: # Authentication"));
        assert!(output.contains("- [payment]: # Payment Flow"));
        assert!(output.contains("- [user]: # User"));
    }

    #[test]
    fn generate_empty_pages() {
        let output = generate(&[], "EmptyProject");
        assert!(output.starts_with("# EmptyProject\n"));
        assert!(output.contains("> Auto-generated wiki summary"));
        // No type sections when there are no pages.
        assert!(!output.contains("## "));
    }

    #[test]
    fn generate_truncates_long_summaries() {
        let long_line = "x".repeat(200);
        let pages = vec![make_page("long", PageType::Concept, &long_line)];
        let output = generate(&pages, "Proj");
        // The summary line should be truncated to 100 chars.
        let summary_line = output.lines().find(|l| l.starts_with("- [long]:")).unwrap();
        // "- [long]: " is 10 chars prefix, then 100 chars of content.
        let after_prefix = summary_line.strip_prefix("- [long]: ").unwrap();
        assert_eq!(after_prefix.len(), 100);
    }

    #[test]
    fn generate_groups_by_type_sorted() {
        let pages = vec![
            make_page("z", PageType::Module, "Z module"),
            make_page("a", PageType::Concept, "A concept"),
            make_page("b", PageType::Module, "B module"),
        ];
        let output = generate(&pages, "Proj");
        // BTreeMap sorts keys: concept < module
        let concept_pos = output.find("## concept").unwrap();
        let module_pos = output.find("## module").unwrap();
        assert!(concept_pos < module_pos);
    }

    #[test]
    fn generate_empty_body_uses_placeholder() {
        let pages = vec![make_page("empty", PageType::Concept, "")];
        let output = generate(&pages, "Proj");
        assert!(output.contains("(no description)"));
    }
}
