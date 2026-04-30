//! Coral lint: structural + semantic checks for wiki pages.

pub mod report;
pub mod semantic;
pub mod structural;

pub use report::{LintCode, LintIssue, LintReport, LintSeverity};

use coral_core::page::Page;

/// Function signature for a structural lint check.
type StructuralCheck = fn(&[Page]) -> Vec<LintIssue>;

/// Runs all structural checks in parallel and returns a consolidated report.
pub fn run_structural(pages: &[Page]) -> LintReport {
    use rayon::prelude::*;
    let checks: Vec<StructuralCheck> = vec![
        structural::check_broken_wikilinks,
        structural::check_orphan_pages,
        structural::check_low_confidence,
        structural::check_high_confidence_without_sources,
        structural::check_stale_status,
    ];
    let issues: Vec<LintIssue> = checks
        .par_iter()
        .flat_map_iter(|check| check(pages))
        .collect();
    LintReport::from_issues(issues)
}
