//! `coral monitor <subcommand>` — scheduled TestCase runs against a
//! long-lived environment (v0.23.1).
//!
//! Second feature of the v0.23 sprint. While `coral test` is one-shot
//! ("run the suite once and tell me what failed"), `coral monitor up`
//! is a cron-like loop that keeps running the same suite at a fixed
//! cadence and appends every iteration to a JSONL file, so an operator
//! can answer "has the staging env been healthy for the last hour?"
//! at a glance — `coral monitor history --tail 60` shows the last 60
//! ticks.
//!
//! v0.23.1 ships the foreground form. `--detach` is parsed but errors
//! at runtime (see `up::run`) — daemonization, PID files, and
//! `monitor stop` (via PID-kill, vs the v0.23.1 stub which prints
//! "use Ctrl-C") land in v0.23.x or v0.24+ pending demand.
//!
//! Four subcommands:
//!
//!   - `up      --env NAME [--monitor NAME]`  foreground monitor loop
//!   - `list    [--env NAME]`                 declared monitors + status
//!   - `history --env NAME --monitor NAME [--tail N]`  read JSONL tail
//!   - `stop    --env NAME --monitor NAME`    v0.23.1 deferred-stub
//!
//! Pattern-matched against the existing `chaos` subcommand: a single
//! `MonitorArgs` shell with a nested `MonitorCmd` enum so v0.24+ can
//! grow `monitor pause`, `monitor restart`, etc., without breaking the
//! CLI surface.

use anyhow::Result;
use clap::{Args, Subcommand};
use std::path::Path;
use std::process::ExitCode;

pub mod history;
pub mod list;
pub mod run;
pub mod stop;
pub mod up;

#[derive(Args, Debug)]
pub struct MonitorArgs {
    #[command(subcommand)]
    pub command: MonitorCmd,
}

#[derive(Subcommand, Debug)]
pub enum MonitorCmd {
    /// Run a foreground monitor loop. Ctrl-C exits cleanly with a
    /// summary; `--detach` errors with a deferred-message in v0.23.1.
    Up(up::UpArgs),
    /// List declared monitors with best-effort running/stopped status
    /// derived from the JSONL file's last-line timestamp.
    List(list::ListArgs),
    /// Print the last N JSONL lines for a monitor (default 20).
    History(history::HistoryArgs),
    /// **v0.23.1 stub**: prints "use Ctrl-C in foreground" and exits 0.
    /// Real PID-kill lands in v0.23.x or v0.24+ once daemonization ships.
    Stop(stop::StopArgs),
}

pub fn run(args: MonitorArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    match args.command {
        MonitorCmd::Up(a) => up::run(a, wiki_root),
        MonitorCmd::List(a) => list::run(a, wiki_root),
        MonitorCmd::History(a) => history::run(a, wiki_root),
        MonitorCmd::Stop(a) => stop::run(a),
    }
}
