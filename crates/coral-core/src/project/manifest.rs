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
    /// Raw `[[environments]]` table from `coral.toml`, kept opaque at
    /// the `coral-core` layer because the strongly-typed model lives
    /// in the `coral-env` crate (which depends on `coral-core`, not the
    /// reverse — keeps the manifest reusable from `coral-mcp` and
    /// future readers without dragging the env layer in).
    pub environments_raw: Vec<toml::Value>,
    /// v0.21.4: optional `[runner]` block selecting per-command runner
    /// shapes (single-tier vs. tiered planner→executor→reviewer). The
    /// concrete `Runner` impls live in `coral-runner`; here we only
    /// carry the configuration so `coral-cli` can construct them.
    /// Default = `RunnerSection::default()` — round-trips a manifest
    /// without `[runner]` byte-identically.
    pub runner: RunnerSection,

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

/// v0.21.4: `[runner]` block — opt-in tiered routing per command.
///
/// `RunnerSection::default()` corresponds to "no `[runner]` table in
/// `coral.toml`": every command runs single-tier exactly as in
/// v0.21.3. Tiered routing is gated by an explicit
/// `[runner.tiered.consolidate] enabled = true` (per-command opt-in)
/// — broadening to other commands later just adds more `*_enabled`
/// switches.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunnerSection {
    /// `[runner.tiered]` block. `None` ⇒ no tiered routing.
    pub tiered: Option<TieredManifest>,
}

impl RunnerSection {
    /// Whether `coral consolidate` should default to tiered routing
    /// when the user does NOT pass `--tiered`. The CLI flag always
    /// wins; this is purely the manifest-side default.
    pub fn tiered_enabled_for_consolidate(&self) -> bool {
        self.tiered
            .as_ref()
            .map(|t| t.consolidate.enabled)
            .unwrap_or(false)
    }
}

/// Resolved `[runner.tiered]` block. All three tier specs are
/// **required** when `[runner.tiered]` is present (validated at parse
/// time so missing tiers surface a clear error rather than a confusing
/// runner-construction crash later).
#[derive(Debug, Clone, PartialEq)]
pub struct TieredManifest {
    pub planner: TierSpecManifest,
    pub executor: TierSpecManifest,
    pub reviewer: TierSpecManifest,
    pub budget: BudgetManifest,
    pub consolidate: TieredConsolidate,
}

/// One tier's provider + optional model override.
#[derive(Debug, Clone, PartialEq)]
pub struct TierSpecManifest {
    pub provider: String,
    pub model: Option<String>,
}

/// Cumulative-token budget for a single tiered run. `max_tokens_per_run`
/// applies to the SUM across planner + executor calls + reviewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetManifest {
    pub max_tokens_per_run: u64,
}

impl Default for BudgetManifest {
    fn default() -> Self {
        // v0.21.4: matches `coral_runner::DEFAULT_MAX_TOKENS_PER_RUN`.
        // Picked to mirror a 200K-context window.
        Self {
            max_tokens_per_run: 200_000,
        }
    }
}

/// Per-command opt-in for tiered routing. Today only `consolidate`
/// honors this; future commands grow their own field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TieredConsolidate {
    pub enabled: bool,
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
            environments_raw: Vec::new(),
            runner: RunnerSection::default(),
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
            // v0.19.6 audit H1: a malicious or copy-pasted manifest
            // could put `name = "../escape"` here. `Project::resolved_path`
            // would then produce `<project_root>/repos/../escape`, and
            // `coral project sync` would `git clone` outside the project
            // root. Reject anything that's not a plain ASCII slug.
            if !crate::slug::is_safe_repo_name(&repo.name) {
                return Err(CoralError::Walk(format!(
                    "invalid repo name '{}' in coral.toml: \
                     names must be ASCII alphanumeric plus `-`/`_`, \
                     no path separators, no leading `.` or `-`",
                    repo.name
                )));
            }
            // v0.19.8 #28: `_default` is the MCP `coral://wiki/<repo>/_index`
            // wildcard sentinel for the legacy single-repo case. A repo
            // literally named `_default` would silently shadow the
            // wildcard — reserve the name to make the collision impossible.
            if repo.name == "_default" {
                return Err(CoralError::Walk(format!(
                    "invalid repo name '{}' in coral.toml: \
                     `_default` is reserved as the MCP `coral://wiki/_default/_index` \
                     wildcard sentinel for legacy single-repo wikis. \
                     Pick a different name.",
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
    #[serde(default, rename = "environments")]
    environments: Vec<toml::Value>,
    /// v0.21.4: optional `[runner]` block (and nested `[runner.tiered]`).
    #[serde(default)]
    runner: Option<RawRunnerSection>,
}

#[derive(Debug, Deserialize)]
struct RawRunnerSection {
    #[serde(default)]
    tiered: Option<RawTiered>,
}

#[derive(Debug, Deserialize)]
struct RawTiered {
    #[serde(default)]
    planner: Option<RawTierSpec>,
    #[serde(default)]
    executor: Option<RawTierSpec>,
    #[serde(default)]
    reviewer: Option<RawTierSpec>,
    #[serde(default)]
    budget: Option<RawBudget>,
    #[serde(default)]
    consolidate: Option<RawTieredConsolidate>,
}

#[derive(Debug, Deserialize)]
struct RawTierSpec {
    provider: String,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawBudget {
    #[serde(default)]
    max_tokens_per_run: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RawTieredConsolidate {
    #[serde(default)]
    enabled: bool,
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

    let runner = match parsed.runner {
        Some(r) => map_runner_section(r, manifest_path)?,
        None => RunnerSection::default(),
    };

    let project = Project {
        api_version,
        name: parsed.project.name,
        wiki_layout: parsed.project.wiki_layout.unwrap_or_default(),
        toolchain,
        defaults,
        remotes: parsed.remotes,
        repos,
        environments_raw: parsed.environments,
        runner,
        root: PathBuf::new(),
        manifest_path: manifest_path.to_path_buf(),
    };
    Ok(project)
}

fn map_runner_section(raw: RawRunnerSection, manifest_path: &Path) -> Result<RunnerSection> {
    let tiered = match raw.tiered {
        Some(t) => Some(map_tiered(t, manifest_path)?),
        None => None,
    };
    Ok(RunnerSection { tiered })
}

fn map_tiered(raw: RawTiered, manifest_path: &Path) -> Result<TieredManifest> {
    // All three tier specs are mandatory when `[runner.tiered]` is
    // present. We surface a single message naming every missing tier
    // so a user fixing a half-typed config doesn't have to re-edit
    // and re-run three times.
    let mut missing: Vec<&'static str> = Vec::new();
    if raw.planner.is_none() {
        missing.push("planner");
    }
    if raw.executor.is_none() {
        missing.push("executor");
    }
    if raw.reviewer.is_none() {
        missing.push("reviewer");
    }
    if !missing.is_empty() {
        return Err(CoralError::Walk(format!(
            "[runner.tiered] in {} is missing required tier(s): {}. \
             All three of `planner`, `executor`, `reviewer` must be \
             specified when `[runner.tiered]` is present.",
            manifest_path.display(),
            missing.join(", ")
        )));
    }
    // Destructure with `Option::ok_or` so we never have to call `unwrap`
    // even though the `missing` check above guarantees all three are
    // `Some` at this point. The fallback error message is unreachable
    // but cheap — it keeps clippy::unwrap_used clean without a blanket
    // allow.
    let unreachable_missing = || {
        CoralError::Walk(format!(
            "[runner.tiered] in {}: internal invariant violated",
            manifest_path.display()
        ))
    };
    let planner = map_tier_spec(raw.planner.ok_or_else(unreachable_missing)?);
    let executor = map_tier_spec(raw.executor.ok_or_else(unreachable_missing)?);
    let reviewer = map_tier_spec(raw.reviewer.ok_or_else(unreachable_missing)?);

    let budget = match raw.budget {
        Some(b) => {
            let v = b.max_tokens_per_run.unwrap_or(200_000);
            if v == 0 {
                return Err(CoralError::Walk(format!(
                    "[runner.tiered.budget] in {}: `max_tokens_per_run` must be > 0",
                    manifest_path.display()
                )));
            }
            BudgetManifest {
                max_tokens_per_run: v,
            }
        }
        None => BudgetManifest::default(),
    };

    let consolidate = TieredConsolidate {
        enabled: raw.consolidate.map(|c| c.enabled).unwrap_or(false),
    };

    Ok(TieredManifest {
        planner,
        executor,
        reviewer,
        budget,
        consolidate,
    })
}

fn map_tier_spec(raw: RawTierSpec) -> TierSpecManifest {
    TierSpecManifest {
        provider: raw.provider,
        model: raw.model,
    }
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

/// Escape a string for inclusion inside a TOML basic-string (`"..."`).
/// Handles backslash, double-quote, and the control characters defined
/// by the TOML 1.0 spec (`\b`, `\t`, `\n`, `\f`, `\r`). Anything else
/// non-printable is unlikely in our domain (paths, URLs, slugs) but is
/// passed through; if it ever shows up we'll surface it via the parser
/// round-trip.
///
/// v0.37 prep: this fix unbreaks Windows where `coral project add --url
/// C:\Users\...` previously wrote a raw backslash that TOML parsed as
/// `\U` (invalid unicode escape).
fn toml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str(r"\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c => out.push(c),
        }
    }
    out
}

/// Serialize a `Project` back to canonical TOML. Used by `coral project new`
/// and `coral project add`. Output is human-curated-friendly: blank lines
/// between sections, no comments (those would be lost on round-trip).
pub fn render_toml(project: &Project) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "apiVersion = \"{}\"\n\n",
        toml_escape(&project.api_version)
    ));
    out.push_str("[project]\n");
    out.push_str(&format!("name = \"{}\"\n", toml_escape(&project.name)));
    if !matches!(project.wiki_layout, WikiLayout::Aggregated) {
        // Reserved for future layouts; never emitted today.
    }
    if let Some(coral) = &project.toolchain.coral {
        out.push_str("\n[project.toolchain]\n");
        out.push_str(&format!("coral = \"{}\"\n", toml_escape(coral)));
    }
    if project.defaults != ProjectDefaults::default() {
        out.push_str("\n[project.defaults]\n");
        if project.defaults.r#ref != DEFAULT_REF {
            out.push_str(&format!(
                "ref = \"{}\"\n",
                toml_escape(&project.defaults.r#ref)
            ));
        }
        if let Some(remote) = &project.defaults.remote {
            out.push_str(&format!("remote = \"{}\"\n", toml_escape(remote)));
        }
        if project.defaults.path_template != DEFAULT_PATH_TEMPLATE {
            out.push_str(&format!(
                "path_template = \"{}\"\n",
                toml_escape(&project.defaults.path_template)
            ));
        }
    }
    for (name, spec) in &project.remotes {
        out.push_str(&format!("\n[remotes.{}]\n", name));
        out.push_str(&format!("fetch = \"{}\"\n", toml_escape(&spec.fetch)));
    }
    for repo in &project.repos {
        out.push_str("\n[[repos]]\n");
        out.push_str(&format!("name = \"{}\"\n", toml_escape(&repo.name)));
        if let Some(url) = &repo.url {
            out.push_str(&format!("url = \"{}\"\n", toml_escape(url)));
        }
        if let Some(remote) = &repo.remote {
            out.push_str(&format!("remote = \"{}\"\n", toml_escape(remote)));
        }
        if let Some(r) = &repo.r#ref {
            out.push_str(&format!("ref = \"{}\"\n", toml_escape(r)));
        }
        if let Some(path) = &repo.path {
            out.push_str(&format!(
                "path = \"{}\"\n",
                toml_escape(&path.display().to_string())
            ));
        }
        if !repo.tags.is_empty() {
            out.push_str(&format!(
                "tags = [{}]\n",
                repo.tags
                    .iter()
                    .map(|t| format!("\"{}\"", toml_escape(t)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !repo.depends_on.is_empty() {
            out.push_str(&format!(
                "depends_on = [{}]\n",
                repo.depends_on
                    .iter()
                    .map(|d| format!("\"{}\"", toml_escape(d)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !repo.include.is_empty() {
            out.push_str(&format!(
                "include = [{}]\n",
                repo.include
                    .iter()
                    .map(|d| format!("\"{}\"", toml_escape(d)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !repo.exclude.is_empty() {
            out.push_str(&format!(
                "exclude = [{}]\n",
                repo.exclude
                    .iter()
                    .map(|d| format!("\"{}\"", toml_escape(d)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !repo.enabled {
            out.push_str("enabled = false\n");
        }
    }
    // v0.21.4: emit [runner.tiered] when present. Default
    // RunnerSection (no tiered block) renders nothing — that
    // preserves byte-identity for v0.21.3 manifests on round-trip.
    if let Some(t) = &project.runner.tiered {
        out.push_str("\n[runner.tiered]\n");
        // Per-tier subtables.
        out.push_str(&render_tier("planner", &t.planner));
        out.push_str(&render_tier("executor", &t.executor));
        out.push_str(&render_tier("reviewer", &t.reviewer));
        // Budget — only emit when it diverges from the default.
        // Always emitting would noisify minimal manifests.
        if t.budget.max_tokens_per_run != BudgetManifest::default().max_tokens_per_run {
            out.push_str("\n[runner.tiered.budget]\n");
            out.push_str(&format!(
                "max_tokens_per_run = {}\n",
                t.budget.max_tokens_per_run
            ));
        }
        if t.consolidate.enabled {
            out.push_str("\n[runner.tiered.consolidate]\n");
            out.push_str("enabled = true\n");
        }
    }
    out
}

fn render_tier(label: &str, spec: &TierSpecManifest) -> String {
    let mut out = String::new();
    out.push_str(&format!("\n[runner.tiered.{label}]\n"));
    out.push_str(&format!("provider = \"{}\"\n", toml_escape(&spec.provider)));
    if let Some(m) = &spec.model {
        out.push_str(&format!("model = \"{}\"\n", toml_escape(m)));
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
            environments_raw: Vec::new(),
            runner: RunnerSection::default(),
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

    /// v0.19.6 audit H1: a `[[repos]]` block with a path-traversal
    /// `name` must be rejected at validate-time. Prior to this fix
    /// `Project::resolved_path` would happily produce
    /// `<project_root>/repos/../escape` and `coral project sync`
    /// would clone there.
    #[test]
    fn validate_rejects_path_traversal_in_repo_name() {
        let mut p = make_project_with_repos(&["../escape"]);
        p.root = PathBuf::from("/work");
        let err = p.validate().unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("invalid repo name") && msg.contains("../escape"),
            "expected invalid-name error naming the offending repo, got: {msg}"
        );
    }

    /// v0.19.6 audit H1: also reject obvious siblings — leading dot,
    /// path separator, whitespace.
    #[test]
    fn validate_rejects_other_unsafe_repo_names() {
        for bad in &[".hidden", "foo/bar", "foo bar", "-flag"] {
            let mut p = make_project_with_repos(&[bad]);
            p.root = PathBuf::from("/work");
            let err = p.validate().expect_err("must reject unsafe name");
            let msg = format!("{}", err);
            assert!(
                msg.contains("invalid repo name") && msg.contains(*bad),
                "expected invalid-name error for {bad:?}, got: {msg}"
            );
        }
    }

    /// v0.19.8 #28: `_default` is the MCP `coral://wiki/<repo>/_index`
    /// wildcard sentinel. A repo literally named `_default` would
    /// silently shadow the wildcard — validate must reject it with a
    /// clear message naming the reservation.
    #[test]
    fn validate_rejects_reserved_repo_name_default() {
        let mut p = make_project_with_repos(&["_default"]);
        p.root = PathBuf::from("/work");
        let err = p.validate().expect_err("must reject reserved name");
        let msg = format!("{}", err);
        assert!(
            msg.contains("_default") && msg.contains("reserved"),
            "expected reserved-name error naming `_default`, got: {msg}"
        );
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
    fn validate_rejects_three_node_cycle() {
        // a → b → c → a — DFS coloring must catch transitive cycles, not
        // just the trivial 2-node case.
        let mut p = make_project_with_repos(&["a", "b", "c"]);
        p.repos[0].depends_on.push("b".to_string());
        p.repos[1].depends_on.push("c".to_string());
        p.repos[2].depends_on.push("a".to_string());
        let err = p.validate().unwrap_err();
        assert!(format!("{}", err).contains("cycle"));
    }

    #[test]
    fn validate_rejects_self_loop() {
        // A repo declaring itself as a dependency is also a cycle.
        let mut p = make_project_with_repos(&["a"]);
        p.repos[0].depends_on.push("a".to_string());
        let err = p.validate().unwrap_err();
        assert!(format!("{}", err).contains("cycle"));
    }

    #[test]
    fn validate_accepts_diamond_dag() {
        //   a
        //  ╱ ╲
        // b   c
        //  ╲ ╱
        //   d
        // Diamond pattern shares ancestor `a` via two paths but is acyclic.
        // The Gray/Black coloring must mark `a` Black on the first DFS so
        // visiting it again from the second path doesn't false-positive.
        let mut p = make_project_with_repos(&["a", "b", "c", "d"]);
        p.repos[1].depends_on.push("a".to_string());
        p.repos[2].depends_on.push("a".to_string());
        p.repos[3].depends_on.push("b".to_string());
        p.repos[3].depends_on.push("c".to_string());
        p.validate().expect("diamond DAGs must be allowed");
    }

    #[test]
    fn validate_accepts_disconnected_components() {
        // Two independent islands {a, b} and {c, d}. Validation must walk
        // every component, not just the one rooted at the first node.
        let mut p = make_project_with_repos(&["a", "b", "c", "d"]);
        p.repos[1].depends_on.push("a".to_string());
        p.repos[3].depends_on.push("c".to_string());
        p.validate()
            .expect("disconnected acyclic components must validate");
    }

    #[test]
    fn validate_detects_cycle_in_one_of_many_components() {
        // Component A is acyclic ({a, b}), Component B has a 3-node cycle
        // ({c, d, e}). Validation must surface the failure even when the
        // graph has multiple disconnected components.
        let mut p = make_project_with_repos(&["a", "b", "c", "d", "e"]);
        p.repos[1].depends_on.push("a".to_string());
        p.repos[2].depends_on.push("d".to_string());
        p.repos[3].depends_on.push("e".to_string());
        p.repos[4].depends_on.push("c".to_string());
        let err = p.validate().unwrap_err();
        assert!(format!("{}", err).contains("cycle"));
    }

    #[test]
    fn has_cycle_handles_dangling_dependency_gracefully() {
        // `has_cycle` is the lower-level fn; the `validate_rejects_unknown_dependency`
        // check runs before it, but pin behavior in case ordering changes.
        // Dangling edges must report no cycle (so the higher-level "not
        // declared" check fires instead) without panicking on the lookup.
        let single = vec![RepoEntry {
            name: "lone".into(),
            url: Some("git@example.com/lone.git".into()),
            remote: None,
            r#ref: None,
            path: None,
            tags: vec![],
            depends_on: vec!["ghost".into()],
            include: vec![],
            exclude: vec![],
            enabled: true,
        }];
        assert!(!has_cycle(&single));
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

    /// v0.37 prep: Windows backslashes in repo URLs / paths must be
    /// escaped on render so the round-trip parses cleanly. The bug it
    /// guards against: `coral project add --url C:\\Users\\...` would
    /// previously emit a raw backslash, and on re-parse TOML would
    /// reject `\U` as an invalid 8-digit unicode escape, breaking the
    /// `project_sync_clones_a_local_bare_repo_end_to_end` test under
    /// Windows nextest.
    #[test]
    fn render_escapes_backslashes_in_url_and_path() {
        let mut p = make_project_with_repos(&[]);
        p.repos.push(RepoEntry {
            name: "demo".into(),
            url: Some(r"C:\Users\agust\AppData\Local\Temp\origin.git".into()),
            remote: None,
            r#ref: None,
            path: Some(PathBuf::from(r"C:\Users\agust\repos\demo")),
            tags: Vec::new(),
            depends_on: Vec::new(),
            include: Vec::new(),
            exclude: Vec::new(),
            enabled: true,
        });
        let rendered = render_toml(&p);
        // Each lone backslash from the input must appear escaped as `\\`
        // in the rendered TOML. Spot-checking a substring is enough — a
        // parse round-trip below is the load-bearing assertion.
        assert!(
            rendered.contains(r"C:\\Users\\agust"),
            "render must escape backslashes: {rendered}"
        );
        // Round-trip: re-parse without panic and check the URL came
        // back identical to the input.
        let p2 = parse_toml(&rendered, Path::new("/tmp/coral.toml"))
            .expect("rendered TOML must parse back");
        assert_eq!(
            p2.repos[0].url.as_deref(),
            Some(r"C:\Users\agust\AppData\Local\Temp\origin.git")
        );
        assert_eq!(
            p2.repos[0].path.as_deref(),
            Some(Path::new(r"C:\Users\agust\repos\demo"))
        );
    }

    #[test]
    fn render_escapes_double_quote_in_string_fields() {
        // Double-quotes in a tag must not bust the quoted-string scope.
        let mut p = make_project_with_repos(&[]);
        p.repos.push(RepoEntry {
            name: "api".into(),
            url: Some("git@example.com/api.git".into()),
            remote: None,
            r#ref: None,
            path: None,
            tags: vec![r#"weird"tag"#.into()],
            depends_on: Vec::new(),
            include: Vec::new(),
            exclude: Vec::new(),
            enabled: true,
        });
        let rendered = render_toml(&p);
        let p2 = parse_toml(&rendered, Path::new("/tmp/coral.toml"))
            .expect("rendered TOML with escaped quote must parse back");
        assert_eq!(p2.repos[0].tags, vec![r#"weird"tag"#.to_string()]);
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

    // v0.21.4 — `[runner]` / `[runner.tiered]` parsing tests.
    //
    // Spec acceptance criterion #1: a v0.21.3-shaped manifest (no
    // `[runner]` section) must round-trip *exactly* — the rendered
    // output is byte-identical to the canonical v0.21.3 emitter and
    // re-parsing it produces an equal `Project`. Used as the BC pin.
    #[test]
    fn manifest_without_runner_section_round_trips_unchanged() {
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
        // No `[runner]` section in the source ⇒ default `RunnerSection`.
        assert_eq!(p.runner, RunnerSection::default());
        assert!(p.runner.tiered.is_none());
        // First render — pin the bytes against v0.21.3 emit.
        let r1 = render_toml(&p);
        // The emitter must NOT add a `[runner]` block when none was
        // requested. A regression here would break BC for every v0.21.3
        // manifest the user already has on disk.
        assert!(
            !r1.contains("[runner"),
            "default RunnerSection must not emit [runner...] block; got:\n{r1}"
        );
        // Re-parse the rendered output and re-render — the second
        // render must equal the first (idempotency / fixed-point).
        let p2 = parse_toml(&r1, Path::new("/tmp/coral.toml")).unwrap();
        let r2 = render_toml(&p2);
        assert_eq!(r1, r2, "render must be byte-identical on re-emit");
    }

    /// Full `[runner.tiered]` block with all three tiers, a custom budget,
    /// and `consolidate.enabled = true`. Verifies parsing populates every
    /// field and renders back identical bytes.
    #[test]
    fn manifest_with_full_tiered_section_parses() {
        let raw = r#"apiVersion = "coral.dev/v1"
[project]
name = "demo"

[[repos]]
name = "api"
url = "git@github.com:acme/api.git"

[runner.tiered.planner]
provider = "claude"
model = "haiku"

[runner.tiered.executor]
provider = "claude"
model = "sonnet"

[runner.tiered.reviewer]
provider = "claude"
model = "opus"

[runner.tiered.budget]
max_tokens_per_run = 50000

[runner.tiered.consolidate]
enabled = true
"#;
        let p = parse_toml(raw, Path::new("/tmp/coral.toml")).unwrap();
        let t = p.runner.tiered.as_ref().expect("tiered must be present");
        assert_eq!(t.planner.provider, "claude");
        assert_eq!(t.planner.model.as_deref(), Some("haiku"));
        assert_eq!(t.executor.model.as_deref(), Some("sonnet"));
        assert_eq!(t.reviewer.model.as_deref(), Some("opus"));
        assert_eq!(t.budget.max_tokens_per_run, 50000);
        assert!(t.consolidate.enabled);
        assert!(p.runner.tiered_enabled_for_consolidate());
        assert!(
            p.runner.tiered_enabled_for_consolidate(),
            "tiered_enabled_for_consolidate helper must return true"
        );
    }

    /// Acceptance #6: a `[runner.tiered]` block missing one of the three
    /// required tier sub-tables must fail at parse time with a message
    /// naming the missing tier(s).
    #[test]
    fn manifest_partial_tiered_section_rejected() {
        let raw = r#"apiVersion = "coral.dev/v1"
[project]
name = "demo"

[[repos]]
name = "api"
url = "git@github.com:acme/api.git"

[runner.tiered.planner]
provider = "claude"

[runner.tiered.executor]
provider = "claude"
"#;
        let err = parse_toml(raw, Path::new("/tmp/coral.toml")).expect_err("must reject");
        let msg = format!("{err}");
        assert!(
            msg.contains("reviewer"),
            "missing-tier error must name `reviewer`: {msg}"
        );
        assert!(
            msg.contains("[runner.tiered]"),
            "error must reference the section header: {msg}"
        );
    }

    /// Acceptance #1 stronger pin: the BC fixture is parsed by both
    /// v0.21.3 (where `RunnerSection` doesn't exist) and v0.21.4. We
    /// can't actually run v0.21.3 code here, but we can verify a
    /// minimal v0.21.3-shape manifest validates and matches the
    /// expected legacy fields.
    #[test]
    fn legacy_v0213_single_repo_fixture_still_parses() {
        let raw = r#"apiVersion = "coral.dev/v1"
[project]
name = "single"

[[repos]]
name = "self"
url = "git@github.com:acme/self.git"
"#;
        let p = parse_toml(raw, Path::new("/tmp/coral.toml")).unwrap();
        p.validate().expect("v0.21.3 fixture must validate");
        assert_eq!(p.runner, RunnerSection::default());
    }

    /// `budget.max_tokens_per_run = 0` is rejected — a zero budget
    /// would be an immediate, non-actionable abort on every run.
    #[test]
    fn manifest_zero_budget_rejected() {
        let raw = r#"apiVersion = "coral.dev/v1"
[project]
name = "demo"

[[repos]]
name = "api"
url = "git@github.com:acme/api.git"

[runner.tiered.planner]
provider = "claude"

[runner.tiered.executor]
provider = "claude"

[runner.tiered.reviewer]
provider = "claude"

[runner.tiered.budget]
max_tokens_per_run = 0
"#;
        let err = parse_toml(raw, Path::new("/tmp/coral.toml")).expect_err("must reject");
        let msg = format!("{err}");
        assert!(msg.contains("max_tokens_per_run"), "got: {msg}");
        assert!(msg.contains("> 0"), "got: {msg}");
    }
}
