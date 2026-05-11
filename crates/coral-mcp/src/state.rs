//! Stateful wiki tracking for MCP push notifications.
//!
//! `WikiState` holds an in-memory cache of parsed wiki pages behind
//! `Arc<RwLock<>>` so the MCP server can serve reads without re-scanning
//! the filesystem on every request. A `dirty` flag is set by the file
//! watcher when mtimes change; the next `resources/read` call triggers a
//! `refresh()` that re-scans the wiki root and updates the cached page
//! set.
//!
//! **M2.4** replaces the `OnceLock`-based cache in `WikiResourceProvider`
//! (which could never be invalidated without a process restart) with a
//! live-reloadable state object that the watcher can poke.

use coral_core::page::Page;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Instant;

/// In-memory wiki page cache with dirty-flag refresh.
///
/// Wrap in `Arc<RwLock<WikiState>>` for shared ownership between the
/// MCP request handler and the background file watcher.
#[derive(Debug)]
pub struct WikiState {
    wiki_root: PathBuf,
    pages: Vec<Page>,
    last_scan: Instant,
    dirty: bool,
}

impl WikiState {
    /// Initial load: scans `wiki_root` for pages and caches the result.
    /// If `wiki_root` does not exist or contains no valid pages, the
    /// cache starts empty.
    pub fn new(wiki_root: PathBuf) -> Self {
        let pages = Self::scan(&wiki_root);
        Self {
            wiki_root,
            pages,
            last_scan: Instant::now(),
            dirty: false,
        }
    }

    /// Re-scan the wiki root, replacing the cached page set. Clears
    /// the dirty flag and updates `last_scan`.
    pub fn refresh(&mut self) {
        self.pages = Self::scan(&self.wiki_root);
        self.last_scan = Instant::now();
        self.dirty = false;
    }

    /// Borrow the current cached page set.
    pub fn pages(&self) -> &[Page] {
        &self.pages
    }

    /// Flag the cache as stale. The next `refresh()` call will re-scan
    /// the wiki root.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Returns `true` when the watcher has detected a filesystem change
    /// since the last `refresh()`.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Returns the instant of the most recent scan.
    #[allow(dead_code)]
    pub fn last_scan(&self) -> Instant {
        self.last_scan
    }

    /// Scan the wiki root for pages. Returns an empty vec if the root
    /// doesn't exist or no valid pages are found.
    fn scan(wiki_root: &PathBuf) -> Vec<Page> {
        if !wiki_root.exists() {
            return Vec::new();
        }
        coral_core::walk::read_pages(wiki_root).unwrap_or_default()
    }
}

/// Convenience constructor for the shared state handle used by the MCP
/// server and watcher.
pub fn shared_state(wiki_root: PathBuf) -> Arc<RwLock<WikiState>> {
    Arc::new(RwLock::new(WikiState::new(wiki_root)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Helper: write a minimal valid wiki page to `dir/<type>/<slug>.md`.
    fn write_page(dir: &std::path::Path, slug: &str) {
        let type_dir = dir.join("module");
        fs::create_dir_all(&type_dir).unwrap();
        let content = format!(
            "\
---
slug: {slug}
type: module
last_updated_commit: abc123
confidence: 0.9
status: draft
---

# {slug}
"
        );
        fs::write(type_dir.join(format!("{slug}.md")), content).unwrap();
    }

    #[test]
    fn new_loads_pages_from_wiki_root() {
        let dir = tempfile::tempdir().unwrap();
        write_page(dir.path(), "alpha");

        let state = WikiState::new(dir.path().to_path_buf());
        assert_eq!(state.pages().len(), 1);
        assert_eq!(state.pages()[0].frontmatter.slug, "alpha");
        assert!(!state.is_dirty());
    }

    #[test]
    fn refresh_detects_new_page() {
        let dir = tempfile::tempdir().unwrap();
        write_page(dir.path(), "first");

        let mut state = WikiState::new(dir.path().to_path_buf());
        assert_eq!(state.pages().len(), 1);

        // Add a second page on disk.
        write_page(dir.path(), "second");

        state.refresh();
        assert_eq!(state.pages().len(), 2);
    }

    #[test]
    fn mark_dirty_flags_state() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = WikiState::new(dir.path().to_path_buf());
        assert!(!state.is_dirty());
        state.mark_dirty();
        assert!(state.is_dirty());
    }

    #[test]
    fn pages_returns_cached_set() {
        let dir = tempfile::tempdir().unwrap();
        write_page(dir.path(), "cached");

        let state = WikiState::new(dir.path().to_path_buf());
        let pages = state.pages();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].frontmatter.slug, "cached");

        // Verify the returned slice is the same cached data (no re-scan).
        let pages_again = state.pages();
        assert_eq!(pages.len(), pages_again.len());
    }

    #[test]
    fn refresh_clears_dirty_flag() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = WikiState::new(dir.path().to_path_buf());
        state.mark_dirty();
        assert!(state.is_dirty());
        state.refresh();
        assert!(!state.is_dirty());
    }

    #[test]
    fn empty_wiki_root_returns_empty_pages() {
        let dir = tempfile::tempdir().unwrap();
        // Empty directory — no pages.
        let state = WikiState::new(dir.path().to_path_buf());
        assert!(state.pages().is_empty());
    }

    #[test]
    fn nonexistent_wiki_root_returns_empty_pages() {
        let state = WikiState::new(PathBuf::from("/nonexistent/wiki/root/that/does/not/exist"));
        assert!(state.pages().is_empty());
        assert!(!state.is_dirty());
    }

    #[test]
    fn shared_state_is_send_sync() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_state(dir.path().to_path_buf());
        // Prove it compiles as Send + Sync by moving across threads.
        let handle = std::thread::spawn(move || {
            let guard = shared.read().unwrap();
            guard.pages().len()
        });
        assert_eq!(handle.join().unwrap(), 0);
    }

    #[test]
    fn last_scan_updates_on_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = WikiState::new(dir.path().to_path_buf());
        let first_scan = state.last_scan();
        // Small sleep to ensure Instant::now() advances.
        std::thread::sleep(std::time::Duration::from_millis(10));
        state.refresh();
        assert!(state.last_scan() > first_scan);
    }
}
