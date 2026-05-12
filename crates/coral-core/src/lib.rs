//! Core types and utilities for Coral.

pub mod atomic;
pub mod cache;
pub mod embeddings;
pub mod embeddings_sqlite;
pub mod error;
pub mod eval;
pub mod frontmatter;
pub mod gc;
pub mod git_remote;
pub mod gitdiff;
pub mod governance;
pub mod index;
pub mod llms_txt;
pub mod log;
pub mod narrative;
pub mod page;
pub mod path;
pub mod project;
pub mod search;
pub mod search_index;
pub mod slug;
pub mod storage;
pub mod symbols;
pub mod vocab;
pub mod walk;
pub mod wikilinks;

pub mod late_chunking;
pub mod reranker;

#[cfg(feature = "tantivy")]
pub mod tantivy_backend;

#[cfg(feature = "pgvector")]
pub mod pgvector;

/// Returns the crate version (CARGO_PKG_VERSION).
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_non_empty() {
        assert!(!version().is_empty());
    }
}
