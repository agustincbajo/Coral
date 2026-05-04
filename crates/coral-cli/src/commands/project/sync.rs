//! `coral project sync` — clone or update every repo declared in `coral.toml`,
//! then write the resolved SHAs to `coral.lock`.
//!
//! Parallelizes by default (rayon) — N repos clone concurrently, network I/O
//! is the bottleneck. Individual auth failures are skipped-with-warning per
//! PRD risk #10: a missing SSH key on one repo doesn't abort the rest of the
//! project.

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Args;
use coral_core::git_remote::{SyncOutcome, sync_repo};
use coral_core::project::{Lockfile, RepoLockEntry};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use std::path::Path;
use std::process::ExitCode;

use crate::commands::common::resolve_project;
use crate::commands::filters::RepoFilters;

#[derive(Args, Debug)]
pub struct SyncArgs {
    #[command(flatten)]
    pub filters: RepoFilters,

    /// Run sequentially instead of paralelizing across repos. Easier to read
    /// the output when debugging a single failing repo.
    #[arg(long)]
    pub sequential: bool,

    /// Exit non-zero if ANY repo's sync failed (auth, dirty tree, hard error).
    /// Default: exit 0 if at least one repo synced.
    #[arg(long)]
    pub strict: bool,
}

pub fn run(args: SyncArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let project = resolve_project(wiki_root)?;
    if project.is_legacy() {
        anyhow::bail!(
            "`coral project sync` requires a coral.toml; this is a legacy single-repo project"
        );
    }

    let selected = args.filters.select(&project);
    if selected.is_empty() {
        println!("no repos selected by filters; nothing to sync");
        return Ok(ExitCode::SUCCESS);
    }

    // Build (name, url, ref, target_path) tuples up front so the parallel
    // closure doesn't need to reach back into &project.
    let plans: Vec<SyncPlan> = selected
        .iter()
        .filter(|repo| {
            let in_place = repo
                .path
                .as_ref()
                .map(|p| p == Path::new("."))
                .unwrap_or(false);
            !in_place
        })
        .filter_map(|repo| {
            let url = project.resolved_url(repo)?;
            let target = project.resolved_path(repo);
            let r#ref = repo
                .r#ref
                .clone()
                .unwrap_or_else(|| project.defaults.r#ref.clone());
            Some(SyncPlan {
                name: repo.name.clone(),
                url,
                r#ref,
                target,
            })
        })
        .collect();

    if plans.is_empty() {
        println!("no remotely-clonable repos in the selected set");
        return Ok(ExitCode::SUCCESS);
    }

    let outcomes: Vec<(SyncPlan, std::result::Result<SyncOutcome, String>)> = if args.sequential {
        plans.into_iter().map(execute_one).collect()
    } else {
        plans.into_par_iter().map(execute_one).collect()
    };

    let lockfile_path = project.lockfile_path();
    let now = Utc::now();

    // v0.19.6 audit H3: the load+modify+save sequence below was racy
    // under concurrent `coral project sync --repo A` / `--repo B`
    // invocations — both processes would `load_or_default` outside any
    // lock, mutate their own in-memory copy, then `Lockfile::write_atomic`
    // would clobber. Same shape as the v0.19.5 H7 ingest-race fix
    // against `index.md`. Wrap the whole upsert+prune+write in
    // `with_exclusive_lock` so cross-process syncs serialize.
    //
    // We re-read INSIDE the lock so the second writer picks up the
    // first writer's freshly-persisted entries before applying its own
    // upserts. We call `atomic_write_string` directly (rather than
    // `Lockfile::write_atomic`) — `Lockfile::write_atomic` itself
    // acquires the same flock from a fresh FD, which on Linux/macOS
    // can self-deadlock when re-entered from the same process.
    let mut failed = 0usize;
    let mut succeeded = 0usize;
    let mut skipped = 0usize;
    let manifest_names: std::collections::BTreeSet<String> =
        project.repos.iter().map(|r| r.name.clone()).collect();
    coral_core::atomic::with_exclusive_lock(&lockfile_path, || {
        let mut lock = Lockfile::load_or_default(&lockfile_path).map_err(|e| {
            coral_core::error::CoralError::Walk(format!(
                "loading {}: {}",
                lockfile_path.display(),
                e
            ))
        })?;
        for (plan, outcome) in &outcomes {
            match outcome {
                Ok(o) => {
                    report_one(&plan.name, o);
                    if let Some(sha) = o.sha() {
                        lock.upsert(
                            &plan.name,
                            RepoLockEntry {
                                url: plan.url.clone(),
                                r#ref: plan.r#ref.clone(),
                                sha: sha.to_string(),
                                synced_at: now,
                            },
                        );
                        succeeded += 1;
                    } else if o.is_skipped() {
                        skipped += 1;
                    } else {
                        failed += 1;
                    }
                }
                Err(e) => {
                    eprintln!("✘ {}: {}", plan.name, e);
                    failed += 1;
                }
            }
        }
        // Drop lockfile entries for repos no longer in the manifest.
        let stale: Vec<String> = lock
            .repos
            .keys()
            .filter(|k| !manifest_names.contains(k.as_str()))
            .cloned()
            .collect();
        for name in stale {
            lock.repos.remove(&name);
        }
        coral_core::atomic::atomic_write_string(&lockfile_path, &lock.to_string())
    })
    .with_context(|| format!("syncing {}", lockfile_path.display()))?;

    println!();
    println!(
        "synced {} repo(s); {} skipped; {} failed",
        succeeded, skipped, failed
    );

    if failed > 0 || (args.strict && skipped > 0) {
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

#[derive(Debug, Clone)]
struct SyncPlan {
    name: String,
    url: String,
    r#ref: String,
    target: std::path::PathBuf,
}

fn execute_one(plan: SyncPlan) -> (SyncPlan, std::result::Result<SyncOutcome, String>) {
    let result = sync_repo(&plan.url, &plan.r#ref, &plan.target).map_err(|e| e.to_string());
    (plan, result)
}

fn report_one(name: &str, outcome: &SyncOutcome) {
    match outcome {
        SyncOutcome::Cloned { sha } => {
            println!("✔ {} cloned (sha {})", name, &sha[..8.min(sha.len())])
        }
        SyncOutcome::Updated { sha } => {
            println!("✔ {} updated (sha {})", name, &sha[..8.min(sha.len())])
        }
        SyncOutcome::SkippedDirty { reason } => {
            println!("⚠ {} skipped: {reason}", name)
        }
        SyncOutcome::SkippedAuth { stderr_tail } => {
            println!("⚠ {} skipped (auth): {}", name, first_line(stderr_tail))
        }
        SyncOutcome::Failed { stderr_tail } => {
            eprintln!("✘ {} failed: {}", name, first_line(stderr_tail))
        }
    }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s).trim()
}
