//! Diff-narrative generation — summarise what changed between two wiki states.
//!
//! Used by `coral diff --narrative` to produce a human-readable markdown
//! summary of what changed between two git refs (post-merge narrative).

use crate::page::Page;
use std::fmt;

/// The kind of change a page underwent between two wiki snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeType {
    Added,
    Removed,
    Modified,
    Unchanged,
}

impl fmt::Display for ChangeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChangeType::Added => write!(f, "added"),
            ChangeType::Removed => write!(f, "removed"),
            ChangeType::Modified => write!(f, "modified"),
            ChangeType::Unchanged => write!(f, "unchanged"),
        }
    }
}

/// Per-page diff summary between two wiki snapshots.
#[derive(Debug, Clone, PartialEq)]
pub struct PageDiff {
    /// The slug of the page.
    pub slug: String,
    /// Whether the page was added, removed, modified, or unchanged.
    pub change_type: ChangeType,
    /// Change in confidence score (after − before). Zero for added/removed.
    pub confidence_delta: f64,
    /// Change in body length in characters (after − before). Positive means
    /// content was added, negative means content was removed. For newly added
    /// pages the value equals the full body length; for removed pages it
    /// equals the negated body length.
    pub body_delta_chars: i64,
}

/// Compute per-page diffs between two sets of pages (before and after).
///
/// Pages are matched by slug. Pages present only in `after` are `Added`;
/// pages present only in `before` are `Removed`. Pages present in both
/// are `Modified` if their body text differs, otherwise `Unchanged`.
/// `Unchanged` pages are **excluded** from the returned vector.
pub fn diff_wiki_states(pages_before: &[Page], pages_after: &[Page]) -> Vec<PageDiff> {
    use std::collections::BTreeMap;

    let before: BTreeMap<&str, &Page> = pages_before
        .iter()
        .map(|p| (p.frontmatter.slug.as_str(), p))
        .collect();
    let after: BTreeMap<&str, &Page> = pages_after
        .iter()
        .map(|p| (p.frontmatter.slug.as_str(), p))
        .collect();

    let mut diffs = Vec::new();

    // Pages in both snapshots — modified or unchanged.
    for (slug, page_before) in &before {
        if let Some(page_after) = after.get(slug) {
            if page_before.body != page_after.body
                || page_before.frontmatter.confidence != page_after.frontmatter.confidence
                || page_before.frontmatter.status != page_after.frontmatter.status
                || page_before.frontmatter.page_type != page_after.frontmatter.page_type
                || page_before.frontmatter.sources != page_after.frontmatter.sources
            {
                diffs.push(PageDiff {
                    slug: slug.to_string(),
                    change_type: ChangeType::Modified,
                    confidence_delta: page_after.frontmatter.confidence.as_f64()
                        - page_before.frontmatter.confidence.as_f64(),
                    body_delta_chars: page_after.body.chars().count() as i64
                        - page_before.body.chars().count() as i64,
                });
            }
            // Unchanged pages are excluded.
        } else {
            // Present in before but not after → removed.
            diffs.push(PageDiff {
                slug: slug.to_string(),
                change_type: ChangeType::Removed,
                confidence_delta: 0.0,
                body_delta_chars: -(page_before.body.chars().count() as i64),
            });
        }
    }

    // Pages only in after → added.
    for (slug, page_after) in &after {
        if !before.contains_key(slug) {
            diffs.push(PageDiff {
                slug: slug.to_string(),
                change_type: ChangeType::Added,
                confidence_delta: 0.0,
                body_delta_chars: page_after.body.chars().count() as i64,
            });
        }
    }

    // Sort by change type (Added, Modified, Removed) then by slug for
    // deterministic output.
    diffs.sort_by(|a, b| {
        fn type_order(ct: &ChangeType) -> u8 {
            match ct {
                ChangeType::Added => 0,
                ChangeType::Modified => 1,
                ChangeType::Removed => 2,
                ChangeType::Unchanged => 3,
            }
        }
        type_order(&a.change_type)
            .cmp(&type_order(&b.change_type))
            .then_with(|| a.slug.cmp(&b.slug))
    });

    diffs
}

/// Generate a markdown narrative summarising a set of page diffs.
///
/// Groups changes by type (added / removed / modified) and includes an
/// overall confidence-trend summary at the end. Returns an empty-ish
/// document (with a header and "no changes" note) when `changes` is empty.
pub fn generate_narrative(changes: &[PageDiff]) -> String {
    let mut out = String::new();
    out.push_str("# Wiki change narrative\n\n");

    if changes.is_empty() {
        out.push_str("No pages changed.\n");
        return out;
    }

    let added: Vec<&PageDiff> = changes
        .iter()
        .filter(|d| d.change_type == ChangeType::Added)
        .collect();
    let removed: Vec<&PageDiff> = changes
        .iter()
        .filter(|d| d.change_type == ChangeType::Removed)
        .collect();
    let modified: Vec<&PageDiff> = changes
        .iter()
        .filter(|d| d.change_type == ChangeType::Modified)
        .collect();

    if !added.is_empty() {
        out.push_str(&format!("## Added ({})\n\n", added.len()));
        for d in &added {
            out.push_str(&format!(
                "- **{}** — +{} chars\n",
                d.slug, d.body_delta_chars
            ));
        }
        out.push('\n');
    }

    if !removed.is_empty() {
        out.push_str(&format!("## Removed ({})\n\n", removed.len()));
        for d in &removed {
            out.push_str(&format!(
                "- **{}** — {} chars\n",
                d.slug, d.body_delta_chars
            ));
        }
        out.push('\n');
    }

    if !modified.is_empty() {
        out.push_str(&format!("## Modified ({})\n\n", modified.len()));
        for d in &modified {
            let delta_sign = if d.body_delta_chars >= 0 { "+" } else { "" };
            let conf_sign = if d.confidence_delta >= 0.0 { "+" } else { "" };
            out.push_str(&format!(
                "- **{}** — {}{} chars, confidence {}{:.2}\n",
                d.slug, delta_sign, d.body_delta_chars, conf_sign, d.confidence_delta
            ));
        }
        out.push('\n');
    }

    // Confidence trend summary.
    let total_conf_delta: f64 = changes.iter().map(|d| d.confidence_delta).sum();
    let net_chars: i64 = changes.iter().map(|d| d.body_delta_chars).sum();
    out.push_str("## Summary\n\n");
    out.push_str(&format!(
        "- **{}** page(s) changed ({} added, {} removed, {} modified)\n",
        changes.len(),
        added.len(),
        removed.len(),
        modified.len(),
    ));
    let net_sign = if net_chars >= 0 { "+" } else { "" };
    out.push_str(&format!(
        "- Net content: {}{} chars\n",
        net_sign, net_chars
    ));
    let conf_sign = if total_conf_delta >= 0.0 { "+" } else { "" };
    out.push_str(&format!(
        "- Confidence trend: {}{:.2}\n",
        conf_sign, total_conf_delta
    ));

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter::{Confidence, Frontmatter, PageType, Status};
    use std::path::PathBuf;

    /// Helper to build a minimal `Page` for testing.
    fn page(slug: &str, conf: f64, body: &str) -> Page {
        Page {
            path: PathBuf::from(format!(".wiki/x/{slug}.md")),
            frontmatter: Frontmatter {
                slug: slug.into(),
                page_type: PageType::Module,
                last_updated_commit: "abc".into(),
                confidence: Confidence::try_new(conf).unwrap(),
                sources: vec![],
                backlinks: vec![],
                status: Status::Reviewed,
                generated_at: None,
                valid_from: None,
                valid_to: None,
                extra: Default::default(),
            },
            body: body.to_string(),
        }
    }

    #[test]
    fn diff_wiki_states_detects_added_pages() {
        let before: Vec<Page> = vec![];
        let after = vec![page("new-page", 0.8, "hello world")];
        let diffs = diff_wiki_states(&before, &after);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].slug, "new-page");
        assert_eq!(diffs[0].change_type, ChangeType::Added);
        assert_eq!(diffs[0].body_delta_chars, 11); // "hello world".len()
        assert_eq!(diffs[0].confidence_delta, 0.0);
    }

    #[test]
    fn diff_wiki_states_detects_removed_pages() {
        let before = vec![page("old-page", 0.7, "goodbye")];
        let after: Vec<Page> = vec![];
        let diffs = diff_wiki_states(&before, &after);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].slug, "old-page");
        assert_eq!(diffs[0].change_type, ChangeType::Removed);
        assert_eq!(diffs[0].body_delta_chars, -7); // -"goodbye".len()
    }

    #[test]
    fn diff_wiki_states_detects_modified_pages() {
        let before = vec![page("page-a", 0.5, "short")];
        let after = vec![page("page-a", 0.9, "much longer body text")];
        let diffs = diff_wiki_states(&before, &after);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].slug, "page-a");
        assert_eq!(diffs[0].change_type, ChangeType::Modified);
        assert!(
            (diffs[0].confidence_delta - 0.4).abs() < 1e-9,
            "expected +0.4, got {}",
            diffs[0].confidence_delta
        );
        // "much longer body text" (21 chars) - "short" (5 chars) = +16
        assert_eq!(diffs[0].body_delta_chars, 16);
    }

    #[test]
    fn diff_wiki_states_unchanged_pages_excluded() {
        let before = vec![page("stable", 0.7, "same body")];
        let after = vec![page("stable", 0.7, "same body")];
        let diffs = diff_wiki_states(&before, &after);
        assert!(
            diffs.is_empty(),
            "unchanged pages must be excluded; got {diffs:?}"
        );
    }

    #[test]
    fn generate_narrative_groups_by_change_type() {
        let changes = vec![
            PageDiff {
                slug: "new-feature".into(),
                change_type: ChangeType::Added,
                confidence_delta: 0.0,
                body_delta_chars: 200,
            },
            PageDiff {
                slug: "core-module".into(),
                change_type: ChangeType::Modified,
                confidence_delta: 0.1,
                body_delta_chars: 50,
            },
            PageDiff {
                slug: "deprecated".into(),
                change_type: ChangeType::Removed,
                confidence_delta: 0.0,
                body_delta_chars: -300,
            },
        ];
        let md = generate_narrative(&changes);
        assert!(md.contains("## Added (1)"), "missing Added section: {md}");
        assert!(
            md.contains("## Removed (1)"),
            "missing Removed section: {md}"
        );
        assert!(
            md.contains("## Modified (1)"),
            "missing Modified section: {md}"
        );
        assert!(
            md.contains("**new-feature**"),
            "missing added slug: {md}"
        );
        assert!(
            md.contains("**deprecated**"),
            "missing removed slug: {md}"
        );
        assert!(
            md.contains("**core-module**"),
            "missing modified slug: {md}"
        );
        assert!(md.contains("## Summary"), "missing Summary section: {md}");
        assert!(
            md.contains("Confidence trend:"),
            "missing confidence trend: {md}"
        );
    }

    #[test]
    fn generate_narrative_empty_changes() {
        let md = generate_narrative(&[]);
        assert!(
            md.contains("# Wiki change narrative"),
            "missing header: {md}"
        );
        assert!(
            md.contains("No pages changed."),
            "missing empty note: {md}"
        );
        assert!(
            !md.contains("## Added"),
            "should not have Added section: {md}"
        );
    }

    #[test]
    fn change_type_display() {
        assert_eq!(format!("{}", ChangeType::Added), "added");
        assert_eq!(format!("{}", ChangeType::Removed), "removed");
        assert_eq!(format!("{}", ChangeType::Modified), "modified");
        assert_eq!(format!("{}", ChangeType::Unchanged), "unchanged");
    }
}
