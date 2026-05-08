//! Multi-step / tiered runner abstraction (v0.21.4).
//!
//! Lets a single logical "run" be decomposed into a planner →
//! executor(s) → reviewer pipeline, with each tier potentially backed
//! by a different model. Concrete impl is [`TieredRunner`]. The
//! [`MultiStepRunner`] trait is the seam so future impls (e.g.
//! debate-style or critic-loop) can be plugged in without churning
//! callers.
//!
//! Design notes — see orchestrator spec §6:
//! - **D1** module location: `coral-runner/src/multi_step.rs`.
//! - **D2** single end-to-end method (`run_tiered`) rather than three
//!   domain-shaped methods. Callers should not have to understand the
//!   internal staging.
//! - **D4** budget = pure-Rust `len/4` token approximation, multiplied
//!   by `1.5x` for the planner pre-flight (the executor and reviewer
//!   prompts are unknown until we have the planner output, so we
//!   gate post-hoc per call). Zero new dependencies.
//! - **D6** every existing `Runner` impl compiles unchanged because
//!   `MultiStepRunner` is a separate trait that delegates to a
//!   `Box<dyn Runner>` per tier.
//!
//! Per-step timeouts: this iteration does NOT introduce a tiered-level
//! timeout. The existing `Prompt::timeout` is per-call, so a tiered
//! run with three `Prompt::timeout = 60s` calls can take up to 180s
//! wall-clock. Documented in `docs/runner-tiered.md`.
//!
//! Streaming: `MultiStepRunner` is non-streaming end-to-end. Callers
//! that branch on `--stream` should fall back to non-streaming when
//! tiered routing is enabled.

use crate::runner::{Prompt, RunOutput, Runner, RunnerError, RunnerResult};

/// Default cumulative-token budget for a tiered run. Picked to match
/// a Claude Sonnet 200K-context window — anything larger is almost
/// certainly the user shoving an entire repo into the planner without
/// realising. Easy to override per-project via
/// `runner.tiered.budget.max_tokens_per_run` in `coral.toml`.
pub const DEFAULT_MAX_TOKENS_PER_RUN: u64 = 200_000;

/// System prompt for the planner tier. Decomposes a high-level
/// instruction into 1–5 sub-tasks emitted as YAML. Kept tight so the
/// planner doesn't waffle.
pub(crate) const PLANNER_SYSTEM: &str = "You are a planning agent. Decompose the user's task into 1-5 concrete sub-tasks. Output valid YAML only, no prose.";

/// System prompt for each executor invocation. One sub-task per call.
pub(crate) const EXECUTOR_SYSTEM: &str = "You are an executor agent. Carry out the single sub-task you are given. Be concise and specific.";

/// System prompt for the reviewer tier. Stitches sub-task outputs into
/// the final answer the caller sees in `TieredOutput::final_output`.
pub(crate) const REVIEWER_SYSTEM: &str = "You are a reviewer agent. Synthesize the sub-task results into a single final answer that satisfies the original task. Output the final answer only.";

/// A multi-step runner: takes the same `Prompt` shape as
/// [`Runner::run`] but invokes a planner → executor → reviewer
/// pipeline under the hood.
///
/// `Send + Sync` for the same reason as [`Runner`]: callers that store
/// runners on a long-lived `Arc<dyn …>` need to share it across
/// threads.
pub trait MultiStepRunner: Send + Sync {
    fn run_tiered(&self, prompt: &Prompt) -> RunnerResult<TieredOutput>;
}

/// Captured output of a single tiered run.
///
/// `final_output` is the reviewer's `RunOutput`. `plan_calls`,
/// `execute_calls`, and `review_calls` carry the per-tier
/// `RunOutput`s in the order they were produced — useful for tests
/// and for `--verbose` summaries.
///
/// `tokens_used` is the cumulative `len/4` estimate of every system +
/// user + stdout string traversed during the run. It's an
/// approximation, not a billing-accurate count.
#[derive(Debug, Clone)]
pub struct TieredOutput {
    pub final_output: RunOutput,
    pub plan_calls: Vec<RunOutput>,
    pub execute_calls: Vec<RunOutput>,
    pub review_calls: Vec<RunOutput>,
    pub tokens_used: u64,
}

/// Configuration for a tiered run. One [`TierSpec`] per tier plus a
/// shared [`BudgetConfig`].
#[derive(Debug, Clone)]
pub struct TieredConfig {
    pub planner: TierSpec,
    pub executor: TierSpec,
    pub reviewer: TierSpec,
    pub budget: BudgetConfig,
}

/// Per-tier provider selection. `provider` is informational at this
/// layer (the actual `Runner` impl is wired by the caller via
/// [`TieredRunner::new`]) but `model` is threaded into each
/// [`Prompt::model`] so per-tier model overrides Just Work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierSpec {
    pub provider: String,
    pub model: Option<String>,
}

/// Cumulative token budget for one tiered run. `max_tokens_per_run`
/// applies to the SUM of system + user + stdout across all tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetConfig {
    pub max_tokens_per_run: u64,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_tokens_per_run: DEFAULT_MAX_TOKENS_PER_RUN,
        }
    }
}

/// Pure-Rust token estimator: `chars / 4`, ceiling-divided. Picked
/// over a tiktoken-style BPE counter to keep the workspace
/// dependency-free (the orchestrator's spec §6 D4 rejected provider-
/// specific counters). A four-char-per-token rule is the common rule
/// of thumb across English text and source code; the budget gate
/// exists to catch order-of-magnitude blowups, not to be billing-
/// accurate.
pub fn approx_tokens(s: &str) -> u64 {
    (s.len() as u64).div_ceil(4)
}

/// Sum the token estimates of a `RunOutput` against its prompt: the
/// system + user prompt is the input cost; the stdout is the output
/// cost. We don't track stderr because it's typically empty on
/// success.
fn output_token_cost(prompt: &Prompt, out: &RunOutput) -> u64 {
    let system_cost = prompt.system.as_deref().map(approx_tokens).unwrap_or(0);
    system_cost + approx_tokens(&prompt.user) + approx_tokens(&out.stdout)
}

/// The concrete tiered runner. Holds three `Box<dyn Runner>` (one per
/// tier) plus a [`TieredConfig`].
///
/// Construct via [`TieredRunner::new`]. Each tier can be a different
/// `Runner` impl — e.g. a fast cheap planner on a `GeminiRunner` and
/// a strong reviewer on a `ClaudeRunner`.
pub struct TieredRunner {
    planner: Box<dyn Runner>,
    executor: Box<dyn Runner>,
    reviewer: Box<dyn Runner>,
    config: TieredConfig,
}

impl std::fmt::Debug for TieredRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TieredRunner")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl TieredRunner {
    pub fn new(
        planner: Box<dyn Runner>,
        executor: Box<dyn Runner>,
        reviewer: Box<dyn Runner>,
        config: TieredConfig,
    ) -> Self {
        Self {
            planner,
            executor,
            reviewer,
            config,
        }
    }
}

/// Single sub-task as parsed from the planner's YAML.
#[derive(Debug, Clone, serde::Deserialize)]
struct SubTask {
    #[allow(dead_code)] // `id` is parsed for future round-tripping but not consumed today.
    #[serde(default)]
    id: Option<String>,
    description: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct PlanDoc {
    #[serde(default)]
    subtasks: Vec<SubTask>,
}

/// Try to parse the planner's stdout as a `subtasks:` YAML doc. Strips
/// a leading ```yaml fence if present. Returns `Some(...)` on success
/// with at least one sub-task, `None` on parse failure or empty list —
/// the caller should fall back to a single-subtask = original-prompt
/// pipeline in that case (see `TieredRunner::run_tiered`).
fn parse_plan_yaml(raw: &str) -> Option<Vec<SubTask>> {
    let trimmed = strip_yaml_fence(raw);
    let doc: PlanDoc = serde_yaml_ng::from_str(trimmed).ok()?;
    if doc.subtasks.is_empty() {
        return None;
    }
    Some(doc.subtasks)
}

/// Strip a leading ```yaml ... ``` markdown fence, if present. Mirrors
/// the helper used by `coral consolidate` for the same parser-tolerance
/// reason — chat-style models like to wrap structured output in fences.
fn strip_yaml_fence(s: &str) -> &str {
    let s = s.trim();
    let s = s
        .strip_prefix("```yaml")
        .or_else(|| s.strip_prefix("```yml"))
        .or_else(|| s.strip_prefix("```"))
        .unwrap_or(s);
    s.trim_end_matches("```").trim()
}

impl MultiStepRunner for TieredRunner {
    fn run_tiered(&self, prompt: &Prompt) -> RunnerResult<TieredOutput> {
        let original_user = prompt.user.clone();
        let budget = self.config.budget.max_tokens_per_run;
        let mut tokens_used: u64 = 0;

        // ---- Pre-flight: budget gate on the planner prompt --------
        // The executor and reviewer prompts depend on the planner's
        // output, so we can't estimate the full pipeline up-front.
        // Instead we apply a 1.5× planner-prompt projection: if even
        // the planner alone is projected to blow the budget, abort
        // before issuing any network call.
        let planner_user = format!(
            "Decompose into 1-5 sub-tasks. Output YAML:\nsubtasks:\n  - id: ...\n    description: ...\n\nTask:\n{original_user}"
        );
        let planner_prompt = Prompt {
            system: Some(PLANNER_SYSTEM.to_string()),
            user: planner_user,
            model: self.config.planner.model.clone(),
            cwd: prompt.cwd.clone(),
            timeout: prompt.timeout,
        };
        let planner_estimate = approx_tokens(planner_prompt.system.as_deref().unwrap_or(""))
            + approx_tokens(&planner_prompt.user);
        let projected = planner_estimate.saturating_mul(3) / 2; // *1.5
        if projected > budget {
            return Err(RunnerError::BudgetExceeded {
                actual: projected,
                budget,
            });
        }

        // ---- Plan call --------------------------------------------
        let plan_out = self.planner.run(&planner_prompt)?;
        tokens_used = tokens_used.saturating_add(output_token_cost(&planner_prompt, &plan_out));

        // ---- Parse, with fallback ---------------------------------
        let subtasks = match parse_plan_yaml(&plan_out.stdout) {
            Some(s) => s,
            None => {
                tracing::warn!(
                    plan_stdout_len = plan_out.stdout.len(),
                    "tiered runner: planner output unparseable, falling back to single sub-task"
                );
                vec![SubTask {
                    id: Some("fallback-0".into()),
                    description: original_user.clone(),
                }]
            }
        };

        // ---- Executor calls (sequential) --------------------------
        let mut execute_calls: Vec<RunOutput> = Vec::with_capacity(subtasks.len());
        let mut joined_execute_outputs = String::new();
        for (i, st) in subtasks.iter().enumerate() {
            let exec_prompt = Prompt {
                system: Some(EXECUTOR_SYSTEM.to_string()),
                user: st.description.clone(),
                model: self.config.executor.model.clone(),
                cwd: prompt.cwd.clone(),
                timeout: prompt.timeout,
            };
            // Pre-flight budget check — if even feeding the next
            // executor prompt in pushes us over, abort cleanly.
            let projected = tokens_used.saturating_add(
                approx_tokens(exec_prompt.system.as_deref().unwrap_or(""))
                    + approx_tokens(&exec_prompt.user),
            );
            if projected > budget {
                return Err(RunnerError::BudgetExceeded {
                    actual: projected,
                    budget,
                });
            }
            let exec_out = self.executor.run(&exec_prompt)?;
            tokens_used = tokens_used.saturating_add(output_token_cost(&exec_prompt, &exec_out));
            if !joined_execute_outputs.is_empty() {
                joined_execute_outputs.push_str("\n\n---\n\n");
            }
            joined_execute_outputs.push_str(&format!("Sub-task {}: {}\n", i + 1, st.description));
            joined_execute_outputs.push_str(&exec_out.stdout);
            execute_calls.push(exec_out);
        }

        // ---- Reviewer call ----------------------------------------
        let reviewer_user = format!(
            "Original task:\n{original_user}\n\nSub-task results:\n{joined_execute_outputs}"
        );
        let reviewer_prompt = Prompt {
            system: Some(REVIEWER_SYSTEM.to_string()),
            user: reviewer_user,
            model: self.config.reviewer.model.clone(),
            cwd: prompt.cwd.clone(),
            timeout: prompt.timeout,
        };
        let projected = tokens_used.saturating_add(
            approx_tokens(reviewer_prompt.system.as_deref().unwrap_or(""))
                + approx_tokens(&reviewer_prompt.user),
        );
        if projected > budget {
            return Err(RunnerError::BudgetExceeded {
                actual: projected,
                budget,
            });
        }
        let review_out = self.reviewer.run(&reviewer_prompt)?;
        tokens_used = tokens_used.saturating_add(output_token_cost(&reviewer_prompt, &review_out));

        Ok(TieredOutput {
            final_output: review_out.clone(),
            plan_calls: vec![plan_out],
            execute_calls,
            review_calls: vec![review_out],
            tokens_used,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockRunner;

    /// Helper: a 3-runner config with model overrides per tier and a
    /// generous budget. Used as the baseline in most tests below.
    fn happy_path_config() -> TieredConfig {
        TieredConfig {
            planner: TierSpec {
                provider: "claude".into(),
                model: Some("haiku".into()),
            },
            executor: TierSpec {
                provider: "claude".into(),
                model: Some("sonnet".into()),
            },
            reviewer: TierSpec {
                provider: "claude".into(),
                model: Some("opus".into()),
            },
            budget: BudgetConfig {
                max_tokens_per_run: 1_000_000,
            },
        }
    }

    fn well_formed_plan() -> &'static str {
        "subtasks:\n  - id: t1\n    description: write a short plan\n  - id: t2\n    description: emit one fact\n"
    }

    /// AC-3: tiered runner invokes the three sub-runners in order.
    /// Asserting `.calls()` on each MockRunner requires sharing each
    /// mock across the test (post-`run_tiered`) and the constructed
    /// `Box<dyn Runner>` (during the run). We resolve that by wrapping
    /// each `Arc<MockRunner>` in a thin `ArcRunner` newtype, since
    /// downcasting `Box<dyn Runner>` back to `MockRunner` would force
    /// every `Runner` impl to opt into `Any` just to enable a single
    /// test-scaffolding pattern.
    #[test]
    fn tiered_runner_calls_three_tiers_in_order() {
        use std::sync::Arc;
        let planner_arc = Arc::new(MockRunner::new());
        planner_arc.push_ok(well_formed_plan());
        let executor_arc = Arc::new(MockRunner::new());
        executor_arc.push_ok("exec result 1");
        executor_arc.push_ok("exec result 2");
        let reviewer_arc = Arc::new(MockRunner::new());
        reviewer_arc.push_ok("FINAL");

        struct ArcRunner(Arc<MockRunner>);
        impl Runner for ArcRunner {
            fn run(&self, p: &Prompt) -> RunnerResult<RunOutput> {
                self.0.run(p)
            }
        }
        let tr = TieredRunner::new(
            Box::new(ArcRunner(planner_arc.clone())),
            Box::new(ArcRunner(executor_arc.clone())),
            Box::new(ArcRunner(reviewer_arc.clone())),
            happy_path_config(),
        );

        let prompt = Prompt {
            system: None,
            user: "build me a thing".into(),
            ..Default::default()
        };
        let out = tr.run_tiered(&prompt).expect("tiered run must succeed");

        // Planner saw exactly one call, with the wrapped user prompt.
        let p_calls = planner_arc.calls();
        assert_eq!(p_calls.len(), 1, "planner should be called exactly once");
        assert!(
            p_calls[0]
                .system
                .as_deref()
                .unwrap_or("")
                .contains("planning"),
            "planner system prompt should mention planning"
        );
        assert!(
            p_calls[0].user.contains("build me a thing"),
            "planner user prompt should embed the original task"
        );

        // Executor saw two calls (one per sub-task).
        let e_calls = executor_arc.calls();
        assert_eq!(
            e_calls.len(),
            2,
            "executor should be called once per sub-task"
        );
        assert_eq!(e_calls[0].user, "write a short plan");
        assert_eq!(e_calls[1].user, "emit one fact");

        // Reviewer saw exactly one call with both exec outputs in user prompt.
        let r_calls = reviewer_arc.calls();
        assert_eq!(r_calls.len(), 1, "reviewer should be called exactly once");
        assert!(r_calls[0].user.contains("Original task:"));
        assert!(r_calls[0].user.contains("build me a thing"));
        assert!(r_calls[0].user.contains("exec result 1"));
        assert!(r_calls[0].user.contains("exec result 2"));

        // AC-4: reviewer stdout becomes final_output.
        assert_eq!(out.final_output.stdout, "FINAL");
        assert_eq!(out.execute_calls.len(), 2);
        assert_eq!(out.plan_calls.len(), 1);
        assert_eq!(out.review_calls.len(), 1);
    }

    /// AC-3 + AC: per-tier model is threaded through each Prompt.
    #[test]
    fn tiered_runner_routes_model_per_tier() {
        use std::sync::Arc;
        let planner = Arc::new(MockRunner::new());
        planner.push_ok(well_formed_plan());
        let executor = Arc::new(MockRunner::new());
        executor.push_ok("e1");
        executor.push_ok("e2");
        let reviewer = Arc::new(MockRunner::new());
        reviewer.push_ok("done");

        struct ArcRunner(Arc<MockRunner>);
        impl Runner for ArcRunner {
            fn run(&self, p: &Prompt) -> RunnerResult<RunOutput> {
                self.0.run(p)
            }
        }
        let tr = TieredRunner::new(
            Box::new(ArcRunner(planner.clone())),
            Box::new(ArcRunner(executor.clone())),
            Box::new(ArcRunner(reviewer.clone())),
            happy_path_config(),
        );

        let prompt = Prompt {
            user: "x".into(),
            ..Default::default()
        };
        let _ = tr.run_tiered(&prompt).unwrap();

        assert_eq!(planner.calls()[0].model.as_deref(), Some("haiku"));
        for c in executor.calls() {
            assert_eq!(c.model.as_deref(), Some("sonnet"));
        }
        assert_eq!(reviewer.calls()[0].model.as_deref(), Some("opus"));
    }

    /// AC-8: pre-flight budget abort BEFORE any planner call.
    #[test]
    fn tiered_runner_budget_pre_flight_aborts() {
        use std::sync::Arc;
        let planner = Arc::new(MockRunner::new());
        // No push_ok — if the budget gate fires correctly the planner
        // must never see a call.
        let executor = Arc::new(MockRunner::new());
        let reviewer = Arc::new(MockRunner::new());

        struct ArcRunner(Arc<MockRunner>);
        impl Runner for ArcRunner {
            fn run(&self, p: &Prompt) -> RunnerResult<RunOutput> {
                self.0.run(p)
            }
        }

        let cfg = TieredConfig {
            planner: TierSpec {
                provider: "claude".into(),
                model: None,
            },
            executor: TierSpec {
                provider: "claude".into(),
                model: None,
            },
            reviewer: TierSpec {
                provider: "claude".into(),
                model: None,
            },
            budget: BudgetConfig {
                max_tokens_per_run: 100,
            },
        };
        let tr = TieredRunner::new(
            Box::new(ArcRunner(planner.clone())),
            Box::new(ArcRunner(executor.clone())),
            Box::new(ArcRunner(reviewer.clone())),
            cfg,
        );

        // 1000-char prompt → ~250 tokens. Times 1.5 = 375 > budget(100).
        let huge = "x".repeat(1000);
        let prompt = Prompt {
            user: huge,
            ..Default::default()
        };
        let err = tr.run_tiered(&prompt).expect_err("budget must reject");
        match err {
            RunnerError::BudgetExceeded { actual, budget } => {
                assert_eq!(budget, 100, "budget value preserved in error");
                assert!(
                    actual > budget,
                    "actual ({actual}) must exceed budget ({budget})"
                );
            }
            other => panic!("expected BudgetExceeded, got {other:?}"),
        }
        assert_eq!(
            planner.calls().len(),
            0,
            "planner must never be called when pre-flight rejects"
        );
        assert_eq!(executor.calls().len(), 0);
        assert_eq!(reviewer.calls().len(), 0);
    }

    /// AC-9: budget exceeded after planner but before executor →
    /// `BudgetExceeded` returned cleanly.
    #[test]
    fn tiered_runner_budget_mid_pipeline_aborts() {
        use std::sync::Arc;
        let planner = Arc::new(MockRunner::new());
        // Planner emits a HUGE stdout that drives tokens_used past
        // the budget for the *next* (executor) pre-flight check.
        let huge_stdout = format!(
            "subtasks:\n  - id: t1\n    description: tiny\n# pad: {}\n",
            "z".repeat(2000)
        );
        planner.push_ok(huge_stdout);
        let executor = Arc::new(MockRunner::new());
        let reviewer = Arc::new(MockRunner::new());

        struct ArcRunner(Arc<MockRunner>);
        impl Runner for ArcRunner {
            fn run(&self, p: &Prompt) -> RunnerResult<RunOutput> {
                self.0.run(p)
            }
        }

        let cfg = TieredConfig {
            planner: TierSpec {
                provider: "claude".into(),
                model: None,
            },
            executor: TierSpec {
                provider: "claude".into(),
                model: None,
            },
            reviewer: TierSpec {
                provider: "claude".into(),
                model: None,
            },
            // Budget chosen so the planner's prompt fits (planner_user ≈ 100
            // chars => ~25 tokens × 1.5 = 38 < 500), but the planner stdout
            // ≈ 2500 chars => ~625 tokens, which pushes tokens_used past 500.
            budget: BudgetConfig {
                max_tokens_per_run: 500,
            },
        };
        let tr = TieredRunner::new(
            Box::new(ArcRunner(planner.clone())),
            Box::new(ArcRunner(executor.clone())),
            Box::new(ArcRunner(reviewer.clone())),
            cfg,
        );

        let prompt = Prompt {
            user: "tiny task".into(),
            ..Default::default()
        };
        let err = tr.run_tiered(&prompt).expect_err("must blow budget");
        assert!(
            matches!(err, RunnerError::BudgetExceeded { .. }),
            "expected BudgetExceeded, got {err:?}"
        );
        assert_eq!(planner.calls().len(), 1, "planner should have been invoked");
        assert_eq!(
            executor.calls().len(),
            0,
            "executor must not be invoked once budget is blown"
        );
        assert_eq!(reviewer.calls().len(), 0);
    }

    /// AC-fallback: unparseable planner output falls back to single
    /// sub-task = original user prompt.
    #[test]
    fn tiered_runner_falls_back_on_unparseable_plan() {
        use std::sync::Arc;
        let planner = Arc::new(MockRunner::new());
        planner.push_ok("not yaml at all { also not yaml ::: ;;;");
        let executor = Arc::new(MockRunner::new());
        executor.push_ok("did the thing");
        let reviewer = Arc::new(MockRunner::new());
        reviewer.push_ok("STITCHED");

        struct ArcRunner(Arc<MockRunner>);
        impl Runner for ArcRunner {
            fn run(&self, p: &Prompt) -> RunnerResult<RunOutput> {
                self.0.run(p)
            }
        }
        let tr = TieredRunner::new(
            Box::new(ArcRunner(planner.clone())),
            Box::new(ArcRunner(executor.clone())),
            Box::new(ArcRunner(reviewer.clone())),
            happy_path_config(),
        );

        let prompt = Prompt {
            user: "do a thing".into(),
            ..Default::default()
        };
        let out = tr.run_tiered(&prompt).expect("fallback must succeed");
        assert_eq!(out.final_output.stdout, "STITCHED");
        // Single executor call with the *original* user prompt as
        // the sub-task description.
        let e_calls = executor.calls();
        assert_eq!(e_calls.len(), 1);
        assert_eq!(e_calls[0].user, "do a thing");
    }

    /// AC-error: an executor error aborts the run and bubbles up.
    #[test]
    fn tiered_runner_propagates_executor_error() {
        use std::sync::Arc;
        let planner = Arc::new(MockRunner::new());
        planner.push_ok(well_formed_plan());
        let executor = Arc::new(MockRunner::new());
        executor.push_err(RunnerError::NotFound);
        let reviewer = Arc::new(MockRunner::new());
        // Reviewer should NOT be called.

        struct ArcRunner(Arc<MockRunner>);
        impl Runner for ArcRunner {
            fn run(&self, p: &Prompt) -> RunnerResult<RunOutput> {
                self.0.run(p)
            }
        }
        let tr = TieredRunner::new(
            Box::new(ArcRunner(planner.clone())),
            Box::new(ArcRunner(executor.clone())),
            Box::new(ArcRunner(reviewer.clone())),
            happy_path_config(),
        );

        let prompt = Prompt {
            user: "x".into(),
            ..Default::default()
        };
        let err = tr.run_tiered(&prompt).expect_err("must propagate");
        assert!(matches!(err, RunnerError::NotFound));
        assert_eq!(reviewer.calls().len(), 0);
    }

    /// AC-14: tokens_used ≈ chars/4 of every system+user+stdout the
    /// runner traversed, within ±25 % of the analytical sum.
    #[test]
    fn tokens_used_approximates_chars_div_4() {
        use std::sync::Arc;
        let planner = Arc::new(MockRunner::new());
        planner.push_ok(well_formed_plan());
        let executor = Arc::new(MockRunner::new());
        executor.push_ok("aaaa".repeat(10)); // 40 chars => ~10 tokens
        executor.push_ok("bbbb".repeat(10));
        let reviewer = Arc::new(MockRunner::new());
        reviewer.push_ok("FINAL".repeat(8)); // 40 chars => ~10 tokens

        struct ArcRunner(Arc<MockRunner>);
        impl Runner for ArcRunner {
            fn run(&self, p: &Prompt) -> RunnerResult<RunOutput> {
                self.0.run(p)
            }
        }
        let tr = TieredRunner::new(
            Box::new(ArcRunner(planner.clone())),
            Box::new(ArcRunner(executor.clone())),
            Box::new(ArcRunner(reviewer.clone())),
            happy_path_config(),
        );

        let prompt = Prompt {
            user: "tiny".into(),
            ..Default::default()
        };
        let out = tr.run_tiered(&prompt).unwrap();
        assert!(out.tokens_used > 0, "tokens_used must be non-zero");
        assert!(
            out.tokens_used <= happy_path_config().budget.max_tokens_per_run,
            "tokens_used must not exceed budget on success"
        );

        // Loose ±25 % sanity check: re-do the same pure math the
        // implementation does and compare.
        let p_calls = planner.calls();
        let e_calls = executor.calls();
        let r_calls = reviewer.calls();

        let mut expected: u64 = 0;
        // planner: system + user + stdout ("subtasks: ...")
        expected += approx_tokens(p_calls[0].system.as_deref().unwrap_or(""));
        expected += approx_tokens(&p_calls[0].user);
        expected += approx_tokens(well_formed_plan());
        for c in &e_calls {
            expected += approx_tokens(c.system.as_deref().unwrap_or(""));
            expected += approx_tokens(&c.user);
            expected += 10; // each executor stdout: 40 chars / 4
        }
        expected += approx_tokens(r_calls[0].system.as_deref().unwrap_or(""));
        expected += approx_tokens(&r_calls[0].user);
        expected += 10; // reviewer stdout: 40 / 4

        let lower = (expected as f64 * 0.75) as u64;
        let upper = (expected as f64 * 1.25) as u64 + 1;
        assert!(
            (lower..=upper).contains(&out.tokens_used),
            "tokens_used={} not within ±25% of expected={}",
            out.tokens_used,
            expected
        );
    }

    #[test]
    fn approx_tokens_matches_chars_div_4_ceil() {
        assert_eq!(approx_tokens(""), 0);
        assert_eq!(approx_tokens("a"), 1);
        assert_eq!(approx_tokens("ab"), 1);
        assert_eq!(approx_tokens("abcd"), 1);
        assert_eq!(approx_tokens("abcde"), 2);
        assert_eq!(approx_tokens(&"x".repeat(40)), 10);
    }

    #[test]
    fn parse_plan_yaml_strips_fence() {
        let raw = "```yaml\nsubtasks:\n  - id: a\n    description: do x\n```";
        let st = parse_plan_yaml(raw).expect("must parse");
        assert_eq!(st.len(), 1);
        assert_eq!(st[0].description, "do x");
    }

    #[test]
    fn parse_plan_yaml_returns_none_on_empty_subtasks() {
        // Valid YAML but no sub-tasks => fallback path.
        assert!(parse_plan_yaml("subtasks: []").is_none());
    }

    /// `BudgetExceeded` Display messages must surface both numbers and
    /// the actionable hint pointing the user at the manifest field. A
    /// regression here would silently swallow the "raise the cap or
    /// shorten" guidance.
    #[test]
    fn budget_exceeded_display_is_actionable() {
        let s = RunnerError::BudgetExceeded {
            actual: 1234,
            budget: 100,
        }
        .to_string();
        assert!(s.contains("1234"), "actual must surface: {s}");
        assert!(s.contains("100"), "budget must surface: {s}");
        assert!(
            s.contains("max_tokens_per_run") || s.contains("budget"),
            "must hint at manifest knob: {s}"
        );
    }

    /// Default budget is the documented 200K tokens.
    #[test]
    fn default_budget_is_200k() {
        assert_eq!(BudgetConfig::default().max_tokens_per_run, 200_000);
        assert_eq!(DEFAULT_MAX_TOKENS_PER_RUN, 200_000);
    }
}
