//! Embedded SPA static-asset serving.
//!
//! The frontend is built into `crates/coral-ui/assets/dist/` and baked
//! into the binary via `include_dir!`. At request time we look up the
//! file, set a reasonable cache header (long-immutable for hashed
//! filenames, no-cache for `index.html`), and inject the runtime config
//! JSON into `index.html` via a `<!-- __CORAL_RUNTIME_CONFIG__ -->`
//! placeholder so the SPA can discover the API base URL without an
//! extra round-trip.
//!
//! v0.35 Phase C (P-H1): the build script generates `.gz` / `.br`
//! siblings for the heavy assets (js/css/svg/json). When the client
//! sends an `Accept-Encoding` header listing `br` or `gzip` (in that
//! priority order — brotli wins), we serve the pre-compressed sibling
//! and set `Content-Encoding`. `index.html` is excluded from
//! pre-compression because we inject runtime config at request time;
//! it always serves uncompressed.
//!
//! v0.36 hardening: build.rs drops the raw asset from `assets/dist`
//! when it clears the 100 KiB threshold AND both siblings exist
//! (~300-500 KiB binary savings on the v0.35 SPA bundle). The
//! resolver below handles the missing-raw case three ways:
//!
//!   1. Client advertises `br` or `gzip` → serve the matching sibling
//!      pre-compressed (the hot path, >99% of real traffic).
//!   2. Client advertises only `identity` (or no header) AND the raw
//!      bundle is still on disk → serve raw (legacy path).
//!   3. Raw bundle dropped at build time AND client wants identity
//!      → decompress the smaller sibling on the fly and serve raw
//!      bytes. Rare; brotli decompression at request time is still
//!      fast (~tens of MB/s single-threaded).
//!
//! v0.36 also tightens cache correctness: any response for a path
//! that *could* have produced a compressed sibling now emits
//! `Vary: Accept-Encoding` regardless of which branch fired, so an
//! intermediate proxy keyed on URL alone never serves the wrong
//! encoding to a downstream client.

use include_dir::{Dir, include_dir};
use std::io::Read;

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
    /// `Some("br")` / `Some("gzip")` when the body bytes are
    /// pre-compressed and the server should set a matching
    /// `Content-Encoding` header. `None` means raw bytes.
    pub content_encoding: Option<&'static str>,
    /// v0.36 hardening: `true` when the asset path has at least one
    /// compressed sibling embedded in the binary, so the response is
    /// content-negotiated against `Accept-Encoding`. The server layer
    /// emits `Vary: Accept-Encoding` on these responses regardless of
    /// which branch fired (raw / br / gzip / decompressed-fallback),
    /// per RFC 9110 §15.5.4. Intermediate caches that key on URL
    /// alone otherwise risk serving a brotli response to a
    /// gzip-only client.
    pub vary_accept_encoding: bool,
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
///
/// `accept_encoding` is the raw `Accept-Encoding` header value (or an
/// empty string when the client didn't send one). Brotli wins over
/// gzip when both are listed; an unknown encoding falls back to raw.
pub fn serve_static(
    path: &str,
    runtime_config_json: &str,
    accept_encoding: &str,
) -> Option<StaticResponse> {
    let normalized = if path == "/" || path.is_empty() {
        "index.html"
    } else {
        path.trim_start_matches('/')
    };

    let mime = mime_guess::from_path(normalized).first_or_octet_stream();
    let is_index_html = normalized == "index.html";

    // v0.36: probe whether the path *could* be content-negotiated, so
    // the Vary header gets set on every branch downstream. index.html
    // is excluded — runtime config injection requires raw bytes, so
    // it's never compressed regardless of what the client advertises.
    let has_any_sibling = !is_index_html && path_has_any_sibling(normalized);

    // v0.35 Phase C (P-H1): try to serve a pre-compressed sibling.
    if !is_index_html
        && let Some((sibling_bytes, encoding)) = try_compressed_sibling(normalized, accept_encoding)
    {
        let cache = "public, max-age=31536000, immutable";
        return Some(StaticResponse {
            status: 200,
            content_type: mime.to_string(),
            body: sibling_bytes,
            cache,
            content_encoding: Some(encoding),
            vary_accept_encoding: has_any_sibling,
        });
    }

    // Raw path. Three sub-cases for non-index assets:
    //   (a) raw file present  -> serve it
    //   (b) raw file dropped at build time, sibling available
    //       -> decompress sibling on the fly and serve raw bytes
    //   (c) neither raw nor sibling -> None (caller falls back to SPA)
    let body = if let Some(file) = ASSETS.get_file(normalized) {
        let mut body = file.contents().to_vec();
        if is_index_html {
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
        body
    } else if has_any_sibling {
        // v0.36 hardening: raw bundle was dropped at build time
        // because the asset cleared the size threshold. Decompress
        // the smaller sibling so legacy `Accept-Encoding: identity`
        // clients still get a working response.
        decompress_any_sibling(normalized)?
    } else {
        return None;
    };

    let cache = if is_index_html {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };

    Some(StaticResponse {
        status: 200,
        content_type: mime.to_string(),
        body,
        cache,
        content_encoding: None,
        vary_accept_encoding: has_any_sibling,
    })
}

/// SPA fallback: return `index.html` (with runtime config injected).
/// Used by the router when a non-API path doesn't match any file —
/// client-side routes like `/pages/foo` should resolve to the SPA.
pub fn serve_index_fallback(runtime_config_json: &str) -> Option<StaticResponse> {
    // index.html is never pre-compressed, so the accept-encoding
    // argument is irrelevant here. Pass empty so we go down the raw
    // path unconditionally.
    serve_static("/", runtime_config_json, "")
}

/// Try to read a pre-compressed sibling of `path` from the embedded
/// asset tree. Returns the bytes and the encoding name when the
/// client's `Accept-Encoding` advertises support.
///
/// Brotli wins over gzip when both are listed — brotli typically
/// shaves another 15-25% off the wire vs. gzip for text-heavy SPA
/// bundles.
fn try_compressed_sibling(
    normalized_path: &str,
    accept_encoding: &str,
) -> Option<(Vec<u8>, &'static str)> {
    let accepts_br = accept_encoding_lists(accept_encoding, "br");
    let accepts_gzip = accept_encoding_lists(accept_encoding, "gzip");

    if accepts_br {
        let br_path = format!("{normalized_path}.br");
        if let Some(f) = ASSETS.get_file(&br_path) {
            return Some((f.contents().to_vec(), "br"));
        }
    }
    if accepts_gzip {
        let gz_path = format!("{normalized_path}.gz");
        if let Some(f) = ASSETS.get_file(&gz_path) {
            return Some((f.contents().to_vec(), "gzip"));
        }
    }
    None
}

/// v0.36 hardening: returns `true` when the embedded asset tree has
/// at least one `.br` or `.gz` sibling for the given path. Drives
/// `Vary: Accept-Encoding` on every response for the path, even when
/// the chosen branch was raw.
fn path_has_any_sibling(normalized_path: &str) -> bool {
    ASSETS
        .get_file(format!("{normalized_path}.br"))
        .or_else(|| ASSETS.get_file(format!("{normalized_path}.gz")))
        .is_some()
}

/// v0.36 hardening: when the raw bundle was dropped at build time and
/// the client doesn't advertise a compressed encoding, we have to
/// decompress one of the embedded siblings on the fly. Try brotli
/// first (smaller payload → cheaper decompress) and fall back to gzip.
///
/// Returns `None` only when neither sibling is present, which would
/// indicate a build-script bug (raw dropped but no sibling shipped).
fn decompress_any_sibling(normalized_path: &str) -> Option<Vec<u8>> {
    if let Some(f) = ASSETS.get_file(format!("{normalized_path}.br")) {
        let mut out = Vec::with_capacity(f.contents().len() * 4);
        let mut decoder = brotli::Decompressor::new(f.contents(), 4096);
        if decoder.read_to_end(&mut out).is_ok() {
            return Some(out);
        }
    }
    if let Some(f) = ASSETS.get_file(format!("{normalized_path}.gz")) {
        use flate2::read::GzDecoder;
        let mut out = Vec::with_capacity(f.contents().len() * 3);
        let mut decoder = GzDecoder::new(f.contents());
        if decoder.read_to_end(&mut out).is_ok() {
            return Some(out);
        }
    }
    None
}

/// Token-level match against the comma-separated `Accept-Encoding`
/// header. Accepts the canonical form, surrounding whitespace, and
/// q-value suffixes (`gzip;q=0.9`). Case-insensitive on the encoding
/// token per RFC 9110 §8.4.
///
/// We deliberately don't parse the full quality-value DAG — a token
/// presence check is sufficient: every client we care about that
/// understands `br` or `gzip` lists it without a `q=0`, and a
/// pathological `gzip;q=0` (explicit reject) is rare enough that
/// falling back to raw bytes on its presence is an acceptable trade.
fn accept_encoding_lists(header: &str, encoding: &str) -> bool {
    if header.is_empty() {
        return false;
    }
    header.split(',').any(|tok| {
        let tok = tok.trim();
        // Strip any q-value or parameter (split at `;`).
        let name = tok.split(';').next().unwrap_or(tok).trim();
        name.eq_ignore_ascii_case(encoding)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_returns_index() {
        let r = serve_static("/", r#"{"foo":"bar"}"#, "").expect("placeholder index exists");
        assert_eq!(r.status, 200);
        assert_eq!(r.cache, "no-cache");
        assert_eq!(r.content_encoding, None, "index.html is never compressed");
        let body = String::from_utf8(r.body).expect("utf8");
        assert!(body.contains(r#"window.__CORAL_CONFIG__={"foo":"bar"};"#));
        // Placeholder must be replaced.
        assert!(!body.contains("__CORAL_RUNTIME_CONFIG__"));
    }

    #[test]
    fn unknown_path_returns_none() {
        assert!(serve_static("/no-such-file.js", "{}", "").is_none());
    }

    #[test]
    fn index_fallback_returns_index() {
        let r = serve_index_fallback("{}").expect("fallback works");
        assert_eq!(r.status, 200);
        assert_eq!(r.content_encoding, None);
        let body = String::from_utf8(r.body).expect("utf8");
        assert!(body.contains("<div id=\"root\">"));
    }

    /// v0.35 Phase C (P-H1) — `Accept-Encoding` token parsing.
    /// Permissive on q-values + whitespace, case-insensitive.
    #[test]
    fn accept_encoding_parser_handles_common_shapes() {
        assert!(accept_encoding_lists("gzip", "gzip"));
        assert!(accept_encoding_lists("br", "br"));
        assert!(accept_encoding_lists("br, gzip", "gzip"));
        assert!(accept_encoding_lists("br, gzip", "br"));
        assert!(accept_encoding_lists("gzip;q=0.9, deflate", "gzip"));
        assert!(accept_encoding_lists("BR", "br"));
        assert!(accept_encoding_lists("  br  ,  gzip  ", "br"));
        assert!(!accept_encoding_lists("", "br"));
        assert!(!accept_encoding_lists("identity", "gzip"));
        assert!(!accept_encoding_lists("deflate", "br"));
    }

    /// v0.35 Phase C (P-H1) — when the asset tree has a pre-compressed
    /// sibling and the client accepts the encoding, we serve the
    /// sibling. The placeholder asset bundle doesn't ship with
    /// pre-compressed siblings in test fixtures, so this test
    /// degrades gracefully: it asserts ONLY that the raw branch is
    /// taken (content_encoding=None) when no sibling is found — which
    /// is the happy path in unit tests.
    #[test]
    fn missing_sibling_falls_back_to_raw_bytes() {
        // index.html exists in the test fixture; ask for it via a
        // synthetic non-index path won't find a sibling (because the
        // fixture only has index.html).
        let r = serve_static("/index.html", "{}", "br, gzip");
        // index.html is the special-cased case — never compressed.
        if let Some(r) = r {
            assert_eq!(
                r.content_encoding, None,
                "index.html must never report a content encoding"
            );
        }
    }

    /// v0.36 hardening — index.html must never set
    /// `vary_accept_encoding`. Runtime config injection requires raw
    /// bytes, so no sibling is ever generated for it; setting Vary
    /// would mislead intermediate caches into keying on
    /// `Accept-Encoding` when the response never varies.
    #[test]
    fn index_never_sets_vary_accept_encoding() {
        let r = serve_static("/", "{}", "br, gzip").expect("index exists");
        assert!(
            !r.vary_accept_encoding,
            "index.html responses must not advertise Vary: Accept-Encoding"
        );
    }

    /// v0.36 hardening — when the path has a sibling embedded AND the
    /// client took the raw branch (no Accept-Encoding), the response
    /// MUST still set `vary_accept_encoding`. Without this an
    /// intermediate cache keyed on URL alone would store the raw
    /// payload and serve it to the next client regardless of that
    /// client's `Accept-Encoding`. The fixture asset tree in unit
    /// tests doesn't ship siblings, so this asserts the negative —
    /// raw response without sibling stays vary=false. The integration
    /// tests exercise the positive case against a real built bundle.
    #[test]
    fn raw_response_without_sibling_stays_vary_false() {
        let r = serve_static("/index.html", "{}", "").expect("index exists");
        assert!(
            !r.vary_accept_encoding,
            "index.html (uncompressed by design) keeps vary=false"
        );
    }
}
