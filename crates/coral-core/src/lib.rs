//! Core types and utilities for Coral.
//!
//! v0.35 Phase C (ARCH-C1): the public module surface was audited
//! against actual cross-crate use. Modules with zero external callers
//! were demoted to `pub(crate)` so they don't accidentally become
//! SemVer-frozen API. The audit grep is recorded in BACKLOG.md.

pub mod atomic;
pub mod auth;
pub mod cache;
pub mod config;
pub mod cost;
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
// v0.35 ARCH-C1: `storage` is the EmbeddingsStorage trait, currently
// consumed only by the feature-gated `pgvector` module inside this
// crate. No external callers — keep crate-private until a downstream
// integration needs the trait. Re-expose via `pub use` at that point.
pub(crate) mod storage;
pub mod symbols;
// v0.35 ARCH-C1: `vocab`, `late_chunking`, and `reranker` are
// scaffolding for future search-relevance work. Zero external uses
// today; demoted to crate-private so the SemVer surface doesn't
// accidentally freeze around an incomplete API.
pub(crate) mod vocab;
pub mod walk;
pub mod wikilinks;

pub(crate) mod late_chunking;
pub(crate) mod reranker;

// v0.35 ARCH-C1: feature-gated experimental backends. Zero external
// callers even with the feature enabled (they integrate via the
// `storage` trait above, also crate-private). Promote to `pub` when a
// downstream consumer activates the feature for production use.
#[cfg(feature = "tantivy")]
pub(crate) mod tantivy_backend;

#[cfg(feature = "pgvector")]
pub(crate) mod pgvector;

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
