//! HttpRunner — generic OpenAI-compatible `/v1/chat/completions` runner.
//!
//! Posts a JSON Chat Completions request to any HTTP endpoint that
//! speaks the OpenAI shape. That covers vLLM, Ollama (its `/v1/...`
//! compatibility surface), OpenAI itself, and any local server that
//! mimics the same wire format. Anthropic Messages users can point
//! this at a compat shim.
//!
//! Conventions:
//!
//! - Shells out to `curl` (same pattern as `embeddings::VoyageProvider`)
//!   so the sync CLI doesn't have to drag in `reqwest` + `tokio`.
//! - `prompt.system` becomes a `system` role message; if `None` the
//!   `messages` array is just the single `user` entry.
//! - `prompt.model` is sent verbatim. When `None`, a literal
//!   `"default"` is sent — strict endpoints will reject this with a
//!   surface-able 4xx, which is the right behavior (we don't want to
//!   silently pick a vendor-specific default).
//! - On non-zero `curl` exit, the same `combine_outputs` +
//!   `is_auth_failure` path used by `LocalRunner` / `GeminiRunner`
//!   maps 401-shaped failures to `RunnerError::AuthFailed` and
//!   everything else to `RunnerError::NonZeroExit`.
//! - Response parsing failures become `RunnerError::Io(io::Error::other(_))`
//!   — `RunnerError` has no dedicated `Parse` variant in this iteration.

use crate::runner::{
    Prompt, RunOutput, Runner, RunnerError, RunnerResult, combine_outputs, is_auth_failure,
    scrub_secrets,
};
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::time::Instant;

/// Fallback model string when `prompt.model` is `None`. Endpoints that
/// require an explicit model will reject this with an actionable 4xx.
const DEFAULT_MODEL_PLACEHOLDER: &str = "default";

/// Generic OpenAI-compatible chat-completions runner.
///
/// Construct with [`HttpRunner::new`] (endpoint URL) and optionally chain
/// [`HttpRunner::with_api_key`] for `Authorization: Bearer …`.
#[derive(Debug, Clone)]
pub struct HttpRunner {
    /// Full URL to the chat-completions endpoint, e.g.
    /// `http://localhost:8000/v1/chat/completions`.
    endpoint: String,
    /// Optional bearer token sent as `Authorization: Bearer <key>`.
    api_key: Option<String>,
}

impl HttpRunner {
    /// Build a runner targeting a specific OpenAI-compatible endpoint.
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            api_key: None,
        }
    }

    /// Attach a bearer token. Used as `Authorization: Bearer <key>`.
    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }
}

// --- Wire types --------------------------------------------------------------

/// One message in the chat-completions `messages` array.
#[derive(Debug, Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

/// Outgoing chat-completions request body. Owned strings keep the
/// serialized payload self-contained so it can be moved into the
/// `Command` arg list without lifetime juggling.
#[derive(Debug, Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    stream: bool,
}

/// Incoming chat-completions response — only the bits we read.
#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Debug, Deserialize)]
struct Message {
    content: String,
}

/// Build the JSON request body for a given [`Prompt`]. Pure function —
/// no I/O — so tests can assert the wire shape without spawning curl.
///
/// Returns the serialized JSON string (ready for `-d`).
pub(crate) fn build_payload(prompt: &Prompt) -> Result<String, serde_json::Error> {
    let model = prompt.model.as_deref().unwrap_or(DEFAULT_MODEL_PLACEHOLDER);
    let mut messages: Vec<ChatMessage<'_>> = Vec::new();
    if let Some(system) = prompt.system.as_deref() {
        if !system.is_empty() {
            messages.push(ChatMessage {
                role: "system",
                content: system,
            });
        }
    }
    messages.push(ChatMessage {
        role: "user",
        content: &prompt.user,
    });
    let req = ChatCompletionRequest {
        model,
        messages,
        stream: false,
    };
    serde_json::to_string(&req)
}

/// Build the curl invocation for a given prompt + body. Public to
/// the crate so tests can assert the argv shape (audit H5/H6/N2)
/// without spawning a real process.
///
/// v0.19.6 audit N2: the prompt body is now piped through stdin
/// (when no API key is set) or written to a sibling temp file and
/// referenced via `--data-binary @<path>` (when an API key is also
/// set, since `-H @-` already consumes all of stdin). Either way
/// the body text never lands in argv where `ps` / `/proc/<pid>/cmdline`
/// would expose it.
///
/// `body_path` is a per-call temp file path resolved by
/// [`runner_body_tempfile_path`] (or `None` when stdin can carry the
/// body). Caller is responsible for writing the body to that path
/// BEFORE spawning, and for removing it after `wait_with_output`.
pub(crate) fn build_curl(
    runner: &HttpRunner,
    prompt: &Prompt,
    body_path: Option<&std::path::Path>,
) -> Command {
    let mut cmd = Command::new("curl");
    cmd.args([
        "-s",
        "--fail-with-body",
        "-X",
        "POST",
        runner.endpoint.as_str(),
        "-H",
        "Content-Type: application/json",
    ]);
    // v0.19.5 audit H5: honour `prompt.timeout` by translating it to
    // curl's `--max-time`. Previously the timeout field on `Prompt`
    // was silently ignored, so callers wiring a deadline (e.g. `coral
    // test` cells) would still hang indefinitely.
    if let Some(t) = prompt.timeout {
        let secs = (t.as_secs_f64()).max(1.0);
        cmd.args(["--max-time", &format!("{secs:.0}")]);
    }
    // v0.19.5 audit H6: pipe Authorization via stdin so the key never
    // appears in argv (where it would be readable by every process
    // via `ps` / `/proc/<pid>/cmdline`). curl's `@-` form tells `-H`
    // to read header lines from stdin.
    if runner.api_key.is_some() {
        cmd.args(["-H", "@-"]);
    }
    // v0.19.6 audit N2: body source. With an API key we already
    // consumed stdin for headers — fall through to `--data-binary
    // @<temp-file>`. Without an API key, stream the body via stdin
    // so we don't touch the disk at all.
    if let Some(p) = body_path {
        // `@<path>` arg form: still leaks the path itself (which is a
        // tmpfile name), but NOT the body contents.
        cmd.args(["--data-binary", &format!("@{}", p.display())]);
    } else {
        cmd.args(["--data-binary", "@-"]);
    }
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd
}

/// Resolve a per-call temp file path for the curl request body.
/// Pid + nanos + atomic counter keeps it unique without bringing in a
/// new dep. Created via [`write_body_tempfile_secure`] and cleaned up
/// by [`TempFileGuard`]'s `Drop` impl.
///
/// v0.19.6 audit N2.
fn runner_body_tempfile_path() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let counter = N.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("coral-runner-body-{pid}-{nanos}-{counter}.json"))
}

/// Write `contents` to `path` with `O_CREAT | O_EXCL | mode 0600` on
/// Unix so the prompt body in `/tmp` can't be `cat`-ed by another
/// local user during the in-flight curl call.
///
/// v0.19.7 audit-followup #24: pre-v0.19.7 the body went out at
/// `mode 0644` (default umask), which restricted WRITE but not READ
/// on Linux multi-tenant hosts where `/tmp` is shared across UIDs.
/// macOS is unaffected because `$TMPDIR` is per-user under
/// `/var/folders/<hash>/T/`. We `create_new(true)` so a pre-positioned
/// symlink at the target path can't trick us into clobbering an
/// attacker-chosen file (defense-in-depth; the path-generation already
/// produces collision-resistant names).
fn write_body_tempfile_secure(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(contents)
}

/// RAII cleanup for the per-call request-body tempfile. Pre-v0.19.7
/// the cleanup was hand-rolled at the success path plus a couple of
/// the error paths; the validator agent caught three error paths
/// (`stdin.write_all` for header, `stdin.write_all` for body,
/// `child.wait_with_output()`) where the file leaked. RAII makes
/// cleanup uniform across every return path including panic unwinding.
///
/// v0.19.7 audit-followup #25.
struct TempFileGuard {
    path: Option<std::path::PathBuf>,
}

impl TempFileGuard {
    /// Bind a guard to `path`. `None` means "no tempfile in play"
    /// (e.g. the no-API-key code path that streams the body via
    /// stdin); the guard's `Drop` is then a no-op.
    fn new(path: Option<std::path::PathBuf>) -> Self {
        Self { path }
    }
    fn as_path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if let Some(p) = self.path.take() {
            let _ = std::fs::remove_file(&p);
        }
    }
}

impl Runner for HttpRunner {
    fn run(&self, prompt: &Prompt) -> RunnerResult<RunOutput> {
        let start = Instant::now();
        let body = build_payload(prompt).map_err(|e| {
            RunnerError::Io(std::io::Error::other(format!("serializing request: {e}")))
        })?;

        tracing::debug!(
            endpoint = %self.endpoint,
            model = ?prompt.model,
            "POST chat-completions"
        );

        // v0.19.6 audit N2: route the body away from argv. When no API
        // key is set, stdin is free → stream the body there. When an
        // API key IS set, the H6 fix already claimed stdin for the
        // header — write the body to a per-call temp file and pass it
        // via `--data-binary @<path>`. Either way the body bytes never
        // appear in `ps` / `/proc/<pid>/cmdline`.
        //
        // v0.19.7 hardening (#24, #25): write the tempfile with
        // `mode 0600` so other local users can't read it from `/tmp`
        // on Linux, and bind a `TempFileGuard` so cleanup happens on
        // every return path (including the three the v0.19.6 review
        // flagged: header-write fail, body-write fail, wait-output fail).
        let (body_guard, body_via_stdin) = if self.api_key.is_some() {
            let p = runner_body_tempfile_path();
            write_body_tempfile_secure(&p, body.as_bytes()).map_err(|e| {
                RunnerError::Io(std::io::Error::other(format!(
                    "writing request body to {}: {e}",
                    p.display()
                )))
            })?;
            (TempFileGuard::new(Some(p)), false)
        } else {
            (TempFileGuard::new(None), true)
        };

        let mut cmd = build_curl(self, prompt, body_guard.as_path());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(RunnerError::NotFound);
            }
            Err(e) => {
                return Err(RunnerError::Io(e));
            }
        };
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            // v0.19.5 audit H6: header via stdin so the bearer token
            // never lands in argv.
            if let Some(key) = &self.api_key {
                let header_line = format!("Authorization: Bearer {key}\n");
                stdin.write_all(header_line.as_bytes()).map_err(|e| {
                    RunnerError::Io(std::io::Error::other(format!(
                        "writing auth header to curl stdin: {e}"
                    )))
                })?;
            }
            // v0.19.6 audit N2: body via stdin only when there's no
            // API key (since `-H @-` already consumed stdin).
            if body_via_stdin {
                stdin.write_all(body.as_bytes()).map_err(|e| {
                    RunnerError::Io(std::io::Error::other(format!(
                        "writing request body to curl stdin: {e}"
                    )))
                })?;
            }
            // Drop stdin so curl sees EOF and the request actually
            // sends. Without this the child blocks waiting for more
            // bytes from us.
            drop(stdin);
        }
        let output = child.wait_with_output().map_err(RunnerError::Io)?;
        // body_guard drops at end of scope → tempfile removed.
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

        let parsed: ChatCompletionResponse =
            serde_json::from_slice(&output.stdout).map_err(|e| {
                RunnerError::Io(std::io::Error::other(format!(
                    "parsing chat-completions response: {e}; body={}",
                    String::from_utf8_lossy(&output.stdout)
                )))
            })?;
        let content = parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| {
                RunnerError::Io(std::io::Error::other(
                    "chat-completions response had no choices",
                ))
            })?;

        Ok(RunOutput {
            stdout: content,
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            duration,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_payload_serializes_full_chat_completion_shape() {
        let prompt = Prompt {
            system: Some("You are a helpful assistant.".into()),
            user: "Say hello.".into(),
            model: Some("gpt-4o-mini".into()),
            ..Default::default()
        };
        let body = build_payload(&prompt).expect("serialize");
        assert!(
            body.contains("\"model\":\"gpt-4o-mini\""),
            "missing model field: {body}"
        );
        assert!(
            body.contains("\"messages\":["),
            "missing messages array: {body}"
        );
        assert!(
            body.contains("\"role\":\"system\""),
            "missing system role: {body}"
        );
        assert!(
            body.contains("\"role\":\"user\""),
            "missing user role: {body}"
        );
        assert!(
            body.contains("\"content\":\"You are a helpful assistant.\""),
            "missing system content: {body}"
        );
        assert!(
            body.contains("\"content\":\"Say hello.\""),
            "missing user content: {body}"
        );
        assert!(body.contains("\"stream\":false"), "missing stream: {body}");
    }

    #[test]
    fn build_payload_omits_system_when_none() {
        let prompt = Prompt {
            user: "ping".into(),
            model: Some("gpt-4o-mini".into()),
            ..Default::default()
        };
        let body = build_payload(&prompt).expect("serialize");
        // Re-parse and check the messages array length is exactly 1.
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("re-parse");
        let messages = parsed["messages"]
            .as_array()
            .expect("messages should be an array");
        assert_eq!(
            messages.len(),
            1,
            "expected 1 message (just user), got {}: {body}",
            messages.len()
        );
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "ping");
    }

    #[test]
    fn build_payload_uses_default_placeholder_when_model_missing() {
        // Documents the contract: prompt.model = None → literal "default".
        let prompt = Prompt {
            user: "ping".into(),
            ..Default::default()
        };
        let body = build_payload(&prompt).expect("serialize");
        assert!(
            body.contains("\"model\":\"default\""),
            "expected default model placeholder: {body}"
        );
    }

    #[test]
    fn build_payload_uses_prompt_model_when_set() {
        // Pin: when prompt.model is Some, that exact string lands in the
        // body's `model` field — no munging, no fallback.
        let prompt = Prompt {
            user: "ping".into(),
            model: Some("llama-3-8b".into()),
            ..Default::default()
        };
        let body = build_payload(&prompt).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("re-parse");
        assert_eq!(parsed["model"], "llama-3-8b");
    }

    #[test]
    fn build_payload_falls_back_to_default_model() {
        // Pin: when prompt.model is None, the literal "default" is sent
        // (per spec — strict endpoints will reject with 4xx, which is
        // the right surfaced behavior).
        let prompt = Prompt {
            user: "ping".into(),
            ..Default::default()
        };
        let body = build_payload(&prompt).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("re-parse");
        assert_eq!(parsed["model"], DEFAULT_MODEL_PLACEHOLDER);
        assert_eq!(parsed["model"], "default");
    }

    #[test]
    fn build_payload_with_only_user_prompt_has_one_message() {
        // Pin: prompt.system = None → messages is exactly [user].
        let prompt = Prompt {
            user: "hi".into(),
            ..Default::default()
        };
        let body = build_payload(&prompt).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("re-parse");
        let messages = parsed["messages"]
            .as_array()
            .expect("messages should be an array");
        assert_eq!(messages.len(), 1, "expected 1 message: {body}");
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "hi");
    }

    #[test]
    fn build_payload_with_system_prompt_has_two_messages() {
        // Pin: prompt.system = Some(non-empty) → messages = [system, user]
        // with that exact ordering and roles.
        let prompt = Prompt {
            system: Some("you are X".into()),
            user: "hello".into(),
            ..Default::default()
        };
        let body = build_payload(&prompt).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("re-parse");
        let messages = parsed["messages"]
            .as_array()
            .expect("messages should be an array");
        assert_eq!(messages.len(), 2, "expected 2 messages: {body}");
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "you are X");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "hello");
    }

    #[test]
    fn build_payload_omits_empty_system_string() {
        // Pin: prompt.system = Some("") behaves like None — the empty
        // system message is dropped, so the wire payload is identical
        // to the no-system case. This avoids sending vendor-confusing
        // `{"role":"system","content":""}` blobs.
        let prompt = Prompt {
            system: Some(String::new()),
            user: "hi".into(),
            ..Default::default()
        };
        let body = build_payload(&prompt).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("re-parse");
        let messages = parsed["messages"]
            .as_array()
            .expect("messages should be an array");
        assert_eq!(
            messages.len(),
            1,
            "empty system should be dropped, got {body}"
        );
        assert_eq!(messages[0]["role"], "user");
    }

    #[test]
    fn build_payload_includes_stream_false() {
        // Pin: streaming is disabled at the wire level. The trait's
        // default `run_streaming` impl handles the chunked surface.
        let prompt = Prompt {
            user: "hi".into(),
            ..Default::default()
        };
        let body = build_payload(&prompt).expect("serialize");
        assert!(
            body.contains("\"stream\":false"),
            "stream:false not in body: {body}"
        );
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("re-parse");
        assert_eq!(parsed["stream"], serde_json::Value::Bool(false));
    }

    #[test]
    fn http_runner_against_unreachable_endpoint_returns_err() {
        // Picks a port that is virtually never bound. curl is universally
        // installed on macOS + Linux CI; the call must return Err of any
        // flavor (NonZeroExit on connection refused, AuthFailed if some
        // local thing answers with 401 — both acceptable). The point is to
        // exercise the curl-spawn + error-mapping path without panicking.
        let r = HttpRunner::new("http://127.0.0.1:1/v1/chat/completions");
        let res = r.run(&Prompt {
            user: "x".into(),
            ..Default::default()
        });
        assert!(res.is_err(), "expected Err, got Ok: {res:?}");
    }

    #[test]
    fn with_api_key_chains_and_stores_token() {
        // Smoke check on the builder pattern; does not spawn curl.
        let r = HttpRunner::new("http://example.invalid/v1/chat/completions")
            .with_api_key("sk-test-1234");
        assert_eq!(r.api_key.as_deref(), Some("sk-test-1234"));
    }

    /// v0.19.5 audit H6: regression — `Authorization: Bearer …` must
    /// not appear in argv. The header instead arrives via stdin.
    #[test]
    fn build_curl_does_not_put_bearer_in_argv() {
        let r = HttpRunner::new("http://example.invalid/v1/chat/completions")
            .with_api_key("sk-test-secret");
        let prompt = Prompt {
            user: "hi".into(),
            ..Default::default()
        };
        // With API key set, body goes via tempfile path.
        let body_path = std::path::PathBuf::from("/tmp/coral-runner-test-body.json");
        let cmd = build_curl(&r, &prompt, Some(&body_path));
        let argv: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(
            argv.iter().all(|a| !a.contains("sk-test-secret")),
            "argv leaked the bearer token: {argv:?}"
        );
        assert!(
            argv.iter().all(|a| !a.starts_with("Authorization:")),
            "argv contained a header line for Authorization: {argv:?}"
        );
        // The `@-` sentinel is what tells curl to read the header
        // from stdin instead. Pin it so future refactors don't drop
        // the indirection.
        assert!(
            argv.iter().any(|a| a == "@-"),
            "missing @- sentinel: {argv:?}"
        );
    }

    /// v0.19.5 audit H5: regression — `prompt.timeout` translates to
    /// curl's `--max-time`.
    #[test]
    fn build_curl_propagates_prompt_timeout() {
        let r = HttpRunner::new("http://example.invalid/v1/chat/completions");
        let prompt = Prompt {
            user: "hi".into(),
            timeout: Some(std::time::Duration::from_secs(7)),
            ..Default::default()
        };
        // No API key → body via stdin.
        let cmd = build_curl(&r, &prompt, None);
        let argv: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(
            argv.iter().any(|a| a == "--max-time"),
            "expected --max-time arg: {argv:?}"
        );
    }

    /// v0.19.6 audit N2: regression — the prompt body itself must NOT
    /// appear in argv. Pre-fix the JSON-serialized request body sat on
    /// the command line as `-d <body>`, exposing it to every other
    /// process via `ps` / `/proc/<pid>/cmdline`.
    #[test]
    fn build_curl_does_not_put_body_in_argv() {
        let r = HttpRunner::new("http://example.invalid/v1/chat/completions")
            .with_api_key("sk-test-secret");
        let prompt = Prompt {
            user: "the secret prompt content sentinel: pineapple-42".into(),
            ..Default::default()
        };
        let body = build_payload(&prompt).expect("serialize");
        // With API key set, run() routes body via tempfile.
        let body_path = std::path::PathBuf::from("/tmp/coral-runner-test-body-2.json");
        let cmd = build_curl(&r, &prompt, Some(&body_path));
        let argv: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(
            argv.iter().all(|a| !a.contains("pineapple-42")),
            "argv leaked the prompt content: {argv:?}"
        );
        assert!(
            argv.iter().all(|a| !a.contains(body.as_str())),
            "argv leaked the JSON body verbatim: {argv:?}"
        );
        // Body should be referenced via `--data-binary @<path>`.
        assert!(
            argv.iter().any(|a| a.starts_with('@')),
            "missing @<path> body reference: {argv:?}"
        );
        // No bare `-d` arg should remain — the migration to
        // `--data-binary` is intentional.
        assert!(
            !argv.iter().any(|a| a == "-d"),
            "argv still contains `-d`; body migration to --data-binary missing: {argv:?}"
        );
    }

    /// v0.19.6 audit N2: when no API key is set, the body is streamed
    /// via stdin (`--data-binary @-`) instead of a tempfile. Either
    /// way it stays out of argv.
    #[test]
    fn build_curl_streams_body_via_stdin_when_no_api_key() {
        let r = HttpRunner::new("http://example.invalid/v1/chat/completions");
        let prompt = Prompt {
            user: "secret-no-key-12345".into(),
            ..Default::default()
        };
        let cmd = build_curl(&r, &prompt, None);
        let argv: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(
            argv.iter().any(|a| a == "@-"),
            "expected `@-` stdin sentinel for body: {argv:?}"
        );
        assert!(
            argv.iter().all(|a| !a.contains("secret-no-key-12345")),
            "argv leaked the prompt content: {argv:?}"
        );
    }

    /// v0.19.7 hardening (#24): the request-body tempfile is created with
    /// mode 0600 on Unix so other local users can't `cat` it from
    /// `/tmp`. Pre-v0.19.7 the file was created via `std::fs::write`
    /// which lands at the umask default (0644 typically), restricting
    /// WRITE but not READ — a privacy leak on shared Linux hosts.
    #[cfg(unix)]
    #[test]
    fn body_tempfile_is_created_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("body.json");
        write_body_tempfile_secure(&path, b"hello").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "tempfile mode is {mode:o}, expected 0600 — see GitHub issue #24"
        );
    }

    /// v0.19.7 hardening (#24): `create_new(true)` semantics — refuse
    /// to clobber a pre-existing file. Defense-in-depth against a
    /// pre-positioned symlink at the target path even though our
    /// path-generation already produces collision-resistant names.
    #[cfg(unix)]
    #[test]
    fn body_tempfile_secure_refuses_to_clobber() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("body.json");
        std::fs::write(&path, b"existing").unwrap();
        let err = write_body_tempfile_secure(&path, b"new").expect_err("must fail");
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    }

    /// v0.19.7 hardening (#25): `TempFileGuard` cleans up on Drop.
    /// Pre-v0.19.7 the cleanup was hand-rolled at three of the four
    /// return paths; the fourth (`stdin.write_all` for the body)
    /// leaked. RAII makes cleanup uniform across success, error, and
    /// panic-unwind paths.
    #[test]
    fn temp_file_guard_removes_path_on_drop() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("guarded.json");
        std::fs::write(&path, b"x").unwrap();
        assert!(path.exists());
        {
            let _g = TempFileGuard::new(Some(path.clone()));
        }
        assert!(
            !path.exists(),
            "TempFileGuard did not remove the file on drop"
        );
    }

    /// v0.19.7 hardening (#25): a guard with `None` is a no-op on Drop.
    /// (Used on the no-API-key code path where the body streams via
    /// stdin and there's no tempfile in play.)
    #[test]
    fn temp_file_guard_with_none_is_noop() {
        let g = TempFileGuard::new(None);
        drop(g);
        // No panic, no file to assert on — the absence of error is
        // the contract.
    }
}
