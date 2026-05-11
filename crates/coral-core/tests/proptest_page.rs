//! Property-based tests for `coral_core::page::Page` write/read round-trip.
//!
//! Same harness pattern as the other proptest files
//! (ProptestConfig::with_cases(64)). Filesystem operations use
//! `tempfile::TempDir` for isolation.
//!
//! Properties:
//! 1. `page_write_read_round_trip` — build a `Page` with arbitrary
//!    frontmatter + body, write it to a tempdir, re-parse via
//!    `Page::from_file`, and verify the recovered page matches on the
//!    structured frontmatter fields and body.
//! 2. `page_write_creates_parent_dirs` — writing to a path with
//!    non-existent parent directories succeeds (the impl creates them).
//! 3. `page_write_overwrites_existing` — writing twice to the same
//!    path uses the second content; the first content is gone.
//! 4. `page_write_via_walk_read_pages` — writing a `Page` to a
//!    sub-directory under a tempdir root and then calling
//!    `walk::read_pages(root)` recovers the page via the parallel
//!    walker.

use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
use coral_core::page::Page;
use coral_core::walk::read_pages;
use proptest::prelude::*;
use std::collections::BTreeMap;
use tempfile::TempDir;

// -----------------------------------------------------------------------------
// Generators
// -----------------------------------------------------------------------------

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
        prop::collection::vec("[a-z][a-z0-9./-]{0,15}", 0..=3),
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
                    valid_from: None,
                    valid_to: None,
                    superseded_by: None,
                    extra: BTreeMap::new(),
                }
            },
        )
}

/// Body strategy — any printable ASCII text plus some newlines, but
/// constrained to chars the YAML serializer doesn't have to escape and
/// the markdown parser is happy with. We avoid leading `---` to dodge
/// the canonical-blank-line stripping ambiguity.
fn body_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 .,!?_\\n-]{0,200}".prop_map(|s| {
        // Strip leading newlines: the parser strips up to one canonical
        // separator newline, and we want our property to be exact on the
        // body field, not modulo whitespace.
        s.trim_start_matches('\n').to_string()
    })
}

// -----------------------------------------------------------------------------
// Properties
// -----------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Write a `Page` to disk, then read it back via `Page::from_file`.
    /// The recovered page matches on the structured frontmatter fields
    /// (slug, page_type, status, confidence, sources, backlinks) and on
    /// the body content. Each case uses a fresh `TempDir` for isolation.
    #[test]
    fn page_write_read_round_trip(
        fm in frontmatter_strategy(),
        body in body_strategy(),
    ) {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join(format!("{}.md", fm.slug));
        let page = Page {
            path: path.clone(),
            frontmatter: fm.clone(),
            body: body.clone(),
        };
        page.write().expect("write");
        prop_assert!(path.exists(), "wrote file should exist at {path:?}");
        let recovered = Page::from_file(&path).expect("re-read");
        prop_assert_eq!(&recovered.frontmatter.slug, &fm.slug);
        prop_assert_eq!(recovered.frontmatter.page_type, fm.page_type);
        prop_assert_eq!(recovered.frontmatter.status, fm.status);
        prop_assert!(
            (recovered.frontmatter.confidence.as_f64() - fm.confidence.as_f64()).abs() < 1e-9,
        );
        prop_assert_eq!(&recovered.frontmatter.sources, &fm.sources);
        prop_assert_eq!(&recovered.frontmatter.backlinks, &fm.backlinks);
        prop_assert_eq!(&recovered.body, &body);
    }

    /// Writing to a path whose parent directory does not yet exist
    /// succeeds: `Page::write` creates intermediate directories.
    #[test]
    fn page_write_creates_parent_dirs(
        fm in frontmatter_strategy(),
        body in body_strategy(),
        depth in 1usize..=4,
    ) {
        let dir = TempDir::new().expect("tempdir");
        // Build a deep path under the tempdir.
        let mut sub = dir.path().to_path_buf();
        for i in 0..depth {
            sub = sub.join(format!("level{i}"));
        }
        let path = sub.join(format!("{}.md", fm.slug));
        let page = Page {
            path: path.clone(),
            frontmatter: fm,
            body,
        };
        page.write().expect("write should create parents");
        prop_assert!(path.exists());
    }

    /// Writing to the same path twice — the second content wins.
    /// We verify by writing a Page with one body, then a Page with a
    /// different body, then re-reading.
    #[test]
    fn page_write_overwrites_existing(
        fm in frontmatter_strategy(),
        body_a in body_strategy(),
        body_b in body_strategy(),
    ) {
        prop_assume!(body_a != body_b);
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join(format!("{}.md", fm.slug));
        let p1 = Page {
            path: path.clone(),
            frontmatter: fm.clone(),
            body: body_a.clone(),
        };
        p1.write().expect("write 1");
        let p2 = Page {
            path: path.clone(),
            frontmatter: fm,
            body: body_b.clone(),
        };
        p2.write().expect("write 2");
        let recovered = Page::from_file(&path).expect("re-read");
        prop_assert_eq!(&recovered.body, &body_b);
        prop_assert_ne!(recovered.body, body_a);
    }
}

// -----------------------------------------------------------------------------
// Single-shot property: round-trip via the parallel walker.
// -----------------------------------------------------------------------------

/// A page written under a tempdir root is recovered by `walk::read_pages`.
/// This pins the contract that the walker discovers + parses pages
/// produced by `Page::write`.
#[test]
fn page_write_via_walk_read_pages() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    let page = Page {
        path: root.join("modules/foo.md"),
        frontmatter: Frontmatter {
            slug: "foo".to_string(),
            page_type: PageType::Module,
            last_updated_commit: "abc".to_string(),
            confidence: Confidence::try_new(0.8).unwrap(),
            sources: vec![],
            backlinks: vec![],
            status: Status::Draft,
            generated_at: None,
            valid_from: None,
            valid_to: None,
            superseded_by: None,
            extra: BTreeMap::new(),
        },
        body: "body line\n".to_string(),
    };
    page.write().expect("write");
    let pages = read_pages(root).expect("walk");
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].frontmatter.slug, "foo");
    assert_eq!(pages[0].body, "body line\n");
}
