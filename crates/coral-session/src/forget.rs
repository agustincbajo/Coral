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
        // v0.20.1 cycle-4 audit H1: delete every `.md` recorded in
        // `entry.distilled_outputs` from `.coral/sessions/distilled/`
        // and from `.wiki/synthesis/` (where `distill --apply`
        // mirrors them). Pre-fix `forget` constructed the path from
        // `<session_id>.md` but distill writes by `<finding.slug>.md`
        // — so distilled outputs got orphaned on every forget.
        //
        // BC: if `distilled_outputs` is empty BUT `distilled` is true,
        // the session predates v0.20.1's tracking. We can't safely
        // sweep — slug-named files might belong to other sessions —
        // so we warn and leave the cleanup to the user.
        let distilled_dir = sessions_dir.join("distilled");
        let wiki_synthesis_dir = opts.project_root.join(".wiki").join("synthesis");
        if entry.distilled_outputs.is_empty() && entry.distilled {
            tracing::warn!(
                session_id = %entry.session_id,
                "session predates v0.20.1 distill-tracking; sweep \
                 .coral/sessions/distilled/ and .wiki/synthesis/ manually \
                 if you want orphan cleanup"
            );
        }
        for basename in &entry.distilled_outputs {
            // Defense-in-depth: refuse anything that looks like a
            // path-traversal segment. Distill only writes
            // `<safe_slug>.md` so this should never trigger, but a
            // tampered `index.json` could try to point us at
            // `../../etc/passwd`. We just skip with a warn.
            if basename.contains('/')
                || basename.contains('\\')
                || basename.contains("..")
                || basename.starts_with('.')
            {
                tracing::warn!(
                    basename = %basename,
                    "skipping suspicious distilled output filename"
                );
                continue;
            }
            for parent in [&distilled_dir, &wiki_synthesis_dir] {
                let p = parent.join(basename);
                if p.exists() {
                    std::fs::remove_file(&p).map_err(|source| {
                        coral_core::error::CoralError::Io {
                            path: p.clone(),
                            source,
                        }
                    })?;
                }
            }
        }
        // Legacy path: also try `<session_id>.md` (matches what
        // pre-v0.20.1 distill would have written had the path been
        // wired correctly — this is a no-op in practice but keeps
        // `forget` idempotent for any user who hand-rolled that name).
        let legacy = distilled_dir.join(format!("{}.md", entry.session_id));
        if legacy.exists() {
            std::fs::remove_file(&legacy).map_err(|source| coral_core::error::CoralError::Io {
                path: legacy.clone(),
                source,
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
            distilled_outputs: Vec::new(),
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

    /// v0.20.1 cycle-4 audit H1: the session forget path used to
    /// look for `.coral/sessions/distilled/<session-id>.md` but
    /// distill writes by `<finding.slug>.md`. Outputs got orphaned
    /// on every forget. This test wires the full distill -> forget
    /// cycle (with mock runner) and asserts every artifact is
    /// removed.
    #[test]
    fn forget_removes_slug_named_distilled_outputs_after_real_distill() {
        use crate::distill::{DistillOptions, distill_session};
        use coral_runner::MockRunner;

        let dir = TempDir::new().unwrap();
        let root = dir.path();
        // Seed an index with a single session entry plus a captured
        // transcript on disk.
        let session_id = "deadbeef000111";
        let captured_path = root
            .join(".coral/sessions")
            .join(format!("{session_id}.jsonl"));
        std::fs::create_dir_all(captured_path.parent().unwrap()).unwrap();
        std::fs::create_dir_all(root.join(".wiki/synthesis")).unwrap();
        let transcript = "{\"type\":\"summary\",\"summary\":\"x\",\"leafUuid\":\"u\"}\n\
{\"type\":\"user\",\"sessionId\":\"deadbeef000111\",\"timestamp\":\"2026-05-08T10:00:00Z\",\
\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n\
{\"type\":\"assistant\",\"sessionId\":\"deadbeef000111\",\"timestamp\":\"2026-05-08T10:00:01Z\",\
\"message\":{\"role\":\"assistant\",\"content\":\"hi\"}}\n";
        std::fs::write(&captured_path, transcript).unwrap();
        let entry = IndexEntry {
            session_id: session_id.into(),
            source: CaptureSource::ClaudeCode,
            captured_at: chrono::Utc.with_ymd_and_hms(2026, 5, 8, 10, 0, 0).unwrap(),
            captured_path: captured_path.clone(),
            message_count: 2,
            redaction_count: 0,
            distilled: false,
            distilled_outputs: Vec::new(),
        };
        let idx = SessionIndex {
            sessions: vec![entry],
        };
        write_index(&root.join(".coral/sessions/index.json"), &idx).unwrap();

        // Distill with a mock runner that emits two findings whose
        // slugs differ from the session_id. The runner output shape
        // is the YAML envelope `findings: [...]` that
        // `parse_findings` expects.
        let runner = MockRunner::new();
        runner.push_ok(
            "findings:\n  - slug: finding-alpha\n    title: Alpha thing\n    body: |\n      A real-feeling body that satisfies the body trim check for finding alpha.\n    sources: []\n  - slug: finding-beta\n    title: Beta thing\n    body: |\n      A real-feeling body that satisfies the body trim check for finding beta.\n    sources: []\n",
        );
        let opts = DistillOptions {
            project_root: root.to_path_buf(),
            session_id: session_id.into(),
            apply: true,
            model: None,
        };
        let outcome = distill_session(&opts, &runner, "mock").expect("distill ok");
        assert_eq!(outcome.findings.len(), 2);

        // Both files exist under both locations.
        assert!(
            root.join(".coral/sessions/distilled/finding-alpha.md")
                .exists()
        );
        assert!(
            root.join(".coral/sessions/distilled/finding-beta.md")
                .exists()
        );
        assert!(root.join(".wiki/synthesis/finding-alpha.md").exists());
        assert!(root.join(".wiki/synthesis/finding-beta.md").exists());

        // Now forget — every artifact must be gone.
        let forget_opts = ForgetOptions {
            project_root: root.to_path_buf(),
            session_id: session_id.into(),
        };
        forget_session(&forget_opts).unwrap();
        assert!(!captured_path.exists(), "raw transcript should be gone");
        assert!(
            !root
                .join(".coral/sessions/distilled/finding-alpha.md")
                .exists(),
            "distilled finding-alpha.md should be gone"
        );
        assert!(
            !root
                .join(".coral/sessions/distilled/finding-beta.md")
                .exists(),
            "distilled finding-beta.md should be gone"
        );
        assert!(
            !root.join(".wiki/synthesis/finding-alpha.md").exists(),
            ".wiki/synthesis/finding-alpha.md should be gone"
        );
        assert!(
            !root.join(".wiki/synthesis/finding-beta.md").exists(),
            ".wiki/synthesis/finding-beta.md should be gone"
        );
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
