//! `coral session forget <id>` — delete a captured session and any
//! distilled artifacts derived from it.
//!
//! Removes:
//!   1. `.coral/sessions/<file>.jsonl` (the raw transcript)
//!   2. `.coral/sessions/distilled/<id>.md` (if present)
//!   3. The matching entry from `.coral/sessions/index.json`
//!
//! All three steps run under [`coral_core::atomic::with_exclusive_lock`]
//! against the index file so a concurrent capture / list cannot
//! observe a torn state. Returns `Ok(())` only after every step
//! succeeded; partial cleanups raise [`crate::error::SessionError::Io`]
//! and the index entry is left in place so a follow-up `forget`
//! can retry.

use crate::capture::{IndexEntry, SessionIndex, read_index, write_index};
use crate::error::{SessionError, SessionResult};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ForgetOptions {
    pub project_root: PathBuf,
    /// Either the full UUID or any unique prefix (≥4 chars) of one.
    pub session_id: String,
}

/// Removes the session record. Matching is by full session_id OR
/// short prefix (first 4+ chars). A prefix that matches more than
/// one session returns `InvalidInput` so the user picks
/// unambiguously.
pub fn forget_session(opts: &ForgetOptions) -> SessionResult<()> {
    if opts.session_id.len() < 4 {
        return Err(SessionError::InvalidInput(
            "session id must be at least 4 chars".into(),
        ));
    }
    let sessions_dir = opts.project_root.join(".coral").join("sessions");
    let index_path = sessions_dir.join("index.json");
    coral_core::atomic::with_exclusive_lock(&index_path, || {
        let mut index = read_index(&index_path).unwrap_or_default();

        // Find matching entries by id or short prefix.
        let matches: Vec<usize> = index
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, e)| matches_id(&e.session_id, &opts.session_id))
            .map(|(i, _)| i)
            .collect();
        if matches.is_empty() {
            return Err(coral_core::error::CoralError::Walk(format!(
                "session not found: {}",
                opts.session_id
            )));
        }
        if matches.len() > 1 {
            return Err(coral_core::error::CoralError::Walk(format!(
                "session id '{}' matches {} sessions; use a longer prefix or full id",
                opts.session_id,
                matches.len()
            )));
        }
        let idx = matches[0];
        let entry: IndexEntry = index.sessions[idx].clone();

        // Delete raw transcript (best-effort: if the file is already
        // gone, silently continue — the index is the source of
        // truth, not the file).
        let raw_path = entry.captured_path.clone();
        if raw_path.exists() {
            std::fs::remove_file(&raw_path).map_err(|source| {
                coral_core::error::CoralError::Io {
                    path: raw_path.clone(),
                    source,
                }
            })?;
        }
        // Delete distilled output if present.
        let distilled_path = sessions_dir
            .join("distilled")
            .join(format!("{}.md", entry.session_id));
        if distilled_path.exists() {
            std::fs::remove_file(&distilled_path).map_err(|source| {
                coral_core::error::CoralError::Io {
                    path: distilled_path.clone(),
                    source,
                }
            })?;
        }

        // Drop the entry and persist the index.
        index.sessions.remove(idx);
        write_atomic_index(&index_path, &index)
    })
    .map_err(|err| match err {
        coral_core::error::CoralError::Io { path, source } => SessionError::Io { path, source },
        coral_core::error::CoralError::Walk(msg) if msg.contains("session not found") => {
            SessionError::NotFound(opts.session_id.clone())
        }
        coral_core::error::CoralError::Walk(msg) => SessionError::InvalidInput(msg),
        other => SessionError::Io {
            path: index_path.clone(),
            source: std::io::Error::other(format!("{other}")),
        },
    })
}

/// Returns true when `candidate` matches the id `target` or a unique
/// prefix of it. We accept full id OR short prefix (4+ chars) so
/// users can type the short form they see in `coral session list`.
fn matches_id(candidate: &str, target: &str) -> bool {
    candidate == target || candidate.starts_with(target)
}

/// Thin wrapper around [`crate::capture::write_index`] kept as an
/// internal alias so the public surface stays narrow.
fn write_atomic_index(path: &Path, index: &SessionIndex) -> coral_core::error::Result<()> {
    write_index(path, index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{CaptureSource, IndexEntry, SessionIndex, write_index};
    use chrono::TimeZone;
    use tempfile::TempDir;

    fn seed(root: &Path, entry: IndexEntry, raw_present: bool, distilled_present: bool) {
        let dir = root.join(".coral/sessions");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(dir.join("distilled")).unwrap();
        if raw_present {
            std::fs::write(&entry.captured_path, "raw").unwrap();
        }
        if distilled_present {
            std::fs::write(
                dir.join("distilled")
                    .join(format!("{}.md", entry.session_id)),
                "distilled",
            )
            .unwrap();
        }
        let idx = SessionIndex {
            sessions: vec![entry],
        };
        write_index(&dir.join("index.json"), &idx).unwrap();
    }

    fn mk_entry(root: &Path, id: &str) -> IndexEntry {
        IndexEntry {
            session_id: id.into(),
            source: CaptureSource::ClaudeCode,
            captured_at: chrono::Utc.with_ymd_and_hms(2026, 5, 8, 10, 0, 0).unwrap(),
            captured_path: root.join(".coral/sessions").join(format!("{id}.jsonl")),
            message_count: 1,
            redaction_count: 0,
            distilled: false,
        }
    }

    #[test]
    fn forget_removes_raw_distilled_and_index_entry() {
        let dir = TempDir::new().unwrap();
        let entry = mk_entry(dir.path(), "abcdef0123456789");
        seed(dir.path(), entry.clone(), true, true);
        let opts = ForgetOptions {
            project_root: dir.path().to_path_buf(),
            session_id: "abcdef01".into(),
        };
        forget_session(&opts).unwrap();
        assert!(!entry.captured_path.exists(), "raw should be gone");
        assert!(
            !dir.path()
                .join(".coral/sessions/distilled/abcdef0123456789.md")
                .exists(),
            "distilled should be gone"
        );
        let idx = read_index(&dir.path().join(".coral/sessions/index.json")).unwrap();
        assert!(idx.sessions.is_empty());
    }

    #[test]
    fn forget_unknown_id_returns_not_found() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".coral/sessions")).unwrap();
        let opts = ForgetOptions {
            project_root: dir.path().to_path_buf(),
            session_id: "abcdfake".into(),
        };
        let err = forget_session(&opts).unwrap_err();
        assert!(matches!(err, SessionError::NotFound(_)));
    }

    #[test]
    fn forget_short_id_too_short_rejects() {
        let dir = TempDir::new().unwrap();
        let opts = ForgetOptions {
            project_root: dir.path().to_path_buf(),
            session_id: "ab".into(),
        };
        let err = forget_session(&opts).unwrap_err();
        assert!(matches!(err, SessionError::InvalidInput(_)));
    }

    #[test]
    fn forget_ambiguous_prefix_rejects() {
        let dir = TempDir::new().unwrap();
        // Two entries sharing prefix "abcd".
        let dir_p = dir.path();
        let sessions_dir = dir_p.join(".coral/sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::create_dir_all(sessions_dir.join("distilled")).unwrap();
        let e1 = mk_entry(dir_p, "abcd1111111");
        let e2 = mk_entry(dir_p, "abcd2222222");
        let idx = SessionIndex {
            sessions: vec![e1, e2],
        };
        write_index(&sessions_dir.join("index.json"), &idx).unwrap();

        let opts = ForgetOptions {
            project_root: dir_p.to_path_buf(),
            session_id: "abcd".into(),
        };
        let err = forget_session(&opts).unwrap_err();
        assert!(
            matches!(&err, SessionError::InvalidInput(s) if s.contains("matches 2")),
            "got: {err:?}"
        );
    }
}
