pub mod chaos;
pub mod ci;
pub mod common;
pub mod context_build;
pub mod contract;
pub mod down;
pub mod env;
pub mod env_resolve;
pub mod export;
pub mod export_agents;
pub mod export_skill;
pub mod filters;
pub mod guarantee;
pub mod history;
pub mod init;
pub mod interface;
pub mod lint;
pub mod mcp;
pub mod monitor;
pub mod notion_push;
pub mod pins;
pub mod project;
pub mod search;
pub mod self_check;
#[cfg(feature = "webui")]
pub mod serve;
pub mod session;
pub mod skill;
pub mod stats;
pub mod status;
pub mod sync;
pub mod test;
pub mod test_discover;
#[cfg(feature = "ui")]
pub mod ui;
pub mod up;
pub mod validate_pin;
pub mod verify;
pub mod wiki;

pub mod bootstrap;
pub mod consolidate;
pub mod diff;
pub mod ingest;
pub mod onboard;
pub mod plan;
pub mod prompt_loader;
pub mod prompts;
pub mod query;

pub mod migrate;
pub mod mutants;
pub mod runner_helper;
pub mod scaffold;

/// v0.30.0 audit cycle 5 B2: documented exit-code contract for the
/// "is-something-wrong-with-my-project" family of commands (`lint`,
/// `verify`, `contract check`). Other commands MAY adopt this; the
/// hard requirement is just that these three distinguish "I ran fine
/// and reported N findings" from "I crashed trying to run".
///
/// | Code | Meaning                                              |
/// |------|------------------------------------------------------|
/// |   0  | Clean. No findings.                                  |
/// |   1  | Findings. User-actionable (lint hits, drift, …).     |
/// |   2  | Usage error. Bad flags, missing required arg.        |
/// |   3  | Internal error. I/O, parse, backend down, panic.     |
///
/// `commands::test::run` already uses `ExitCode::from(2)` for usage
/// errors via the clap subcommand layer. This module documents the
/// rest; the actual `Err -> ExitCode::from(3)` mapping happens at the
/// dispatch boundary in `main.rs` (see `dispatch_with_internal_exit`).
pub mod exit_codes {
    use std::process::ExitCode;

    pub const CLEAN: u8 = 0;
    pub const FINDINGS: u8 = 1;
    /// Usage / argument errors. Reserved for clap-driven failures;
    /// `commands::test::run` already returns this for unknown test
    /// kinds.
    pub const USAGE: u8 = 2;
    /// Internal / crash. The command never produced a report; the
    /// caller should treat this as "tool was unable to determine
    /// whether the project is clean" rather than "project is dirty".
    pub const INTERNAL: u8 = 3;

    /// Helper: turn a `Result<ExitCode>` from a command module into a
    /// `Result<ExitCode>` that maps `Err` to `Ok(ExitCode::from(3))`.
    /// Errors are still printed (caller does that). Used at the
    /// dispatch boundary for commands that opt into the contract.
    pub fn map_internal_err(result: anyhow::Result<ExitCode>) -> (ExitCode, Option<anyhow::Error>) {
        match result {
            Ok(code) => (code, None),
            Err(e) => (ExitCode::from(INTERNAL), Some(e)),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// v0.30.0 audit cycle 5 B2: the exit-code contract constants
        /// must match what `lint`/`verify`/`contract` and external CI
        /// systems agree on. A change here is a breaking change to
        /// the CLI contract — the test pins the numeric values so a
        /// careless refactor can't silently shift them.
        #[test]
        fn exit_code_contract_constants_are_pinned() {
            assert_eq!(CLEAN, 0, "0 is reserved for `clean`");
            assert_eq!(FINDINGS, 1, "1 is reserved for `findings`");
            assert_eq!(USAGE, 2, "2 is reserved for `usage error`");
            assert_eq!(INTERNAL, 3, "3 is reserved for `internal error`");
        }

        /// v0.30.0 audit cycle 5 B2: `map_internal_err` converts an
        /// `Err` to `ExitCode 3` and preserves the error for the
        /// caller to print. An `Ok(code)` passes through unchanged.
        #[test]
        fn map_internal_err_rewrites_err_to_3_and_passes_ok_through() {
            // Ok-finding case: ExitCode 1 passes through unchanged.
            let (code, err) = map_internal_err(Ok(ExitCode::from(FINDINGS)));
            assert!(err.is_none(), "Ok input must not produce an error");
            // ExitCode does not implement PartialEq; compare via Debug.
            assert_eq!(
                format!("{code:?}"),
                format!("{:?}", ExitCode::from(FINDINGS))
            );

            // Err case: rewritten to ExitCode 3 with the error preserved.
            let (code, err) = map_internal_err(Err(anyhow::anyhow!("backend down")));
            assert!(err.is_some(), "Err input must surface the error");
            assert!(
                err.unwrap().to_string().contains("backend down"),
                "the original error message must survive"
            );
            assert_eq!(
                format!("{code:?}"),
                format!("{:?}", ExitCode::from(INTERNAL))
            );
        }
    }
}

/// Shared cwd mutex for tests across all command modules.
///
/// Process cwd is global, so any test that calls `set_current_dir` must
/// serialize against every other such test — not just within the same
/// `mod tests`. Each command module has tests that mutate cwd; using
/// per-module mutexes meant cross-module races. Tests grab this single lock
/// instead. Poison-tolerant: if a previous test panicked while holding it,
/// the next test recovers via `unwrap_or_else(|p| p.into_inner())`.
#[cfg(test)]
pub(crate) static CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
