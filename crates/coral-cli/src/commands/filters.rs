//! Common repo filters for the multi-repo era.
//!
//! Every command that operates over a `Project` accepts the same five
//! flags: `--repo`, `--tag`, `--affected`, `--since`, and `--exclude`.
//! Implementing them once in this module — rather than having each
//! command duplicate the clap parsing and the filter logic — keeps the
//! UX consistent and the bug surface tiny.
//!
//! In v0.15 (legacy / single-repo) every filter resolves to "the only
//! repo is included", so the same code paths work without callers having
//! to special-case anything.

use clap::Args;
use coral_core::project::{Project, RepoEntry};
use std::collections::BTreeSet;

/// The five filter flags. Embed in any command's `Args` struct via
/// `#[command(flatten)]`.
#[derive(Args, Debug, Clone, Default)]
pub struct RepoFilters {
    /// Only operate on the named repos. Repeatable.
    #[arg(long = "repo", value_name = "NAME", num_args = 1..)]
    pub repo: Vec<String>,

    /// Only operate on repos with at least one matching tag. Repeatable.
    #[arg(long = "tag", value_name = "TAG", num_args = 1..)]
    pub tag: Vec<String>,

    /// Skip the named repos. Applied **after** `--repo`/`--tag`.
    #[arg(long = "exclude", value_name = "NAME", num_args = 1..)]
    pub exclude: Vec<String>,

    /// Only operate on repos with files changed since this git ref.
    /// Must be used together with `--affected`.
    #[arg(long = "since", value_name = "REF", requires = "affected")]
    pub since: Option<String>,

    /// Enable affected-repo detection. Requires `--since`.
    #[arg(long = "affected", requires = "since")]
    pub affected: bool,
}

impl RepoFilters {
    /// Returns the subset of `project.repos` that match this filter set.
    /// Always preserves declaration order.
    ///
    /// In legacy (single-repo, synthesized) projects the only repo is
    /// always included regardless of flags — single-repo workflows
    /// stay zero-friction.
    pub fn select<'p>(&self, project: &'p Project) -> Vec<&'p RepoEntry> {
        if project.is_legacy() {
            return project.repos.iter().filter(|r| r.enabled).collect();
        }
        let want_names: BTreeSet<&str> = self.repo.iter().map(String::as_str).collect();
        let want_tags: BTreeSet<&str> = self.tag.iter().map(String::as_str).collect();
        let exclude_names: BTreeSet<&str> = self.exclude.iter().map(String::as_str).collect();

        let mut result: Vec<&'p RepoEntry> = project
            .repos
            .iter()
            .filter(|r| r.enabled)
            .filter(|r| {
                if !want_names.is_empty() && !want_names.contains(r.name.as_str()) {
                    return false;
                }
                if !want_tags.is_empty() && !r.tags.iter().any(|t| want_tags.contains(t.as_str())) {
                    return false;
                }
                if exclude_names.contains(r.name.as_str()) {
                    return false;
                }
                true
            })
            .collect();

        // Affected-repo filter: only keep repos with changes since `--since`.
        if self.affected
            && let Some(ref since_ref) = self.since
        {
            result.retain(|repo| repo_has_changes(repo, project, since_ref));
        }

        result
    }

    /// `true` when none of the flags is set. Useful for short-circuit
    /// "select all" behavior in callers that want to log "operating on
    /// N repos" with extra context when filters were applied.
    pub fn is_empty(&self) -> bool {
        self.repo.is_empty() && self.tag.is_empty() && self.exclude.is_empty() && !self.affected
    }
}

/// Returns true if the repo has any file changes between `since_ref` and HEAD.
///
/// For repos that live as a subdirectory within a larger git repository (the
/// mono-repo / meta-repo pattern), we run
///   `git diff --name-only <since_ref>..HEAD -- <repo_path>`
/// from the project root so only changes within that subtree are considered.
///
/// If the repo is not cloned locally or git fails, we conservatively include it.
fn repo_has_changes(repo: &RepoEntry, project: &Project, since_ref: &str) -> bool {
    use std::process::Command;

    let repo_path = repo
        .path
        .as_ref()
        .map(|p| project.root.join(p))
        .unwrap_or_else(|| project.root.clone());

    if !repo_path.exists() {
        // Repo not cloned locally — conservatively include it.
        return true;
    }

    // Determine whether this repo is its own git root or a subdirectory of
    // a parent git repo. We always run `git diff` from the git toplevel and
    // use `-- <relative_path>` to scope the diff to the repo's subtree.
    let toplevel = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&repo_path)
        .output();

    let (work_dir, pathspec) = match toplevel {
        Ok(ref o) if o.status.success() => {
            let tl = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let tl_path = std::path::PathBuf::from(&tl);
            // Canonicalize both paths to handle symlinks (e.g. /tmp -> /private/tmp on macOS).
            let canonical_tl = tl_path.canonicalize().unwrap_or_else(|_| tl_path.clone());
            let canonical_repo = repo_path.canonicalize().unwrap_or(repo_path.clone());
            // Compute the relative path from the git root to the repo dir.
            let rel = canonical_repo
                .strip_prefix(&canonical_tl)
                .unwrap_or(std::path::Path::new("."));
            if rel == std::path::Path::new("") || rel == std::path::Path::new(".") {
                // The repo IS the git root — no pathspec needed.
                (canonical_tl, None)
            } else {
                (canonical_tl, Some(rel.to_path_buf()))
            }
        }
        _ => {
            // Can't determine toplevel — just run from repo_path with no pathspec.
            (repo_path, None)
        }
    };

    let mut args = vec![
        "diff".to_string(),
        "--name-only".to_string(),
        format!("{since_ref}..HEAD"),
    ];
    if let Some(ref ps) = pathspec {
        args.push("--".to_string());
        args.push(ps.display().to_string());
    }

    let output = Command::new("git")
        .args(&args)
        .current_dir(&work_dir)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            // If there's any output, files changed.
            !o.stdout.is_empty()
        }
        _ => {
            // Git failed — conservatively include.
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_core::project::manifest;
    use std::path::Path;

    fn project_from_toml(raw: &str) -> Project {
        let mut p = manifest::parse_toml(raw, Path::new("/tmp/coral.toml")).unwrap();
        p.root = Path::new("/tmp").to_path_buf();
        p
    }

    #[test]
    fn empty_filters_select_all_enabled_repos() {
        let p = project_from_toml(
            r#"apiVersion = "coral.dev/v1"
[project]
name = "demo"

[[repos]]
name = "api"
url  = "git@example.com:acme/api.git"

[[repos]]
name = "worker"
url  = "git@example.com:acme/worker.git"
"#,
        );
        let f = RepoFilters::default();
        let selected = f.select(&p);
        assert_eq!(selected.len(), 2);
        assert!(f.is_empty());
    }

    #[test]
    fn repo_filter_picks_named_repos_only() {
        let p = project_from_toml(
            r#"apiVersion = "coral.dev/v1"
[project]
name = "demo"

[[repos]]
name = "api"
url  = "git@example.com:acme/api.git"

[[repos]]
name = "worker"
url  = "git@example.com:acme/worker.git"

[[repos]]
name = "shared"
url  = "git@example.com:acme/shared.git"
"#,
        );
        let f = RepoFilters {
            repo: vec!["api".into(), "shared".into()],
            ..Default::default()
        };
        let selected: Vec<&str> = f.select(&p).iter().map(|r| r.name.as_str()).collect();
        assert_eq!(selected, vec!["api", "shared"]);
    }

    #[test]
    fn tag_filter_matches_any_overlap() {
        let p = project_from_toml(
            r#"apiVersion = "coral.dev/v1"
[project]
name = "demo"

[[repos]]
name = "api"
url  = "git@example.com:acme/api.git"
tags = ["service", "team:platform"]

[[repos]]
name = "worker"
url  = "git@example.com:acme/worker.git"
tags = ["service", "team:data"]

[[repos]]
name = "shared"
url  = "git@example.com:acme/shared.git"
tags = ["library"]
"#,
        );
        let f = RepoFilters {
            tag: vec!["service".into()],
            ..Default::default()
        };
        let selected: Vec<&str> = f.select(&p).iter().map(|r| r.name.as_str()).collect();
        assert_eq!(selected, vec!["api", "worker"]);
    }

    #[test]
    fn exclude_filter_runs_last() {
        let p = project_from_toml(
            r#"apiVersion = "coral.dev/v1"
[project]
name = "demo"

[[repos]]
name = "api"
url  = "git@example.com:acme/api.git"
tags = ["service"]

[[repos]]
name = "worker"
url  = "git@example.com:acme/worker.git"
tags = ["service"]
"#,
        );
        let f = RepoFilters {
            tag: vec!["service".into()],
            exclude: vec!["worker".into()],
            ..Default::default()
        };
        let selected: Vec<&str> = f.select(&p).iter().map(|r| r.name.as_str()).collect();
        assert_eq!(selected, vec!["api"]);
    }

    #[test]
    fn legacy_project_ignores_filters() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = Project::synthesize_legacy(dir.path());
        let f = RepoFilters {
            repo: vec!["nonexistent".into()],
            ..Default::default()
        };
        let selected = f.select(&p);
        assert_eq!(
            selected.len(),
            1,
            "legacy project always selects its only repo"
        );
    }

    #[test]
    fn is_empty_returns_false_when_affected_is_set() {
        let f = RepoFilters {
            affected: true,
            since: Some("main".into()),
            ..Default::default()
        };
        assert!(!f.is_empty());
    }

    #[test]
    fn default_filters_have_affected_disabled() {
        let f = RepoFilters::default();
        assert!(!f.affected);
        assert!(f.since.is_none());
    }

    #[test]
    fn affected_filter_excludes_repos_without_changes() {
        // This test uses a real git repo in a tempdir to verify the
        // affected filter actually shells out to git and excludes repos
        // that have no diff.
        use std::process::Command;

        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();

        // Initialize a git repo with one commit on main.
        Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(root)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(root)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(root)
            .output()
            .unwrap();

        // Create sub-directories for two repos.
        std::fs::create_dir_all(root.join("api")).unwrap();
        std::fs::create_dir_all(root.join("worker")).unwrap();
        std::fs::write(root.join("api/main.rs"), "fn main() {}").unwrap();
        std::fs::write(root.join("worker/main.rs"), "fn main() {}").unwrap();

        Command::new("git")
            .args(["add", "."])
            .current_dir(root)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(root)
            .output()
            .unwrap();

        // Now only touch a file in `api/`.
        std::fs::write(root.join("api/main.rs"), "fn main() { /* changed */ }").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(root)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "change api"])
            .current_dir(root)
            .output()
            .unwrap();

        // Build a project that maps those directories.
        let toml_str = r#"apiVersion = "coral.dev/v1"
[project]
name = "test"

[[repos]]
name = "api"
url  = "git@example.com:acme/api.git"
path = "api"

[[repos]]
name = "worker"
url  = "git@example.com:acme/worker.git"
path = "worker"
"#
        .to_string();
        let manifest_path = root.join("coral.toml");
        std::fs::write(&manifest_path, &toml_str).unwrap();
        let mut p = manifest::parse_toml(&toml_str, &manifest_path).unwrap();
        p.root = root.to_path_buf();

        // With `--affected --since HEAD~1`, only `api` should be selected.
        let f = RepoFilters {
            affected: true,
            since: Some("HEAD~1".into()),
            ..Default::default()
        };
        let selected: Vec<&str> = f.select(&p).iter().map(|r| r.name.as_str()).collect();
        assert_eq!(selected, vec!["api"]);
    }
}
