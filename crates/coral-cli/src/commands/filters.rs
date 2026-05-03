//! Common repo filters for the multi-repo era.
//!
//! Every command that operates over a `Project` accepts the same four
//! flags: `--repo`, `--tag`, `--affected`, and `--exclude`. Implementing
//! them once in this module — rather than having each command duplicate
//! the clap parsing and the filter logic — keeps the UX consistent and
//! the bug surface tiny.
//!
//! In v0.15 (legacy / single-repo) every filter resolves to "the only
//! repo is included", so the same code paths work without callers having
//! to special-case anything.

use clap::Args;
use coral_core::project::{Project, RepoEntry};
use std::collections::BTreeSet;

/// The four filter flags. Embed in any command's `Args` struct via
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

        project
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
            .collect()
    }

    /// `true` when none of the flags is set. Useful for short-circuit
    /// "select all" behavior in callers that want to log "operating on
    /// N repos" with extra context when filters were applied.
    pub fn is_empty(&self) -> bool {
        self.repo.is_empty() && self.tag.is_empty() && self.exclude.is_empty()
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
}
