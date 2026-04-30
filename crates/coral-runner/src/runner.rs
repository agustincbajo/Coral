//! Runner abstraction over the `claude` CLI binary.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("claude binary not found in PATH; install Claude Code")]
    NotFound,
    #[error("claude exited with code {code:?}: {stderr}")]
    NonZeroExit { code: Option<i32>, stderr: String },
    #[error("claude invocation timed out after {0:?}")]
    Timeout(Duration),
    #[error("io error invoking claude: {0}")]
    Io(#[from] std::io::Error),
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
            return Err(RunnerError::NonZeroExit {
                code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }

        Ok(RunOutput {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
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
