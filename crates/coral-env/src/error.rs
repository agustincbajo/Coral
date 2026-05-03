//! Errors emitted by the env layer.
//!
//! Mirrors `coral_runner::RunnerError` — the same error-shape pattern
//! gives users actionable, consistent messages across both layers.

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
#[allow(clippy::large_enum_variant)] // mirror coral_runner::RunnerError shape; size is fine for an error path
pub enum EnvError {
    /// The backend's CLI binary (e.g. `docker compose`) is not on PATH.
    /// Hint includes install pointers for the major platforms.
    #[error(
        "backend '{backend}' is unavailable: {hint}. Install one of: docker (https://docs.docker.com/engine/install/), podman (https://podman.io/docs/installation)."
    )]
    BackendNotFound { backend: String, hint: String },

    /// The backend is installed but its version is below what Coral
    /// requires for the requested feature (e.g. compose `develop.watch`
    /// requires 2.22+). Includes the minimum and observed versions.
    #[error("backend '{backend}' v{found} is too old; v{required}+ is required for {feature}")]
    BackendVersionTooOld {
        backend: String,
        found: String,
        required: String,
        feature: String,
    },

    /// The user asked about a service name that isn't declared in the
    /// active environment.
    #[error("service '{0}' is not declared in this environment")]
    ServiceNotFound(String),

    /// A service crashed during `up` or after a healthcheck failure.
    #[error("service '{service}' crashed (exit code {code}): {stderr_tail}")]
    Crashed {
        service: String,
        code: i32,
        stderr_tail: String,
    },

    /// Generic timeout — used by the healthcheck loop and by long-running
    /// subprocess commands.
    #[error("timeout after {seconds}s while waiting for {what}")]
    Timeout { what: String, seconds: u64 },

    /// The on-disk plan hash doesn't match what the backend last
    /// reported. Indicates the user edited `coral.toml` (or one of its
    /// inputs) without re-running `coral up`.
    #[error("environment drift detected: regenerate via `coral up --build`")]
    Drift,

    /// Live-reload was requested but the backend doesn't support it
    /// (e.g. compose < 2.22).
    #[error("backend '{backend}' does not support live-reload watch")]
    WatchNotSupported { backend: String },

    /// The manifest is structurally fine but the env section is invalid
    /// (cycle in depends_on, unknown backend, etc.).
    #[error("invalid environment spec: {0}")]
    InvalidSpec(String),

    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Anything else from the backend subprocess that doesn't fit a
    /// more specific variant.
    #[error("backend '{backend}' error: {message}")]
    BackendError { backend: String, message: String },
}

pub type EnvResult<T> = std::result::Result<T, EnvError>;
