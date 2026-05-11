//! Core types and utilities for Coral.

pub mod atomic;
pub mod cache;
pub mod embeddings;
pub mod embeddings_sqlite;
pub mod error;
pub mod frontmatter;
pub mod gc;
pub mod git_remote;
pub mod gitdiff;
pub mod index;
pub mod log;
pub mod page;
pub mod path;
pub mod project;
pub mod search;
pub mod slug;
pub mod storage;
pub mod walk;
pub mod wikilinks;

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
