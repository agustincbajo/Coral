//! Property-based tests for the structural lint engine.
//!
//! These tests complement the hand-crafted unit tests in `src/structural.rs`
//! by exercising the lint pipeline on randomly-generated `Page` graphs.
//! The strategy emits realistic-looking pages (slug-shaped strings, valid
//! confidence values, plausible bodies with embedded wikilinks) so that the
//! aggregator and individual checks are stress-tested for invariants the
//! example-based suite cannot easily cover (panics, sort instability,
//! false-positive bugs in conditional rules).
//!
//! Each property is documented with a doc-comment that states the invariant
//! in plain English. Case counts are dialed down from proptest's default of
//! 256 to 64 to keep the integration test fast (still ample for catching
//! regressions, given the size of the input space we cover).

use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
use coral_core::page::Page;
use coral_lint::report::{LintCode, LintIssue};
use coral_lint::run_structural;
use proptest::collection::vec;
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

// -----------------------------------------------------------------------------
// Generators
// -----------------------------------------------------------------------------

/// Returns a strategy that emits 3–20 character lowercase-alphanumeric-or-dash
/// slugs. Mirrors the shape of slugs the wikilinks regex / structural checks
/// actually see in the wild.
fn slug_strategy() -> impl Strategy<Value = String> {
    "[a-z0-9][a-z0-9-]{2,19}".prop_map(|s| s.to_string())
}

/// Returns a strategy for non-system `PageType` values (Index/Log/Schema/Readme
/// are roots by design; the orphan check skips them and we want the generator
/// to drive the *general* path).
fn non_system_page_type_strategy() -> impl Strategy<Value = PageType> {
    prop_oneof![
        Just(PageType::Module),
        Just(PageType::Concept),
        Just(PageType::Entity),
        Just(PageType::Flow),
        Just(PageType::Decision),
        Just(PageType::Synthesis),
        Just(PageType::Operation),
        Just(PageType::Source),
        Just(PageType::Gap),
        Just(PageType::Reference),
    ]
}

/// Returns a strategy for `Status` variants. All variants are valid.
fn status_strategy() -> impl Strategy<Value = Status> {
    prop_oneof![
        Just(Status::Draft),
        Just(Status::Reviewed),
        Just(Status::Verified),
        Just(Status::Stale),
        Just(Status::Archived),
        Just(Status::Reference),
    ]
}

/// 40-char hex SHA — the shape of a real git commit. None of these will exist
/// in the test repo's git log, but the commit-in-git check skips when git
/// itself is unavailable, so this is just realistic input.
fn sha_strategy() -> impl Strategy<Value = String> {
    "[0-9a-f]{40}".prop_map(|s| s.to_string())
}

/// Returns a strategy for short string sources (path-shaped, no http:// prefix
/// to avoid the URL bypass). 0–3 entries per page.
fn sources_strategy() -> impl Strategy<Value = Vec<String>> {
    vec(
        "src/[a-z0-9_-]{1,12}\\.rs".prop_map(|s| s.to_string()),
        0..=3,
    )
}

/// Returns a strategy for backlink slugs (0–3 entries, slug-shaped).
fn backlinks_strategy() -> impl Strategy<Value = Vec<String>> {
    vec(slug_strategy(), 0..=3)
}

/// Returns a strategy for body text: 0–500 chars of arbitrary printable text
/// with optional embedded `[[wikilink]]` references mixed in.
fn body_strategy() -> impl Strategy<Value = String> {
    let text = "[a-zA-Z0-9 \n.,!?-]{0,500}".prop_map(|s| s.to_string());
    let wikilinks = vec(slug_strategy().prop_map(|s| format!("[[{s}]]")), 0..=5);
    (text, wikilinks).prop_map(|(t, links)| format!("{t} {}", links.join(" ")))
}

/// Maps a `PageType` to its conventional `.wiki/<dir>/` subdirectory.
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
        PageType::Schema => ".",
        PageType::Readme => ".",
        PageType::Reference => "references",
        PageType::Interface => "interfaces",
    }
}

/// Returns a strategy for a single `Page` with a non-system page type.
fn page_strategy() -> impl Strategy<Value = Page> {
    (
        slug_strategy(),
        non_system_page_type_strategy(),
        sha_strategy(),
        // Confidence sampled across the full [0.0, 1.0] range, finite by construction.
        (0.0f64..=1.0f64),
        sources_strategy(),
        backlinks_strategy(),
        status_strategy(),
        body_strategy(),
    )
        .prop_map(
            |(slug, page_type, sha, conf, sources, backlinks, status, body)| {
                let subdir = page_type_subdir(page_type);
                let path = PathBuf::from(format!(".wiki/{subdir}/{slug}.md"));
                Page {
                    path,
                    frontmatter: Frontmatter {
                        slug,
                        page_type,
                        last_updated_commit: sha,
                        confidence: Confidence::try_new(conf)
                            .expect("0.0..=1.0 is always a valid Confidence"),
                        sources,
                        backlinks,
                        status,
                        generated_at: None,
                        valid_from: None,
                        valid_to: None,
                        extra: BTreeMap::new(),
                    },
                    body,
                }
            },
        )
}

/// Returns a strategy for a `Vec<Page>` of 1–20 pages with unique slugs.
/// Dedupes by slug *after* generation to guarantee uniqueness while keeping
/// the strategy simple.
fn pages_strategy() -> impl Strategy<Value = Vec<Page>> {
    vec(page_strategy(), 1..=20).prop_map(|pages| {
        let mut seen = BTreeSet::new();
        pages
            .into_iter()
            .filter(|p| seen.insert(p.frontmatter.slug.clone()))
            .collect()
    })
}

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

/// Serializes a `LintCode` to its snake_case string representation. Used as a
/// stable key in `BTreeSet` (LintCode has Hash but not Ord).
fn code_key(code: LintCode) -> String {
    serde_json::to_value(code)
        .expect("LintCode serializes to JSON")
        .as_str()
        .expect("LintCode serializes as a string")
        .to_string()
}

/// Build the canonical comparison key for an issue: (code, page, message).
fn issue_key(i: &LintIssue) -> (String, Option<PathBuf>, String) {
    (code_key(i.code), i.page.clone(), i.message.clone())
}

// -----------------------------------------------------------------------------
// Properties
// -----------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Property 1: `run_structural` never panics for any pages vec produced by
    /// the strategy. This is the most basic invariant — the entry point is
    /// total over its declared input domain.
    #[test]
    fn property_run_structural_never_panics(pages in pages_strategy()) {
        let _report = run_structural(&pages);
    }

    /// Property 2: every `LintIssue` returned by `run_structural` satisfies
    /// the structural invariants we publish in `LintReport`'s JSON schema —
    /// severity is one of the three known variants (u8 in 0..=2), code
    /// serializes to one of the published snake_case names (round-trip via
    /// serde_json), and any non-`None` `page` field points at a real input
    /// page (the lint engine never fabricates paths).
    #[test]
    fn property_issue_invariants_hold(pages in pages_strategy()) {
        let report = run_structural(&pages);
        let input_paths: BTreeSet<PathBuf> =
            pages.iter().map(|p| p.path.clone()).collect();
        // Snake_case names mirror `#[serde(rename_all = "snake_case")]` on
        // `LintCode`. Updating LintCode without updating this set should
        // intentionally fail this property.
        let known_codes: BTreeSet<&str> = [
            "broken_wikilink",
            "orphan_page",
            "low_confidence",
            "high_confidence_without_sources",
            "stale_status",
            "commit_not_in_git",
            "source_not_found",
            "archived_page_linked",
            "unknown_extra_field",
            "contradiction",
            "obsolete_claim",
        ]
        .into_iter()
        .collect();
        for issue in &report.issues {
            // Severity is always one of the three known variants (u8 ≤ 2).
            let sev_byte: u8 = issue.severity.into();
            prop_assert!(
                sev_byte <= 2,
                "severity byte out of range: {sev_byte} for {issue:?}"
            );

            // Code round-trips through serde_json::to_value (catches accidental
            // enum mutations that would break the published JSON schema
            // contract). The result must be one of the published names.
            let json = serde_json::to_value(issue.code)
                .expect("LintCode must serialize");
            let name = json
                .as_str()
                .expect("LintCode serializes as a JSON string");
            prop_assert!(
                known_codes.contains(name),
                "LintCode serialized to unknown name `{name}`: {issue:?}"
            );

            // If the issue is bound to a page, that page must exist in the
            // input slice. The lint engine never invents paths.
            if let Some(p) = &issue.page {
                prop_assert!(
                    input_paths.contains(p),
                    "issue page {p:?} not in input set: {issue:?}"
                );
            }
        }
    }

    /// Property 4: `run_structural` is order-independent. Shuffling the input
    /// pages produces the SAME set of issues (compared via the canonical
    /// `(code, page, message)` tuple). Catches order-dependent state in any
    /// of the seven pure checks.
    #[test]
    fn property_stable_across_reordering(
        pages in pages_strategy(),
        seed in any::<u64>(),
    ) {
        let report_a = run_structural(&pages);

        // Deterministic shuffle using the seed (simple Fisher-Yates).
        let mut shuffled = pages.clone();
        let mut rng_state = seed.wrapping_add(1);
        for i in (1..shuffled.len()).rev() {
            // xorshift64* — small, deterministic, no extra deps.
            rng_state ^= rng_state >> 12;
            rng_state ^= rng_state << 25;
            rng_state ^= rng_state >> 27;
            let r = rng_state.wrapping_mul(0x2545_F491_4F6C_DD1D);
            let j = (r as usize) % (i + 1);
            shuffled.swap(i, j);
        }
        let report_b = run_structural(&shuffled);

        let set_a: BTreeSet<_> = report_a.issues.iter().map(issue_key).collect();
        let set_b: BTreeSet<_> = report_b.issues.iter().map(issue_key).collect();
        prop_assert_eq!(
            set_a, set_b,
            "lint issue set changed under reordering"
        );
    }

    /// Property 6: `HighConfidenceWithoutSources` only fires for pages whose
    /// frontmatter actually satisfies the predicate (`confidence >= 0.6` AND
    /// `sources.is_empty()`). Walks every reported issue of that code and
    /// resolves the page by path; catches false-positive bugs introduced by
    /// future refactors of the check.
    #[test]
    fn property_high_conf_without_sources_predicate_holds(pages in pages_strategy()) {
        let report = run_structural(&pages);
        let by_path: BTreeMap<PathBuf, &Page> =
            pages.iter().map(|p| (p.path.clone(), p)).collect();
        for issue in &report.issues {
            if issue.code != LintCode::HighConfidenceWithoutSources {
                continue;
            }
            let path = issue.page.as_ref().expect("issue must have a page");
            let page = by_path.get(path).expect("page must exist for issue");
            prop_assert!(
                page.frontmatter.confidence.as_f64() >= 0.6,
                "HighConfidenceWithoutSources fired for confidence < 0.6: {page:?}"
            );
            prop_assert!(
                page.frontmatter.sources.is_empty(),
                "HighConfidenceWithoutSources fired despite sources: {page:?}"
            );
        }
    }
}

// -----------------------------------------------------------------------------
// Non-proptest properties (single-shot or constructed inputs)
// -----------------------------------------------------------------------------

/// Property 3: `run_structural` on an empty pages vec ALWAYS returns an empty
/// `LintReport` (equal to `LintReport::default()`). Trivially established but
/// pinned here so future "warn on empty workspace" features stay opt-in.
#[test]
fn property_empty_pages_yield_empty_report() {
    let report = run_structural(&[]);
    assert!(
        report.issues.is_empty(),
        "empty input must yield empty report, got {report:?}"
    );
    assert_eq!(report, coral_lint::LintReport::default());
}

/// Property 5: `OrphanPage` never fires for system page types
/// (Index / Log / Schema / Readme), regardless of slug or surrounding graph.
/// Constructs one page of each system type with no incoming references and
/// asserts no `OrphanPage` issue is reported.
#[test]
fn property_orphan_skips_system_page_types() {
    let system_types = [
        PageType::Index,
        PageType::Log,
        PageType::Schema,
        PageType::Readme,
        PageType::Interface,
    ];
    for pt in system_types {
        let slug = match pt {
            PageType::Index => "index",
            PageType::Log => "log",
            PageType::Schema => "schema",
            PageType::Readme => "readme",
            PageType::Interface => "interface",
            _ => unreachable!(),
        };
        let page = Page {
            path: PathBuf::from(format!(".wiki/{slug}.md")),
            frontmatter: Frontmatter {
                slug: slug.to_string(),
                page_type: pt,
                last_updated_commit: "abc".to_string(),
                confidence: Confidence::try_new(0.8).unwrap(),
                sources: vec!["src/lib.rs".to_string()],
                backlinks: vec![],
                status: Status::Draft,
                generated_at: None,
                valid_from: None,
                valid_to: None,
                extra: BTreeMap::new(),
            },
            body: String::new(),
        };
        let report = run_structural(std::slice::from_ref(&page));
        let orphan_count = report
            .issues
            .iter()
            .filter(|i| i.code == LintCode::OrphanPage)
            .count();
        assert_eq!(
            orphan_count, 0,
            "system PageType::{pt:?} produced OrphanPage: {report:?}"
        );
    }
}
