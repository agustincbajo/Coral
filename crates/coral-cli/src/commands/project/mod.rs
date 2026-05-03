//! `coral project ...` subcommand family — the multi-repo toolbox.
//!
//! Lives behind a single `coral project <verb>` dispatcher rather than
//! polluting the top-level `coral` namespace with seven new commands.
//! v0.16 ships `new`, `list`, `add`, `doctor`, and `lock`. `sync`
//! follows in v0.16.x once the git-clone harness is wired.

pub mod add;
pub mod doctor;
pub mod list;
pub mod lock;
pub mod new;

use clap::{Args, Subcommand};
use std::path::Path;
use std::process::ExitCode;

#[derive(Args, Debug)]
pub struct ProjectArgs {
    #[command(subcommand)]
    pub command: ProjectCmd,
}

#[derive(Subcommand, Debug)]
pub enum ProjectCmd {
    /// Create a new `coral.toml` + aggregated `.wiki/` in the current dir.
    New(new::NewArgs),
    /// List repos declared in `coral.toml`.
    List(list::ListArgs),
    /// Add a repo entry to `coral.toml`.
    Add(add::AddArgs),
    /// Diagnose the project (manifest, lockfile, repo clones, drift).
    Doctor(doctor::DoctorArgs),
    /// Refresh `coral.lock` from the manifest without pulling.
    Lock(lock::LockArgs),
}

pub fn run(args: ProjectArgs, wiki_root: Option<&Path>) -> anyhow::Result<ExitCode> {
    match args.command {
        ProjectCmd::New(a) => new::run(a, wiki_root),
        ProjectCmd::List(a) => list::run(a, wiki_root),
        ProjectCmd::Add(a) => add::run(a, wiki_root),
        ProjectCmd::Doctor(a) => doctor::run(a, wiki_root),
        ProjectCmd::Lock(a) => lock::run(a, wiki_root),
    }
}
