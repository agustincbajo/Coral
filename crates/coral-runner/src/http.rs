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
/// the crate so tests can assert the argv shape (audit H5/H6) without
/// spawning a real process.
pub(crate) fn build_curl(runner: &HttpRunner, prompt: &Prompt, body: &str) -> Command {
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
    cmd.args(["-d", body]);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd
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

        let mut cmd = build_curl(self, prompt, &body);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(RunnerError::NotFound);
            }
            Err(e) => return Err(RunnerError::Io(e)),
        };
        if let Some(key) = &self.api_key
            && let Some(mut stdin) = child.stdin.take()
        {
            let header_line = format!("Authorization: Bearer {key}\n");
            std::io::Write::write_all(&mut stdin, header_line.as_bytes()).map_err(|e| {
                RunnerError::Io(std::io::Error::other(format!(
                    "writing auth header to curl stdin: {e}"
                )))
            })?;
        }
        let output = child.wait_with_output().map_err(RunnerError::Io)?;
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
        let cmd = build_curl(&r, &prompt, "{}");
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
        let cmd = build_curl(&r, &prompt, "{}");
        let argv: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(
            argv.iter().any(|a| a == "--max-time"),
            "expected --max-time arg: {argv:?}"
        );
    }
}
