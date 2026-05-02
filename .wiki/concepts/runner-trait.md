---
slug: runner-trait
type: concept
last_updated_commit: 721050563f1ed29954b279fe334bf6bc8c8e2c34
confidence: 0.95
sources:
- crates/coral-runner/src/runner.rs
- crates/coral-runner/src/mock.rs
backlinks:
- runner
- mock-runner
- multi-agent-flow
status: verified
---

# Runner trait

The single abstraction that makes every LLM-touching code path in Coral testable.

```rust
pub trait Runner: Send + Sync {
    fn run(&self, prompt: &Prompt) -> RunnerResult<RunOutput>;
}
```

## Two implementations

- **`ClaudeRunner`** — production. Shells out to `claude --print` with the prompt's system / user / model. Manages timeout via `try_wait` poll. See [[runner]].
- **`MockRunner`** — tests. FIFO queue of scripted responses; captures every received prompt for assertions. See [[mock-runner]].

## The pattern in CLI commands

Every LLM-using subcommand exposes:

- `pub fn run(args, root) -> Result<ExitCode>` — constructs `ClaudeRunner::new()` and dispatches.
- `pub fn run_with_runner(args, root, &dyn Runner) -> Result<ExitCode>` — testable seam.

The binary calls `run`. Integration tests call `run_with_runner` with a `MockRunner`. Same code path, different runner.

This is what made the [[multi-agent-flow]] tractable: every Phase E2 test runs the real subcommand pipeline without needing `claude` in the sandbox.
