//! Shared test fixtures for cross-runner integration tests.
//!
//! Lives in `tests/common/mod.rs` so each integration test file can pull
//! it in with `mod common;`. Cargo treats `common/mod.rs` as a module of
//! the parent test binary, NOT as its own test target — that's the
//! recommended pattern for sharing helpers between integration tests.

/// Returns a `(TempDir, PathBuf)` holding an executable shell script that
/// ignores every CLI arg and writes `y\n` forever. Drop-in replacement
/// for `/usr/bin/yes` in timeout tests.
///
/// Why not `/usr/bin/yes` directly: GNU coreutils 9.4+ on Ubuntu 24.04
/// rejects unknown long options like `--print`, `--no-display-prompt`,
/// `--append-system-prompt`. Each Runner unconditionally adds at least
/// one such flag, so the child exits NonZeroExit before the timeout
/// deadline can fire.
///
/// Why TempDir + fs::write (not NamedTempFile): NamedTempFile keeps the
/// fd open with write access. Linux refuses to execute a file that has
/// an open writable fd — `ETXTBSY` "Text file busy". `fs::write` closes
/// the fd on completion, then we `chmod 755` separately.
///
/// Caller must hold the returned `TempDir` for the duration of the test
/// (Drop deletes the directory tree).
#[cfg(unix)]
pub fn forever_yes_script() -> (tempfile::TempDir, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::Builder::new()
        .prefix("coral-yes-")
        .tempdir()
        .expect("tempdir");
    let path = dir.path().join("yes.sh");
    std::fs::write(&path, "#!/bin/sh\nwhile :; do echo y; done\n").expect("write script");
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).expect("chmod 755");
    (dir, path)
}
