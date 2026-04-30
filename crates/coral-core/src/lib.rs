//! Core types and utilities for Coral.

pub mod error;
pub mod frontmatter;
pub mod wikilinks;

/// Returns the crate version (CARGO_PKG_VERSION).
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_non_empty() {
        assert!(!version().is_empty());
    }
}
