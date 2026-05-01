//! Runner abstraction over the `claude` CLI binary.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("claude binary not found in PATH; install Claude Code from https://claude.com/code")]
    NotFound,
    #[error(
        "claude is not authenticated. Run `claude setup-token` or export ANTHROPIC_API_KEY in this shell.\n\nProvider response:\n{0}"
    )]
    AuthFailed(String),
    #[error("claude exited with code {code:?}: {stderr}")]
    NonZeroExit { code: Option<i32>, stderr: String },
    #[error("claude invocation timed out after {0:?}")]
    Timeout(Duration),
    #[error("io error invoking claude: {0}")]
    Io(#[from] std::io::Error),
}

/// Combine stdout and stderr into a single error-message string.
/// `claude --print` writes auth errors to stdout, so a non-zero exit with
/// empty stderr would otherwise lose the actionable detail.
pub(crate) fn combine_outputs(stdout: &str, stderr: &str) -> String {
    let stdout = stdout.trim();
    let stderr = stderr.trim();
    match (stderr.is_empty(), stdout.is_empty()) {
        (true, true) => String::new(),
        (true, false) => stdout.to_string(),
        (false, true) => stderr.to_string(),
        (false, false) => format!("{stderr}\n{stdout}"),
    }
}

/// Heuristic for spotting a provider auth failure in combined runner output.
pub(crate) fn is_auth_failure(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("authenticate")
        || lower.contains("401")
        || lower.contains("invalid_api_key")
        || lower.contains("invalid authentication")
}

pub type RunnerResult<T> = std::result::Result<T, RunnerError>;

/// A complete prompt ready for execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Prompt {
    /// Optional system prompt (typically the contents of a subagent .md).
    /// Passed via `--append-system-prompt`.
    pub system: Option<String>,
    /// The user prompt — what the LLM sees as the input.
    pub user: String,
    /// Optional model alias passed to `--model` (e.g., "sonnet", "haiku",
    /// or a full id like "claude-sonnet-4-6"). When None, claude uses default.
    pub model: Option<String>,
    /// Optional working directory for the claude process. When None,
    /// inherits the current process's cwd.
    pub cwd: Option<PathBuf>,
    /// Optional max wall-clock seconds. When elapsed, the process is killed.
    /// When None, no timeout (waits forever).
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunOutput {
    pub stdout: String,
    pub stderr: String,
    pub duration: Duration,
}

pub trait Runner: Send + Sync {
    /// Execute a prompt and return the captured output.
    /// Implementations must NOT panic on bad input — return RunnerError instead.
    fn run(&self, prompt: &Prompt) -> RunnerResult<RunOutput>;

    /// Streaming variant: invokes `on_chunk` with each newly-emitted chunk
    /// from the underlying provider, then returns the full accumulated output.
    ///
    /// The default implementation falls back to `run()` and emits a single
    /// chunk with the full stdout — fine for mocks and tests.
    ///
    /// Note: timeouts are NOT enforced in this default streaming path; the
    /// real runner override may enforce them, but the v0.2 implementation
    /// reads stdout to completion without polling a deadline.
    fn run_streaming(
        &self,
        prompt: &Prompt,
        on_chunk: &mut dyn FnMut(&str),
    ) -> RunnerResult<RunOutput> {
        let out = self.run(prompt)?;
        on_chunk(&out.stdout);
        Ok(out)
    }
}

/// The real runner that invokes `claude --print` as a subprocess.
#[derive(Debug, Clone)]
pub struct ClaudeRunner {
    /// Override the binary name/path. Default: "claude" (resolved via PATH).
    binary: PathBuf,
}

impl Default for ClaudeRunner {
    fn default() -> Self {
        Self {
            binary: PathBuf::from("claude"),
        }
    }
}

impl ClaudeRunner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the binary path (useful for tests, integration, or pinned installs).
    pub fn with_binary(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }
}

impl Runner for ClaudeRunner {
    fn run(&self, prompt: &Prompt) -> RunnerResult<RunOutput> {
        let start = Instant::now();
        let mut cmd = Command::new(&self.binary);
        cmd.arg("--print");
        if let Some(system) = &prompt.system {
            cmd.arg("--append-system-prompt").arg(system);
        }
        if let Some(model) = &prompt.model {
            cmd.arg("--model").arg(model);
        }
        if let Some(cwd) = &prompt.cwd {
            cmd.current_dir(cwd);
        }
        cmd.arg(&prompt.user);
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());

        tracing::debug!(binary = %self.binary.display(), model = ?prompt.model, "spawning claude");

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(RunnerError::NotFound);
            }
            Err(e) => return Err(RunnerError::Io(e)),
        };

        let output = if let Some(timeout) = prompt.timeout {
            // Poll-based timeout: try_wait every 100ms until elapsed >= timeout.
            let deadline = Instant::now() + timeout;
            loop {
                match child.try_wait()? {
                    Some(_status) => break child.wait_with_output()?,
                    None => {
                        if Instant::now() >= deadline {
                            let _ = child.kill();
                            let _ = child.wait();
                            return Err(RunnerError::Timeout(timeout));
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            }
        } else {
            child.wait_with_output()?
        };

        let duration = start.elapsed();
        if !output.status.success() {
            let stdout_str = String::from_utf8_lossy(&output.stdout);
            let stderr_str = String::from_utf8_lossy(&output.stderr);
            let combined = combine_outputs(&stdout_str, &stderr_str);
            if is_auth_failure(&combined) {
                return Err(RunnerError::AuthFailed(combined));
            }
            return Err(RunnerError::NonZeroExit {
                code: output.status.code(),
                stderr: combined,
            });
        }

        Ok(RunOutput {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            duration,
        })
    }

    /// Streaming runner: spawns claude, reads stdout line-by-line, invokes
    /// `on_chunk` for each line (including its trailing `\n`), and accumulates
    /// the full stdout for the returned `RunOutput`.
    ///
    /// Honors `prompt.timeout`: a separate thread reads stdout into an
    /// `mpsc::channel`, and the main loop waits on `recv_timeout` against the
    /// remaining deadline. If the deadline elapses, the child is killed and
    /// `RunnerError::Timeout` is returned.
    fn run_streaming(
        &self,
        prompt: &Prompt,
        on_chunk: &mut dyn FnMut(&str),
    ) -> RunnerResult<RunOutput> {
        use std::io::{BufRead, BufReader, Read};
        use std::sync::mpsc;
        use std::thread;

        let start = Instant::now();
        let mut cmd = Command::new(&self.binary);
        cmd.arg("--print");
        if let Some(system) = &prompt.system {
            cmd.arg("--append-system-prompt").arg(system);
        }
        if let Some(model) = &prompt.model {
            cmd.arg("--model").arg(model);
        }
        if let Some(cwd) = &prompt.cwd {
            cmd.current_dir(cwd);
        }
        cmd.arg(&prompt.user);
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());

        tracing::debug!(binary = %self.binary.display(), model = ?prompt.model, "spawning claude (streaming)");

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(RunnerError::NotFound);
            }
            Err(e) => return Err(RunnerError::Io(e)),
        };

        let stdout_handle = child
            .stdout
            .take()
            .ok_or_else(|| RunnerError::Io(std::io::Error::other("failed to capture stdout")))?;
        let (tx, rx) = mpsc::channel::<String>();
        let reader_thread = thread::spawn(move || {
            let mut reader = BufReader::new(stdout_handle);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if tx.send(line.clone()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        let mut accumulated = String::new();
        loop {
            let remaining = match prompt.timeout {
                Some(t) => match t.checked_sub(start.elapsed()) {
                    Some(r) => r,
                    None => {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = reader_thread.join();
                        return Err(RunnerError::Timeout(t));
                    }
                },
                None => Duration::from_secs(86_400),
            };
            match rx.recv_timeout(remaining) {
                Ok(line) => {
                    on_chunk(&line);
                    accumulated.push_str(&line);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader_thread.join();
                    let t = prompt
                        .timeout
                        .expect("must be Some to hit RecvTimeoutError::Timeout");
                    return Err(RunnerError::Timeout(t));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        let status = child.wait()?;
        let _ = reader_thread.join();
        let mut stderr = String::new();
        if let Some(mut h) = child.stderr.take() {
            let _ = h.read_to_string(&mut stderr);
        }
        let duration = start.elapsed();

        if !status.success() {
            let combined = combine_outputs(&accumulated, &stderr);
            if is_auth_failure(&combined) {
                return Err(RunnerError::AuthFailed(combined));
            }
            return Err(RunnerError::NonZeroExit {
                code: status.code(),
                stderr: combined,
            });
        }

        Ok(RunOutput {
            stdout: accumulated,
            stderr,
            duration,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_runner_uses_echo_substitute() {
        let r = ClaudeRunner::with_binary("/bin/echo");
        let prompt = Prompt {
            user: "hello world".into(),
            ..Default::default()
        };
        let out = r.run(&prompt).unwrap();
        // /bin/echo receives args ["--print", "hello world"] and prints them
        // space-separated on its single output line.
        assert!(out.stdout.contains("hello world"));
        assert!(out.stdout.contains("--print"));
    }

    #[test]
    fn claude_runner_not_found_returns_not_found() {
        let r = ClaudeRunner::with_binary("/nonexistent/coral-test-binary-xyz123");
        let err = r
            .run(&Prompt {
                user: "x".into(),
                ..Default::default()
            })
            .unwrap_err();
        assert!(matches!(err, RunnerError::NotFound));
    }

    #[test]
    fn claude_runner_non_zero_returns_error() {
        // /usr/bin/false exists on both macOS and Linux; /bin/false is missing on macOS.
        let r = ClaudeRunner::with_binary("/usr/bin/false");
        let err = r
            .run(&Prompt {
                user: "x".into(),
                ..Default::default()
            })
            .unwrap_err();
        assert!(matches!(err, RunnerError::NonZeroExit { .. }));
    }

    /// Verifies that a streaming run honors `prompt.timeout` and kills the
    /// child if the deadline elapses. Uses `/usr/bin/yes` which writes "y\n"
    /// indefinitely to stdout and ignores its CLI args — perfect substitute
    /// for a long-running streaming process. Available on macOS + Linux.
    #[test]
    fn claude_runner_streaming_timeout_kills_child() {
        use std::time::Instant;
        let r = ClaudeRunner::with_binary("/usr/bin/yes");
        let mut chunks: Vec<String> = Vec::new();
        let prompt = Prompt {
            user: "ignored".into(),
            timeout: Some(Duration::from_millis(200)),
            ..Default::default()
        };
        let start = Instant::now();
        let err = r
            .run_streaming(&prompt, &mut |c| chunks.push(c.to_string()))
            .unwrap_err();
        let elapsed = start.elapsed();
        assert!(
            matches!(err, RunnerError::Timeout(_)),
            "expected Timeout, got {err:?}"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "should kill within 2s, took {elapsed:?}"
        );
    }

    #[test]
    fn combine_outputs_handles_all_combinations() {
        assert_eq!(combine_outputs("", ""), "");
        assert_eq!(combine_outputs("out", ""), "out");
        assert_eq!(combine_outputs("", "err"), "err");
        assert_eq!(combine_outputs("out", "err"), "err\nout");
        // Trims whitespace at the boundaries so the formatted output is tidy.
        assert_eq!(combine_outputs("  out  \n", "\n  err\n"), "err\nout");
    }

    #[test]
    fn is_auth_failure_recognizes_provider_signatures() {
        assert!(is_auth_failure(
            "Failed to authenticate. API Error: 401 Invalid authentication credentials"
        ));
        assert!(is_auth_failure("error 401"));
        assert!(is_auth_failure("invalid_api_key"));
        assert!(is_auth_failure("Could not authenticate the request"));
        assert!(!is_auth_failure("model overloaded"));
        assert!(!is_auth_failure("rate limit exceeded"));
    }

    #[test]
    #[ignore]
    fn claude_runner_smoke_real_claude() {
        let r = ClaudeRunner::new();
        let prompt = Prompt {
            user: "Decí solo OK.".into(),
            timeout: Some(Duration::from_secs(60)),
            ..Default::default()
        };
        let out = r.run(&prompt).expect("real claude smoke");
        assert!(
            out.stdout.contains("OK") || out.stdout.to_lowercase().contains("ok"),
            "stdout did not contain OK: {}",
            out.stdout
        );
    }
}
