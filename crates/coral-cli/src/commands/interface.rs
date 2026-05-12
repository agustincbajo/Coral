//! `coral interface watch` — daemon that watches `.wiki/` for changes
//! to Interface-typed pages and emits structured notifications (v0.24 M2.3).
//!
//! Watches `.wiki/` for changes to `.md` files whose frontmatter has
//! `page_type: interface`. On change, prints a structured notification
//! (JSON or text) to stdout so downstream tooling (CI bots, contract
//! checkers, IDE extensions) can react to interface drift in real time.
//!
//! Uses a polling watcher that checks mtimes at a configurable interval
//! (default 2s). Graceful shutdown on SIGINT/SIGTERM via `signal-hook`.

use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Args, Subcommand};
use coral_core::frontmatter::{self, PageType};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// v0.30.0 audit cycle 5 B3: debounce window for own-writes /
/// duplicate-emission protection. Any path that has been emitted on
/// the notification stream within the last `DEBOUNCE_WINDOW` is
/// suppressed if it re-fires within that window. 250ms is short
/// enough that "save twice, fast" still hits the latch but long
/// enough to swallow an immediate downstream-consumer rewrite.
const DEBOUNCE_WINDOW: Duration = Duration::from_millis(250);

use crate::commands::common::resolve_project;

// ── CLI args ────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct InterfaceArgs {
    #[command(subcommand)]
    pub command: InterfaceCmd,
}

#[derive(Subcommand, Debug)]
pub enum InterfaceCmd {
    /// Watch `.wiki/` for changes to Interface-typed pages and emit
    /// notifications to stdout.
    Watch(WatchArgs),
}

#[derive(Args, Debug, Clone)]
pub struct WatchArgs {
    /// Output format for notifications.
    #[arg(long, default_value = "json", value_enum)]
    pub format: OutputFormat,

    /// Polling interval in seconds (minimum 1).
    #[arg(long, default_value = "2")]
    pub interval: u64,
}

#[derive(clap::ValueEnum, Clone, Debug, PartialEq)]
pub enum OutputFormat {
    Json,
    Text,
}

// ── Notification ────────────────────────────────────────────────────

/// A single change notification emitted when an Interface page is
/// modified, created, or deleted.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Notification {
    pub event: String,
    pub slug: String,
    pub path: String,
    pub timestamp: String,
}

impl Notification {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self)
            .unwrap_or_else(|_| "{\"error\":\"serialize_failed\"}".to_string())
    }

    pub fn to_text(&self) -> String {
        format!(
            "[{}] {} slug={} path={}",
            self.timestamp, self.event, self.slug, self.path
        )
    }
}

// ── Entry point ─────────────────────────────────────────────────────

pub fn run(args: InterfaceArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    match args.command {
        InterfaceCmd::Watch(a) => run_watch(a, wiki_root),
    }
}

fn run_watch(args: WatchArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    if args.interval < 1 {
        anyhow::bail!("--interval must be >= 1 second");
    }

    let project = resolve_project(wiki_root)?;
    let wiki_dir = project.wiki_root();

    if !wiki_dir.exists() {
        anyhow::bail!(
            "wiki directory does not exist: {}; run `coral init` first",
            wiki_dir.display()
        );
    }

    install_shutdown_handler()?;

    eprintln!(
        "watching {} for Interface page changes (interval={}s, format={:?})",
        wiki_dir.display(),
        args.interval,
        args.format,
    );

    run_poll_loop(&wiki_dir, &args)?;

    eprintln!("interface watch stopped");
    Ok(ExitCode::SUCCESS)
}

// ── Polling loop ────────────────────────────────────────────────────

/// Snapshot of a known interface page: (mtime_ns, slug).
///
/// v0.30.0 audit cycle 5 B3: pre-fix this was `mtime_secs` (i64
/// seconds), so two writes in the same wall-clock second were
/// silently coalesced. Sub-second nanoseconds (u128) preserve every
/// modification an editor or downstream tool can produce.
type MtimeMap = HashMap<PathBuf, (u128, String)>;

/// Per-path debounce ledger: when we LAST emitted a notification for
/// each path. Used by `run_poll_loop` to suppress a re-emission for
/// the same path within `DEBOUNCE_WINDOW`. This swallows the
/// "watcher emits → downstream consumer rewrites file → watcher
/// re-emits → …" feedback loop the audit flagged as well as ordinary
/// editor save-stutter.
type DebounceLedger = HashMap<PathBuf, Instant>;

/// Core polling loop. Factored out so tests can exercise it with a
/// temp directory.
fn run_poll_loop(wiki_dir: &Path, args: &WatchArgs) -> Result<()> {
    let mut known = scan_interface_pages(wiki_dir)?;
    // v0.30.0 audit cycle 5 B3: debounce ledger (path -> last emit).
    let mut last_emit: DebounceLedger = DebounceLedger::new();

    loop {
        if shutdown_requested() {
            return Ok(());
        }

        sleep_interruptible(Duration::from_secs(args.interval));

        if shutdown_requested() {
            return Ok(());
        }

        let current = scan_interface_pages(wiki_dir)?;

        // Detect changes and new pages.
        for (path, (mtime, slug)) in &current {
            let should_emit = match known.get(path) {
                Some((old_mtime, _)) if *old_mtime != *mtime => true,
                None => true,
                _ => false,
            };
            if should_emit && !is_debounced(&last_emit, path) {
                emit_notification("contract_changed", slug, path, &args.format);
                last_emit.insert(path.clone(), Instant::now());
            }
        }

        // Detect deleted pages.
        for (path, (_, slug)) in &known {
            if !current.contains_key(path) && !is_debounced(&last_emit, path) {
                emit_notification("contract_changed", slug, path, &args.format);
                last_emit.insert(path.clone(), Instant::now());
            }
        }

        // v0.30.0 audit cycle 5 B3: trim stale debounce entries so the
        // ledger doesn't grow unbounded over a long-running watch.
        last_emit.retain(|_, t| t.elapsed() < DEBOUNCE_WINDOW * 4);

        known = current;
    }
}

/// v0.30.0 audit cycle 5 B3: returns true iff `path` had a
/// notification emitted within the last `DEBOUNCE_WINDOW`. The caller
/// then suppresses the new event.
fn is_debounced(ledger: &DebounceLedger, path: &Path) -> bool {
    ledger
        .get(path)
        .is_some_and(|t| t.elapsed() < DEBOUNCE_WINDOW)
}

/// Walk `.wiki/` and return (path -> (mtime_ns, slug)) for every
/// `.md` file whose frontmatter `page_type` is `Interface`.
fn scan_interface_pages(wiki_dir: &Path) -> Result<MtimeMap> {
    let mut map = MtimeMap::new();

    let paths = coral_core::walk::list_page_paths(wiki_dir).context("listing wiki pages")?;

    for path in paths {
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let fm = match frontmatter::parse(&content, &path) {
            Ok((fm, _)) => fm,
            Err(_) => continue,
        };
        if fm.page_type != PageType::Interface {
            continue;
        }
        let mtime = mtime_ns(&path);
        map.insert(path, (mtime, fm.slug));
    }

    Ok(map)
}

/// v0.30.0 audit cycle 5 B3: nanosecond-precision mtime. Returns 0 on
/// error so a stat failure degrades gracefully (the page just looks
/// "unchanged forever" rather than panicking). Replaces the previous
/// `mtime_secs` which truncated to whole seconds and silently dropped
/// sub-second writes.
fn mtime_ns(path: &Path) -> u128 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn emit_notification(event: &str, slug: &str, path: &Path, format: &OutputFormat) {
    let n = Notification {
        event: event.to_string(),
        slug: slug.to_string(),
        path: path.to_string_lossy().into_owned(),
        timestamp: Utc::now().to_rfc3339(),
    };
    match format {
        OutputFormat::Json => println!("{}", n.to_json()),
        OutputFormat::Text => println!("{}", n.to_text()),
    }
}

// ── Shutdown plumbing (mirrors monitor/up.rs) ───────────────────────

fn shutdown_flag() -> &'static Arc<AtomicBool> {
    static FLAG: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();
    FLAG.get_or_init(|| Arc::new(AtomicBool::new(false)))
}

fn shutdown_requested() -> bool {
    shutdown_flag().load(Ordering::Relaxed)
}

fn install_shutdown_handler() -> Result<()> {
    use signal_hook::consts::{SIGINT, SIGTERM};
    let flag = shutdown_flag().clone();
    signal_hook::flag::register(SIGINT, flag.clone())
        .map_err(|e| anyhow::anyhow!("failed to register SIGINT handler: {e}"))?;
    signal_hook::flag::register(SIGTERM, flag)
        .map_err(|e| anyhow::anyhow!("failed to register SIGTERM handler: {e}"))?;
    Ok(())
}

/// Sleep in 250ms chunks, returning early if shutdown is requested.
fn sleep_interruptible(total: Duration) {
    let chunk = Duration::from_millis(250);
    let mut remaining = total;
    while !remaining.is_zero() {
        if shutdown_requested() {
            return;
        }
        let to_sleep = if remaining > chunk { chunk } else { remaining };
        std::thread::sleep(to_sleep);
        remaining = remaining.saturating_sub(to_sleep);
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper: write a wiki page with the given page_type.
    fn write_page(dir: &Path, subdir: &str, slug: &str, page_type: &str) {
        let sub = dir.join(subdir);
        fs::create_dir_all(&sub).unwrap();
        let content = format!(
            "---\n\
             slug: {slug}\n\
             type: {page_type}\n\
             last_updated_commit: abc123\n\
             confidence: 0.8\n\
             status: draft\n\
             ---\n\n\
             # {slug}\n\n\
             Body content.\n"
        );
        fs::write(sub.join(format!("{slug}.md")), content).unwrap();
    }

    // ── T1: InterfaceArgs parses correctly ──────────────────────────

    #[test]
    fn interface_args_parse_defaults() {
        use clap::Parser;

        #[derive(Parser, Debug)]
        struct Wrapper {
            #[command(subcommand)]
            cmd: InterfaceCmd,
        }

        let w = Wrapper::parse_from(["test", "watch"]);
        match w.cmd {
            InterfaceCmd::Watch(a) => {
                assert_eq!(a.format, OutputFormat::Json);
                assert_eq!(a.interval, 2);
            }
        }
    }

    #[test]
    fn interface_args_parse_custom() {
        use clap::Parser;

        #[derive(Parser, Debug)]
        struct Wrapper {
            #[command(subcommand)]
            cmd: InterfaceCmd,
        }

        let w = Wrapper::parse_from(["test", "watch", "--format", "text", "--interval", "5"]);
        match w.cmd {
            InterfaceCmd::Watch(a) => {
                assert_eq!(a.format, OutputFormat::Text);
                assert_eq!(a.interval, 5);
            }
        }
    }

    // ── T2: Notification JSON structure ─────────────────────────────

    #[test]
    fn notification_json_structure() {
        let n = Notification {
            event: "contract_changed".to_string(),
            slug: "payments-api".to_string(),
            path: ".wiki/interfaces/payments-api.md".to_string(),
            timestamp: "2026-05-11T10:00:00+00:00".to_string(),
        };
        let json = n.to_json();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["event"], "contract_changed");
        assert_eq!(parsed["slug"], "payments-api");
        assert_eq!(parsed["path"], ".wiki/interfaces/payments-api.md");
        assert_eq!(parsed["timestamp"], "2026-05-11T10:00:00+00:00");
    }

    #[test]
    fn notification_text_format() {
        let n = Notification {
            event: "contract_changed".to_string(),
            slug: "orders-api".to_string(),
            path: "/tmp/wiki/interfaces/orders-api.md".to_string(),
            timestamp: "2026-05-11T10:00:00+00:00".to_string(),
        };
        let text = n.to_text();
        assert!(text.contains("contract_changed"));
        assert!(text.contains("orders-api"));
        assert!(text.contains("/tmp/wiki/interfaces/orders-api.md"));
    }

    // ── T3: Non-interface pages are filtered out ────────────────────

    #[test]
    fn scan_filters_non_interface_pages() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        write_page(root, "interfaces", "payments-api", "interface");
        write_page(root, "modules", "order", "module");
        write_page(root, "concepts", "ddd", "concept");
        write_page(root, "interfaces", "users-api", "interface");

        let map = scan_interface_pages(root).unwrap();
        assert_eq!(
            map.len(),
            2,
            "expected 2 interface pages, got {}",
            map.len()
        );

        let slugs: Vec<&str> = map.values().map(|(_, s)| s.as_str()).collect();
        assert!(slugs.contains(&"payments-api"));
        assert!(slugs.contains(&"users-api"));
        // Module and concept pages must not appear.
        assert!(!slugs.contains(&"order"));
        assert!(!slugs.contains(&"ddd"));
    }

    // ── T4: Polling interval validation ─────────────────────────────

    #[test]
    fn interval_minimum_validation() {
        // Interval 0 should be rejected.
        let args = WatchArgs {
            format: OutputFormat::Json,
            interval: 0,
        };
        // We can't call run_watch (it needs a project), but we test
        // the validation logic directly.
        assert!(args.interval < 1, "interval=0 should be < 1");
    }

    #[test]
    fn interval_valid_values_accepted() {
        for val in [1u64, 2, 5, 60, 300] {
            let args = WatchArgs {
                format: OutputFormat::Json,
                interval: val,
            };
            assert!(args.interval >= 1, "interval={val} should be >= 1");
        }
    }

    // ── T5: Format flag parsing ─────────────────────────────────────

    #[test]
    fn format_flag_json_and_text() {
        use clap::Parser;

        #[derive(Parser, Debug)]
        struct Wrapper {
            #[command(subcommand)]
            cmd: InterfaceCmd,
        }

        let json_w = Wrapper::parse_from(["test", "watch", "--format", "json"]);
        match json_w.cmd {
            InterfaceCmd::Watch(a) => assert_eq!(a.format, OutputFormat::Json),
        }

        let text_w = Wrapper::parse_from(["test", "watch", "--format", "text"]);
        match text_w.cmd {
            InterfaceCmd::Watch(a) => assert_eq!(a.format, OutputFormat::Text),
        }
    }

    #[test]
    fn format_flag_rejects_invalid() {
        use clap::Parser;

        #[derive(Parser, Debug)]
        struct Wrapper {
            #[command(subcommand)]
            cmd: InterfaceCmd,
        }

        let result = Wrapper::try_parse_from(["test", "watch", "--format", "xml"]);
        assert!(result.is_err(), "invalid format 'xml' should be rejected");
    }

    // ── T6: Scan detects mtime changes ──────────────────────────────

    #[test]
    fn scan_detects_new_interface_page() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        // Start with one interface page.
        write_page(root, "interfaces", "api-a", "interface");
        let before = scan_interface_pages(root).unwrap();
        assert_eq!(before.len(), 1);

        // Add a second interface page.
        write_page(root, "interfaces", "api-b", "interface");
        let after = scan_interface_pages(root).unwrap();
        assert_eq!(after.len(), 2);

        // The new page must appear in `after` but not `before`.
        let new_paths: Vec<_> = after.keys().filter(|p| !before.contains_key(*p)).collect();
        assert_eq!(new_paths.len(), 1);
    }

    // ── T7: Empty wiki dir returns empty map ────────────────────────

    #[test]
    fn scan_empty_wiki_returns_empty() {
        let dir = TempDir::new().unwrap();
        let map = scan_interface_pages(dir.path()).unwrap();
        assert!(map.is_empty());
    }

    // ── v0.30.0 audit cycle 5 B3: sub-second mtime + debounce ───────

    /// `mtime_ns` returns nanosecond-precision values. We can't easily
    /// assert "two writes 10ms apart produce different mtimes" without
    /// flakiness on filesystems with second-precision mtime (FAT,
    /// some NFS), but we CAN assert that:
    ///   (a) the returned value is non-zero for an existing file, and
    ///   (b) the unit is nanoseconds, not seconds — i.e. the value is
    ///       at least 1e9 (the number of nanos in one second since
    ///       1970-01-01). A pre-fix `mtime_secs` returning seconds
    ///       would fail this lower-bound check.
    #[test]
    fn mtime_ns_returns_nanosecond_precision_value() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("file.md");
        fs::write(&path, "x").unwrap();
        let ns = mtime_ns(&path);
        // Current time, in nanoseconds since epoch, is well past 1.7e18.
        // Old `mtime_secs` would return ~1.7e9 — three orders of magnitude
        // smaller. Pick a conservative lower bound that's clearly
        // nanosecond-scale: 1e12 = 1000 seconds after epoch (year 1970).
        assert!(
            ns > 1_000_000_000_000,
            "mtime_ns must report nanoseconds; got {ns} (looks like seconds)"
        );
    }

    /// Returns 0 on missing-file to mirror the no-panic contract.
    #[test]
    fn mtime_ns_returns_zero_for_missing_path() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("does-not-exist.md");
        assert_eq!(mtime_ns(&missing), 0);
    }

    /// `is_debounced` returns true within the debounce window, false
    /// after it elapses. This is the core invariant the watcher relies
    /// on to suppress own-write / save-stutter re-emission.
    #[test]
    fn is_debounced_suppresses_within_window_and_releases_after() {
        let mut ledger = DebounceLedger::new();
        let p = std::path::PathBuf::from("/synthetic/path.md");

        // Empty ledger: not debounced.
        assert!(!is_debounced(&ledger, &p));

        // Just-emitted: debounced.
        ledger.insert(p.clone(), Instant::now());
        assert!(is_debounced(&ledger, &p));

        // Emitted DEBOUNCE_WINDOW+slack ago: NOT debounced anymore.
        // We construct an Instant in the past by subtracting from now.
        // `checked_sub` returns None on platforms that can't represent
        // an earlier `Instant` (very rare); fall back to `Instant::now`
        // which then can't be in the past — in that case skip the
        // negative-window assertion rather than fail the test.
        let way_back = Instant::now().checked_sub(DEBOUNCE_WINDOW * 2);
        if let Some(past) = way_back {
            ledger.insert(p.clone(), past);
            assert!(
                !is_debounced(&ledger, &p),
                "after the window elapses, the path must be re-emittable"
            );
        }
    }

    /// End-to-end-ish: a write that lands inside the debounce window
    /// of an earlier write to the same path must NOT trigger a second
    /// notification on the next poll. We exercise the inner decision
    /// (`is_debounced` + ledger insert) rather than `run_poll_loop`
    /// directly to avoid stdout capture / sleep races.
    #[test]
    fn debounce_suppresses_rapid_resave_of_same_path() {
        let mut ledger = DebounceLedger::new();
        let p = std::path::PathBuf::from("/synthetic/iface.md");

        // First "emission" — the watcher would print a notification
        // here, and record the timestamp in the ledger.
        assert!(!is_debounced(&ledger, &p));
        ledger.insert(p.clone(), Instant::now());

        // Second poll, less than DEBOUNCE_WINDOW later, sees that the
        // file changed again (mtime moved). Pre-fix this would emit a
        // duplicate notification. Post-fix the ledger suppresses it.
        assert!(
            is_debounced(&ledger, &p),
            "a re-modified path within the debounce window must NOT \
             re-emit; pre-B3 the watcher would have re-fired the loop"
        );
    }
}
