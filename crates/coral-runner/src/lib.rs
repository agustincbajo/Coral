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
pub use prompt::PromptBuilder;
pub use runner::{ClaudeRunner, Prompt, RunOutput, Runner, RunnerError, RunnerResult};
