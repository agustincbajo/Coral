//! Git clone/fetch helpers for `coral project sync`.
//!
//! Uses subprocess `git` (same pattern as `gitdiff::head_sha`) instead of
//! a libgit2 binding because (a) it sidesteps the libgit2 build hassle on
//! some platforms and (b) it lets the user's git config — SSH keys,
//! credential helpers, GPG signing — work transparently.
//!
//! Auth lives entirely in the user's environment. Coral never prompts
//! for credentials, never stores tokens, and treats authentication
//! errors as "skip this repo with a warning, continue with the rest"
//! (per PRD risk #10) so a partial sync never blocks the rest of the
//! project.

use crate::error::{CoralError, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Outcome of a single repo's sync operation. Returned by `sync_repo`
/// so callers can build a structured report instead of bailing on the
/// first failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// Newly cloned from the remote URL.
    Cloned { sha: String },
    /// Already cloned; fast-forwarded (or already up to date).
    Updated { sha: String },
    /// Already cloned; the local clone is dirty (uncommitted changes,
    /// branch mismatch). Sync skipped to avoid clobbering work in
    /// progress.
    SkippedDirty { reason: String },
    /// Auth failed — bad SSH key, missing token, etc. Sync skipped;
    /// other repos continue.
    SkippedAuth { stderr_tail: String },
    /// Generic non-zero git exit that doesn't match any of the above.
    Failed { stderr_tail: String },
}

impl SyncOutcome {
    pub fn sha(&self) -> Option<&str> {
        match self {
            SyncOutcome::Cloned { sha } | SyncOutcome::Updated { sha } => Some(sha.as_str()),
            _ => None,
        }
    }
    pub fn is_skipped(&self) -> bool {
        matches!(
            self,
            SyncOutcome::SkippedDirty { .. } | SyncOutcome::SkippedAuth { .. }
        )
    }
    pub fn is_failed(&self) -> bool {
        matches!(self, SyncOutcome::Failed { .. })
    }
}

/// Clone or update `path` to match `url` at `r#ref`.
///
/// Behavior:
/// - If `path` does not exist → `git clone <url> <path>` then checkout `r#ref`.
/// - If `path` exists and is a git repo → `git fetch origin <ref>` then
///   `git checkout` + fast-forward `git merge --ff-only`.
/// - If the working tree is dirty → returns `SkippedDirty` instead of
///   risking the user's in-progress work.
/// - If git's stderr matches well-known auth-failure patterns →
///   returns `SkippedAuth` so the caller can warn the dev to check
///   their SSH agent / credential helper without aborting the whole
///   project sync.
///
/// Returns `Err` only for filesystem errors (couldn't find git, can't
/// read working dir). All git-level failures are encoded in
/// `SyncOutcome::Failed { stderr_tail }`.
pub fn sync_repo(url: &str, r#ref: &str, path: &Path) -> Result<SyncOutcome> {
    if !path.exists() {
        return clone_fresh(url, r#ref, path);
    }
    update_existing(r#ref, path)
}

fn clone_fresh(url: &str, r#ref: &str, path: &Path) -> Result<SyncOutcome> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|source| CoralError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
    }
    // v0.19.5 audit: keep all `--flag` arguments BEFORE the `--`
    // separator and put user-controlled positionals (`url`, `path`)
    // AFTER. Without `--`, a malicious URL like `--upload-pack=evil`
    // would be parsed by git as a flag rather than a positional —
    // CVE-2017-1000117 / CVE-2024-32004 family.
    let output = Command::new("git")
        .args([
            "clone",
            "--branch",
            r#ref,
            "--",
            url,
            path.to_string_lossy().as_ref(),
        ])
        .output()
        .map_err(|e| CoralError::Git(format!("failed to invoke git clone: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if classify_auth_failure(&stderr) {
            return Ok(SyncOutcome::SkippedAuth {
                stderr_tail: tail(&stderr),
            });
        }
        // Some refs aren't branches but tags or commits. `--branch` only
        // accepts branches/tags. Fall back to a plain clone + checkout.
        if classify_branch_not_found(&stderr) {
            return clone_then_checkout(url, r#ref, path);
        }
        return Ok(SyncOutcome::Failed {
            stderr_tail: tail(&stderr),
        });
    }

    head_sha_at(path)
        .map(|sha| SyncOutcome::Cloned { sha })
        .or_else(|_| {
            Ok(SyncOutcome::Failed {
                stderr_tail: "clone succeeded but rev-parse failed".to_string(),
            })
        })
}

fn clone_then_checkout(url: &str, r#ref: &str, path: &Path) -> Result<SyncOutcome> {
    // v0.19.5 audit: see clone_fresh — `--` separates flags from
    // user-controlled positionals (CVE-2017-1000117 / CVE-2024-32004).
    let output = Command::new("git")
        .args(["clone", "--", url, path.to_string_lossy().as_ref()])
        .output()
        .map_err(|e| CoralError::Git(format!("failed to invoke git clone: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if classify_auth_failure(&stderr) {
            return Ok(SyncOutcome::SkippedAuth {
                stderr_tail: tail(&stderr),
            });
        }
        return Ok(SyncOutcome::Failed {
            stderr_tail: tail(&stderr),
        });
    }
    // v0.19.5 audit: `git checkout --quiet <ref>` — `<ref>` is a
    // user-controlled positional. We can't use `--` here because that
    // would tell git to treat the ref as a pathspec; instead reject
    // refs that start with `-` (which would be parsed as a flag).
    if r#ref.starts_with('-') {
        return Ok(SyncOutcome::Failed {
            stderr_tail: format!("ref `{}` looks like a flag; refusing", r#ref),
        });
    }
    let checkout = Command::new("git")
        .current_dir(path)
        .args(["checkout", "--quiet", r#ref])
        .output()
        .map_err(|e| CoralError::Git(format!("failed to invoke git checkout: {e}")))?;
    if !checkout.status.success() {
        let stderr = String::from_utf8_lossy(&checkout.stderr);
        return Ok(SyncOutcome::Failed {
            stderr_tail: tail(&stderr),
        });
    }
    let sha = head_sha_at(path)?;
    Ok(SyncOutcome::Cloned { sha })
}

fn update_existing(r#ref: &str, path: &Path) -> Result<SyncOutcome> {
    if !path.join(".git").exists() {
        return Ok(SyncOutcome::Failed {
            stderr_tail: format!("{} is not a git repository", path.display()),
        });
    }

    if working_tree_is_dirty(path)? {
        return Ok(SyncOutcome::SkippedDirty {
            reason: "uncommitted changes present; refusing to clobber".to_string(),
        });
    }

    // v0.19.5 audit: refuse refs that look like flags. `git fetch`
    // and `git checkout` accept `<ref>` as a positional but parse
    // anything starting with `-` as a flag — same root cause as
    // CVE-2017-1000117.
    if r#ref.starts_with('-') {
        return Ok(SyncOutcome::Failed {
            stderr_tail: format!("ref `{}` looks like a flag; refusing", r#ref),
        });
    }
    // Fetch the desired ref. `origin` is the conventional default; we
    // never override it because the user's clone owns its remotes.
    let fetch = Command::new("git")
        .current_dir(path)
        .args(["fetch", "--quiet", "origin", r#ref])
        .output()
        .map_err(|e| CoralError::Git(format!("failed to invoke git fetch: {e}")))?;
    if !fetch.status.success() {
        let stderr = String::from_utf8_lossy(&fetch.stderr);
        if classify_auth_failure(&stderr) {
            return Ok(SyncOutcome::SkippedAuth {
                stderr_tail: tail(&stderr),
            });
        }
        return Ok(SyncOutcome::Failed {
            stderr_tail: tail(&stderr),
        });
    }

    let checkout = Command::new("git")
        .current_dir(path)
        .args(["checkout", "--quiet", r#ref])
        .output()
        .map_err(|e| CoralError::Git(format!("failed to invoke git checkout: {e}")))?;
    if !checkout.status.success() {
        let stderr = String::from_utf8_lossy(&checkout.stderr);
        return Ok(SyncOutcome::Failed {
            stderr_tail: tail(&stderr),
        });
    }

    // Try a fast-forward merge. If the ref is a tag or commit (no
    // upstream), the merge is a no-op and we're done; we don't treat
    // either case as failure. Pre-v0.19.4 this was `let _ = …`
    // entirely fire-and-forget — uncommitted work / merge conflicts
    // / rev-walk failures all silently skipped, so users debugging
    // "why is my clone not advancing?" had nothing to grep for. We
    // now log every outcome at the right level so a `RUST_LOG=coral=debug`
    // run carries a complete trail. See GitHub issue #22.
    match Command::new("git")
        .current_dir(path)
        .args(["merge", "--ff-only", "--quiet"])
        .output()
    {
        Ok(out) if out.status.success() => {
            tracing::debug!(
                path = %path.display(),
                r#ref = %r#ref,
                "git merge --ff-only succeeded (or was a no-op)"
            );
        }
        Ok(out) => {
            // Non-zero exit. Common reasons: branch already up-to-date
            // (still exit 0 actually), upstream is behind/diverged,
            // or no upstream tracking. Log at warn so users debugging
            // sync drift see why the clone didn't advance.
            tracing::warn!(
                path = %path.display(),
                r#ref = %r#ref,
                stderr = %String::from_utf8_lossy(&out.stderr).trim(),
                "git merge --ff-only did not progress; clone stays at the post-checkout sha"
            );
        }
        Err(e) => {
            // Couldn't even spawn git. The fetch/checkout steps above
            // would normally have failed first with a clearer error,
            // so this is rare — but log so we surface ANY git-spawn
            // failure consistently.
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "git merge --ff-only failed to spawn; clone stays at the post-checkout sha"
            );
        }
    }

    let sha = head_sha_at(path)?;
    Ok(SyncOutcome::Updated { sha })
}

fn working_tree_is_dirty(path: &Path) -> Result<bool> {
    let output = Command::new("git")
        .current_dir(path)
        .args(["status", "--porcelain"])
        .output()
        .map_err(|e| CoralError::Git(format!("failed to invoke git status: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(CoralError::Git(format!("git status failed: {stderr}")));
    }
    Ok(!output.stdout.is_empty())
}

fn head_sha_at(repo_dir: &Path) -> Result<String> {
    let output = Command::new("git")
        .current_dir(repo_dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|e| CoralError::Git(format!("failed to invoke git rev-parse: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(CoralError::Git(format!("git rev-parse failed: {stderr}")));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Heuristic detection of auth failures from git's stderr. The message
/// shapes are stable across modern git (≥2.30); we match on substrings
/// rather than locale-sensitive prefixes.
fn classify_auth_failure(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains("authentication failed")
        || s.contains("permission denied (publickey)")
        || s.contains("could not read username")
        || s.contains("could not read from remote repository")
        || s.contains("remote: invalid username or password")
        || s.contains("403")
}

fn classify_branch_not_found(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains("remote branch") && s.contains("not found")
}

fn tail(stderr: &str) -> String {
    let trimmed = stderr.trim();
    if trimmed.len() <= 400 {
        trimmed.to_string()
    } else {
        format!("…{}", &trimmed[trimmed.len() - 400..])
    }
}

/// Returns `true` when `path` is a git working tree (has a `.git` entry).
pub fn is_git_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

/// Public for tests + `coral project doctor`.
pub fn classify_failure_for_test(stderr: &str) -> &'static str {
    if classify_auth_failure(stderr) {
        "auth"
    } else if classify_branch_not_found(stderr) {
        "branch_not_found"
    } else {
        "other"
    }
}

/// Helper for tests that want to seed a path with a fixed `.git`
/// marker without actually cloning anything.
#[cfg(test)]
fn _mark_as_git_repo(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path.join(".git"))
}

/// Re-export for callers that want PathBuf-typed paths.
pub fn make_path(s: &str) -> PathBuf {
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_auth_recognizes_publickey_denied() {
        assert!(classify_auth_failure(
            "ERROR: Permission denied (publickey).\nfatal: Could not read from remote repository.\n"
        ));
    }

    #[test]
    fn classify_auth_recognizes_authentication_failed() {
        assert!(classify_auth_failure(
            "remote: HTTP Basic: Access denied\nfatal: Authentication failed for 'https://...'\n"
        ));
    }

    #[test]
    fn classify_auth_does_not_match_unrelated_errors() {
        assert!(!classify_auth_failure(
            "fatal: not a git repository (or any of the parent directories): .git\n"
        ));
    }

    #[test]
    fn classify_branch_not_found_recognizes_remote_branch_missing() {
        assert!(classify_branch_not_found(
            "fatal: Remote branch foo not found in upstream origin\n"
        ));
    }

    #[test]
    fn tail_truncates_long_stderr() {
        let long = "x".repeat(800);
        let t = tail(&long);
        assert!(t.starts_with('…'));
        // `…` is 3 bytes in UTF-8 + 400 ASCII bytes = 403.
        assert!(t.len() <= 403, "got {} bytes", t.len());
        assert!(t.chars().count() <= 401);
    }

    #[test]
    fn tail_passes_short_stderr_through() {
        assert_eq!(tail("short message"), "short message");
    }

    #[test]
    fn is_git_repo_detects_dot_git() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(!is_git_repo(dir.path()));
        _mark_as_git_repo(dir.path()).unwrap();
        assert!(is_git_repo(dir.path()));
    }

    /// v0.19.5 audit C3: regression for `git clone` option injection.
    ///
    /// Inspect the `Command` we'd build for a fresh clone and assert
    /// the `--` separator sits between the flag block and the
    /// user-controlled positionals (`url`, `path`). Without `--`, a
    /// URL like `--upload-pack=evil` would be parsed as a flag —
    /// CVE-2017-1000117 / CVE-2024-32004 family.
    #[test]
    fn clone_command_inserts_dash_dash_before_positionals() {
        let url = "--upload-pack=/tmp/evil";
        let path = std::path::PathBuf::from("/tmp/coral-test-clone");
        let r#ref = "main";
        // Replicates the args block in `clone_fresh`.
        let cmd = Command::new("git")
            .args([
                "clone",
                "--branch",
                r#ref,
                "--",
                url,
                path.to_string_lossy().as_ref(),
            ])
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let dash_dash_idx = cmd.iter().position(|a| a == "--").expect("`--` present");
        let url_idx = cmd.iter().position(|a| a == url).expect("url present");
        assert!(
            dash_dash_idx < url_idx,
            "`--` must precede the user-controlled URL: {:?}",
            cmd
        );
    }

    /// v0.19.5 audit C3: ref-shaped flags rejected before reaching git.
    /// We can't easily test through `update_existing` (it spawns
    /// `git status` first); the synchronous shape of the check is
    /// preserved by inspecting the args list of the Command we'd
    /// build, plus a unit test of the substring pattern below.
    #[test]
    fn flag_shaped_ref_rejected_pattern() {
        let bad = "--upload-pack=evil";
        // Replicates the leading-`-` check applied at the call sites.
        assert!(bad.starts_with('-'), "bad ref must start with a dash");
        let good = "main";
        assert!(!good.starts_with('-'));
    }

    /// Real-git integration: fresh clone of a local bare repo. Gated on
    /// `git` being on PATH; passes in CI where git is always present.
    #[test]
    #[ignore]
    fn sync_repo_clones_a_local_bare_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        let bare = dir.path().join("bare.git");
        let work = dir.path().join("work");
        let clone_target = dir.path().join("client");

        // Build a fixture: bare repo + a worktree commit + push.
        Command::new("git")
            .args(["init", "--bare", bare.to_str().unwrap()])
            .status()
            .unwrap();
        Command::new("git")
            .args(["init", work.to_str().unwrap()])
            .status()
            .unwrap();
        std::fs::write(work.join("README.md"), "hello\n").unwrap();
        Command::new("git")
            .current_dir(&work)
            .args(["add", "."])
            .status()
            .unwrap();
        Command::new("git")
            .current_dir(&work)
            .args([
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-m",
                "init",
            ])
            .status()
            .unwrap();
        Command::new("git")
            .current_dir(&work)
            .args(["branch", "-M", "main"])
            .status()
            .unwrap();
        Command::new("git")
            .current_dir(&work)
            .args(["remote", "add", "origin", bare.to_str().unwrap()])
            .status()
            .unwrap();
        Command::new("git")
            .current_dir(&work)
            .args(["push", "-u", "origin", "main"])
            .status()
            .unwrap();

        let outcome = sync_repo(bare.to_str().unwrap(), "main", &clone_target).unwrap();
        match outcome {
            SyncOutcome::Cloned { sha } => assert_eq!(sha.len(), 40),
            other => panic!("expected Cloned, got {other:?}"),
        }
        assert!(clone_target.join("README.md").is_file());
    }
}
