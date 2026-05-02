//! Property-based tests for `coral_core::index::WikiIndex` round-trip.
//!
//! Same harness pattern as the other proptest files
//! (ProptestConfig::with_cases(64)).
//!
//! Properties:
//! 1. `index_round_trip` — build a `WikiIndex` with N entries, render
//!    via `to_string()`, parse back. The recovered set of entries
//!    matches (compared as multisets, since `to_string()` sorts entries
//!    by page_type then slug).
//! 2. `index_upsert_idempotent` — calling `upsert` with the same slug
//!    twice results in exactly one entry for that slug; the second
//!    upsert overwrites the first.
//! 3. `index_last_commit_preserved` — the `last_commit` field survives
//!    the to_string / parse round trip exactly.
//! 4. `index_empty_round_trip` — an empty index renders + parses cleanly,
//!    yielding an index with 0 entries and the same `last_commit`.

use chrono::{Duration, TimeZone, Utc};
use coral_core::frontmatter::{Confidence, PageType, Status};
use coral_core::index::{IndexEntry, WikiIndex};
use proptest::prelude::*;
use std::collections::BTreeSet;

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
        Just(PageType::Index),
        Just(PageType::Log),
        Just(PageType::Schema),
        Just(PageType::Readme),
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

fn path_strategy() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9-]{0,8}/[a-z][a-z0-9-]{2,12}\\.md".prop_map(|s| s)
}

/// Confidence rounded to 2 decimal places — the index serializer prints
/// `{:.2}` so the parse round trip is exact only on 2-decimal values.
fn confidence_strategy() -> impl Strategy<Value = Confidence> {
    (0u32..=100).prop_map(|n| Confidence::try_new(n as f64 / 100.0).unwrap())
}

fn commit_strategy() -> impl Strategy<Value = String> {
    "[0-9a-f]{40}".prop_map(|s| s)
}

fn entry_strategy() -> impl Strategy<Value = IndexEntry> {
    (
        slug_strategy(),
        page_type_strategy(),
        path_strategy(),
        confidence_strategy(),
        status_strategy(),
        commit_strategy(),
    )
        .prop_map(
            |(slug, page_type, path, confidence, status, last_updated_commit)| IndexEntry {
                slug,
                page_type,
                path,
                confidence,
                status,
                last_updated_commit,
            },
        )
}

/// 0..=8 entries with unique slugs (the impl treats slugs as primary
/// key for upsert; if we let duplicates through they get collapsed by
/// upsert and the round-trip count check would be off).
fn unique_entries_strategy() -> impl Strategy<Value = Vec<IndexEntry>> {
    prop::collection::vec(entry_strategy(), 0..=8).prop_map(|raw| {
        let mut seen = BTreeSet::new();
        raw.into_iter()
            .filter(|e| seen.insert(e.slug.clone()))
            .collect()
    })
}

fn build_index(commit: &str, entries: Vec<IndexEntry>) -> WikiIndex {
    let mut idx = WikiIndex {
        last_commit: commit.to_string(),
        // Anchor generated_at to a fixed-second timestamp so rfc3339
        // round-trip is byte-identical.
        generated_at: Utc
            .with_ymd_and_hms(2026, 4, 30, 18, 0, 0)
            .single()
            .unwrap()
            + Duration::seconds(0),
        entries: Vec::new(),
    };
    for e in entries {
        idx.upsert(e);
    }
    idx
}

// -----------------------------------------------------------------------------
// Properties
// -----------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// `WikiIndex` → string → `WikiIndex` recovers the same set of
    /// entries (comparing as a set on (slug, page_type, path)) and the
    /// same last_commit. We don't compare ordering — `to_string()`
    /// sorts entries by (page_type, slug) before serializing.
    #[test]
    fn index_round_trip(
        commit in commit_strategy(),
        entries in unique_entries_strategy(),
    ) {
        let idx = build_index(&commit, entries.clone());
        let serialized = idx.to_string().expect("serialize");
        let reparsed = WikiIndex::parse(&serialized).expect("parse");
        prop_assert_eq!(reparsed.last_commit, idx.last_commit);
        prop_assert_eq!(reparsed.entries.len(), idx.entries.len());
        for orig in &idx.entries {
            let found = reparsed.entries.iter().find(|e| e.slug == orig.slug);
            prop_assert!(found.is_some(), "entry {:?} missing", orig.slug);
            let got = found.unwrap();
            prop_assert_eq!(got.page_type, orig.page_type);
            prop_assert_eq!(&got.path, &orig.path);
            prop_assert!((got.confidence.as_f64() - orig.confidence.as_f64()).abs() < 1e-2);
            prop_assert_eq!(got.status, orig.status);
            prop_assert_eq!(&got.last_updated_commit, &orig.last_updated_commit);
        }
    }

    /// Calling `upsert` twice with the same slug results in exactly one
    /// entry for that slug. The second upsert wins (overwrites the
    /// first one's fields).
    #[test]
    fn index_upsert_idempotent(
        commit in commit_strategy(),
        first in entry_strategy(),
        // Build the second from the first by mutating non-slug fields.
        new_path in path_strategy(),
        new_status in status_strategy(),
    ) {
        let mut idx = build_index(&commit, vec![]);
        idx.upsert(first.clone());
        let mut second = first.clone();
        second.path = new_path.clone();
        second.status = new_status;
        idx.upsert(second);
        // Calling once more should be a no-op on count.
        let third = IndexEntry {
            path: new_path.clone(),
            status: new_status,
            ..first.clone()
        };
        idx.upsert(third);
        prop_assert_eq!(idx.entries.len(), 1);
        let only = &idx.entries[0];
        prop_assert_eq!(&only.slug, &first.slug);
        prop_assert_eq!(&only.path, &new_path);
        prop_assert_eq!(only.status, new_status);
    }

    /// `last_commit` survives the round trip exactly. Pinned
    /// separately from the full round-trip property because the field
    /// goes through frontmatter (not the table body) and we want a
    /// regression on it specifically.
    #[test]
    fn index_last_commit_preserved(
        commit in commit_strategy(),
        entries in unique_entries_strategy(),
    ) {
        let idx = build_index(&commit, entries);
        let serialized = idx.to_string().expect("serialize");
        let reparsed = WikiIndex::parse(&serialized).expect("parse");
        prop_assert_eq!(reparsed.last_commit, commit);
    }
}

// -----------------------------------------------------------------------------
// Non-proptest properties (single-shot)
// -----------------------------------------------------------------------------

/// Empty index renders + parses cleanly: 0 entries in, 0 entries out.
#[test]
fn index_empty_round_trip() {
    let idx = WikiIndex::new("zero-commit");
    let serialized = idx.to_string().expect("serialize");
    let reparsed = WikiIndex::parse(&serialized).expect("parse");
    assert_eq!(reparsed.last_commit, "zero-commit");
    assert!(reparsed.entries.is_empty());
}
