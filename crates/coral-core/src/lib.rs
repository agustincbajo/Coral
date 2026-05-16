//! Core types and utilities for Coral.
//!
//! v0.35 Phase C (ARCH-C1): the public module surface was audited
//! against actual cross-crate use. Modules with zero external callers
//! were demoted to `pub(crate)` so they don't accidentally become
//! SemVer-frozen API. The audit grep is recorded in BACKLOG.md.
//!
//! v0.36 ARCH-C1 (remainder): 10 additional `pub mod` declarations
//! were converted to `pub(crate)` with curated `pub use` re-exports
//! at crate root. Each demoted module had 1-4 distinct items used
//! externally; re-exporting them at crate root preserves the public
//! contract while shrinking the SemVer surface from 33 → ~17 mods
//! (~48%). See BACKLOG.md "v0.35 Phase C deferrals (ARCH-C1)".

pub mod atomic;
pub mod auth;
pub(crate) mod cache;
// v0.41 P1: lightweight progress! macro for human-readable runner feedback.
// `pub` so coral-runner, coral-cli, and coral-mcp can all import the macro
// and the `progress_enabled` / `emit_progress` helpers.
pub mod observability;
pub mod config;
pub mod cost;
pub(crate) mod embeddings;
pub(crate) mod embeddings_sqlite;
pub mod error;
pub(crate) mod eval;
pub mod frontmatter;
pub(crate) mod gc;
pub(crate) mod git_remote;
pub mod gitdiff;
pub mod governance;
pub(crate) mod index;
pub(crate) mod llms_txt;
pub mod log;
pub(crate) mod narrative;
pub mod page;
pub mod path;
pub mod project;
pub mod search;
pub(crate) mod search_index;
pub mod slug;
// v0.35 ARCH-C1: `storage` is the EmbeddingsStorage trait, currently
// consumed only by the feature-gated `pgvector` module inside this
// crate. No external callers — keep crate-private until a downstream
// integration needs the trait. Re-expose via `pub use` at that point.
pub(crate) mod storage;
pub(crate) mod symbols;
// v0.35 ARCH-C1: `vocab`, `late_chunking`, and `reranker` are
// scaffolding for future search-relevance work. Zero external uses
// today; demoted to crate-private so the SemVer surface doesn't
// accidentally freeze around an incomplete API.
pub(crate) mod vocab;
pub mod walk;
pub(crate) mod wikilinks;

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

// v0.36 ARCH-C1: curated `pub use` shim for the 10 demoted modules.
// Each line documents the external callsites that drove the re-export
// list. Adding a new external consumer? Surface it here, do NOT
// re-pub the module — keeps the SemVer contract narrow and explicit.

// `coral_core::index` — 3 callsites in coral-cli (bootstrap, ingest,
// init, status) + integration tests.
pub use crate::index::{IndexEntry, WikiIndex};
// `coral_core::cache` — 2 callsites in coral-cli::commands::search.
pub use crate::cache::WalkCache;
// `coral_core::embeddings` — coral-cli::commands::search + integration
// tests.
pub use crate::embeddings::EmbeddingsIndex;
// `coral_core::embeddings_sqlite` — coral-cli::commands::search +
// integration tests.
pub use crate::embeddings_sqlite::{SQLITE_FILENAME, SqliteEmbeddingsIndex};
// `coral_core::eval` — coral-cli::commands::search (run_eval).
pub use crate::eval::{evaluate, load_goldset, render_markdown as eval_render_markdown};
// `coral_core::narrative` — coral-cli::commands::diff.
pub use crate::narrative::{PageDiff, diff_wiki_states, generate_narrative};
// `coral_core::llms_txt` — coral-cli::commands::export.
pub use crate::llms_txt::generate as llms_txt_generate;
// `coral_core::gc` — coral-cli::commands::consolidate.
pub use crate::gc::{
    analyze as gc_analyze, render_json as gc_render_json, render_markdown as gc_render_markdown,
};
// `coral_core::symbols` — coral-cli::commands::{bootstrap, stats}.
pub use crate::symbols::{Symbol, SymbolKind, extract_from_dir, find_symbols_for_slug};
// `coral_core::git_remote` — coral-cli::commands::project::sync.
pub use crate::git_remote::{SyncOutcome, sync_repo};
// `coral_core::search_index` — coral-cli::commands::search.
pub use crate::search_index::search_with_index;
// `coral_core::wikilinks` — coral-cli::commands::consolidate +
// integration tests + benches.
pub use crate::wikilinks::extract as wikilinks_extract;

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
