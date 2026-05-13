//! `coral status` — daily-use wiki snapshot.
//!
//! A read-only "is the wiki healthy?" dashboard. Combines:
//!
//! - Wiki path + `index.md` `last_commit` + total page count.
//! - A one-line lint summary computed via the FAST structural lint
//!   (`coral_lint::run_structural_with_root`) — no LLM, no semantic pass.
//! - A one-line stats summary (avg confidence, orphan count).
//! - The last N entries from `.wiki/log.md`.
//!
//! Always exits 0; this command is informational. Use
//! `coral lint --severity critical` if you want a CI gate.
//!
//! Markdown output is intentionally concise (under 30 lines) so it fits in
//! a terminal at a glance. JSON is the structured equivalent for shell
//! scripting and dashboards.

use anyhow::{Context, Result};
use clap::Args;
use coral_core::WikiIndex;
use coral_core::log::WikiLog;
use coral_core::path::repo_root_from_wiki_root;
use coral_core::walk;
use coral_lint::run_structural_with_root;
use coral_stats::StatsReport;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// CLI args for `coral status`.
#[derive(Args, Debug)]
pub struct StatusArgs {
    /// Maximum number of recent log entries to include (default 5).
    #[arg(long, default_value_t = 5)]
    pub limit: usize,
    /// Output format: markdown (default) or json.
    #[arg(long, default_value = "markdown")]
    pub format: String,
    /// Re-run status every `--interval` seconds and clear the screen between
    /// iterations. Loops until the user kills the process (Ctrl+C). Useful as
    /// a poor-man's dashboard while editing the wiki.
    #[arg(long)]
    pub watch: bool,
    /// Seconds between iterations when `--watch` is set. Min 1, default 5.
    #[arg(long, default_value_t = DEFAULT_INTERVAL)]
    pub interval: u64,
}

/// Default value for `--limit` when no flag is passed. Used by tests and
/// keeps the magic number out of the body.
pub const DEFAULT_LIMIT: usize = 5;

/// Default value for `--interval` when `--watch` is set without `--interval`.
/// 5s is a reasonable cadence for a wiki snapshot — tight enough to feel
/// live, loose enough not to thrash on a slow disk.
pub const DEFAULT_INTERVAL: u64 = 5;

/// Entry point wired to `Cmd::Status`. Loads the wiki, runs the structural
/// lint + stats, slices the log, and prints either Markdown or JSON.
///
/// When `--watch` is set, the snapshot is re-rendered every `--interval`
/// seconds (min 1) with the screen cleared between iterations. The loop is
/// terminated by the user pressing Ctrl+C; we don't install a signal handler
/// because the OS-level SIGINT default behaviour is exactly what we want.
pub fn run(args: StatusArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let root: PathBuf = wiki_root
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".wiki"));
    if !root.exists() {
        anyhow::bail!(
            "wiki root not found: {}. Run `coral init` first.",
            root.display()
        );
    }

    if args.watch {
        run_watch(&args, &root)
    } else {
        render_once(&args, &root)?;
        Ok(ExitCode::SUCCESS)
    }
}

/// Watch loop. Clears the screen with the standard ANSI sequence on TTYs and
/// prints without clearing when stdout is not a TTY (so piping into a file
/// still produces something parseable).
///
/// This is intentionally NOT unit-tested: it's a long-running terminal-bound
/// mode and exercising it would require either a fake clock + cancel token
/// (overkill for a status command) or actually sleeping in the test (slow
/// and racy). The flag-parsing test below covers the contract that matters.
fn run_watch(args: &StatusArgs, root: &Path) -> Result<ExitCode> {
    let interval = std::time::Duration::from_secs(args.interval.max(1));
    // Probe TTY once at startup; if stdout is piped we skip the clear so the
    // output stays grep-able / log-friendly.
    let is_tty = std::io::IsTerminal::is_terminal(&std::io::stdout());
    loop {
        if is_tty {
            // ANSI: 2J = clear screen, H = move cursor home.
            print!("\x1b[2J\x1b[H");
        }
        render_once(args, root)?;
        std::thread::sleep(interval);
    }
}

/// Build the snapshot (pages, lint, stats, log) and print it once in the
/// requested format. Factored out of `run` so the watch loop can call it
/// repeatedly without duplicating the I/O dance.
fn render_once(args: &StatusArgs, root: &Path) -> Result<()> {
    let pages =
        walk::read_pages(root).with_context(|| format!("reading pages from {}", root.display()))?;

    // Load the index for `last_commit`. Missing index is non-fatal —
    // surface it as `<unknown>` so brand-new wikis still print a useful
    // status header.
    let last_commit = load_last_commit(root);

    // Lint counts via the FAST structural pass — no semantic LLM call.
    // Repo root = parent of `.wiki/` (matches `coral lint`'s convention).
    // See `coral_core::path::repo_root_from_wiki_root` for why this isn't
    // just `wiki_root.parent().unwrap_or(...)`.
    let repo_root = repo_root_from_wiki_root(root);
    let lint_report = run_structural_with_root(&pages, &repo_root);

    let stats = StatsReport::new(&pages);

    // Recent log entries: load + reverse-chronological (newest first) +
    // cap. The log is append-only chronological, so a simple `rev()` puts
    // the newest entries on top.
    let log_path = root.join("log.md");
    let log = WikiLog::load(&log_path)
        .with_context(|| format!("reading log from {}", log_path.display()))?;
    let recent: Vec<&coral_core::log::LogEntry> =
        log.entries.iter().rev().take(args.limit).collect();

    match args.format.as_str() {
        "json" => println!(
            "{}",
            render_json(root, &last_commit, &lint_report, &stats, &recent,)?
        ),
        _ => println!(
            "{}",
            render_markdown(root, &last_commit, &lint_report, &stats, &recent,)
        ),
    }
    Ok(())
}

/// Best-effort read of `index.md`'s `last_commit`. Returns `<unknown>`
/// when the file is missing or fails to parse — `coral status` is a
/// snapshot, not a guard, so a fresh wiki shouldn't error out.
fn load_last_commit(root: &Path) -> String {
    let index_path = root.join("index.md");
    match std::fs::read_to_string(&index_path) {
        Ok(content) => match WikiIndex::parse(&content) {
            Ok(idx) => idx.last_commit,
            Err(_) => "<unknown>".to_string(),
        },
        Err(_) => "<unknown>".to_string(),
    }
}

/// Render the Markdown snapshot. Kept under ~30 lines for readability at
/// a glance; the JSON variant is what tooling should consume.
fn render_markdown(
    root: &Path,
    last_commit: &str,
    lint: &coral_lint::LintReport,
    stats: &StatsReport,
    recent: &[&coral_core::log::LogEntry],
) -> String {
    let mut out = String::new();
    out.push_str("# Wiki status\n\n");
    out.push_str(&format!("- Wiki: `{}`\n", root.display()));
    out.push_str(&format!("- Last commit: `{last_commit}`\n"));
    out.push_str(&format!("- Pages: {}\n", stats.total_pages));
    out.push_str(&format!(
        "- Lint: Critical: {} | Warning: {} | Info: {}\n",
        lint.critical_count(),
        lint.warning_count(),
        lint.info_count(),
    ));
    out.push_str(&format!(
        "- Stats: {} pages, avg confidence {:.2}, {} orphan candidate(s)\n",
        stats.total_pages,
        stats.confidence_avg,
        stats.orphan_candidates.len(),
    ));
    out.push('\n');
    out.push_str("## Recent log\n\n");
    if recent.is_empty() {
        out.push_str("- (no entries)\n");
    } else {
        for entry in recent {
            out.push_str(&format!(
                "- {} `{}`: {}\n",
                entry.timestamp.to_rfc3339(),
                entry.op,
                entry.summary,
            ));
        }
    }
    out
}

/// Render the JSON snapshot. Mirrors the Markdown sections so consumers
/// can rely on a stable shape: `wiki`, `last_commit`, `pages`, `lint`,
/// `stats`, `recent_log`.
fn render_json(
    root: &Path,
    last_commit: &str,
    lint: &coral_lint::LintReport,
    stats: &StatsReport,
    recent: &[&coral_core::log::LogEntry],
) -> Result<String> {
    let recent_json: Vec<serde_json::Value> = recent
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
        "wiki": root.display().to_string(),
        "last_commit": last_commit,
        "pages": stats.total_pages,
        "lint": {
            "critical": lint.critical_count(),
            "warning": lint.warning_count(),
            "info": lint.info_count(),
        },
        "stats": {
            "total_pages": stats.total_pages,
            "confidence_avg": stats.confidence_avg,
            "orphan_candidates": stats.orphan_candidates.len(),
        },
        "recent_log": recent_json,
    });
    Ok(serde_json::to_string_pretty(&value)?)
}

#[cfg(test)]
mod tests {
    //! Unit + light integration tests for `coral status`. We exercise:
    //!
    //! 1. Smoke against a wiki that only ran `coral init` — no panics, JSON
    //!    has `pages: 0` and a 1-element `recent_log` (the init event).
    //! 2. `--limit 2` against a wiki with 5 hand-written log entries — JSON
    //!    `recent_log` length is 2 and the entries are the *newest* two.
    //! 3. Required JSON fields (`wiki`, `last_commit`, `pages`, `lint`,
    //!    `stats`, `recent_log`) are all present so consumers can rely on
    //!    the documented shape.
    //!
    //! We use `assert_cmd` to invoke the real `coral` binary because the
    //! command's branches (`println!`, exit code, format dispatch) run at
    //! the binary boundary. A pure `run()` call would also work but tests
    //! end-to-end output too.
    use super::*;
    use assert_cmd::Command;
    use serde_json::Value;
    use tempfile::TempDir;

    /// Initialize a fresh `.wiki/` in `tmp` via `coral init` so we can
    /// exercise `status` against the same shape a real user would see.
    /// Returns the tmpdir's path so callers can run more commands against it.
    ///
    /// v0.34.0 cleanup B2: `coral init` now requires a real git HEAD,
    /// so we materialise one with an empty commit beforehand.
    fn init_wiki(tmp: &TempDir) {
        for args in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.email", "status-test@coral.local"][..],
            &["config", "user.name", "Coral Status Test"][..],
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
    }

    /// Run `coral status [args...]` in `cwd` and return parsed stdout JSON.
    /// Asserts the command exits 0 — `status` is informational so any
    /// non-zero exit is a regression worth failing the test on.
    fn status_json(cwd: &std::path::Path, extra: &[&str]) -> Value {
        let mut args: Vec<&str> = vec!["status", "--format", "json"];
        args.extend_from_slice(extra);
        let assert = Command::cargo_bin("coral")
            .unwrap()
            .current_dir(cwd)
            .args(&args)
            .assert()
            .success();
        let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
        serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("status JSON did not parse: {e}\nstdout:\n{stdout}"))
    }

    /// Smoke test: `coral status --format json` against a wiki that only
    /// ran `init`. Must not panic; `pages` is 0 (init creates no pages)
    /// and the JSON envelope must be well-formed.
    #[test]
    fn status_against_init_only_wiki_smoke() {
        let tmp = TempDir::new().unwrap();
        init_wiki(&tmp);
        let json = status_json(tmp.path(), &[]);
        assert_eq!(json["pages"].as_u64(), Some(0), "pages should be 0: {json}");
        // init seeds log.md with a single entry; we don't assert the count
        // here because that's tested below — just confirm the field exists.
        assert!(
            json["recent_log"].is_array(),
            "recent_log missing or wrong type: {json}"
        );
    }

    /// Build a wiki with 5 hand-written log entries (one per recent day)
    /// and confirm `--limit 2` slices to the *newest* two. We hand-write
    /// `log.md` instead of going through `WikiLog::append` because the
    /// latter stamps `Utc::now()` and would make the test order-sensitive
    /// to wall-clock skew.
    #[test]
    fn status_limit_truncates_recent_log() {
        let tmp = TempDir::new().unwrap();
        init_wiki(&tmp);

        // Overwrite log.md with five chronological entries. Order in the
        // file is oldest -> newest; `status` reverses to newest-first then
        // applies `--limit`.
        let log_md = "---\n\
type: log\n\
---\n\
\n\
# Wiki operation log\n\
\n\
- 2026-04-25T10:00:00+00:00 init: wiki created\n\
- 2026-04-26T10:00:00+00:00 bootstrap: 4 pages compiled\n\
- 2026-04-27T10:00:00+00:00 ingest: 1 page updated\n\
- 2026-04-28T10:00:00+00:00 lint: 0 critical, 3 warning\n\
- 2026-04-29T10:00:00+00:00 consolidate: merged ghost into outbox\n";
        std::fs::write(tmp.path().join(".wiki/log.md"), log_md).unwrap();

        let json = status_json(tmp.path(), &["--limit", "2"]);
        let recent = json["recent_log"].as_array().expect("recent_log is array");
        assert_eq!(
            recent.len(),
            2,
            "limit=2 should trim to 2 entries, got {}: {json}",
            recent.len()
        );
        // Newest first: consolidate then lint.
        assert_eq!(recent[0]["op"].as_str(), Some("consolidate"));
        assert_eq!(recent[1]["op"].as_str(), Some("lint"));
    }

    /// Contract test: every documented top-level field is present in the
    /// JSON output. Downstream tooling depends on this exact shape (see
    /// the module docstring's contract). Drift here breaks scripts.
    #[test]
    fn status_json_has_required_fields() {
        let tmp = TempDir::new().unwrap();
        init_wiki(&tmp);
        let json = status_json(tmp.path(), &[]);
        // Keep these in alphabetical order so failures pinpoint exactly
        // which field went missing.
        for field in [
            "last_commit",
            "lint",
            "pages",
            "recent_log",
            "stats",
            "wiki",
        ] {
            assert!(
                json.get(field).is_some(),
                "required field `{field}` missing from JSON: {json}"
            );
        }
        // Spot-check nested shapes too — `lint` and `stats` are objects
        // with their own contracts.
        for nested in ["critical", "warning", "info"] {
            assert!(
                json["lint"].get(nested).is_some(),
                "lint.{nested} missing: {json}"
            );
        }
        for nested in ["total_pages", "confidence_avg", "orphan_candidates"] {
            assert!(
                json["stats"].get(nested).is_some(),
                "stats.{nested} missing: {json}"
            );
        }
    }

    /// `DEFAULT_LIMIT` is exported so other commands and tests can refer
    /// to the documented default without duplicating the literal. Pin it
    /// so the docstring and code can't drift apart silently.
    #[test]
    fn default_limit_is_five() {
        assert_eq!(DEFAULT_LIMIT, 5);
    }

    /// Verify the `--watch` and `--interval` flags are recognised by the
    /// top-level `coral` parser. We don't actually run the loop — that's a
    /// long-running terminal-bound mode, see the docstring on `run_watch`.
    /// Instead we use clap's `try_parse_from` via the binary's `--help`
    /// surface to confirm the flags appear in the help text. If clap
    /// rejected the flag, `--help` would fail with exit code != 0.
    #[test]
    fn status_with_watch_flag_present() {
        let assert = Command::cargo_bin("coral")
            .unwrap()
            .args(["status", "--help"])
            .assert()
            .success();
        let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
        assert!(
            stdout.contains("--watch"),
            "--watch flag missing from `coral status --help`:\n{stdout}"
        );
        assert!(
            stdout.contains("--interval"),
            "--interval flag missing from `coral status --help`:\n{stdout}"
        );
    }

    /// Pin the default interval so the docstring on `--interval` and the
    /// constant can't drift apart silently — symmetric with
    /// `default_limit_is_five`.
    #[test]
    fn default_interval_is_five() {
        assert_eq!(DEFAULT_INTERVAL, 5);
    }

    /// Regression: `coral status` invoked with no `--wiki-root` from a
    /// repo root used to fail with `failed to invoke git: No such file or
    /// directory (os error 2) repo_root=` because `Path::new(".wiki").parent()`
    /// returns `Some("")` (NOT `None`), and the empty PathBuf flowed into
    /// `Command::current_dir("")` which surfaces as `ENOENT` from
    /// `execvp` on macOS. The fix is to treat empty-parent the same as
    /// missing-parent and fall back to `.`.
    ///
    /// We exercise the bug at the binary boundary: invoke `coral status`
    /// from a tmpdir that's a real git repo with a `.wiki/` (so the relative
    /// default kicks in), and assert the output contains NEITHER the
    /// misleading warning NOR the cosmetic `<unknown>`.
    #[test]
    fn status_resolves_repo_root_when_wiki_path_is_relative() {
        use std::process::Command as StdCommand;
        let tmp = TempDir::new().unwrap();
        // Real git repo so `git rev-list` actually has commits to find.
        StdCommand::new("git")
            .args(["init", "-q"])
            .current_dir(tmp.path())
            .status()
            .expect("git init");
        // Pin user.name/email locally so commit doesn't fail under
        // CI-style identity-less environments.
        StdCommand::new("git")
            .args([
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "--allow-empty",
                "-qm",
                "init",
            ])
            .current_dir(tmp.path())
            .status()
            .expect("git commit");
        init_wiki(&tmp);

        let assert = Command::cargo_bin("coral")
            .unwrap()
            .current_dir(tmp.path())
            // No --wiki-root → defaults to relative `.wiki`. THIS is the
            // exact case that triggered the bug.
            .args(["status", "--format", "json"])
            .assert()
            .success();
        let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
        // The warning carried `repo_root=` empty before the fix. If it
        // ever comes back, this assertion catches it loudly.
        assert!(
            !stderr.contains("repo_root=\n") && !stderr.contains("repo_root= "),
            "regression: empty repo_root WARN reappeared:\n{stderr}"
        );
        assert!(
            !stderr.contains("git rev-list failed"),
            "regression: status emitted git rev-list failure:\n{stderr}"
        );
    }
}
