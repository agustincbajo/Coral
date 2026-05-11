//! Property-based tests for the offline TF-IDF / BM25 search engine.
//!
//! These tests complement the example-based unit tests in `src/search.rs` by
//! exercising both `search()` and `search_bm25()` on randomly-generated page
//! corpora and queries. The strategies emit realistic-looking inputs (slug-
//! shaped strings, body text drawn from a small token vocabulary so queries
//! actually have a chance to match) so that the public ranking entry points
//! are stress-tested for the invariants every search implementation must
//! satisfy: totality (no panics), bounded output, monotone scores, sortedness,
//! and slug-membership.
//!
//! Each property is documented with a doc-comment that states the invariant
//! in plain English. Case counts are dialed down from proptest's default of
//! 256 to 64 to keep the integration test fast (still ample for catching
//! regressions, given the size of the input space we cover).
//!
//! NOTE on TF-IDF vs BM25 set equivalence: the two algorithms use the same
//! tokenization but differ in their IDF and term-frequency weighting. BM25
//! clamps IDF at 0 for terms appearing in more than half the corpus, so it
//! drops those matches entirely. TF-IDF in this module uses
//! `ln((N+1)/(df+1)) + 1`, which is always positive, so it keeps them.
//! Therefore the asserted property is `BM25_slug_set ⊆ TF-IDF_slug_set`,
//! not strict equality.

use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
use coral_core::page::Page;
use coral_core::search::{search, search_bm25};
use proptest::collection::vec;
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

// -----------------------------------------------------------------------------
// Generators
// -----------------------------------------------------------------------------

/// Returns a strategy that emits 3–20 character lowercase-alphanumeric-or-dash
/// slugs. Mirrors the shape of slugs the search engine actually sees in the wild.
fn slug_strategy() -> impl Strategy<Value = String> {
    "[a-z0-9][a-z0-9-]{2,19}".prop_map(|s| s.to_string())
}

/// Returns a strategy for arbitrary `PageType` values. The search engine treats
/// every page identically regardless of type, so we don't filter system types.
fn any_page_type_strategy() -> impl Strategy<Value = PageType> {
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

/// Returns a strategy for `Status` variants.
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

/// A small fixed vocabulary that body and query strategies share. Drawing both
/// from the same set guarantees that we generate "interesting" queries that
/// actually match a non-trivial fraction of corpora — pure random ASCII would
/// almost never collide and most properties would be vacuously true.
const VOCAB: &[&str] = &[
    "outbox",
    "dispatcher",
    "handler",
    "wikilink",
    "frontmatter",
    "embedding",
    "tokenizer",
    "lint",
    "scrub",
    "pipeline",
    "module",
    "concept",
    "synthesis",
    "decision",
    "voyage",
    "anthropic",
    "rust",
    "cargo",
    "agent",
    "graph",
];

/// Returns a strategy for body text: a space-separated sequence of 0–60 tokens,
/// each picked uniformly from `VOCAB`. Optionally mixes in some non-vocab
/// alphanumeric noise so the snippet builder also gets exercised on filler.
fn body_strategy() -> impl Strategy<Value = String> {
    let token = prop::sample::select(VOCAB).prop_map(|s| s.to_string());
    let tokens = vec(token, 0..=60);
    let noise = "[a-zA-Z0-9 ]{0,80}".prop_map(|s| s.to_string());
    (tokens, noise).prop_map(|(toks, n)| {
        let mut out = toks.join(" ");
        if !n.is_empty() {
            out.push(' ');
            out.push_str(&n);
        }
        out
    })
}

/// Returns a strategy for a single `Page`. We don't filter system page types —
/// search treats them all the same.
fn page_strategy() -> impl Strategy<Value = Page> {
    (
        slug_strategy(),
        any_page_type_strategy(),
        // Confidence sampled across the full [0.0, 1.0] range, finite by construction.
        0.0f64..=1.0f64,
        status_strategy(),
        body_strategy(),
    )
        .prop_map(|(slug, page_type, conf, status, body)| {
            let path = PathBuf::from(format!(".wiki/modules/{slug}.md"));
            Page {
                path,
                frontmatter: Frontmatter {
                    slug,
                    page_type,
                    last_updated_commit: "abc1234".to_string(),
                    confidence: Confidence::try_new(conf)
                        .expect("0.0..=1.0 is always a valid Confidence"),
                    sources: vec![],
                    backlinks: vec![],
                    status,
                    generated_at: None,
                    valid_from: None,
                    valid_to: None,
                    superseded_by: None,
                    extra: BTreeMap::new(),
                },
                body,
            }
        })
}

/// Returns a strategy for a `Vec<Page>` of 1–15 pages. Slugs are NOT deduped —
/// the search engine treats duplicate-slug pages as separate documents, and
/// the `slug_membership` property is written to be robust to that.
fn pages_strategy() -> impl Strategy<Value = Vec<Page>> {
    vec(page_strategy(), 1..=15)
}

/// Returns a strategy for a 1–4 token query string drawn from `VOCAB`, joined
/// with spaces. This guarantees most queries hit at least one page in a
/// generated corpus, which is what makes the score / ordering properties
/// non-vacuous.
fn query_strategy() -> impl Strategy<Value = String> {
    vec(
        prop::sample::select(VOCAB).prop_map(|s| s.to_string()),
        1..=4,
    )
    .prop_map(|toks| toks.join(" "))
}

/// Returns a strategy for a "limit" parameter in the realistic 1..=20 range.
/// Zero is exercised separately via a dedicated test (the algorithms truncate
/// to 0, returning empty — already covered by property #2 in spirit).
fn limit_strategy() -> impl Strategy<Value = usize> {
    1usize..=20
}

// -----------------------------------------------------------------------------
// Properties
// -----------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Property 1a: `search` (TF-IDF) never panics on any (pages, query, limit)
    /// drawn from the strategies. Total over its declared input domain.
    #[test]
    fn search_never_panics(
        pages in pages_strategy(),
        query in ".*",
        limit in 0usize..=50,
    ) {
        let _ = search(&pages, &query, limit);
    }

    /// Property 1b: `search_bm25` never panics on any (pages, query, limit)
    /// drawn from the strategies. Total over its declared input domain.
    #[test]
    fn search_bm25_never_panics(
        pages in pages_strategy(),
        query in ".*",
        limit in 0usize..=50,
    ) {
        let _ = search_bm25(&pages, &query, limit);
    }

    /// Property 2: result count is always `<= limit` for both algorithms.
    /// The truncation step is the last thing each `search*` does, so this is
    /// essentially a "the truncate didn't get accidentally removed" guard.
    #[test]
    fn result_count_within_limit(
        pages in pages_strategy(),
        query in query_strategy(),
        limit in limit_strategy(),
    ) {
        let tfidf = search(&pages, &query, limit);
        let bm25 = search_bm25(&pages, &query, limit);
        prop_assert!(tfidf.len() <= limit, "tfidf returned {} > limit {}", tfidf.len(), limit);
        prop_assert!(bm25.len() <= limit, "bm25 returned {} > limit {}", bm25.len(), limit);
    }

    /// Property 3: every score is non-negative for both algorithms. TF-IDF
    /// scores cannot be negative because IDF is `ln((N+1)/(df+1)) + 1 >= 1`
    /// and TF is a count. BM25 scores can't be negative because we clamp the
    /// IDF at 0; this property pins that clamp.
    #[test]
    fn scores_are_non_negative(
        pages in pages_strategy(),
        query in query_strategy(),
        limit in limit_strategy(),
    ) {
        for r in search(&pages, &query, limit) {
            prop_assert!(r.score >= 0.0, "tfidf produced negative score: {r:?}");
        }
        for r in search_bm25(&pages, &query, limit) {
            prop_assert!(r.score >= 0.0, "bm25 produced negative score: {r:?}");
        }
    }

    /// Property 4: results are sorted by score in descending order. Both
    /// algorithms run a `sort_by(|a,b| b.partial_cmp(&a))` at the end; this
    /// catches accidental swap of a/b or removal of the sort.
    #[test]
    fn results_sorted_descending(
        pages in pages_strategy(),
        query in query_strategy(),
        limit in limit_strategy(),
    ) {
        let tfidf = search(&pages, &query, limit);
        prop_assert!(
            tfidf.windows(2).all(|w| w[0].score >= w[1].score),
            "tfidf not sorted descending: {tfidf:?}"
        );
        let bm25 = search_bm25(&pages, &query, limit);
        prop_assert!(
            bm25.windows(2).all(|w| w[0].score >= w[1].score),
            "bm25 not sorted descending: {bm25:?}"
        );
    }

    /// Property 7 (slug membership): every result's `slug` is one of the input
    /// pages' slugs. Don't assert uniqueness across results — the engine
    /// currently treats duplicate-slug pages as separate docs, and that's a
    /// load-bearing fact we're not pinning down here.
    #[test]
    fn result_slugs_are_input_slugs(
        pages in pages_strategy(),
        query in query_strategy(),
        limit in limit_strategy(),
    ) {
        let input_slugs: BTreeSet<String> =
            pages.iter().map(|p| p.frontmatter.slug.clone()).collect();
        for r in search(&pages, &query, limit) {
            prop_assert!(
                input_slugs.contains(&r.slug),
                "tfidf result slug {:?} not in input set", r.slug
            );
        }
        for r in search_bm25(&pages, &query, limit) {
            prop_assert!(
                input_slugs.contains(&r.slug),
                "bm25 result slug {:?} not in input set", r.slug
            );
        }
    }

    /// Property 8 (algorithm cross-check): every slug surfaced by BM25 is also
    /// surfaced by TF-IDF. The reverse is NOT generally true: TF-IDF's IDF
    /// floor (`+1.0`) keeps very-common-term matches alive, while BM25's
    /// classic clamp drops them entirely (zero IDF → zero contribution). So
    /// strict set equality fails on corpora dominated by a single common
    /// query token. The asserted subset relation is the strongest property
    /// that holds across the full input space.
    #[test]
    fn bm25_slug_set_subset_of_tfidf(
        pages in pages_strategy(),
        query in query_strategy(),
        // limit large enough to hold every match — we don't want the truncate
        // step to drop slugs that would otherwise be in both sets.
        limit in 50usize..=200,
    ) {
        let tfidf_slugs: BTreeSet<String> =
            search(&pages, &query, limit).into_iter().map(|r| r.slug).collect();
        let bm25_slugs: BTreeSet<String> =
            search_bm25(&pages, &query, limit).into_iter().map(|r| r.slug).collect();
        prop_assert!(
            bm25_slugs.is_subset(&tfidf_slugs),
            "bm25 surfaced slugs not in tfidf set; bm25={bm25_slugs:?} tfidf={tfidf_slugs:?}"
        );
    }
}

// -----------------------------------------------------------------------------
// Non-proptest properties (single-shot)
// -----------------------------------------------------------------------------

/// Helper: build a minimal `Page` for the constructed-input tests below.
fn make_page(slug: &str, body: &str) -> Page {
    Page {
        path: PathBuf::from(format!(".wiki/modules/{slug}.md")),
        frontmatter: Frontmatter {
            slug: slug.to_string(),
            page_type: PageType::Module,
            last_updated_commit: "abc".to_string(),
            confidence: Confidence::try_new(0.8).expect("valid confidence"),
            sources: vec![],
            backlinks: vec![],
            status: Status::Draft,
            generated_at: None,
            valid_from: None,
            valid_to: None,
            superseded_by: None,
            extra: BTreeMap::new(),
        },
        body: body.to_string(),
    }
}

/// Property 5: empty query string yields empty results for both algorithms,
/// regardless of the corpus. Trivial but pinned so a future "default to all
/// pages on empty query" feature would have to be added intentionally.
#[test]
fn empty_query_returns_empty() {
    let pages = vec![
        make_page("a", "outbox dispatcher handler"),
        make_page("b", "lorem ipsum dolor"),
    ];
    assert!(
        search(&pages, "", 5).is_empty(),
        "tfidf returned results for empty query"
    );
    assert!(
        search_bm25(&pages, "", 5).is_empty(),
        "bm25 returned results for empty query"
    );
}

/// Property 6: empty pages slice yields empty results for both algorithms,
/// regardless of the query.
#[test]
fn empty_pages_returns_empty() {
    assert!(search(&[], "anything", 5).is_empty());
    assert!(search_bm25(&[], "anything", 5).is_empty());
}

/// Property 9: a query containing only tokens that don't appear in any page
/// body returns empty for both algorithms. Picked a deliberately
/// out-of-vocabulary token unlikely to collide with realistic content.
#[test]
fn no_match_query_returns_empty() {
    let pages = vec![
        make_page("a", "outbox dispatcher handler"),
        make_page("b", "lorem ipsum dolor"),
        make_page("c", "embedding pipeline runs"),
    ];
    let results_tfidf = search(&pages, "zzqxgquadrazzz", 5);
    let results_bm25 = search_bm25(&pages, "zzqxgquadrazzz", 5);
    assert!(
        results_tfidf.is_empty(),
        "tfidf found a match for non-existent token: {results_tfidf:?}"
    );
    assert!(
        results_bm25.is_empty(),
        "bm25 found a match for non-existent token: {results_bm25:?}"
    );
}
