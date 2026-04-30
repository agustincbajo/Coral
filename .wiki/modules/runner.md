---
slug: runner
type: module
last_updated_commit: 213ac997cf61ad89610b3cfbe40af05e6b7fa8a8
confidence: 0.9
sources:
  - crates/coral-runner/src/
backlinks:
  - cli
  - lint
  - mock-runner
  - runner-trait
status: verified
---

# `coral-runner` — LLM subprocess wrapper

Lives at `crates/coral-runner`. 13 unit tests + 1 ignored (real `claude` smoke).

## Surface

- `Runner` trait with `fn run(&self, prompt: &Prompt) -> RunnerResult<RunOutput>`.
- `Prompt` struct: `system`, `user`, `model`, `cwd`, `timeout`.
- `ClaudeRunner` — production impl. Shells out to `claude --print --append-system-prompt <S> --model <M> <user>`.
- `MockRunner` — tests impl with FIFO scripted responses + call capture. See [[mock-runner]].
- `PromptBuilder` — `{{var}}` regex substitution for prompt templates.

## Why a trait

Because every LLM-using subcommand in [[cli]] needs to be testable with scripted responses. Without the trait, integration tests would need a real `claude` CLI in the test environment — which doesn't exist in CI.

The split is documented in [[runner-trait]].

## Error semantics

- `NotFound` — `claude` binary missing in `PATH`.
- `NonZeroExit` — exit code + stderr capture.
- `Timeout` — wall-clock cutoff via `try_wait` poll.
- `Io` — anything else.
