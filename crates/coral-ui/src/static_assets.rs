//! Embedded SPA static-asset serving.
//!
//! The frontend is built into `crates/coral-ui/assets/dist/` and baked
//! into the binary via `include_dir!`. At request time we look up the
//! file, set a reasonable cache header (long-immutable for hashed
//! filenames, no-cache for `index.html`), and inject the runtime config
//! JSON into `index.html` via a `<!-- __CORAL_RUNTIME_CONFIG__ -->`
//! placeholder so the SPA can discover the API base URL without an
//! extra round-trip.

use include_dir::{Dir, include_dir};

pub static ASSETS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/assets/dist");

/// In-memory representation of a static-file response, ready to be
/// wrapped in a `tiny_http::Response`. We use this intermediate struct
/// so routes can return one shape and the server layer handles the
/// conversion.
pub struct StaticResponse {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
    pub cache: &'static str,
}

/// Resolve a path to a static asset.
///
/// `path` is the request URL path (e.g. `"/"`, `"/index.html"`,
/// `"/assets/foo.js"`). Returns `None` if no file matches; the server
/// falls back to serving `index.html` for SPA deep-links in that case.
///
/// `runtime_config_json` is injected into `index.html` via the
/// `<!-- __CORAL_RUNTIME_CONFIG__ -->` placeholder. It should be a
/// pre-stringified JSON object — the helper wraps it in a `<script>`
/// tag that assigns to `window.__CORAL_CONFIG__`.
pub fn serve_static(path: &str, runtime_config_json: &str) -> Option<StaticResponse> {
    let normalized = if path == "/" || path.is_empty() {
        "index.html"
    } else {
        path.trim_start_matches('/')
    };

    let file = ASSETS.get_file(normalized)?;
    let mime = mime_guess::from_path(normalized).first_or_octet_stream();
    let mut body = file.contents().to_vec();

    if normalized == "index.html" {
        let html = String::from_utf8_lossy(&body);
        let injected = html.replace(
            "<!-- __CORAL_RUNTIME_CONFIG__ -->",
            &format!(
                r#"<script>window.__CORAL_CONFIG__={};</script>"#,
                runtime_config_json
            ),
        );
        body = injected.into_bytes();
    }

    let cache = if normalized == "index.html" {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };

    Some(StaticResponse {
        status: 200,
        content_type: mime.to_string(),
        body,
        cache,
    })
}

/// SPA fallback: return `index.html` (with runtime config injected).
/// Used by the router when a non-API path doesn't match any file —
/// client-side routes like `/pages/foo` should resolve to the SPA.
pub fn serve_index_fallback(runtime_config_json: &str) -> Option<StaticResponse> {
    serve_static("/", runtime_config_json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_returns_index() {
        let r = serve_static("/", r#"{"foo":"bar"}"#).expect("placeholder index exists");
        assert_eq!(r.status, 200);
        assert_eq!(r.cache, "no-cache");
        let body = String::from_utf8(r.body).expect("utf8");
        assert!(body.contains(r#"window.__CORAL_CONFIG__={"foo":"bar"};"#));
        // Placeholder must be replaced.
        assert!(!body.contains("__CORAL_RUNTIME_CONFIG__"));
    }

    #[test]
    fn unknown_path_returns_none() {
        assert!(serve_static("/no-such-file.js", "{}").is_none());
    }

    #[test]
    fn index_fallback_returns_index() {
        let r = serve_index_fallback("{}").expect("fallback works");
        assert_eq!(r.status, 200);
        let body = String::from_utf8(r.body).expect("utf8");
        assert!(body.contains("<div id=\"root\">"));
    }
}
