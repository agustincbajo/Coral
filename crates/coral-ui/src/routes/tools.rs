//! `POST /api/v1/tools/{verify,run_test,up,down}` — gated mutation
//! endpoints that shell out to the running `coral` binary.
//!
//! Each handler re-invokes `std::env::current_exe()` with the
//! corresponding subcommand and JSON-encoded args. This sidesteps the
//! cyclic dependency we'd hit by linking `coral-cli` directly from
//! `coral-ui`, and keeps the CLI as the single source of truth for
//! behavior. Output is captured with `Command::output()` (fully
//! buffered) — fine for the kind of short-lived commands these tools
//! invoke; if any of them grow into long-running streaming jobs we
//! can promote them to the `/api/v1/query` SSE pattern.
//!
//! All four handlers respect `state.allow_write_tools`. When the flag
//! is false (the default), they return `WRITE_TOOLS_DISABLED` (403)
//! before doing any work — that's both the gate the operator opts into
//! at startup and the surface the SPA reads via `runtime_config_json`
//! to hide the "Run" buttons.
//!
//! stdout/stderr are tailed to the last 4 KiB before returning, so a
//! verbose run doesn't blow up the response. The full output stays
//! visible in the operator's terminal (we don't redirect it anywhere
//! else); the API surface is for the SPA's status badge, not for
//! collecting logs.

use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::state::AppState;

const TAIL_BYTES: usize = 4096;

#[derive(Debug, Deserialize, Default)]
pub struct VerifyBody {
    #[serde(default)]
    pub env: Option<String>,
}

/// `coral test` flag set (no `run` subcommand exists; the flat form is
/// `coral test --service <s> --kind <k> --tag <t> --env <e>`). All
/// filters are repeatable in the CLI; the WebUI exposes them as
/// optional arrays.
#[derive(Debug, Deserialize, Default)]
pub struct RunTestBody {
    #[serde(default)]
    pub services: Vec<String>,
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub env: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct UpBody {
    #[serde(default)]
    pub env: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct DownBody {
    #[serde(default)]
    pub env: Option<String>,
    #[serde(default)]
    pub volumes: Option<bool>,
}

#[derive(Debug, Serialize)]
struct ToolResult {
    status: String,
    exit_code: Option<i32>,
    stdout_tail: String,
    stderr_tail: String,
    duration_ms: u128,
}

fn check_gate(state: &AppState) -> Result<(), ApiError> {
    if !state.allow_write_tools {
        return Err(ApiError::WriteToolsDisabled);
    }
    Ok(())
}

/// Sanitize a free-form string before passing it as a CLI argument. We
/// trust `current_exe()` (no shell), but `coral verify --env` parses
/// the value itself, and we don't want to give the SPA a way to smuggle
/// option-injection (`--something-malicious`) through here. The same
/// charset as `affected`'s git ref check.
fn sanitize_arg(value: &str, label: &str) -> Result<(), ApiError> {
    if value.is_empty() {
        return Err(ApiError::InvalidFilter(format!("{label}: empty value")));
    }
    // Leading `-` would be parsed as an option flag by the child
    // process. Reject early so a caller can't smuggle `--something`.
    if value.starts_with('-') {
        return Err(ApiError::InvalidFilter(format!(
            "{label}: must not start with '-' ({value:?})"
        )));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
    {
        return Err(ApiError::InvalidFilter(format!(
            "{label}: invalid characters in {value:?}"
        )));
    }
    Ok(())
}

pub fn handle_verify(state: &Arc<AppState>, body: &[u8]) -> Result<Vec<u8>, ApiError> {
    check_gate(state)?;
    let req: VerifyBody = if body.is_empty() {
        VerifyBody::default()
    } else {
        serde_json::from_slice(body)
            .map_err(|e| ApiError::InvalidFilter(format!("malformed JSON body: {e}")))?
    };
    let exe = std::env::current_exe().map_err(|e| anyhow::anyhow!(e))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("verify");
    if let Some(env) = &req.env {
        sanitize_arg(env, "env")?;
        cmd.arg("--env").arg(env);
    }
    cmd.current_dir(repo_root(state));
    run_and_wrap(cmd)
}

pub fn handle_run_test(state: &Arc<AppState>, body: &[u8]) -> Result<Vec<u8>, ApiError> {
    check_gate(state)?;
    let req: RunTestBody = if body.is_empty() {
        RunTestBody::default()
    } else {
        serde_json::from_slice(body)
            .map_err(|e| ApiError::InvalidFilter(format!("malformed JSON body: {e}")))?
    };
    let exe = std::env::current_exe().map_err(|e| anyhow::anyhow!(e))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("test");
    if let Some(env) = &req.env {
        sanitize_arg(env, "env")?;
        cmd.arg("--env").arg(env);
    }
    for s in &req.services {
        sanitize_arg(s, "service")?;
        cmd.arg("--service").arg(s);
    }
    for k in &req.kinds {
        sanitize_arg(k, "kind")?;
        cmd.arg("--kind").arg(k);
    }
    for t in &req.tags {
        sanitize_arg(t, "tag")?;
        cmd.arg("--tag").arg(t);
    }
    cmd.arg("--format").arg("json");
    cmd.current_dir(repo_root(state));
    run_and_wrap(cmd)
}

pub fn handle_up(state: &Arc<AppState>, body: &[u8]) -> Result<Vec<u8>, ApiError> {
    check_gate(state)?;
    let req: UpBody = if body.is_empty() {
        UpBody::default()
    } else {
        serde_json::from_slice(body)
            .map_err(|e| ApiError::InvalidFilter(format!("malformed JSON body: {e}")))?
    };
    let exe = std::env::current_exe().map_err(|e| anyhow::anyhow!(e))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("up");
    if let Some(env) = &req.env {
        sanitize_arg(env, "env")?;
        cmd.arg("--env").arg(env);
    }
    cmd.current_dir(repo_root(state));
    run_and_wrap(cmd)
}

pub fn handle_down(state: &Arc<AppState>, body: &[u8]) -> Result<Vec<u8>, ApiError> {
    check_gate(state)?;
    let req: DownBody = if body.is_empty() {
        DownBody::default()
    } else {
        serde_json::from_slice(body)
            .map_err(|e| ApiError::InvalidFilter(format!("malformed JSON body: {e}")))?
    };
    let exe = std::env::current_exe().map_err(|e| anyhow::anyhow!(e))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("down");
    if let Some(env) = &req.env {
        sanitize_arg(env, "env")?;
        cmd.arg("--env").arg(env);
    }
    if req.volumes.unwrap_or(false) {
        cmd.arg("--volumes");
    }
    cmd.current_dir(repo_root(state));
    run_and_wrap(cmd)
}

fn repo_root(state: &AppState) -> std::path::PathBuf {
    state
        .wiki_root
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| state.wiki_root.clone())
}

fn run_and_wrap(mut cmd: std::process::Command) -> Result<Vec<u8>, ApiError> {
    let started = Instant::now();
    let output = cmd.output().map_err(|e| anyhow::anyhow!(e))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let result = ToolResult {
        status: if output.status.success() {
            "ok".into()
        } else {
            "error".into()
        },
        exit_code: output.status.code(),
        stdout_tail: tail(&stdout),
        stderr_tail: tail(&stderr),
        duration_ms: started.elapsed().as_millis(),
    };
    let body = serde_json::json!({"data": result});
    serde_json::to_vec(&body).map_err(|e| anyhow::anyhow!(e).into())
}

/// Last `TAIL_BYTES` of a UTF-8 string, snapped to a char boundary so
/// we don't slice a multi-byte sequence in half. Returns the whole
/// string if it's already short enough.
fn tail(s: &str) -> String {
    if s.len() <= TAIL_BYTES {
        return s.to_string();
    }
    let start = s.len() - TAIL_BYTES;
    let mut snap = start;
    while snap < s.len() && !s.is_char_boundary(snap) {
        snap += 1;
    }
    s[snap..].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn state(allow_write_tools: bool) -> Arc<AppState> {
        Arc::new(AppState {
            bind: "127.0.0.1".into(),
            port: 3838,
            wiki_root: PathBuf::from("/tmp/coral-test/.wiki"),
            token: None,
            allow_write_tools,
            runner: None,
        })
    }

    #[test]
    fn verify_blocked_when_gate_off() {
        let s = state(false);
        let err = handle_verify(&s, b"{}").unwrap_err();
        assert_eq!(err.code(), "WRITE_TOOLS_DISABLED");
    }

    #[test]
    fn run_test_blocked_when_gate_off() {
        let s = state(false);
        let err = handle_run_test(&s, b"{}").unwrap_err();
        assert_eq!(err.code(), "WRITE_TOOLS_DISABLED");
    }

    #[test]
    fn up_blocked_when_gate_off() {
        let s = state(false);
        let err = handle_up(&s, b"{}").unwrap_err();
        assert_eq!(err.code(), "WRITE_TOOLS_DISABLED");
    }

    #[test]
    fn down_blocked_when_gate_off() {
        let s = state(false);
        let err = handle_down(&s, b"{}").unwrap_err();
        assert_eq!(err.code(), "WRITE_TOOLS_DISABLED");
    }

    #[test]
    fn sanitize_rejects_injection() {
        assert!(sanitize_arg("--all", "env").is_err());
        assert!(sanitize_arg("foo;rm", "env").is_err());
        assert!(sanitize_arg("$(id)", "env").is_err());
        assert!(sanitize_arg("", "env").is_err());
    }

    #[test]
    fn sanitize_accepts_safe_values() {
        assert!(sanitize_arg("staging", "env").is_ok());
        assert!(sanitize_arg("prod-eu", "env").is_ok());
        assert!(sanitize_arg("v1.2.3", "case_id").is_ok());
        assert!(sanitize_arg("nested/path", "kind").is_ok());
    }

    #[test]
    fn tail_short_string_unchanged() {
        assert_eq!(tail("hello"), "hello");
    }

    #[test]
    fn tail_truncates_to_last_4k() {
        let s = "x".repeat(8000);
        let t = tail(&s);
        assert_eq!(t.len(), TAIL_BYTES);
    }

    #[test]
    fn tail_handles_multibyte_at_boundary() {
        // Repeat a 3-byte char so the naive `len - 4096` cut would land
        // mid-codepoint. The snap-to-boundary logic should never panic.
        let s = "あ".repeat(4000);
        let _ = tail(&s);
    }

    #[test]
    fn malformed_json_body_rejected_when_gate_on() {
        let s = state(true);
        let err = handle_verify(&s, b"{not json").unwrap_err();
        assert_eq!(err.code(), "INVALID_FILTER");
    }

    #[test]
    fn empty_body_uses_defaults_when_gate_on_but_spawn_may_fail() {
        // With the gate on and an empty body, we attempt to spawn
        // current_exe() with "verify". In the test harness the exe is
        // the test binary, which will exit non-zero — but we still get
        // back a valid envelope (status=error). Spawn errors are an
        // INTERNAL error, but Command::output() succeeding with a
        // non-zero exit is the path we expect here.
        let s = state(true);
        match handle_verify(&s, b"") {
            Ok(body) => {
                let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert!(v["data"]["status"].is_string());
                assert!(v["data"]["duration_ms"].is_number());
            }
            Err(e) => {
                // Acceptable if the test binary can't be spawned in
                // this environment — we just need the path to be
                // exercised. Any error must be `INTERNAL`.
                assert_eq!(e.code(), "INTERNAL");
            }
        }
    }
}
