//! Property-based tests for `coral_core::frontmatter::parse` round-trip.
//!
//! Same harness pattern as the lint / search / wikilinks proptest files.
//! ProptestConfig::with_cases(64).
//!
//! The "round-trip" is: build a `Frontmatter`, render it to YAML via
//! serde, prepend `---\n` and `---\n`, parse back, assert equality on
//! the recovered `Frontmatter` (modulo `extra` ordering, which is
//! stable for `BTreeMap`).
//!
//! Properties:
//! 1. `parse_round_trip` — `Frontmatter` → YAML → `Frontmatter` is
//!    identity.
//! 2. `parse_preserves_body_verbatim` — body bytes after the closing
//!    `---\n` are preserved (modulo the canonical single-empty-line
//!    separator that `parse` strips per its docstring).
//! 3. `parse_rejects_missing_frontmatter` — content that doesn't start
//!    with `---` returns `Err(MissingFrontmatter)` always.
//! 4. `parse_rejects_unterminated` — content with `---` open but no
//!    close returns `Err(UnterminatedFrontmatter)`.

use coral_core::error::CoralError;
use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status, parse};
use proptest::prelude::*;
use std::collections::BTreeMap;

fn page_type_strategy() -> impl Strategy<Value = PageType> {
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
        // Skip Index/Log/Schema/Readme — they're system pages and don't
        // typically appear in user-authored frontmatter.
    ]
}

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

fn slug_strategy() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9-]{2,15}".prop_map(|s| s)
}

fn confidence_strategy() -> impl Strategy<Value = Confidence> {
    (0u32..=100).prop_map(|n| Confidence::try_new(n as f64 / 100.0).unwrap())
}

fn commit_strategy() -> impl Strategy<Value = String> {
    "[0-9a-f]{40}".prop_map(|s| s)
}

fn frontmatter_strategy() -> impl Strategy<Value = Frontmatter> {
    (
        slug_strategy(),
        page_type_strategy(),
        commit_strategy(),
        confidence_strategy(),
        prop::collection::vec("[a-z][a-z0-9./-]{0,20}", 0..=3),
        prop::collection::vec(slug_strategy(), 0..=3),
        status_strategy(),
    )
        .prop_map(
            |(slug, page_type, last_updated_commit, confidence, sources, backlinks, status)| {
                Frontmatter {
                    slug,
                    page_type,
                    last_updated_commit,
                    confidence,
                    sources,
                    backlinks,
                    status,
                    generated_at: None,
                    extra: BTreeMap::new(),
                }
            },
        )
}

fn render_to_markdown(fm: &Frontmatter, body: &str) -> String {
    let yaml = serde_yaml_ng::to_string(fm).expect("Frontmatter always serializes");
    format!("---\n{yaml}---\n\n{body}")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Frontmatter → YAML markdown → Frontmatter is identity (the YAML
    /// serializer + parser are inverses on the structured fields).
    #[test]
    fn parse_round_trip(fm in frontmatter_strategy(), body in "[a-zA-Z0-9 .,!?\\n]{0,200}") {
        let md = render_to_markdown(&fm, &body);
        let (parsed, _parsed_body) = parse(&md, "test.md").expect("round-trip parses");
        prop_assert_eq!(parsed.slug, fm.slug);
        prop_assert_eq!(parsed.page_type, fm.page_type);
        prop_assert_eq!(parsed.last_updated_commit, fm.last_updated_commit);
        prop_assert!((parsed.confidence.as_f64() - fm.confidence.as_f64()).abs() < 1e-9);
        prop_assert_eq!(parsed.sources, fm.sources);
        prop_assert_eq!(parsed.backlinks, fm.backlinks);
        prop_assert_eq!(parsed.status, fm.status);
    }

    /// The body string after the closing `---\n` is preserved
    /// byte-for-byte (modulo the one canonical leading newline `parse`
    /// strips per its docstring).
    #[test]
    fn parse_preserves_body_verbatim(fm in frontmatter_strategy(), body in "[a-zA-Z0-9 .,!?-]{1,200}") {
        let md = render_to_markdown(&fm, &body);
        let (_, parsed_body) = parse(&md, "test.md").expect("parses");
        prop_assert_eq!(parsed_body.as_str(), body.as_str());
    }

    /// Content that doesn't start with `---` is rejected as
    /// MissingFrontmatter, regardless of what follows.
    #[test]
    fn parse_rejects_missing_frontmatter(content in "[a-z][a-zA-Z0-9 \\n]{0,100}") {
        // The strategy guarantees the first char is lowercase alpha — never `-`.
        let result = parse(&content, "test.md");
        let is_missing = matches!(result, Err(CoralError::MissingFrontmatter { .. }));
        prop_assert!(is_missing, "expected MissingFrontmatter for content: {content:?}");
    }
}

#[test]
fn parse_rejects_unterminated_frontmatter() {
    let content = "---\nslug: order\ntype: module\n";
    let result = parse(content, "test.md");
    assert!(matches!(
        result,
        Err(CoralError::UnterminatedFrontmatter { .. })
    ));
}

#[test]
fn parse_handles_empty_body() {
    let content = "---\nslug: order\ntype: module\nlast_updated_commit: abc\nconfidence: 0.5\nstatus: draft\n---\n";
    let (fm, body) = parse(content, "test.md").unwrap();
    assert_eq!(fm.slug, "order");
    assert_eq!(body, "");
}

#[test]
fn parse_handles_body_with_no_separator_blank_line() {
    // Body starts immediately after `---\n` with no canonical blank
    // line — parse should still return the body verbatim.
    let content = "---\nslug: order\ntype: module\nlast_updated_commit: abc\nconfidence: 0.5\nstatus: draft\n---\nFirst body line.\n";
    let (_, body) = parse(content, "test.md").unwrap();
    assert_eq!(body, "First body line.\n");
}
