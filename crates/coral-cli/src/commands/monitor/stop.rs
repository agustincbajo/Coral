//! `coral monitor stop` — **v0.23.1 deferred-stub**.
//!
//! v0.23.1 ships only the foreground `monitor up` form, so there's no
//! background daemon to stop. This subcommand exists in the CLI
//! surface so v0.24+ can drop in the real PID-kill flow without
//! breaking script users that already wrote `coral monitor stop ...`.
//!
//! Today: prints the deferred-message and exits 0. The exit code is
//! deliberately 0 (not non-zero) — script wrappers calling `monitor
//! stop` repeatedly should not crash on each invocation; the message
//! tells them what to do.

use anyhow::Result;
use clap::Args;
use std::process::ExitCode;

#[derive(Args, Debug)]
pub struct StopArgs {
    #[arg(long)]
    pub env: String,
    #[arg(long = "monitor")]
    pub monitor: String,
}

pub fn run(args: StopArgs) -> Result<ExitCode> {
    println!(
        "monitor stop is deferred to v0.24+; for the foreground `monitor up --env {} --monitor {}` \
         in another shell, send Ctrl-C (SIGINT) directly to that process",
        args.env, args.monitor
    );
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_returns_success() {
        // The handler prints to stdout (we don't capture here) and
        // exits 0. Just smoke-check that the function returns Ok.
        let args = StopArgs {
            env: "dev".into(),
            monitor: "smoke".into(),
        };
        let exit = run(args).expect("ok");
        let dbg = format!("{exit:?}");
        assert!(dbg.to_lowercase().contains("success") || dbg.contains("0"));
    }
}
