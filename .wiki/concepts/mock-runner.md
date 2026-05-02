---
slug: mock-runner
type: concept
last_updated_commit: 721050563f1ed29954b279fe334bf6bc8c8e2c34
confidence: 0.9
sources:
- crates/coral-runner/src/mock.rs
backlinks:
- runner
- runner-trait
status: verified
---

# MockRunner

Test double for the [[runner-trait]]. Lives at `coral-runner/src/mock.rs`.

```rust
let r = MockRunner::new();
r.push_ok("scripted response");
r.push_err(RunnerError::NotFound);
// invoke runner.run(...) — pops responses FIFO
let calls = r.calls(); // captured Prompts in order
```

## Surface

- `push_ok(stdout)` — push successful response.
- `push_err(err)` — push specific error variant.
- `calls()` — Vec<Prompt> in invocation order.
- `remaining()` — queue depth.

When the queue is empty, `run()` returns an `Ok(RunOutput { stdout: "", … })` instead of panicking. This makes the tests fail-soft when the assertion is "did the runner get called" rather than "did it return X".

## Why mutex-wrapped?

`Mutex<VecDeque<…>>` and `Mutex<Vec<Prompt>>` so the trait `Runner: Send + Sync` is satisfied with `&self` methods. Tests own the runner, so contention is nil — the lock just lives there to let `&dyn Runner` cross thread boundaries cleanly.

## Used in

- All `commands/*::run_with_runner` integration tests in `coral-cli`.
- `lint::semantic::tests` to verify `severity:slug:message` parsing.
- E2E lifecycle tests in `tests/e2e_*.rs`.
