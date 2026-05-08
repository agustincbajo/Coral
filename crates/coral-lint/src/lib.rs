//! Coral lint: structural + semantic checks for wiki pages.

pub mod report;
pub mod semantic;
pub mod structural;

pub use report::{LintCode, LintIssue, LintReport, LintSeverity};

use coral_core::page::Page;
use std::path::Path;

/// Function signature for a structural lint check that needs only the page
/// graph.
type StructuralCheck = fn(&[Page]) -> Vec<LintIssue>;

/// Function signature for a structural check that also needs to know the
/// repo root (for git / filesystem lookups).
type StructuralCheckWithRoot = fn(&[Page], &Path) -> Vec<LintIssue>;

/// Runs all structural checks in parallel and returns a consolidated report.
///
/// This is the legacy entry point — kept for backward compatibility. It
/// defaults `repo_root` to `.` so the two context-aware checks
/// (`check_commit_in_git`, `check_source_exists`) still run. They degrade
/// gracefully if git is unavailable or the cwd is wrong.
///
/// New callers should prefer [`run_structural_with_root`] and pass the actual
/// repo root explicitly (the parent of `.wiki/`).
pub fn run_structural(pages: &[Page]) -> LintReport {
    run_structural_with_root(pages, Path::new("."))
}

/// Runs all structural checks in parallel against `pages`, using `repo_root`
/// for the context-aware checks (commit-in-git, source-exists). Returns a
/// consolidated, sorted [`LintReport`].
///
/// All 9 checks fan out via rayon. Pure checks operate on `&[Page]` only;
/// context-aware checks additionally borrow `repo_root`.
pub fn run_structural_with_root(pages: &[Page], repo_root: &Path) -> LintReport {
    use rayon::prelude::*;

    let pure_checks: Vec<StructuralCheck> = vec![
        structural::check_broken_wikilinks,
        structural::check_orphan_pages,
        structural::check_low_confidence,
        structural::check_high_confidence_without_sources,
        structural::check_stale_status,
        structural::check_archived_linked_from_head,
        structural::check_unknown_extra_field,
        // v0.20.0: trust-by-curation gate for `coral session distill`
        // output. Critical so the pre-commit hook blocks any
        // `reviewed: false` page from being committed.
        structural::check_unreviewed_distilled,
    ];
    let context_checks: Vec<StructuralCheckWithRoot> = vec![
        structural::check_commit_in_git,
        structural::check_source_exists,
    ];

    let pure_issues: Vec<LintIssue> = pure_checks
        .par_iter()
        .flat_map_iter(|check| check(pages))
        .collect();
    let context_issues: Vec<LintIssue> = context_checks
        .par_iter()
        .flat_map_iter(|check| check(pages, repo_root))
        .collect();

    let mut issues = pure_issues;
    issues.extend(context_issues);
    LintReport::from_issues(issues)
}
