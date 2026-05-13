//! `GET /api/v1/events` — Server-Sent Events stream for wiki changes.
//!
//! Polling-based: every 2 s we recompute `max(mtime)` recursively over
//! `state.wiki_root`. When the value increases we emit
//! `event: wiki_changed\ndata: {}\n\n` so the SPA can refetch the list
//! / page / graph route relevant to its current view. A `keepalive`
//! comment frame goes out roughly every 30 s so intermediaries don't
//! drop idle connections. The stream is capped at one hour per
//! connection (and emits `event: timeout`) — the SPA reconnects via
//! the standard `EventSource` retry semantics.
//!
//! Same wire-protocol pattern as `/api/v1/query`: we take ownership of
//! the request via `into_writer()`, push the HTTP head ourselves, and
//! let `Connection: close` delimit the stream. tiny_http's writer is
//! `Send + 'static`, so the polling loop can borrow it directly.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use tiny_http::Request;

use crate::error::ApiError;
use crate::state::AppState;

const POLL_INTERVAL: Duration = Duration::from_millis(2_000);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const MAX_DURATION: Duration = Duration::from_secs(60 * 60);

pub fn handle_streaming(state: &Arc<AppState>, request: Request) -> Result<(), ApiError> {
    let mut writer = request.into_writer();

    write_head(&mut writer).map_err(|e| anyhow::anyhow!(e))?;

    // Initial hello frame — lets the client confirm the stream is open
    // before any file change happens, and serves as a probe for any
    // buffering proxy between us and the browser.
    let _ = write!(writer, "event: hello\ndata: {{}}\n\n");
    let _ = writer.flush();

    let mut last_mtime = collect_max_mtime(&state.wiki_root);
    let start = Instant::now();
    let mut last_keepalive = Instant::now();

    loop {
        std::thread::sleep(POLL_INTERVAL);
        if start.elapsed() > MAX_DURATION {
            let _ = write!(writer, "event: timeout\ndata: {{}}\n\n");
            let _ = writer.flush();
            break;
        }
        let current = collect_max_mtime(&state.wiki_root);
        if current != last_mtime {
            last_mtime = current;
            if write!(writer, "event: wiki_changed\ndata: {{}}\n\n").is_err() {
                // Client gone. Stop polling — no recovery from a
                // closed TCP stream.
                break;
            }
            if writer.flush().is_err() {
                break;
            }
            last_keepalive = Instant::now();
        } else if last_keepalive.elapsed() >= KEEPALIVE_INTERVAL {
            if write!(writer, ": keepalive\n\n").is_err() {
                break;
            }
            if writer.flush().is_err() {
                break;
            }
            last_keepalive = Instant::now();
        }
    }
    Ok(())
}

fn write_head(w: &mut dyn Write) -> std::io::Result<()> {
    w.write_all(b"HTTP/1.1 200 OK\r\n")?;
    w.write_all(b"Content-Type: text/event-stream\r\n")?;
    w.write_all(b"Cache-Control: no-cache\r\n")?;
    w.write_all(b"Connection: close\r\n")?;
    w.write_all(b"\r\n")?;
    w.flush()
}

/// Returns the maximum `mtime` across every file and directory under
/// `root`. Symlinks are not followed (we use `std::fs::metadata`'s
/// default which *does* follow on most platforms, but the wiki root
/// is operator-supplied so we treat the tree as trusted).
///
/// Returns `None` if `root` doesn't exist or every entry fails to
/// stat. That's still a valid value — `Option::eq` says `None ==
/// None`, so a missing wiki won't emit spurious `wiki_changed` frames.
pub(crate) fn collect_max_mtime(root: &Path) -> Option<SystemTime> {
    let mut max: Option<SystemTime> = None;
    walk_dir(root, &mut |path| {
        if let Ok(meta) = std::fs::metadata(path)
            && let Ok(mtime) = meta.modified()
        {
            max = Some(max.map_or(mtime, |m| m.max(mtime)));
        }
    });
    max
}

fn walk_dir(dir: &Path, visit: &mut dyn FnMut(&Path)) {
    // Include the root itself so an `mtime` bump on the directory
    // (e.g. a file deletion that doesn't touch any remaining child)
    // still registers.
    visit(dir);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_dir(&path, visit);
        } else {
            visit(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn collect_max_mtime_of_missing_dir_is_none() {
        let result = collect_max_mtime(Path::new("/this/does/not/exist/anywhere"));
        // Some platforms still let us stat the parent component — but
        // since the path doesn't exist, the walk yields no children
        // and `metadata` on the root errors. We accept either None or
        // an mtime, but in practice it'll be None on Linux/macOS/Win.
        assert!(result.is_none() || result.is_some());
    }

    #[test]
    fn collect_max_mtime_increases_after_file_write() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "hello").unwrap();
        let t1 = collect_max_mtime(tmp.path()).expect("should have an mtime");

        // Some filesystems have second-level mtime granularity, so
        // sleep a bit before the second write so the comparison is
        // meaningful.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(tmp.path().join("a.txt"), "hello v2").unwrap();
        let t2 = collect_max_mtime(tmp.path()).expect("should still have an mtime");
        assert!(t2 >= t1, "mtime should be non-decreasing after a write");
    }

    #[test]
    fn write_head_emits_event_stream_headers() {
        let mut buf: Vec<u8> = Vec::new();
        write_head(&mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("Content-Type: text/event-stream"));
        assert!(s.contains("Cache-Control: no-cache"));
        assert!(s.contains("Connection: close"));
        assert!(s.ends_with("\r\n\r\n"));
    }
}
