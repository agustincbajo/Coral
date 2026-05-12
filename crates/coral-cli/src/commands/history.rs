//! `coral history <slug>` — log entries that touched a slug.
//!
//! Filters `.wiki/log.md` to entries whose body (the `summary` field
//! after parsing) literally mentions `<slug>` — case-sensitive substring
//! match, since slugs are kebab-case identifiers and we don't want
//! "order" to match "border-collie". Output is reverse chronological
//! (newest first), capped at `--limit` (default 20).
//!
//! Read-only: no LLM, no mutation. Always exits 0.

use anyhow::{Context, Result};
use clap::Args;
use coral_core::log::{LogEntry, WikiLog};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// CLI args for `coral history`.
#[derive(Args, Debug)]
pub struct HistoryArgs {
    /// Slug to filter for. Case-sensitive substring match against each
    /// log entry's body.
    pub slug: String,
    /// Maximum number of entries to print (default 20).
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
    /// Output format: markdown (default) or json.
    #[arg(long, default_value = "markdown")]
    pub format: String,
}

/// Default value for `--limit` when no flag is passed.
pub const DEFAULT_LIMIT: usize = 20;

/// Entry point wired to `Cmd::History`. Loads `.wiki/log.md`, filters to
/// entries that mention `slug`, reverses, caps, and prints.
pub fn run(args: HistoryArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let root: PathBuf = wiki_root
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".wiki"));
    if !root.exists() {
        anyhow::bail!(
            "wiki root not found: {}. Run `coral init` first.",
            root.display()
        );
    }

    let log_path = root.join("log.md");
    let log = WikiLog::load(&log_path)
        .with_context(|| format!("reading log from {}", log_path.display()))?;

    let matches: Vec<&LogEntry> = filter_entries(&log.entries, &args.slug, args.limit);

    match args.format.as_str() {
        "json" => println!("{}", render_json(&args.slug, &matches)?),
        _ => println!("{}", render_markdown(&args.slug, &matches)),
    }
    Ok(ExitCode::SUCCESS)
}

/// Filter `entries` down to those whose `summary` contains `slug`
/// (case-sensitive), in reverse chronological order, capped at `limit`.
/// Pulled out so tests can exercise the slicing logic without disk I/O.
pub(crate) fn filter_entries<'a>(
    entries: &'a [LogEntry],
    slug: &str,
    limit: usize,
) -> Vec<&'a LogEntry> {
    entries
        .iter()
        .rev()
        .filter(|e| e.summary.contains(slug))
        .take(limit)
        .collect()
}

/// Markdown rendering. Empty match set produces the documented
/// `No log entries mention '<slug>'.` line.
fn render_markdown(slug: &str, matches: &[&LogEntry]) -> String {
    if matches.is_empty() {
        return format!("No log entries mention '{slug}'.\n");
    }
    let mut out = String::new();
    out.push_str(&format!("# Log entries mentioning `{slug}`\n\n"));
    for entry in matches {
        out.push_str(&format!(
            "- {} `{}`: {}\n",
            entry.timestamp.to_rfc3339(),
            entry.op,
            entry.summary,
        ));
    }
    out
}

/// JSON rendering. Mirrors the Markdown shape: top-level `slug` and
/// `entries: [{timestamp, op, summary}, ...]`. Empty match set produces
/// `entries: []` (NOT an error) so scripts can branch on length.
fn render_json(slug: &str, matches: &[&LogEntry]) -> Result<String> {
    let entries: Vec<serde_json::Value> = matches
        .iter()
        .map(|e| {
            json!({
                "timestamp": e.timestamp.to_rfc3339(),
                "op": e.op,
                "summary": e.summary,
            })
        })
        .collect();
    let value = json!({
        "slug": slug,
        "entries": entries,
    });
    Ok(serde_json::to_string_pretty(&value)?)
}

#[cfg(test)]
mod tests {
    //! Tests cover the pure `filter_entries` helper (slicing semantics,
    //! reverse order, case sensitivity, empty match) plus end-to-end
    //! `coral history` smoke tests in markdown and JSON modes against a
    //! tempdir wiki.
    //!
    //! Hand-written `LogEntry`s use fixed `Utc.with_ymd_and_hms()`
    //! timestamps instead of `Utc::now()` so order is deterministic
    //! independent of test scheduling.
    use super::*;
    use assert_cmd::Command;
    use chrono::{Datelike, TimeZone};
    use serde_json::Value;
    use tempfile::TempDir;

    /// Build a `LogEntry` with a fixed timestamp. Day is a 1-31 day in
    /// April 2026 — keeps timestamps in chronological order so tests can
    /// reason about reverse iteration without depending on wall clock.
    fn entry(day: u32, op: &str, summary: &str) -> LogEntry {
        LogEntry {
            timestamp: chrono::Utc
                .with_ymd_and_hms(2026, 4, day, 10, 0, 0)
                .single()
                .expect("valid date"),
            op: op.to_string(),
            summary: summary.to_string(),
        }
    }

    /// Five entries; `order` appears in two of them (days 25 and 28).
    /// We expect those two back, newest-first (28, then 25).
    #[test]
    fn filter_entries_returns_matches_reverse_chronological() {
        let entries = vec![
            entry(25, "ingest", "updated order page sources"),
            entry(26, "lint", "no critical issues"),
            entry(27, "consolidate", "merged ghost into outbox"),
            entry(28, "ingest", "updated order page body"),
            entry(29, "lint", "1 critical: outbox missing source"),
        ];
        let got = filter_entries(&entries, "order", DEFAULT_LIMIT);
        assert_eq!(got.len(), 2, "expected 2 matches, got {}", got.len());
        // Newest first.
        assert_eq!(got[0].timestamp.day(), 28);
        assert_eq!(got[1].timestamp.day(), 25);
    }

    /// `--limit 1` against the same data returns only the newest match.
    /// Pins the slicing logic (limit applies *after* the filter).
    #[test]
    fn filter_entries_respects_limit_one() {
        let entries = vec![
            entry(25, "ingest", "updated order page sources"),
            entry(28, "ingest", "updated order page body"),
        ];
        let got = filter_entries(&entries, "order", 1);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].timestamp.day(), 28, "should be the newest match");
    }

    /// A slug nobody mentions returns an empty `Vec` — explicitly NOT an
    /// error, because the JSON output relies on `entries: []` for empty
    /// matches and we don't want CLI scripts to have to handle two error
    /// modes.
    #[test]
    fn filter_entries_no_matches_returns_empty() {
        let entries = vec![
            entry(25, "ingest", "updated order page"),
            entry(26, "lint", "no issues"),
        ];
        let got = filter_entries(&entries, "ghost", DEFAULT_LIMIT);
        assert!(got.is_empty(), "expected empty, got {} entries", got.len());
    }

    /// Case sensitivity is documented behavior — `Order` (capital O)
    /// must NOT match `order`. This guards against an accidental
    /// `to_lowercase()` regression that would let prefixes leak (e.g.
    /// `"order"` matching `"Border-collie"` after both are lowered).
    #[test]
    fn filter_entries_is_case_sensitive() {
        let entries = vec![
            entry(25, "ingest", "updated order page"),
            entry(26, "ingest", "updated Order Page (typo)"),
        ];
        let lower = filter_entries(&entries, "order", DEFAULT_LIMIT);
        assert_eq!(lower.len(), 1);
        let upper = filter_entries(&entries, "Order", DEFAULT_LIMIT);
        assert_eq!(upper.len(), 1);
        // Sanity: each only matched its own casing — not the other.
        assert!(lower[0].summary.contains("order"));
        assert!(upper[0].summary.contains("Order"));
    }

    /// Initialize a fresh `.wiki/` in `tmp` via `coral init`, then
    /// overwrite `log.md` with hand-written entries so end-to-end tests
    /// have stable, deterministic content to filter against.
    ///
    /// v0.34.0 cleanup B2: `coral init` now requires a real git HEAD,
    /// so we materialise one with an empty commit before invoking it.
    fn init_wiki_with_log(tmp: &TempDir) {
        for args in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.email", "history-test@coral.local"][..],
            &["config", "user.name", "Coral History Test"][..],
            &["commit", "-q", "--allow-empty", "-m", "fixture"][..],
        ] {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(tmp.path())
                .status()
                .expect("git invocation failed");
            assert!(
                status.success(),
                "git {args:?} failed in {}",
                tmp.path().display()
            );
        }
        Command::cargo_bin("coral")
            .unwrap()
            .current_dir(tmp.path())
            .arg("init")
            .assert()
            .success();
        let log_md = "---\n\
type: log\n\
---\n\
\n\
# Wiki operation log\n\
\n\
- 2026-04-25T10:00:00+00:00 ingest: updated order page sources\n\
- 2026-04-26T10:00:00+00:00 lint: no critical issues\n\
- 2026-04-27T10:00:00+00:00 ingest: updated order page body\n";
        std::fs::write(tmp.path().join(".wiki/log.md"), log_md).unwrap();
    }

    /// End-to-end smoke (markdown): `coral history order` against a
    /// tempdir wiki should exit 0 and contain both matching entries
    /// in the rendered list. We only check for the bullet markers and
    /// the slug heading — exact wording is pinned in the snapshot test.
    #[test]
    fn history_markdown_e2e_smoke() {
        let tmp = TempDir::new().unwrap();
        init_wiki_with_log(&tmp);
        let assert = Command::cargo_bin("coral")
            .unwrap()
            .current_dir(tmp.path())
            .args(["history", "order"])
            .assert()
            .success();
        let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
        assert!(
            stdout.contains("Log entries mentioning `order`"),
            "missing header: {stdout}"
        );
        // Two entries match `order` in the seed log.
        let bullet_count = stdout.lines().filter(|l| l.starts_with("- ")).count();
        assert_eq!(
            bullet_count, 2,
            "expected 2 bullets, got {bullet_count}: {stdout}"
        );
    }

    /// End-to-end smoke (JSON): `coral history order --format json`
    /// must produce `{"slug": "order", "entries": [...]}` with the
    /// matching entries. Acts as a contract test for the JSON shape.
    #[test]
    fn history_json_e2e_smoke() {
        let tmp = TempDir::new().unwrap();
        init_wiki_with_log(&tmp);
        let assert = Command::cargo_bin("coral")
            .unwrap()
            .current_dir(tmp.path())
            .args(["history", "order", "--format", "json"])
            .assert()
            .success();
        let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
        let json: Value = serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("history JSON did not parse: {e}\nstdout:\n{stdout}"));
        assert_eq!(json["slug"].as_str(), Some("order"));
        let entries = json["entries"].as_array().expect("entries array");
        assert_eq!(
            entries.len(),
            2,
            "expected 2 entries, got {}",
            entries.len()
        );
        // Each entry has the documented {timestamp, op, summary} shape.
        for e in entries {
            assert!(e.get("timestamp").is_some(), "entry missing timestamp: {e}");
            assert!(e.get("op").is_some(), "entry missing op: {e}");
            assert!(e.get("summary").is_some(), "entry missing summary: {e}");
        }
    }

    /// `DEFAULT_LIMIT` is exported as the documented default. Pin it so
    /// the docstring and code can't drift apart silently.
    #[test]
    fn default_limit_is_twenty() {
        assert_eq!(DEFAULT_LIMIT, 20);
    }
}
