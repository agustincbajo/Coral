//! `coral session` — capture and distill agent transcripts.
//!
//! Five subcommands (v0.20.0 MVP):
//!
//! - `capture --from claude-code [path-or-glob]` — copy a Claude Code
//!   JSONL transcript into `.coral/sessions/`, scrubbing secrets by
//!   default. Auto-discovery walks `~/.claude/projects/` and picks the
//!   most-recent transcript whose `cwd` matches the current project.
//! - `list [--format markdown|json]` — render the index of captured
//!   sessions.
//! - `forget <id> [--yes]` — delete a session + distilled outputs + index entry.
//!   Without `--yes` an interactive `[y/N]` prompt is shown.
//! - `distill <id> [--apply] [--provider …] [--model …]` — single-pass
//!   LLM call that emits 1-3 synthesis pages with `reviewed: false`
//!   frontmatter. `--apply` additionally writes the same pages under
//!   `.wiki/synthesis/<slug>.md` so they show up in `coral lint` /
//!   `coral search` (still flagged as `reviewed: false` until a human
//!   flips them).
//! - `show <id>` — print the session metadata + first 5 message
//!   previews. Useful for inspecting before distilling.
//!
//! Privacy posture and trust gating are documented in
//! `docs/SESSIONS.md` (committed alongside this file).

use anyhow::{Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use coral_runner::Runner;
use coral_session::capture::{CaptureOptions, CaptureSource, capture_from_path};
use coral_session::claude_code::{find_latest_for_cwd, parse_transcript};
use coral_session::distill::{DistillOptions, distill_session};
use coral_session::forget::{ForgetOptions, forget_session};
use coral_session::list::{ListFormat, list_sessions};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Top-level args. Carries the subcommand discriminant + the
/// arguments common to every subcommand (currently none — wiki_root
/// is a global flag handled in `main.rs`).
#[derive(Args, Debug)]
pub struct SessionArgs {
    #[command(subcommand)]
    pub cmd: SessionCmd,
}

#[derive(Subcommand, Debug)]
pub enum SessionCmd {
    /// Capture an agent transcript into `.coral/sessions/`.
    Capture(CaptureArgs),
    /// List captured sessions.
    List(ListArgs),
    /// Delete a captured session.
    Forget(ForgetArgs),
    /// Distill a captured transcript into synthesis pages.
    Distill(DistillArgs),
    /// Print metadata + first messages of a captured session.
    Show(ShowArgs),
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum FromClient {
    /// Claude Code (the only client supported in v0.20.0).
    ClaudeCode,
    /// Cursor — recognized at parse time, but `capture` currently
    /// emits a clear "not yet implemented; track #16" error.
    Cursor,
    /// ChatGPT export (Markdown). Same status as Cursor.
    Chatgpt,
}

impl FromClient {
    fn to_source(self) -> CaptureSource {
        match self {
            FromClient::ClaudeCode => CaptureSource::ClaudeCode,
            FromClient::Cursor => CaptureSource::Cursor,
            FromClient::Chatgpt => CaptureSource::Chatgpt,
        }
    }
}

#[derive(Args, Debug)]
pub struct CaptureArgs {
    /// Source agent client. Only `claude-code` is fully implemented in v0.20.0.
    #[arg(long, value_enum, default_value_t = FromClient::ClaudeCode)]
    pub from: FromClient,

    /// Path to a transcript file. When omitted, walk
    /// `~/.claude/projects/` and pick the most-recent transcript whose
    /// recorded `cwd` matches the current project root.
    #[arg(value_name = "PATH")]
    pub path: Option<PathBuf>,

    /// Skip the privacy scrubber. **DANGEROUS** — captured bytes
    /// will retain any tokens / secrets pasted into the chat. Must
    /// be combined with `--yes-i-really-mean-it` to take effect.
    #[arg(long)]
    pub no_scrub: bool,

    /// Confirmation flag for `--no-scrub`. The PRD mandates a literal
    /// `--yes-i-really-mean-it` so the user actively types the
    /// confirmation when opting out.
    #[arg(long)]
    pub yes_i_really_mean_it: bool,
}

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Output format.
    #[arg(long, value_enum, default_value_t = ListOutputFormat::Markdown)]
    pub format: ListOutputFormat,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum ListOutputFormat {
    Markdown,
    Json,
}

#[derive(Args, Debug)]
pub struct ForgetArgs {
    /// The session id to delete (full UUID or any unique 4+-char prefix).
    #[arg(value_name = "SESSION_ID")]
    pub session_id: String,

    /// Skip the interactive confirmation prompt.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

#[derive(Args, Debug)]
pub struct DistillArgs {
    /// The session id to distill (full UUID or any unique 4+-char prefix).
    #[arg(value_name = "SESSION_ID")]
    pub session_id: String,

    /// Additionally write each finding to `.wiki/synthesis/<slug>.md`
    /// so it shows up in `coral lint` / `coral search`. The pages
    /// land with `reviewed: false` and lint blocks the commit until
    /// the reviewer flips it.
    #[arg(long)]
    pub apply: bool,

    /// LLM provider override (claude / gemini / local / http). Falls
    /// back to `CORAL_PROVIDER` env or `claude` otherwise.
    #[arg(long)]
    pub provider: Option<String>,

    /// Model alias to pass through to the runner (e.g. `sonnet`,
    /// `haiku`).
    #[arg(long)]
    pub model: Option<String>,
}

#[derive(Args, Debug)]
pub struct ShowArgs {
    /// The session id to show (full UUID or any unique 4+-char prefix).
    #[arg(value_name = "SESSION_ID")]
    pub session_id: String,

    /// Number of messages to preview (default: 5).
    #[arg(long, default_value_t = 5)]
    pub n: usize,
}

pub fn run(args: SessionArgs, _wiki_root: Option<&Path>) -> Result<ExitCode> {
    // The session feature lives at the project root, not under
    // `.wiki/`. We use the cwd as the project root — same convention
    // as `coral env`, `coral up`, `coral test`.
    let project_root = std::env::current_dir().context("getting cwd")?;
    match args.cmd {
        SessionCmd::Capture(a) => run_capture(a, &project_root),
        SessionCmd::List(a) => run_list(a, &project_root),
        SessionCmd::Forget(a) => run_forget(a, &project_root),
        SessionCmd::Distill(a) => run_distill(a, &project_root, None),
        SessionCmd::Show(a) => run_show(a, &project_root),
    }
}

fn run_capture(args: CaptureArgs, project_root: &Path) -> Result<ExitCode> {
    if args.no_scrub && !args.yes_i_really_mean_it {
        anyhow::bail!(
            "refusing to skip the privacy scrubber: pass --yes-i-really-mean-it alongside --no-scrub.\n\
             Captured transcripts may contain API keys, JWTs, AWS creds, etc. — see docs/SESSIONS.md."
        );
    }
    let source_path = match args.path.clone() {
        Some(p) => p,
        None => match args.from {
            FromClient::ClaudeCode => {
                let home = home_dir().ok_or_else(|| {
                    anyhow::anyhow!("could not determine $HOME to discover Claude Code transcripts")
                })?;
                let found =
                    find_latest_for_cwd(&home, project_root).map_err(|e| anyhow::anyhow!(e))?;
                found.ok_or_else(|| {
                    anyhow::anyhow!(
                        "no Claude Code transcript found whose `cwd` matches {} \
                         under {}/.claude/projects/. \
                         Run a Claude Code session in this project first, or pass an explicit path.",
                        project_root.display(),
                        home.display()
                    )
                })?
            }
            other => anyhow::bail!(
                "auto-discovery is not implemented for source '{:?}'. Pass an explicit path. \
                 Cross-format support is tracked in issue #16.",
                other
            ),
        },
    };

    let opts = CaptureOptions {
        source_path,
        source: args.from.to_source(),
        project_root: project_root.to_path_buf(),
        scrub_secrets: !args.no_scrub,
    };
    let outcome = capture_from_path(&opts).map_err(|e| anyhow::anyhow!(e))?;
    println!(
        "captured {} ({} messages, {} redactions) → {}",
        outcome.session_id,
        outcome.message_count,
        outcome.redaction_count,
        outcome.captured_path.display()
    );
    Ok(ExitCode::SUCCESS)
}

fn run_list(args: ListArgs, project_root: &Path) -> Result<ExitCode> {
    let fmt = match args.format {
        ListOutputFormat::Markdown => ListFormat::Markdown,
        ListOutputFormat::Json => ListFormat::Json,
    };
    let out = list_sessions(project_root, fmt).map_err(|e| anyhow::anyhow!(e))?;
    print!("{out}");
    if !out.ends_with('\n') {
        println!();
    }
    Ok(ExitCode::SUCCESS)
}

fn run_forget(args: ForgetArgs, project_root: &Path) -> Result<ExitCode> {
    // v0.20.2 audit-followup #42: resolve the prefix to its
    // canonical full session id BEFORE prompting the user, so the
    // confirmation message shows what's actually about to be
    // deleted (helpful for ambiguity-prone short prefixes — and a
    // mismatch between user-typed prefix and resolved id is
    // exactly when accidental deletes happen).
    if !args.yes {
        // Reuse the same matching rule as the underlying
        // `forget_session` (which collects then errors on >1) so a
        // doomed forget surfaces the ambiguity error here too —
        // with a friendlier prompt cancellation flow.
        let resolved = resolve_session_for_forget(project_root, &args.session_id)?;
        let ok = prompt_yes_no(&format!("Delete session {resolved}? [y/N]: "))?;
        if !ok {
            // v0.20.2 audit-followup #42: previously we returned
            // `Ok(ExitCode::SUCCESS)` on user-abort, which made
            // calling scripts treat the no-op as a successful
            // delete. Surface a real error so the CLI maps to a
            // non-zero exit code.
            anyhow::bail!("aborted");
        }
    }
    let opts = ForgetOptions {
        project_root: project_root.to_path_buf(),
        session_id: args.session_id.clone(),
    };
    forget_session(&opts).map_err(|e| anyhow::anyhow!(e))?;
    println!("deleted session {}", args.session_id);
    Ok(ExitCode::SUCCESS)
}

/// v0.20.2 audit-followup #42: resolve a user-typed prefix to its
/// canonical full session id by reading `index.json` and matching
/// the same "exact id OR starts_with prefix" rule that
/// `forget_session` uses internally. The CLI calls this before
/// prompting so the confirmation message echoes the canonical id
/// rather than the prefix the user typed.
fn resolve_session_for_forget(project_root: &Path, prefix: &str) -> Result<String> {
    let index_path = project_root.join(".coral/sessions/index.json");
    let index = coral_session::capture::read_index(&index_path).map_err(|e| anyhow::anyhow!(e))?;
    let matches: Vec<&coral_session::capture::IndexEntry> = index
        .sessions
        .iter()
        .filter(|e| e.session_id == prefix || e.session_id.starts_with(prefix))
        .collect();
    if matches.is_empty() {
        anyhow::bail!("session not found: {prefix}");
    }
    if matches.len() > 1 {
        anyhow::bail!(
            "session id '{prefix}' matches {} sessions; use a longer prefix or full id",
            matches.len()
        );
    }
    Ok(matches[0].session_id.clone())
}

/// `run_distill` factored to take an optional injected runner so the
/// integration tests can swap in a `MockRunner` without re-routing
/// through clap. Public-but-not-doc-published.
pub(crate) fn run_distill(
    args: DistillArgs,
    project_root: &Path,
    injected_runner: Option<Box<dyn Runner>>,
) -> Result<ExitCode> {
    let (runner, runner_name): (Box<dyn Runner>, String) = match injected_runner {
        Some(r) => (r, "mock".into()),
        None => {
            let provider = super::runner_helper::resolve_provider(args.provider.as_deref())
                .map_err(|e| anyhow::anyhow!(e))?;
            let r = super::runner_helper::make_runner(provider);
            (r, format!("{provider:?}").to_lowercase())
        }
    };
    let opts = DistillOptions {
        project_root: project_root.to_path_buf(),
        session_id: args.session_id.clone(),
        apply: args.apply,
        model: args.model.clone(),
    };
    let outcome =
        distill_session(&opts, runner.as_ref(), &runner_name).map_err(|e| anyhow::anyhow!(e))?;
    println!(
        "distilled {} → {} finding(s):",
        outcome.session_id,
        outcome.findings.len()
    );
    for (i, f) in outcome.findings.iter().enumerate() {
        println!("  {i}. {}: {}", f.slug, f.title);
    }
    println!("written:");
    for w in &outcome.written {
        println!("  - {}", w.display());
    }
    println!(
        "\nNOTE: every emitted page is `reviewed: false`. Flip to `true` after human review;\n\
         `coral lint` blocks any commit that contains an unreviewed page."
    );
    Ok(ExitCode::SUCCESS)
}

fn run_show(args: ShowArgs, project_root: &Path) -> Result<ExitCode> {
    let index_path = project_root.join(".coral/sessions/index.json");
    let index = coral_session::capture::read_index(&index_path).map_err(|e| anyhow::anyhow!(e))?;
    // v0.20.2 audit-followup #41: collect every entry whose session
    // id matches the provided prefix instead of `.find`-ing the
    // first one. `forget`/`distill` already raise on `>1` matches;
    // `show` was silently picking the first, which is the wrong
    // page to display when two sessions share a 4-char prefix.
    let matches: Vec<&coral_session::capture::IndexEntry> = index
        .sessions
        .iter()
        .filter(|e| e.session_id == args.session_id || e.session_id.starts_with(&args.session_id))
        .collect();
    if matches.is_empty() {
        anyhow::bail!("session not found: {}", args.session_id);
    }
    if matches.len() > 1 {
        // Match the message shape used by forget/distill so the
        // user gets a consistent error across the three commands.
        anyhow::bail!(
            "session id '{}' matches {} sessions; use a longer prefix or full id",
            args.session_id,
            matches.len()
        );
    }
    let entry = matches[0];
    let parsed = parse_transcript(&entry.captured_path).map_err(|e| anyhow::anyhow!(e))?;
    println!("# session {}", entry.session_id);
    println!("source:        {}", entry.source.as_str());
    println!("captured_at:   {}", entry.captured_at.to_rfc3339());
    println!("captured_path: {}", entry.captured_path.display());
    println!("messages:      {}", entry.message_count);
    println!("redactions:    {}", entry.redaction_count);
    println!("distilled:     {}", entry.distilled);
    println!(
        "\n## first {} message(s)\n",
        args.n.min(parsed.messages.len())
    );
    for (i, m) in parsed.messages.iter().take(args.n).enumerate() {
        const PREVIEW_CHARS: usize = 200;
        let snippet: String = m.text.chars().take(PREVIEW_CHARS).collect();
        let dots = if m.text.chars().count() > PREVIEW_CHARS {
            "…"
        } else {
            ""
        };
        println!("{i}. [{}] {snippet}{dots}", m.role);
    }
    Ok(ExitCode::SUCCESS)
}

/// Reads `$HOME` (or `%USERPROFILE%` on Windows). Pure stdlib so we
/// don't pull in `dirs` for one path.
fn home_dir() -> Option<PathBuf> {
    if let Ok(h) = std::env::var("HOME") {
        if !h.is_empty() {
            return Some(PathBuf::from(h));
        }
    }
    if cfg!(windows) {
        if let Ok(h) = std::env::var("USERPROFILE") {
            if !h.is_empty() {
                return Some(PathBuf::from(h));
            }
        }
    }
    None
}

/// Tiny interactive `[y/N]` prompt. Defaults to "no" on empty input
/// or anything that doesn't start with `y`/`Y`. Reads a single line
/// from stdin.
fn prompt_yes_no(prompt: &str) -> Result<bool> {
    use std::io::Write as _;
    print!("{prompt}");
    std::io::stdout().flush().ok();
    let mut buf = String::new();
    std::io::stdin()
        .read_line(&mut buf)
        .context("reading confirmation from stdin")?;
    Ok(matches!(buf.trim().chars().next(), Some('y') | Some('Y')))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `--no-scrub` without the confirmation flag fails fast with a
    /// clear message — the privacy gate the PRD mandates.
    #[test]
    fn capture_no_scrub_without_confirmation_fails() {
        let dir = tempfile::TempDir::new().unwrap();
        let args = CaptureArgs {
            from: FromClient::ClaudeCode,
            path: Some(PathBuf::from("/dev/null")),
            no_scrub: true,
            yes_i_really_mean_it: false,
        };
        let err = run_capture(args, dir.path()).unwrap_err();
        assert!(
            err.to_string().contains("yes-i-really-mean-it"),
            "expected confirmation hint, got: {err}"
        );
    }

    /// `home_dir()` returns Some when `HOME` is set.
    #[test]
    fn home_dir_returns_home_env() {
        // SAFETY: this test only reads HOME; doesn't mutate. Avoid
        // contention with other env-mutating tests.
        if let Ok(h) = std::env::var("HOME") {
            if !h.is_empty() {
                assert_eq!(home_dir().as_deref(), Some(Path::new(&h)));
            }
        }
    }

    /// v0.20.2 audit-followup #41: regression — `coral session
    /// show <prefix>` rejects ambiguous prefixes the same way
    /// `forget` / `distill` do, instead of silently picking the
    /// first match.
    ///
    /// The matrix:
    /// - 0 matches → "session not found"
    /// - 1 match → renders the session normally
    /// - 2+ matches → "matches N sessions; use a longer prefix"
    #[test]
    fn run_show_rejects_ambiguous_prefix() {
        use chrono::TimeZone;
        use coral_session::capture::{CaptureSource, IndexEntry, SessionIndex, write_index};

        let dir = tempfile::TempDir::new().unwrap();
        let sessions_dir = dir.path().join(".coral/sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let index_path = sessions_dir.join("index.json");

        // Two sessions with the same 7-char prefix `5c359da`.
        let captured_at = chrono::Utc.with_ymd_and_hms(2026, 5, 8, 10, 0, 0).unwrap();
        let mut idx = SessionIndex::default();
        for short in ["5c359daf", "5c359dab"] {
            idx.sessions.push(IndexEntry {
                session_id: format!("{short}-full-id"),
                source: CaptureSource::ClaudeCode,
                captured_at,
                captured_path: sessions_dir.join(format!("{short}.jsonl")),
                message_count: 1,
                redaction_count: 0,
                distilled: false,
                distilled_outputs: Vec::new(),
            });
        }
        write_index(&index_path, &idx).unwrap();

        let args = ShowArgs {
            session_id: "5c359da".into(),
            n: 5,
        };
        let err = run_show(args, dir.path()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("matches 2 sessions"),
            "expected ambiguity error mentioning 2 matches, got: {msg}"
        );
        assert!(
            msg.contains("longer prefix"),
            "error must hint at the fix: {msg}"
        );
    }

    /// v0.20.2 audit-followup #41: a unique prefix continues to work.
    /// Pin the negative case so we don't regress the happy path.
    #[test]
    fn run_show_accepts_unique_prefix() {
        use chrono::TimeZone;
        use coral_session::capture::{CaptureSource, IndexEntry, SessionIndex, write_index};

        let dir = tempfile::TempDir::new().unwrap();
        let sessions_dir = dir.path().join(".coral/sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let captured_path = sessions_dir.join("session.jsonl");
        // Build a minimal valid Claude Code transcript so
        // parse_transcript inside run_show doesn't fail.
        std::fs::write(
            &captured_path,
            r#"{"type":"user","sessionId":"unique-001","timestamp":"2026-05-08T10:00:00Z","cwd":"/x","message":{"content":"hi"}}
"#,
        )
        .unwrap();
        let captured_at = chrono::Utc.with_ymd_and_hms(2026, 5, 8, 10, 0, 0).unwrap();
        let mut idx = SessionIndex::default();
        idx.sessions.push(IndexEntry {
            session_id: "unique-001".into(),
            source: CaptureSource::ClaudeCode,
            captured_at,
            captured_path: captured_path.clone(),
            message_count: 1,
            redaction_count: 0,
            distilled: false,
            distilled_outputs: Vec::new(),
        });
        let index_path = sessions_dir.join("index.json");
        write_index(&index_path, &idx).unwrap();
        let args = ShowArgs {
            session_id: "uniq".into(),
            n: 5,
        };
        let exit = run_show(args, dir.path()).expect("unique prefix must succeed");
        assert_eq!(exit, ExitCode::SUCCESS);
    }

    /// v0.20.2 audit-followup #41: 0 matches → not-found error.
    #[test]
    fn run_show_returns_not_found_for_unknown_prefix() {
        use coral_session::capture::{SessionIndex, write_index};
        let dir = tempfile::TempDir::new().unwrap();
        let sessions_dir = dir.path().join(".coral/sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        write_index(&sessions_dir.join("index.json"), &SessionIndex::default()).unwrap();
        let args = ShowArgs {
            session_id: "nonexistent".into(),
            n: 5,
        };
        let err = run_show(args, dir.path()).unwrap_err();
        assert!(
            err.to_string().contains("session not found"),
            "expected not-found, got: {err}"
        );
    }

    /// v0.20.2 audit-followup #42: `resolve_session_for_forget`
    /// returns the canonical id for a unique prefix, errors with
    /// "matches N sessions" for ambiguous, and "session not found"
    /// for unknown.
    #[test]
    fn resolve_session_for_forget_canonicalizes_prefix() {
        use chrono::TimeZone;
        use coral_session::capture::{CaptureSource, IndexEntry, SessionIndex, write_index};

        let dir = tempfile::TempDir::new().unwrap();
        let sessions_dir = dir.path().join(".coral/sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let mut idx = SessionIndex::default();
        idx.sessions.push(IndexEntry {
            session_id: "5c359daf-full".into(),
            source: CaptureSource::ClaudeCode,
            captured_at: chrono::Utc.with_ymd_and_hms(2026, 5, 8, 10, 0, 0).unwrap(),
            captured_path: sessions_dir.join("session.jsonl"),
            message_count: 1,
            redaction_count: 0,
            distilled: false,
            distilled_outputs: Vec::new(),
        });
        write_index(&sessions_dir.join("index.json"), &idx).unwrap();
        let canonical = resolve_session_for_forget(dir.path(), "5c359").unwrap();
        assert_eq!(canonical, "5c359daf-full");
    }

    /// v0.20.2 audit-followup #42: `resolve_session_for_forget`
    /// errors on ambiguous prefix.
    #[test]
    fn resolve_session_for_forget_rejects_ambiguous_prefix() {
        use chrono::TimeZone;
        use coral_session::capture::{CaptureSource, IndexEntry, SessionIndex, write_index};

        let dir = tempfile::TempDir::new().unwrap();
        let sessions_dir = dir.path().join(".coral/sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let mut idx = SessionIndex::default();
        for short in ["5c359daf", "5c359dab"] {
            idx.sessions.push(IndexEntry {
                session_id: format!("{short}-full"),
                source: CaptureSource::ClaudeCode,
                captured_at: chrono::Utc.with_ymd_and_hms(2026, 5, 8, 10, 0, 0).unwrap(),
                captured_path: sessions_dir.join(format!("{short}.jsonl")),
                message_count: 1,
                redaction_count: 0,
                distilled: false,
                distilled_outputs: Vec::new(),
            });
        }
        write_index(&sessions_dir.join("index.json"), &idx).unwrap();
        let err = resolve_session_for_forget(dir.path(), "5c359").unwrap_err();
        assert!(err.to_string().contains("matches 2 sessions"), "got: {err}");
    }

    /// v0.20.2 audit-followup #42: `coral session forget --yes`
    /// without an interactive prompt still works (no abort path).
    /// This is the regression-anchor: pin that --yes still bypasses
    /// the prompt entirely.
    #[test]
    fn run_forget_yes_bypasses_prompt() {
        use chrono::TimeZone;
        use coral_session::capture::{CaptureSource, IndexEntry, SessionIndex, write_index};
        let dir = tempfile::TempDir::new().unwrap();
        let sessions_dir = dir.path().join(".coral/sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let captured = sessions_dir.join("session.jsonl");
        std::fs::write(&captured, "raw").unwrap();
        let mut idx = SessionIndex::default();
        idx.sessions.push(IndexEntry {
            session_id: "abcdef0123456".into(),
            source: CaptureSource::ClaudeCode,
            captured_at: chrono::Utc.with_ymd_and_hms(2026, 5, 8, 10, 0, 0).unwrap(),
            captured_path: captured.clone(),
            message_count: 1,
            redaction_count: 0,
            distilled: false,
            distilled_outputs: Vec::new(),
        });
        write_index(&sessions_dir.join("index.json"), &idx).unwrap();
        let args = ForgetArgs {
            session_id: "abcdef01".into(),
            yes: true,
        };
        let exit = run_forget(args, dir.path()).expect("--yes path must succeed");
        assert_eq!(exit, ExitCode::SUCCESS);
        assert!(!captured.exists(), "session must be deleted");
    }
}
