//! Parity check: the JSON and SQLite `EmbeddingsIndex` backends must produce
//! the same search results given the same input vectors. This is the
//! contract the CLI relies on when switching backends via the
//! `CORAL_EMBEDDINGS_BACKEND` env var — flipping the toggle must not change
//! ranking, only storage.

use coral_core::embeddings::EmbeddingsIndex;
use coral_core::embeddings_sqlite::SqliteEmbeddingsIndex;

fn fixture_vectors() -> Vec<(&'static str, Vec<f32>)> {
    // Hand-picked, deterministic 4-d vectors with very different cosine
    // profiles so any silent normalisation/encoding bug shows up in the
    // ordering. No two vectors share a direction.
    vec![
        ("alpha", vec![1.0, 0.0, 0.0, 0.0]),
        ("beta", vec![0.0, 1.0, 0.0, 0.0]),
        ("gamma", vec![0.0, 0.0, 1.0, 0.0]),
        ("delta", vec![0.5, 0.5, 0.5, 0.5]),
        ("epsilon", vec![-1.0, 0.0, 0.0, 0.0]),
        ("zeta", vec![0.7, 0.1, 0.1, 0.1]),
    ]
}

#[test]
fn json_and_sqlite_produce_identical_search_results() {
    let dim = 4;
    let provider = "voyage-3";
    let vectors = fixture_vectors();

    let mut json = EmbeddingsIndex::empty(provider, dim);
    let mut sqlite = SqliteEmbeddingsIndex::empty(provider, dim);

    for (slug, vec) in &vectors {
        json.upsert(*slug, 1, vec.clone());
        sqlite.upsert(slug, 1, vec.clone()).unwrap();
    }

    // A handful of query directions — including one biased toward delta and
    // one near gamma — exercise both backends across the cosine spectrum.
    let queries: Vec<Vec<f32>> = vec![
        vec![1.0, 0.0, 0.0, 0.0],
        vec![0.5, 0.5, 0.5, 0.5],
        vec![0.1, 0.1, 0.9, 0.0],
        vec![-0.9, 0.0, 0.0, 0.1],
    ];

    for q in &queries {
        let json_results = json.search(q, 6);
        let sqlite_results = sqlite.search(q, 6).unwrap();
        assert_eq!(
            json_results.len(),
            sqlite_results.len(),
            "result count mismatch for query {q:?}"
        );
        for (a, b) in json_results.iter().zip(sqlite_results.iter()) {
            assert_eq!(
                a.0, b.0,
                "ranking divergence on query {q:?}: json={a:?} sqlite={b:?}"
            );
            // Cosine should match within float tolerance — encoding is
            // lossless f32 LE → f32, but the sum-order may differ slightly
            // depending on iteration order.
            assert!(
                (a.1 - b.1).abs() < 1e-6,
                "score divergence on query {q:?}: json={a:?} sqlite={b:?}"
            );
        }
    }
}

#[test]
fn json_and_sqlite_agree_on_top1_after_prune() {
    // Prune from both, then search — same survivors, same ranking.
    let dim = 3;
    let provider = "voyage-3";

    let mut json = EmbeddingsIndex::empty(provider, dim);
    let mut sqlite = SqliteEmbeddingsIndex::empty(provider, dim);

    for (slug, v) in [
        ("keep-a", vec![1.0, 0.0, 0.0]),
        ("drop-1", vec![0.0, 1.0, 0.0]),
        ("keep-b", vec![0.5, 0.0, 0.5]),
        ("drop-2", vec![0.0, 0.0, 1.0]),
    ] {
        json.upsert(slug, 1, v.clone());
        sqlite.upsert(slug, 1, v).unwrap();
    }

    let mut live = std::collections::HashSet::new();
    live.insert("keep-a".to_string());
    live.insert("keep-b".to_string());

    let json_dropped = json.prune(&live);
    let sqlite_dropped = sqlite.prune(&live).unwrap();
    assert_eq!(json_dropped, sqlite_dropped);
    assert_eq!(json_dropped, 2);

    let q = vec![1.0, 0.0, 0.0];
    let j = json.search(&q, 5);
    let s = sqlite.search(&q, 5).unwrap();
    assert_eq!(j.len(), 2);
    assert_eq!(s.len(), 2);
    assert_eq!(j[0].0, s[0].0);
    assert_eq!(j[1].0, s[1].0);
}
