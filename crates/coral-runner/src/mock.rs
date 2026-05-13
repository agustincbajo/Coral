//! Mock runner for use in tests in this crate and downstream crates.
//!
//! v0.36 clippy: this module is a test fixture. The `.lock().unwrap()` /
//! `.unwrap()` patterns below all sit on `std::sync::Mutex` guards
//! exercised only by single-threaded test bodies — a poisoned mutex
//! here always means a test panicked while holding the lock, which we
//! WANT to surface, not paper over. Module-wide allow keeps the
//! production ratchet (workspace-level `unwrap_used = "warn"`) from
//! drowning real production warnings in test-fixture noise.
#![allow(clippy::unwrap_used)]

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use crate::runner::{Prompt, RunOutput, Runner, RunnerError, RunnerResult, TokenUsage};

/// Closure type for `with_timeout_handler`. Receives the
/// `prompt.timeout` of every call so tests can assert their plumbing
/// passed it correctly. Returns `Some(result)` to short-circuit the
/// scripted response queue (e.g. for synthetic `RunnerError::Timeout`
/// returns); `None` to fall through to the FIFO.
///
/// v0.20.2 audit-followup #40.
pub type TimeoutHandler =
    Box<dyn FnMut(Option<Duration>) -> Option<RunnerResult<RunOutput>> + Send + Sync>;

/// A mock runner that returns scripted responses in FIFO order.
/// Used in tests to avoid invoking real `claude`.
#[derive(Default)]
pub struct MockRunner {
    responses: Mutex<VecDeque<RunnerResult<RunOutput>>>,
    /// Captures the prompts the runner has been called with, in order.
    calls: Mutex<Vec<Prompt>>,
    /// Optional per-response chunk lists for streaming. Same FIFO position
    /// as `responses`. `None` => default behaviour (single chunk in
    /// `run_streaming`).
    streaming_chunks: Mutex<VecDeque<Option<Vec<String>>>>,
    /// v0.20.2 audit-followup #40: optional handler invoked at the
    /// top of `run` / `run_streaming` with the `prompt.timeout` of
    /// the call. Lets tests assert that callers thread `timeout`
    /// through correctly without `std::thread::sleep`-ing for the
    /// real duration. Default behaviour (handler is `None`) is
    /// unchanged from pre-v0.20.2: returns the next scripted
    /// response immediately.
    timeout_handler: Mutex<Option<TimeoutHandler>>,
}

impl std::fmt::Debug for MockRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockRunner")
            .field("responses", &self.responses)
            .field("calls", &self.calls)
            .field("streaming_chunks", &self.streaming_chunks)
            .field(
                "timeout_handler",
                &self
                    .timeout_handler
                    .try_lock()
                    .ok()
                    .map(|g| if g.is_some() { "Some(_)" } else { "None" })
                    .unwrap_or("<locked>"),
            )
            .finish()
    }
}

impl MockRunner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pushes a successful response onto the queue.
    pub fn push_ok(&self, stdout: impl Into<String>) {
        self.responses.lock().unwrap().push_back(Ok(RunOutput {
            stdout: stdout.into(),
            stderr: String::new(),
            duration: Duration::from_millis(0),
            usage: None,
        }));
        self.streaming_chunks.lock().unwrap().push_back(None);
    }

    /// v0.34.0 (FR-ONB-29): push a scripted response that carries a
    /// real `TokenUsage`. Used by `coral bootstrap --max-cost` /
    /// `--resume` tests to drive mid-flight cost accumulation without
    /// hitting a real provider.
    pub fn push_ok_with_usage(&self, stdout: impl Into<String>, usage: TokenUsage) {
        self.responses.lock().unwrap().push_back(Ok(RunOutput {
            stdout: stdout.into(),
            stderr: String::new(),
            duration: Duration::from_millis(0),
            usage: Some(usage),
        }));
        self.streaming_chunks.lock().unwrap().push_back(None);
    }

    /// Pushes a successful response that, when read via `run_streaming`, will
    /// emit the provided chunks in order. The `run` (non-streaming) variant
    /// returns the chunks concatenated.
    pub fn push_ok_chunked(&self, chunks: Vec<&str>) {
        let stdout = chunks.concat();
        let owned: Vec<String> = chunks.iter().map(|s| (*s).to_string()).collect();
        self.responses.lock().unwrap().push_back(Ok(RunOutput {
            stdout,
            stderr: String::new(),
            duration: Duration::from_millis(0),
            usage: None,
        }));
        self.streaming_chunks.lock().unwrap().push_back(Some(owned));
    }

    /// Pushes an error response onto the queue.
    pub fn push_err(&self, err: RunnerError) {
        self.responses.lock().unwrap().push_back(Err(err));
        self.streaming_chunks.lock().unwrap().push_back(None);
    }

    /// Returns the prompts that were passed to `run`, in invocation order.
    pub fn calls(&self) -> Vec<Prompt> {
        self.calls.lock().unwrap().clone()
    }

    /// Returns the number of remaining queued responses.
    pub fn remaining(&self) -> usize {
        self.responses.lock().unwrap().len()
    }

    /// Install a closure invoked at the top of every `run` /
    /// `run_streaming` call with the `prompt.timeout` of that call.
    /// The handler can:
    ///
    /// - Record the value (e.g. into a captured `Mutex<Option<Duration>>`)
    ///   to assert the caller threaded `timeout` through correctly.
    /// - Return `Some(Err(RunnerError::Timeout(t)))` to synthesize a
    ///   timeout outcome without actually sleeping.
    /// - Return `None` to fall through to the FIFO scripted response
    ///   queue.
    ///
    /// v0.20.2 audit-followup #40: pre-fix `MockRunner` ignored
    /// `prompt.timeout` entirely, so tests using it could never
    /// validate that `LocalRunner` / `HttpRunner` / `GeminiRunner`'s
    /// timeout-honoring behaviour was actually wired by callers.
    /// Real runners pass `prompt.timeout` to `--max-time` (curl) or
    /// `wait_timeout` (subprocess); the mock now lets tests pin that
    /// contract.
    pub fn with_timeout_handler<F>(self, handler: F) -> Self
    where
        F: FnMut(Option<Duration>) -> Option<RunnerResult<RunOutput>> + Send + Sync + 'static,
    {
        *self.timeout_handler.lock().unwrap() = Some(Box::new(handler));
        self
    }

    /// Variant for `&self` so a test that has already moved the
    /// runner into an `Arc` can install the handler later.
    pub fn set_timeout_handler<F>(&self, handler: F)
    where
        F: FnMut(Option<Duration>) -> Option<RunnerResult<RunOutput>> + Send + Sync + 'static,
    {
        *self.timeout_handler.lock().unwrap() = Some(Box::new(handler));
    }
}

impl Runner for MockRunner {
    fn run(&self, prompt: &Prompt) -> RunnerResult<RunOutput> {
        self.calls.lock().unwrap().push(prompt.clone());
        // v0.20.2 audit-followup #40: invoke the timeout handler
        // (if installed) BEFORE consuming the FIFO queue, so a
        // handler that returns `Some(Err(Timeout))` short-circuits
        // without burning a scripted response. Default behaviour
        // (handler `None`) is unchanged.
        if let Some(handler) = self.timeout_handler.lock().unwrap().as_mut()
            && let Some(short_circuit) = handler(prompt.timeout)
        {
            // Drain the streaming-chunks slot too so the FIFOs
            // stay aligned with `responses`.
            let _ = self.streaming_chunks.lock().unwrap().pop_front();
            return short_circuit;
        }
        // Pop the streaming side too so the FIFOs stay aligned.
        let _ = self.streaming_chunks.lock().unwrap().pop_front();
        match self.responses.lock().unwrap().pop_front() {
            Some(r) => r,
            None => Ok(RunOutput {
                stdout: String::new(),
                stderr: String::new(),
                duration: Duration::from_millis(0),
                usage: None,
            }),
        }
    }

    fn run_streaming(
        &self,
        prompt: &Prompt,
        on_chunk: &mut dyn FnMut(&str),
    ) -> RunnerResult<RunOutput> {
        self.calls.lock().unwrap().push(prompt.clone());
        // v0.20.2 audit-followup #40: same handler hook on the
        // streaming path so tests don't have to special-case the
        // two surfaces.
        if let Some(handler) = self.timeout_handler.lock().unwrap().as_mut()
            && let Some(short_circuit) = handler(prompt.timeout)
        {
            let _ = self.streaming_chunks.lock().unwrap().pop_front();
            return short_circuit;
        }
        let chunks = self.streaming_chunks.lock().unwrap().pop_front().flatten();
        let response = self.responses.lock().unwrap().pop_front();
        match response {
            Some(Ok(out)) => {
                if let Some(chunks) = chunks {
                    for c in &chunks {
                        on_chunk(c);
                    }
                } else {
                    on_chunk(&out.stdout);
                }
                Ok(out)
            }
            Some(Err(e)) => Err(e),
            None => Ok(RunOutput {
                stdout: String::new(),
                stderr: String::new(),
                duration: Duration::from_millis(0),
                usage: None,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// v0.34.0 (FR-ONB-29): `push_ok_with_usage` carries a real
    /// `TokenUsage` through to the `RunOutput.usage` field. Used by
    /// `coral bootstrap --max-cost` tests to drive mid-flight cost
    /// accumulation without a real provider.
    #[test]
    fn mock_propagates_token_usage_when_pushed_with_usage() {
        let m = MockRunner::new();
        let usage = TokenUsage {
            input_tokens: 42,
            output_tokens: 7,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        };
        m.push_ok_with_usage("hello", usage);
        let p = Prompt {
            user: "x".into(),
            ..Default::default()
        };
        let out = m.run(&p).unwrap();
        assert_eq!(out.stdout, "hello");
        let got = out.usage.expect("usage propagated");
        assert_eq!(got.input_tokens, 42);
        assert_eq!(got.output_tokens, 7);
    }

    /// v0.34.0 (FR-ONB-29): the default `push_ok` path leaves `usage`
    /// at `None`, preserving prior mock-based test behaviour.
    #[test]
    fn mock_push_ok_leaves_usage_none() {
        let m = MockRunner::new();
        m.push_ok("hi");
        let p = Prompt {
            user: "x".into(),
            ..Default::default()
        };
        let out = m.run(&p).unwrap();
        assert!(out.usage.is_none());
    }

    #[test]
    fn mock_returns_pushed_response_fifo() {
        let m = MockRunner::new();
        m.push_ok("first");
        m.push_ok("second");

        let p = Prompt {
            user: "a".into(),
            ..Default::default()
        };
        let out1 = m.run(&p).unwrap();
        let out2 = m.run(&p).unwrap();

        assert_eq!(out1.stdout, "first");
        assert_eq!(out2.stdout, "second");
    }

    #[test]
    fn mock_captures_prompts() {
        let m = MockRunner::new();
        m.push_ok("");
        m.push_ok("");

        let p1 = Prompt {
            user: "first prompt".into(),
            ..Default::default()
        };
        let p2 = Prompt {
            user: "second prompt".into(),
            model: Some("haiku".into()),
            ..Default::default()
        };

        let _ = m.run(&p1).unwrap();
        let _ = m.run(&p2).unwrap();

        let calls = m.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].user, "first prompt");
        assert_eq!(calls[1].user, "second prompt");
        assert_eq!(calls[1].model.as_deref(), Some("haiku"));
    }

    #[test]
    fn mock_returns_default_when_empty() {
        let m = MockRunner::new();
        let p = Prompt {
            user: "x".into(),
            ..Default::default()
        };
        let out = m.run(&p).unwrap();
        assert!(out.stdout.is_empty());
        assert!(out.stderr.is_empty());
    }

    #[test]
    fn mock_push_err_propagates() {
        let m = MockRunner::new();
        m.push_err(RunnerError::NotFound);

        let p = Prompt {
            user: "x".into(),
            ..Default::default()
        };
        let err = m.run(&p).unwrap_err();
        assert!(matches!(err, RunnerError::NotFound));
    }

    #[test]
    fn mock_remaining_reflects_queue() {
        let m = MockRunner::new();
        m.push_ok("a");
        m.push_ok("b");
        m.push_ok("c");
        assert_eq!(m.remaining(), 3);

        let p = Prompt {
            user: "x".into(),
            ..Default::default()
        };
        let _ = m.run(&p).unwrap();
        assert_eq!(m.remaining(), 2);
    }

    #[test]
    fn mock_run_streaming_emits_chunks_when_pushed_chunked() {
        let m = MockRunner::new();
        m.push_ok_chunked(vec!["hello ", "world"]);

        let p = Prompt {
            user: "x".into(),
            ..Default::default()
        };
        let mut received: Vec<String> = Vec::new();
        let out = m
            .run_streaming(&p, &mut |c| received.push(c.to_string()))
            .unwrap();
        assert_eq!(received, vec!["hello ".to_string(), "world".to_string()]);
        assert_eq!(out.stdout, "hello world");
    }

    #[test]
    fn mock_run_streaming_emits_single_chunk_for_push_ok() {
        let m = MockRunner::new();
        m.push_ok("xyz");

        let p = Prompt {
            user: "x".into(),
            ..Default::default()
        };
        let mut received: Vec<String> = Vec::new();
        let out = m
            .run_streaming(&p, &mut |c| received.push(c.to_string()))
            .unwrap();
        assert_eq!(received, vec!["xyz".to_string()]);
        assert_eq!(out.stdout, "xyz");
    }

    /// v0.20.2 audit-followup #40: a `with_timeout_handler` records
    /// the `prompt.timeout` of every call so tests can assert
    /// callers thread the timeout through correctly. Returning
    /// `None` from the handler falls through to the FIFO scripted
    /// response (the default behaviour), so a recorder that doesn't
    /// short-circuit is observably equivalent to the pre-fix mock.
    #[test]
    fn mock_runner_records_prompt_timeout_via_handler() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let recorded: Arc<Mutex<Vec<Option<Duration>>>> = Arc::new(Mutex::new(Vec::new()));
        let counter = Arc::new(AtomicUsize::new(0));
        let recorded_clone = Arc::clone(&recorded);
        let counter_clone = Arc::clone(&counter);

        let m = MockRunner::new().with_timeout_handler(move |t| {
            recorded_clone.lock().unwrap().push(t);
            counter_clone.fetch_add(1, Ordering::SeqCst);
            None // fall through to FIFO
        });
        m.push_ok("scripted-1");
        m.push_ok("scripted-2");

        // Two calls with distinct timeouts.
        let p1 = Prompt {
            user: "a".into(),
            timeout: Some(Duration::from_secs(7)),
            ..Default::default()
        };
        let p2 = Prompt {
            user: "b".into(),
            timeout: None,
            ..Default::default()
        };
        let r1 = m.run(&p1).unwrap();
        let r2 = m.run(&p2).unwrap();

        assert_eq!(r1.stdout, "scripted-1");
        assert_eq!(r2.stdout, "scripted-2");
        let captured = recorded.lock().unwrap().clone();
        assert_eq!(
            captured,
            vec![Some(Duration::from_secs(7)), None],
            "recorded timeouts must mirror the prompts"
        );
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    /// v0.20.2 audit-followup #40: handler returning
    /// `Some(Err(Timeout))` short-circuits the FIFO queue. The
    /// scripted response stays in place for the next call. This
    /// matches the contract real runners follow: a timed-out call
    /// doesn't consume backend state.
    #[test]
    fn mock_runner_timeout_handler_short_circuits_without_consuming_fifo() {
        let m = MockRunner::new().with_timeout_handler(|t| t.map(|d| Err(RunnerError::Timeout(d))));
        m.push_ok("would-be-scripted");
        let p = Prompt {
            user: "x".into(),
            timeout: Some(Duration::from_millis(250)),
            ..Default::default()
        };
        let err = m.run(&p).unwrap_err();
        match err {
            RunnerError::Timeout(d) => assert_eq!(d, Duration::from_millis(250)),
            other => panic!("expected Timeout, got {other:?}"),
        }
        // FIFO still has the scripted response since the handler
        // short-circuited — it should NOT have been popped.
        // (The streaming-chunks slot is popped to keep alignment;
        // that's fine — the next push_ok will land both queues in
        // sync again. We verify this by another `push_ok` then
        // `run` returning the second message.)
        m.push_ok("post-timeout");
        // The first scripted ("would-be-scripted") was not consumed
        // by the timeout call (timeout drained the streaming-chunks
        // slot, but `responses` was untouched). So the next no-
        // timeout call returns the original scripted response.
        let p_no_timeout = Prompt {
            user: "y".into(),
            timeout: None,
            ..Default::default()
        };
        let out = m.run(&p_no_timeout).unwrap();
        assert_eq!(out.stdout, "would-be-scripted");
    }

    /// v0.20.2 audit-followup #40: default (no handler) behaviour is
    /// unchanged — `prompt.timeout` is recorded in `calls()` but
    /// otherwise ignored, and the FIFO scripted response queue
    /// drives the return value.
    #[test]
    fn mock_runner_without_handler_ignores_timeout_field() {
        let m = MockRunner::new();
        m.push_ok("ok");
        let p = Prompt {
            user: "x".into(),
            timeout: Some(Duration::from_secs(99)),
            ..Default::default()
        };
        let out = m.run(&p).unwrap();
        assert_eq!(out.stdout, "ok");
        // Timeout still recorded in `calls()` for assertion.
        let calls = m.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].timeout, Some(Duration::from_secs(99)));
    }
}
