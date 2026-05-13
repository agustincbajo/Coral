//! Wiki garbage collection analysis.
//!
//! Detects orphan pages, broken wikilinks, stale/broken backlinks, and
//! archived pages that are still referenced by non-archived pages. The
//! output is a read-only report — no mutations.

use crate::frontmatter::{PageType, Status};
use crate::page::Page;
use crate::wikilinks;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// Summary of garbage-collection findings.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GcReport {
    /// Pages with zero inbound wikilinks from other pages and zero
    /// declared backlinks. Index pages are excluded.
    pub orphans: Vec<String>,
    /// `(source_slug, target_slug)` pairs where the body of
    /// `source_slug` contains a `[[target_slug]]` wikilink but
    /// `target_slug` does not exist in the page set.
    pub broken_wikilinks: Vec<(String, String)>,
    /// `(page_slug, declared_backlink)` pairs where the declared
    /// backlink page does NOT actually link back to `page_slug` in its
    /// body (or doesn't exist at all).
    pub stale_backlinks: Vec<(String, String)>,
    /// `(archived_slug, vec_of_referencing_non_archived_slugs)` for
    /// archived pages that are still wikilinked from live pages.
    pub archived_still_referenced: Vec<(String, Vec<String>)>,
}

impl GcReport {
    /// Returns `true` when all finding lists are empty.
    pub fn is_clean(&self) -> bool {
        self.orphans.is_empty()
            && self.broken_wikilinks.is_empty()
            && self.stale_backlinks.is_empty()
            && self.archived_still_referenced.is_empty()
    }

    /// Total number of individual findings across all categories.
    pub fn total_findings(&self) -> usize {
        self.orphans.len()
            + self.broken_wikilinks.len()
            + self.stale_backlinks.len()
            + self.archived_still_referenced.len()
    }

    /// Renders the report as human-readable Markdown.
    ///
    /// Prefer the free function [`render_markdown`] when you only have
    /// a `&GcReport` reference.
    pub fn to_markdown(&self) -> String {
        render_markdown(self)
    }
}

/// Renders a [`GcReport`] as human-readable Markdown.
pub fn render_markdown(report: &GcReport) -> String {
    let mut out = String::new();
    out.push_str("# Wiki GC Report\n\n");

    if report.is_clean() {
        out.push_str("No issues found — wiki is clean.\n");
        return out;
    }

    // Orphans
    if !report.orphans.is_empty() {
        out.push_str(&format!("## Orphan pages ({})\n\n", report.orphans.len()));
        out.push_str("Pages with no inbound wikilinks and no declared backlinks:\n\n");
        for slug in &report.orphans {
            out.push_str(&format!("- `{slug}`\n"));
        }
        out.push('\n');
    }

    // Broken wikilinks
    if !report.broken_wikilinks.is_empty() {
        out.push_str(&format!(
            "## Broken wikilinks ({})\n\n",
            report.broken_wikilinks.len()
        ));
        out.push_str("Wikilinks whose target page does not exist:\n\n");
        for (source, target) in &report.broken_wikilinks {
            out.push_str(&format!("- `{source}` → `[[{target}]]`\n"));
        }
        out.push('\n');
    }

    // Stale backlinks
    if !report.stale_backlinks.is_empty() {
        out.push_str(&format!(
            "## Stale backlinks ({})\n\n",
            report.stale_backlinks.len()
        ));
        out.push_str(
            "Declared `backlinks:` entries where the referenced page doesn't actually link back:\n\n",
        );
        for (page, bl) in &report.stale_backlinks {
            out.push_str(&format!("- `{page}` declares backlink from `{bl}`\n"));
        }
        out.push('\n');
    }

    // Archived still referenced
    if !report.archived_still_referenced.is_empty() {
        out.push_str(&format!(
            "## Archived pages still referenced ({})\n\n",
            report.archived_still_referenced.len()
        ));
        out.push_str("Archived pages that are still wikilinked from non-archived pages:\n\n");
        for (archived, refs) in &report.archived_still_referenced {
            let ref_list = refs
                .iter()
                .map(|r| format!("`{r}`"))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("- `{archived}` ← {ref_list}\n"));
        }
        out.push('\n');
    }

    out.push_str(&format!(
        "**Total findings: {}**\n",
        report.total_findings()
    ));
    out
}

/// Renders a [`GcReport`] as pretty-printed JSON.
///
/// # Panics
///
/// Never panics — `GcReport` is a plain `#[derive(Serialize)]` struct with
/// no failure modes (no `serialize_with` closures, no maps with
/// non-string keys). The `expect` is documentation, not error handling.
#[allow(
    clippy::expect_used,
    reason = "GcReport is a pure data struct; serde_json cannot fail"
)]
pub fn render_json(report: &GcReport) -> String {
    serde_json::to_string_pretty(report).expect("GcReport is always serializable")
}

/// Analyse a set of wiki pages and return a [`GcReport`].
///
/// The analysis is purely in-memory and performs no disk I/O.
///
/// 1. **Orphans** — pages that receive zero inbound wikilinks from any
///    other page body AND have an empty `backlinks` frontmatter field.
///    Pages whose type is `Index` are excluded (they are structural
///    roots).
///
/// 2. **Broken wikilinks** — a page body contains `[[target]]` but
///    `target` does not exist in the page set.
///
/// 3. **Stale backlinks** — a page declares `backlinks: [slug-x]` but
///    `slug-x` does not contain a `[[this-page]]` wikilink in its body
///    (or `slug-x` doesn't even exist in the page set).
///
/// 4. **Archived still referenced** — a page with `status: archived` is
///    still wikilinked from at least one non-archived page body.
pub fn analyze(pages: &[Page]) -> GcReport {
    let slug_set: HashSet<&str> = pages.iter().map(|p| p.frontmatter.slug.as_str()).collect();

    // Pre-compute body wikilinks per page (owned Strings, no backlinks mixed in).
    let body_links: Vec<(&str, Vec<String>)> = pages
        .iter()
        .map(|p| (p.frontmatter.slug.as_str(), wikilinks::extract(&p.body)))
        .collect();

    // Inbound map: slug → set of slugs whose *body* wikilinks mention it.
    let mut inbound: HashMap<&str, Vec<&str>> = HashMap::new();
    for slug in &slug_set {
        inbound.insert(slug, Vec::new());
    }
    for (from_slug, links) in &body_links {
        for link in links {
            if let Some(entry) = inbound.get_mut(link.as_str()) {
                entry.push(from_slug);
            }
        }
    }

    let mut report = GcReport::default();

    // ── 1. Orphans ───────────────────────────────────────────────────
    for p in pages {
        let slug = p.frontmatter.slug.as_str();
        // Index pages are structural roots — never orphans.
        if p.frontmatter.page_type == PageType::Index {
            continue;
        }
        let has_inbound = inbound.get(slug).is_some_and(|v| !v.is_empty());
        let has_backlinks = !p.frontmatter.backlinks.is_empty();
        if !has_inbound && !has_backlinks {
            report.orphans.push(slug.to_string());
        }
    }
    report.orphans.sort();

    // ── 2. Broken wikilinks ─────────────────────────────────────────
    for (from_slug, links) in &body_links {
        for link in links {
            if !slug_set.contains(link.as_str()) {
                report
                    .broken_wikilinks
                    .push((from_slug.to_string(), link.clone()));
            }
        }
    }
    report
        .broken_wikilinks
        .sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    // ── 3. Stale backlinks ───────────────────────────────────────────
    for p in pages {
        let my_slug = &p.frontmatter.slug;
        for bl in &p.frontmatter.backlinks {
            // `bl` claims to link to this page. Verify that `bl`'s body
            // actually contains a wikilink to `my_slug`.
            let bl_links_here = body_links
                .iter()
                .find(|(s, _)| *s == bl.as_str())
                .map(|(_, links)| links.iter().any(|l| l == my_slug))
                .unwrap_or(false); // bl doesn't exist ⇒ stale
            if !bl_links_here {
                report.stale_backlinks.push((my_slug.clone(), bl.clone()));
            }
        }
    }
    report
        .stale_backlinks
        .sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    // ── 4. Archived pages still referenced ───────────────────────────
    let status_map: HashMap<&str, Status> = pages
        .iter()
        .map(|p| (p.frontmatter.slug.as_str(), p.frontmatter.status))
        .collect();

    for p in pages {
        if p.frontmatter.status != Status::Archived {
            continue;
        }
        let slug = p.frontmatter.slug.as_str();
        let refs: Vec<String> = inbound
            .get(slug)
            .map(|v| {
                v.iter()
                    .filter(|s| {
                        status_map
                            .get(**s)
                            .map(|st| *st != Status::Archived)
                            .unwrap_or(false)
                    })
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();
        if !refs.is_empty() {
            let mut sorted_refs = refs;
            sorted_refs.sort();
            sorted_refs.dedup();
            report
                .archived_still_referenced
                .push((slug.to_string(), sorted_refs));
        }
    }
    report
        .archived_still_referenced
        .sort_by(|a, b| a.0.cmp(&b.0));

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter::{Confidence, Frontmatter};
    use std::path::PathBuf;

    fn make_page(
        slug: &str,
        page_type: PageType,
        status: Status,
        body: &str,
        backlinks: Vec<&str>,
    ) -> Page {
        Page {
            path: PathBuf::from(format!(".wiki/{slug}.md")),
            frontmatter: Frontmatter {
                slug: slug.to_string(),
                page_type,
                last_updated_commit: "abc123".to_string(),
                confidence: Confidence::try_new(0.8).unwrap(),
                sources: vec![],
                backlinks: backlinks.into_iter().map(String::from).collect(),
                status,
                generated_at: None,
                valid_from: None,
                valid_to: None,
                superseded_by: None,
                extra: Default::default(),
            },
            body: body.to_string(),
        }
    }

    // ── Orphan detection ─────────────────────────────────────────────

    #[test]
    fn analyze_detects_orphaned_pages() {
        // alpha ↔ beta form a cycle; orphan has no links in or out.
        let pages = vec![
            make_page(
                "alpha",
                PageType::Module,
                Status::Reviewed,
                "See [[beta]]",
                vec![],
            ),
            make_page(
                "beta",
                PageType::Concept,
                Status::Reviewed,
                "See [[alpha]]",
                vec![],
            ),
            make_page(
                "orphan",
                PageType::Module,
                Status::Reviewed,
                "Lonely page",
                vec![],
            ),
        ];
        let report = analyze(&pages);
        assert_eq!(report.orphans, vec!["orphan"]);
    }

    #[test]
    fn index_page_is_never_orphan() {
        let pages = vec![make_page(
            "master-index",
            PageType::Index,
            Status::Reviewed,
            "Welcome",
            vec![],
        )];
        let report = analyze(&pages);
        assert!(report.orphans.is_empty(), "index pages must not be orphans");
    }

    #[test]
    fn backlink_not_orphan() {
        let pages = vec![make_page(
            "alpha",
            PageType::Module,
            Status::Reviewed,
            "Content",
            vec!["external"],
        )];
        let report = analyze(&pages);
        assert!(
            report.orphans.is_empty(),
            "page with backlinks is not an orphan"
        );
    }

    // ── Broken wikilinks ─────────────────────────────────────────────

    #[test]
    fn analyze_detects_broken_wikilinks() {
        let pages = vec![
            make_page(
                "alpha",
                PageType::Module,
                Status::Reviewed,
                "See [[nonexistent]] and [[beta]]",
                vec![],
            ),
            make_page(
                "beta",
                PageType::Concept,
                Status::Reviewed,
                "See [[alpha]]",
                vec![],
            ),
        ];
        let report = analyze(&pages);
        assert_eq!(
            report.broken_wikilinks,
            vec![("alpha".to_string(), "nonexistent".to_string())]
        );
    }

    #[test]
    fn valid_wikilink_not_broken() {
        let pages = vec![
            make_page(
                "alpha",
                PageType::Module,
                Status::Reviewed,
                "See [[beta]]",
                vec![],
            ),
            make_page(
                "beta",
                PageType::Concept,
                Status::Reviewed,
                "See [[alpha]]",
                vec![],
            ),
        ];
        let report = analyze(&pages);
        assert!(report.broken_wikilinks.is_empty());
    }

    // ── Stale backlinks ──────────────────────────────────────────────

    #[test]
    fn analyze_detects_broken_backlinks() {
        // `beta` declares backlink from `alpha`, but alpha doesn't link to beta.
        let pages = vec![
            make_page(
                "alpha",
                PageType::Module,
                Status::Reviewed,
                "Nothing here",
                vec![],
            ),
            make_page(
                "beta",
                PageType::Concept,
                Status::Reviewed,
                "Content",
                vec!["alpha"],
            ),
        ];
        let report = analyze(&pages);
        assert_eq!(
            report.stale_backlinks,
            vec![("beta".to_string(), "alpha".to_string())]
        );
    }

    #[test]
    fn valid_backlink_not_stale() {
        // `beta` declares backlink from `alpha`, and alpha DOES link to beta.
        let pages = vec![
            make_page(
                "alpha",
                PageType::Module,
                Status::Reviewed,
                "See [[beta]]",
                vec![],
            ),
            make_page(
                "beta",
                PageType::Concept,
                Status::Reviewed,
                "Content",
                vec!["alpha"],
            ),
        ];
        let report = analyze(&pages);
        assert!(report.stale_backlinks.is_empty());
    }

    #[test]
    fn nonexistent_backlink_is_stale() {
        let pages = vec![make_page(
            "beta",
            PageType::Concept,
            Status::Reviewed,
            "Content",
            vec!["ghost"],
        )];
        let report = analyze(&pages);
        assert_eq!(
            report.stale_backlinks,
            vec![("beta".to_string(), "ghost".to_string())]
        );
    }

    // ── Archived still referenced ────────────────────────────────────

    #[test]
    fn analyze_detects_archived_with_live_refs() {
        let pages = vec![
            make_page(
                "alive",
                PageType::Module,
                Status::Reviewed,
                "See [[old-stuff]]",
                vec![],
            ),
            make_page(
                "old-stuff",
                PageType::Concept,
                Status::Archived,
                "Archived content",
                vec![],
            ),
        ];
        let report = analyze(&pages);
        assert_eq!(
            report.archived_still_referenced,
            vec![("old-stuff".to_string(), vec!["alive".to_string()])]
        );
    }

    #[test]
    fn archived_referenced_by_archived_is_ok() {
        let pages = vec![
            make_page(
                "old-a",
                PageType::Module,
                Status::Archived,
                "See [[old-b]]",
                vec![],
            ),
            make_page(
                "old-b",
                PageType::Concept,
                Status::Archived,
                "Old content",
                vec![],
            ),
        ];
        let report = analyze(&pages);
        assert!(
            report.archived_still_referenced.is_empty(),
            "archived-to-archived references should not be flagged"
        );
    }

    // ── Clean wiki ───────────────────────────────────────────────────

    #[test]
    fn analyze_clean_wiki_returns_empty_report() {
        let pages = vec![
            make_page(
                "index",
                PageType::Index,
                Status::Reviewed,
                "Welcome",
                vec![],
            ),
            make_page(
                "alpha",
                PageType::Module,
                Status::Reviewed,
                "See [[beta]]",
                vec![],
            ),
            make_page(
                "beta",
                PageType::Concept,
                Status::Reviewed,
                "See [[alpha]]",
                vec![],
            ),
        ];
        let report = analyze(&pages);
        assert!(report.is_clean(), "expected clean report: {report:?}");
    }

    // ── Rendering ────────────────────────────────────────────────────

    #[test]
    fn render_markdown_formats_correctly() {
        let report = GcReport {
            orphans: vec!["lonely".to_string()],
            broken_wikilinks: vec![("alpha".to_string(), "missing".to_string())],
            stale_backlinks: vec![("a".to_string(), "b".to_string())],
            archived_still_referenced: vec![("old".to_string(), vec!["live".to_string()])],
        };
        let md = render_markdown(&report);
        assert!(md.contains("## Orphan pages (1)"));
        assert!(md.contains("## Broken wikilinks (1)"));
        assert!(md.contains("## Stale backlinks (1)"));
        assert!(md.contains("## Archived pages still referenced (1)"));
        assert!(md.contains("**Total findings: 4**"));
        // Also verify the method delegates correctly.
        assert_eq!(md, report.to_markdown());
    }

    #[test]
    fn render_json_is_valid() {
        let report = GcReport {
            orphans: vec!["lonely".to_string()],
            broken_wikilinks: vec![("src".to_string(), "tgt".to_string())],
            stale_backlinks: vec![("a".to_string(), "b".to_string())],
            archived_still_referenced: vec![("old".to_string(), vec!["live".to_string()])],
        };
        let json = render_json(&report);
        let parsed: serde_json::Value =
            serde_json::from_str(&json).expect("render_json must produce valid JSON");
        assert!(parsed["orphans"].is_array());
        assert!(parsed["broken_wikilinks"].is_array());
        assert!(parsed["stale_backlinks"].is_array());
        assert!(parsed["archived_still_referenced"].is_array());
    }

    #[test]
    fn clean_markdown_report() {
        let report = GcReport::default();
        let md = render_markdown(&report);
        assert!(md.contains("No issues found"));
    }
}
