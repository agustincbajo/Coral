//! `coral session list` — render the captured-session index as a
//! Markdown table or JSON array.

use crate::capture::{IndexEntry, read_index};
use crate::error::SessionResult;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ListFormat {
    Markdown,
    Json,
}

/// One row in the rendered list. Mirrors [`IndexEntry`] but adds a
/// short id (first 8 chars of session_id) so users have a stable
/// short handle to pass to `distill` / `forget`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    pub session_id: String,
    pub short_id: String,
    pub source: String,
    pub captured_at: String,
    pub message_count: usize,
    pub redaction_count: usize,
    pub distilled: bool,
}

impl From<&IndexEntry> for SessionEntry {
    fn from(e: &IndexEntry) -> Self {
        let short = e.session_id.chars().take(8).collect::<String>();
        Self {
            session_id: e.session_id.clone(),
            short_id: short,
            source: e.source.as_str().to_string(),
            captured_at: e.captured_at.to_rfc3339(),
            message_count: e.message_count,
            redaction_count: e.redaction_count,
            distilled: e.distilled,
        }
    }
}

/// Lists the captured sessions under `<project_root>/.coral/sessions/`.
/// Returns the rendered string ready to print. Caller decides what
/// to do on stdout.
pub fn list_sessions(project_root: &Path, format: ListFormat) -> SessionResult<String> {
    let index_path = project_root
        .join(".coral")
        .join("sessions")
        .join("index.json");
    let index = read_index(&index_path)?;
    let entries: Vec<SessionEntry> = index.sessions.iter().map(SessionEntry::from).collect();
    match format {
        ListFormat::Json => {
            Ok(serde_json::to_string_pretty(&entries).unwrap_or_else(|_| "[]".into()))
        }
        ListFormat::Markdown => Ok(render_markdown(&entries)),
    }
}

/// Renders a Markdown table. Empty index renders a clear "no
/// sessions yet" message instead of a header-only table.
fn render_markdown(entries: &[SessionEntry]) -> String {
    if entries.is_empty() {
        return "_No captured sessions yet._ Run `coral session capture --from claude-code` to record one.\n".into();
    }
    let mut out = String::new();
    out.push_str("| id | source | captured_at | messages | redactions | distilled |\n");
    out.push_str("|----|--------|-------------|----------|------------|-----------|\n");
    for e in entries {
        out.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {} |\n",
            e.short_id,
            e.source,
            e.captured_at,
            e.message_count,
            e.redaction_count,
            if e.distilled { "yes" } else { "no" }
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{CaptureSource, IndexEntry, SessionIndex, write_index};
    use chrono::TimeZone;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn seed_index(root: &Path, entries: Vec<IndexEntry>) {
        let dir = root.join(".coral/sessions");
        std::fs::create_dir_all(&dir).unwrap();
        let idx = SessionIndex { sessions: entries };
        write_index(&dir.join("index.json"), &idx).unwrap();
    }

    fn mk_entry(session_id: &str, captured_at: chrono::DateTime<chrono::Utc>) -> IndexEntry {
        IndexEntry {
            session_id: session_id.into(),
            source: CaptureSource::ClaudeCode,
            captured_at,
            captured_path: PathBuf::from("dummy.jsonl"),
            message_count: 5,
            redaction_count: 1,
            distilled: false,
            distilled_outputs: Vec::new(),
        }
    }

    #[test]
    fn list_empty_renders_friendly_message() {
        let dir = TempDir::new().unwrap();
        let out = list_sessions(dir.path(), ListFormat::Markdown).unwrap();
        assert!(out.contains("No captured sessions"), "got: {out}");
    }

    #[test]
    fn list_markdown_includes_table_header_and_rows() {
        let dir = TempDir::new().unwrap();
        let ts = chrono::Utc.with_ymd_and_hms(2026, 5, 8, 10, 0, 0).unwrap();
        seed_index(dir.path(), vec![mk_entry("abcdef0123456789", ts)]);
        let out = list_sessions(dir.path(), ListFormat::Markdown).unwrap();
        assert!(out.contains("| id |"), "missing header: {out}");
        assert!(out.contains("`abcdef01`"), "missing short id: {out}");
        assert!(out.contains("claude-code"), "missing source: {out}");
    }

    #[test]
    fn list_json_returns_array_with_short_id() {
        let dir = TempDir::new().unwrap();
        let ts = chrono::Utc.with_ymd_and_hms(2026, 5, 8, 10, 0, 0).unwrap();
        seed_index(dir.path(), vec![mk_entry("12345678ffff", ts)]);
        let out = list_sessions(dir.path(), ListFormat::Json).unwrap();
        let parsed: Vec<SessionEntry> = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].short_id, "12345678");
    }
}
