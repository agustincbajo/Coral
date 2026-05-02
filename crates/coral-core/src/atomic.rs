//! Atomic file writes via the temp-file + rename pattern.
//!
//! `std::fs::write` opens with `O_TRUNC | O_CREAT`, truncates to zero
//! bytes immediately, and only then starts writing. A concurrent
//! reader hitting the file between the truncate and the writes sees a
//! partial (or empty) file — which corrupts downstream parsing.
//!
//! `atomic_write_string` writes to a sibling `*.tmp.<pid>` file first,
//! then `rename`s it onto the target path. POSIX `rename` is atomic
//! within a single filesystem: readers see either the OLD contents or
//! the NEW contents, never a half-written state. Windows `MoveFileEx`
//! with `MOVEFILE_REPLACE_EXISTING` provides the same guarantee.
//!
//! This does NOT solve the lost-update race for load+modify+save
//! patterns — two concurrent writers can both produce a complete
//! `*.tmp` file and the second `rename` clobbers the first writer's
//! data. For that, callers need true file locking (a v0.15+ design
//! item that requires a new dep).

use crate::error::{CoralError, Result};
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Process-unique counter for temp filename uniqueness across threads
/// within the same process (PID alone is not enough — every thread in
/// a process shares the same PID, so two threads writing the same
/// target would collide on `<filename>.tmp.<pid>`).
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Atomically replaces `path` with `content`. Creates parent dirs if
/// they don't exist. Uses a sibling
/// `<filename>.tmp.<pid>.<thread-counter>` temp file + rename to avoid
/// exposing readers to a torn write.
///
/// On the temp filename: keeping the temp file in the same directory
/// as the target ensures `rename` stays within one filesystem
/// (cross-fs rename returns `EXDEV`). The `<pid>.<counter>` suffix
/// guarantees uniqueness across all writers — PID alone is NOT
/// enough because every thread in a process shares the same PID,
/// so two threads writing the same target would collide.
pub fn atomic_write_string(path: impl AsRef<Path>, content: &str) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|source| CoralError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
    }

    let pid = std::process::id();
    let counter = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_filename = match path.file_name() {
        Some(name) => {
            let mut s = name.to_os_string();
            s.push(format!(".tmp.{pid}.{counter}"));
            s
        }
        None => {
            // path has no filename component — extremely unusual, fall
            // back to plain fs::write rather than fail.
            return fs::write(path, content).map_err(|source| CoralError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let tmp_path = path.with_file_name(&tmp_filename);

    // Scope: write to tmp + flush + close, then rename. Drop the file
    // handle BEFORE renaming so Windows lets us complete the move
    // (POSIX doesn't care, but the explicit drop is portable).
    {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp_path)
            .map_err(|source| CoralError::Io {
                path: tmp_path.clone(),
                source,
            })?;
        f.write_all(content.as_bytes())
            .and_then(|_| f.flush())
            .map_err(|source| CoralError::Io {
                path: tmp_path.clone(),
                source,
            })?;
    }

    fs::rename(&tmp_path, path).map_err(|source| {
        // Best-effort cleanup so we don't leave a dangling tmp file
        // behind when the rename fails (rare but possible).
        let _ = fs::remove_file(&tmp_path);
        CoralError::Io {
            path: path.to_path_buf(),
            source,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn atomic_write_creates_target_with_content() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("file.txt");
        atomic_write_string(&target, "hello world").expect("write");
        assert_eq!(fs::read_to_string(&target).expect("read"), "hello world");
    }

    #[test]
    fn atomic_write_creates_parent_dirs() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("a/b/c/file.txt");
        atomic_write_string(&target, "nested").expect("write");
        assert!(target.exists());
        assert_eq!(fs::read_to_string(&target).expect("read"), "nested");
    }

    #[test]
    fn atomic_write_replaces_existing_content() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("file.txt");
        atomic_write_string(&target, "old").expect("seed");
        atomic_write_string(&target, "new").expect("replace");
        assert_eq!(fs::read_to_string(&target).expect("read"), "new");
    }

    #[test]
    fn atomic_write_leaves_no_tmp_files_on_success() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("file.txt");
        atomic_write_string(&target, "content").expect("write");
        let entries: Vec<_> = fs::read_dir(dir.path())
            .expect("readdir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name())
            .collect();
        assert_eq!(entries.len(), 1, "expected exactly target file, got {entries:?}");
    }

    /// Stress test: 50 threads each replace the target with their own
    /// payload. Every reader (including the final assert) must see a
    /// COMPLETE payload from one writer — never an empty file, never
    /// a partial mix. Acceptable failure modes are limited to "the
    /// observed final content is one of the 50 candidates".
    ///
    /// This is the property `fs::write` violates and `atomic_write_string`
    /// is supposed to provide.
    #[test]
    fn atomic_write_concurrent_readers_never_see_torn_writes() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("contended.txt");
        // Seed with a known starting payload so readers hitting before
        // the first writer wins still see something parseable.
        atomic_write_string(&target, "seed").expect("seed");

        const N: usize = 50;
        // Build a long-ish payload so a torn write would be observable
        // (a 1-byte file is hard to tear).
        let payloads: Vec<String> = (0..N)
            .map(|i| format!("payload-{i:03}-{}", "x".repeat(2048)))
            .collect();
        let read_observations = std::sync::Mutex::new(Vec::<String>::new());

        std::thread::scope(|s| {
            // N writers
            for payload in &payloads {
                let target = target.clone();
                let payload = payload.clone();
                s.spawn(move || {
                    atomic_write_string(&target, &payload).expect("atomic_write");
                });
            }
            // N readers — each takes one snapshot during the write storm.
            for _ in 0..N {
                let target = target.clone();
                let observations = &read_observations;
                s.spawn(move || {
                    let content = fs::read_to_string(&target).expect("read");
                    observations.lock().unwrap().push(content);
                });
            }
        });

        // Every observation must be either "seed" or one of the
        // payloads — NEVER a torn / partial / empty file.
        let observations = read_observations.into_inner().unwrap();
        let valid: std::collections::BTreeSet<&str> = std::iter::once("seed")
            .chain(payloads.iter().map(String::as_str))
            .collect();
        for obs in &observations {
            assert!(
                valid.contains(obs.as_str()),
                "reader observed a torn/partial write: len={}, first 64 bytes: {:?}",
                obs.len(),
                obs.chars().take(64).collect::<String>()
            );
        }
        // Final state must also be one of the payloads (NOT seed,
        // since at least one writer ran after the seed write).
        let final_content = fs::read_to_string(&target).expect("read final");
        assert!(
            payloads.contains(&final_content),
            "final content is not one of the {N} payloads"
        );
    }
}
