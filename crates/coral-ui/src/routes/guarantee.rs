//! `GET /api/v1/guarantee?env=<env>&strict=<bool>` — M3 deployment verdict.
//!
//! Shells out to `coral test guarantee --can-i-deploy --format json` and
//! returns the parsed JSON verdict as `data`. The exit code is preserved
//! in `meta.exit_code` so the SPA can distinguish RED (non-zero, deploy
//! blocked) from GREEN (zero, all gates green) without re-parsing the
//! verdict body. Importantly, we parse stdout *even when the exit code
//! is non-zero* — RED is the whole point of the endpoint and emits a
//! valid JSON body alongside its non-zero exit.

use std::sync::Arc;

use crate::error::ApiError;
use crate::state::AppState;

pub fn handle(state: &Arc<AppState>, query_string: &str) -> Result<Vec<u8>, ApiError> {
    let env = query_string
        .split('&')
        .find_map(|p| p.strip_prefix("env="))
        .unwrap_or("");
    let strict = query_string.split('&').any(|p| p == "strict=true");

    // Same sanitization charset as `tools` — we don't want a query
    // string to smuggle a `--malicious` option through to the child.
    if !env.is_empty() && !valid_token(env) {
        return Err(ApiError::InvalidFilter(format!(
            "invalid env: {env:?} (allowed: [A-Za-z0-9._/-])"
        )));
    }

    let exe = std::env::current_exe().map_err(|e| anyhow::anyhow!(e))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("test")
        .arg("guarantee")
        .arg("--can-i-deploy")
        .arg("--format")
        .arg("json");
    if strict {
        cmd.arg("--strict");
    }
    if !env.is_empty() {
        cmd.arg("--env").arg(env);
    }
    cmd.current_dir(
        state
            .wiki_root
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| state.wiki_root.clone()),
    );

    let output = cmd.output().map_err(|e| anyhow::anyhow!(e))?;
    let verdict_json: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| {
        anyhow::anyhow!(
            "guarantee output not JSON: {e}; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })?;
    let body = serde_json::json!({
        "data": verdict_json,
        "meta": {"exit_code": output.status.code()}
    });
    serde_json::to_vec(&body).map_err(|e| anyhow::anyhow!(e).into())
}

/// True iff `s` is non-empty, does not start with `-` (would be parsed
/// as a CLI option), and contains only the conservative
/// `[A-Za-z0-9._/-]` charset. Used to gate query-string values that
/// we forward as CLI arguments to a child process.
fn valid_token(s: &str) -> bool {
    if s.is_empty() || s.starts_with('-') {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn state() -> Arc<AppState> {
        Arc::new(AppState {
            bind: "127.0.0.1".into(),
            port: 3838,
            wiki_root: PathBuf::from("/tmp/coral-test/.wiki"),
            token: None,
            allow_write_tools: false,
            runner: None,
        })
    }

    #[test]
    fn rejects_invalid_env() {
        let s = state();
        let err = handle(&s, "env=--all").unwrap_err();
        assert_eq!(err.code(), "INVALID_FILTER");
        let err = handle(&s, "env=foo;rm").unwrap_err();
        assert_eq!(err.code(), "INVALID_FILTER");
    }

    #[test]
    fn valid_token_accepts_simple_strings() {
        assert!(valid_token("staging"));
        assert!(valid_token("prod-eu"));
        assert!(valid_token("v1.2.3"));
        assert!(!valid_token("--all"));
        assert!(!valid_token("a b"));
    }

    #[test]
    fn child_spawn_failure_surfaces_internal() {
        // The current_exe in tests is the test binary; running it
        // with `test guarantee --can-i-deploy --format json` will not
        // emit JSON to stdout, so we expect an INTERNAL error from
        // the JSON parse step. (If the binary somehow emits JSON, the
        // route returns Ok — also acceptable; we only assert no panic
        // and a deterministic shape.)
        let s = state();
        match handle(&s, "") {
            Ok(body) => {
                let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert!(v["data"].is_object() || v["data"].is_array() || v["data"].is_null());
            }
            Err(e) => assert_eq!(e.code(), "INTERNAL"),
        }
    }
}
