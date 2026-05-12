//! GeminiRunner — real (non-stub) wrapper around the `gemini` CLI.
//!
//! Unlike the v0.2 stub, this runner builds its own argv per Gemini CLI
//! conventions instead of reusing ClaudeRunner's flags. The default flag
//! convention follows the open-source `gemini-cli`:
//!
//! - `-p <user>` for the prompt (non-interactive mode)
//! - `-m <model>` for the model id (e.g. `gemini-2.5-flash`)
//! - System prompts are **prepended** to the user prompt (with a blank-line
//!   separator) rather than passed via a flag. This is dialect-agnostic and
//!   works against any CLI that accepts `-p`. If your install supports a
//!   `--system-instruction` flag, point `with_binary` at a thin wrapper
//!   script that translates.
//!
//! On 401/auth-style failures the same `combine_outputs` + `is_auth_failure`
//! logic that ClaudeRunner uses surfaces an actionable message via
//! `RunnerError::AuthFailed`.

use crate::runner::{
    Prompt, RunOutput, Runner, RunnerError, RunnerResult, combine_outputs, is_auth_failure,
    parse_usage_from_stdout, run_streaming_command, scrub_secrets,
};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct GeminiRunner {
    binary: PathBuf,
}

impl Default for GeminiRunner {
    fn default() -> Self {
        Self {
            binary: PathBuf::from("gemini"),
        }
    }
}

impl GeminiRunner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the binary path (e.g. `/usr/local/bin/gemini-cli-v2`, or a
    /// wrapper script that adapts your install's flag conventions).
    pub fn with_binary(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    /// Build the argv that gets passed to `gemini`. Pure function so tests
    /// can assert the CLI shape without spawning a process.
    pub(crate) fn build_args(prompt: &Prompt) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();
        if let Some(model) = &prompt.model {
            args.push("-m".into());
            args.push(model.clone());
        }
        let combined_prompt = match &prompt.system {
            Some(s) if !s.is_empty() => format!("{s}\n\n{}", prompt.user),
            _ => prompt.user.clone(),
        };
        args.push("-p".into());
        args.push(combined_prompt);
        args
    }

    fn build_command(&self, prompt: &Prompt) -> Command {
        let mut cmd = Command::new(&self.binary);
        cmd.args(Self::build_args(prompt));
        if let Some(cwd) = &prompt.cwd {
            cmd.current_dir(cwd);
        }
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());
        cmd
    }
}

impl Runner for GeminiRunner {
    fn run(&self, prompt: &Prompt) -> RunnerResult<RunOutput> {
        let start = Instant::now();
        let mut cmd = self.build_command(prompt);
        tracing::debug!(
            binary = %self.binary.display(),
            model = ?prompt.model,
            "spawning gemini"
        );

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(RunnerError::NotFound);
            }
            Err(e) => return Err(RunnerError::Io(e)),
        };

        let output = if let Some(timeout) = prompt.timeout {
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
            // v0.19.6 audit H2: scrub bearer/x-api-key substrings before
            // surfacing the runner's stdout/stderr in our error envelope.
            // Sister of v0.19.5 H8 (http.rs / runner.rs / embeddings.rs).
            let scrubbed = scrub_secrets(&combined);
            if is_auth_failure(&scrubbed) {
                return Err(RunnerError::AuthFailed(scrubbed));
            }
            return Err(RunnerError::NonZeroExit {
                code: output.status.code(),
                stderr: scrubbed,
            });
        }

        // v0.34.0 (FR-ONB-29): best-effort usage extraction. The
        // gemini-cli prose mode does NOT emit a structured `usage`
        // block; users who want real cost gating must wrap the binary
        // in a shim that converts the Gemini API's `usageMetadata`
        // into the shared `usage:{input_tokens,output_tokens}` JSON
        // envelope. When the stdout isn't JSON, `parse_usage_from_stdout`
        // returns `(None, None)` and the caller falls back to the
        // heuristic in `coral_core::cost`.
        let raw_stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let (usage, inner) = parse_usage_from_stdout(&raw_stdout);
        let stdout = inner.unwrap_or(raw_stdout);
        Ok(RunOutput {
            stdout,
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            duration,
            usage,
        })
    }

    /// Streaming via the shared `run_streaming_command` helper. Lets
    /// `coral query --provider gemini` see the response token-by-token
    /// when gemini-cli streams (recent versions do; older `-p` mode buffers).
    fn run_streaming(
        &self,
        prompt: &Prompt,
        on_chunk: &mut dyn FnMut(&str),
    ) -> RunnerResult<RunOutput> {
        let mut cmd = Command::new(&self.binary);
        cmd.args(Self::build_args(prompt));
        if let Some(cwd) = &prompt.cwd {
            cmd.current_dir(cwd);
        }
        tracing::debug!(binary = %self.binary.display(), model = ?prompt.model, "spawning gemini (streaming)");
        run_streaming_command(cmd, prompt.timeout, on_chunk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::test_script_lock;

    #[test]
    fn build_args_passes_user_prompt_under_dash_p() {
        let prompt = Prompt {
            user: "what is rust".into(),
            ..Default::default()
        };
        let args = GeminiRunner::build_args(&prompt);
        assert_eq!(args, vec!["-p".to_string(), "what is rust".into()]);
    }

    #[test]
    fn build_args_passes_model_under_dash_m() {
        let prompt = Prompt {
            user: "x".into(),
            model: Some("gemini-2.5-flash".into()),
            ..Default::default()
        };
        let args = GeminiRunner::build_args(&prompt);
        assert!(args.iter().any(|a| a == "-m"));
        let m_idx = args.iter().position(|a| a == "-m").unwrap();
        assert_eq!(args[m_idx + 1], "gemini-2.5-flash");
    }

    #[test]
    fn build_args_prepends_system_prompt_to_user_prompt() {
        let prompt = Prompt {
            user: "what is the capital of France?".into(),
            system: Some("You are a geography tutor.".into()),
            ..Default::default()
        };
        let args = GeminiRunner::build_args(&prompt);
        let p_idx = args.iter().position(|a| a == "-p").unwrap();
        let combined = &args[p_idx + 1];
        // System content first, then a blank line, then the user prompt.
        assert!(combined.starts_with("You are a geography tutor."));
        assert!(combined.contains("what is the capital"));
        assert!(combined.contains("\n\n"));
    }

    #[test]
    fn build_args_omits_system_when_empty() {
        // An empty system prompt should not pollute the user prompt with
        // a leading "\n\n".
        let prompt = Prompt {
            user: "ping".into(),
            system: Some("".into()),
            ..Default::default()
        };
        let args = GeminiRunner::build_args(&prompt);
        let p_idx = args.iter().position(|a| a == "-p").unwrap();
        assert_eq!(args[p_idx + 1], "ping");
    }

    #[test]
    fn build_args_orders_model_before_prompt() {
        // Some CLIs require flags before positional-ish args.
        let prompt = Prompt {
            user: "x".into(),
            model: Some("gemini-2.5-pro".into()),
            ..Default::default()
        };
        let args = GeminiRunner::build_args(&prompt);
        let m_idx = args.iter().position(|a| a == "-m").unwrap();
        let p_idx = args.iter().position(|a| a == "-p").unwrap();
        assert!(m_idx < p_idx);
    }

    #[test]
    fn gemini_runner_with_unknown_binary_returns_not_found() {
        let r = GeminiRunner::with_binary("/nonexistent/coral-test-gemini-xyz");
        let err = r
            .run(&Prompt {
                user: "x".into(),
                ..Default::default()
            })
            .unwrap_err();
        assert!(matches!(err, RunnerError::NotFound));
    }

    #[test]
    fn gemini_runner_runs_against_echo_substitute_with_real_args() {
        // /bin/echo doesn't understand -p / -m, but it does echo whatever
        // it receives. Asserts that we actually pass the new GeminiRunner
        // argv (not ClaudeRunner's --print flag) to the process.
        let r = GeminiRunner::with_binary("/bin/echo");
        let out = r
            .run(&Prompt {
                user: "ping".into(),
                model: Some("gemini-2.5-flash".into()),
                ..Default::default()
            })
            .unwrap();
        assert!(out.stdout.contains("ping"));
        assert!(out.stdout.contains("-p"));
        assert!(out.stdout.contains("-m"));
        assert!(out.stdout.contains("gemini-2.5-flash"));
        // It must NOT contain ClaudeRunner's flag style.
        assert!(
            !out.stdout.contains("--print"),
            "GeminiRunner must not use ClaudeRunner's --print flag"
        );
        assert!(
            !out.stdout.contains("--append-system-prompt"),
            "GeminiRunner must not use ClaudeRunner's --append-system-prompt flag"
        );
    }

    #[test]
    fn gemini_runner_non_zero_returns_error() {
        let r = GeminiRunner::with_binary("/usr/bin/false");
        let err = r
            .run(&Prompt {
                user: "x".into(),
                ..Default::default()
            })
            .unwrap_err();
        assert!(matches!(err, RunnerError::NonZeroExit { .. }));
    }

    /// v0.19.6 audit H2: stderr that contains a bearer-shaped header
    /// must be scrubbed before being wrapped in `RunnerError::*`.
    #[test]
    fn gemini_runner_non_zero_scrubs_bearer_token_from_error() {
        use std::io::Write as _;
        #[cfg(unix)]
        let _lock = test_script_lock();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let script = dir.path().join("fake-gemini.sh");
        // Linux CI fix: `sync_all()` + explicit drop before exec avoids
        // `ETXTBSY` (errno 26) under parallel test load. Same race the
        // streaming-failure-modes helper hits.
        {
            let mut f = std::fs::File::create(&script).expect("create script");
            f.write_all(
                b"#!/bin/sh\n\
                  echo 'request failed; received Authorization: Bearer sk-ant-secret-xxx' 1>&2\n\
                  exit 1\n",
            )
            .expect("write");
            f.sync_all().expect("sync");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script, perms).unwrap();
        }
        let r = GeminiRunner::with_binary(&script);
        let err = r
            .run(&Prompt {
                user: "x".into(),
                ..Default::default()
            })
            .unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            !msg.contains("sk-ant-secret-xxx"),
            "RunnerError leaked the bearer token: {msg}"
        );
        assert!(
            msg.contains("<redacted>"),
            "expected redaction marker: {msg}"
        );
    }

    /// Real smoke against an installed `gemini` CLI. Marked `#[ignore]` because
    /// CI does not install gemini-cli. Run locally with:
    ///
    /// ```bash
    /// cargo test -p coral-runner gemini_runner_smoke_real_gemini -- --ignored
    /// ```
    #[test]
    #[ignore]
    fn gemini_runner_smoke_real_gemini() {
        let r = GeminiRunner::new();
        let prompt = Prompt {
            user: "Say only OK.".into(),
            timeout: Some(Duration::from_secs(60)),
            ..Default::default()
        };
        let out = r.run(&prompt).expect("real gemini smoke");
        assert!(
            out.stdout.contains("OK") || out.stdout.to_lowercase().contains("ok"),
            "stdout did not contain OK: {}",
            out.stdout
        );
    }
}
