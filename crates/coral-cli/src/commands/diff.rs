//! `coral diff <slugA> <slugB>` — compare two wiki pages structurally.
//!
//! Use cases:
//! - You suspect two pages overlap and want to see whether to merge them.
//! - You're considering retiring a page and want to spot what claims it
//!   shares with neighboring pages.
//! - You're reviewing a `wiki/auto-ingest` PR and want to see what changed
//!   between an existing page and its proposed replacement.
//!
//! v0.5 ships the **structural** diff: frontmatter delta, sources/backlinks
//! set arithmetic, wikilink overlap, and body length stats. A future
//! `--semantic` flag would add an LLM pass to surface contradictions
//! between the bodies — that's tracked separately and is not in this MVP.

use anyhow::{Context, Result};
use clap::Args;
use coral_core::page::Page;
use coral_core::walk;
use coral_core::wikilinks;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Args, Debug)]
pub struct DiffArgs {
    /// Slug of the first page.
    pub slug_a: String,
    /// Slug of the second page.
    pub slug_b: String,
    /// Output format: markdown (default) or json.
    #[arg(long, default_value = "markdown")]
    pub format: String,
}

pub fn run(args: DiffArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
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
    let a = find_page(&pages, &args.slug_a)
        .with_context(|| format!("page `{}` not found in {}", args.slug_a, root.display()))?;
    let b = find_page(&pages, &args.slug_b)
        .with_context(|| format!("page `{}` not found in {}", args.slug_b, root.display()))?;

    let report = compute_diff(a, b);
    match args.format.as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(&report.as_json())?),
        _ => println!("{}", report.as_markdown()),
    }
    Ok(ExitCode::SUCCESS)
}

fn find_page<'a>(pages: &'a [Page], slug: &str) -> Result<&'a Page> {
    pages
        .iter()
        .find(|p| p.frontmatter.slug == slug)
        .ok_or_else(|| anyhow::anyhow!("no page with slug `{slug}`"))
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DiffReport {
    pub slug_a: String,
    pub slug_b: String,
    pub same_type: bool,
    pub type_a: String,
    pub type_b: String,
    pub same_status: bool,
    pub status_a: String,
    pub status_b: String,
    pub confidence_delta: f64,
    pub sources_common: BTreeSet<String>,
    pub sources_only_a: BTreeSet<String>,
    pub sources_only_b: BTreeSet<String>,
    pub wikilinks_common: BTreeSet<String>,
    pub wikilinks_only_a: BTreeSet<String>,
    pub wikilinks_only_b: BTreeSet<String>,
    pub body_chars_a: usize,
    pub body_chars_b: usize,
}

impl DiffReport {
    pub fn as_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "# Diff: `{}` ↔ `{}`\n\n",
            self.slug_a, self.slug_b
        ));

        out.push_str("## Frontmatter\n\n");
        out.push_str(&format!(
            "| field | `{a}` | `{b}` |\n|---|---|---|\n",
            a = self.slug_a,
            b = self.slug_b
        ));
        out.push_str(&format!(
            "| type | `{}` | `{}` |{}\n",
            self.type_a,
            self.type_b,
            if self.same_type { "" } else { " ⚠️ differ" }
        ));
        out.push_str(&format!(
            "| status | `{}` | `{}` |{}\n",
            self.status_a,
            self.status_b,
            if self.same_status {
                ""
            } else {
                " ⚠️ differ"
            }
        ));
        out.push_str(&format!(
            "| confidence Δ | — | — | {:+.2} |\n",
            self.confidence_delta
        ));
        out.push_str(&format!(
            "| body chars | {} | {} |\n\n",
            self.body_chars_a, self.body_chars_b
        ));

        section(
            &mut out,
            "Sources",
            &self.sources_common,
            &self.sources_only_a,
            &self.sources_only_b,
            &self.slug_a,
            &self.slug_b,
        );
        section(
            &mut out,
            "Wikilinks",
            &self.wikilinks_common,
            &self.wikilinks_only_a,
            &self.wikilinks_only_b,
            &self.slug_a,
            &self.slug_b,
        );

        out
    }

    pub fn as_json(&self) -> serde_json::Value {
        serde_json::json!({
            "slug_a": self.slug_a,
            "slug_b": self.slug_b,
            "type": {
                "a": self.type_a, "b": self.type_b, "same": self.same_type,
            },
            "status": {
                "a": self.status_a, "b": self.status_b, "same": self.same_status,
            },
            "confidence_delta": self.confidence_delta,
            "body_chars": { "a": self.body_chars_a, "b": self.body_chars_b },
            "sources": {
                "common": self.sources_common,
                "only_a": self.sources_only_a,
                "only_b": self.sources_only_b,
            },
            "wikilinks": {
                "common": self.wikilinks_common,
                "only_a": self.wikilinks_only_a,
                "only_b": self.wikilinks_only_b,
            },
        })
    }
}

fn section(
    out: &mut String,
    label: &str,
    common: &BTreeSet<String>,
    only_a: &BTreeSet<String>,
    only_b: &BTreeSet<String>,
    slug_a: &str,
    slug_b: &str,
) {
    out.push_str(&format!("## {label}\n\n"));
    if common.is_empty() && only_a.is_empty() && only_b.is_empty() {
        out.push_str("_(none)_\n\n");
        return;
    }
    if !common.is_empty() {
        out.push_str(&format!("### Common ({})\n\n", common.len()));
        for s in common {
            out.push_str(&format!("- {s}\n"));
        }
        out.push('\n');
    }
    if !only_a.is_empty() {
        out.push_str(&format!("### Only in `{slug_a}` ({})\n\n", only_a.len()));
        for s in only_a {
            out.push_str(&format!("- {s}\n"));
        }
        out.push('\n');
    }
    if !only_b.is_empty() {
        out.push_str(&format!("### Only in `{slug_b}` ({})\n\n", only_b.len()));
        for s in only_b {
            out.push_str(&format!("- {s}\n"));
        }
        out.push('\n');
    }
}

pub(crate) fn compute_diff(a: &Page, b: &Page) -> DiffReport {
    let sources_a: BTreeSet<String> = a.frontmatter.sources.iter().cloned().collect();
    let sources_b: BTreeSet<String> = b.frontmatter.sources.iter().cloned().collect();
    let wikis_a: BTreeSet<String> = wikilinks::extract(&a.body).into_iter().collect();
    let wikis_b: BTreeSet<String> = wikilinks::extract(&b.body).into_iter().collect();

    DiffReport {
        slug_a: a.frontmatter.slug.clone(),
        slug_b: b.frontmatter.slug.clone(),
        same_type: a.frontmatter.page_type == b.frontmatter.page_type,
        type_a: format!("{:?}", a.frontmatter.page_type).to_lowercase(),
        type_b: format!("{:?}", b.frontmatter.page_type).to_lowercase(),
        same_status: a.frontmatter.status == b.frontmatter.status,
        status_a: format!("{:?}", a.frontmatter.status).to_lowercase(),
        status_b: format!("{:?}", b.frontmatter.status).to_lowercase(),
        confidence_delta: b.frontmatter.confidence.as_f64() - a.frontmatter.confidence.as_f64(),
        sources_common: sources_a.intersection(&sources_b).cloned().collect(),
        sources_only_a: sources_a.difference(&sources_b).cloned().collect(),
        sources_only_b: sources_b.difference(&sources_a).cloned().collect(),
        wikilinks_common: wikis_a.intersection(&wikis_b).cloned().collect(),
        wikilinks_only_a: wikis_a.difference(&wikis_b).cloned().collect(),
        wikilinks_only_b: wikis_b.difference(&wikis_a).cloned().collect(),
        body_chars_a: a.body.chars().count(),
        body_chars_b: b.body.chars().count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};

    fn page(
        slug: &str,
        ty: PageType,
        status: Status,
        conf: f64,
        body: &str,
        sources: &[&str],
    ) -> Page {
        Page {
            path: PathBuf::from(format!(".wiki/x/{slug}.md")),
            frontmatter: Frontmatter {
                slug: slug.into(),
                page_type: ty,
                last_updated_commit: "abc".into(),
                confidence: Confidence::try_new(conf).unwrap(),
                sources: sources.iter().map(|s| s.to_string()).collect(),
                backlinks: vec![],
                status,
                generated_at: None,
                extra: Default::default(),
            },
            body: body.to_string(),
        }
    }

    #[test]
    fn diff_finds_common_sources_and_wikilinks() {
        let a = page(
            "order",
            PageType::Module,
            Status::Reviewed,
            0.8,
            "See [[outbox]] and [[idempotency]].",
            &["src/order.rs", "docs/adr/0001.md"],
        );
        let b = page(
            "checkout",
            PageType::Flow,
            Status::Reviewed,
            0.7,
            "Goes through [[outbox]] then [[payment]].",
            &["src/checkout.rs", "docs/adr/0001.md"],
        );
        let r = compute_diff(&a, &b);
        assert_eq!(r.slug_a, "order");
        assert_eq!(r.slug_b, "checkout");
        assert!(!r.same_type);
        assert!(r.same_status);
        assert!((r.confidence_delta - (-0.1)).abs() < 1e-9);
        assert_eq!(r.sources_common, set(&["docs/adr/0001.md"]));
        assert_eq!(r.sources_only_a, set(&["src/order.rs"]));
        assert_eq!(r.sources_only_b, set(&["src/checkout.rs"]));
        assert_eq!(r.wikilinks_common, set(&["outbox"]));
        assert_eq!(r.wikilinks_only_a, set(&["idempotency"]));
        assert_eq!(r.wikilinks_only_b, set(&["payment"]));
    }

    #[test]
    fn diff_markdown_includes_each_section_only_when_non_empty() {
        let a = page(
            "a",
            PageType::Module,
            Status::Reviewed,
            0.7,
            "no links",
            &["src/a.rs"],
        );
        let b = page(
            "b",
            PageType::Module,
            Status::Reviewed,
            0.7,
            "also no links",
            &["src/a.rs"],
        );
        let md = compute_diff(&a, &b).as_markdown();
        assert!(md.contains("# Diff: `a` ↔ `b`"));
        assert!(md.contains("## Sources"));
        assert!(md.contains("Common (1)"));
        // Wikilinks empty on both sides → "_(none)_".
        assert!(md.contains("## Wikilinks"));
        assert!(md.contains("_(none)_"));
    }

    #[test]
    fn diff_json_round_trips_to_a_serde_value() {
        let a = page(
            "a",
            PageType::Module,
            Status::Reviewed,
            0.7,
            "[[x]]",
            &["s1"],
        );
        let b = page(
            "b",
            PageType::Module,
            Status::Reviewed,
            0.7,
            "[[y]]",
            &["s2"],
        );
        let v = compute_diff(&a, &b).as_json();
        assert_eq!(v["slug_a"], "a");
        assert_eq!(v["wikilinks"]["only_a"][0], "x");
        assert_eq!(v["sources"]["only_b"][0], "s2");
    }

    #[test]
    fn diff_confidence_delta_is_b_minus_a() {
        let a = page("a", PageType::Module, Status::Reviewed, 0.5, "", &[]);
        let b = page("b", PageType::Module, Status::Reviewed, 0.9, "", &[]);
        let r = compute_diff(&a, &b);
        assert!((r.confidence_delta - 0.4).abs() < 1e-9);
    }

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }
}
