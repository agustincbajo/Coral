//! Claude Code JSONL transcript adapter.
//!
//! Claude Code persists every conversation as line-delimited JSON
//! under `~/.claude/projects/<project-id>/<session-uuid>.jsonl`.
//! Each line is one record. Empirically observed types (v2.1.x):
//!
//! - `queue-operation` — sidecar metadata (enqueue/dequeue events).
//!   Skipped during message extraction.
//! - `last-prompt` — pointer to the most recent prompt's
//!   leaf-uuid. Skipped.
//! - `attachment` — tool-resource attachments (deferred-tool deltas,
//!   image uploads, file reads). Skipped.
//! - `system` — hook/notification events from Claude Code internals
//!   (stop hook summaries, etc.). Skipped.
//! - `user` — user prompt or tool result. The `message.content`
//!   field is either a plain string (initial user prompt) or a list
//!   of content blocks. We extract text blocks verbatim.
//! - `assistant` — assistant turn. `message.content` is a list of
//!   blocks: `text`, `thinking`, `tool_use`, `tool_result`. We
//!   extract `text` (final answer prose) and `tool_use` (tool name +
//!   input) so distillation has both narrative and tool-call
//!   evidence.
//!
//! ## Versioned adapter
//!
//! Per the v0.16-style PRD risk note, Claude Code's JSONL schema
//! WILL drift. This module parses what's there *defensively*: every
//! field is `#[serde(default)]` so a new optional field upstream
//! doesn't break us, and an unknown record `type` is logged at
//! `tracing::debug!` and skipped rather than raising.
//!
//! When a future version of the schema is materially different, fork
//! this module to `claude_code/v2.rs` and gate via Cargo feature.
//! The MVP only needs to read what claude-code 2.1.x writes.

use crate::error::{SessionError, SessionResult};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// One record in a Claude Code transcript JSONL. Every field is
/// optional because the schema mixes record kinds into a single
/// stream — the `type` discriminator selects which fields are
/// meaningful.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeCodeRecord {
    /// Discriminator: `"queue-operation" | "user" | "assistant" |
    /// "attachment" | "system" | "last-prompt" | …`.
    #[serde(default, rename = "type")]
    pub record_type: String,
    /// Per-session UUID (stable across all records of one
    /// conversation).
    #[serde(default, rename = "sessionId")]
    pub session_id: Option<String>,
    /// ISO-8601 timestamp.
    #[serde(default)]
    pub timestamp: Option<String>,
    /// Working directory the agent was invoked from. Used to map a
    /// transcript back to its project root.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Per-message UUID (only present on `user` / `assistant` /
    /// `system`).
    #[serde(default)]
    pub uuid: Option<String>,
    /// User-prompt text (only on `user`-type records when content
    /// is a plain string instead of a content-block list).
    #[serde(default)]
    pub message: Option<serde_json::Value>,
    /// Free-form per-record content (only meaningful for
    /// `queue-operation` records; the body of the prompt + workspace
    /// summary).
    #[serde(default)]
    pub content: Option<serde_json::Value>,
    /// Catch-all for fields we don't model. Lets us deserialize
    /// records with unexpected keys without erroring.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// One extracted message ready for display / distillation. Tool calls
/// and their results are flattened into one stream alongside human
/// turns so the distill prompt sees the full conversation arc.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClaudeCodeMessage {
    /// `"user" | "assistant" | "tool_use" | "tool_result"`.
    pub role: String,
    /// Plain-text body. For `tool_use` rows this is a serialized
    /// `tool: <name>, input: <json>` string; for `tool_result` it's
    /// the truncated stringified content.
    pub text: String,
    /// ISO-8601 timestamp passed through from the source record.
    pub timestamp: Option<String>,
}

/// Result of parsing a Claude Code transcript: extracted messages
/// plus session metadata.
#[derive(Debug, Clone)]
pub struct ParsedTranscript {
    pub session_id: String,
    /// First record's timestamp (or empty if no record carried one).
    pub captured_at: DateTime<Utc>,
    /// Working directory captured from the first record that had
    /// one. May be `None` for transcripts where no `user` /
    /// `assistant` / `system` record exists.
    pub cwd: Option<String>,
    pub messages: Vec<ClaudeCodeMessage>,
}

/// Parses a Claude Code transcript JSONL file from disk into a
/// [`ParsedTranscript`]. Each line MUST be valid JSON; a malformed
/// line raises [`SessionError::ParseError`] with the offending line
/// number (1-indexed) so the user can `sed -n <n>p` to inspect.
pub fn parse_transcript(path: impl AsRef<Path>) -> SessionResult<ParsedTranscript> {
    let path = path.as_ref();
    let content = std::fs::read_to_string(path).map_err(|source| SessionError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_transcript_text(&content, path)
}

/// Parses a Claude Code transcript from in-memory text. Same
/// semantics as [`parse_transcript`] but skips the file read; used
/// by tests with embedded fixtures.
pub fn parse_transcript_text(content: &str, path: &Path) -> SessionResult<ParsedTranscript> {
    let mut session_id = String::new();
    let mut cwd: Option<String> = None;
    let mut first_ts: Option<DateTime<Utc>> = None;
    let mut messages = Vec::new();

    for (lineno, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let record: ClaudeCodeRecord =
            serde_json::from_str(line).map_err(|e| SessionError::ParseError {
                path: path.to_path_buf(),
                line: lineno + 1,
                message: e.to_string(),
            })?;

        // Pin the session_id from the first record that carries one;
        // `sessionId` is repeated on every record but defensive.
        if session_id.is_empty()
            && let Some(s) = &record.session_id
        {
            session_id = s.clone();
        }
        if cwd.is_none()
            && let Some(c) = &record.cwd
        {
            cwd = Some(c.clone());
        }
        if first_ts.is_none()
            && let Some(ts) = &record.timestamp
            && let Ok(parsed) = DateTime::parse_from_rfc3339(ts)
        {
            first_ts = Some(parsed.with_timezone(&Utc));
        }

        match record.record_type.as_str() {
            "user" => {
                if let Some(m) = &record.message {
                    extract_user_message(m, record.timestamp.as_deref(), &mut messages);
                }
            }
            "assistant" => {
                if let Some(m) = &record.message {
                    extract_assistant_message(m, record.timestamp.as_deref(), &mut messages);
                }
            }
            // queue-operation, attachment, system, last-prompt, …
            // — all skipped per the module-level docstring.
            _ => {
                tracing::debug!(record_type = %record.record_type, "skipping non-message record");
            }
        }
    }

    if session_id.is_empty() {
        return Err(SessionError::ParseError {
            path: path.to_path_buf(),
            line: 0,
            message: "no sessionId found in any record".into(),
        });
    }

    Ok(ParsedTranscript {
        session_id,
        captured_at: first_ts.unwrap_or_else(Utc::now),
        cwd,
        messages,
    })
}

/// Walks `<message.content>` for a `user`-type record. Content can be:
///   - a plain string (initial prompt) — emit one `user` message.
///   - a list of content blocks — extract each `text` and
///     `tool_result` block.
fn extract_user_message(
    msg: &serde_json::Value,
    timestamp: Option<&str>,
    out: &mut Vec<ClaudeCodeMessage>,
) {
    let Some(content) = msg.get("content") else {
        return;
    };
    if let Some(s) = content.as_str() {
        out.push(ClaudeCodeMessage {
            role: "user".into(),
            text: s.to_string(),
            timestamp: timestamp.map(str::to_string),
        });
        return;
    }
    let Some(arr) = content.as_array() else {
        return;
    };
    for block in arr {
        let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match block_type {
            "text" => {
                if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                    out.push(ClaudeCodeMessage {
                        role: "user".into(),
                        text: t.to_string(),
                        timestamp: timestamp.map(str::to_string),
                    });
                }
            }
            "tool_result" => {
                let body = block
                    .get("content")
                    .map(stringify_tool_payload)
                    .unwrap_or_default();
                out.push(ClaudeCodeMessage {
                    role: "tool_result".into(),
                    text: body,
                    timestamp: timestamp.map(str::to_string),
                });
            }
            _ => {} // image / unknown — skip
        }
    }
}

/// Same as [`extract_user_message`] but for assistant turns: extracts
/// `text` blocks (final answer prose) and `tool_use` blocks
/// (tool name + input). `thinking` blocks are explicitly **dropped**
/// per v0.20 PRD design Q5: thinking content is high-leverage signal
/// for prompt drift but also high-risk for leaking partial reasoning
/// the user didn't intend to capture.
fn extract_assistant_message(
    msg: &serde_json::Value,
    timestamp: Option<&str>,
    out: &mut Vec<ClaudeCodeMessage>,
) {
    let Some(content) = msg.get("content") else {
        return;
    };
    let Some(arr) = content.as_array() else {
        return;
    };
    for block in arr {
        let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match block_type {
            "text" => {
                if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                    out.push(ClaudeCodeMessage {
                        role: "assistant".into(),
                        text: t.to_string(),
                        timestamp: timestamp.map(str::to_string),
                    });
                }
            }
            "tool_use" => {
                let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let input = block
                    .get("input")
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                out.push(ClaudeCodeMessage {
                    role: "tool_use".into(),
                    text: format!("tool: {name}, input: {input}"),
                    timestamp: timestamp.map(str::to_string),
                });
            }
            "thinking" => {
                // Intentionally skipped (see fn docstring).
            }
            _ => {}
        }
    }
}

/// Helper: stringify a tool_result content payload (which may itself
/// be a nested string, list-of-blocks, or arbitrary JSON). We only
/// need a readable summary for the distill prompt, so flatten to
/// JSON and truncate.
fn stringify_tool_payload(v: &serde_json::Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    let s = v.to_string();
    // Truncate to keep the distill prompt tractable.
    const TRUNCATE_AT: usize = 4_000;
    if s.len() > TRUNCATE_AT {
        format!("{} … <truncated>", &s[..TRUNCATE_AT])
    } else {
        s
    }
}

/// Walks Claude Code's local transcript store under `~/.claude/projects`
/// and picks the most-recently-modified `*.jsonl` whose first
/// `cwd`-bearing record matches `target_cwd`.
///
/// Returns `Ok(None)` if no transcript matches; the CLI surfaces this
/// as a clear "no captured sessions yet for this project" error
/// rather than a panic.
///
/// `home_dir` is taken explicitly so tests can point at a tmpdir.
pub fn find_latest_for_cwd(home_dir: &Path, target_cwd: &Path) -> SessionResult<Option<PathBuf>> {
    let store = home_dir.join(".claude").join("projects");
    if !store.exists() {
        return Ok(None);
    }
    let target_cwd = target_cwd
        .canonicalize()
        .unwrap_or(target_cwd.to_path_buf());
    let mut candidates: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for entry in WalkDir::new(&store)
        .min_depth(2)
        .max_depth(2)
        .into_iter()
        .filter_map(Result::ok)
    {
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        // Cheaply read the first 64 KiB to find a record with `cwd`
        // matching ours. Avoids deserializing huge transcripts when
        // we only care about the prefix.
        let Ok(metadata) = std::fs::metadata(p) else {
            continue;
        };
        let mtime = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
        let prefix_bytes = read_prefix(p, 64 * 1024).unwrap_or_default();
        let mut matched = false;
        for line in prefix_bytes.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(rec) = serde_json::from_str::<ClaudeCodeRecord>(line)
                && let Some(c) = rec.cwd
            {
                let cwd_path = PathBuf::from(&c);
                let canonical = cwd_path.canonicalize().unwrap_or(cwd_path.clone());
                if canonical == target_cwd || cwd_path == target_cwd {
                    matched = true;
                    break;
                }
            }
        }
        if matched {
            candidates.push((mtime, p.to_path_buf()));
        }
    }
    candidates.sort_by_key(|c| std::cmp::Reverse(c.0));
    Ok(candidates.into_iter().next().map(|(_, p)| p))
}

/// Returns the first `n` bytes of `path` as a UTF-8 String (lossy on
/// bad bytes). Used by [`find_latest_for_cwd`] for cheap header
/// inspection.
fn read_prefix(path: &Path, n: usize) -> std::io::Result<String> {
    use std::io::Read as _;
    let mut f = std::fs::File::open(path)?;
    let mut buf = vec![0u8; n];
    let read = f.read(&mut buf)?;
    buf.truncate(read);
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Sample two-line JSONL: queue-operation skipped, user prompt
    /// extracted as plain string.
    #[test]
    fn parse_transcript_text_extracts_plain_user_prompt() {
        let raw = r#"{"type":"queue-operation","operation":"enqueue","timestamp":"2026-05-08T10:00:00Z","sessionId":"abc-123","content":"hello"}
{"type":"user","message":{"role":"user","content":"hola que tal"},"uuid":"u1","timestamp":"2026-05-08T10:00:01Z","cwd":"/tmp/proj","sessionId":"abc-123"}
"#;
        let parsed = parse_transcript_text(raw, &PathBuf::from("test.jsonl")).unwrap();
        assert_eq!(parsed.session_id, "abc-123");
        assert_eq!(parsed.cwd.as_deref(), Some("/tmp/proj"));
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].role, "user");
        assert_eq!(parsed.messages[0].text, "hola que tal");
    }

    /// Assistant content with a `text` block + a `tool_use` block —
    /// both extracted, `thinking` block dropped.
    #[test]
    fn parse_transcript_text_extracts_assistant_text_and_tool_use_skips_thinking() {
        let raw = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"long internal reasoning"},{"type":"text","text":"the answer is 42"},{"type":"tool_use","id":"tu_1","name":"Read","input":{"file_path":"/x.md"}}]},"sessionId":"abc-123","timestamp":"2026-05-08T10:00:02Z"}
"#;
        let parsed = parse_transcript_text(raw, &PathBuf::from("test.jsonl")).unwrap();
        assert_eq!(
            parsed.messages.len(),
            2,
            "expected text+tool_use, no thinking"
        );
        assert_eq!(parsed.messages[0].role, "assistant");
        assert_eq!(parsed.messages[0].text, "the answer is 42");
        assert_eq!(parsed.messages[1].role, "tool_use");
        assert!(parsed.messages[1].text.contains("Read"));
        assert!(parsed.messages[1].text.contains("/x.md"));
    }

    /// User message with a content list containing a `tool_result`
    /// block — extracted as `tool_result`.
    #[test]
    fn parse_transcript_text_extracts_tool_result_in_user_block() {
        let raw = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu_1","content":"file contents here"}]},"sessionId":"abc-123","timestamp":"2026-05-08T10:00:03Z"}
"#;
        let parsed = parse_transcript_text(raw, &PathBuf::from("test.jsonl")).unwrap();
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].role, "tool_result");
        assert!(parsed.messages[0].text.contains("file contents"));
    }

    /// Malformed JSONL line raises a ParseError with the right line
    /// number.
    #[test]
    fn parse_transcript_text_reports_line_number_on_malformed_line() {
        let raw = r#"{"type":"user","sessionId":"abc","message":{"content":"ok"}}
this is not json
"#;
        let err = parse_transcript_text(raw, &PathBuf::from("test.jsonl")).unwrap_err();
        match err {
            SessionError::ParseError { line, .. } => assert_eq!(line, 2),
            other => panic!("expected ParseError, got {other:?}"),
        }
    }

    /// Transcript with no `sessionId` anywhere — surfaces a clear
    /// error rather than silently producing a session_id of "".
    #[test]
    fn parse_transcript_text_errors_on_missing_session_id() {
        let raw = r#"{"type":"queue-operation","operation":"enqueue","content":"ok"}
"#;
        let err = parse_transcript_text(raw, &PathBuf::from("test.jsonl")).unwrap_err();
        assert!(matches!(err, SessionError::ParseError { .. }));
    }

    /// `find_latest_for_cwd` returns None when the store doesn't
    /// exist (clean machine; user has never used Claude Code).
    #[test]
    fn find_latest_for_cwd_none_on_missing_store() {
        let dir = TempDir::new().unwrap();
        let result = find_latest_for_cwd(dir.path(), &PathBuf::from("/tmp/whatever")).unwrap();
        assert!(result.is_none());
    }

    /// `find_latest_for_cwd` finds the matching JSONL among multiple
    /// candidates, picking the most-recently-modified one.
    #[test]
    fn find_latest_for_cwd_picks_most_recent_matching_jsonl() {
        let home = TempDir::new().unwrap();
        let proj_dir = home.path().join(".claude").join("projects").join("p1");
        std::fs::create_dir_all(&proj_dir).unwrap();
        let target_cwd = TempDir::new().unwrap();
        // Use serde_json to escape the path string into a valid JSON
        // literal. On Windows the raw `\` separators would otherwise
        // be invalid `\` escapes in the JSON document and the entire
        // record would fail to deserialize.
        let target_cwd_json = serde_json::to_string(&target_cwd.path().display().to_string())
            .expect("cwd path must serialize to JSON");

        // Two transcripts: one for the target cwd, one for a different one.
        let match_path = proj_dir.join("match.jsonl");
        std::fs::write(
            &match_path,
            format!(
                "{{\"type\":\"user\",\"sessionId\":\"s1\",\"cwd\":{target_cwd_json},\"message\":{{\"content\":\"x\"}}}}\n"
            ),
        )
        .unwrap();
        let other_path = proj_dir.join("other.jsonl");
        std::fs::write(
            &other_path,
            r#"{"type":"user","sessionId":"s2","cwd":"/somewhere/else","message":{"content":"y"}}
"#,
        )
        .unwrap();

        let result = find_latest_for_cwd(home.path(), target_cwd.path()).unwrap();
        let chosen = result.expect("expected match");
        assert_eq!(chosen, match_path);
    }
}
