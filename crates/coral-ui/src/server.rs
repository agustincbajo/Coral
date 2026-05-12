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

    // Build the default runner. We don't fail if this errors — the
    // runner is only invoked on `/api/v1/query`, and the rest of the
    // read-only surface should remain usable on a system without
    // `claude` installed.
    let runner: Option<Arc<dyn Runner>> = Some(Arc::new(ClaudeRunner::new()) as Arc<dyn Runner>);

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

fn handle(state: &Arc<AppState>, request: Request) {
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
    let token_required = path == "/api/v1/query"
        || path.starts_with("/api/v1/tools/")
        || state.token.is_some();
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

    // Streaming route consumes the request.
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
    std::process::Command::new("open").arg(url).spawn().map(|_| ())
}

#[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
fn open_browser(url: &str) -> std::io::Result<()> {
    std::process::Command::new("xdg-open").arg(url).spawn().map(|_| ())
}
