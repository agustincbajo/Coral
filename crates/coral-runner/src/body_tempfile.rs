//! Shared RAII helper for routing curl request bodies through a per-call
//! mode-0600 tempfile instead of argv.
//!
//! v0.20.2 lift (audit cycle-4 #43, #44): the same pattern that
//! `HttpRunner` uses (v0.19.6 N2 + v0.19.7 #24/#25 hardening) now
//! covers `coral notion-push` (#43) and the embeddings providers
//! (#44). Three callers were each open-coding identical scaffolding;
//! consolidating here means one bug fix lands once instead of three
//! times next cycle.
//!
//! ## Why a tempfile and not stdin?
//!
//! curl's `-H @-` stdin form already consumes stdin for the secret
//! header (audit v0.19.5 H6). When the caller also needs to send a
//! request body, stdin is taken — so the body goes to a per-call
//! tempfile referenced via `--data-binary @<path>`.
//!
//! ## Hardening invariants
//!
//! - **Mode 0600 on Unix.** `/tmp` is shared across UIDs on multi-tenant
//!   Linux hosts. macOS is unaffected because `$TMPDIR` is per-user
//!   (`/var/folders/<hash>/T/`).
//! - **`create_new(true)`.** Refuse to clobber a pre-positioned
//!   symlink at the target path even though our path-generation
//!   already yields collision-resistant names (defense-in-depth).
//! - **RAII cleanup.** Pre-v0.19.7 the cleanup was hand-rolled at
//!   each return path; the validator agent caught three error paths
//!   where the file leaked. The guard's `Drop` impl makes cleanup
//!   uniform across success, error, and panic-unwind paths.

/// Resolve a per-call tempfile path for a curl request body.
///
/// Pid + nanos + atomic counter keep the path unique without bringing
/// in a new dep. The caller is responsible for writing to that path
/// via [`write_body_tempfile_secure`] and binding the result into a
/// [`TempFileGuard`] for cleanup.
///
/// `prefix` lets each caller distinguish its own tempfiles (e.g.
/// `"coral-runner-body"`, `"coral-notion-body"`, `"coral-embed-body"`)
/// — useful when triaging a leaked file.
pub fn body_tempfile_path(prefix: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let counter = N.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("{prefix}-{pid}-{nanos}-{counter}.json"))
}

/// Write `contents` to `path` with `O_CREAT | O_EXCL | mode 0600` on
/// Unix.
///
/// v0.19.7 audit-followup #24 (lifted in v0.20.2 #43/#44): the body
/// of a curl call must not be world-readable while curl is reading
/// it back via `--data-binary @<path>`. Pre-v0.19.7 the file went out
/// at `mode 0644` (default umask), restricting WRITE but not READ.
pub fn write_body_tempfile_secure(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(contents)
}

/// RAII cleanup for a per-call request-body tempfile. Wraps an
/// optional path so callers can bind a single guard regardless of
/// whether the no-API-key code path streams the body via stdin
/// (no tempfile in play → guard's `Drop` is a no-op).
///
/// Pre-v0.19.7 cleanup was hand-rolled and three error paths leaked
/// the file (header-write, body-write, wait-output). RAII closes that
/// gap uniformly.
pub struct TempFileGuard {
    path: Option<std::path::PathBuf>,
}

impl TempFileGuard {
    /// Bind a guard to `path`. `None` means "no tempfile in play"
    /// (e.g. the no-API-key code path that streams the body via
    /// stdin); the guard's `Drop` is then a no-op.
    pub fn new(path: Option<std::path::PathBuf>) -> Self {
        Self { path }
    }

    /// Borrow the guarded path. Returns `None` for guards bound to
    /// `None` (the stdin-only code path).
    pub fn as_path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if let Some(p) = self.path.take() {
            let _ = std::fs::remove_file(&p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// v0.20.2 lift: regression — the body tempfile is created with
    /// mode 0600 on Unix so other local users can't `cat` it from
    /// `/tmp`.
    #[cfg(unix)]
    #[test]
    fn body_tempfile_is_created_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("body.json");
        write_body_tempfile_secure(&path, b"hello").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "tempfile mode is {mode:o}, expected 0600 — see GitHub issue #24"
        );
    }

    /// v0.20.2 lift: regression — `create_new(true)` semantics refuse
    /// to clobber a pre-existing file. Defense-in-depth against a
    /// pre-positioned symlink.
    #[cfg(unix)]
    #[test]
    fn body_tempfile_secure_refuses_to_clobber() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("body.json");
        std::fs::write(&path, b"existing").unwrap();
        let err = write_body_tempfile_secure(&path, b"new").expect_err("must fail");
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    }

    /// v0.20.2 lift: `TempFileGuard` cleans up on Drop.
    #[test]
    fn temp_file_guard_removes_path_on_drop() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("guarded.json");
        std::fs::write(&path, b"x").unwrap();
        assert!(path.exists());
        {
            let _g = TempFileGuard::new(Some(path.clone()));
        }
        assert!(
            !path.exists(),
            "TempFileGuard did not remove the file on drop"
        );
    }

    /// v0.20.2 lift: a guard with `None` is a no-op on Drop. Used on
    /// the no-API-key code path where the body streams via stdin and
    /// there's no tempfile in play.
    #[test]
    fn temp_file_guard_with_none_is_noop() {
        let g = TempFileGuard::new(None);
        drop(g);
        // No panic, no file to assert on — the absence of error is
        // the contract.
    }

    /// Pin the body_tempfile_path uniqueness contract: two consecutive
    /// calls must yield distinct paths even within the same process.
    #[test]
    fn body_tempfile_path_yields_unique_paths() {
        let a = body_tempfile_path("coral-runner-body");
        let b = body_tempfile_path("coral-runner-body");
        assert_ne!(a, b, "tempfile paths must be unique within a process");
    }

    /// Pin the body_tempfile_path prefix contract: the chosen prefix
    /// appears in the file name so leaked tempfiles can be triaged.
    #[test]
    fn body_tempfile_path_includes_prefix() {
        let p = body_tempfile_path("coral-test-prefix");
        let name = p.file_name().unwrap().to_string_lossy();
        assert!(
            name.starts_with("coral-test-prefix-"),
            "expected prefix in tempfile name: {name}"
        );
    }
}
