//! Error type for the `coral-session` crate.
//!
//! `thiserror` shape mirrors `coral-test::error::TestError` and the rest of
//! the workspace: each variant carries actionable context (the path that
//! failed, the parser line number, the wrapped runner failure) and a
//! `Display` message that's safe to surface to a user.

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SessionError {
    /// Filesystem failure (read/write/walk). The wrapped path is the
    /// site of the failure (the source transcript, the index file, the
    /// distilled output target, …).
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// JSONL parse failure inside a transcript. `line` is 1-indexed so
    /// the user can `sed -n <line>p file` directly. `path` is the
    /// transcript that produced the bad line.
    #[error("parse error at {path}:{line}: {message}")]
    ParseError {
        path: PathBuf,
        line: usize,
        message: String,
    },

    /// The user asked for a session id that doesn't exist on disk.
    #[error("session not found: {0}")]
    NotFound(String),

    /// Invalid input (CLI-shaped error). Used when --from is unsupported
    /// or required arguments are missing.
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// The LLM provider returned a non-OK exit (auth, network, etc.).
    /// Surfaced verbatim from `coral_runner::RunnerError` so the user
    /// gets the actionable hint about `claude setup-token` /
    /// `ANTHROPIC_API_KEY` straight from the source.
    #[error("runner failed: {0}")]
    RunnerFailed(#[from] coral_runner::RunnerError),

    /// The scrubber regex set failed to compile. Practically
    /// unreachable because every pattern is unit-tested at build time,
    /// but kept as a real variant so callers don't have to `unwrap()`.
    #[error("scrubber init failed: {0}")]
    ScrubberFailed(String),

    /// Glob expansion produced an unusable input. Mirrored from
    /// `walkdir::Error` since the v0.20 MVP doesn't pull in the `glob`
    /// crate (we expand manually under `walkdir`).
    #[error("glob error: {0}")]
    Glob(String),

    /// Distillation produced output that didn't match the contract
    /// (e.g. missing required YAML keys). We propagate the LLM stdout
    /// so the user can re-run by hand if needed.
    #[error("distill output malformed: {0}")]
    DistillMalformed(String),

    /// Source transcript exceeds the 32 MiB cap. Hard-error rather
    /// than warn-and-skip because the caller asked for *this* file
    /// specifically; silently dropping it would surprise. Surface a
    /// path + actual size + cap so the user can split or trim.
    ///
    /// v0.20.2 audit-followup #34. Mirrors
    /// [`coral_core::walk::read_pages`]'s 32 MiB cap (v0.19.5 N3) and
    /// [`coral_test::contract_check::parse_spec_file`]'s warn-cap.
    #[error(
        "session source too large: {path} is {size} bytes; cap is {cap} bytes (32 MiB). Split or trim the transcript."
    )]
    TooLarge { path: PathBuf, size: u64, cap: u64 },

    /// User aborted an interactive prompt (e.g. answered "N" to
    /// `coral session forget`). CLI maps this to a non-zero exit
    /// code so calling scripts can detect the abort.
    ///
    /// v0.20.2 audit-followup #42.
    #[error("aborted by user")]
    UserAborted,
}

pub type SessionResult<T> = std::result::Result<T, SessionError>;
