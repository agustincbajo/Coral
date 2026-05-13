//! Coral runner: wraps `claude` CLI invocations.
//!
//! Provides a [`Runner`] trait abstraction so that tests can swap a [`MockRunner`]
//! for the real [`ClaudeRunner`]. The runner handles invocation of the `claude`
//! binary in headless `--print` mode with versioned prompts and subagent system
//! prompts.

pub mod body_tempfile;
pub mod embeddings;
pub mod gemini;
pub mod http;
pub mod local;
pub mod mock;
pub mod multi_step;
pub mod prompt;
pub mod runner;

pub use embeddings::{
    AnthropicProvider, DEFAULT_OPENAI_DIM, DEFAULT_OPENAI_MODEL, DEFAULT_VOYAGE_DIM,
    DEFAULT_VOYAGE_MODEL, EmbedResult, EmbeddingsError, EmbeddingsProvider, MockEmbeddingsProvider,
    OpenAIProvider, PLACEHOLDER_ANTHROPIC_DIM, PLACEHOLDER_ANTHROPIC_MODEL, VoyageProvider,
};
pub use gemini::GeminiRunner;
pub use http::HttpRunner;
pub use local::LocalRunner;
pub use mock::MockRunner;
pub use multi_step::{
    BudgetConfig, DEFAULT_MAX_TOKENS_PER_RUN, MultiStepRunner, TierSpec, TieredConfig,
    TieredOutput, TieredRunner, approx_tokens,
};
pub use prompt::PromptBuilder;
pub use runner::{ClaudeRunner, Prompt, RunOutput, Runner, RunnerError, RunnerResult, TokenUsage};

/// **TEST-ONLY** — do not call from production code.
///
/// Serialiser for code that writes + fork-execs a small shell script.
/// The Linux kernel `do_open_execat` ETXTBSY race (errno 26) fires
/// when two parallel tests are in the write-then-exec window even
/// when they target distinct tempfiles. Cargo runs the lib's
/// `#[test]`s in one binary and each integration test in its own —
/// having a `pub` lock here (rather than module-scoped) means all
/// callers across both binaries can share the same Mutex *within a
/// single binary*. The integration-test binary still gets its own
/// static (Rust runs the lib's `lib.rs` for each binary separately),
/// but every test inside that binary now coordinates through it.
///
/// Tests acquire via `let _lock = test_script_lock();` and hold
/// through the spawn. Marked `pub` (not `pub(crate)`) ONLY because
/// integration tests under `tests/` (separate `#[test]` binaries from
/// the lib) need to import it as `coral_runner::test_script_lock()`.
/// Rust's `#[cfg(test)]` doesn't cross the lib → integration-test
/// crate boundary, so we can't gate the function with it.
///
/// **v0.35 ARCH-C2 decision**: kept `pub` + `#[doc(hidden)]` instead
/// of moving to a `coral-test-utils` dev-dep crate. Option (c) from
/// the Phase C spec — option (b) was rejected as net-negative churn
/// for a single 6-line helper. The `#[doc(hidden)]` attribute hides
/// it from `cargo doc` output so it doesn't pollute the API
/// reference; the heavy comment block here is the documented
/// contract. See `BACKLOG.md` for the revisit trigger (3+ helpers
/// or runtime-relevant usage). Also flagged by ARCH-H4 in the v0.35
/// architecture audit.
///
/// Effectively a no-op at runtime in release builds — `OnceLock`
/// init is one-shot and `Mutex::lock` on uncontended access is
/// a single atomic CAS.
#[doc(hidden)]
pub fn test_script_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
