//! Polling-based wiki watcher for MCP subscription push.
//!
//! Runs a background thread that checks `.wiki/` mtimes every N seconds.
//! When a change is detected, triggers `notify_resources_list_changed()`
//! so subscribed MCP clients know to re-fetch resources.
//!
//! **Design note (v0.24 #13):** the current `WikiResourceProvider` uses
//! `OnceLock` for page caching, which cannot be invalidated without a
//! process restart. This watcher still provides value: it pushes the
//! MCP `notifications/resources/list_changed` signal so clients know
//! *something* changed — even if this server process serves stale data
//! until restarted. A follow-up PR will swap `OnceLock` → `Mutex<Option<>>`
//! to enable in-process cache invalidation.

use crate::server::McpHandler;
use crate::state::WikiState;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

/// Configuration for the wiki watcher.
pub struct WatcherConfig {
    /// Path to the wiki root directory.
    pub wiki_root: PathBuf,
    /// Polling interval.
    pub interval: Duration,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            wiki_root: PathBuf::from(".wiki"),
            interval: Duration::from_secs(2),
        }
    }
}

/// Start the watcher in a background thread. Returns a handle that
/// stops the watcher when dropped (via the channel disconnect).
///
/// # Example (integration sketch — not wired in the CLI yet)
///
/// ```ignore
/// let handler = Arc::new(McpHandler::new(config, resources, tools));
/// let _watcher = coral_mcp::watcher::start_watcher(
///     WatcherConfig::default(),
///     Arc::clone(&handler),
/// );
/// handler.serve_stdio()?;
/// ```
pub fn start_watcher(config: WatcherConfig, handler: Arc<McpHandler>) -> WatcherHandle {
    start_watcher_with_state(config, handler, None)
}

/// Start the watcher with an optional `WikiState`. When a change is
/// detected, the watcher calls `mark_dirty()` on the state (if
/// provided) AND sends the MCP `notifications/resources/list_changed`
/// signal. The MCP request handler can then call `refresh()` on the
/// state when the next `resources/read` arrives.
pub fn start_watcher_with_state(
    config: WatcherConfig,
    handler: Arc<McpHandler>,
    wiki_state: Option<Arc<RwLock<WikiState>>>,
) -> WatcherHandle {
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();

    let thread = std::thread::spawn(move || {
        let mut last_mtime = collect_max_mtime(&config.wiki_root);

        loop {
            // Block until either the stop signal arrives or the poll
            // interval elapses.
            match stop_rx.recv_timeout(config.interval) {
                Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }

            let current_mtime = collect_max_mtime(&config.wiki_root);
            if current_mtime != last_mtime {
                tracing::debug!(
                    wiki_root = %config.wiki_root.display(),
                    "wiki change detected, sending notification"
                );
                // Mark the in-memory cache as stale so the next
                // resources/read triggers a refresh.
                if let Some(ref state) = wiki_state {
                    if let Ok(mut s) = state.write() {
                        s.mark_dirty();
                    }
                }
                handler.notify_resources_list_changed();
                last_mtime = current_mtime;
            }
        }
    });

    WatcherHandle {
        _stop_tx: stop_tx,
        _thread: Some(thread),
    }
}

/// Collects the maximum mtime across all files in the wiki directory.
/// Returns `None` if the directory doesn't exist or is empty.
///
/// The walk is recursive but non-parallelized — `.wiki/` is small
/// (hundreds of files max) so a simple `read_dir` loop is sufficient.
fn collect_max_mtime(wiki_root: &Path) -> Option<SystemTime> {
    if !wiki_root.exists() {
        return None;
    }
    let mut max_mtime: Option<SystemTime> = None;

    if let Ok(entries) = std::fs::read_dir(wiki_root) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if let Ok(mtime) = meta.modified() {
                    max_mtime = Some(match max_mtime {
                        Some(current) => current.max(mtime),
                        None => mtime,
                    });
                }
            }
            // Recurse into subdirectories.
            if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                if let Some(sub_mtime) = collect_max_mtime(&entry.path()) {
                    max_mtime = Some(match max_mtime {
                        Some(current) => current.max(sub_mtime),
                        None => sub_mtime,
                    });
                }
            }
        }
    }
    max_mtime
}

/// Handle returned by [`start_watcher`]. The watcher thread stops when
/// this handle is dropped — the `_stop_tx` sender disconnect causes
/// `recv_timeout` in the background thread to return `Disconnected`,
/// which breaks the loop.
pub struct WatcherHandle {
    _stop_tx: std::sync::mpsc::Sender<()>,
    _thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::WikiResourceProvider;
    use crate::server::{McpHandler, NoOpDispatcher};
    use crate::ServerConfig;
    use std::fs;
    use std::sync::Arc;

    fn make_handler(wiki_root: &Path) -> Arc<McpHandler> {
        let cfg = ServerConfig::default();
        let resources = Arc::new(WikiResourceProvider::new(wiki_root.to_path_buf()));
        let tools = Arc::new(NoOpDispatcher);
        Arc::new(McpHandler::new(cfg, resources, tools))
    }

    #[test]
    fn collect_max_mtime_returns_none_for_missing_dir() {
        let mtime = collect_max_mtime(Path::new("/nonexistent/path/that/does/not/exist"));
        assert!(mtime.is_none());
    }

    #[test]
    fn collect_max_mtime_finds_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("page.md"), "hello").unwrap();
        let mtime = collect_max_mtime(dir.path());
        assert!(mtime.is_some());
    }

    #[test]
    fn collect_max_mtime_recurses_into_subdirs() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("nested.md"), "nested").unwrap();
        let mtime = collect_max_mtime(dir.path());
        assert!(mtime.is_some());
    }

    #[test]
    fn watcher_detects_change_and_sends_notification() {
        let dir = tempfile::tempdir().unwrap();
        let wiki_root = dir.path().to_path_buf();
        fs::write(wiki_root.join("initial.md"), "v1").unwrap();

        let handler = make_handler(&wiki_root);
        let (tx, rx) = std::sync::mpsc::channel();
        handler.set_notification_sender(tx);

        let config = WatcherConfig {
            wiki_root: wiki_root.clone(),
            interval: Duration::from_millis(50),
        };
        let _handle = start_watcher(config, Arc::clone(&handler));

        // Wait a bit, then modify a file.
        std::thread::sleep(Duration::from_millis(80));
        fs::write(wiki_root.join("new.md"), "new content").unwrap();

        // Give the watcher time to detect the change.
        std::thread::sleep(Duration::from_millis(150));

        let msg = rx.try_recv().expect("watcher must send notification on change");
        assert_eq!(msg["method"], "notifications/resources/list_changed");
    }

    #[test]
    fn watcher_marks_wiki_state_dirty_on_change() {
        let dir = tempfile::tempdir().unwrap();
        let wiki_root = dir.path().to_path_buf();
        fs::write(wiki_root.join("initial.md"), "v1").unwrap();

        let handler = make_handler(&wiki_root);
        let state = crate::state::shared_state(wiki_root.clone());

        assert!(!state.read().unwrap().is_dirty());

        let config = WatcherConfig {
            wiki_root: wiki_root.clone(),
            interval: Duration::from_millis(50),
        };
        let _handle =
            start_watcher_with_state(config, Arc::clone(&handler), Some(Arc::clone(&state)));

        // Wait, then modify a file.
        std::thread::sleep(Duration::from_millis(80));
        fs::write(wiki_root.join("new.md"), "new content").unwrap();

        // Give the watcher time to detect the change.
        std::thread::sleep(Duration::from_millis(150));

        assert!(
            state.read().unwrap().is_dirty(),
            "watcher must mark WikiState dirty on filesystem change"
        );
    }

    #[test]
    fn watcher_stops_when_handle_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let handler = make_handler(dir.path());

        let config = WatcherConfig {
            wiki_root: dir.path().to_path_buf(),
            interval: Duration::from_millis(50),
        };
        let handle = start_watcher(config, handler);
        // Drop should not panic or hang.
        drop(handle);
    }
}
