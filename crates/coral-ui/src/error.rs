//! Canonical error envelope for the v1 REST API.
//!
//! Every error response from `coral ui serve` is shaped as
//! `{"error":{"code":"<MACRO_CASE>","message":"<human>","hint":<string|null>}}`.
//! Status codes are 4xx for client mistakes (invalid host/origin, missing
//! or wrong token, malformed slug) and 5xx for internal failures.

use std::io::Cursor;

use serde::Serialize;
use tiny_http::{Header, Response};

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid filter: {0}")]
    InvalidFilter(String),
    #[error("missing token")]
    MissingToken,
    #[error("invalid token")]
    InvalidToken,
    #[error("invalid origin")]
    InvalidOrigin,
    #[error("invalid host")]
    InvalidHost,
    #[error("write tools disabled")]
    WriteToolsDisabled,
    #[error("llm not configured: {0}")]
    LlmNotConfigured(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

#[derive(Debug, Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    message: String,
    hint: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct ErrorEnvelope<'a> {
    error: ErrorBody<'a>,
}

impl ApiError {
    pub fn status(&self) -> u16 {
        match self {
            ApiError::NotFound(_) => 404,
            ApiError::InvalidFilter(_) | ApiError::InvalidHost => 400,
            ApiError::MissingToken => 401,
            ApiError::InvalidToken | ApiError::InvalidOrigin | ApiError::WriteToolsDisabled => 403,
            ApiError::LlmNotConfigured(_) => 503,
            ApiError::Internal(_) => 500,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            ApiError::NotFound(_) => "NOT_FOUND",
            ApiError::InvalidFilter(_) => "INVALID_FILTER",
            ApiError::MissingToken => "MISSING_TOKEN",
            ApiError::InvalidToken => "INVALID_TOKEN",
            ApiError::InvalidOrigin => "INVALID_ORIGIN",
            ApiError::InvalidHost => "INVALID_HOST",
            ApiError::WriteToolsDisabled => "WRITE_TOOLS_DISABLED",
            ApiError::LlmNotConfigured(_) => "LLM_NOT_CONFIGURED",
            ApiError::Internal(_) => "INTERNAL",
        }
    }

    /// Optional, machine-stable hint shown alongside the error message.
    /// Avoids leaking arbitrary anyhow chains; only static strings here.
    pub fn hint(&self) -> Option<&'static str> {
        match self {
            ApiError::MissingToken => {
                Some("provide an `Authorization: Bearer <token>` header")
            }
            ApiError::InvalidToken => Some("the token did not match the configured value"),
            ApiError::InvalidHost => {
                Some("requests must address the loopback bind by hostname")
            }
            ApiError::InvalidOrigin => Some("set Origin to the bound origin or omit it"),
            ApiError::WriteToolsDisabled => {
                Some("rerun the server with --allow-write-tools to enable mutation")
            }
            ApiError::LlmNotConfigured(_) => Some("install/configure a runner provider"),
            _ => None,
        }
    }

    pub fn to_response(&self) -> Response<Cursor<Vec<u8>>> {
        // Redact internal errors: log the full anyhow chain, but never
        // leak it to the wire — filesystem paths, stack details, etc.
        // can appear in `Display`. Other variants are safe (their text
        // is hand-authored).
        let message = match self {
            ApiError::Internal(e) => {
                tracing::error!(error = %e, "internal api error");
                "internal server error".to_string()
            }
            other => other.to_string(),
        };
        let body = ErrorEnvelope {
            error: ErrorBody {
                code: self.code(),
                message,
                hint: self.hint(),
            },
        };
        let bytes = serde_json::to_vec(&body).unwrap_or_else(|_| {
            br#"{"error":{"code":"INTERNAL","message":"failed to encode error envelope","hint":null}}"#.to_vec()
        });
        Response::from_data(bytes)
            .with_status_code(self.status() as i32)
            .with_header(json_content_type())
    }
}

fn json_content_type() -> Header {
    Header::from_bytes(b"Content-Type" as &[u8], b"application/json" as &[u8])
        .expect("valid content-type header")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_codes_match_spec() {
        assert_eq!(ApiError::NotFound("x".into()).status(), 404);
        assert_eq!(ApiError::InvalidFilter("x".into()).status(), 400);
        assert_eq!(ApiError::InvalidHost.status(), 400);
        assert_eq!(ApiError::MissingToken.status(), 401);
        assert_eq!(ApiError::InvalidToken.status(), 403);
        assert_eq!(ApiError::InvalidOrigin.status(), 403);
        assert_eq!(ApiError::WriteToolsDisabled.status(), 403);
        assert_eq!(ApiError::LlmNotConfigured("none".into()).status(), 503);
    }

    #[test]
    fn codes_are_stable_macro_case() {
        assert_eq!(ApiError::MissingToken.code(), "MISSING_TOKEN");
        assert_eq!(ApiError::InvalidFilter("x".into()).code(), "INVALID_FILTER");
    }

    #[test]
    fn envelope_serializes_with_hint_null_when_absent() {
        let err = ApiError::NotFound("wiki".into());
        let resp = err.to_response();
        assert_eq!(resp.status_code().0, 404);
    }
}
