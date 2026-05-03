pub mod common;
pub mod context_build;
pub mod down;
pub mod env;
pub mod env_resolve;
pub mod export;
pub mod export_agents;
pub mod filters;
pub mod history;
pub mod init;
pub mod lint;
pub mod mcp;
pub mod notion_push;
pub mod pins;
pub mod project;
pub mod search;
pub mod stats;
pub mod status;
pub mod sync;
pub mod test;
pub mod test_discover;
pub mod up;
pub mod validate_pin;
pub mod verify;

pub mod bootstrap;
pub mod consolidate;
pub mod diff;
pub mod ingest;
pub mod onboard;
pub mod plan;
pub mod prompt_loader;
pub mod prompts;
pub mod query;

pub mod runner_helper;

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
