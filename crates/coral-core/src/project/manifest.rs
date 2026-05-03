//! `coral.toml` schema and parser.
//!
//! Schema version is encoded as `apiVersion = "coral.dev/v1"` (k8s-style)
//! to allow forward-compatible migration. v0.16 only accepts
//! `coral.dev/v1`; future versions will hard-fail with an actionable
//! error pointing at `coral migrate`.

use crate::error::{CoralError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const CURRENT_API_VERSION: &str = "coral.dev/v1";
pub const DEFAULT_PATH_TEMPLATE: &str = "repos/{name}";
pub const DEFAULT_REF: &str = "main";

/// The on-disk manifest model. Field naming is deliberate: it matches
/// `coral.toml` 1:1 so users reading the source see the same names they
/// type. Helpers like `Project::resolved_url` and `Project::resolved_path`
/// apply the project's defaults to a `RepoEntry`.
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub api_version: String,
    pub name: String,
    pub wiki_layout: WikiLayout,
    pub toolchain: Toolchain,
    pub defaults: ProjectDefaults,
    pub remotes: BTreeMap<String, RemoteSpec>,
    pub repos: Vec<RepoEntry>,

    /// Absolute path of the directory containing `coral.toml`. Set at
    /// load time, **not** in the manifest itself. Empty for legacy
    /// (synthesized) projects — use `Project::is_legacy()` to detect.
    pub root: PathBuf,
    /// Absolute path of the `coral.toml` file. Empty for legacy
    /// projects.
    pub manifest_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WikiLayout {
    /// One `.wiki/` at the meta-repo root, slugs namespaced as
    /// `<repo>/<slug>` when more than one repo is present. Default.
    #[default]
    Aggregated,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Toolchain {
    /// Pinned Coral version, like `.coral-pins.toml` but at the project
    /// level. Allows reproducibility cross-team. None means "no pin".
    pub coral: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectDefaults {
    pub r#ref: String,
    pub remote: Option<String>,
    pub path_template: String,
}

impl Default for ProjectDefaults {
    fn default() -> Self {
        Self {
            r#ref: DEFAULT_REF.to_string(),
            remote: None,
            path_template: DEFAULT_PATH_TEMPLATE.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteSpec {
    /// URL template; `{name}` is substituted with the repo's `name` when
    /// no explicit `url` is given on the repo entry.
    pub fetch: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RepoEntry {
    pub name: String,
    /// Explicit git URL. When `None`, resolved from `defaults.remote` +
    /// the project's `[remotes.<name>]` template.
    pub url: Option<String>,
    /// Override of `defaults.remote` for this repo only.
    pub remote: Option<String>,
    /// Override of `defaults.ref` for this repo only.
    pub r#ref: Option<String>,
    /// Override of `defaults.path_template` for this repo only.
    /// Relative paths are resolved against `Project.root`.
    pub path: Option<PathBuf>,
    pub tags: Vec<String>,
    /// Implicit cross-repo dependencies. Used by `--affected` to walk
    /// the DFS and include downstream consumers.
    pub depends_on: Vec<String>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub enabled: bool,
}

impl Project {
    /// Construct a single-repo legacy project. Used by the synthesis
    /// path; tests also use it directly.
    pub fn single_repo(name: String, root: PathBuf) -> Self {
        let repo = RepoEntry {
            name: name.clone(),
            url: None,
            remote: None,
            r#ref: None,
            path: Some(PathBuf::from(".")),
            tags: Vec::new(),
            depends_on: Vec::new(),
            include: Vec::new(),
            exclude: Vec::new(),
            enabled: true,
        };
        Self {
            api_version: CURRENT_API_VERSION.to_string(),
            name,
            wiki_layout: WikiLayout::Aggregated,
            toolchain: Toolchain::default(),
            defaults: ProjectDefaults::default(),
            remotes: BTreeMap::new(),
            repos: vec![repo],
            root,
            manifest_path: PathBuf::new(),
        }
    }

    /// Validate post-parse invariants. Run on every `load_from_manifest`.
    pub fn validate(&self) -> Result<()> {
        if self.api_version != CURRENT_API_VERSION {
            return Err(CoralError::Walk(format!(
                "unsupported apiVersion '{}': this binary supports '{}'. Run `coral migrate` for upgrade guidance.",
                self.api_version, CURRENT_API_VERSION
            )));
        }
        if self.name.trim().is_empty() {
            return Err(CoralError::Walk(
                "project.name must not be empty".to_string(),
            ));
        }
        // Repo name uniqueness — slugs would clash in the aggregated
        // wiki without it.
        let mut seen = std::collections::BTreeSet::new();
        for repo in &self.repos {
            if !seen.insert(&repo.name) {
                return Err(CoralError::Walk(format!(
                    "duplicate repo name '{}' in coral.toml",
                    repo.name
                )));
            }
            if repo.url.is_none() && repo.remote.is_none() && self.defaults.remote.is_none() {
                // The legacy single-repo case has `path = "."`. Allow
                // it; the user hasn't asked Coral to resolve a URL.
                let is_inplace = repo
                    .path
                    .as_ref()
                    .map(|p| p == Path::new("."))
                    .unwrap_or(false);
                if !is_inplace {
                    return Err(CoralError::Walk(format!(
                        "repo '{}' has no `url`, no `remote`, and no `defaults.remote` — cannot resolve git URL",
                        repo.name
                    )));
                }
            }
        }
        // Cycle detection on depends_on. DFS with three-color marking.
        let names: Vec<&str> = self.repos.iter().map(|r| r.name.as_str()).collect();
        for repo in &self.repos {
            for dep in &repo.depends_on {
                if !names.contains(&dep.as_str()) {
                    return Err(CoralError::Walk(format!(
                        "repo '{}' depends_on '{}' which is not declared in coral.toml",
                        repo.name, dep
                    )));
                }
            }
        }
        if has_cycle(&self.repos) {
            return Err(CoralError::Walk(
                "depends_on cycle detected among repos".to_string(),
            ));
        }
        Ok(())
    }
}

fn has_cycle(repos: &[RepoEntry]) -> bool {
    use std::collections::HashMap;
    let by_name: HashMap<&str, &RepoEntry> = repos.iter().map(|r| (r.name.as_str(), r)).collect();
    enum Color {
        White,
        Gray,
        Black,
    }
    let mut color: HashMap<&str, Color> = repos
        .iter()
        .map(|r| (r.name.as_str(), Color::White))
        .collect();

    fn dfs<'a>(
        node: &'a str,
        by_name: &HashMap<&'a str, &'a RepoEntry>,
        color: &mut HashMap<&'a str, Color>,
    ) -> bool {
        let entry = match by_name.get(node) {
            Some(e) => *e,
            None => return false,
        };
        color.insert(node, Color::Gray);
        for dep in &entry.depends_on {
            match color.get(dep.as_str()) {
                Some(Color::Gray) => return true,
                Some(Color::White) | None => {
                    if dfs(dep.as_str(), by_name, color) {
                        return true;
                    }
                }
                Some(Color::Black) => {}
            }
        }
        color.insert(node, Color::Black);
        false
    }

    let names: Vec<&str> = repos.iter().map(|r| r.name.as_str()).collect();
    for n in names {
        if matches!(color.get(n), Some(Color::White) | None) && dfs(n, &by_name, &mut color) {
            return true;
        }
    }
    false
}

// ---- Raw on-disk shape (TOML) ---------------------------------------
//
// We deserialize into a private `Raw*` struct family and then map to the
// canonical `Project` shape. This keeps the public type stable when we
// iterate the on-disk schema.

#[derive(Debug, Deserialize)]
struct RawRoot {
    #[serde(rename = "apiVersion")]
    api_version: Option<String>,
    project: RawProject,
    #[serde(default)]
    remotes: BTreeMap<String, RemoteSpec>,
    #[serde(default, rename = "repos")]
    repos: Vec<RawRepo>,
}

#[derive(Debug, Deserialize)]
struct RawProject {
    name: String,
    #[serde(default)]
    wiki_layout: Option<WikiLayout>,
    #[serde(default)]
    toolchain: Option<RawToolchain>,
    #[serde(default)]
    defaults: Option<RawDefaults>,
}

#[derive(Debug, Deserialize)]
struct RawToolchain {
    coral: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawDefaults {
    #[serde(rename = "ref")]
    ref_: Option<String>,
    remote: Option<String>,
    path_template: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawRepo {
    name: String,
    url: Option<String>,
    remote: Option<String>,
    #[serde(rename = "ref")]
    ref_: Option<String>,
    path: Option<PathBuf>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    include: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default = "default_true")]
    enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Parse the raw bytes of a `coral.toml`. Does NOT set `root` or
/// `manifest_path` — the loader fills those in.
pub fn parse_toml(raw: &str, manifest_path: &Path) -> Result<Project> {
    let parsed: RawRoot = toml::from_str(raw).map_err(|e| {
        CoralError::Walk(format!(
            "failed to parse {}: {}",
            manifest_path.display(),
            e
        ))
    })?;

    let api_version = parsed
        .api_version
        .unwrap_or_else(|| CURRENT_API_VERSION.to_string());
    let defaults = parsed
        .project
        .defaults
        .map(map_defaults)
        .unwrap_or_default();
    let toolchain = parsed
        .project
        .toolchain
        .map(map_toolchain)
        .unwrap_or_default();

    let repos = parsed.repos.into_iter().map(map_repo).collect();

    let project = Project {
        api_version,
        name: parsed.project.name,
        wiki_layout: parsed.project.wiki_layout.unwrap_or_default(),
        toolchain,
        defaults,
        remotes: parsed.remotes,
        repos,
        root: PathBuf::new(),
        manifest_path: manifest_path.to_path_buf(),
    };
    Ok(project)
}

fn map_defaults(raw: RawDefaults) -> ProjectDefaults {
    ProjectDefaults {
        r#ref: raw.ref_.unwrap_or_else(|| DEFAULT_REF.to_string()),
        remote: raw.remote,
        path_template: raw
            .path_template
            .unwrap_or_else(|| DEFAULT_PATH_TEMPLATE.to_string()),
    }
}

fn map_toolchain(raw: RawToolchain) -> Toolchain {
    Toolchain { coral: raw.coral }
}

fn map_repo(raw: RawRepo) -> RepoEntry {
    RepoEntry {
        name: raw.name,
        url: raw.url,
        remote: raw.remote,
        r#ref: raw.ref_,
        path: raw.path,
        tags: raw.tags,
        depends_on: raw.depends_on,
        include: raw.include,
        exclude: raw.exclude,
        enabled: raw.enabled,
    }
}

/// Serialize a `Project` back to canonical TOML. Used by `coral project new`
/// and `coral project add`. Output is human-curated-friendly: blank lines
/// between sections, no comments (those would be lost on round-trip).
pub fn render_toml(project: &Project) -> String {
    let mut out = String::new();
    out.push_str(&format!("apiVersion = \"{}\"\n\n", project.api_version));
    out.push_str("[project]\n");
    out.push_str(&format!("name = \"{}\"\n", project.name));
    if !matches!(project.wiki_layout, WikiLayout::Aggregated) {
        // Reserved for future layouts; never emitted today.
    }
    if let Some(coral) = &project.toolchain.coral {
        out.push_str("\n[project.toolchain]\n");
        out.push_str(&format!("coral = \"{}\"\n", coral));
    }
    if project.defaults != ProjectDefaults::default() {
        out.push_str("\n[project.defaults]\n");
        if project.defaults.r#ref != DEFAULT_REF {
            out.push_str(&format!("ref = \"{}\"\n", project.defaults.r#ref));
        }
        if let Some(remote) = &project.defaults.remote {
            out.push_str(&format!("remote = \"{}\"\n", remote));
        }
        if project.defaults.path_template != DEFAULT_PATH_TEMPLATE {
            out.push_str(&format!(
                "path_template = \"{}\"\n",
                project.defaults.path_template
            ));
        }
    }
    for (name, spec) in &project.remotes {
        out.push_str(&format!("\n[remotes.{}]\n", name));
        out.push_str(&format!("fetch = \"{}\"\n", spec.fetch));
    }
    for repo in &project.repos {
        out.push_str("\n[[repos]]\n");
        out.push_str(&format!("name = \"{}\"\n", repo.name));
        if let Some(url) = &repo.url {
            out.push_str(&format!("url = \"{}\"\n", url));
        }
        if let Some(remote) = &repo.remote {
            out.push_str(&format!("remote = \"{}\"\n", remote));
        }
        if let Some(r) = &repo.r#ref {
            out.push_str(&format!("ref = \"{}\"\n", r));
        }
        if let Some(path) = &repo.path {
            out.push_str(&format!("path = \"{}\"\n", path.display()));
        }
        if !repo.tags.is_empty() {
            out.push_str(&format!(
                "tags = [{}]\n",
                repo.tags
                    .iter()
                    .map(|t| format!("\"{}\"", t))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !repo.depends_on.is_empty() {
            out.push_str(&format!(
                "depends_on = [{}]\n",
                repo.depends_on
                    .iter()
                    .map(|d| format!("\"{}\"", d))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !repo.include.is_empty() {
            out.push_str(&format!(
                "include = [{}]\n",
                repo.include
                    .iter()
                    .map(|d| format!("\"{}\"", d))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !repo.exclude.is_empty() {
            out.push_str(&format!(
                "exclude = [{}]\n",
                repo.exclude
                    .iter()
                    .map(|d| format!("\"{}\"", d))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !repo.enabled {
            out.push_str("enabled = false\n");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_project_with_repos(names: &[&str]) -> Project {
        let mut repos = Vec::new();
        for n in names {
            repos.push(RepoEntry {
                name: n.to_string(),
                url: Some(format!("git@example.com/{}.git", n)),
                remote: None,
                r#ref: None,
                path: None,
                tags: Vec::new(),
                depends_on: Vec::new(),
                include: Vec::new(),
                exclude: Vec::new(),
                enabled: true,
            });
        }
        Project {
            api_version: CURRENT_API_VERSION.to_string(),
            name: "demo".to_string(),
            wiki_layout: WikiLayout::Aggregated,
            toolchain: Toolchain::default(),
            defaults: ProjectDefaults::default(),
            remotes: BTreeMap::new(),
            repos,
            root: PathBuf::new(),
            manifest_path: PathBuf::new(),
        }
    }

    #[test]
    fn parse_minimal_manifest() {
        let raw = r#"apiVersion = "coral.dev/v1"
[project]
name = "demo"

[[repos]]
name = "api"
url = "git@github.com:acme/api.git"
"#;
        let p = parse_toml(raw, Path::new("/tmp/coral.toml")).unwrap();
        assert_eq!(p.api_version, CURRENT_API_VERSION);
        assert_eq!(p.name, "demo");
        assert_eq!(p.repos.len(), 1);
        assert_eq!(p.repos[0].name, "api");
        assert!(p.repos[0].enabled);
    }

    #[test]
    fn parse_rejects_unknown_api_version() {
        let raw = r#"apiVersion = "coral.dev/v999"
[project]
name = "demo"

[[repos]]
name = "api"
url = "git@github.com:acme/api.git"
"#;
        let p = parse_toml(raw, Path::new("/tmp/coral.toml")).unwrap();
        let err = p.validate().unwrap_err();
        assert!(format!("{}", err).contains("apiVersion"));
    }

    #[test]
    fn parse_full_manifest_with_defaults_and_remotes() {
        let raw = r#"apiVersion = "coral.dev/v1"
[project]
name = "orchestra"

[project.toolchain]
coral = "0.16.0"

[project.defaults]
ref = "main"
remote = "github"
path_template = "services/{name}"

[remotes.github]
fetch = "git@github.com:acme/{name}.git"

[[repos]]
name = "api"
ref = "release/v3"
tags = ["service", "team:platform"]

[[repos]]
name = "worker"
depends_on = ["api"]
"#;
        let p = parse_toml(raw, Path::new("/tmp/coral.toml")).unwrap();
        assert_eq!(p.toolchain.coral.as_deref(), Some("0.16.0"));
        assert_eq!(p.defaults.path_template, "services/{name}");
        assert_eq!(
            p.remotes.get("github").unwrap().fetch,
            "git@github.com:acme/{name}.git"
        );
        assert_eq!(p.repos[1].depends_on, vec!["api".to_string()]);
        assert_eq!(p.repos[0].tags, vec!["service", "team:platform"]);
    }

    #[test]
    fn validate_rejects_duplicate_repo_names() {
        let mut p = make_project_with_repos(&["api", "api"]);
        p.root = PathBuf::from("/work");
        let err = p.validate().unwrap_err();
        assert!(format!("{}", err).contains("duplicate repo name"));
    }

    #[test]
    fn validate_rejects_unknown_dependency() {
        let mut p = make_project_with_repos(&["api"]);
        p.repos[0].depends_on.push("missing".to_string());
        let err = p.validate().unwrap_err();
        assert!(format!("{}", err).contains("not declared"));
    }

    #[test]
    fn validate_rejects_dependency_cycle() {
        let mut p = make_project_with_repos(&["a", "b"]);
        p.repos[0].depends_on.push("b".to_string());
        p.repos[1].depends_on.push("a".to_string());
        let err = p.validate().unwrap_err();
        assert!(format!("{}", err).contains("cycle"));
    }

    #[test]
    fn validate_rejects_missing_url_without_remote() {
        let raw = r#"apiVersion = "coral.dev/v1"
[project]
name = "demo"

[[repos]]
name = "api"
"#;
        let p = parse_toml(raw, Path::new("/tmp/coral.toml")).unwrap();
        let err = p.validate().unwrap_err();
        assert!(format!("{}", err).contains("cannot resolve git URL"));
    }

    #[test]
    fn render_roundtrip_preserves_essentials() {
        let raw = r#"apiVersion = "coral.dev/v1"
[project]
name = "orchestra"

[project.defaults]
remote = "github"

[remotes.github]
fetch = "git@github.com:acme/{name}.git"

[[repos]]
name = "api"
tags = ["service"]

[[repos]]
name = "worker"
depends_on = ["api"]
"#;
        let p = parse_toml(raw, Path::new("/tmp/coral.toml")).unwrap();
        let rendered = render_toml(&p);
        let p2 = parse_toml(&rendered, Path::new("/tmp/coral.toml")).unwrap();
        assert_eq!(p2.name, p.name);
        assert_eq!(p2.repos.len(), p.repos.len());
        assert_eq!(p2.repos[0].name, "api");
        assert_eq!(p2.repos[0].tags, vec!["service"]);
        assert_eq!(p2.repos[1].depends_on, vec!["api".to_string()]);
        assert_eq!(p2.defaults.remote.as_deref(), Some("github"));
        assert_eq!(p2.remotes.get("github"), p.remotes.get("github"));
    }
}
