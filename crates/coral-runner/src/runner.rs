//! Runner abstraction over the `claude` CLI binary.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error(
        "runner binary not found in PATH. \
         For Claude: install Claude Code from https://claude.com/code. \
         For Gemini: install gemini-cli. For Local: install llama-cli (llama.cpp). \
         For HTTP: --provider http reads CORAL_HTTP_ENDPOINT instead of a binary."
    )]
    NotFound,
    #[error(
        "runner not authenticated. \
         For Claude: run `claude setup-token` or export ANTHROPIC_API_KEY. \
         For Gemini: run `gemini auth login` or export GEMINI_API_KEY. \
         For HTTP: set CORAL_HTTP_API_KEY (if your endpoint requires auth).\n\nProvider response:\n{0}"
    )]
    AuthFailed(String),
    #[error("runner exited with code {code:?}: {stderr}")]
    NonZeroExit { code: Option<i32>, stderr: String },
    #[error("runner invocation timed out after {0:?}")]
    Timeout(Duration),
    #[error("io error invoking runner: {0}")]
    Io(#[from] std::io::Error),
    /// v0.21.4: surfaced by `MultiStepRunner` implementations when the
    /// cumulative token estimate of a tiered run would exceed the
    /// configured budget. `actual` is the token count we'd reach if the
    /// next sub-call ran; `budget` is `BudgetConfig::max_tokens_per_run`.
    /// Tip in the message points the user at how to bump the cap.
    #[error(
        "tiered run aborted: cumulative token estimate {actual} would exceed budget {budget}. \
         Raise `runner.tiered.budget.max_tokens_per_run` in coral.toml or shorten the prompt."
    )]
    BudgetExceeded { actual: u64, budget: u64 },
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

/// Scrub strings that look like API keys / bearer tokens from a
/// runner's stdout/stderr before it lands in an error message.
///
/// v0.19.5 audit H8: providers occasionally echo the request headers
/// they received in their error body (e.g. `"received Authorization:
/// Bearer sk-…"`). Surfacing that verbatim in `RunnerError::AuthFailed`
/// would leak the key into logs / error traces. Filter out a small
/// allowlist of header-shaped substrings.
pub(crate) fn scrub_secrets(text: &str) -> String {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        // (?i) for case-insensitive matching. Three forms we cover:
        //   Authorization: Bearer <tok>
        //   x-api-key: <tok>
        //   Bearer <tok>     (bare; no Authorization: prefix)
        // The trailing `(?:\s+\S+)?` consumes the secret token after
        // the keyword. Swallowing too much is preferable to leaking;
        // surrounding "innocuous text" can be reconstructed from the
        // pre-redaction bug report.
        regex::Regex::new(r"(?i)(?:authorization|x-api-key)\s*:\s*\S+(?:\s+\S+)?|bearer\s+\S+")
            .expect("valid regex")
    });
    re.replace_all(text, "<redacted>").to_string()
}

/// Shared streaming runner used by Claude / Gemini / Local. Spawns the
/// already-configured `Command`, reads stdout line-by-line in a worker
/// thread, invokes `on_chunk` for each line, accumulates the full stdout
/// for `RunOutput.stdout`, and honors `timeout` via `recv_timeout` so the
/// child is killed when the deadline elapses.
///
/// Auth/non-zero-exit handling matches the non-streaming `run` path: 401-
/// shaped failures surface as `RunnerError::AuthFailed` via the shared
/// `combine_outputs` + `is_auth_failure` helpers.
pub(crate) fn run_streaming_command(
    mut cmd: Command,
    timeout: Option<Duration>,
    on_chunk: &mut dyn FnMut(&str),
) -> RunnerResult<RunOutput> {
    use std::io::{BufRead, BufReader, Read};
    use std::sync::mpsc;
    use std::thread;

    let start = Instant::now();
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

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
        let remaining = match timeout {
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
                let t = timeout.expect("must be Some to hit RecvTimeoutError::Timeout");
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
        // v0.19.5 audit H8: scrub bearer/x-api-key substrings before
        // surfacing the runner's stdout/stderr in our error envelope.
        let scrubbed = scrub_secrets(&combined);
        if is_auth_failure(&scrubbed) {
            return Err(RunnerError::AuthFailed(scrubbed));
        }
        return Err(RunnerError::NonZeroExit {
            code: status.code(),
            stderr: scrubbed,
        });
    }

    Ok(RunOutput {
        stdout: accumulated,
        stderr,
        duration,
    })
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
        // v0.30.0 audit cycle 5 B9: `prompt.user` is user-controlled and
        // is passed as a bare positional to `claude`. If it starts with
        // `--` (e.g. a user pastes `--system rogue-prompt` into a chat
        // box that ends up in `prompt.user`), the child CLI would parse
        // it as a flag instead of a prompt. Inserting `--` here forces
        // the rest of the args to be treated as positionals — same
        // CVE-2017-1000117 / CVE-2024-32004 family pattern that git
        // adopted years ago. `GeminiRunner` / `LocalRunner` are immune
        // because they use `-p <value>` (clap consumes the next arg).
        cmd.arg("--");
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
            // v0.19.5 audit H8: scrub bearer/x-api-key substrings.
            let scrubbed = scrub_secrets(&combined);
            if is_auth_failure(&scrubbed) {
                return Err(RunnerError::AuthFailed(scrubbed));
            }
            return Err(RunnerError::NonZeroExit {
                code: output.status.code(),
                stderr: scrubbed,
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
    /// the full stdout for the returned `RunOutput`. Delegates the I/O loop to
    /// the shared `run_streaming_command` helper so all runners share timeout
    /// + auth-detection semantics.
    fn run_streaming(
        &self,
        prompt: &Prompt,
        on_chunk: &mut dyn FnMut(&str),
    ) -> RunnerResult<RunOutput> {
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
        // v0.30.0 audit cycle 5 B9: see the matching note above on the
        // non-streaming `run` path. Same `--` separator for the same
        // reason — `prompt.user` is user-controlled and must not be
        // parsed as a flag by `claude`.
        cmd.arg("--");
        cmd.arg(&prompt.user);
        tracing::debug!(binary = %self.binary.display(), model = ?prompt.model, "spawning claude (streaming)");
        run_streaming_command(cmd, prompt.timeout, on_chunk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// v0.19.5 audit H8: scrub_secrets removes bearer / x-api-key /
    /// Authorization substrings from runner output before it lands in
    /// an error message.
    #[test]
    fn scrub_secrets_redacts_bearer_token() {
        let raw = "echoed your headers: Authorization: Bearer sk-test-abc and another";
        let out = scrub_secrets(raw);
        assert!(!out.contains("sk-test-abc"), "leaked token: {out}");
        assert!(out.contains("<redacted>"), "no redaction marker: {out}");
    }

    #[test]
    fn scrub_secrets_redacts_x_api_key() {
        let raw = "got: x-api-key: super-secret-1234 (rejected)";
        let out = scrub_secrets(raw);
        assert!(!out.contains("super-secret-1234"), "leaked key: {out}");
    }

    #[test]
    fn scrub_secrets_handles_no_match_idempotently() {
        let raw = "innocuous text without anything sensitive";
        assert_eq!(scrub_secrets(raw), raw);
    }

    /// Returns a `(TempDir, PathBuf)` holding an executable shell script
    /// that ignores every CLI arg and writes `y\n` forever. Replaces
    /// `/usr/bin/yes` for timeout tests because GNU coreutils 9.4+
    /// rejects unknown long options (the runner adds `--print` which we
    /// can't suppress).
    ///
    /// Uses `fs::write` (which closes the fd on completion) and then
    /// `set_permissions`. Avoids `tempfile::NamedTempFile` because that
    /// keeps the file open and Linux refuses to execute a file with an
    /// open writable fd (`ETXTBSY` "Text file busy").
    ///
    /// Caller must keep the returned `TempDir` alive for the duration of
    /// the test (Drop deletes the directory tree).
    #[cfg(unix)]
    fn forever_yes_script() -> (tempfile::TempDir, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::Builder::new()
            .prefix("coral-yes-")
            .tempdir()
            .expect("tempdir");
        let path = dir.path().join("yes.sh");
        std::fs::write(&path, "#!/bin/sh\nwhile :; do echo y; done\n").expect("write script");
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod 755");
        (dir, path)
    }

    #[test]
    fn claude_runner_uses_echo_substitute() {
        let r = ClaudeRunner::with_binary("/bin/echo");
        let prompt = Prompt {
            user: "hello world".into(),
            ..Default::default()
        };
        let out = r.run(&prompt).unwrap();
        // /bin/echo receives args ["--print", "--", "hello world"] (the
        // bare `--` was added in v0.30.0 audit cycle 5 B9 to prevent
        // user-controlled `prompt.user` from being parsed as a flag).
        // `/bin/echo` is not getopt-aware so it just prints them
        // space-separated.
        assert!(out.stdout.contains("hello world"));
        assert!(out.stdout.contains("--print"));
    }

    /// v0.30.0 audit cycle 5 B9: a `prompt.user` value that starts with
    /// `--` must reach the child as a positional, not be parsed as a
    /// flag. We can't easily intercept the spawned subprocess without
    /// `Command::get_args` (stable since 1.57), so we use `/bin/echo`
    /// as a stand-in: it prints every arg it received, so we can grep
    /// the output for the user prompt to confirm it survived the
    /// child's argv parsing intact.
    ///
    /// The exact argv this test pins is:
    ///   `<echo> --print -- --system rogue-prompt`
    /// Pre-fix the args were:
    ///   `<echo> --print --system rogue-prompt`
    /// which `/bin/echo` happily prints (it's not getopt-aware), but a
    /// real `claude` CLI would parse `--system rogue-prompt` as a flag.
    /// The `--` is what protects us.
    #[cfg(unix)]
    #[test]
    fn claude_runner_inserts_double_dash_before_user_prompt() {
        let r = ClaudeRunner::with_binary("/bin/echo");
        let prompt = Prompt {
            user: "--system rogue-prompt".into(),
            ..Default::default()
        };
        let out = r.run(&prompt).unwrap();
        // /bin/echo echoes its argv space-separated. We expect to see
        // the `--` separator followed by the user-controlled string.
        assert!(
            out.stdout.contains("-- --system rogue-prompt"),
            "expected the `--` separator immediately before the user \
             prompt, got stdout: {:?}",
            out.stdout
        );
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

    /// Non-streaming `run` honors `prompt.timeout` and returns
    /// `RunnerError::Timeout` when the deadline elapses. Uses a tempdir
    /// shell script that ignores all CLI args and writes "y\n" forever
    /// — equivalent to `yes` but tolerant of `--print` and other flags.
    /// Plain `/usr/bin/yes` rejects unknown long options on GNU coreutils
    /// 9.4+ (Ubuntu 24.04).
    #[cfg(unix)]
    #[test]
    fn claude_runner_run_honors_timeout() {
        use std::time::Instant;
        let (_dir, script) = forever_yes_script();
        let r = ClaudeRunner::with_binary(&script);
        let prompt = Prompt {
            user: "ignored".into(),
            timeout: Some(Duration::from_millis(200)),
            ..Default::default()
        };
        let start = Instant::now();
        let err = r.run(&prompt).unwrap_err();
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

    /// Display message of every `RunnerError` variant — pins the
    /// user-facing error format. A change here will surface in `coral` as
    /// `error: {e}` printed to stderr; we want to know if the wording
    /// drifts.
    #[test]
    fn runner_error_display_messages_are_actionable() {
        let not_found = RunnerError::NotFound.to_string();
        assert!(not_found.contains("not found"), "got: {not_found}");
        assert!(
            not_found.contains("claude.com/code"),
            "should link install URL: {not_found}"
        );

        let auth = RunnerError::AuthFailed("HTTP 401 fake response".into()).to_string();
        assert!(auth.contains("not authenticated"), "got: {auth}");
        assert!(
            auth.contains("setup-token") || auth.contains("ANTHROPIC_API_KEY"),
            "should hint fix: {auth}"
        );
        assert!(
            auth.contains("HTTP 401"),
            "should include provider response: {auth}"
        );

        let nonzero = RunnerError::NonZeroExit {
            code: Some(2),
            stderr: "syntax error".into(),
        }
        .to_string();
        assert!(nonzero.contains("Some(2)"), "got: {nonzero}");
        assert!(nonzero.contains("syntax error"), "got: {nonzero}");

        let timeout = RunnerError::Timeout(Duration::from_secs(5)).to_string();
        assert!(timeout.contains("timed out"), "got: {timeout}");
        assert!(timeout.contains("5s"), "should include duration: {timeout}");

        let io = RunnerError::Io(std::io::Error::other("disk full")).to_string();
        assert!(io.contains("io error"), "got: {io}");
        assert!(io.contains("disk full"), "got: {io}");
    }

    /// Verifies that a streaming run honors `prompt.timeout` and kills the
    /// child if the deadline elapses. Uses a tempdir shell script that
    /// writes "y\n" forever — same fixture as `claude_runner_run_honors_timeout`.
    /// Plain `/usr/bin/yes` no longer works because GNU coreutils 9.4+ rejects
    /// `--print` as an unknown long option (we can't suppress the runner's
    /// `--print` flag).
    #[cfg(unix)]
    #[test]
    fn claude_runner_streaming_timeout_kills_child() {
        use std::time::Instant;
        let (_dir, script) = forever_yes_script();
        let r = ClaudeRunner::with_binary(&script);
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
