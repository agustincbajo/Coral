//! Error types for Coral.

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoralError {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("yaml parse error: {0}")]
    Yaml(#[from] serde_yaml_ng::Error),

    #[error("frontmatter missing — expected leading `---` block in {path}")]
    MissingFrontmatter { path: PathBuf },

    #[error("frontmatter unterminated — expected closing `---` in {path}")]
    UnterminatedFrontmatter { path: PathBuf },

    #[error("invalid confidence value {0}: must be in [0.0, 1.0]")]
    InvalidConfidence(f64),

    #[error("invalid page type: {0}")]
    InvalidPageType(String),

    #[error("invalid status: {0}")]
    InvalidStatus(String),

    #[error("git error: {0}")]
    Git(String),

    #[error("walk error: {0}")]
    Walk(String),
}

pub type Result<T> = std::result::Result<T, CoralError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_confidence_display_includes_value_and_range() {
        let err = CoralError::InvalidConfidence(1.5);
        let msg = err.to_string();
        assert!(msg.contains("1.5"), "msg should contain value: {msg}");
        assert!(
            msg.contains("[0.0, 1.0]"),
            "msg should contain range: {msg}"
        );
    }
}
