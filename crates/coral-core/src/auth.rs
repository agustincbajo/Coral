//! Shared HTTP-auth primitives for Coral's two HTTP-speaking surfaces
//! (`coral ui serve` and `coral mcp serve --transport http`).
//!
//! v0.35 SEC-01 / CP-3: pre-fix only `coral ui` enforced a bearer token;
//! `coral mcp serve --transport http` had only an Origin allowlist and
//! the 127.0.0.1 default bind. A user who followed the `--bind 0.0.0.0`
//! warning would expose an unauthenticated tool-execution endpoint on
//! the LAN. The two surfaces had near-identical `validate_host` /
//! `require_bearer` / `constant_time_eq` logic; extracting it here
//! makes the MCP HTTP transport reuse the same checks the WebUI relies
//! on (and gives both crates one set of tests to update when the rules
//! evolve).
//!
//! The module is transport-agnostic: it doesn't depend on `tiny_http`
//! or any specific request type, so the two callers can adapt their
//! own header iteration without forcing a shared HTTP type. Callers
//! pass byte slices for `expected` / `provided` tokens and string
//! slices for header values.

/// Loopback-ish bind addresses that are safe to serve without a token.
/// Matches the union of names IPv4 / IPv6 stacks recognize as the
/// local machine (the `[::1]` and bracketed forms cover what HTTP
/// `Host` headers actually carry for IPv6 loopback).
pub fn is_loopback(bind: &str) -> bool {
    matches!(bind, "127.0.0.1" | "localhost" | "::1" | "[::1]")
}

/// Constant-time byte-slice equality. Avoids early-exit timing leaks
/// when comparing user-supplied tokens against the configured one —
/// the loop visits every byte even when the first differs, so an
/// attacker can't learn the matched prefix from response latency.
///
/// Returns `false` immediately on length mismatch (the length itself
/// is not a secret in our threat model — tokens are minted as
/// fixed-width 256-bit hex strings, so all valid tokens share a
/// length).
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Parse a `Bearer <token>` header value, accepting both canonical
/// `Bearer` and lowercase `bearer` casings. Returns the token slice
/// when the prefix matches, `None` otherwise.
///
/// This is split out from `verify_bearer` so callers that want to
/// 401-with-a-custom-error on bad shape can do so without duplicating
/// the prefix-strip logic.
pub fn extract_bearer_token(header_value: &str) -> Option<&str> {
    let trimmed = header_value.trim();
    trimmed
        .strip_prefix("Bearer ")
        .or_else(|| trimmed.strip_prefix("bearer "))
}

/// Verify a presented `Authorization: Bearer <token>` header against
/// the expected token in constant time.
///
/// Returns:
/// - `Ok(())` when the prefix is correct and the token matches.
/// - `Err(BearerAuthError::MissingHeader)` when `header_value` is
///   `None` (no Authorization header sent).
/// - `Err(BearerAuthError::MalformedHeader)` when the header is
///   present but doesn't start with `Bearer ` / `bearer `.
/// - `Err(BearerAuthError::TokenMismatch)` when the prefix is correct
///   but the token doesn't match the expected value.
///
/// The three error variants let the caller pick how granular the 401
/// response should be — the MCP transport collapses everything into
/// a single `401 Unauthorized` (clients don't need to know whether
/// they sent the wrong shape or the wrong value), while the WebUI
/// surfaces distinct `ApiError` variants for the same conditions so
/// the SPA can show a tailored error toast.
pub fn verify_bearer(
    header_value: Option<&str>,
    expected_token: &[u8],
) -> Result<(), BearerAuthError> {
    let header_value = header_value.ok_or(BearerAuthError::MissingHeader)?;
    let token = extract_bearer_token(header_value).ok_or(BearerAuthError::MalformedHeader)?;
    if constant_time_eq(token.as_bytes(), expected_token) {
        Ok(())
    } else {
        Err(BearerAuthError::TokenMismatch)
    }
}

/// Failure modes for [`verify_bearer`]. The three variants let callers
/// surface different error messages without re-implementing the check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BearerAuthError {
    /// No `Authorization` header was sent.
    MissingHeader,
    /// Header was present but didn't start with `Bearer ` / `bearer `.
    MalformedHeader,
    /// Header was well-formed but the token didn't match.
    TokenMismatch,
}

impl BearerAuthError {
    /// Human-readable label suitable for inclusion in a 401 response
    /// body. Stable strings — the MCP transport tests pin them.
    pub fn label(self) -> &'static str {
        match self {
            BearerAuthError::MissingHeader => "missing Authorization header",
            BearerAuthError::MalformedHeader => "Authorization header must start with 'Bearer '",
            BearerAuthError::TokenMismatch => "invalid bearer token",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_aliases_recognized() {
        assert!(is_loopback("127.0.0.1"));
        assert!(is_loopback("localhost"));
        assert!(is_loopback("::1"));
        assert!(is_loopback("[::1]"));
        assert!(!is_loopback("0.0.0.0"));
        assert!(!is_loopback("192.168.1.1"));
        assert!(!is_loopback(""));
    }

    #[test]
    fn constant_time_eq_basic_equality() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"", b"x"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn extract_bearer_accepts_both_casings() {
        assert_eq!(extract_bearer_token("Bearer xyz"), Some("xyz"));
        assert_eq!(extract_bearer_token("bearer xyz"), Some("xyz"));
        assert_eq!(extract_bearer_token("  Bearer xyz  "), Some("xyz"));
        // No prefix → None
        assert_eq!(extract_bearer_token("xyz"), None);
        // Wrong prefix → None
        assert_eq!(extract_bearer_token("Basic xyz"), None);
        // Empty → None
        assert_eq!(extract_bearer_token(""), None);
    }

    #[test]
    fn verify_bearer_happy_path() {
        assert_eq!(verify_bearer(Some("Bearer secret"), b"secret"), Ok(()));
        assert_eq!(verify_bearer(Some("bearer secret"), b"secret"), Ok(()));
    }

    #[test]
    fn verify_bearer_missing_header() {
        assert_eq!(
            verify_bearer(None, b"secret"),
            Err(BearerAuthError::MissingHeader)
        );
    }

    #[test]
    fn verify_bearer_malformed_header() {
        assert_eq!(
            verify_bearer(Some("Basic abc"), b"secret"),
            Err(BearerAuthError::MalformedHeader)
        );
        assert_eq!(
            verify_bearer(Some("nothing"), b"secret"),
            Err(BearerAuthError::MalformedHeader)
        );
    }

    #[test]
    fn verify_bearer_token_mismatch() {
        assert_eq!(
            verify_bearer(Some("Bearer wrong"), b"secret"),
            Err(BearerAuthError::TokenMismatch)
        );
        // Length-mismatch is still a TokenMismatch — exposing
        // "wrong length" vs "wrong bytes" would leak the secret's
        // length, which is fine for our threat model but no reason
        // to distinguish.
        assert_eq!(
            verify_bearer(Some("Bearer short"), b"a-much-longer-secret"),
            Err(BearerAuthError::TokenMismatch)
        );
    }

    #[test]
    fn bearer_auth_error_labels_are_stable() {
        // These strings are part of the wire contract for the MCP
        // 401 body — the e2e tests pin them. Pin here too so a
        // rename in this enum has to come through code review.
        assert_eq!(
            BearerAuthError::MissingHeader.label(),
            "missing Authorization header"
        );
        assert_eq!(
            BearerAuthError::MalformedHeader.label(),
            "Authorization header must start with 'Bearer '"
        );
        assert_eq!(
            BearerAuthError::TokenMismatch.label(),
            "invalid bearer token"
        );
    }
}
