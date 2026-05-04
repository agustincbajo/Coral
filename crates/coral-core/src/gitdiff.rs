//! Git diff invocation and `--name-status` parsing.
//!
//! Wraps `git diff --name-status <range>` with a typed `DiffEntry`. The
//! parser is independent of git itself so it can be tested without a repo;
//! `run` and `head_sha` shell out and are exercised behind `#[ignore]`.

use crate::error::{CoralError, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// File change status reported by `git diff --name-status`.
/// Maps the single-letter codes to a typed enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Unmerged,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiffEntry {
    pub kind: ChangeKind,
    pub path: PathBuf,
    /// Only set for Renamed/Copied: the original path.
    pub from_path: Option<PathBuf>,
}

/// Parses the stdout of `git diff --name-status [--no-renames]`.
/// Each line is one of:
///   - `A\t<path>`           — added
///   - `M\t<path>`            — modified
///   - `D\t<path>`            — deleted
///   - `R<n>\t<from>\t<to>`   — renamed (with similarity index)
///   - `C<n>\t<from>\t<to>`   — copied
///   - `T\t<path>`            — type changed
///   - `U\t<path>`            — unmerged
///
/// Empty lines and lines that don't match are skipped silently.
pub fn parse_name_status(stdout: &str) -> Vec<DiffEntry> {
    let mut entries = Vec::new();

    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 2 {
            // No tab separator → skip silently.
            continue;
        }

        let status = parts[0];
        let first_char = match status.chars().next() {
            Some(c) => c,
            None => continue,
        };

        match first_char {
            'A' if status == "A" => {
                entries.push(DiffEntry {
                    kind: ChangeKind::Added,
                    path: PathBuf::from(parts[1]),
                    from_path: None,
                });
            }
            'M' if status == "M" => {
                entries.push(DiffEntry {
                    kind: ChangeKind::Modified,
                    path: PathBuf::from(parts[1]),
                    from_path: None,
                });
            }
            'D' if status == "D" => {
                entries.push(DiffEntry {
                    kind: ChangeKind::Deleted,
                    path: PathBuf::from(parts[1]),
                    from_path: None,
                });
            }
            'T' if status == "T" => {
                entries.push(DiffEntry {
                    kind: ChangeKind::TypeChanged,
                    path: PathBuf::from(parts[1]),
                    from_path: None,
                });
            }
            'U' if status == "U" => {
                entries.push(DiffEntry {
                    kind: ChangeKind::Unmerged,
                    path: PathBuf::from(parts[1]),
                    from_path: None,
                });
            }
            'R' => {
                // Renamed: R<n>\t<from>\t<to>.
                if parts.len() >= 3 {
                    entries.push(DiffEntry {
                        kind: ChangeKind::Renamed,
                        path: PathBuf::from(parts[2]),
                        from_path: Some(PathBuf::from(parts[1])),
                    });
                }
            }
            'C' => {
                // Copied: C<n>\t<from>\t<to>.
                if parts.len() >= 3 {
                    entries.push(DiffEntry {
                        kind: ChangeKind::Copied,
                        path: PathBuf::from(parts[2]),
                        from_path: Some(PathBuf::from(parts[1])),
                    });
                }
            }
            _ => {
                entries.push(DiffEntry {
                    kind: ChangeKind::Unknown,
                    path: PathBuf::from(parts[1]),
                    from_path: None,
                });
            }
        }
    }

    entries
}

/// Runs `git diff --name-status <range>` in `repo_dir` and returns parsed entries.
/// Returns CoralError::Git(msg) on:
///   - git not found
///   - non-zero exit
///   - invalid range
///
/// `range` is something like "HEAD~5..HEAD" or "abc123..def456".
pub fn run(repo_dir: impl AsRef<Path>, range: &str) -> Result<Vec<DiffEntry>> {
    // v0.19.5 audit: a range that starts with `-` would be parsed as
    // a flag by `git diff` (CVE-2017-1000117 family). Reject early.
    // We also append `--` so any future pathspec extension stays
    // unambiguous: `git diff <range> -- [paths]` is the documented
    // shape — note `--` cannot precede the range or git would treat
    // the range as a pathspec.
    if range.starts_with('-') {
        return Err(CoralError::Git(format!(
            "git diff range `{range}` looks like a flag; refusing"
        )));
    }
    let output = Command::new("git")
        .current_dir(repo_dir.as_ref())
        .args(["diff", "--name-status", range, "--"])
        .output()
        .map_err(|e| CoralError::Git(format!("failed to invoke git: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(CoralError::Git(format!("git diff failed: {stderr}")));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_name_status(&stdout))
}

/// Convenience: HEAD as a string. Resolves the current HEAD sha.
/// Returns CoralError::Git on non-zero exit.
pub fn head_sha(repo_dir: impl AsRef<Path>) -> Result<String> {
    let output = Command::new("git")
        .current_dir(repo_dir.as_ref())
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|e| CoralError::Git(format!("failed to invoke git: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(CoralError::Git(format!("git rev-parse failed: {stderr}")));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    #[test]
    fn parse_added() {
        let entries = parse_name_status("A\tmodules/foo.md\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, ChangeKind::Added);
        assert_eq!(entries[0].path, PathBuf::from("modules/foo.md"));
        assert!(entries[0].from_path.is_none());
    }

    #[test]
    fn parse_modified_and_deleted() {
        let input = "M\tmodules/order.md\nD\tmodules/old.md\n";
        let entries = parse_name_status(input);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].kind, ChangeKind::Modified);
        assert_eq!(entries[0].path, PathBuf::from("modules/order.md"));
        assert_eq!(entries[1].kind, ChangeKind::Deleted);
        assert_eq!(entries[1].path, PathBuf::from("modules/old.md"));
    }

    #[test]
    fn parse_renamed_with_similarity() {
        let entries = parse_name_status("R100\told.md\tnew.md\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, ChangeKind::Renamed);
        assert_eq!(entries[0].path, PathBuf::from("new.md"));
        assert_eq!(entries[0].from_path, Some(PathBuf::from("old.md")));
    }

    #[test]
    fn parse_copied() {
        let entries = parse_name_status("C75\tsrc.md\tdup.md\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, ChangeKind::Copied);
        assert_eq!(entries[0].path, PathBuf::from("dup.md"));
        assert_eq!(entries[0].from_path, Some(PathBuf::from("src.md")));
    }

    #[test]
    fn parse_type_changed() {
        let entries = parse_name_status("T\tfile\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, ChangeKind::TypeChanged);
        assert_eq!(entries[0].path, PathBuf::from("file"));
    }

    #[test]
    fn parse_unmerged() {
        let entries = parse_name_status("U\tconflict.md\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, ChangeKind::Unmerged);
        assert_eq!(entries[0].path, PathBuf::from("conflict.md"));
    }

    #[test]
    fn parse_unknown() {
        let entries = parse_name_status("X\tweird.md\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, ChangeKind::Unknown);
        assert_eq!(entries[0].path, PathBuf::from("weird.md"));
    }

    #[test]
    fn parse_skips_empty_lines() {
        let input = "\n\nA\tvalid.md\n\n";
        let entries = parse_name_status(input);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, ChangeKind::Added);
    }

    #[test]
    fn parse_skips_lines_without_tab() {
        let input = "not a status\nA\tvalid.md\n";
        let entries = parse_name_status(input);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, ChangeKind::Added);
        assert_eq!(entries[0].path, PathBuf::from("valid.md"));
    }

    #[test]
    fn parse_handles_multiple_renames() {
        let input = "R100\ta.md\tb.md\nR90\tc.md\td.md\nR80\te.md\tf.md\n";
        let entries = parse_name_status(input);
        assert_eq!(entries.len(), 3);
        for entry in &entries {
            assert_eq!(entry.kind, ChangeKind::Renamed);
            assert!(entry.from_path.is_some());
        }
        assert_eq!(entries[0].path, PathBuf::from("b.md"));
        assert_eq!(entries[1].path, PathBuf::from("d.md"));
        assert_eq!(entries[2].path, PathBuf::from("f.md"));
    }

    /// Runs `git` with deterministic author/committer env vars set, so tests
    /// don't pick up the developer's personal config and don't fail on a CI
    /// box that has no global git identity.
    fn run_git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .status()
            .expect("git invocation failed");
        assert!(status.success(), "git {args:?} failed in {dir:?}");
    }

    #[test]
    #[ignore]
    fn run_against_real_repo() {
        let dir = TempDir::new().expect("tempdir");
        let repo = dir.path();

        run_git(repo, &["init", "-q"]);
        run_git(repo, &["commit", "--allow-empty", "-q", "-m", "first"]);

        std::fs::write(repo.join("file.md"), "hello").expect("write");
        run_git(repo, &["add", "file.md"]);
        run_git(repo, &["commit", "-q", "-m", "second"]);

        let entries = run(repo, "HEAD~1..HEAD").expect("git diff");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, ChangeKind::Added);
        assert_eq!(entries[0].path, PathBuf::from("file.md"));
    }

    /// v0.19.5 audit: refuse ranges that look like CLI flags
    /// (CVE-2017-1000117 family). Doesn't need a real repo because
    /// the validation happens before the spawn.
    #[test]
    fn run_rejects_flag_shaped_range() {
        let dir = TempDir::new().expect("tempdir");
        let err = run(dir.path(), "--upload-pack=evil").expect_err("must reject");
        let msg = format!("{err}");
        assert!(
            msg.contains("looks like a flag"),
            "unexpected error message: {msg}"
        );
    }

    #[test]
    #[ignore]
    fn head_sha_returns_40_char_hex() {
        let dir = TempDir::new().expect("tempdir");
        let repo = dir.path();

        run_git(repo, &["init", "-q"]);
        run_git(repo, &["commit", "--allow-empty", "-q", "-m", "first"]);

        let sha = head_sha(repo).expect("head_sha");
        assert_eq!(sha.len(), 40, "expected 40-char sha, got {sha:?}");
        assert!(
            sha.chars().all(|c| c.is_ascii_hexdigit()),
            "non-hex chars in sha: {sha}"
        );
    }
}
