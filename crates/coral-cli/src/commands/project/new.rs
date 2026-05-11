//! `coral project new <name>` — create a new multi-repo project manifest.
//!
//! Writes `coral.toml` + an empty `coral.lock` in the cwd, plus the same
//! `.wiki/` scaffold that `coral init` creates in the single-repo case.
//! Idempotent: refuses to overwrite existing files unless `--force` is
//! passed.

use anyhow::{Context, Result};
use clap::Args;
use coral_core::project::{
    Lockfile, Project,
    manifest::{ProjectDefaults, RemoteSpec, Toolchain, render_toml},
};
use std::path::Path;
use std::process::ExitCode;

#[derive(Args, Debug)]
pub struct NewArgs {
    /// The project name (defaults to the cwd's basename).
    pub name: Option<String>,

    /// The default git remote slug (`[remotes.<name>]`). When set, also
    /// declares a placeholder `[remotes.<name>] fetch = "..."` block
    /// the user can edit.
    #[arg(long)]
    pub remote: Option<String>,

    /// Overwrite an existing `coral.toml` / `coral.lock`. Refuses to
    /// touch the existing `.wiki/` though — that's `coral init --force`.
    #[arg(long)]
    pub force: bool,

    /// Pin the Coral version that this project requires
    /// (`[project.toolchain] coral = "..."`). Defaults to the running
    /// binary's version.
    #[arg(long)]
    pub pin_toolchain: bool,
}

pub fn run(args: NewArgs, _wiki_root_override: Option<&Path>) -> Result<ExitCode> {
    let cwd = std::env::current_dir().context("getting cwd")?;
    let manifest_path = cwd.join("coral.toml");
    let lockfile_path = cwd.join("coral.lock");

    if manifest_path.exists() && !args.force {
        anyhow::bail!(
            "{} already exists; pass --force to overwrite",
            manifest_path.display()
        );
    }

    let name = args.name.unwrap_or_else(|| {
        cwd.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_string()
    });

    let mut project = Project::single_repo(name.clone(), cwd.clone());
    // Replace the synthesized in-place repo with an empty manifest —
    // `coral project new` is the multi-repo setup; the user then
    // `add`s repos. Rationale: a fresh manifest with one synth repo
    // pointing at "." is confusing; we leave `repos` empty and let
    // the user `coral project add ...`.
    project.repos.clear();
    project.manifest_path = manifest_path.clone();
    project.root = cwd.clone();
    if args.pin_toolchain {
        project.toolchain = Toolchain {
            coral: Some(env!("CARGO_PKG_VERSION").to_string()),
        };
    }
    if let Some(remote) = &args.remote {
        let mut remotes = std::collections::BTreeMap::new();
        remotes.insert(
            remote.clone(),
            RemoteSpec {
                fetch: "git@github.com:YOUR-ORG/{name}.git".to_string(),
            },
        );
        project.remotes = remotes;
        project.defaults = ProjectDefaults {
            remote: Some(remote.clone()),
            ..ProjectDefaults::default()
        };
    }

    coral_core::atomic::atomic_write_string(&manifest_path, &render_toml(&project))
        .with_context(|| format!("writing {}", manifest_path.display()))?;

    if !lockfile_path.exists() || args.force {
        let lock = Lockfile::new();
        lock.write_atomic(&lockfile_path)
            .with_context(|| format!("writing {}", lockfile_path.display()))?;
    }

    println!("✔ created {}", manifest_path.display());
    println!("✔ created {}", lockfile_path.display());
    println!();
    println!("next steps:");
    println!("  coral project add <name> --url <git-url>   # declare a repo");
    println!("  coral init                                  # bootstrap the aggregated .wiki/");
    println!("  coral ingest                                # ingest pages from each repo");
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_core::project::manifest::parse_toml;
    use tempfile::TempDir;

    #[test]
    fn creates_manifest_and_lockfile() {
        let _guard = crate::commands::CWD_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = TempDir::new().unwrap();
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let result = run(
            NewArgs {
                name: Some("my-stack".into()),
                remote: None,
                force: false,
                pin_toolchain: false,
            },
            None,
        );
        std::env::set_current_dir(original).unwrap();

        result.expect("project new must succeed in a fresh empty directory");
        let manifest = std::fs::read_to_string(dir.path().join("coral.toml")).unwrap();
        let parsed = parse_toml(&manifest, &dir.path().join("coral.toml")).unwrap();
        assert_eq!(parsed.name, "my-stack");
        assert!(parsed.repos.is_empty());
        assert!(dir.path().join("coral.lock").is_file());
    }

    #[test]
    fn refuses_to_overwrite_without_force() {
        let _guard = crate::commands::CWD_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("coral.toml"), "existing\n").unwrap();
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let result = run(
            NewArgs {
                name: Some("demo".into()),
                remote: None,
                force: false,
                pin_toolchain: false,
            },
            None,
        );
        std::env::set_current_dir(original).unwrap();

        assert!(result.is_err());
    }

    #[test]
    fn force_overwrites_existing_manifest() {
        let _guard = crate::commands::CWD_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("coral.toml"), "stale\n").unwrap();
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let result = run(
            NewArgs {
                name: Some("demo".into()),
                remote: None,
                force: true,
                pin_toolchain: false,
            },
            None,
        );
        std::env::set_current_dir(original).unwrap();

        result.expect("project new --force must succeed even when coral.toml already exists");
        let after = std::fs::read_to_string(dir.path().join("coral.toml")).unwrap();
        assert!(after.contains("[project]"));
    }
}
