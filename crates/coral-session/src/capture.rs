//! `coral session capture` — copy + scrub a transcript into the
//! project's `.coral/sessions/` directory.
//!
//! Layout written:
//!
//! ```text
//! .coral/sessions/
//! ├── 2026-05-08_claude-code_<sha8>.jsonl     # captured transcript
//! └── index.json                              # metadata for `list/forget`
//! ```
//!
//! The filename's `<sha8>` is a deterministic hash of the
//! claude-side session id + first-message timestamp so re-capturing
//! the same session lands in the same file (idempotent, even though
//! the JSONL bytes themselves may differ on a repeat capture if the
//! source moved). The `index.json` is a single
//! `{ sessions: [...] }` document that `list` reads and `forget`
//! prunes from.

use crate::claude_code::{ParsedTranscript, parse_transcript};
use crate::error::{SessionError, SessionResult};
use crate::scrub;
use chrono::{DateTime, Utc};
use coral_core::atomic::atomic_write_string;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Source-format selector. Only `ClaudeCode` is implemented in the
/// v0.20.0 MVP. The other variants exist on the enum so the CLI can
/// emit a clean "not yet implemented; track issue #16" error rather
/// than rejecting the flag at clap-parse time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureSource {
    ClaudeCode,
    Cursor,
    Chatgpt,
}

impl CaptureSource {
    pub fn as_str(self) -> &'static str {
        match self {
            CaptureSource::ClaudeCode => "claude-code",
            CaptureSource::Cursor => "cursor",
            CaptureSource::Chatgpt => "chatgpt",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "claude-code" | "claude_code" | "claude" => Ok(Self::ClaudeCode),
            "cursor" => Ok(Self::Cursor),
            "chatgpt" => Ok(Self::Chatgpt),
            other => Err(format!(
                "unknown source '{other}' (valid: claude-code, cursor, chatgpt)"
            )),
        }
    }
}

/// Knobs for [`capture_from_path`].
#[derive(Debug, Clone)]
pub struct CaptureOptions {
    /// Path to the source transcript file. Required for the v0.20
    /// MVP — auto-discovery via `~/.claude/projects` lives in the
    /// CLI layer (it composes [`crate::claude_code::find_latest_for_cwd`]
    /// with this function).
    pub source_path: PathBuf,
    /// Source-format adapter to use.
    pub source: CaptureSource,
    /// Project root. The capture lands under
    /// `<project_root>/.coral/sessions/`.
    pub project_root: PathBuf,
    /// When false, [`scrub::scrub`] is skipped and the original
    /// bytes are written verbatim. The CLI requires a literal
    /// `--yes-i-really-mean-it` confirmation flag to opt out — see
    /// the v0.20 PRD risk note.
    pub scrub_secrets: bool,
}

/// Outcome of a successful capture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureOutcome {
    pub session_id: String,
    pub captured_path: PathBuf,
    pub message_count: usize,
    pub redaction_count: usize,
    pub source: CaptureSource,
    pub captured_at: DateTime<Utc>,
}

/// Single entry in the on-disk `index.json`. Public so `list.rs` and
/// `forget.rs` can read/write the same shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub session_id: String,
    pub source: CaptureSource,
    pub captured_at: DateTime<Utc>,
    pub captured_path: PathBuf,
    pub message_count: usize,
    pub redaction_count: usize,
    /// `true` once `coral session distill <id>` has run successfully
    /// against this entry. The CLI flips it; the field is in the
    /// index so `list` can emit a `distilled: yes/no` column without
    /// re-parsing every JSONL.
    #[serde(default)]
    pub distilled: bool,
}

/// On-disk shape of `.coral/sessions/index.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionIndex {
    pub sessions: Vec<IndexEntry>,
}

/// Captures `opts.source_path` into `.coral/sessions/`. Atomic write
/// + index update under [`coral_core::atomic::with_exclusive_lock`].
///
/// Behavior contract:
///
/// 1. Source path must exist and be a parseable Claude Code JSONL
///    (the only source supported in v0.20).
/// 2. Output filename is deterministic for a given (session_id,
///    captured_at) pair so re-running capture against the same
///    source overwrites in place rather than creating duplicates.
/// 3. The transcript is scrubbed by default. `opts.scrub_secrets =
///    false` writes the original bytes verbatim.
/// 4. `index.json` is updated atomically in place — either the
///    entry is appended (new session) or replaced (re-capture of
///    the same session).
pub fn capture_from_path(opts: &CaptureOptions) -> SessionResult<CaptureOutcome> {
    if opts.source != CaptureSource::ClaudeCode {
        return Err(SessionError::InvalidInput(format!(
            "source '{}' is not yet implemented; track issue #16. Only --from claude-code ships in v0.20.",
            opts.source.as_str()
        )));
    }
    if !opts.source_path.exists() {
        return Err(SessionError::NotFound(format!(
            "source transcript not found: {}",
            opts.source_path.display()
        )));
    }

    let parsed: ParsedTranscript = parse_transcript(&opts.source_path)?;

    // Deterministic short hash: first 8 hex chars of FNV-1a over
    // (session_id || captured_at). Stays stable across capture
    // invocations of the same session. We lift FNV from std-only
    // primitives so we don't pull in another hashing crate.
    let sha8 = short_hash(&parsed.session_id, &parsed.captured_at.to_rfc3339());

    let date = parsed.captured_at.format("%Y-%m-%d").to_string();
    let sessions_dir = opts.project_root.join(".coral").join("sessions");
    std::fs::create_dir_all(&sessions_dir).map_err(|source| SessionError::Io {
        path: sessions_dir.clone(),
        source,
    })?;
    let captured_filename = format!("{date}_{}_{sha8}.jsonl", opts.source.as_str());
    let captured_path = sessions_dir.join(&captured_filename);

    // Read source bytes and (optionally) scrub.
    let raw = std::fs::read_to_string(&opts.source_path).map_err(|source| SessionError::Io {
        path: opts.source_path.clone(),
        source,
    })?;
    let (final_text, redactions) = if opts.scrub_secrets {
        scrub::scrub(&raw)
    } else {
        (raw, Vec::new())
    };

    atomic_write_string(&captured_path, &final_text)
        .map_err(|e| coral_core_error_to_session(&captured_path, e))?;

    // Update index.json under exclusive lock so a concurrent
    // capture against the same project doesn't lose entries.
    let index_path = sessions_dir.join("index.json");
    coral_core::atomic::with_exclusive_lock(&index_path, || {
        let mut index = read_index(&index_path).unwrap_or_default();
        let entry = IndexEntry {
            session_id: parsed.session_id.clone(),
            source: opts.source,
            captured_at: parsed.captured_at,
            captured_path: captured_path.clone(),
            message_count: parsed.messages.len(),
            redaction_count: redactions.len(),
            distilled: false,
        };
        // Replace any prior entry for the same session_id.
        index.sessions.retain(|e| e.session_id != entry.session_id);
        index.sessions.push(entry);
        // Sort by captured_at descending so `list` is already in
        // display order without re-sorting at read time.
        index
            .sessions
            .sort_by_key(|e| std::cmp::Reverse(e.captured_at));
        write_index(&index_path, &index)
    })
    .map_err(|e| coral_core_error_to_session(&index_path, e))?;

    Ok(CaptureOutcome {
        session_id: parsed.session_id,
        captured_path,
        message_count: parsed.messages.len(),
        redaction_count: redactions.len(),
        source: opts.source,
        captured_at: parsed.captured_at,
    })
}

/// FNV-1a 64-bit, returned as the first 8 hex chars. Pure stdlib so
/// we don't pull in `sha2` for a non-cryptographic identifier.
fn short_hash(a: &str, b: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h: u64 = FNV_OFFSET;
    for byte in a.bytes().chain(b'\0'..=b'\0').chain(b.bytes()) {
        h ^= byte as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    format!("{:08x}", (h >> 32) as u32)
}

/// Reads `index.json`. Returns `Ok(None)` if the file doesn't exist;
/// callers default to an empty index.
pub fn read_index(path: &Path) -> SessionResult<SessionIndex> {
    if !path.exists() {
        return Ok(SessionIndex::default());
    }
    let content = std::fs::read_to_string(path).map_err(|source| SessionError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&content).map_err(|e| SessionError::ParseError {
        path: path.to_path_buf(),
        line: 0,
        message: e.to_string(),
    })
}

/// Atomic-writes `index.json`. Caller is responsible for holding the
/// exclusive lock — this helper does NOT take it on its own (the
/// canonical pattern is to call this inside `with_exclusive_lock`).
pub fn write_index(path: &Path, index: &SessionIndex) -> coral_core::error::Result<()> {
    let json = serde_json::to_string_pretty(index)
        .map_err(|e| coral_core::error::CoralError::Walk(format!("serialize index: {e}")))?;
    atomic_write_string(path, &json)
}

/// Lift a `coral_core::error::CoralError` into our `SessionError`,
/// preserving the path that failed when possible.
fn coral_core_error_to_session(
    fallback_path: &Path,
    err: coral_core::error::CoralError,
) -> SessionError {
    match err {
        coral_core::error::CoralError::Io { path, source } => SessionError::Io { path, source },
        other => SessionError::Io {
            path: fallback_path.to_path_buf(),
            source: std::io::Error::other(format!("{other}")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// End-to-end: a small fixture transcript captures into the
    /// expected file, scrubs the embedded API key, and updates the
    /// index.
    #[test]
    fn capture_writes_redacted_jsonl_and_index() {
        let proj = TempDir::new().unwrap();
        let src_dir = TempDir::new().unwrap();
        let src_path = src_dir.path().join("session.jsonl");
        // Tiny fixture with one message containing a fake Anthropic
        // key. The scrubber should redact it before bytes hit
        // .coral/sessions/.
        std::fs::write(
            &src_path,
            r#"{"type":"user","sessionId":"sess-001","timestamp":"2026-05-08T10:00:00Z","cwd":"/x","message":{"content":"my key is sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAA"}}
"#,
        )
        .unwrap();

        let opts = CaptureOptions {
            source_path: src_path.clone(),
            source: CaptureSource::ClaudeCode,
            project_root: proj.path().to_path_buf(),
            scrub_secrets: true,
        };
        let outcome = capture_from_path(&opts).unwrap();
        assert_eq!(outcome.session_id, "sess-001");
        assert_eq!(outcome.message_count, 1);
        assert_eq!(outcome.redaction_count, 1);
        assert!(outcome.captured_path.exists());

        // Captured bytes must NOT contain the original key.
        let captured = std::fs::read_to_string(&outcome.captured_path).unwrap();
        assert!(!captured.contains("sk-ant-api03"));
        assert!(captured.contains("[REDACTED:anthropic_key]"));

        // index.json must have one entry pointing at the captured path.
        let idx_path = proj.path().join(".coral/sessions/index.json");
        let idx = read_index(&idx_path).unwrap();
        assert_eq!(idx.sessions.len(), 1);
        assert_eq!(idx.sessions[0].session_id, "sess-001");
        assert_eq!(idx.sessions[0].redaction_count, 1);
        assert!(!idx.sessions[0].distilled);
    }

    /// Re-capturing the same source replaces the prior index entry
    /// rather than appending — idempotency contract.
    #[test]
    fn recapture_replaces_index_entry_in_place() {
        let proj = TempDir::new().unwrap();
        let src_dir = TempDir::new().unwrap();
        let src_path = src_dir.path().join("session.jsonl");
        std::fs::write(
            &src_path,
            r#"{"type":"user","sessionId":"sess-002","timestamp":"2026-05-08T10:00:00Z","cwd":"/x","message":{"content":"hi"}}
"#,
        )
        .unwrap();
        let opts = CaptureOptions {
            source_path: src_path.clone(),
            source: CaptureSource::ClaudeCode,
            project_root: proj.path().to_path_buf(),
            scrub_secrets: true,
        };
        let _ = capture_from_path(&opts).unwrap();
        let _ = capture_from_path(&opts).unwrap();
        let idx_path = proj.path().join(".coral/sessions/index.json");
        let idx = read_index(&idx_path).unwrap();
        assert_eq!(idx.sessions.len(), 1, "expected idempotent capture");
    }

    /// Cursor / chatgpt sources return a clean InvalidInput error
    /// pointing at the issue tracker.
    #[test]
    fn unsupported_sources_return_invalid_input() {
        let proj = TempDir::new().unwrap();
        let opts = CaptureOptions {
            source_path: PathBuf::from("/dev/null"),
            source: CaptureSource::Cursor,
            project_root: proj.path().to_path_buf(),
            scrub_secrets: true,
        };
        let err = capture_from_path(&opts).unwrap_err();
        match err {
            SessionError::InvalidInput(msg) => {
                assert!(msg.contains("not yet implemented"));
                assert!(msg.contains("#16"));
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    /// Source path that doesn't exist surfaces NotFound with a
    /// helpful hint.
    #[test]
    fn missing_source_returns_not_found() {
        let proj = TempDir::new().unwrap();
        let opts = CaptureOptions {
            source_path: PathBuf::from("/no/such/file.jsonl"),
            source: CaptureSource::ClaudeCode,
            project_root: proj.path().to_path_buf(),
            scrub_secrets: true,
        };
        let err = capture_from_path(&opts).unwrap_err();
        assert!(matches!(err, SessionError::NotFound(_)));
    }

    /// `--no-scrub` (scrub_secrets = false) writes the original bytes.
    #[test]
    fn no_scrub_preserves_original_bytes() {
        let proj = TempDir::new().unwrap();
        let src_dir = TempDir::new().unwrap();
        let src_path = src_dir.path().join("s.jsonl");
        let key_text = "sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        std::fs::write(
            &src_path,
            format!(
                r#"{{"type":"user","sessionId":"sess-003","timestamp":"2026-05-08T10:00:00Z","cwd":"/x","message":{{"content":"key {key_text}"}}}}
"#
            ),
        )
        .unwrap();
        let opts = CaptureOptions {
            source_path: src_path.clone(),
            source: CaptureSource::ClaudeCode,
            project_root: proj.path().to_path_buf(),
            scrub_secrets: false,
        };
        let outcome = capture_from_path(&opts).unwrap();
        let captured = std::fs::read_to_string(&outcome.captured_path).unwrap();
        assert!(captured.contains(key_text), "no-scrub must preserve bytes");
        assert_eq!(outcome.redaction_count, 0);
    }

    /// `short_hash` is deterministic across calls.
    #[test]
    fn short_hash_is_deterministic() {
        let a = short_hash("session-x", "ts-y");
        let b = short_hash("session-x", "ts-y");
        assert_eq!(a, b);
        assert_eq!(a.len(), 8);
    }

    /// `short_hash` differs for distinct inputs.
    #[test]
    fn short_hash_differs_per_input() {
        let a = short_hash("a", "z");
        let b = short_hash("b", "z");
        assert_ne!(a, b);
    }
}
