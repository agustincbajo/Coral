//! Lightweight TF-IDF search over a wiki page collection.
//!
//! v0.2: pure TF-IDF on tokens (slug + body). No embeddings, no external
//! APIs. Suitable for wikis up to ~500 pages.
//!
//! v0.3 (issue #5 follow-up): switch to embeddings (Voyage AI or
//! Anthropic), persisted in sqlite-vec or qmd. See ADR 0006.

use crate::page::Page;
use std::collections::HashMap;

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

const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "has", "he", "in", "is", "it",
    "its", "of", "on", "that", "the", "to", "was", "were", "will", "with", // Spanish
    "el", "la", "los", "las", "de", "y", "en", "que", "es", "se", "un", "una", "para", "por",
    "con", "del", "al",
];

fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 1)
        .filter(|t| !STOPWORDS.contains(t))
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
}
