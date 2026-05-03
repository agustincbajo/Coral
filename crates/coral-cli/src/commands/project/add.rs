//! `coral project add <name>` — append a repo to `coral.toml`.

use anyhow::{Context, Result};
use clap::Args;
use coral_core::project::Project;
use coral_core::project::manifest::{RepoEntry, render_toml};
use std::path::Path;
use std::process::ExitCode;

#[derive(Args, Debug)]
pub struct AddArgs {
    /// Repo name (must be unique within the project).
    pub name: String,

    /// Explicit git URL. Mutually exclusive with `--remote`.
    #[arg(long)]
    pub url: Option<String>,

    /// Use the named `[remotes.<remote>]` template to derive the URL.
    /// Mutually exclusive with `--url`.
    #[arg(long)]
    pub remote: Option<String>,

    /// Override `defaults.ref` for this repo (e.g. `release/v3`).
    #[arg(long)]
    pub r#ref: Option<String>,

    /// Override `defaults.path_template` for this repo.
    #[arg(long)]
    pub path: Option<std::path::PathBuf>,

    /// Tags for filtering (`--tags service team:platform`).
    #[arg(long, num_args = 1..)]
    pub tags: Vec<String>,

    /// Cross-repo dependencies (used by `--affected`).
    #[arg(long, num_args = 1..)]
    pub depends_on: Vec<String>,
}

pub fn run(args: AddArgs, _wiki_root_override: Option<&Path>) -> Result<ExitCode> {
    let cwd = std::env::current_dir().context("getting cwd")?;
    let manifest_path = cwd.join("coral.toml");
    if !manifest_path.exists() {
        anyhow::bail!(
            "no coral.toml at {}; run `coral project new` first",
            cwd.display()
        );
    }

    let mut project = Project::load_from_manifest(&manifest_path)?;
    if project.repos.iter().any(|r| r.name == args.name) {
        anyhow::bail!(
            "a repo named '{}' is already declared in {}",
            args.name,
            manifest_path.display()
        );
    }
    if args.url.is_some() && args.remote.is_some() {
        anyhow::bail!("--url and --remote are mutually exclusive");
    }

    let entry = RepoEntry {
        name: args.name.clone(),
        url: args.url,
        remote: args.remote,
        r#ref: args.r#ref,
        path: args.path,
        tags: args.tags,
        depends_on: args.depends_on,
        include: Vec::new(),
        exclude: Vec::new(),
        enabled: true,
    };
    project.repos.push(entry);
    project.validate().context("validating updated manifest")?;

    coral_core::atomic::with_exclusive_lock(&manifest_path, || {
        coral_core::atomic::atomic_write_string(&manifest_path, &render_toml(&project))
    })
    .with_context(|| format!("writing {}", manifest_path.display()))?;

    println!(
        "✔ added repo '{}' to {}",
        args.name,
        manifest_path.display()
    );
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_core::project::manifest::parse_toml;
    use tempfile::TempDir;

    #[test]
    fn adds_repo_to_existing_manifest() {
        let _guard = crate::commands::CWD_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("coral.toml"),
            r#"apiVersion = "coral.dev/v1"
[project]
name = "demo"
"#,
        )
        .unwrap();
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = run(
            AddArgs {
                name: "api".into(),
                url: Some("git@example.com/api.git".into()),
                remote: None,
                r#ref: None,
                path: None,
                tags: vec!["service".into()],
                depends_on: Vec::new(),
            },
            None,
        );
        std::env::set_current_dir(original).unwrap();
        assert!(result.is_ok(), "expected success, got {result:?}");

        let raw = std::fs::read_to_string(dir.path().join("coral.toml")).unwrap();
        let parsed = parse_toml(&raw, &dir.path().join("coral.toml")).unwrap();
        assert_eq!(parsed.repos.len(), 1);
        assert_eq!(parsed.repos[0].name, "api");
        assert_eq!(parsed.repos[0].tags, vec!["service"]);
    }

    #[test]
    fn rejects_duplicate_name() {
        let _guard = crate::commands::CWD_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("coral.toml"),
            r#"apiVersion = "coral.dev/v1"
[project]
name = "demo"

[[repos]]
name = "api"
url = "git@example.com/api.git"
"#,
        )
        .unwrap();
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = run(
            AddArgs {
                name: "api".into(),
                url: Some("git@example.com/api2.git".into()),
                remote: None,
                r#ref: None,
                path: None,
                tags: vec![],
                depends_on: Vec::new(),
            },
            None,
        );
        std::env::set_current_dir(original).unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn rejects_url_and_remote_together() {
        let _guard = crate::commands::CWD_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("coral.toml"),
            r#"apiVersion = "coral.dev/v1"
[project]
name = "demo"
"#,
        )
        .unwrap();
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = run(
            AddArgs {
                name: "api".into(),
                url: Some("git@example.com/api.git".into()),
                remote: Some("github".into()),
                r#ref: None,
                path: None,
                tags: vec![],
                depends_on: Vec::new(),
            },
            None,
        );
        std::env::set_current_dir(original).unwrap();
        assert!(result.is_err());
    }
}
