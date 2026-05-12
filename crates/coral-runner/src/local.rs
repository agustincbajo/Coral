//! LocalRunner — wraps a `llama-cli` (llama.cpp) binary for offline / cheap
//! batch operations like nightly semantic lint or weekly consolidate.
//!
//! Default flag convention follows llama.cpp's `llama-cli`:
//!
//! - `-p <user>` for the prompt
//! - `-m <model>` for the model file (a `.gguf` path); the runner reads
//!   `prompt.model` as that path verbatim. If `prompt.model` is `None`,
//!   `-m` is omitted and llama-cli falls back to whatever default the
//!   binary picks (usually fails — set the model explicitly).
//! - `--no-display-prompt` so stdout contains only the response, not the
//!   prompt echo. This is the same reason `claude --print` exists for
//!   ClaudeRunner.
//! - System prompts are **prepended** to the user prompt with a blank-line
//!   separator (llama.cpp's basic CLI has no system-prompt flag; richer
//!   templating happens server-side).
//!
//! If your install uses different flags (e.g. a wrapper binary), point
//! `with_binary` at a thin shell script that translates.

use crate::runner::{
    Prompt, RunOutput, Runner, RunnerError, RunnerResult, combine_outputs, is_auth_failure,
    run_streaming_command, scrub_secrets,
};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct LocalRunner {
    binary: PathBuf,
}

impl Default for LocalRunner {
    fn default() -> Self {
        Self {
            binary: PathBuf::from("llama-cli"),
        }
    }
}

impl LocalRunner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the binary path (e.g. `/usr/local/bin/llama-cli`, or a
    /// wrapper script that adapts a different install's flag conventions).
    pub fn with_binary(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    /// Build the argv that gets passed to the local binary. Pure function
    /// so tests can assert the CLI shape without spawning a process.
    pub(crate) fn build_args(prompt: &Prompt) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();
        if let Some(model) = &prompt.model {
            args.push("-m".into());
            args.push(model.clone());
        }
        args.push("--no-display-prompt".into());
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

impl Runner for LocalRunner {
    fn run(&self, prompt: &Prompt) -> RunnerResult<RunOutput> {
        let start = Instant::now();
        let mut cmd = self.build_command(prompt);
        tracing::debug!(
            binary = %self.binary.display(),
            model = ?prompt.model,
            "spawning llama-cli"
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
            // Local runners typically don't auth, but the helper is
            // conservative — a wrapper script that hits a hosted endpoint
            // (or a misconfigured llama.cpp pointed at an auth proxy) can
            // still echo back an `Authorization: Bearer …` header.
            // Without scrubbing, the key would land in CI logs / stack
            // traces. Sister of v0.19.5's H8 fix that already covered
            // http.rs / runner.rs / embeddings.rs.
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

    /// Streaming via the shared `run_streaming_command` helper. llama.cpp
    /// emits tokens as they're generated, so `coral query --provider local`
    /// sees the response unfold in real time.
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
        tracing::debug!(binary = %self.binary.display(), model = ?prompt.model, "spawning llama-cli (streaming)");
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
        let args = LocalRunner::build_args(&prompt);
        let p_idx = args.iter().position(|a| a == "-p").unwrap();
        assert_eq!(args[p_idx + 1], "what is rust");
    }

    #[test]
    fn build_args_includes_no_display_prompt_flag() {
        let prompt = Prompt {
            user: "x".into(),
            ..Default::default()
        };
        let args = LocalRunner::build_args(&prompt);
        assert!(args.iter().any(|a| a == "--no-display-prompt"));
    }

    #[test]
    fn build_args_passes_model_path_under_dash_m() {
        let prompt = Prompt {
            user: "x".into(),
            model: Some("/models/llama-3-8b-Q4_K_M.gguf".into()),
            ..Default::default()
        };
        let args = LocalRunner::build_args(&prompt);
        let m_idx = args.iter().position(|a| a == "-m").unwrap();
        assert_eq!(args[m_idx + 1], "/models/llama-3-8b-Q4_K_M.gguf");
    }

    #[test]
    fn build_args_omits_dash_m_when_model_is_none() {
        let prompt = Prompt {
            user: "x".into(),
            ..Default::default()
        };
        let args = LocalRunner::build_args(&prompt);
        assert!(!args.iter().any(|a| a == "-m"));
    }

    #[test]
    fn build_args_prepends_system_prompt_to_user_prompt() {
        let prompt = Prompt {
            user: "what year is it?".into(),
            system: Some("You are a calendar.".into()),
            ..Default::default()
        };
        let args = LocalRunner::build_args(&prompt);
        let p_idx = args.iter().position(|a| a == "-p").unwrap();
        let combined = &args[p_idx + 1];
        assert!(combined.starts_with("You are a calendar."));
        assert!(combined.contains("\n\n"));
        assert!(combined.contains("what year is it?"));
    }

    #[test]
    fn local_runner_with_unknown_binary_returns_not_found() {
        let r = LocalRunner::with_binary("/nonexistent/coral-test-llama-xyz");
        let err = r
            .run(&Prompt {
                user: "x".into(),
                ..Default::default()
            })
            .unwrap_err();
        assert!(matches!(err, RunnerError::NotFound));
    }

    #[test]
    fn local_runner_runs_against_echo_substitute_with_real_args() {
        // /bin/echo doesn't understand llama flags, but it does echo whatever
        // it receives. Asserts that we actually pass the new LocalRunner argv
        // (not ClaudeRunner's --print or GeminiRunner's argv) to the process.
        let r = LocalRunner::with_binary("/bin/echo");
        let out = r
            .run(&Prompt {
                user: "ping".into(),
                model: Some("/m.gguf".into()),
                ..Default::default()
            })
            .unwrap();
        assert!(out.stdout.contains("ping"));
        assert!(out.stdout.contains("--no-display-prompt"));
        assert!(out.stdout.contains("-m"));
        assert!(out.stdout.contains("/m.gguf"));
        assert!(
            !out.stdout.contains("--print"),
            "LocalRunner must not use ClaudeRunner's --print flag"
        );
    }

    #[test]
    fn local_runner_non_zero_returns_error() {
        let r = LocalRunner::with_binary("/usr/bin/false");
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
    /// We exercise this end-to-end by pointing the runner at a tiny
    /// shell script that prints a fake `Authorization: Bearer …` line
    /// and exits non-zero — the resulting `RunnerError::NonZeroExit`'s
    /// stderr must NOT contain the secret.
    #[test]
    fn local_runner_non_zero_scrubs_bearer_token_from_error() {
        use std::io::Write as _;
        #[cfg(unix)]
        let _lock = test_script_lock();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let script = dir.path().join("fake-llama.sh");
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
        let r = LocalRunner::with_binary(&script);
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

    /// Real smoke against an installed `llama-cli`. Marked `#[ignore]`
    /// because CI does not install llama.cpp. Run locally with:
    ///
    /// ```bash
    /// cargo test -p coral-runner local_runner_smoke_real_llama -- --ignored
    /// ```
    ///
    /// The model path defaults to `$LLAMA_MODEL` if set, otherwise the
    /// test panics with a helpful message instead of guessing.
    #[test]
    #[ignore]
    fn local_runner_smoke_real_llama() {
        let model = std::env::var("LLAMA_MODEL")
            .expect("LLAMA_MODEL env var (path to .gguf file) required for this ignored test");
        let r = LocalRunner::new();
        let prompt = Prompt {
            user: "Reply with the single word OK.".into(),
            model: Some(model),
            timeout: Some(Duration::from_secs(120)),
            ..Default::default()
        };
        let out = r.run(&prompt).expect("real llama-cli smoke");
        assert!(
            out.stdout.to_lowercase().contains("ok"),
            "stdout did not contain OK: {}",
            out.stdout
        );
    }
}
