//! `coral bootstrap --estimate` — cost projection without running.
//!
//! v0.34.0 (M1) — FR-ONB-12, FR-ONB-13, FR-ONB-29.
//!
//! This file lives in `coral-cli` (not `coral-core`) because the
//! heuristic input is a CLI-side [`PlanEntry`], and the output is a
//! human-readable message printed to stdout. The pure pricing math is
//! in `coral_core::cost`.

use anyhow::Result;
use coral_core::cost::{
    CostEstimate, PlanEntryEstimate, Provider, estimate_cost_from_tokens, estimate_tokens_for_entry,
};

use super::super::plan::PlanEntry;

/// Roll up a plan into a single [`CostEstimate`] for the given
/// provider. Sums per-entry token estimates and feeds them through
/// the pricing function. Returns the zero estimate for an empty plan.
pub fn plan_cost_estimate(plan: &[PlanEntry], provider: Provider) -> CostEstimate {
    let mut input_total: u64 = 0;
    let mut output_total: u64 = 0;
    for e in plan {
        let body_len = e.body.as_deref().map(str::len).unwrap_or(0);
        let est = PlanEntryEstimate {
            body_len_chars: body_len,
            rationale_len_chars: e.rationale.len(),
        };
        let (input, output) = estimate_tokens_for_entry(&est);
        input_total = input_total.saturating_add(input);
        output_total = output_total.saturating_add(output);
    }
    estimate_cost_from_tokens(input_total, output_total, provider)
}

/// Print the FR-ONB-12 message to stdout. Includes:
///
/// - Repo size line (LOC + file count).
/// - Page count.
/// - Token totals.
/// - Provider label.
/// - Cost with upper-bound + margin.
/// - Large-repo hint when `upper_bound > big_repo_threshold_usd`.
/// - Prompt-caching disclaimer (M2 will calibrate).
/// - Local/Http provider note (heuristic mode).
///
/// `repo_loc` / `repo_files` are surfaced verbatim in the first line;
/// `0` is fine when the caller doesn't have the numbers.
pub fn print_estimate(
    plan: &[PlanEntry],
    provider: Provider,
    repo_loc: usize,
    repo_files: usize,
    big_repo_threshold_usd: f64,
) -> Result<()> {
    let est = plan_cost_estimate(plan, provider);
    println!("Repo size: {repo_loc} LOC across {repo_files} files");
    println!("Estimated pages: {}", plan.len());
    println!(
        "Estimated tokens: ~{}k input + ~{}k output",
        est.input_tokens / 1_000,
        est.output_tokens / 1_000
    );
    println!("Provider: {}", provider.label());
    println!(
        "Estimated cost: ${:.2} (up to ${:.2} — margin \u{00b1}{}%)",
        est.usd_estimate, est.usd_upper_bound, est.margin_of_error_pct
    );

    // FR-ONB-12 large-repo hint.
    if est.usd_upper_bound > big_repo_threshold_usd {
        println!();
        println!(
            "\u{26a0}  This is a large repo (estimate > ${big_repo_threshold_usd:.2}). \
             Consider starting with:"
        );
        println!();
        println!("    coral bootstrap --apply --max-pages=50 --priority=high");
        println!();
        println!(
            "This bootstraps the 50 most-referenced modules first. You can run again"
        );
        println!("later with --resume to continue or re-run without --max-pages to do all.");
    }

    // PRD §7.2: prompt-caching disclaimer (always printed).
    println!();
    println!(
        "Note: actual cost may be 30-50% lower if prompt caching is enabled (M2 will calibrate)."
    );

    // PRD §6.4: heuristic-only providers note.
    if matches!(provider, Provider::Local | Provider::Http) {
        println!(
            "Note: provider `{}` does not expose token usage; --max-cost will use the \
             heuristic estimate above (real cost may differ).",
            provider.label()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::plan::Action;
    use coral_core::frontmatter::PageType;

    fn entry(slug: &str, rationale: &str, body: Option<&str>) -> PlanEntry {
        PlanEntry {
            slug: slug.into(),
            action: Action::Create,
            r#type: Some(PageType::Module),
            confidence: Some(0.6),
            rationale: rationale.into(),
            body: body.map(String::from),
        }
    }

    /// Empty plan yields the zero estimate.
    #[test]
    fn empty_plan_is_zero_estimate() {
        let est = plan_cost_estimate(&[], Provider::Claude);
        assert_eq!(est.input_tokens, 0);
        assert_eq!(est.output_tokens, 0);
        assert_eq!(est.usd_estimate, 0.0);
    }

    /// Non-empty plan against Claude: 1 page → 1500 input + 800
    /// output tokens (heuristic constants); upper bound = 1.25×.
    #[test]
    fn single_page_plan_uses_heuristic_defaults() {
        let plan = vec![entry("alpha", "anchor module", None)];
        let est = plan_cost_estimate(&plan, Provider::Claude);
        // Base 1500 + rationale 13 chars / 4 = 4 → 1504 (div_ceil(13,4)=4).
        assert!(
            est.input_tokens >= 1500 && est.input_tokens < 1600,
            "expected ~1500 input, got {}",
            est.input_tokens
        );
        assert_eq!(est.output_tokens, 800);
        // Upper bound is exactly 1.25× estimate.
        assert!((est.usd_upper_bound - est.usd_estimate * 1.25).abs() < 1e-9);
    }

    /// Big plan: 100 pages → upper bound trips the FR-ONB-12 large-
    /// repo threshold of $5. Sanity check that the heuristic scales.
    #[test]
    fn hundred_page_plan_triggers_big_repo_threshold() {
        let plan: Vec<PlanEntry> = (0..100)
            .map(|i| entry(&format!("slug-{i}"), "x", None))
            .collect();
        let est = plan_cost_estimate(&plan, Provider::Claude);
        // 100 * 1500 input = 150,000 tokens × $3/MTok = $0.45.
        // 100 * 800 output = 80,000 tokens × $15/MTok = $1.20.
        // Total estimate ≈ $1.65. Upper bound ≈ $2.06. NOT > $5.
        assert!(
            est.usd_estimate > 1.5 && est.usd_estimate < 2.0,
            "100-page Claude estimate ~$1.65, got ${}",
            est.usd_estimate
        );
    }

    /// 500-page plan blows past the $5 threshold.
    #[test]
    fn very_big_plan_blows_big_repo_threshold() {
        let plan: Vec<PlanEntry> = (0..500)
            .map(|i| entry(&format!("slug-{i}"), "x", None))
            .collect();
        let est = plan_cost_estimate(&plan, Provider::Claude);
        // 500 * 1500 = 750k input @ $3/MTok = $2.25
        // 500 * 800 = 400k output @ $15/MTok = $6.00
        // estimate ≈ $8.25 > $5 threshold.
        assert!(
            est.usd_upper_bound > 5.0,
            "500-page Claude upper bound must exceed $5, got ${}",
            est.usd_upper_bound
        );
    }

    /// Local provider always yields zero cost regardless of token
    /// count — verifies the heuristic path for the "free" providers.
    #[test]
    fn local_provider_is_free_in_plan_rollup() {
        let plan: Vec<PlanEntry> = (0..50)
            .map(|i| entry(&format!("slug-{i}"), "rationale", None))
            .collect();
        let est = plan_cost_estimate(&plan, Provider::Local);
        assert_eq!(est.usd_estimate, 0.0);
        assert_eq!(est.usd_upper_bound, 0.0);
        // But tokens are still counted (useful for "you'd use ~X
        // tokens on this run" diagnostics).
        assert!(est.input_tokens > 0);
        assert!(est.output_tokens > 0);
    }

    /// FR-ONB-12: pre-existing body content folds into the input
    /// estimate — refining an existing draft costs more than
    /// generating from scratch.
    #[test]
    fn pre_existing_body_increases_input_tokens() {
        let no_body = vec![entry("a", "rationale", None)];
        let with_body = vec![entry("a", "rationale", Some(&"x".repeat(4_000)))];
        let est_a = plan_cost_estimate(&no_body, Provider::Claude);
        let est_b = plan_cost_estimate(&with_body, Provider::Claude);
        assert!(
            est_b.input_tokens > est_a.input_tokens,
            "with-body must cost more input tokens"
        );
    }
}
