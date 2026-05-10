//! Lightweight TF-IDF and BM25 search over a wiki page collection.
//!
//! v0.2: pure TF-IDF on tokens (slug + body). No embeddings, no external
//! APIs. Suitable for wikis up to ~500 pages.
//!
//! v0.3 (issue #5 follow-up): switch to embeddings (Voyage AI or
//! Anthropic), persisted in sqlite-vec or qmd. See ADR 0006.
//!
//! v0.5 (this module): added BM25 as an alternative ranking inside the
//! offline tf-idf family. Same tokenization, same `SearchResult` shape;
//! see [`search_bm25`].

use crate::page::Page;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub slug: String,
    pub score: f64,
    pub snippet: String,
}

/// Returns the top-N pages ranked by TF-IDF relevance for `query`.
/// Tokenization: lowercase, alphanumeric only, single-char tokens dropped.
/// Stopwords: a small Spanish + English list (the, and, of, etc.).
/// Score: sum over query tokens of (term_freq_in_page * idf), normalized
/// by sqrt(page_token_count).
pub fn search(pages: &[Page], query: &str, limit: usize) -> Vec<SearchResult> {
    let query_tokens = tokenize(query);
    if query_tokens.is_empty() || pages.is_empty() {
        return vec![];
    }

    let n_docs = pages.len() as f64;

    // Document frequency per term.
    let mut df: HashMap<String, usize> = HashMap::new();
    let mut tokenized: Vec<(usize, Vec<String>)> = Vec::with_capacity(pages.len());
    for (i, p) in pages.iter().enumerate() {
        let combined = format!("{} {}", p.frontmatter.slug, p.body);
        let tokens = tokenize(&combined);
        let unique: std::collections::HashSet<&String> = tokens.iter().collect();
        for t in unique {
            *df.entry(t.clone()).or_insert(0) += 1;
        }
        tokenized.push((i, tokens));
    }

    // Score each doc.
    let mut results: Vec<SearchResult> = Vec::with_capacity(pages.len());
    for (i, tokens) in &tokenized {
        if tokens.is_empty() {
            continue;
        }
        let tf: HashMap<&String, usize> = tokens.iter().fold(HashMap::new(), |mut acc, t| {
            *acc.entry(t).or_insert(0) += 1;
            acc
        });
        let mut score = 0.0;
        for q in &query_tokens {
            if let Some(&count) = tf.get(q) {
                let df_count = *df.get(q).unwrap_or(&1) as f64;
                let idf = ((n_docs + 1.0) / (df_count + 1.0)).ln() + 1.0;
                score += (count as f64) * idf;
            }
        }
        if score == 0.0 {
            continue;
        }
        let norm = (tokens.len() as f64).sqrt();
        let final_score = score / norm;

        let p = &pages[*i];
        let snippet = build_snippet(&p.body, &query_tokens, 200);
        results.push(SearchResult {
            slug: p.frontmatter.slug.clone(),
            score: final_score,
            snippet,
        });
    }

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(limit);
    results
}

/// BM25 term-frequency saturation parameter.
///
/// `1.5` is the standard general-text default. Larger values give linear
/// reward to repeated occurrences; smaller values saturate faster (a third
/// hit barely adds anything). Exposed as `pub const` so callers (tests,
/// future tuning UIs) can introspect what was actually used.
pub const BM25_K1: f64 = 1.5;

/// BM25 length-normalization parameter in `[0.0, 1.0]`.
///
/// `0.75` is the canonical Robertson/Sparck-Jones default. `0.0` disables
/// length normalization entirely (long pages aren't penalized); `1.0`
/// fully normalizes by `|D|/avgdl`. Exposed as `pub const` for the same
/// reason as [`BM25_K1`].
pub const BM25_B: f64 = 0.75;

/// Returns the top-N pages ranked by Okapi BM25 relevance for `query`.
///
/// Same signature, same `SearchResult` shape, and same tokenization as
/// [`search`] — drop-in alternative ranking. Snippet generation is
/// identical (we delegate to the shared `build_snippet` helper).
///
/// Score formula:
///
/// ```text
/// score(D, Q) = Σ_{q ∈ Q} IDF(q) · (tf(q, D) · (k1 + 1))
///                          / (tf(q, D) + k1 · (1 - b + b · |D|/avgdl))
/// ```
///
/// IDF uses the BM25 variant clamped at 0, so very common terms (those
/// appearing in more than half the corpus) don't push scores negative:
///
/// ```text
/// IDF(q) = ln(1 + (N - n(q) + 0.5) / (n(q) + 0.5))
/// ```
///
/// Constants live in [`BM25_K1`] and [`BM25_B`].
///
/// On 100+ page wikis BM25 generally has better precision than the
/// length-normalized TF-IDF cosine variant in [`search`]; on tiny
/// corpora the two are close to indistinguishable.
pub fn search_bm25(pages: &[Page], query: &str, limit: usize) -> Vec<SearchResult> {
    let query_tokens = tokenize(query);
    if query_tokens.is_empty() || pages.is_empty() {
        return vec![];
    }

    let n_docs = pages.len() as f64;

    // Tokenize every page once; remember total length per doc for avgdl.
    let mut tokenized: Vec<(usize, Vec<String>)> = Vec::with_capacity(pages.len());
    let mut df: HashMap<String, usize> = HashMap::new();
    let mut total_len: usize = 0;
    for (i, p) in pages.iter().enumerate() {
        let combined = format!("{} {}", p.frontmatter.slug, p.body);
        let tokens = tokenize(&combined);
        total_len += tokens.len();
        let unique: std::collections::HashSet<&String> = tokens.iter().collect();
        for t in unique {
            *df.entry(t.clone()).or_insert(0) += 1;
        }
        tokenized.push((i, tokens));
    }

    // Average document length, in tokens. Guard against the all-empty
    // corpus case — every doc would have been skipped below anyway, but
    // we don't want a division-by-zero hiding in the inner loop.
    let avgdl = if !tokenized.is_empty() && total_len > 0 {
        total_len as f64 / n_docs
    } else {
        1.0
    };

    // Precompute IDF per query term once.
    let idf: HashMap<&String, f64> = query_tokens
        .iter()
        .map(|q| {
            let n_q = *df.get(q).unwrap_or(&0) as f64;
            let raw = ((n_docs - n_q + 0.5) / (n_q + 0.5) + 1.0).ln();
            // Clamp at 0: BM25's classic "negative IDF" problem for terms
            // present in > N/2 docs. Better to ignore them than to
            // actively punish docs that contain them.
            (q, raw.max(0.0))
        })
        .collect();

    let mut results: Vec<SearchResult> = Vec::with_capacity(pages.len());
    for (i, tokens) in &tokenized {
        if tokens.is_empty() {
            continue;
        }
        let dl = tokens.len() as f64;
        let length_norm = 1.0 - BM25_B + BM25_B * (dl / avgdl);

        // Term frequency lookup for this doc.
        let tf: HashMap<&String, usize> = tokens.iter().fold(HashMap::new(), |mut acc, t| {
            *acc.entry(t).or_insert(0) += 1;
            acc
        });

        let mut score = 0.0;
        for q in &query_tokens {
            if let Some(&count) = tf.get(q) {
                let f = count as f64;
                let term_idf = *idf.get(q).unwrap_or(&0.0);
                let numerator = f * (BM25_K1 + 1.0);
                let denominator = f + BM25_K1 * length_norm;
                if denominator > 0.0 {
                    score += term_idf * (numerator / denominator);
                }
            }
        }
        if score == 0.0 {
            continue;
        }

        let p = &pages[*i];
        let snippet = build_snippet(&p.body, &query_tokens, 200);
        results.push(SearchResult {
            slug: p.frontmatter.slug.clone(),
            score,
            snippet,
        });
    }

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(limit);
    results
}

fn stopwords() -> &'static HashSet<&'static str> {
    static INSTANCE: OnceLock<HashSet<&'static str>> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        [
            "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "has", "he", "in",
            "is", "it", "its", "of", "on", "that", "the", "to", "was", "were", "will", "with",
            // Spanish
            "el", "la", "los", "las", "de", "y", "en", "que", "es", "se", "un", "una", "para",
            "por", "con", "del", "al",
        ]
        .into_iter()
        .collect()
    })
}

fn tokenize(text: &str) -> Vec<String> {
    let sw = stopwords();
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 1)
        .filter(|t| !sw.contains(t))
        .map(String::from)
        .collect()
}

fn build_snippet(body: &str, query_tokens: &[String], max_len: usize) -> String {
    let lower = body.to_lowercase();
    for q in query_tokens {
        if let Some(pos) = lower.find(q.as_str()) {
            let start = floor_char_boundary(body, pos.saturating_sub(40));
            let end = ceil_char_boundary(body, (pos + q.len() + max_len).min(body.len()));
            return body[start..end].chars().take(max_len).collect::<String>();
        }
    }
    body.chars().take(max_len).collect::<String>()
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char_boundary(s: &str, mut i: usize) -> usize {
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter::{Confidence, Frontmatter, PageType, Status};
    use std::path::PathBuf;

    fn page(slug: &str, body: &str) -> Page {
        Page {
            path: PathBuf::from(format!(".wiki/modules/{slug}.md")),
            frontmatter: Frontmatter {
                slug: slug.to_string(),
                page_type: PageType::Module,
                last_updated_commit: "abc".to_string(),
                confidence: Confidence::try_new(0.8).unwrap(),
                sources: vec![],
                backlinks: vec![],
                status: Status::Reviewed,
                generated_at: None,
                extra: Default::default(),
            },
            body: body.to_string(),
        }
    }

    #[test]
    fn search_empty_pages_returns_empty() {
        assert!(search(&[], "query", 5).is_empty());
    }

    #[test]
    fn search_empty_query_returns_empty() {
        let pages = vec![page("a", "outbox pattern guarantees delivery")];
        assert!(search(&pages, "", 5).is_empty());
    }

    #[test]
    fn search_returns_relevant_pages_first() {
        let pages = vec![
            page("a", "outbox pattern guarantees delivery"),
            page("b", "lorem ipsum dolor"),
            page("c", "the outbox dispatcher polls every second"),
        ];
        let results = search(&pages, "outbox dispatcher", 5);
        assert!(!results.is_empty());
        // c should rank highest (matches both terms)
        assert_eq!(results[0].slug, "c");
    }

    #[test]
    fn search_limits_result_count() {
        let pages: Vec<Page> = (0..20)
            .map(|i| page(&format!("p{i}"), "the outbox handles it"))
            .collect();
        let results = search(&pages, "outbox", 3);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn search_skips_stopwords() {
        let pages = vec![page("a", "the and of"), page("b", "outbox handler")];
        let results = search(&pages, "the and of outbox", 5);
        // Stopwords filtered, only "outbox" matches → b ranks high.
        assert_eq!(results[0].slug, "b");
    }

    #[test]
    fn search_includes_snippet() {
        let body = "lorem ipsum the outbox dispatcher dolor sit amet";
        let pages = vec![page("a", body)];
        let results = search(&pages, "outbox", 5);
        assert!(results[0].snippet.contains("outbox"));
    }

    #[test]
    fn search_does_not_panic_on_multibyte_chars_near_match() {
        // Em-dash (—) is 3 bytes in UTF-8. With the match positioned so that
        // `pos - 40` or `pos + len + max_len` lands inside the em-dash, the
        // previous byte-indexed snippet builder panicked.
        let prefix = "Karpathy's wiki — bypasses RAG with an evolving markdown library";
        let body = format!("{prefix} that uses embeddings under the hood for retrieval.");
        let pages = vec![page("a", &body)];
        let results = search(&pages, "embeddings", 5);
        assert!(!results.is_empty());
        assert!(results[0].snippet.contains("embeddings"));
    }

    // ───────────────────────── BM25 tests ─────────────────────────

    #[test]
    fn bm25_constants_have_spec_defaults() {
        // Robertson/Sparck-Jones canonical defaults; if someone retunes these
        // they should retune intentionally and update this test.
        assert_eq!(BM25_K1, 1.5);
        assert_eq!(BM25_B, 0.75);
    }

    #[test]
    fn search_bm25_empty_pages_returns_empty() {
        assert!(search_bm25(&[], "outbox", 5).is_empty());
    }

    #[test]
    fn search_bm25_empty_query_returns_empty() {
        let pages = vec![page("a", "outbox pattern guarantees delivery")];
        assert!(search_bm25(&pages, "", 5).is_empty());
    }

    #[test]
    fn search_bm25_single_matching_page_ranks_first() {
        let pages = vec![
            page("a", "lorem ipsum dolor sit amet"),
            page("b", "the outbox dispatcher polls every second"),
            page("c", "completely unrelated content here"),
        ];
        let results = search_bm25(&pages, "outbox", 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].slug, "b");
        assert!(
            results[0].score > 0.0,
            "expected positive score, got {}",
            results[0].score
        );
    }

    #[test]
    fn search_bm25_multi_token_query_ranks_both_term_doc_first() {
        let pages = vec![
            page("a", "outbox"),
            page("b", "dispatcher"),
            page("c", "outbox dispatcher polls"),
        ];
        let results = search_bm25(&pages, "outbox dispatcher", 5);
        assert_eq!(results.len(), 3);
        assert_eq!(
            results[0].slug, "c",
            "page c (matches both terms) must rank first; got: {results:?}"
        );
        // a and b each match one term — order between them is fine either way.
        let next_two: Vec<&str> = results[1..].iter().map(|r| r.slug.as_str()).collect();
        assert!(
            next_two.contains(&"a") && next_two.contains(&"b"),
            "expected a and b in remaining slots, got {next_two:?}"
        );
    }

    #[test]
    fn search_bm25_length_normalization_favors_shorter_doc() {
        // b > 0 means BM25 penalizes very long docs containing the query
        // term once vs short docs containing it once. We use a single-term
        // match so length is the only difference.
        let short_body = "outbox".to_string();
        let long_body = format!("outbox {}", "lorem ipsum dolor sit amet ".repeat(50));
        let pages = vec![page("short", &short_body), page("long", &long_body)];
        let results = search_bm25(&pages, "outbox", 5);
        assert_eq!(results.len(), 2);
        assert_eq!(
            results[0].slug, "short",
            "shorter doc should outrank longer doc under BM25 length-norm; got {results:?}"
        );
        let short_score = results.iter().find(|r| r.slug == "short").unwrap().score;
        let long_score = results.iter().find(|r| r.slug == "long").unwrap().score;
        assert!(
            short_score > long_score,
            "short ({short_score}) must score higher than long ({long_score})"
        );
    }

    #[test]
    fn search_bm25_idf_rewards_rarer_terms() {
        // 5 pages all share "the outbox" — "outbox" becomes a common term
        // (IDF small or zero). 1 page has a unique-rare-token. A query for
        // the rare token must produce a positive score for that page.
        let mut pages: Vec<Page> = (0..5)
            .map(|i| page(&format!("common{i}"), "the outbox handler runs"))
            .collect();
        pages.push(page("rare", "outbox unique-rare-token"));

        let rare_results = search_bm25(&pages, "unique-rare-token", 5);
        assert_eq!(
            rare_results.len(),
            1,
            "only the 'rare' page should match the rare token"
        );
        assert_eq!(rare_results[0].slug, "rare");
        assert!(
            rare_results[0].score > 0.0,
            "rare-term hit must have positive score, got {}",
            rare_results[0].score
        );
    }

    #[test]
    fn search_bm25_limit_honored() {
        let pages: Vec<Page> = (0..10)
            .map(|i| page(&format!("p{i}"), "outbox handler runs each cycle"))
            .collect();
        let results = search_bm25(&pages, "outbox", 3);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn search_bm25_stopwords_filtered() {
        // "the" is in STOPWORDS — adding it to the query must not change
        // the ranking vs. the bare term.
        let pages = vec![
            page("a", "outbox pattern guarantees delivery"),
            page("b", "the outbox dispatcher polls every second"),
            page("c", "lorem ipsum dolor"),
        ];
        let bare = search_bm25(&pages, "outbox", 5);
        let with_stopword = search_bm25(&pages, "the outbox", 5);
        let bare_order: Vec<&str> = bare.iter().map(|r| r.slug.as_str()).collect();
        let stopword_order: Vec<&str> = with_stopword.iter().map(|r| r.slug.as_str()).collect();
        assert_eq!(
            bare_order, stopword_order,
            "stopword in query must not change ranking"
        );
    }

    #[test]
    fn search_and_search_bm25_both_empty_for_empty_corpus() {
        assert_eq!(search(&[], "x", 5), search_bm25(&[], "x", 5));
        assert!(search(&[], "x", 5).is_empty());
    }

    #[test]
    fn search_and_search_bm25_are_not_aliases() {
        // Hand-crafted corpus where the two algorithms should disagree on
        // either ranking or score. With one very long doc that contains the
        // query term twice and one short doc that contains it once,
        // TF-IDF's sqrt(N) normalization and BM25's saturating tf weighting
        // produce noticeably different scores.
        let short_body = "outbox handler".to_string();
        let long_body = format!(
            "outbox stuff {} outbox more {}",
            "lorem ipsum dolor sit amet ".repeat(30),
            "consectetur adipiscing elit ".repeat(30)
        );
        let pages = vec![page("short", &short_body), page("long", &long_body)];

        let tfidf = search(&pages, "outbox", 5);
        let bm25 = search_bm25(&pages, "outbox", 5);
        assert_eq!(tfidf.len(), 2);
        assert_eq!(bm25.len(), 2);

        // Both should return the same set of slugs, but scores must differ
        // OR top-1 ordering must differ. (We don't assert which — only that
        // the two algorithms aren't accidentally identical.)
        let same_top = tfidf[0].slug == bm25[0].slug;
        let scores_differ = (tfidf[0].score - bm25[0].score).abs() > 1e-9
            || (tfidf[1].score - bm25[1].score).abs() > 1e-9;
        assert!(
            !same_top || scores_differ,
            "TF-IDF and BM25 must not produce identical results; tfidf={tfidf:?} bm25={bm25:?}"
        );
    }
}
