// v0.35 ARCH-C1: feature-gated stub demoted to pub(crate). The
// stub items are scaffolding for a future BM25 backend swap; until
// that lands, dead_code is intentional.
#![allow(dead_code)]

//! Tantivy BM25 search backend stub.
//!
//! Gated behind `#[cfg(feature = "tantivy")]`. This module provides the
//! structural skeleton for a full-text search backend powered by the Tantivy
//! library, targeting wikis with 5000+ pages where the in-memory TF-IDF/BM25
//! implementations become impractical.
//!
//! v0.25: stub only — no actual tantivy crate dependency. Methods return
//! descriptive errors indicating the backend is not yet connected.

use crate::search::SearchResult;

/// Error type for Tantivy backend operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TantivyError {
    /// The tantivy backend is structurally defined but not yet connected
    /// to the actual tantivy crate.
    NotConnected(String),
    /// Index does not exist or failed to open.
    IndexNotFound(String),
    /// A query parse error.
    QueryParse(String),
}

impl std::fmt::Display for TantivyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConnected(msg) => write!(f, "tantivy backend not yet connected: {msg}"),
            Self::IndexNotFound(msg) => write!(f, "tantivy index not found: {msg}"),
            Self::QueryParse(msg) => write!(f, "tantivy query parse error: {msg}"),
        }
    }
}

impl std::error::Error for TantivyError {}

/// Configuration for a Tantivy search index.
#[derive(Debug, Clone)]
pub struct TantivyConfig {
    /// Path to the index directory on disk.
    pub index_path: String,
    /// Maximum number of results to return per query.
    pub max_results: usize,
    /// Heap size in bytes for the index writer (future use).
    pub writer_heap_bytes: usize,
}

impl Default for TantivyConfig {
    fn default() -> Self {
        Self {
            index_path: ".wiki/.tantivy".to_string(),
            max_results: 40,
            writer_heap_bytes: 50_000_000, // 50 MB
        }
    }
}

/// Tantivy-backed full-text search for large wikis (5000+ pages).
///
/// v0.25 stub: all methods return `TantivyError::NotConnected`.
#[derive(Debug)]
pub struct TantivySearchBackend {
    config: TantivyConfig,
}

impl TantivySearchBackend {
    /// Create a new backend with the given configuration.
    pub fn new(config: TantivyConfig) -> Self {
        Self { config }
    }

    /// Create a backend with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(TantivyConfig::default())
    }

    /// Returns the configured index path.
    pub fn index_path(&self) -> &str {
        &self.config.index_path
    }

    /// Index a set of pages. Stub: returns error.
    pub fn index_pages(&self, _pages: &[crate::page::Page]) -> Result<usize, TantivyError> {
        Err(TantivyError::NotConnected(
            "index_pages: tantivy crate not linked in v0.25".to_string(),
        ))
    }

    /// Search the index for pages matching `query`. Stub: returns error.
    pub fn search(&self, _query: &str, _limit: usize) -> Result<Vec<SearchResult>, TantivyError> {
        Err(TantivyError::NotConnected(
            "search: tantivy crate not linked in v0.25".to_string(),
        ))
    }

    /// Delete the on-disk index. Stub: returns error.
    pub fn delete_index(&self) -> Result<(), TantivyError> {
        Err(TantivyError::NotConnected(
            "delete_index: tantivy crate not linked in v0.25".to_string(),
        ))
    }

    /// Check whether the index exists and is healthy. Stub: returns error.
    pub fn health_check(&self) -> Result<(), TantivyError> {
        Err(TantivyError::NotConnected(
            "health_check: tantivy crate not linked in v0.25".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tantivy_stub_search_returns_not_connected() {
        let backend = TantivySearchBackend::with_defaults();
        let result = backend.search("test query", 10);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, TantivyError::NotConnected(_)));
        assert!(
            err.to_string().contains("not yet connected"),
            "error message should mention 'not yet connected': {err}"
        );
    }

    #[test]
    fn tantivy_stub_index_returns_not_connected() {
        let backend = TantivySearchBackend::with_defaults();
        let result = backend.index_pages(&[]);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), TantivyError::NotConnected(_)));
    }

    #[test]
    fn tantivy_config_default_values() {
        let config = TantivyConfig::default();
        assert_eq!(config.index_path, ".wiki/.tantivy");
        assert_eq!(config.max_results, 40);
        assert_eq!(config.writer_heap_bytes, 50_000_000);
    }

    #[test]
    fn tantivy_backend_index_path_accessor() {
        let backend = TantivySearchBackend::new(TantivyConfig {
            index_path: "/custom/path".to_string(),
            ..Default::default()
        });
        assert_eq!(backend.index_path(), "/custom/path");
    }
}
