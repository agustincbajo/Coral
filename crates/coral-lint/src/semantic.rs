//! Semantic lint — uses the LLM runner to detect contradictions, obsolete
//! claims, and other content issues that structural checks can't catch.
//!
//! Implemented in Phase D once `coral-runner` is ready. For now, returns
//! an empty issue list.

use crate::report::LintIssue;
use coral_core::page::Page;

/// Placeholder. Phase D will accept a `Runner` and return real issues.
pub fn check_semantic(_pages: &[Page]) -> Vec<LintIssue> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_stub_returns_empty() {
        assert!(check_semantic(&[]).is_empty());
    }
}
