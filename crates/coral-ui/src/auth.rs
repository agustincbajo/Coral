//! Request-time security checks: Host / Origin / bearer token.
//!
//! Defaults are tight: bind defaults to `127.0.0.1`, requests addressed
//! to any other Host are rejected, POST requests with mismatched Origin
//! are rejected, and any LLM- or tool-touching route requires the bearer
//! token (always — even on loopback, because LLM calls cost money).

use tiny_http::Request;

use crate::error::ApiError;
use crate::state::AppState;

/// Loopback-ish bind that's safe without a token.
pub fn is_loopback(bind: &str) -> bool {
    matches!(bind, "127.0.0.1" | "localhost" | "::1" | "[::1]")
}

/// Validate the `Host` request header. Accepts `<bind>:<port>`,
/// `127.0.0.1:<port>`, and `localhost:<port>` so the server tolerates
/// browser quirks (modern browsers normalize loopback aliases freely).
///
/// Missing Host is allowed only when bind is loopback; HTTP/1.1
/// technically mandates Host but tiny_http strips it before we see it.
pub fn validate_host(state: &AppState, request: &Request) -> Result<(), ApiError> {
    let host = header_value(request, "host");
    let bound_pair = format!("{}:{}", state.bind, state.port);
    let loopback_127 = format!("127.0.0.1:{}", state.port);
    let loopback_name = format!("localhost:{}", state.port);

    match host {
        None => {
            if is_loopback(&state.bind) {
                Ok(())
            } else {
                Err(ApiError::InvalidHost)
            }
        }
        Some(h) => {
            let h = h.trim();
            if h.eq_ignore_ascii_case(&bound_pair)
                || h.eq_ignore_ascii_case(&loopback_127)
                || h.eq_ignore_ascii_case(&loopback_name)
            {
                Ok(())
            } else {
                Err(ApiError::InvalidHost)
            }
        }
    }
}

/// Validate the bearer token when required.
///
/// Required when:
///   - `state.token.is_some()` (operator configured one), OR
///   - bind is not loopback (token is mandatory off-loopback).
///
/// Both conditions are guaranteed by `serve()` at startup: a non-loopback
/// bind without a token is rejected before the server starts. This
/// function therefore degenerates to "require if `state.token.is_some()`"
/// in practice — but we keep both checks for defense in depth.
pub fn require_bearer(state: &AppState, request: &Request) -> Result<(), ApiError> {
    let needs_token = state.token.is_some() || !is_loopback(&state.bind);
    if !needs_token {
        return Ok(());
    }

    let Some(expected) = state.token.as_ref() else {
        // Operator bound to a non-loopback address but did not configure
        // a token. `serve()` rejects this at startup, so reaching this
        // arm means we wired things up wrong — surface as
        // LlmNotConfigured rather than a 500.
        return Err(ApiError::LlmNotConfigured(
            "non-loopback bind without --token".into(),
        ));
    };

    let auth = header_value(request, "authorization")
        .ok_or(ApiError::MissingToken)?;
    let trimmed = auth.trim();
    let Some(token) = trimmed.strip_prefix("Bearer ").or_else(|| trimmed.strip_prefix("bearer "))
    else {
        return Err(ApiError::MissingToken);
    };
    if constant_time_eq(token.as_bytes(), expected.as_bytes()) {
        Ok(())
    } else {
        Err(ApiError::InvalidToken)
    }
}

/// Validate the `Origin` header on POST requests. The header is optional
/// (Origin is sent by browsers but not by curl); when present it must
/// match the bound origin or one of its loopback aliases.
pub fn validate_origin(state: &AppState, request: &Request) -> Result<(), ApiError> {
    let Some(origin) = header_value(request, "origin") else {
        return Ok(());
    };
    let origin = origin.trim();
    if origin.is_empty() || origin == "null" {
        return Ok(());
    }
    let accepted = state.accepted_origins();
    // Loopback aliases that we always accept (both schemes), so a user
    // who binds to `0.0.0.0` but accesses via `localhost` doesn't get
    // a spurious 403. The Host header check (`validate_host`) is the
    // hard fence against DNS-rebinding; Origin just rejects obvious
    // cross-site requests.
    let aliases = [
        format!("http://127.0.0.1:{}", state.port),
        format!("https://127.0.0.1:{}", state.port),
        format!("http://localhost:{}", state.port),
        format!("https://localhost:{}", state.port),
    ];
    let matches = accepted
        .iter()
        .chain(aliases.iter())
        .any(|c| origin.eq_ignore_ascii_case(c));
    if matches {
        Ok(())
    } else {
        Err(ApiError::InvalidOrigin)
    }
}

fn header_value(request: &Request, name: &str) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str().to_string())
}

/// Compare two byte slices in constant time. Avoid early-exit timing
/// leaks when comparing user-supplied tokens against the configured one.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_aliases_recognized() {
        assert!(is_loopback("127.0.0.1"));
        assert!(is_loopback("localhost"));
        assert!(is_loopback("::1"));
        assert!(!is_loopback("0.0.0.0"));
        assert!(!is_loopback("192.168.1.1"));
    }

    #[test]
    fn constant_time_eq_basic() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }
}
