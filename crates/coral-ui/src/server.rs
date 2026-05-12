//! tiny_http server entrypoint + dispatch loop.
//!
//! `serve(config)` is the public API: it validates the security
//! preconditions (non-loopback bind requires a token), boots the HTTP
//! server, installs SIGINT/SIGTERM handlers via signal-hook (same
//! pattern as `coral wiki serve`), and runs the recv loop with a 250ms
//! `recv_timeout` so shutdown latency is bounded.
//!
//! Routing is intentionally inlined into `handle` (rather than dispatched
//! through a `routes::dispatch` helper) so we keep ownership of the
//! `Request` for as long as we might need to respond on it. The
//! streaming `/api/v1/query` route takes ownership and writes raw bytes
//! itself; every other route returns a `(status, body)` pair that we
//! wrap in `Response::from_data`.

use std::io::Read;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use coral_runner::{ClaudeRunner, Runner};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::error::ApiError;
use crate::routes;
use crate::state::AppState;
pub use crate::state::ServeConfig;
use crate::static_assets;

/// Boot the WebUI server. Blocks until SIGINT/SIGTERM is received.
pub fn serve(config: ServeConfig) -> Result<()> {
    // Security precondition: non-loopback bind requires a token.
    if !crate::auth::is_loopback(&config.bind) && config.token.is_none() {
        anyhow::bail!(
            "refusing to bind to non-loopback address {:?} without --token (or CORAL_UI_TOKEN env var); \
             see `coral ui serve --help`",
            config.bind
        );
    }

    if !config.wiki_root.exists() {
        anyhow::bail!(
            "wiki directory '{}' does not exist; run `coral init` first",
            config.wiki_root.display()
        );
    }

    // Build the default runner *only if* the `claude` binary is
    // resolvable. We don't fail startup if it isn't — read-only routes
    // stay usable on a system without Claude installed, and the
    // `/api/v1/query` handler explicitly returns
    // `LLM_NOT_CONFIGURED` when `state.runner` is `None`.
    //
    // PATH lookup is a cheap one-off `which`-style scan; we cache
    // the result in `state.runner`, so subsequent /query calls don't
    // re-probe the filesystem.
    let runner: Option<Arc<dyn Runner>> = if claude_binary_present() {
        Some(Arc::new(ClaudeRunner::new()) as Arc<dyn Runner>)
    } else {
        tracing::warn!(
            "coral ui serve: `claude` binary not found in PATH — /api/v1/query will return LLM_NOT_CONFIGURED"
        );
        None
    };

    let state = Arc::new(AppState {
        bind: config.bind.clone(),
        port: config.port,
        wiki_root: config.wiki_root.clone(),
        token: config.token.clone(),
        allow_write_tools: config.allow_write_tools,
        runner,
    });

    let addr: SocketAddr = format!("{}:{}", config.bind, config.port)
        .parse()
        .with_context(|| format!("invalid bind address: {}:{}", config.bind, config.port))?;

    let server = Server::http(addr)
        .map_err(|e| anyhow::anyhow!("failed to start HTTP server on {}: {}", addr, e))?;

    tracing::info!(addr = %addr, wiki = %config.wiki_root.display(), "coral ui serve: listening");
    eprintln!("coral ui serve: listening on http://{}", addr);

    if config.open_browser {
        let url = format!("http://{}:{}", config.bind, config.port);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            if let Err(e) = open_browser(&url) {
                tracing::warn!(error = %e, "failed to open browser; continue with manual nav");
            }
        });
    }

    // Shutdown plumbing — identical pattern to `coral wiki serve`.
    let shutdown = Arc::new(AtomicBool::new(false));
    install_shutdown_handler(shutdown.clone())?;

    loop {
        if shutdown.load(Ordering::Relaxed) {
            eprintln!("\ncoral ui serve: shutting down...");
            break;
        }
        match server.recv_timeout(Duration::from_millis(250)) {
            Ok(Some(req)) => handle(&state, req),
            Ok(None) => continue,
            Err(_) => continue,
        }
    }
    Ok(())
}

fn handle(state: &Arc<AppState>, mut request: Request) {
    let url = request.url().to_string();
    let path = url.split('?').next().unwrap_or(&url).to_string();
    let is_api = path.starts_with("/api/") || path == "/health";

    if !is_api {
        let cfg = runtime_config_json(state);
        let resp = static_assets::serve_static(&path, &cfg)
            .or_else(|| static_assets::serve_index_fallback(&cfg));
        if let Some(r) = resp {
            respond_static(request, r);
        } else {
            let err = ApiError::NotFound(format!("no asset for {path}"));
            let _ = request.respond(err.to_response());
        }
        return;
    }

    // API path.
    let method = request.method().clone();

    if let Err(e) = crate::auth::validate_host(state, &request) {
        let _ = request.respond(e.to_response());
        return;
    }
    if matches!(method, Method::Post) {
        if let Err(e) = crate::auth::validate_origin(state, &request) {
            let _ = request.respond(e.to_response());
            return;
        }
    }
    let token_required =
        path == "/api/v1/query" || path.starts_with("/api/v1/tools/") || state.token.is_some();
    if token_required {
        if let Err(e) = crate::auth::require_bearer(state, &request) {
            let _ = request.respond(e.to_response());
            return;
        }
    }

    let query_string = url
        .split_once('?')
        .map(|(_, q)| q.to_string())
        .unwrap_or_default();

    // Streaming routes consume the request.
    if matches!(method, Method::Post) && path == "/api/v1/query" {
        match routes::query::handle_streaming(state, request) {
            Ok(()) => {}
            Err(e) => {
                tracing::warn!(error = %e, "query streaming failed; head not yet sent");
                // We may or may not still own the wire here — depends
                // on where the error happened. In any case, the
                // request has been moved; nothing more to do.
            }
        }
        return;
    }
    if matches!(method, Method::Get) && path == "/api/v1/events" {
        match routes::events::handle_streaming(state, request) {
            Ok(()) => {}
            Err(e) => {
                tracing::warn!(error = %e, "events streaming failed; head not yet sent");
            }
        }
        return;
    }

    // POST routes that read a JSON body — drain it once here, then
    // dispatch to the handler with the buffered bytes. The tools
    // gate (`state.allow_write_tools`) is enforced inside each
    // handler so the 403 envelope shape matches the rest of the API.
    let body: Result<Vec<u8>, ApiError> = if matches!(method, Method::Post) {
        read_body(&mut request)
    } else {
        Ok(Vec::new())
    };

    let result: Result<(u16, Vec<u8>), ApiError> = match (&method, path.as_str()) {
        (Method::Get, "/health") => routes::health::handle(state).map(|b| (200, b)),
        (Method::Get, "/api/v1/pages") => {
            routes::pages::list(state, &query_string).map(|b| (200, b))
        }
        (Method::Get, p) if p.starts_with("/api/v1/pages/") => {
            let rest = p.trim_start_matches("/api/v1/pages/");
            routes::pages::single(state, rest).map(|b| (200, b))
        }
        (Method::Get, "/api/v1/search") => {
            routes::search::handle(state, &query_string).map(|b| (200, b))
        }
        (Method::Get, "/api/v1/graph") => {
            routes::graph::handle(state, &query_string).map(|b| (200, b))
        }
        (Method::Get, "/api/v1/manifest") => routes::manifest::manifest(state).map(|b| (200, b)),
        (Method::Get, "/api/v1/lock") => routes::manifest::lock(state).map(|b| (200, b)),
        (Method::Get, "/api/v1/stats") => routes::manifest::stats(state).map(|b| (200, b)),
        (Method::Get, "/api/v1/interfaces") => routes::interfaces::handle(state).map(|b| (200, b)),
        (Method::Get, "/api/v1/contract_status") => {
            routes::contracts::handle(state).map(|b| (200, b))
        }
        (Method::Get, "/api/v1/affected") => {
            routes::affected::handle(state, &query_string).map(|b| (200, b))
        }
        (Method::Get, "/api/v1/guarantee") => {
            routes::guarantee::handle(state, &query_string).map(|b| (200, b))
        }
        (Method::Post, "/api/v1/tools/verify") => body
            .as_ref()
            .map_err(|e| ApiError::InvalidFilter(format!("body read failed: {e}")))
            .and_then(|b| routes::tools::handle_verify(state, b))
            .map(|b| (200, b)),
        (Method::Post, "/api/v1/tools/run_test") => body
            .as_ref()
            .map_err(|e| ApiError::InvalidFilter(format!("body read failed: {e}")))
            .and_then(|b| routes::tools::handle_run_test(state, b))
            .map(|b| (200, b)),
        (Method::Post, "/api/v1/tools/up") => body
            .as_ref()
            .map_err(|e| ApiError::InvalidFilter(format!("body read failed: {e}")))
            .and_then(|b| routes::tools::handle_up(state, b))
            .map(|b| (200, b)),
        (Method::Post, "/api/v1/tools/down") => body
            .as_ref()
            .map_err(|e| ApiError::InvalidFilter(format!("body read failed: {e}")))
            .and_then(|b| routes::tools::handle_down(state, b))
            .map(|b| (200, b)),
        _ => Err(ApiError::NotFound(format!("{} {}", method.as_str(), path))),
    };

    match result {
        Ok((status, body)) => {
            let resp = Response::from_data(body)
                .with_status_code(status as i32)
                .with_header(json_header());
            let _ = request.respond(resp);
        }
        Err(e) => {
            if e.status() >= 500 {
                tracing::error!(path = %path, error = %e, "api 5xx");
            }
            let _ = request.respond(e.to_response());
        }
    }
}

/// Max body bytes accepted on non-streaming POST endpoints. The
/// streaming `/api/v1/query` route has its own (larger) cap; the
/// tool endpoints take small JSON payloads so 64 KiB is plenty.
const MAX_POST_BODY: usize = 64 * 1024;

fn read_body(request: &mut Request) -> Result<Vec<u8>, ApiError> {
    let body_len = request.body_length().unwrap_or(0);
    if body_len > MAX_POST_BODY {
        return Err(ApiError::InvalidFilter(format!(
            "request body too large ({body_len} > {MAX_POST_BODY})"
        )));
    }
    let mut buf = Vec::with_capacity(body_len.min(MAX_POST_BODY));
    request
        .as_reader()
        .take(MAX_POST_BODY as u64)
        .read_to_end(&mut buf)
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(buf)
}

fn respond_static(request: Request, r: static_assets::StaticResponse) {
    let ct = Header::from_bytes(b"Content-Type" as &[u8], r.content_type.as_bytes())
        .expect("valid content-type");
    let cache = Header::from_bytes(b"Cache-Control" as &[u8], r.cache.as_bytes())
        .expect("valid cache-control");
    let resp = Response::from_data(r.body)
        .with_status_code(r.status as i32)
        .with_header(ct)
        .with_header(cache);
    let _ = request.respond(resp);
}

fn json_header() -> Header {
    Header::from_bytes(b"Content-Type" as &[u8], b"application/json" as &[u8])
        .expect("valid header")
}

fn runtime_config_json(state: &Arc<AppState>) -> String {
    serde_json::json!({
        "apiBase": "/api/v1",
        "version": env!("CARGO_PKG_VERSION"),
        "writeToolsEnabled": state.allow_write_tools,
    })
    .to_string()
}

fn install_shutdown_handler(flag: Arc<AtomicBool>) -> Result<()> {
    use signal_hook::consts::{SIGINT, SIGTERM};
    signal_hook::flag::register(SIGINT, flag.clone())
        .map_err(|e| anyhow::anyhow!("failed to register SIGINT handler: {e}"))?;
    signal_hook::flag::register(SIGTERM, flag)
        .map_err(|e| anyhow::anyhow!("failed to register SIGTERM handler: {e}"))?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn open_browser(url: &str) -> std::io::Result<()> {
    std::process::Command::new("cmd")
        .args(["/c", "start", "", url])
        .spawn()
        .map(|_| ())
}

#[cfg(target_os = "macos")]
fn open_browser(url: &str) -> std::io::Result<()> {
    std::process::Command::new("open")
        .arg(url)
        .spawn()
        .map(|_| ())
}

#[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
fn open_browser(url: &str) -> std::io::Result<()> {
    std::process::Command::new("xdg-open")
        .arg(url)
        .spawn()
        .map(|_| ())
}

/// Probes PATH for the `claude` (or `claude.exe`) binary. Used at
/// startup to decide whether to build a default runner. Cheap: no
/// process spawn, only filesystem stat per PATH entry.
fn claude_binary_present() -> bool {
    let binary_names: &[&str] = if cfg!(target_os = "windows") {
        &["claude.exe", "claude.cmd", "claude.bat"]
    } else {
        &["claude"]
    };
    let Ok(path_env) = std::env::var("PATH") else {
        return false;
    };
    let sep = if cfg!(target_os = "windows") {
        ';'
    } else {
        ':'
    };
    for dir in path_env.split(sep) {
        if dir.is_empty() {
            continue;
        }
        for name in binary_names {
            let candidate = std::path::Path::new(dir).join(name);
            if candidate.is_file() {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_binary_present_does_not_panic_on_missing_path() {
        // SAFETY: Test mutates PATH then restores. Acceptable in
        // single-test isolation; production code reads PATH read-only.
        // Note: env operations are unsafe in 2024 edition.
        let original = std::env::var("PATH").ok();
        unsafe {
            std::env::remove_var("PATH");
        }
        let result = claude_binary_present();
        if let Some(p) = original {
            unsafe {
                std::env::set_var("PATH", p);
            }
        }
        assert!(!result, "missing PATH should report no claude binary");
    }
}
