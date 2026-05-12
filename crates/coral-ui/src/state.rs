//! Shared server configuration and runtime state.
//!
//! `ServeConfig` is the public API consumed by the CLI; `AppState` is the
//! immutable bundle wired into the request handlers (wiki root, token,
//! write-tools toggle, the bind URL we computed for `Origin` checks).

use std::path::PathBuf;
use std::sync::Arc;

use coral_runner::Runner;

/// User-facing configuration for `coral_ui::serve`.
#[derive(Debug, Clone)]
pub struct ServeConfig {
    /// Bind address (default `127.0.0.1`).
    pub bind: String,
    /// Bind port (default `3838`).
    pub port: u16,
    /// Wiki root directory (default `.wiki`).
    pub wiki_root: PathBuf,
    /// Optional bearer token. None = unauthenticated (loopback-only).
    /// Required when binding to anything other than loopback.
    pub token: Option<String>,
    /// When true, mutation endpoints (`/api/v1/tools/*`) become callable.
    /// Default false. Currently a stub — write tools are not implemented
    /// in v0.32.0 but the gate is wired up so the route surface can grow.
    pub allow_write_tools: bool,
    /// When true (default), open the user's browser at the bind URL on
    /// startup. The CLI wrapper toggles this via `--no-open`.
    pub open_browser: bool,
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".into(),
            port: 3838,
            wiki_root: PathBuf::from(".wiki"),
            token: None,
            allow_write_tools: false,
            open_browser: true,
        }
    }
}

/// Immutable shared state passed to every request handler.
///
/// Wrapped in `Arc` so the recv loop can hand each request a cheap clone.
/// `runner` is `Arc<dyn Runner>` because constructing one (`ClaudeRunner`)
/// is cheap but `Runner` is `Send + Sync` and we want to share a single
/// instance across the (rare) `/api/v1/query` requests rather than
/// rebuild it per call.
pub struct AppState {
    pub bind: String,
    pub port: u16,
    pub wiki_root: PathBuf,
    pub token: Option<String>,
    pub allow_write_tools: bool,
    /// Lazily-constructed default Claude runner. Built once at startup;
    /// callers of `/api/v1/query` can pass `model` via the request body.
    /// `None` means the runner couldn't be constructed (e.g. `claude`
    /// binary missing — but we don't fail-fast for that, only fail at
    /// query time so the rest of the API stays usable).
    pub runner: Option<Arc<dyn Runner>>,
}

impl AppState {
    /// Returns the loopback origin string used to validate `Origin`
    /// headers on POST requests, e.g. `http://127.0.0.1:3838`.
    pub fn bind_origin(&self) -> String {
        format!("http://{}:{}", self.bind, self.port)
    }
}
