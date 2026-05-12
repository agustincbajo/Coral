//! Token-and-USD cost model for `coral bootstrap --estimate`.
//!
//! v0.34.0 (M1) — FR-ONB-12, FR-ONB-13, FR-ONB-29.
//!
//! ## What this module does
//!
//! Two pure, side-effect-free building blocks:
//!
//! 1. [`estimate_tokens_for_entry`]: heuristic per-page token estimate
//!    from a small [`PlanEntryEstimate`] descriptor (slug, body
//!    length, type). Used by `coral bootstrap --estimate` to project
//!    cost BEFORE making any LLM call, and also as the fallback when
//!    a runner returns `usage: None` mid-flight.
//!
//! 2. [`estimate_cost_from_tokens`]: pricing lookup from
//!    `(input_tokens, output_tokens, Provider)` → a [`CostEstimate`]
//!    with `usd_estimate`, `usd_upper_bound` (= estimate × 1.25 in
//!    M1), and `margin_of_error_pct: 25`.
//!
//! ## Why these numbers
//!
//! - Anthropic Sonnet 4.5: `$3 / MTok` input + `$15 / MTok` output —
//!   the public list price at v0.34.0 cut (May 2026). Pinned here so
//!   a price drop is a one-line change with a test that confirms
//!   `usd_estimate` did move in the right direction.
//! - Gemini 2.0 Flash: `$0.10 / MTok` input + `$0.40 / MTok` output —
//!   ~30× cheaper than Sonnet, which is why we surface it as the
//!   "if you're cost-sensitive, switch provider" option in the
//!   bootstrap skill.
//! - Local + HTTP: `$0` — local LLMs have no per-call cost; HTTP is
//!   user-configured (we don't know the endpoint's pricing model)
//!   and conservatively treated as free in M1. The bootstrap path
//!   surfaces "actual cost may vary; we have no model for your
//!   endpoint" when `--provider http`.
//! - `usd_upper_bound = estimate × 1.25` (margin ±25%): until we
//!   have ≥30 calibration runs (deferred to M2), the spread is the
//!   conservative bound. PRD §11 decision #3 fallback target.
//!
//! ## What this module does NOT do
//!
//! - Doesn't talk to the network (no live price API ping).
//! - Doesn't read `.coral/config.toml` — caller wires thresholds.
//! - Doesn't model prompt caching (M2 — PRD §7.2 note). The
//!   conservative no-caching upper bound is reported; the skill
//!   message tells the user real cost can be 30–50% lower.

use serde::{Deserialize, Serialize};

/// Cost estimate returned by [`estimate_cost_from_tokens`].
///
/// `usd_upper_bound` is the conservative ceiling we surface to the
/// user — `coral bootstrap --max-cost` gates pre-flight on this, not
/// on `usd_estimate`. PRD FR-ONB-29.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CostEstimate {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Mid-point USD estimate (provider price list × token counts).
    pub usd_estimate: f64,
    /// `usd_estimate × 1.25` in M1 (±25% margin). M2 may recalibrate
    /// once ≥30 real runs are collected.
    pub usd_upper_bound: f64,
    /// Pinned at 25 (percent) for M1. Surface as `±NN%` in the
    /// skill output.
    pub margin_of_error_pct: u8,
}

impl CostEstimate {
    /// Zero-cost zero-token estimate. Used as the identity for sums
    /// and as the trivial result when the plan is empty.
    pub fn zero() -> Self {
        Self {
            input_tokens: 0,
            output_tokens: 0,
            usd_estimate: 0.0,
            usd_upper_bound: 0.0,
            margin_of_error_pct: 25,
        }
    }
}

/// The four providers Coral knows how to estimate cost for. Mirror
/// of `coral_cli::commands::runner_helper::ProviderName` but kept in
/// `coral-core` so the cost model has no dependency on the CLI crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    /// Anthropic Claude (Sonnet 4.5 list price; via `claude --print`
    /// or the `anthropic` HTTP shim).
    Claude,
    /// Google Gemini (2.0 Flash list price; via the gemini-cli or
    /// the Gemini REST API).
    Gemini,
    /// Generic OpenAI-compatible HTTP endpoint. Cost is user-driven
    /// (we don't know their endpoint's pricing) → modeled as `$0`
    /// in M1. The skill flashes a "cost may vary" notice.
    Http,
    /// Local llama.cpp / Ollama via [`LocalRunner`]. `$0` — runs on
    /// the user's hardware.
    Local,
}

impl Provider {
    /// Human-readable label for the provider, used by `coral bootstrap
    /// --estimate` output. Matches the value of `--provider <name>`.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Claude => "claude-sonnet-4-5",
            Self::Gemini => "gemini-2.0-flash",
            Self::Http => "http",
            Self::Local => "local",
        }
    }
}

/// Per-page input descriptor for the [`estimate_tokens_for_entry`]
/// heuristic. Kept deliberately small so callers in `coral-cli` can
/// build it from a `PlanEntry` without importing the heavy plan
/// types into `coral-core`.
///
/// Fields:
/// - `body_len_chars`: length of any pre-existing draft body. For
///   bootstrap entries without a body, pass `0` — the heuristic
///   uses a default per-page token budget.
/// - `rationale_len_chars`: length of the LLM-emitted `rationale`
///   string. Folds into the input-token estimate.
#[derive(Debug, Clone, Copy)]
pub struct PlanEntryEstimate {
    pub body_len_chars: usize,
    pub rationale_len_chars: usize,
}

/// Heuristic `(input_tokens, output_tokens)` for one wiki page.
///
/// **Input** (what we feed the model for that page):
///
/// - Base prompt + system message: `1500` tokens (PRD §6.4 FR-ONB-13
///   default — the bootstrap prompt template is ~6 KB).
/// - `rationale` echoed back into the per-page prompt: `chars / 4`.
/// - `body` (when an existing draft is being refined): `chars / 4`.
///
/// **Output** (what we expect the model to write back):
///
/// - 800 tokens default — the calibration target picked by the
///   "average page is ~3 KB" sample from `v0.33.0` dogfood runs.
///   Will be re-pinned in M2 with ≥30 calibration runs.
///
/// Returns `(input_tokens, output_tokens)`.
pub fn estimate_tokens_for_entry(entry: &PlanEntryEstimate) -> (u64, u64) {
    /// Base prompt + system + plan-entry framing per page.
    const BASE_INPUT_TOKENS: u64 = 1500;
    /// Average output tokens for one page body.
    const AVG_OUTPUT_TOKENS: u64 = 800;
    let extra_input = ((entry.body_len_chars + entry.rationale_len_chars) as u64).div_ceil(4);
    let input = BASE_INPUT_TOKENS.saturating_add(extra_input);
    (input, AVG_OUTPUT_TOKENS)
}

/// USD cost for `(input_tokens, output_tokens)` on the given
/// `provider`. The returned `usd_upper_bound = usd_estimate × 1.25`.
///
/// Pricing is pinned per [`Provider`]; see module-level docs for
/// the rationale + sources.
pub fn estimate_cost_from_tokens(
    input_tokens: u64,
    output_tokens: u64,
    provider: Provider,
) -> CostEstimate {
    let (input_rate_per_mtok, output_rate_per_mtok) = match provider {
        Provider::Claude => (3.0, 15.0),
        Provider::Gemini => (0.10, 0.40),
        Provider::Http | Provider::Local => (0.0, 0.0),
    };
    let usd_estimate = (input_tokens as f64) * input_rate_per_mtok / 1_000_000.0
        + (output_tokens as f64) * output_rate_per_mtok / 1_000_000.0;
    CostEstimate {
        input_tokens,
        output_tokens,
        usd_estimate,
        usd_upper_bound: usd_estimate * 1.25,
        margin_of_error_pct: 25,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sonnet 4.5 pricing: $3 / MTok input + $15 / MTok output.
    /// 1M input + 1M output = $18.00.
    #[test]
    fn claude_pricing_matches_sonnet_4_5_list() {
        let est = estimate_cost_from_tokens(1_000_000, 1_000_000, Provider::Claude);
        assert!(
            (est.usd_estimate - 18.0).abs() < 1e-9,
            "Claude 1M+1M must be $18.00, got ${}",
            est.usd_estimate
        );
    }

    /// Gemini 2.0 Flash: $0.10 / MTok in + $0.40 / MTok out.
    /// 1M + 1M = $0.50.
    #[test]
    fn gemini_pricing_matches_flash_list() {
        let est = estimate_cost_from_tokens(1_000_000, 1_000_000, Provider::Gemini);
        assert!(
            (est.usd_estimate - 0.50).abs() < 1e-9,
            "Gemini 1M+1M must be $0.50, got ${}",
            est.usd_estimate
        );
    }

    /// Local + HTTP: zero cost in M1.
    #[test]
    fn local_and_http_are_free_in_m1() {
        for p in [Provider::Local, Provider::Http] {
            let est = estimate_cost_from_tokens(123, 456, p);
            assert_eq!(est.usd_estimate, 0.0, "provider {p:?} must be free");
            assert_eq!(est.usd_upper_bound, 0.0);
        }
    }

    /// FR-ONB-12: upper bound is exactly `estimate × 1.25` for every
    /// provider (including the zero-cost case).
    #[test]
    fn upper_bound_is_125_percent_of_estimate() {
        for p in [
            Provider::Claude,
            Provider::Gemini,
            Provider::Local,
            Provider::Http,
        ] {
            let est = estimate_cost_from_tokens(500_000, 200_000, p);
            let expected_upper = est.usd_estimate * 1.25;
            assert!(
                (est.usd_upper_bound - expected_upper).abs() < 1e-9,
                "{p:?}: upper bound {} != estimate {} * 1.25",
                est.usd_upper_bound,
                est.usd_estimate
            );
            assert_eq!(est.margin_of_error_pct, 25);
        }
    }

    /// Edge case: zero input tokens → only output-side cost.
    #[test]
    fn zero_input_tokens_yields_output_only_cost() {
        let est = estimate_cost_from_tokens(0, 1_000_000, Provider::Claude);
        assert!(
            (est.usd_estimate - 15.0).abs() < 1e-9,
            "0 input + 1M output Claude = $15.00, got ${}",
            est.usd_estimate
        );
        assert!(
            (est.usd_upper_bound - 15.0 * 1.25).abs() < 1e-9,
            "upper bound off"
        );
    }

    /// Edge case: zero output tokens → only input-side cost.
    #[test]
    fn zero_output_tokens_yields_input_only_cost() {
        let est = estimate_cost_from_tokens(1_000_000, 0, Provider::Claude);
        assert!(
            (est.usd_estimate - 3.0).abs() < 1e-9,
            "1M input + 0 output Claude = $3.00, got ${}",
            est.usd_estimate
        );
    }

    /// Edge case: zero+zero is zero — used as the identity for plan
    /// rollups (`CostEstimate::zero` matches).
    #[test]
    fn zero_zero_is_zero_for_every_provider() {
        for p in [
            Provider::Claude,
            Provider::Gemini,
            Provider::Local,
            Provider::Http,
        ] {
            let est = estimate_cost_from_tokens(0, 0, p);
            assert_eq!(est.usd_estimate, 0.0);
            assert_eq!(est.usd_upper_bound, 0.0);
        }
    }

    /// FR-ONB-13: heuristic is monotonic in `body + rationale`
    /// length — longer drafts cost more input tokens. The
    /// constant-output assumption holds across entry sizes.
    #[test]
    fn token_estimate_grows_with_input_length() {
        let small = PlanEntryEstimate {
            body_len_chars: 0,
            rationale_len_chars: 100,
        };
        let big = PlanEntryEstimate {
            body_len_chars: 4_000,
            rationale_len_chars: 100,
        };
        let (small_in, _) = estimate_tokens_for_entry(&small);
        let (big_in, big_out) = estimate_tokens_for_entry(&big);
        assert!(big_in > small_in, "longer body must cost more input tokens");
        assert_eq!(big_out, 800, "output budget is constant per page");
    }

    /// Smoke: provider labels match what the skill prints.
    #[test]
    fn provider_labels_are_stable() {
        assert_eq!(Provider::Claude.label(), "claude-sonnet-4-5");
        assert_eq!(Provider::Gemini.label(), "gemini-2.0-flash");
        assert_eq!(Provider::Local.label(), "local");
        assert_eq!(Provider::Http.label(), "http");
    }
}
