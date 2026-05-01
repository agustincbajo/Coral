//! Mock runner for use in tests in this crate and downstream crates.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use crate::runner::{Prompt, RunOutput, Runner, RunnerError, RunnerResult};

/// A mock runner that returns scripted responses in FIFO order.
/// Used in tests to avoid invoking real `claude`.
#[derive(Debug, Default)]
pub struct MockRunner {
    responses: Mutex<VecDeque<RunnerResult<RunOutput>>>,
    /// Captures the prompts the runner has been called with, in order.
    calls: Mutex<Vec<Prompt>>,
    /// Optional per-response chunk lists for streaming. Same FIFO position
    /// as `responses`. `None` => default behaviour (single chunk in
    /// `run_streaming`).
    streaming_chunks: Mutex<VecDeque<Option<Vec<String>>>>,
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
}

impl Runner for MockRunner {
    fn run(&self, prompt: &Prompt) -> RunnerResult<RunOutput> {
        self.calls.lock().unwrap().push(prompt.clone());
        // Pop the streaming side too so the FIFOs stay aligned.
        let _ = self.streaming_chunks.lock().unwrap().pop_front();
        match self.responses.lock().unwrap().pop_front() {
            Some(r) => r,
            None => Ok(RunOutput {
                stdout: String::new(),
                stderr: String::new(),
                duration: Duration::from_millis(0),
            }),
        }
    }

    fn run_streaming(
        &self,
        prompt: &Prompt,
        on_chunk: &mut dyn FnMut(&str),
    ) -> RunnerResult<RunOutput> {
        self.calls.lock().unwrap().push(prompt.clone());
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
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
