use coral_runner::{ClaudeRunner, Runner};

/// Constructs the default runner. Subcommands that need an LLM use this
/// when the test harness hasn't injected its own runner.
pub fn default_runner() -> Box<dyn Runner> {
    Box::new(ClaudeRunner::new())
}
