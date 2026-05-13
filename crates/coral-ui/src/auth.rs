//! Request-time security checks: Host / Origin / bearer token.
//!
//! Defaults are tight: bind defaults to `127.0.0.1`, requests addressed
//! to any other Host are rejected, POST requests with mismatched Origin
//! are rejected, and any LLM- or tool-touching route requires the bearer
//! token (always — even on loopback, because LLM calls cost money).
//!
//! v0.35 SEC-01: the bearer + loopback + constant-time primitives now
//! live in `coral_core::auth`. The WebUI keeps its tiny_http-shaped
//! `validate_host` / `validate_origin` / `require_bearer` wrappers (so
//! it can surface granular `ApiError` variants for the SPA) but each
//! defers to a pure `check_*_impl` helper that takes header values as
//! `Option<&str>` — making the rules unit-testable without a real
//! `tiny_http::Request` (which has no public constructor).
//! `coral_mcp::transport::http_sse` shares the same `coral_core::auth`
//! helpers, so any change to the constant-time compare lives in one
//! place.

use tiny_http::Request;

use crate::error::ApiError;
use crate::state::AppState;

// Re-export the shared loopback helper at the historical path so any
// downstream caller (tests, the SPA's session JSON, the `serve()`
// pre-flight that bails on non-loopback-without-token) keeps working.
pub use coral_core::auth::is_loopback;

/// Validate the `Host` request header. Accepts `<bind>:<port>`,
/// `127.0.0.1:<port>`, and `localhost:<port>` so the server tolerates
/// browser quirks (modern browsers normalize loopback aliases freely).
///
/// Missing Host is allowed only when bind is loopback; HTTP/1.1
/// technically mandates Host but tiny_http strips it before we see it.
pub fn validate_host(state: &AppState, request: &Request) -> Result<(), ApiError> {
    let host = header_value(request, "host");
    check_host(&state.bind, state.port, host.as_deref())
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
    let auth = header_value(request, "authorization");
    check_bearer(&state.bind, state.token.as_deref(), auth.as_deref())
}

/// Validate the `Origin` header on POST requests. The header is optional
/// (Origin is sent by browsers but not by curl); when present it must
/// match the bound origin or one of its loopback aliases.
pub fn validate_origin(state: &AppState, request: &Request) -> Result<(), ApiError> {
    let origin = header_value(request, "origin");
    let accepted = state.accepted_origins();
    check_origin(state.port, &accepted, origin.as_deref())
}

// --- Pure helpers — same rules, no tiny_http coupling ----------------

/// Pure form of [`validate_host`] — takes the configured bind / port
/// and the `Host` header value (if any), returns the same `ApiError`
/// the request-shaped wrapper does. Used by the unit tests so the
/// rules can be exercised without a real `tiny_http::Request` (the
/// type has no public constructor).
pub(crate) fn check_host(bind: &str, port: u16, host: Option<&str>) -> Result<(), ApiError> {
    let bound_pair = format!("{bind}:{port}");
    let loopback_127 = format!("127.0.0.1:{port}");
    let loopback_name = format!("localhost:{port}");
    match host {
        None => {
            if is_loopback(bind) {
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

/// Pure form of [`require_bearer`] — takes the configured bind /
/// optional token and the `Authorization` header value (if any),
/// returns the same `ApiError` the request-shaped wrapper does.
///
/// `expected_token`:
/// - `None` + loopback bind → no auth required (returns `Ok`).
/// - `None` + non-loopback → `LlmNotConfigured` (`serve()` rejects
///   this at startup, but we keep the runtime guard so any caller
///   that constructs `AppState` directly can't accidentally bypass).
/// - `Some(_)` → header must be `Bearer <token>` and match.
pub(crate) fn check_bearer(
    bind: &str,
    expected_token: Option<&str>,
    header: Option<&str>,
) -> Result<(), ApiError> {
    let needs_token = expected_token.is_some() || !is_loopback(bind);
    if !needs_token {
        return Ok(());
    }
    let Some(expected) = expected_token else {
        return Err(ApiError::LlmNotConfigured(
            "non-loopback bind without --token".into(),
        ));
    };
    match coral_core::auth::verify_bearer(header, expected.as_bytes()) {
        Ok(()) => Ok(()),
        // Pre-fix the WebUI collapsed both "missing header" and
        // "malformed header" into `MissingToken` and kept
        // `InvalidToken` for value mismatch — preserve those wire
        // semantics so the SPA's error-toast strings don't change.
        Err(coral_core::auth::BearerAuthError::MissingHeader)
        | Err(coral_core::auth::BearerAuthError::MalformedHeader) => Err(ApiError::MissingToken),
        Err(coral_core::auth::BearerAuthError::TokenMismatch) => Err(ApiError::InvalidToken),
    }
}

/// Pure form of [`validate_origin`] — takes the configured port, the
/// `AppState::accepted_origins()` precomputed pair, and the `Origin`
/// header value (if any), returns the same `ApiError` the
/// request-shaped wrapper does.
pub(crate) fn check_origin(
    port: u16,
    accepted: &[String],
    origin: Option<&str>,
) -> Result<(), ApiError> {
    let Some(origin) = origin else {
        return Ok(());
    };
    let origin = origin.trim();
    if origin.is_empty() || origin == "null" {
        return Ok(());
    }
    // Loopback aliases that we always accept (both schemes), so a user
    // who binds to `0.0.0.0` but accesses via `localhost` doesn't get
    // a spurious 403. The Host header check (`validate_host`) is the
    // hard fence against DNS-rebinding; Origin just rejects obvious
    // cross-site requests.
    let aliases = [
        format!("http://127.0.0.1:{port}"),
        format!("https://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
        format!("https://localhost:{port}"),
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

// `constant_time_eq` lives in `coral_core::auth` and is exercised by
// that crate's tests. The previous duplicate here was deleted as part
// of v0.35 SEC-01's DRY pass.

#[cfg(test)]
mod tests {
    use super::*;

    // Tests exercise the pure `check_*` helpers — `tiny_http::Request`
    // has no public constructor, so the request-shaped wrappers are
    // tested implicitly via the e2e suite and explicitly here via the
    // pure helpers they delegate to.

    fn accepted(bind: &str, port: u16) -> [String; 2] {
        [
            format!("http://{bind}:{port}"),
            format!("https://{bind}:{port}"),
        ]
    }

    /// TEST-01 #1 — loopback aliases recognized (re-export wires through
    /// to `coral_core::auth::is_loopback`).
    #[test]
    fn loopback_aliases_recognized_via_reexport() {
        assert!(is_loopback("127.0.0.1"));
        assert!(is_loopback("localhost"));
        assert!(is_loopback("::1"));
        assert!(is_loopback("[::1]"));
        assert!(!is_loopback("0.0.0.0"));
        assert!(!is_loopback("192.168.1.1"));
    }

    /// TEST-01 #2 — `check_host` accepts the bound `<bind>:<port>` pair
    /// AND the `127.0.0.1:<port>` / `localhost:<port>` loopback aliases
    /// (case-insensitive).
    #[test]
    fn check_host_accepts_loopback_pair_and_aliases() {
        for h in &["127.0.0.1:3838", "localhost:3838", "LOCALHOST:3838"] {
            assert!(
                check_host("127.0.0.1", 3838, Some(h)).is_ok(),
                "host accepted: {h}"
            );
        }
    }

    /// TEST-01 #3 — `check_host` rejects an arbitrary external host when
    /// bind is loopback (DNS-rebind defense).
    #[test]
    fn check_host_rejects_external_when_bind_loopback() {
        assert!(matches!(
            check_host("127.0.0.1", 3838, Some("attacker.com")),
            Err(ApiError::InvalidHost)
        ));
        assert!(matches!(
            check_host("127.0.0.1", 3838, Some("192.168.1.1:3838")),
            Err(ApiError::InvalidHost)
        ));
    }

    /// TEST-01 #4 — `check_host` allows missing Host header only when
    /// bind is loopback.
    #[test]
    fn check_host_missing_header_loopback_vs_external() {
        // Loopback + no header → ok.
        assert!(check_host("127.0.0.1", 3838, None).is_ok());
        // External bind + no header → reject.
        assert!(matches!(
            check_host("0.0.0.0", 3838, None),
            Err(ApiError::InvalidHost)
        ));
    }

    /// TEST-01 #5 — `check_origin` accepts the loopback alias even when
    /// the bind is `0.0.0.0` (matches the documented alias list — a
    /// user who binds to 0.0.0.0 but accesses via `localhost` must not
    /// get a spurious 403).
    #[test]
    fn check_origin_accepts_loopback_alias_when_bind_external() {
        let accepted = accepted("0.0.0.0", 3838);
        for o in &[
            "http://localhost:3838",
            "http://127.0.0.1:3838",
            "https://localhost:3838",
        ] {
            assert!(
                check_origin(3838, &accepted, Some(o)).is_ok(),
                "origin accepted: {o}"
            );
        }
    }

    /// TEST-01 #6 — `check_origin` rejects an unconfigured external
    /// origin.
    #[test]
    fn check_origin_rejects_attacker_origin() {
        let accepted = accepted("127.0.0.1", 3838);
        assert!(matches!(
            check_origin(3838, &accepted, Some("https://attacker.com")),
            Err(ApiError::InvalidOrigin)
        ));
        assert!(matches!(
            check_origin(3838, &accepted, Some("http://example.com:3838")),
            Err(ApiError::InvalidOrigin)
        ));
    }

    /// TEST-01 #7 — `check_origin` accepts a missing / null / empty
    /// Origin header (curl, file://, native clients don't send Origin).
    #[test]
    fn check_origin_accepts_missing_or_null_or_empty() {
        let accepted = accepted("127.0.0.1", 3838);
        assert!(check_origin(3838, &accepted, None).is_ok());
        assert!(check_origin(3838, &accepted, Some("null")).is_ok());
        assert!(check_origin(3838, &accepted, Some("")).is_ok());
        assert!(check_origin(3838, &accepted, Some("   ")).is_ok());
    }

    /// TEST-01 #8 — `check_bearer` is a no-op when no token is
    /// configured and the bind is loopback (matches the "loopback
    /// without --token is fine" UX path).
    #[test]
    fn check_bearer_no_op_when_loopback_and_no_token() {
        assert!(check_bearer("127.0.0.1", None, None).is_ok());
        // Even a stray Authorization header is ignored if no token is configured.
        assert!(check_bearer("127.0.0.1", None, Some("Bearer whatever")).is_ok());
    }

    /// TEST-01 #9 — `check_bearer` requires a token when bind is
    /// non-loopback even if `expected_token` was wired up as `None`
    /// (defense in depth — `serve()` rejects this at startup).
    #[test]
    fn check_bearer_external_bind_without_token_errors() {
        let err = check_bearer("0.0.0.0", None, Some("Bearer anything")).unwrap_err();
        assert!(matches!(err, ApiError::LlmNotConfigured(_)));
    }

    /// TEST-01 #10 — `check_bearer` rejects a missing Authorization
    /// header when a token IS configured.
    #[test]
    fn check_bearer_rejects_missing_header() {
        assert!(matches!(
            check_bearer("127.0.0.1", Some("expected"), None),
            Err(ApiError::MissingToken)
        ));
        // A malformed header is also collapsed to MissingToken (pre-fix
        // wire contract — the SPA can't distinguish the two cases).
        assert!(matches!(
            check_bearer("127.0.0.1", Some("expected"), Some("Basic abc")),
            Err(ApiError::MissingToken)
        ));
    }

    /// TEST-01 #11 — `check_bearer` rejects a wrong token value via the
    /// constant-time path in `coral_core::auth`.
    #[test]
    fn check_bearer_rejects_mismatched_token() {
        assert!(matches!(
            check_bearer("127.0.0.1", Some("expected"), Some("Bearer wrong")),
            Err(ApiError::InvalidToken)
        ));
        // Length mismatch is also InvalidToken (length is not a secret).
        assert!(matches!(
            check_bearer(
                "127.0.0.1",
                Some("the-real-token-is-much-longer"),
                Some("Bearer short")
            ),
            Err(ApiError::InvalidToken)
        ));
    }

    /// TEST-01 #12 — `check_bearer` accepts the matching token in both
    /// canonical `Bearer` and lowercase `bearer` casings.
    #[test]
    fn check_bearer_accepts_matching_token_both_casings() {
        assert!(check_bearer("127.0.0.1", Some("expected"), Some("Bearer expected")).is_ok());
        assert!(check_bearer("127.0.0.1", Some("expected"), Some("bearer expected")).is_ok());
        // Leading whitespace is tolerated by the underlying extractor.
        assert!(check_bearer("127.0.0.1", Some("expected"), Some("  Bearer expected  ")).is_ok());
    }

    /// TEST-01 #13 — constant_time_eq (shared via coral_core::auth) is
    /// length-independent and stable. Pin the wire behavior here since
    /// the WebUI relies on it for token compares.
    #[test]
    fn constant_time_eq_via_core_is_stable() {
        use coral_core::auth::constant_time_eq;
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"", b"x"));
        assert!(constant_time_eq(b"", b""));
    }
}
