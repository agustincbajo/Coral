//! Errors emitted by the test layer.
//!
//! Test-level errors are distinct from `EnvError` because they
//! represent test infrastructure problems (couldn't read a YAML, no
//! handle to the environment, snapshot dir missing) — *not* test
//! failures, which are encoded as `TestStatus::Fail`.

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
#[allow(clippy::large_enum_variant)] // mirror EnvError shape; size doesn't matter on the error path
pub enum TestError {
    #[error("test fixture not found: {0}")]
    FixtureNotFound(PathBuf),

    #[error("test spec failed to parse: {path}: {reason}")]
    InvalidSpec { path: PathBuf, reason: String },

    #[error("environment is not up; run `coral up` first")]
    EnvNotUp,

    #[error("service '{0}' is not exposed by the running environment")]
    ServiceNotExposed(String),

    #[error("snapshot directory missing or unwritable: {0}")]
    SnapshotDir(PathBuf),

    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("env error: {0}")]
    Env(#[from] coral_env::EnvError),

    #[error("yaml error: {0}")]
    Yaml(#[from] serde_yaml_ng::Error),
}

pub type TestResult<T> = std::result::Result<T, TestError>;
