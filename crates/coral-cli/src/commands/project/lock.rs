//! `coral project lock` — regenerate `coral.lock` from the manifest without
//! pulling. Idempotent dry-run of `coral project sync`.
//!
//! Reads the existing lockfile (if any) and the manifest. Emits a
//! refreshed lockfile entry per repo with **the SHA already present
//! (or zeros if absent)**. Real ref-resolution + git fetch lands in
//! v0.16.x with `coral project sync`.

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Args;
use coral_core::project::{Lockfile, RepoLockEntry};
use std::path::Path;
use std::process::ExitCode;

use crate::commands::common::resolve_project;

#[derive(Args, Debug)]
pub struct LockArgs {
    /// Dry-run: print the lockfile contents to stdout instead of writing.
    #[arg(long)]
    pub dry_run: bool,
}

pub fn run(args: LockArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let project = resolve_project(wiki_root)?;
    if project.is_legacy() {
        anyhow::bail!(
            "`coral project lock` requires a coral.toml; this is a legacy single-repo project"
        );
    }

    let lock_path = project.lockfile_path();

    // v0.30.x audit #005: pre-fix the load-mutate-save sequence below
    // ran outside any flock — a concurrent `coral project sync` or
    // a second `coral project lock` could read the same snapshot,
    // each apply their own mutations, and the second `write_atomic`
    // would silently clobber the first. Wrap the whole body in
    // `with_exclusive_lock` (same pattern as `project::sync::run`),
    // re-reading INSIDE the lock so a freshly-persisted snapshot
    // from sync is picked up.
    //
    // We call `atomic_write_string` directly rather than
    // `Lockfile::write_atomic`: the latter re-acquires the same
    // flock from a fresh FD, which on Linux/macOS can self-deadlock
    // when re-entered from the same process.
    let now = Utc::now();
    let dry_run = args.dry_run;
    let manifest_names: std::collections::BTreeSet<String> =
        project.repos.iter().map(|r| r.name.clone()).collect();

    coral_core::atomic::with_exclusive_lock(&lock_path, || {
        let mut lock = Lockfile::load_or_default(&lock_path).map_err(|e| {
            coral_core::error::CoralError::Walk(format!(
                "loading {}: {}",
                lock_path.display(),
                e
            ))
        })?;

        for repo in &project.repos {
            if !repo.enabled {
                continue;
            }
            let in_place = repo
                .path
                .as_ref()
                .map(|p| p == Path::new("."))
                .unwrap_or(false);
            if in_place {
                continue;
            }
            let url = match project.resolved_url(repo) {
                Some(u) => u,
                None => continue,
            };
            let r#ref = repo
                .r#ref
                .clone()
                .unwrap_or_else(|| project.defaults.r#ref.clone());

            let sha = lock
                .repos
                .get(&repo.name)
                .map(|e| e.sha.clone())
                .unwrap_or_else(|| "0000000000000000000000000000000000000000".to_string());

            lock.upsert(
                &repo.name,
                RepoLockEntry {
                    url,
                    r#ref,
                    sha,
                    synced_at: now,
                },
            );
        }

        // Drop entries that no longer correspond to a repo.
        let stale: Vec<String> = lock
            .repos
            .keys()
            .filter(|k| !manifest_names.contains(k.as_str()))
            .cloned()
            .collect();
        for name in stale {
            lock.repos.remove(&name);
        }

        if dry_run {
            print!("{lock}");
            return Ok(());
        }
        coral_core::atomic::atomic_write_string(&lock_path, &lock.to_string())
    })
    .with_context(|| format!("locking {}", lock_path.display()))?;

    if !dry_run {
        println!("✔ updated {}", lock_path.display());
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn lock_writes_lockfile_for_declared_repos() {
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
url = "git@example.com:acme/api.git"
"#,
        )
        .unwrap();
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = run(LockArgs { dry_run: false }, None);
        std::env::set_current_dir(original).unwrap();
        result.expect("lock run must succeed on a valid manifest");

        let raw = std::fs::read_to_string(dir.path().join("coral.lock")).unwrap();
        let lock = Lockfile::parse(&raw).unwrap();
        assert_eq!(lock.repos.len(), 1);
        assert!(lock.repos.contains_key("api"));
    }

    #[test]
    fn lock_drops_stale_entries() {
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
url = "git@example.com:acme/api.git"
"#,
        )
        .unwrap();
        // Pre-seed lock with a stale 'gone' entry.
        std::fs::write(
            dir.path().join("coral.lock"),
            r#"# Generated by `coral project sync`
apiVersion = "coral.dev/v1"
resolved_at = "2026-05-01T00:00:00Z"

[repos.gone]
url = "git@example.com:acme/gone.git"
ref = "main"
sha = "deadbeef"
synced_at = "2026-05-01T00:00:00Z"
"#,
        )
        .unwrap();

        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = run(LockArgs { dry_run: false }, None);
        std::env::set_current_dir(original).unwrap();
        result.expect("lock run must succeed and drop stale entries");
        let raw = std::fs::read_to_string(dir.path().join("coral.lock")).unwrap();
        let lock = Lockfile::parse(&raw).unwrap();
        assert!(!lock.repos.contains_key("gone"));
        assert!(lock.repos.contains_key("api"));
    }

    /// v0.30.x audit #005 regression: concurrent load-mutate-save against
    /// the same `coral.lock` via `with_exclusive_lock` must serialize,
    /// so every thread's upsert lands in the final file. Pre-fix the
    /// `coral project lock` body ran outside any flock and lost updates
    /// under a concurrent `coral project sync`.
    ///
    /// We exercise the lock-acquisition path with N threads each performing
    /// the same load → mutate → atomic_write_string sequence the production
    /// code now uses. If serialization is broken we lose at least one
    /// upsert and the final entry count will be < N.
    #[test]
    fn project_lock_serializes_concurrent_writers() {
        use chrono::Utc;
        const N: usize = 8;
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("coral.lock");
        // Seed with an empty lockfile so load_or_default has something
        // valid to parse on the first reader (it tolerates missing file).
        std::thread::scope(|s| {
            for i in 0..N {
                let lock_path = lock_path.clone();
                s.spawn(move || {
                    let name = format!("repo-{i}");
                    coral_core::atomic::with_exclusive_lock(&lock_path, || {
                        let mut lock = Lockfile::load_or_default(&lock_path).map_err(|e| {
                            coral_core::error::CoralError::Walk(format!(
                                "loading {}: {}",
                                lock_path.display(),
                                e
                            ))
                        })?;
                        lock.upsert(
                            &name,
                            RepoLockEntry {
                                url: format!("git@example.com:acme/{name}.git"),
                                r#ref: "main".into(),
                                sha: "0000000000000000000000000000000000000000".into(),
                                synced_at: Utc::now(),
                            },
                        );
                        coral_core::atomic::atomic_write_string(&lock_path, &lock.to_string())
                    })
                    .expect("with_exclusive_lock must succeed under contention");
                });
            }
        });

        let raw = std::fs::read_to_string(&lock_path).expect("final lockfile present");
        let lock = Lockfile::parse(&raw).expect("final lockfile parseable");
        assert_eq!(
            lock.repos.len(),
            N,
            "all {N} concurrent upserts must land; got: {:?}",
            lock.repos.keys().collect::<Vec<_>>()
        );
        for i in 0..N {
            assert!(
                lock.repos.contains_key(&format!("repo-{i}")),
                "missing repo-{i} upsert — lost update under concurrent lock contention"
            );
        }
    }

    #[test]
    fn lock_rejects_legacy_project() {
        let _guard = crate::commands::CWD_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = TempDir::new().unwrap();
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = run(LockArgs { dry_run: false }, None);
        std::env::set_current_dir(original).unwrap();
        assert!(result.is_err());
    }
}
