//! Property-based tests for `coral_core::embeddings::EmbeddingsIndex`
//! save/load round-trip and search/upsert/prune invariants.
//!
//! Same harness pattern as the other proptest files
//! (ProptestConfig::with_cases(64)). Filesystem operations use
//! `tempfile::TempDir` for isolation.
//!
//! Properties:
//! 1. `embeddings_index_round_trip` — build an index with N
//!    (slug, mtime, vector) entries → save to a tempdir → load → entries
//!    match (slug → mtime, slug → vector componentwise).
//! 2. `embeddings_index_upsert_replaces` — calling `upsert` with the
//!    same slug twice keeps only the second mtime/vector.
//! 3. `embeddings_index_prune_removes_dead_slugs` — `prune(live)` with
//!    a `live` set smaller than the current entries removes only the
//!    non-live slugs; the count of dropped entries equals the
//!    difference.
//! 4. `embeddings_index_search_returns_correct_count` — searching with
//!    `limit = N` over an index of `M` entries (all with non-zero,
//!    well-shaped vectors and a non-zero query) returns
//!    `min(N, M)` results.

use coral_core::EmbeddingsIndex;
use proptest::prelude::*;
use std::collections::HashSet;
use tempfile::TempDir;

// -----------------------------------------------------------------------------
// Generators
// -----------------------------------------------------------------------------

const DIM: usize = 4; // Small fixed dim — fast tests, still exercises the code.

fn slug_strategy() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9-]{2,12}".prop_map(|s| s)
}

fn vector_strategy() -> impl Strategy<Value = Vec<f32>> {
    // Each component in [-1.0, 1.0], with at least one non-zero so the
    // norm isn't 0 (the search impl filters out zero-vector rows).
    prop::collection::vec(-1.0f32..=1.0f32, DIM..=DIM).prop_map(|mut v| {
        // Force at least one non-zero component so cosine similarity is defined.
        if v.iter().all(|x| *x == 0.0) {
            v[0] = 1.0;
        }
        v
    })
}

fn mtime_strategy() -> impl Strategy<Value = i64> {
    1i64..=1_000_000i64
}

fn entries_strategy() -> impl Strategy<Value = Vec<(String, i64, Vec<f32>)>> {
    prop::collection::vec(
        (slug_strategy(), mtime_strategy(), vector_strategy()),
        0..=8,
    )
    .prop_map(|raw| {
        // Dedupe by slug — upsert collapses duplicates, and we want the
        // vec we hand out to map 1:1 with what ends up in the index so
        // the round-trip count check is exact.
        let mut seen = HashSet::new();
        raw.into_iter()
            .filter(|(s, _, _)| seen.insert(s.clone()))
            .collect()
    })
}

fn build_index(entries: Vec<(String, i64, Vec<f32>)>) -> EmbeddingsIndex {
    let mut idx = EmbeddingsIndex::empty("voyage-3", DIM);
    for (slug, mtime, vec) in entries {
        idx.upsert(slug, mtime, vec);
    }
    idx
}

// -----------------------------------------------------------------------------
// Properties
// -----------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Build → save → load is identity on entry slugs, mtimes, and
    /// vectors. Each case uses a fresh `TempDir` for isolation.
    #[test]
    fn embeddings_index_round_trip(entries in entries_strategy()) {
        let idx = build_index(entries.clone());
        let dir = TempDir::new().expect("tempdir");
        idx.save(dir.path()).expect("save");
        let loaded = EmbeddingsIndex::load(dir.path()).expect("load");
        prop_assert_eq!(loaded.entries.len(), idx.entries.len());
        prop_assert_eq!(&loaded.provider, &idx.provider);
        prop_assert_eq!(loaded.dim, idx.dim);
        for (slug, mtime, vec) in &entries {
            let got = loaded.entries.get(slug).expect("slug present after load");
            prop_assert_eq!(got.mtime_secs, *mtime);
            prop_assert_eq!(got.vector.len(), vec.len());
            for (a, b) in got.vector.iter().zip(vec.iter()) {
                // f32 round-trip through serde_json is exact for finite
                // f32 values via `{:e}` style formatting. We use a tiny
                // epsilon to be safe against printer quirks.
                prop_assert!((a - b).abs() < 1e-6, "vector mismatch");
            }
        }
    }

    /// `upsert(slug, ...)` called twice keeps only one entry — the
    /// second one wins (mtime + vector).
    #[test]
    fn embeddings_index_upsert_replaces(
        slug in slug_strategy(),
        m1 in mtime_strategy(),
        v1 in vector_strategy(),
        m2 in mtime_strategy(),
        v2 in vector_strategy(),
    ) {
        let mut idx = EmbeddingsIndex::empty("voyage-3", DIM);
        idx.upsert(slug.clone(), m1, v1.clone());
        idx.upsert(slug.clone(), m2, v2.clone());
        prop_assert_eq!(idx.entries.len(), 1);
        let entry = idx.entries.get(&slug).expect("present");
        prop_assert_eq!(entry.mtime_secs, m2);
        prop_assert_eq!(&entry.vector, &v2);
    }

    /// `prune(live)` removes exactly the non-live slugs. The return
    /// value (count dropped) equals `before - live_intersection_size`.
    #[test]
    fn embeddings_index_prune_removes_dead_slugs(
        entries in entries_strategy(),
        // Pick the first half (or so) as the "live" set; the rest get pruned.
        keep_first in 0usize..=8,
    ) {
        let idx_orig = build_index(entries.clone());
        let mut idx = idx_orig.clone();
        let total = idx.entries.len();
        let live_slugs: HashSet<String> = entries
            .iter()
            .take(keep_first.min(total))
            .map(|(s, _, _)| s.clone())
            .collect();
        let dropped = idx.prune(&live_slugs);
        // Live entries that actually existed in the index.
        let expected_live = idx_orig
            .entries
            .keys()
            .filter(|k| live_slugs.contains(*k))
            .count();
        prop_assert_eq!(dropped, total - expected_live);
        prop_assert_eq!(idx.entries.len(), expected_live);
        // Every remaining slug is in `live_slugs`.
        for k in idx.entries.keys() {
            prop_assert!(live_slugs.contains(k), "non-live slug survived: {k}");
        }
    }

    /// Search with `limit = N` over an index of `M` non-zero entries
    /// returns `min(N, M)` results when the query is also non-zero
    /// and dim-matched. (All our generated vectors have at least one
    /// non-zero component, and the query is constructed the same way.)
    #[test]
    fn embeddings_index_search_returns_correct_count(
        entries in entries_strategy(),
        query in vector_strategy(),
        limit in 0usize..=20,
    ) {
        let idx = build_index(entries);
        let m = idx.entries.len();
        let results = idx.search(&query, limit);
        let expected = if limit == 0 || m == 0 { 0 } else { limit.min(m) };
        prop_assert_eq!(results.len(), expected);
        // All returned slugs must exist in the index.
        for (s, _) in &results {
            prop_assert!(idx.entries.contains_key(s), "stray slug in results: {s}");
        }
        // Results sorted descending by score.
        for w in results.windows(2) {
            prop_assert!(w[0].1 >= w[1].1, "results not descending: {results:?}");
        }
    }
}

// -----------------------------------------------------------------------------
// Non-proptest properties (single-shot)
// -----------------------------------------------------------------------------

/// An empty index round-trips cleanly.
#[test]
fn embeddings_index_empty_round_trip() {
    let idx = EmbeddingsIndex::empty("voyage-3", DIM);
    let dir = TempDir::new().expect("tempdir");
    idx.save(dir.path()).expect("save");
    let loaded = EmbeddingsIndex::load(dir.path()).expect("load");
    assert!(loaded.entries.is_empty());
    assert_eq!(loaded.dim, DIM);
    assert_eq!(loaded.provider, "voyage-3");
}
