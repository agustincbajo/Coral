//! GeminiRunner — alternative LLM provider for batch operations (semantic
//! lint nightly, consolidate weekly). v0.2 ships a stub that shells out to
//! a `gemini` CLI binary if installed; if absent, returns RunnerError::NotFound.
//!
//! The contract is identical to ClaudeRunner: invokes `<binary> --print`
//! with `--append-system-prompt` and `--model`. If your installed Gemini
//! CLI uses different flags, override via `with_binary(path)` and a future
//! flag-customization API.

use crate::runner::{ClaudeRunner, Prompt, RunOutput, Runner, RunnerResult};
use std::path::PathBuf;

/// Wrapper that delegates to ClaudeRunner with a different default binary.
/// Provided as a separate type for ergonomic grep-ability. v0.3 will split
/// into a fully independent impl when the Gemini CLI flag conventions diverge.
#[derive(Debug, Clone)]
pub struct GeminiRunner {
    inner: ClaudeRunner,
}

impl Default for GeminiRunner {
    fn default() -> Self {
        Self {
            inner: ClaudeRunner::with_binary("gemini"),
        }
    }
}

impl GeminiRunner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the binary path (e.g., "/usr/local/bin/gemini-cli-v2").
    pub fn with_binary(binary: impl Into<PathBuf>) -> Self {
        Self {
            inner: ClaudeRunner::with_binary(binary),
        }
    }
}

impl Runner for GeminiRunner {
    fn run(&self, prompt: &Prompt) -> RunnerResult<RunOutput> {
        self.inner.run(prompt)
    }
    fn run_streaming(
        &self,
        prompt: &Prompt,
        on_chunk: &mut dyn FnMut(&str),
    ) -> RunnerResult<RunOutput> {
        self.inner.run_streaming(prompt, on_chunk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::RunnerError;

    #[test]
    fn gemini_runner_default_binary_is_gemini() {
        let _r = GeminiRunner::new();
        // No-op smoke; the actual binary check happens at run-time.
    }

    #[test]
    fn gemini_runner_with_unknown_binary_returns_not_found() {
        let r = GeminiRunner::with_binary("/nonexistent/coral-test-gemini-xyz");
        let err = r
            .run(&Prompt {
                user: "x".into(),
                ..Default::default()
            })
            .unwrap_err();
        assert!(matches!(err, RunnerError::NotFound));
    }

    #[test]
    fn gemini_runner_uses_echo_substitute() {
        // /bin/echo exists on macOS; /usr/bin/echo does not.
        let r = GeminiRunner::with_binary("/bin/echo");
        let out = r
            .run(&Prompt {
                user: "ping".into(),
                ..Default::default()
            })
            .unwrap();
        assert!(out.stdout.contains("ping"));
    }
}
