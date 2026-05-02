//! `WikiLog` — append-only operation log for the Coral wiki.

use crate::error::{CoralError, Result};
use chrono::{DateTime, Utc};
use regex::Regex;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::sync::OnceLock;

/// Single log entry.
#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub op: String,
    pub summary: String,
}

/// Append-only log of operations on the wiki.
#[derive(Debug, Clone, Default)]
pub struct WikiLog {
    pub entries: Vec<LogEntry>,
}

fn log_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Accept both `Z` and `+HH:MM` / `-HH:MM` offsets so we round-trip our own output
        // (chrono's `to_rfc3339()` emits `+00:00`, not `Z`).
        Regex::new(
            r"^- (\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})) (\w[\w-]*): (.+)$",
        )
        .expect("valid log regex")
    })
}

impl WikiLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses a log file content. Lines that don't match the format are skipped silently.
    /// Header `# Wiki operation log` and frontmatter are skipped.
    pub fn parse(content: &str) -> Result<Self> {
        let re = log_re();
        let mut entries = Vec::new();
        for raw_line in content.lines() {
            let line = raw_line.trim_end();
            if let Some(cap) = re.captures(line) {
                let ts_str = cap.get(1).expect("group 1").as_str();
                let op = cap.get(2).expect("group 2").as_str().to_string();
                let summary = cap.get(3).expect("group 3").as_str().to_string();
                let timestamp = match DateTime::parse_from_rfc3339(ts_str) {
                    Ok(dt) => dt.with_timezone(&Utc),
                    Err(_) => continue,
                };
                entries.push(LogEntry {
                    timestamp,
                    op,
                    summary,
                });
            }
        }
        Ok(Self { entries })
    }

    /// Loads a log from disk. If the file doesn't exist, returns an empty `WikiLog`.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        match fs::read_to_string(path) {
            Ok(content) => Self::parse(&content),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(source) => Err(CoralError::Io {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Appends a new entry with `timestamp = Utc::now()`. Returns the appended entry.
    pub fn append(&mut self, op: impl Into<String>, summary: impl Into<String>) -> &LogEntry {
        let entry = LogEntry {
            timestamp: Utc::now(),
            op: op.into(),
            summary: summary.into(),
        };
        self.entries.push(entry);
        self.entries.last().expect("just pushed")
    }

    /// Serializes to a Markdown document.
    #[allow(clippy::inherent_to_string)]
    pub fn to_string(&self) -> String {
        let mut out = String::new();
        out.push_str("---\n");
        out.push_str("type: log\n");
        out.push_str("---\n\n");
        out.push_str("# Wiki operation log\n\n");
        for entry in &self.entries {
            out.push_str(&format!(
                "- {} {}: {}\n",
                entry.timestamp.to_rfc3339(),
                entry.op,
                entry.summary
            ));
        }
        out
    }

    /// Persists the entire log to disk (overwrite). Atomic via
    /// temp-file + rename, so concurrent readers never see a torn file.
    /// Creates parent dirs.
    ///
    /// For SINGLE-ENTRY appends prefer `WikiLog::append_atomic` — it
    /// uses `O_APPEND` and avoids the load+modify+save lost-update race
    /// entirely.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        crate::atomic::atomic_write_string(path, &self.to_string())
    }

    /// **Atomic single-entry append.** Opens the log file in append mode
    /// and writes a single formatted entry line. Race-free under
    /// concurrent writers — POSIX guarantees `O_APPEND` writes ≤ PIPE_BUF
    /// (typically 4096 bytes) are atomic, and a single log line is well
    /// under that limit.
    ///
    /// This is the recommended path for callers that only need to add
    /// ONE entry and don't care about the full history (e.g. `coral
    /// ingest`, `coral consolidate`, `coral lint --auto-fix`). It avoids
    /// the load+modify+save race that the `WikiLog::load` →
    /// `WikiLog::append` → `WikiLog::save` pattern has under concurrent
    /// writers (documented in `crates/coral-core/tests/concurrency.rs`).
    ///
    /// On the first write, also seeds the YAML frontmatter + heading
    /// block via `OpenOptions::create_new` to guarantee at most one
    /// thread writes the header (any racing writer that loses the
    /// `create_new` race opens in plain append mode and only writes its
    /// entry line, which is correct because the winner has already
    /// produced the header before its entry).
    ///
    /// Creates parent dirs.
    pub fn append_atomic(
        path: impl AsRef<Path>,
        op: impl Into<String>,
        summary: impl Into<String>,
    ) -> Result<LogEntry> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|source| CoralError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
        }

        let entry = LogEntry {
            timestamp: Utc::now(),
            op: op.into(),
            summary: summary.into(),
        };
        let line = format!(
            "- {} {}: {}\n",
            entry.timestamp.to_rfc3339(),
            entry.op,
            entry.summary
        );

        // Try to create-and-seed the file with header + entry. If it
        // already exists, fall through to the plain append path.
        //
        // CRITICAL: even the first-writer path uses `append(true)` to
        // get O_APPEND semantics. Without it, the first writer's cursor
        // sits at offset 0 and a concurrent append-mode writer (one
        // that lost the create_new race) can write at the current EOF
        // — then the first writer's NEXT write (the entry line, after
        // the header) overwrites the append-writer's bytes because the
        // first writer's cursor advanced linearly. With O_APPEND on
        // both sides, every write atomically seeks to EOF first, so
        // bytes are never overwritten regardless of interleaving.
        //
        // This race is benign for the header itself: the winner writes
        // header+entry; concurrent first-time writers lose create_new
        // and only write their entry line. The header is written at
        // most once.
        match fs::OpenOptions::new()
            .append(true)
            .create_new(true)
            .open(path)
        {
            Ok(mut f) => {
                let header = "---\ntype: log\n---\n\n# Wiki operation log\n\n";
                f.write_all(header.as_bytes())
                    .and_then(|_| f.write_all(line.as_bytes()))
                    .map_err(|source| CoralError::Io {
                        path: path.to_path_buf(),
                        source,
                    })?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let mut f = fs::OpenOptions::new()
                    .append(true)
                    .open(path)
                    .map_err(|source| CoralError::Io {
                        path: path.to_path_buf(),
                        source,
                    })?;
                f.write_all(line.as_bytes())
                    .map_err(|source| CoralError::Io {
                        path: path.to_path_buf(),
                        source,
                    })?;
            }
            Err(source) => {
                return Err(CoralError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        }

        Ok(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn log_new_is_empty() {
        let log = WikiLog::new();
        assert!(log.entries.is_empty());
    }

    #[test]
    fn log_append_increments_entries() {
        let mut log = WikiLog::new();
        log.append("bootstrap", "12 pages created");
        log.append("ingest", "3 pages updated");
        log.append("lint", "no warnings");
        assert_eq!(log.entries.len(), 3);
        let last = log.entries.last().unwrap();
        assert_eq!(last.op, "lint");
        assert_eq!(last.summary, "no warnings");
    }

    #[test]
    fn log_serialize_roundtrip() {
        let mut log = WikiLog::new();
        log.append("bootstrap", "12 pages created");
        log.append("ingest", "3 pages updated");

        let serialized = log.to_string();
        let reparsed = WikiLog::parse(&serialized).expect("parse");
        assert_eq!(reparsed.entries.len(), log.entries.len());
        for (a, b) in log.entries.iter().zip(reparsed.entries.iter()) {
            assert_eq!(a.op, b.op);
            assert_eq!(a.summary, b.summary);
            // Timestamps compared as rfc3339 strings — the serialize path truncates / re-encodes.
            assert_eq!(a.timestamp.to_rfc3339(), b.timestamp.to_rfc3339());
        }
    }

    #[test]
    fn log_parse_skips_malformed_lines() {
        let content = "\
---
type: log
---

# Wiki operation log

- 2026-04-30T18:00:00Z bootstrap: 12 pages created
- not a timestamp at all
random nonsense
";
        let log = WikiLog::parse(content).expect("parse");
        assert_eq!(log.entries.len(), 1);
        assert_eq!(log.entries[0].op, "bootstrap");
        assert_eq!(log.entries[0].summary, "12 pages created");
    }

    #[test]
    fn log_load_returns_empty_when_file_missing() {
        let bogus = std::path::PathBuf::from("/definitely/does/not/exist/log-xyz-9999.md");
        let log = WikiLog::load(&bogus).expect("missing file should be Ok empty");
        assert!(log.entries.is_empty());
    }

    #[test]
    fn log_save_creates_parent_dirs() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("a/b/log.md");
        let mut log = WikiLog::new();
        log.append("bootstrap", "12 pages created");
        log.save(&target).expect("save");
        assert!(target.exists());
        let reloaded = WikiLog::load(&target).expect("reload");
        assert_eq!(reloaded.entries.len(), 1);
        assert_eq!(reloaded.entries[0].op, "bootstrap");
    }

    #[test]
    fn append_atomic_seeds_header_and_writes_entry_when_file_missing() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("log.md");
        let entry =
            WikiLog::append_atomic(&target, "bootstrap", "12 pages").expect("first append_atomic");
        assert_eq!(entry.op, "bootstrap");
        assert_eq!(entry.summary, "12 pages");
        let reloaded = WikiLog::load(&target).expect("reload");
        assert_eq!(reloaded.entries.len(), 1);
        assert_eq!(reloaded.entries[0].op, "bootstrap");
        // Header must be present (otherwise WikiLog::parse silently
        // skips lines, and our test would still pass; but check the raw
        // content to pin the format).
        let raw = std::fs::read_to_string(&target).expect("read raw");
        assert!(raw.starts_with("---\ntype: log\n---\n\n# Wiki operation log\n\n"));
    }

    #[test]
    fn append_atomic_appends_to_existing_log_without_dropping_history() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("log.md");
        let mut log = WikiLog::new();
        log.append("seed-1", "first");
        log.append("seed-2", "second");
        log.save(&target).expect("save seed");

        WikiLog::append_atomic(&target, "ingest", "third").expect("append_atomic");

        let reloaded = WikiLog::load(&target).expect("reload");
        assert_eq!(reloaded.entries.len(), 3);
        assert_eq!(reloaded.entries[0].op, "seed-1");
        assert_eq!(reloaded.entries[1].op, "seed-2");
        assert_eq!(reloaded.entries[2].op, "ingest");
        assert_eq!(reloaded.entries[2].summary, "third");
    }

    #[test]
    fn append_atomic_creates_parent_dirs() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("nested/dirs/log.md");
        WikiLog::append_atomic(&target, "init", "wiki initialized").expect("append_atomic");
        assert!(target.exists());
        let reloaded = WikiLog::load(&target).expect("reload");
        assert_eq!(reloaded.entries.len(), 1);
    }

    #[test]
    fn append_atomic_concurrent_preserves_all_entries() {
        // Race-free under N concurrent writers — POSIX O_APPEND guarantees
        // single writes ≤ PIPE_BUF are atomic. This is the test that
        // documents the v0.14 fix to the load+modify+save race tracked in
        // crates/coral-core/tests/concurrency.rs::wikilog_append_concurrent.
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("log.md");
        const N: usize = 20;
        std::thread::scope(|s| {
            for i in 0..N {
                let target = target.clone();
                s.spawn(move || {
                    WikiLog::append_atomic(&target, "test", format!("entry-{i}"))
                        .expect("append_atomic");
                });
            }
        });
        let final_log = WikiLog::load(&target).expect("reload");
        assert_eq!(
            final_log.entries.len(),
            N,
            "atomic-append must preserve all {N} entries under concurrent writers; \
             observed {}",
            final_log.entries.len()
        );
        // Verify every per-thread marker landed.
        let summaries: std::collections::BTreeSet<&str> = final_log
            .entries
            .iter()
            .map(|e| e.summary.as_str())
            .collect();
        for i in 0..N {
            let expected = format!("entry-{i}");
            assert!(
                summaries.contains(expected.as_str()),
                "missing entry: {expected}"
            );
        }
    }
}
