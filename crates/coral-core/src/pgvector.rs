// v0.35 ARCH-C1: feature-gated stub demoted to pub(crate). The
// real wiring requires a concrete PostgreSQL integration that
// hasn't landed yet; dead_code stays allowed until it does.
#![allow(dead_code)]

//! pgvector-backed embeddings storage (opt-in, feature-gated).
//!
//! This module provides a `PgvectorEmbeddingsStorage` struct that implements
//! the [`EmbeddingsStorage`](crate::storage::EmbeddingsStorage) trait using
//! PostgreSQL with the pgvector extension as the backing store. It is intended
//! for wikis with 10k+ pages where SQLite becomes a bottleneck.
//!
//! # Activation
//!
//! Requires the `pgvector` Cargo feature and the `CORAL_EMBEDDINGS_BACKEND=pgvector`
//! environment variable. Connection URL is read from `CORAL_PGVECTOR_URL`.
//!
//! # Schema (v0.25 reference — not yet executed)
//!
//! ```sql
//! CREATE EXTENSION IF NOT EXISTS vector;
//!
//! CREATE TABLE coral_embeddings (
//!     slug        TEXT PRIMARY KEY,
//!     mtime_secs  BIGINT NOT NULL,
//!     embedding   vector(1536) NOT NULL,
//!     provider    TEXT NOT NULL DEFAULT 'voyage-3',
//!     created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
//!     updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
//! );
//!
//! -- IVFFlat index for approximate nearest-neighbor search.
//! -- Lists tuned for ~10k–50k rows; retune for larger wikis.
//! CREATE INDEX coral_embeddings_ivfflat_idx
//!     ON coral_embeddings
//!     USING ivfflat (embedding vector_cosine_ops)
//!     WITH (lists = 100);
//!
//! -- B-tree on mtime for freshness checks.
//! CREATE INDEX coral_embeddings_mtime_idx
//!     ON coral_embeddings (mtime_secs);
//! ```
//!
//! # v0.25 scope
//!
//! This milestone validates the architecture: the struct compiles, the trait
//! is satisfied, and configuration is validated. Actual PostgreSQL I/O (via
//! `tokio-postgres` or `sqlx`) is deferred to a future release.

use crate::error::{CoralError, Result};
use crate::storage::EmbeddingsStorage;
use std::collections::HashSet;

/// Environment variable that selects the embeddings backend.
pub const ENV_BACKEND: &str = "CORAL_EMBEDDINGS_BACKEND";

/// Environment variable for the PostgreSQL connection URL.
pub const ENV_PGVECTOR_URL: &str = "CORAL_PGVECTOR_URL";

/// The backend identifier string that activates this implementation.
pub const BACKEND_ID: &str = "pgvector";

/// Default vector dimensionality (OpenAI text-embedding-3-small / Voyage-3 large).
const DEFAULT_DIM: usize = 1536;

/// Configuration for the pgvector backend, parsed from environment.
#[derive(Debug, Clone)]
pub struct PgvectorConfig {
    /// PostgreSQL connection URL (e.g. `postgres://user:pass@host:port/db`).
    pub url: String,
    /// Provider name for embeddings model.
    pub provider: String,
    /// Vector dimensionality.
    pub dim: usize,
}

impl PgvectorConfig {
    /// Parse configuration from environment variables.
    ///
    /// Returns an error if `CORAL_PGVECTOR_URL` is missing or malformed.
    pub fn from_env() -> Result<Self> {
        let url = std::env::var(ENV_PGVECTOR_URL).map_err(|_| {
            CoralError::Sqlite(format!(
                "pgvector backend requires {ENV_PGVECTOR_URL} environment variable"
            ))
        })?;

        Self::validate_url(&url)?;

        Ok(Self {
            url,
            provider: String::from("voyage-3"),
            dim: DEFAULT_DIM,
        })
    }

    /// Validate that the URL looks like a Postgres connection string.
    fn validate_url(url: &str) -> Result<()> {
        if url.is_empty() {
            return Err(CoralError::Sqlite("pgvector URL is empty".to_string()));
        }
        if !url.starts_with("postgres://") && !url.starts_with("postgresql://") {
            return Err(CoralError::Sqlite(format!(
                "pgvector URL must start with postgres:// or postgresql://, got: {url}"
            )));
        }
        // Basic structure check: must have host component after scheme
        let after_scheme = url.split("://").nth(1).unwrap_or("");
        if after_scheme.is_empty() || after_scheme == "/" {
            return Err(CoralError::Sqlite(
                "pgvector URL is missing host component".to_string(),
            ));
        }
        Ok(())
    }
}

/// pgvector-backed embeddings storage.
///
/// In v0.25 this is a stub that validates configuration but does not
/// perform actual PostgreSQL I/O. All trait methods return appropriate
/// errors indicating the backend is not yet connected.
#[derive(Debug)]
pub struct PgvectorEmbeddingsStorage {
    config: PgvectorConfig,
}

impl PgvectorEmbeddingsStorage {
    /// Create a new pgvector storage instance.
    ///
    /// Validates configuration but does **not** open a connection in v0.25.
    pub fn new(config: PgvectorConfig) -> Self {
        Self { config }
    }

    /// Create from environment variables.
    pub fn from_env() -> Result<Self> {
        let config = PgvectorConfig::from_env()?;
        Ok(Self::new(config))
    }

    /// Returns the configured connection URL (for diagnostics).
    pub fn url(&self) -> &str {
        &self.config.url
    }

    fn stub_error(method: &str) -> CoralError {
        CoralError::Sqlite(format!(
            "pgvector backend: {method}() is a stub in v0.25 — \
             PostgreSQL connection not yet implemented"
        ))
    }
}

impl EmbeddingsStorage for PgvectorEmbeddingsStorage {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn dim(&self) -> usize {
        self.config.dim
    }

    fn is_fresh(&self, _slug: &str, _mtime: i64) -> Result<bool> {
        Err(Self::stub_error("is_fresh"))
    }

    fn upsert(&mut self, _slug: &str, _mtime: i64, _vector: Vec<f32>) -> Result<()> {
        Err(Self::stub_error("upsert"))
    }

    fn prune(&mut self, _live_slugs: &HashSet<String>) -> Result<()> {
        Err(Self::stub_error("prune"))
    }

    fn search(&self, _query_vector: &[f32], _limit: usize) -> Result<Vec<(String, f32)>> {
        Err(Self::stub_error("search"))
    }

    fn flush(&self) -> Result<()> {
        // Flush is a no-op for the stub since there is nothing buffered.
        Ok(())
    }
}

/// Select the appropriate embeddings backend based on the
/// `CORAL_EMBEDDINGS_BACKEND` environment variable.
///
/// Returns `Some(PgvectorEmbeddingsStorage)` when the env var is set to
/// `"pgvector"`. Returns `None` for any other value (allowing the caller
/// to fall through to the default SQLite/JSON backend). Returns `Err` if
/// pgvector is selected but configuration is invalid.
pub fn select_pgvector_backend() -> Result<Option<PgvectorEmbeddingsStorage>> {
    match std::env::var(ENV_BACKEND).ok() {
        Some(ref val) if val == BACKEND_ID => {
            let storage = PgvectorEmbeddingsStorage::from_env()?;
            Ok(Some(storage))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    /// Helper: run a closure with specific env vars set, then restore.
    ///
    /// # Safety
    ///
    /// Tests using this helper must be run single-threaded (the default
    /// for `cargo test`) because `env::set_var` / `env::remove_var` are
    /// not thread-safe.
    fn with_env_vars<F, R>(vars: &[(&str, Option<&str>)], f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let originals: Vec<_> = vars.iter().map(|(k, _)| (*k, env::var(k).ok())).collect();

        for (k, v) in vars {
            // SAFETY: tests are run single-threaded.
            match v {
                Some(val) => unsafe { env::set_var(k, val) },
                None => unsafe { env::remove_var(k) },
            }
        }

        let result = f();

        for (k, original) in &originals {
            // SAFETY: tests are run single-threaded.
            match original {
                Some(val) => unsafe { env::set_var(k, val) },
                None => unsafe { env::remove_var(k) },
            }
        }

        result
    }

    #[test]
    fn pgvector_module_compiles_and_trait_is_satisfied() {
        // Proves the struct implements EmbeddingsStorage.
        let config = PgvectorConfig {
            url: "postgres://localhost/coral".to_string(),
            provider: "voyage-3".to_string(),
            dim: 1536,
        };
        let storage = PgvectorEmbeddingsStorage::new(config);
        // Access via trait methods
        assert_eq!(storage.provider(), "voyage-3");
        assert_eq!(storage.dim(), 1536);
    }

    #[test]
    fn backend_selection_returns_none_when_not_pgvector() {
        with_env_vars(
            &[(ENV_BACKEND, Some("sqlite")), (ENV_PGVECTOR_URL, None)],
            || {
                let result = select_pgvector_backend().unwrap();
                assert!(result.is_none());
            },
        );
    }

    #[test]
    fn backend_selection_returns_some_when_pgvector() {
        with_env_vars(
            &[
                (ENV_BACKEND, Some("pgvector")),
                (
                    ENV_PGVECTOR_URL,
                    Some("postgres://user:pass@localhost:5432/coral"),
                ),
            ],
            || {
                let result = select_pgvector_backend().unwrap();
                assert!(result.is_some());
                let storage = result.unwrap();
                assert_eq!(storage.provider(), "voyage-3");
                assert_eq!(storage.url(), "postgres://user:pass@localhost:5432/coral");
            },
        );
    }

    #[test]
    fn config_validation_rejects_missing_url() {
        with_env_vars(
            &[(ENV_BACKEND, Some("pgvector")), (ENV_PGVECTOR_URL, None)],
            || {
                let err = select_pgvector_backend().unwrap_err();
                let msg = err.to_string();
                assert!(
                    msg.contains(ENV_PGVECTOR_URL),
                    "error should mention env var: {msg}"
                );
            },
        );
    }

    #[test]
    fn config_validation_rejects_malformed_url() {
        with_env_vars(
            &[
                (ENV_BACKEND, Some("pgvector")),
                (ENV_PGVECTOR_URL, Some("http://not-a-pg-url")),
            ],
            || {
                let err = select_pgvector_backend().unwrap_err();
                let msg = err.to_string();
                assert!(
                    msg.contains("postgres://"),
                    "error should mention expected scheme: {msg}"
                );
            },
        );
    }

    #[test]
    fn config_validation_rejects_empty_url() {
        with_env_vars(
            &[
                (ENV_BACKEND, Some("pgvector")),
                (ENV_PGVECTOR_URL, Some("")),
            ],
            || {
                let err = select_pgvector_backend().unwrap_err();
                let msg = err.to_string();
                assert!(msg.contains("empty"), "error should mention empty: {msg}");
            },
        );
    }

    #[test]
    fn stub_methods_return_errors() {
        let config = PgvectorConfig {
            url: "postgres://localhost/coral".to_string(),
            provider: "voyage-3".to_string(),
            dim: 1536,
        };
        let mut storage = PgvectorEmbeddingsStorage::new(config);

        // is_fresh → Err
        assert!(storage.is_fresh("test", 123).is_err());

        // upsert → Err
        assert!(storage.upsert("test", 123, vec![0.0; 1536]).is_err());

        // prune → Err
        let live = HashSet::new();
        assert!(storage.prune(&live).is_err());

        // search → Err
        assert!(storage.search(&[0.0; 1536], 5).is_err());

        // flush → Ok (no-op).
        // v0.30.0 audit cycle 5 B10: use `.expect` so a future
        // regression surfaces the actual `Err` variant in the CI log
        // rather than the opaque "assertion failed: ...is_ok()".
        storage.flush().expect("flush is a no-op and must succeed");
    }

    #[test]
    fn backend_selection_returns_none_when_env_unset() {
        with_env_vars(&[(ENV_BACKEND, None), (ENV_PGVECTOR_URL, None)], || {
            let result = select_pgvector_backend().unwrap();
            assert!(result.is_none());
        });
    }
}
