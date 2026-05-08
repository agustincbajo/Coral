//! Shared helpers for the CLI commands.
//!
//! In v0.16 the only public helper is `resolve_project`, the **single
//! point of entry** that every command uses to find out which `Project`
//! it operates on. It walks up from the cwd looking for a `coral.toml`
//! and falls back to the legacy single-repo synthesis when none is
//! found — preserving full v0.15 behavior.
//!
//! Future shared bits (config resolution, telemetry, error context)
//! land here.

pub mod untrusted_fence;

use anyhow::{Context, Result};
use coral_core::project::Project;
use std::path::{Path, PathBuf};

/// Resolve the `Project` for a command invocation.
///
/// Resolution order:
/// 1. If `wiki_root_override` is set (the user passed `--wiki-root`),
///    behave **exactly like v0.15** — synthesize a single-repo project
///    whose wiki points at that path. Doing anything else here would
///    break every test and script that wires `--wiki-root` to a
///    fixture path.
/// 2. Otherwise walk up from `cwd` looking for `coral.toml`.
/// 3. If no manifest is found, synthesize a legacy single-repo project
///    rooted at the cwd. This is the v0.15 case.
///
/// **Backward-compat invariant:** returning a legacy `Project` must
/// behave identically to v0.15 — its `wiki_root()` is `<cwd>/.wiki/`
/// (or the override) and there is no aggregation, no namespaced slugs,
/// no manifest disk I/O.
pub fn resolve_project(wiki_root_override: Option<&Path>) -> Result<Project> {
    let cwd = std::env::current_dir().context("getting cwd")?;
    if let Some(override_path) = wiki_root_override {
        return Ok(legacy_with_wiki_root(override_path, &cwd));
    }
    Project::discover(&cwd).context("resolving project from cwd")
}

/// Synthesize a legacy project where the wiki lives at
/// `wiki_root_override` instead of `<cwd>/.wiki`. Used for the
/// `--wiki-root` flag and for tests that pass an explicit fixture
/// path.
///
/// Implementation note: the project root is the **parent** of the
/// override path, so `Project.wiki_root()` returns the override
/// itself. The repo `path` stays at the cwd.
fn legacy_with_wiki_root(wiki_root_override: &Path, cwd: &Path) -> Project {
    let absolute_override = if wiki_root_override.is_absolute() {
        wiki_root_override.to_path_buf()
    } else {
        cwd.join(wiki_root_override)
    };
    // Parent of `<x>/.wiki` is `<x>` — Project::wiki_root() returns
    // `<root>/.wiki`, so root = parent works.
    let root = absolute_override
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut project = Project::synthesize_legacy(&root);
    project.root = root;
    project
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn resolve_returns_legacy_when_no_manifest() {
        let dir = TempDir::new().unwrap();
        let _guard = crate::commands::CWD_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = resolve_project(None);
        std::env::set_current_dir(original).unwrap();
        let project = result.unwrap();
        assert!(project.is_legacy());
    }

    #[test]
    fn resolve_finds_manifest_above_cwd() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("coral.toml"),
            r#"apiVersion = "coral.dev/v1"
[project]
name = "demo"

[[repos]]
name = "api"
url  = "git@example.com:acme/api.git"
"#,
        )
        .unwrap();
        let nested = dir.path().join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        let _guard = crate::commands::CWD_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(&nested).unwrap();
        let result = resolve_project(None);
        std::env::set_current_dir(original).unwrap();
        let project = result.unwrap();
        assert!(!project.is_legacy());
        assert_eq!(project.name, "demo");
    }

    #[test]
    fn resolve_with_wiki_root_override_keeps_legacy_shape() {
        let dir = TempDir::new().unwrap();
        let override_path = dir.path().join(".wiki");
        let _guard = crate::commands::CWD_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = resolve_project(Some(&override_path));
        std::env::set_current_dir(original).unwrap();
        let project = result.unwrap();
        assert!(project.is_legacy());
        assert_eq!(project.wiki_root(), override_path);
    }
}
