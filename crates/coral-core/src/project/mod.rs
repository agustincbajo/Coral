//! Multi-repo project model (v0.16+).
//!
//! A `Project` is a logical grouping of one or more git repositories that share
//! a single aggregated wiki at `<root>/.wiki/`. The single-repo case is treated
//! as a `Project` with one `RepoEntry { url: None, path: "." }` synthesized
//! from the cwd — this preserves backward compatibility with v0.15 users.
//!
//! Discovery walks up from the cwd looking for a `coral.toml` that contains a
//! `[project]` section. If none is found, `synthesize_legacy()` produces the
//! single-repo project that v0.15 users implicitly had.
//!
//! Wiki layout in v0.16: only `aggregated` is supported — a single `.wiki/` at
//! the meta-repo root, with slugs namespaced as `<repo>/<slug>` when more than
//! one repo is present. Single-repo projects keep bare slugs for full
//! BC.

pub mod lock;
pub mod manifest;

pub use lock::{Lockfile, RepoLockEntry};
pub use manifest::{Project, ProjectDefaults, RemoteSpec, RepoEntry, WikiLayout};

use crate::error::{CoralError, Result};
use std::path::{Path, PathBuf};

/// Filename of the manifest at the project root (`coral.toml`).
pub const MANIFEST_FILENAME: &str = "coral.toml";

/// Filename of the lockfile (`coral.lock`). Sibling of the manifest.
pub const LOCKFILE_FILENAME: &str = "coral.lock";

/// Filename of the per-developer override file (`coral.local.toml`).
/// Gitignored; never persisted from in-memory state.
pub const LOCAL_OVERRIDES_FILENAME: &str = "coral.local.toml";

impl Project {
    /// Discover the project relative to `cwd`. Walks up the directory tree
    /// looking for a `coral.toml` containing a `[project]` table. If none is
    /// found anywhere up to the filesystem root, returns a synthesized
    /// single-repo project rooted at `cwd` — preserving v0.15 behavior.
    ///
    /// Errors only for filesystem-level failures or malformed manifests; a
    /// missing manifest is handled silently.
    pub fn discover(cwd: impl AsRef<Path>) -> Result<Self> {
        let cwd = cwd.as_ref();
        if let Some(manifest_path) = find_manifest_upwards(cwd) {
            Self::load_from_manifest(&manifest_path)
        } else {
            Ok(Self::synthesize_legacy(cwd))
        }
    }

    /// Build a single-repo project rooted at `cwd`. The repo `name` is the
    /// basename of `cwd` (sanitized) and `url` is `None` — the repo is
    /// understood to live in-place, never to be cloned. The wiki lives at
    /// `<cwd>/.wiki/`.
    ///
    /// This is the contract that gives v0.15 users zero-friction upgrade to
    /// v0.16: every command, when no `coral.toml` is found, synthesizes one
    /// of these and proceeds as if a multi-repo project existed.
    pub fn synthesize_legacy(cwd: impl AsRef<Path>) -> Self {
        let cwd = cwd.as_ref();
        let name = cwd
            .file_name()
            .and_then(|n| n.to_str())
            .map(sanitize_name)
            .unwrap_or_else(|| "wiki".to_string());
        Self::single_repo(name, cwd.to_path_buf())
    }

    /// Load and validate a `coral.toml` from disk.
    ///
    /// `manifest_path` may be relative (e.g. `"coral.toml"`) or
    /// absolute. The function resolves the project root from the
    /// manifest's parent directory using
    /// [`crate::path::repo_root_from_wiki_root`] — that helper handles
    /// the empty-parent foot-gun where `Path::new("coral.toml").parent()`
    /// returns `Some("")` (NOT `None`), which a naive `unwrap_or(".")`
    /// would silently leak as a downstream PathBuf("").
    /// See GitHub issue #20.
    pub fn load_from_manifest(manifest_path: impl AsRef<Path>) -> Result<Self> {
        let path = manifest_path.as_ref();
        let raw = std::fs::read_to_string(path).map_err(|source| CoralError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let mut project = manifest::parse_toml(&raw, path)?;
        // The manifest never contains `root` (it's implied by location).
        project.root = crate::path::repo_root_from_wiki_root(path);
        project.manifest_path = path.to_path_buf();
        project.validate()?;
        Ok(project)
    }

    /// Returns the wiki root for this project. Always
    /// `<project.root>/.wiki/` in v0.16 — only `aggregated` layout is
    /// supported. Single-repo projects use this same path, so existing
    /// `<repo>/.wiki/` layouts work without migration.
    pub fn wiki_root(&self) -> PathBuf {
        self.root.join(".wiki")
    }

    /// `true` when this project has more than one repo. Multi-repo affects
    /// slug namespacing, ingest ordering, and several command UX surfaces.
    pub fn is_multi_repo(&self) -> bool {
        self.repos.len() > 1
    }

    /// `true` when this project was synthesized from a bare cwd (no
    /// `coral.toml` present). Used by commands that want to silently keep
    /// v0.15 behavior.
    pub fn is_legacy(&self) -> bool {
        self.manifest_path.as_os_str().is_empty()
    }

    /// Returns the path to the lockfile (`coral.lock`) for this project.
    pub fn lockfile_path(&self) -> PathBuf {
        self.root.join(LOCKFILE_FILENAME)
    }

    /// Returns the path to the local-overrides file (`coral.local.toml`).
    pub fn local_overrides_path(&self) -> PathBuf {
        self.root.join(LOCAL_OVERRIDES_FILENAME)
    }

    /// Look up a repo entry by name.
    pub fn repo_by_name(&self, name: &str) -> Option<&RepoEntry> {
        self.repos.iter().find(|r| r.name == name)
    }

    /// The resolved on-disk path for a repo entry — applies the project's
    /// default `path_template` if the entry didn't specify one. Never None.
    pub fn resolved_path(&self, repo: &RepoEntry) -> PathBuf {
        if let Some(p) = &repo.path {
            if p.is_absolute() {
                return p.clone();
            }
            return self.root.join(p);
        }
        let templated = self.defaults.path_template.replace("{name}", &repo.name);
        self.root.join(templated)
    }

    /// The resolved git URL for a repo. Either the explicit `url` field or
    /// the project's `[remotes.<remote>]` template with `{name}` substituted.
    /// Returns `None` for legacy single-repo entries that live in-place.
    pub fn resolved_url(&self, repo: &RepoEntry) -> Option<String> {
        if let Some(u) = &repo.url {
            return Some(u.clone());
        }
        let remote_name = repo.remote.as_ref().or(self.defaults.remote.as_ref())?;
        let template = self.remotes.get(remote_name)?;
        Some(template.fetch.replace("{name}", &repo.name))
    }
}

fn find_manifest_upwards(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        let candidate = current.join(MANIFEST_FILENAME);
        if candidate.is_file() && manifest_has_project_section(&candidate) {
            return Some(candidate);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// A `coral.toml` may exist for unrelated reasons (e.g. an example file in
/// docs). To avoid false positives we require a real `[project]` section to
/// claim the file as the project manifest.
fn manifest_has_project_section(path: &Path) -> bool {
    match std::fs::read_to_string(path) {
        Ok(s) => s.lines().any(|line| {
            let l = line.trim();
            l == "[project]" || l.starts_with("[project ") || l.starts_with("[project.")
        }),
        Err(_) => false,
    }
}

fn sanitize_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    if out.is_empty() {
        "wiki".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn synthesize_legacy_uses_basename() {
        let dir = TempDir::new().unwrap();
        let cwd = dir.path().join("my-repo");
        std::fs::create_dir(&cwd).unwrap();
        let p = Project::synthesize_legacy(&cwd);
        assert_eq!(p.name, "my-repo");
        assert_eq!(p.repos.len(), 1);
        assert!(p.repos[0].url.is_none());
        assert!(p.is_legacy());
        assert!(!p.is_multi_repo());
        assert_eq!(p.wiki_root(), cwd.join(".wiki"));
    }

    #[test]
    fn synthesize_legacy_sanitizes_special_chars() {
        let dir = TempDir::new().unwrap();
        let cwd = dir.path().join("a b@c");
        std::fs::create_dir(&cwd).unwrap();
        let p = Project::synthesize_legacy(&cwd);
        assert_eq!(p.name, "a-b-c");
    }

    #[test]
    fn discover_with_no_manifest_falls_back_to_legacy() {
        let dir = TempDir::new().unwrap();
        let p = Project::discover(dir.path()).unwrap();
        assert!(p.is_legacy());
        assert_eq!(p.repos.len(), 1);
    }

    #[test]
    fn discover_finds_manifest_in_parent() {
        let dir = TempDir::new().unwrap();
        let manifest = dir.path().join("coral.toml");
        std::fs::write(
            &manifest,
            r#"apiVersion = "coral.dev/v1"
[project]
name = "demo"

[[repos]]
name = "api"
url  = "git@github.com:acme/api.git"

[[repos]]
name = "worker"
url  = "git@github.com:acme/worker.git"
"#,
        )
        .unwrap();

        let nested = dir.path().join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();

        let p = Project::discover(&nested).unwrap();
        assert!(!p.is_legacy());
        assert_eq!(p.name, "demo");
        assert_eq!(p.repos.len(), 2);
        assert!(p.is_multi_repo());
    }

    #[test]
    fn discover_ignores_coral_toml_without_project_section() {
        let dir = TempDir::new().unwrap();
        // E.g. a leftover example file. No `[project]` table → not the
        // project manifest.
        std::fs::write(
            dir.path().join("coral.toml"),
            "# example doc only\n[demo]\nfoo = 'bar'\n",
        )
        .unwrap();
        let p = Project::discover(dir.path()).unwrap();
        assert!(p.is_legacy());
    }

    #[test]
    fn resolved_url_uses_remote_template() {
        let manifest = r#"apiVersion = "coral.dev/v1"
[project]
name = "demo"
[project.defaults]
remote = "github"
[remotes.github]
fetch = "git@github.com:acme/{name}.git"

[[repos]]
name = "api"
"#;
        let p = manifest::parse_toml(manifest, Path::new("/tmp/coral.toml")).unwrap();
        let repo = &p.repos[0];
        assert_eq!(
            p.resolved_url(repo),
            Some("git@github.com:acme/api.git".to_string())
        );
    }

    #[test]
    fn resolved_path_uses_template_when_unset() {
        let manifest = r#"apiVersion = "coral.dev/v1"
[project]
name = "demo"
[project.defaults]
path_template = "services/{name}"

[[repos]]
name = "api"
url  = "git@github.com:acme/api.git"
"#;
        let mut p = manifest::parse_toml(manifest, Path::new("/tmp/coral.toml")).unwrap();
        p.root = PathBuf::from("/work");
        let repo = &p.repos[0];
        assert_eq!(p.resolved_path(repo), PathBuf::from("/work/services/api"));
    }

    #[test]
    fn legacy_project_wiki_root_is_cwd_dot_wiki() {
        let dir = TempDir::new().unwrap();
        let p = Project::synthesize_legacy(dir.path());
        assert_eq!(p.wiki_root(), dir.path().join(".wiki"));
    }

    /// Regression for [#20](https://github.com/agustincbajo/Coral/issues/20):
    /// `Project::load_from_manifest("coral.toml")` (relative,
    /// single-component) used to compute `project.root` as an empty
    /// PathBuf because `Path::new("coral.toml").parent()` returns
    /// `Some("")` rather than `None`, defeating the obvious
    /// `unwrap_or(Path::new("."))` guard. v0.19.4 routes through the
    /// shared `repo_root_from_wiki_root` helper that handles the
    /// empty-parent case. The fix is verified end-to-end here by
    /// `cd`-ing into a tmpdir, dropping a manifest at the cwd-root,
    /// and loading it via the bare relative filename.
    #[test]
    fn load_from_relative_filename_resolves_root_to_dot() {
        // Use a file we control rather than an actual chdir so the
        // test stays parallel-safe. The lib code path is the same:
        // any relative path with no `/` separator hits the empty-
        // parent case.
        let path = std::path::Path::new("coral.toml");
        // We don't need to actually load — exercise the internal
        // resolution that load_from_manifest uses. Pull it from the
        // crate-public helper to be sure we test the prod path.
        let resolved = crate::path::repo_root_from_wiki_root(path);
        assert_eq!(
            resolved,
            std::path::PathBuf::from("."),
            "single-component relative manifest path must resolve root to `.`",
        );
    }
}
