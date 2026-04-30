//! Structural lint checks — pure functions over `&[Page]`.

use crate::report::{LintCode, LintIssue, LintSeverity};
use coral_core::frontmatter::{PageType, Status};
use coral_core::page::Page;
use std::collections::{HashMap, HashSet};

/// Reports a `BrokenWikilink` Critical for any outbound wikilink whose target
/// is not the slug of any page in the workspace.
pub fn check_broken_wikilinks(pages: &[Page]) -> Vec<LintIssue> {
    let slugs: HashSet<&str> = pages.iter().map(|p| p.frontmatter.slug.as_str()).collect();

    let mut issues = Vec::new();
    for page in pages {
        for link in page.outbound_links() {
            if !slugs.contains(link.as_str()) {
                issues.push(LintIssue {
                    code: LintCode::BrokenWikilink,
                    severity: LintSeverity::Critical,
                    page: Some(page.path.clone()),
                    message: format!("Wikilink target '{link}' has no matching page"),
                    context: Some(link.clone()),
                });
            }
        }
    }
    issues
}

/// Reports an `OrphanPage` Warning for any page that has zero incoming backlinks
/// AND zero references in any other page's body. Skips system pages
/// (PageType::Index, Log, Schema, Readme) — those are roots by design.
pub fn check_orphan_pages(pages: &[Page]) -> Vec<LintIssue> {
    let mut inbound: HashMap<String, usize> = HashMap::new();
    for page in pages {
        for link in page.outbound_links() {
            *inbound.entry(link).or_insert(0) += 1;
        }
    }

    let mut issues = Vec::new();
    for page in pages {
        if matches!(
            page.frontmatter.page_type,
            PageType::Index | PageType::Log | PageType::Schema | PageType::Readme
        ) {
            continue;
        }
        let count = inbound.get(&page.frontmatter.slug).copied().unwrap_or(0);
        if count == 0 {
            issues.push(LintIssue {
                code: LintCode::OrphanPage,
                severity: LintSeverity::Warning,
                page: Some(page.path.clone()),
                message: format!("Page '{}' has no inbound backlinks", page.frontmatter.slug),
                context: None,
            });
        }
    }
    issues
}

/// Reports `LowConfidence` for pages with confidence < 0.6.
/// Severity: Critical if < 0.3, Warning otherwise.
/// Skips pages with status == Reference (they're examples, exempt).
pub fn check_low_confidence(pages: &[Page]) -> Vec<LintIssue> {
    let mut issues = Vec::new();
    for page in pages {
        if page.frontmatter.status == Status::Reference {
            continue;
        }
        let conf = page.frontmatter.confidence.as_f64();
        if conf < 0.3 {
            issues.push(LintIssue {
                code: LintCode::LowConfidence,
                severity: LintSeverity::Critical,
                page: Some(page.path.clone()),
                message: format!("Confidence {conf} below critical threshold 0.3"),
                context: None,
            });
        } else if conf < 0.6 {
            issues.push(LintIssue {
                code: LintCode::LowConfidence,
                severity: LintSeverity::Warning,
                page: Some(page.path.clone()),
                message: format!("Confidence {conf} below threshold 0.6"),
                context: None,
            });
        }
    }
    issues
}

/// Reports a `HighConfidenceWithoutSources` Warning for any page with
/// confidence >= 0.6 but `sources` field is empty.
pub fn check_high_confidence_without_sources(pages: &[Page]) -> Vec<LintIssue> {
    let mut issues = Vec::new();
    for page in pages {
        if page.frontmatter.confidence.as_f64() >= 0.6 && page.frontmatter.sources.is_empty() {
            issues.push(LintIssue {
                code: LintCode::HighConfidenceWithoutSources,
                severity: LintSeverity::Warning,
                page: Some(page.path.clone()),
                message: format!(
                    "Page '{}' has confidence >= 0.6 but no sources listed",
                    page.frontmatter.slug
                ),
                context: None,
            });
        }
    }
    issues
}

/// Reports a `StaleStatus` Info for any page with status == Stale.
/// (This just surfaces explicit `stale` markings; staleness *detection* via
/// commit age is a future check.)
pub fn check_stale_status(pages: &[Page]) -> Vec<LintIssue> {
    let mut issues = Vec::new();
    for page in pages {
        if page.frontmatter.status == Status::Stale {
            issues.push(LintIssue {
                code: LintCode::StaleStatus,
                severity: LintSeverity::Info,
                page: Some(page.path.clone()),
                message: "Page marked as stale".to_string(),
                context: None,
            });
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn mk_page(
        slug: &str,
        page_type: PageType,
        body: &str,
        confidence: f64,
        status: Status,
        sources: Vec<&str>,
    ) -> Page {
        Page {
            path: PathBuf::from(format!(".wiki/modules/{slug}.md")),
            frontmatter: Frontmatter {
                slug: slug.to_string(),
                page_type,
                last_updated_commit: "abc".to_string(),
                confidence: Confidence::try_new(confidence).unwrap(),
                sources: sources.into_iter().map(String::from).collect(),
                backlinks: vec![],
                status,
                generated_at: None,
                extra: BTreeMap::new(),
            },
            body: body.to_string(),
        }
    }

    // --- broken wikilinks -----------------------------------------------------

    #[test]
    fn broken_wikilink_critical() {
        let pages = vec![
            mk_page(
                "a",
                PageType::Module,
                "see [[nonexistent]]",
                0.8,
                Status::Draft,
                vec!["src/a.rs"],
            ),
            mk_page(
                "b",
                PageType::Module,
                "body",
                0.8,
                Status::Draft,
                vec!["src/b.rs"],
            ),
        ];
        let issues = check_broken_wikilinks(&pages);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, LintCode::BrokenWikilink);
        assert_eq!(issues[0].severity, LintSeverity::Critical);
        assert_eq!(issues[0].context.as_deref(), Some("nonexistent"));
    }

    #[test]
    fn wikilink_to_existing_page_no_issue() {
        let pages = vec![
            mk_page(
                "a",
                PageType::Module,
                "see [[b]]",
                0.8,
                Status::Draft,
                vec!["src/a.rs"],
            ),
            mk_page(
                "b",
                PageType::Module,
                "body",
                0.8,
                Status::Draft,
                vec!["src/b.rs"],
            ),
        ];
        let issues = check_broken_wikilinks(&pages);
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    #[test]
    fn wikilink_with_anchor_resolves_to_slug() {
        let pages = vec![
            mk_page(
                "a",
                PageType::Module,
                "see [[b#section]]",
                0.8,
                Status::Draft,
                vec!["src/a.rs"],
            ),
            mk_page(
                "b",
                PageType::Module,
                "body",
                0.8,
                Status::Draft,
                vec!["src/b.rs"],
            ),
        ];
        let issues = check_broken_wikilinks(&pages);
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    // --- orphans --------------------------------------------------------------

    #[test]
    fn orphan_page_emits_warning() {
        // Graph: B → A. C is isolated.
        // Expected orphans: B and C (nobody links to them). A is NOT orphan (B links to A).
        let pages = vec![
            mk_page(
                "a",
                PageType::Module,
                "alone",
                0.8,
                Status::Draft,
                vec!["src/a.rs"],
            ),
            mk_page(
                "b",
                PageType::Module,
                "see [[a]]",
                0.8,
                Status::Draft,
                vec!["src/b.rs"],
            ),
            mk_page(
                "c",
                PageType::Module,
                "lonely",
                0.8,
                Status::Draft,
                vec!["src/c.rs"],
            ),
        ];
        let issues = check_orphan_pages(&pages);
        let orphan_slugs: Vec<&str> = issues
            .iter()
            .filter_map(|i| {
                i.page
                    .as_ref()
                    .and_then(|p| p.file_stem())
                    .map(|s| s.to_str().unwrap())
            })
            .collect();
        assert!(
            orphan_slugs.contains(&"b"),
            "b should be orphan: {issues:?}"
        );
        assert!(
            orphan_slugs.contains(&"c"),
            "c should be orphan: {issues:?}"
        );
        assert!(
            !orphan_slugs.contains(&"a"),
            "a is referenced by b, must NOT be orphan: {issues:?}"
        );
        assert_eq!(issues.len(), 2);
    }

    #[test]
    fn orphan_skips_system_pages() {
        let pages = vec![mk_page(
            "index",
            PageType::Index,
            "nothing",
            0.8,
            Status::Draft,
            vec!["src/i.rs"],
        )];
        let issues = check_orphan_pages(&pages);
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    #[test]
    fn orphan_skips_log_schema_readme() {
        let pages = vec![
            mk_page(
                "log",
                PageType::Log,
                "",
                0.8,
                Status::Draft,
                vec!["src/l.rs"],
            ),
            mk_page(
                "schema",
                PageType::Schema,
                "",
                0.8,
                Status::Draft,
                vec!["src/s.rs"],
            ),
            mk_page(
                "readme",
                PageType::Readme,
                "",
                0.8,
                Status::Draft,
                vec!["src/r.rs"],
            ),
        ];
        let issues = check_orphan_pages(&pages);
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    // --- low confidence -------------------------------------------------------

    #[test]
    fn low_confidence_critical_below_03() {
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.2,
            Status::Draft,
            vec![],
        )];
        let issues = check_low_confidence(&pages);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, LintSeverity::Critical);
        assert_eq!(issues[0].code, LintCode::LowConfidence);
    }

    #[test]
    fn low_confidence_warning_below_06() {
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.5,
            Status::Draft,
            vec![],
        )];
        let issues = check_low_confidence(&pages);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, LintSeverity::Warning);
    }

    #[test]
    fn low_confidence_no_issue_at_or_above_06() {
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.6,
            Status::Draft,
            vec![],
        )];
        let issues = check_low_confidence(&pages);
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    #[test]
    fn low_confidence_skips_reference_status() {
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.1,
            Status::Reference,
            vec![],
        )];
        let issues = check_low_confidence(&pages);
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    // --- high confidence without sources --------------------------------------

    #[test]
    fn high_confidence_without_sources_warns() {
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.7,
            Status::Draft,
            vec![],
        )];
        let issues = check_high_confidence_without_sources(&pages);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, LintSeverity::Warning);
        assert_eq!(issues[0].code, LintCode::HighConfidenceWithoutSources);
    }

    #[test]
    fn high_confidence_with_sources_no_issue() {
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.7,
            Status::Draft,
            vec!["src"],
        )];
        let issues = check_high_confidence_without_sources(&pages);
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    #[test]
    fn low_confidence_without_sources_no_issue() {
        // This SPECIFIC check (high_conf_without_sources) should not emit when conf < 0.6.
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.4,
            Status::Draft,
            vec![],
        )];
        let issues = check_high_confidence_without_sources(&pages);
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    // --- stale status ---------------------------------------------------------

    #[test]
    fn stale_status_emits_info() {
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.8,
            Status::Stale,
            vec!["src"],
        )];
        let issues = check_stale_status(&pages);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, LintSeverity::Info);
        assert_eq!(issues[0].code, LintCode::StaleStatus);
    }

    #[test]
    fn stale_status_other_no_issue() {
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.8,
            Status::Reviewed,
            vec!["src"],
        )];
        let issues = check_stale_status(&pages);
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    // --- run_structural aggregator -------------------------------------------

    #[test]
    fn run_structural_aggregates_all_checks() {
        // Setup:
        // - 1 broken wikilink: page "a" links to "nonexistent"
        // - 1 orphan: page "c" (nobody links to c)
        // - 1 low confidence: page "d" with confidence 0.2 (and conf >= 0.6 isn't here, so
        //   no high-conf-without-sources for d). a is conf 0.8 with sources, so no issue.
        // - "a" is also orphan, but a is referenced if b → a... let's design carefully.
        //
        // Layout:
        //   a (conf 0.8, sources=[src/a.rs], links to "nonexistent") → BrokenWikilink
        //   b (conf 0.8, sources=[src/b.rs], links to "a")           → no issue (b is orphan though)
        //   c (conf 0.8, sources=[src/c.rs])                          → OrphanPage
        //   d (conf 0.2, sources=[src/d.rs], links to "a")            → LowConfidence (also orphan)
        //
        // Expected issues (counting): 1 broken + 3 orphans (a is referenced by b+d so NOT orphan;
        // b, c, d are all orphans) + 1 low conf = 5.
        // Actually a: referenced by b ([[a]]) and d ([[a]]) → not orphan. b: nobody. c: nobody.
        // d: nobody. So orphans = {b, c, d} = 3.
        let pages = vec![
            mk_page(
                "a",
                PageType::Module,
                "see [[nonexistent]]",
                0.8,
                Status::Draft,
                vec!["src/a.rs"],
            ),
            mk_page(
                "b",
                PageType::Module,
                "see [[a]]",
                0.8,
                Status::Draft,
                vec!["src/b.rs"],
            ),
            mk_page(
                "c",
                PageType::Module,
                "",
                0.8,
                Status::Draft,
                vec!["src/c.rs"],
            ),
            mk_page(
                "d",
                PageType::Module,
                "see [[a]]",
                0.2,
                Status::Draft,
                vec!["src/d.rs"],
            ),
        ];
        let report = crate::run_structural(&pages);
        let broken = report
            .issues
            .iter()
            .filter(|i| i.code == LintCode::BrokenWikilink)
            .count();
        let orphans = report
            .issues
            .iter()
            .filter(|i| i.code == LintCode::OrphanPage)
            .count();
        let low_conf = report
            .issues
            .iter()
            .filter(|i| i.code == LintCode::LowConfidence)
            .count();
        assert_eq!(broken, 1, "expected 1 broken wikilink: {report:?}");
        assert_eq!(orphans, 3, "expected 3 orphans (b, c, d): {report:?}");
        assert_eq!(low_conf, 1, "expected 1 low confidence: {report:?}");
    }

    #[test]
    fn run_structural_sorts_by_severity() {
        // 1 critical (broken wikilink), 1 warning (orphan), 1 info (stale).
        let pages = vec![
            mk_page(
                "a",
                PageType::Module,
                "see [[ghost]]",
                0.8,
                Status::Stale,
                vec!["src/a.rs"],
            ),
            mk_page(
                "b",
                PageType::Module,
                "see [[a]]",
                0.8,
                Status::Draft,
                vec!["src/b.rs"],
            ),
        ];
        // Pages: a → broken wikilink (Critical) + stale (Info). b → orphan (Warning).
        let report = crate::run_structural(&pages);
        // The first issue must be critical, the last must be info.
        assert!(!report.issues.is_empty());
        assert_eq!(report.issues[0].severity, LintSeverity::Critical);
        let last = report.issues.last().expect("at least one");
        assert_eq!(last.severity, LintSeverity::Info);
        // And in the middle there's at least one warning.
        let has_warning = report
            .issues
            .iter()
            .any(|i| i.severity == LintSeverity::Warning);
        assert!(has_warning, "expected a warning in: {report:?}");
    }
}
