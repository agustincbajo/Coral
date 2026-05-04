//! Path resolution helpers — currently focused on the
//! "repo root from wiki root" computation that's been bitten by
//! `Path::parent()`'s subtle behavior on relative single-component
//! paths multiple times.
//!
//! ## Background — the empty-parent foot-gun
//!
//! `Path::new(".wiki").parent()` returns `Some("")` (NOT `None`).
//! That empty `PathBuf` propagates downstream into
//! `std::process::Command::current_dir("")`, which on macOS surfaces
//! as `ENOENT` from `execvp`. The bug appeared first in `coral lint`
//! (fixed in commit d2d7012, v0.19.0), then again in `coral status`
//! (fixed in v0.19.2 after a user report), and a v0.19.3 audit found
//! the same class in `coral lint --fix`, `coral onboard --apply`, and
//! `coral consolidate`.
//!
//! The fix is the same one-liner everywhere: treat empty parent as
//! "no useful parent" and fall back to `.` (the current working dir).
//! Centralizing it here means future callers can't open-code the wrong
//! variant by accident.

use std::path::{Path, PathBuf};

/// Compute the project's repo root from a wiki root path.
///
/// **Contract:**
/// - If `wiki_root` has a non-empty parent component, return it.
/// - Otherwise (single-component relative path like `.wiki`, OR the
///   filesystem root itself), return `PathBuf::from(".")` so subprocess
///   `current_dir` calls work correctly relative to the user's cwd.
///
/// **Why not `wiki_root.parent().unwrap_or_else(|| PathBuf::from("."))`?**
/// Because `Path::new(".wiki").parent()` returns `Some("")` instead of
/// `None`, the `unwrap_or_else` doesn't fire and the empty PathBuf
/// propagates. The match-on-non-empty pattern below fires correctly
/// for both the `Some("")` case and the `None` case.
///
/// **Example:**
/// ```
/// use std::path::Path;
/// use coral_core::path::repo_root_from_wiki_root;
///
/// // Relative single-component — falls back to cwd marker `.`
/// assert_eq!(
///     repo_root_from_wiki_root(Path::new(".wiki")),
///     Path::new("."),
/// );
/// // Absolute — returns the actual parent directory
/// assert_eq!(
///     repo_root_from_wiki_root(Path::new("/work/repo/.wiki")),
///     Path::new("/work/repo"),
/// );
/// // Nested relative — returns the actual parent
/// assert_eq!(
///     repo_root_from_wiki_root(Path::new("repos/api/.wiki")),
///     Path::new("repos/api"),
/// );
/// ```
pub fn repo_root_from_wiki_root(wiki_root: &Path) -> PathBuf {
    match wiki_root.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_single_component_falls_back_to_dot() {
        // The exact case that bit `coral status` and `coral onboard`.
        assert_eq!(repo_root_from_wiki_root(Path::new(".wiki")), Path::new("."));
    }

    #[test]
    fn relative_nested_returns_real_parent() {
        assert_eq!(
            repo_root_from_wiki_root(Path::new("repos/api/.wiki")),
            Path::new("repos/api"),
        );
    }

    #[test]
    fn absolute_returns_real_parent() {
        assert_eq!(
            repo_root_from_wiki_root(Path::new("/work/repo/.wiki")),
            Path::new("/work/repo"),
        );
    }

    #[test]
    fn root_path_falls_back_to_dot() {
        // `Path::new("/").parent()` is `None`. Fall back to cwd.
        assert_eq!(repo_root_from_wiki_root(Path::new("/")), Path::new("."));
    }

    #[test]
    fn current_dir_marker_falls_back_to_dot() {
        // `Path::new(".").parent()` is `Some("")` — same trap as `.wiki`.
        // Must NOT return the empty PathBuf.
        assert_eq!(repo_root_from_wiki_root(Path::new(".")), Path::new("."));
    }

    #[test]
    fn returned_path_is_never_empty_for_relative_inputs() {
        // Property: regardless of input shape, the returned PathBuf is
        // safe to pass to `Command::current_dir` — never the empty
        // string that triggers ENOENT from execvp on macOS.
        for input in &[".wiki", ".", "wiki", "..", "../..", "a", "a/b/.wiki"] {
            let result = repo_root_from_wiki_root(Path::new(input));
            assert!(
                !result.as_os_str().is_empty(),
                "input {input:?} produced empty PathBuf"
            );
        }
    }
}
