//! Atomic file writes + cross-process file locking.
//!
//! `atomic_write_string` writes to a sibling `*.tmp.<pid>.<counter>`
//! file first, then `rename`s it onto the target path. POSIX `rename`
//! is atomic within a single filesystem: readers see either the OLD
//! contents or the NEW contents, never a half-written state. Windows
//! `MoveFileEx` with `MOVEFILE_REPLACE_EXISTING` provides the same
//! guarantee.
//!
//! `with_exclusive_lock` (v0.15) wraps a closure in an `flock(2)`
//! exclusive advisory lock so the load+modify+save pattern is
//! actually safe under concurrent writers — both threads within one
//! process AND across multiple cooperating processes (e.g. two
//! `coral ingest` invocations against the same `.wiki/`). Closes the
//! lost-update race documented in v0.14.

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
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| CoralError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
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

/// Atomic-write counterpart to [`atomic_write_string`] for binary payloads.
///
/// Same tmp+rename pattern, but accepts `&[u8]` so callers persisting
/// bincoded / msgpacked / otherwise binary content don't have to
/// round-trip through `String` (which would also UTF-8-validate and
/// reject non-UTF-8 bytes outright).
pub fn atomic_write_bytes(path: impl AsRef<Path>, content: &[u8]) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| CoralError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
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
            return fs::write(path, content).map_err(|source| CoralError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let tmp_path = path.with_file_name(&tmp_filename);

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
        f.write_all(content)
            .and_then(|_| f.flush())
            .and_then(|_| f.sync_all())
            .map_err(|source| CoralError::Io {
                path: tmp_path.clone(),
                source,
            })?;
    }

    fs::rename(&tmp_path, path).map_err(|source| {
        let _ = fs::remove_file(&tmp_path);
        CoralError::Io {
            path: path.to_path_buf(),
            source,
        }
    })
}

/// Runs `f` while holding an exclusive `flock(2)` advisory lock on
/// `<path>.lock`. Race-free under concurrent writers — both threads
/// within one process AND cooperating processes that go through this
/// helper. Blocks until the lock is acquired.
///
/// **Use this for any load+modify+save sequence on shared state.** The
/// canonical pattern:
///
/// ```rust,ignore
/// coral_core::atomic::with_exclusive_lock(&idx_path, || {
///     let content = std::fs::read_to_string(&idx_path)?;
///     let mut idx = WikiIndex::parse(&content)?;
///     idx.upsert(entry);
///     atomic_write_string(&idx_path, &idx.to_string()?)
/// })?
/// ```
///
/// Why a sibling `.lock` file (instead of locking the target itself):
/// `atomic_write_string` `rename`s a fresh inode onto the target path,
/// so the target's inode changes between writes. `flock` attaches to
/// inodes, not paths — locking the target file directly would let two
/// writers each end up with a lock on a DIFFERENT (stale) inode.
/// Locking a sibling `.lock` file that no one ever renames keeps every
/// participant attached to the same inode.
///
/// The lock file is created on first use (with `OpenOptions::create`)
/// and is NEVER removed — by design. Removing it would create a TOCTOU
/// where a process between "open existing lock file" and "lock it"
/// could see the file disappear and re-create it, ending up locking a
/// fresh inode that another process isn't holding. Leaving the file in
/// place (it's empty, ~0 bytes) is the conventional pattern. Add
/// `*.lock` to `.gitignore` if it shows up in `git status`.
///
/// v0.19.5 audit M7 considered cleanup but rejected it after the
/// `cross_process_lock_serializes_n_subprocess_increments` test
/// surfaced lost-update behavior under contention. The artefact is
/// intentional.
///
/// Errors from `f` propagate as-is; lock release is best-effort but
/// happens automatically on `File` drop even if explicit unlock fails.
pub fn with_exclusive_lock<F, T>(path: impl AsRef<Path>, f: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    let path = path.as_ref();
    let lock_path = lock_file_path(path);

    if let Some(parent) = lock_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| CoralError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let lock_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|source| CoralError::Io {
            path: lock_path.clone(),
            source,
        })?;

    // Note on UFCS: Rust 1.89 added `File::lock_exclusive` / `unlock`
    // to stdlib. Calling the methods via the inherent path would
    // resolve to the stdlib impl, which would push our MSRV to 1.89.
    // Using fully-qualified syntax `<File as fs4 trait>::method`
    // pins the call to the fs4 trait, which works on every Rust
    // version since 1.85 (our MSRV).
    use fs4::fs_std::FileExt as Fs4Ext;
    Fs4Ext::lock_exclusive(&lock_file).map_err(|source| CoralError::Io {
        path: lock_path.clone(),
        source,
    })?;

    // Run the user closure under the lock. If it panics, the File
    // drop releases the lock anyway — we don't poison anything.
    let result = f();

    // Best-effort explicit unlock. The Drop impl on File will also
    // release any held flock, so an error here is informational
    // only — return the user's result regardless.
    let _ = Fs4Ext::unlock(&lock_file);

    result
}

/// Returns the conventional lock-file path for a target file.
/// Pure, no I/O.
fn lock_file_path(path: &Path) -> std::path::PathBuf {
    let mut s = path
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    s.push(".lock");
    path.with_file_name(s)
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
        assert_eq!(
            entries.len(),
            1,
            "expected exactly target file, got {entries:?}"
        );
    }

    // ---- with_exclusive_lock --------------------------------------

    #[test]
    fn lock_file_path_is_sibling_dot_lock() {
        assert_eq!(
            lock_file_path(Path::new("/x/y/z/index.md")),
            Path::new("/x/y/z/index.md.lock")
        );
        assert_eq!(
            lock_file_path(Path::new("relative/log.md")),
            Path::new("relative/log.md.lock")
        );
    }

    #[test]
    fn with_exclusive_lock_runs_closure_and_returns_value() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("target.txt");
        let value: u32 = with_exclusive_lock(&target, || Ok(42)).expect("lock + closure");
        assert_eq!(value, 42);
        // Lock file persists by design (see fn docstring).
        assert!(dir.path().join("target.txt.lock").exists());
    }

    #[test]
    fn with_exclusive_lock_propagates_closure_error() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("target.txt");
        let result: Result<()> =
            with_exclusive_lock(&target, || Err(CoralError::Walk("test error".into())));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(format!("{err:?}").contains("test error"));
    }

    /// The flagship invariant: under N concurrent `with_exclusive_lock`
    /// callers each running a load+modify+save round-trip against the
    /// same shared file, ALL N updates must persist. This is the test
    /// that pins the v0.15 fix for the lost-update race documented in
    /// `crates/coral-core/tests/concurrency.rs`.
    #[test]
    fn with_exclusive_lock_serializes_concurrent_load_modify_save() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("counter.txt");
        // Seed with "0" so the first reader has a parseable starting
        // value.
        atomic_write_string(&target, "0").expect("seed");

        const N: usize = 50;
        std::thread::scope(|s| {
            for _ in 0..N {
                let target = target.clone();
                s.spawn(move || {
                    with_exclusive_lock(&target, || {
                        // Classic load+modify+save: read the counter,
                        // increment, write back. Without locking this
                        // loses updates under concurrency.
                        let current: u32 = fs::read_to_string(&target)
                            .expect("read")
                            .trim()
                            .parse()
                            .expect("parse");
                        atomic_write_string(&target, &(current + 1).to_string())
                    })
                    .expect("locked closure");
                });
            }
        });

        let final_value: u32 = fs::read_to_string(&target)
            .expect("read final")
            .trim()
            .parse()
            .expect("parse final");
        assert_eq!(
            final_value,
            N as u32,
            "v0.15 with_exclusive_lock must serialize all {N} writers; \
             observed final counter = {final_value} (lost {} updates)",
            N as u32 - final_value
        );
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
