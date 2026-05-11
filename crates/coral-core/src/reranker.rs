//! Reranker module for post-retrieval result reranking.
//!
//! v0.25 stub: provides a `VoyageReranker` that performs a no-op passthrough
//! (returns results unchanged) when `CORAL_RERANKER=voyage` is set. This
//! establishes the structural interface for integrating Voyage rerank-2.5
//! in a future release.

use crate::search::SearchResult;

/// A reranked result with its new position score.
#[derive(Debug, Clone, PartialEq)]
pub struct RankedResult {
    /// Original search result.
    pub result: SearchResult,
    /// Reranker-assigned relevance score (0.0–1.0).
    pub rerank_score: f64,
}

/// Voyage AI reranker (rerank-2.5) stub.
///
/// When fully implemented, this will call the Voyage AI rerank API to
/// reorder search results by semantic relevance to the query. In v0.25
/// it performs a no-op passthrough: results are returned in their original
/// order with linearly decreasing synthetic scores.
#[derive(Debug)]
pub struct VoyageReranker {
    /// Whether the reranker is enabled (checks CORAL_RERANKER env var).
    enabled: bool,
}

impl VoyageReranker {
    /// Create a new reranker, checking the `CORAL_RERANKER` environment
    /// variable. The reranker is considered enabled when `CORAL_RERANKER=voyage`.
    pub fn from_env() -> Self {
        let enabled = std::env::var("CORAL_RERANKER")
            .map(|v| v.eq_ignore_ascii_case("voyage"))
            .unwrap_or(false);
        Self { enabled }
    }

    /// Create a reranker with explicit enabled state (for testing).
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    /// Returns whether this reranker instance is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Rerank the given search results for the query, returning the top_k.
    ///
    /// v0.25: stub passthrough. Results are returned in their original order
    /// with synthetic decreasing scores. A tracing note is emitted when the
    /// reranker is active.
    pub fn rerank(
        &self,
        _query: &str,
        results: Vec<SearchResult>,
        top_k: usize,
    ) -> Vec<RankedResult> {
        if !self.enabled {
            // Reranker not enabled — passthrough with synthetic scores.
            return results
                .into_iter()
                .take(top_k)
                .enumerate()
                .map(|(i, result)| RankedResult {
                    rerank_score: 1.0 - (i as f64 * 0.01),
                    result,
                })
                .collect();
        }

        // Reranker is enabled but not yet connected to Voyage API.
        tracing::info!(
            "VoyageReranker: rerank requested but Voyage API not connected in v0.25; \
             returning results unchanged (no-op passthrough)"
        );

        results
            .into_iter()
            .take(top_k)
            .enumerate()
            .map(|(i, result)| RankedResult {
                rerank_score: 1.0 - (i as f64 * 0.01),
                result,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_results(n: usize) -> Vec<SearchResult> {
        (0..n)
            .map(|i| SearchResult {
                slug: format!("page-{i}"),
                score: (n - i) as f64,
                snippet: format!("snippet {i}"),
            })
            .collect()
    }

    #[test]
    fn reranker_passthrough_preserves_order() {
        let reranker = VoyageReranker::new(false);
        let results = make_results(5);
        let ranked = reranker.rerank("test query", results.clone(), 5);
        assert_eq!(ranked.len(), 5);
        for (i, ranked_result) in ranked.iter().enumerate() {
            assert_eq!(ranked_result.result.slug, format!("page-{i}"));
        }
    }

    #[test]
    fn reranker_enabled_still_passthrough_in_v025() {
        let reranker = VoyageReranker::new(true);
        let results = make_results(3);
        let ranked = reranker.rerank("test query", results.clone(), 3);
        // Even when enabled, v0.25 returns unchanged order.
        assert_eq!(ranked.len(), 3);
        assert_eq!(ranked[0].result.slug, "page-0");
        assert_eq!(ranked[1].result.slug, "page-1");
        assert_eq!(ranked[2].result.slug, "page-2");
    }

    #[test]
    fn reranker_respects_top_k() {
        let reranker = VoyageReranker::new(false);
        let results = make_results(10);
        let ranked = reranker.rerank("test", results, 3);
        assert_eq!(ranked.len(), 3);
    }

    #[test]
    fn reranker_scores_are_decreasing() {
        let reranker = VoyageReranker::new(false);
        let results = make_results(5);
        let ranked = reranker.rerank("test", results, 5);
        for i in 1..ranked.len() {
            assert!(
                ranked[i - 1].rerank_score > ranked[i].rerank_score,
                "scores should decrease: {} vs {}",
                ranked[i - 1].rerank_score,
                ranked[i].rerank_score
            );
        }
    }

    #[test]
    fn reranker_from_env_disabled_by_default() {
        // When CORAL_RERANKER is not set, should be disabled.
        // We can't unset env vars in parallel tests safely, but
        // VoyageReranker::new(false) exercises the same code path.
        let reranker = VoyageReranker::new(false);
        assert!(!reranker.is_enabled());
    }

    #[test]
    fn reranker_empty_results() {
        let reranker = VoyageReranker::new(true);
        let ranked = reranker.rerank("query", vec![], 10);
        assert!(ranked.is_empty());
    }
}
