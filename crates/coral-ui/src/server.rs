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
//!
//! v0.35 CP-2: the recv loop dispatches every request onto a freshly
//! spawned thread, capped at [`MAX_CONCURRENT_HANDLERS`] in-flight via an
//! `Arc<AtomicUsize>` semaphore. Pre-fix `coral ui serve` was a single-
//! threaded recv loop, so a long-running `/api/v1/query` stream (the
//! Claude subprocess can take 10+s) blocked `/health`, `/api/v1/pages`,
//! and the SPA static assets — the browser tab visibly stalled. The
//! pattern is a direct port of `coral-mcp::transport::http_sse`
//! (`MAX_CONCURRENT_HANDLERS = 32`, RAII guard on the in-flight counter,
//! 503 JSON envelope when the cap is hit). We deliberately keep the
//! 250 ms `recv_timeout` shutdown poll already in place — the
//! std-thread / atomics pattern is intentionally tokio-free, matching
//! the validated CP-1 architecture decision.

use std::io::Read;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use coral_runner::{ClaudeRunner, Runner};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::error::ApiError;
use crate::routes;
use crate::state::AppState;
pub use crate::state::ServeConfig;
use crate::static_assets;

/// Maximum concurrent request handlers. A request that would push the
/// in-flight count past this cap is answered `503 Service Unavailable`
/// immediately, with the same JSON envelope shape the rest of the API
/// uses. Matches `coral_mcp::transport::http_sse::MAX_CONCURRENT_HANDLERS`
/// (32 — keeps the FD budget sane on macOS's 256 default ulimit while
/// remaining several orders of magnitude over any realistic browser
/// concurrency).
pub const MAX_CONCURRENT_HANDLERS: usize = 32;

/// Graceful-drain deadline on shutdown. After the recv loop exits we
/// wait up to this long for in-flight handler threads to finish before
/// returning from [`serve`] (and letting the process exit). Five seconds
/// is the same drain budget `coral wiki serve` uses; CP-2 reused it
/// verbatim for parity. A pathologically stuck handler past the budget
/// is abandoned — the OS reaps the threads when the process exits.
const SHUTDOWN_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// Poll interval used while waiting for the in-flight counter to drain.
/// 50 ms balances responsiveness against busy-spin — a typical handler
/// returns within a few ms, so 1-2 iterations is the common case.
const SHUTDOWN_DRAIN_POLL: Duration = Duration::from_millis(50);

/// 503 envelope returned when the concurrent-handler cap is hit. The
/// shape mirrors [`crate::error::ApiError`] so the frontend can deserialize
/// it through the same `{ "error": ... }` reader it already uses for
/// every other 4xx/5xx response.
const BUSY_BODY: &str = r#"{"error":"server busy: too many concurrent requests"}"#;

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

    run_recv_loop(server, state, shutdown);
    Ok(())
}

/// Recv loop body, extracted so integration tests can drive the
/// dispatch logic with a caller-owned `Server` (port 0 + no signal
/// handlers). Pre-extract, the loop was only reachable through
/// [`serve`], which binds a fixed port and registers SIGINT/SIGTERM —
/// both incompatible with `cargo nextest` running tests in parallel
/// inside one process.
///
/// Behaviour:
/// 1. Pre-increment the in-flight counter; over-cap requests get a
///    503 immediately without spawning a thread.
/// 2. Otherwise, spawn a fresh handler thread guarded by
///    [`ActiveGuard`] (decrements on Drop, even on panic).
/// 3. Poll `shutdown` every `recv_timeout` window (250 ms) so the
///    loop exits within ~250 ms of the flag being raised.
/// 4. After the loop exits, drain in-flight handlers up to
///    [`SHUTDOWN_DRAIN_TIMEOUT`] before returning.
pub(crate) fn run_recv_loop(server: Server, state: Arc<AppState>, shutdown: Arc<AtomicBool>) {
    // v0.35 CP-2: in-flight handler counter. We pre-increment + check
    // against the cap before spawning, and decrement via the RAII
    // [`ActiveGuard`] when the handler returns (success, panic, or
    // network error). This keeps the head-of-line block out of the
    // recv loop — `/health` no longer waits for a slow streaming
    // `/api/v1/query` to drain.
    let active = Arc::new(AtomicUsize::new(0));

    loop {
        if shutdown.load(Ordering::Relaxed) {
            eprintln!("\ncoral ui serve: shutting down...");
            break;
        }
        match server.recv_timeout(Duration::from_millis(250)) {
            Ok(Some(req)) => {
                // Reserve a slot atomically. If the post-increment value
                // exceeds the cap, the request gets an immediate 503 and
                // the slot is released — no thread is spawned.
                let inflight = active.fetch_add(1, Ordering::SeqCst) + 1;
                if inflight > MAX_CONCURRENT_HANDLERS {
                    active.fetch_sub(1, Ordering::SeqCst);
                    let _ = respond_busy(req);
                    continue;
                }
                let state_clone = Arc::clone(&state);
                let active_for_guard = Arc::clone(&active);
                let spawn_result = std::thread::Builder::new()
                    .name("coral-ui-http".to_string())
                    .spawn(move || {
                        let _guard = ActiveGuard(active_for_guard);
                        handle(&state_clone, req);
                    });
                if let Err(e) = spawn_result {
                    // Thread::spawn can only really fail when the OS
                    // refuses a new thread (FD / pthread limit). Release
                    // the slot we reserved and log; we don't have the
                    // request anymore (it was moved into the closure on
                    // the happy path), so the recv loop just continues
                    // with the next request.
                    active.fetch_sub(1, Ordering::SeqCst);
                    tracing::warn!(error = %e, "coral ui: could not spawn handler thread");
                }
            }
            Ok(None) => continue,
            Err(_) => continue,
        }
    }

    // v0.35 CP-2: graceful drain. Give in-flight handlers up to
    // [`SHUTDOWN_DRAIN_TIMEOUT`] to finish before we return (and the
    // process exits). The recv loop has already stopped accepting new
    // requests at this point, so the counter is monotonically non-
    // increasing. We poll every [`SHUTDOWN_DRAIN_POLL`] rather than
    // parking on a condvar — the common case is "no in-flight
    // requests" which exits in the first iteration without paying for
    // the synchronization primitive.
    let drain_start = Instant::now();
    loop {
        let inflight = active.load(Ordering::Acquire);
        if inflight == 0 {
            break;
        }
        if drain_start.elapsed() >= SHUTDOWN_DRAIN_TIMEOUT {
            tracing::warn!(
                inflight,
                "coral ui serve: shutdown drain timeout exceeded; abandoning in-flight handlers"
            );
            break;
        }
        std::thread::sleep(SHUTDOWN_DRAIN_POLL);
    }
}

/// RAII helper — decrement the in-flight counter when the handler
/// thread returns (success, panic, or I/O error). Mirrors the same
/// pattern in `coral_mcp::transport::http_sse::ActiveGuard`.
struct ActiveGuard(Arc<AtomicUsize>);

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Emit the canonical busy-response (`503 Service Unavailable` with a
/// JSON envelope). Factored out so the recv loop's overflow branch and
/// the integration tests stay in sync on the wire shape.
fn respond_busy(request: Request) -> std::io::Result<()> {
    let resp = Response::from_string(BUSY_BODY)
        .with_status_code(503)
        .with_header(json_header());
    request.respond(resp)
}

fn handle(state: &Arc<AppState>, mut request: Request) {
    let url = request.url().to_string();
    let path = url.split('?').next().unwrap_or(&url).to_string();
    // v0.35 CP-2: `/_test/*` is only routable in `#[cfg(test)]` builds
    // (the match arm below is conditionally compiled). We mark the
    // path as "api-shaped" here so it bypasses static-asset routing,
    // regardless of cfg — in release builds, the unmatched test path
    // falls through to the API 404 with the same error envelope shape
    // (no accidental SPA fallback for a test path).
    let is_api = path.starts_with("/api/") || path == "/health" || path.starts_with("/_test/");

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
        // v0.35 CP-2: test-only slow route. Sleeps for the duration
        // encoded in the query string (e.g. `?ms=500`) before
        // returning, then echoes the slept-for duration in the body.
        // Compiled out of release builds — keeps the production route
        // surface clean while letting integration tests synthesize
        // head-of-line scenarios that the real streaming routes
        // ([`routes::query`], [`routes::events`]) are too heavyweight
        // to set up in a unit test.
        #[cfg(test)]
        (Method::Get, "/_test/slow") => {
            let ms: u64 = query_string
                .split('&')
                .find_map(|kv| kv.strip_prefix("ms="))
                .and_then(|v| v.parse().ok())
                .unwrap_or(50);
            std::thread::sleep(Duration::from_millis(ms));
            Ok((200, format!("slept {ms}").into_bytes()))
        }
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
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;
    use std::path::PathBuf;

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

    // -- v0.35 CP-2 thread-pool tests --------------------------------

    /// Pin the canonical busy-envelope shape so frontend deserializers
    /// (and the overflow branch in [`run_recv_loop`]) stay in sync.
    #[test]
    fn busy_body_is_canonical_error_envelope() {
        let v: serde_json::Value = serde_json::from_str(BUSY_BODY).expect("BUSY_BODY is JSON");
        assert!(v["error"].is_string(), "envelope must carry `error`: {v}");
        assert!(
            v["error"].as_str().unwrap().contains("busy"),
            "error message should mention 'busy': {v}"
        );
    }

    /// `ActiveGuard` decrements the in-flight counter exactly once when
    /// dropped — including via a `panic` unwind. The recv loop relies
    /// on this so a panicking handler doesn't permanently leak its slot.
    #[test]
    fn active_guard_decrements_on_normal_drop() {
        let active = Arc::new(AtomicUsize::new(0));
        active.fetch_add(1, Ordering::SeqCst);
        {
            let _g = ActiveGuard(Arc::clone(&active));
            assert_eq!(active.load(Ordering::SeqCst), 1);
        }
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn active_guard_decrements_on_panic_unwind() {
        let active = Arc::new(AtomicUsize::new(0));
        active.fetch_add(1, Ordering::SeqCst);
        let active_for_thread = Arc::clone(&active);
        let handle = std::thread::spawn(move || {
            let _g = ActiveGuard(active_for_thread);
            panic!("simulated handler panic");
        });
        assert!(handle.join().is_err(), "thread should propagate panic");
        assert_eq!(
            active.load(Ordering::SeqCst),
            0,
            "guard must decrement even on panic"
        );
    }

    // -- helpers for the recv-loop integration tests -----------------

    /// Build an `AppState` suitable for the in-process recv-loop tests.
    /// The wiki root points at the per-test tempdir so the (rare)
    /// route that touches the filesystem ([`routes::manifest`]) sees
    /// an empty wiki rather than panicking on a missing directory.
    fn test_state(bind: &str, port: u16, wiki_root: PathBuf) -> Arc<AppState> {
        Arc::new(AppState {
            bind: bind.to_string(),
            port,
            wiki_root,
            token: None,
            allow_write_tools: false,
            runner: None,
        })
    }

    /// Spin up `run_recv_loop` on a port-0 server in a background thread
    /// and return `(port, shutdown_flag, JoinHandle)`. The caller passes
    /// a builder closure so the state can be constructed with the real
    /// bound port — `check_host` rejects requests whose `Host:` value
    /// doesn't match `bind:port`, and the OS-picked port isn't known
    /// until after `Server::http("127.0.0.1:0")` returns.
    ///
    /// Joining bounds the recv-loop poll latency (250 ms) + the drain
    /// timeout.
    fn spawn_test_server<F>(make_state: F) -> (u16, Arc<AtomicBool>, std::thread::JoinHandle<()>)
    where
        F: FnOnce(u16) -> Arc<AppState>,
    {
        let server = Server::http("127.0.0.1:0").expect("bind 127.0.0.1:0");
        // tiny_http exposes the bound address even when port==0.
        let port = match server.server_addr() {
            tiny_http::ListenAddr::IP(s) => s.port(),
            #[cfg(unix)]
            tiny_http::ListenAddr::Unix(_) => unreachable!("we bound an IP socket"),
        };
        let state = make_state(port);
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_for_thread = Arc::clone(&shutdown);
        let handle = std::thread::spawn(move || {
            run_recv_loop(server, state, shutdown_for_thread);
        });
        (port, shutdown, handle)
    }

    /// Send a single GET request and return `(status, body)`. Uses a
    /// raw `TcpStream` rather than a fat HTTP client crate so we don't
    /// pull `reqwest` / `ureq` just for these tests. The `Host:` header
    /// is set to `127.0.0.1:<port>` so it satisfies `check_host`
    /// (which compares against `bind:port` case-insensitively).
    fn http_get(port: u16, path: &str) -> std::io::Result<(u16, String)> {
        let mut s = TcpStream::connect(("127.0.0.1", port))?;
        s.set_read_timeout(Some(Duration::from_secs(10)))?;
        let req =
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
        s.write_all(req.as_bytes())?;
        let mut reader = BufReader::new(s);
        let mut status_line = String::new();
        reader.read_line(&mut status_line)?;
        // "HTTP/1.1 200 OK\r\n" → 200
        let status: u16 = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        // Drain headers.
        loop {
            let mut line = String::new();
            reader.read_line(&mut line)?;
            if line == "\r\n" || line.is_empty() {
                break;
            }
        }
        let mut body = String::new();
        reader
            .read_to_string(&mut body)
            .map_err(|e| std::io::Error::other(format!("body read: {e}")))?;
        Ok((status, body))
    }

    /// v0.35 CP-2 / P-C3 — `/health` must answer quickly even while a
    /// slow handler is parked. Pre-fix the single-threaded recv loop
    /// serialized every request, so a 500 ms `/_test/slow` blocked the
    /// next 8 `/health` requests in lock-step. Post-fix each request
    /// runs on its own thread, so `/health` returns in single-digit ms
    /// regardless of what else is in flight.
    #[test]
    fn health_not_blocked_by_concurrent_slow_handler() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let wiki = tmp.path().to_path_buf();
        let (port, shutdown, handle) =
            spawn_test_server(|port| test_state("127.0.0.1", port, wiki));

        // Fire a slow request in the background. It will hold its
        // handler thread for ~500 ms — long enough that any
        // serialization would push the `/health` latencies past the
        // bound below.
        let slow = std::thread::spawn(move || http_get(port, "/_test/slow?ms=500"));
        // Give the slow handler a moment to actually enter its sleep
        // (so we're measuring the "while-blocked" case, not racing the
        // request setup). 100 ms is well under the 500 ms sleep.
        std::thread::sleep(Duration::from_millis(100));

        // Fan out 8 concurrent `/health` GETs. Each one should
        // complete in < 200 ms — the assertion budget is generous
        // (real latency on local loopback is <10 ms) to absorb CI
        // jitter, but still leaves a comfortable gap below the 500 ms
        // serialization-failure threshold.
        let started = Instant::now();
        let workers: Vec<_> = (0..8)
            .map(|_| std::thread::spawn(move || http_get(port, "/health")))
            .collect();
        for w in workers {
            let (status, body) = w.join().expect("worker panicked").expect("http_get");
            assert_eq!(status, 200, "health should be 200, got body: {body}");
            assert!(
                body.contains("\"status\":\"ok\""),
                "health body should be the canonical envelope: {body}"
            );
        }
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(400),
            "8 concurrent /health took {elapsed:?} — should be ≪ 500 ms (the slow handler's \
             duration); pre-fix this would serialize behind the slow handler"
        );

        // Let the slow handler finish, then shut down.
        let (slow_status, _) = slow.join().expect("slow panicked").expect("slow http_get");
        assert_eq!(slow_status, 200, "slow handler should also return 200");
        shutdown.store(true, Ordering::SeqCst);
        handle.join().expect("recv loop thread join");
    }

    /// v0.35 CP-2 / cap test — when the in-flight handler count would
    /// exceed [`MAX_CONCURRENT_HANDLERS`], the recv loop answers 503
    /// immediately without spawning a thread. We force the overflow by
    /// firing (cap + 8) slow handlers in parallel; (cap) should succeed
    /// and the surplus should each get a 503 with the canonical
    /// `{"error":"..."}` envelope.
    #[test]
    fn concurrent_cap_returns_503_above_threshold() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let wiki = tmp.path().to_path_buf();
        let (port, shutdown, handle) =
            spawn_test_server(|port| test_state("127.0.0.1", port, wiki));

        // Fire (cap + 8) slow requests. Each sleeps for 400 ms; the
        // surplus 8 should bounce off the cap with 503 long before
        // any in-flight handler finishes.
        let total = MAX_CONCURRENT_HANDLERS + 8;
        let workers: Vec<_> = (0..total)
            .map(|_| std::thread::spawn(move || http_get(port, "/_test/slow?ms=400")))
            .collect();
        let mut ok = 0;
        let mut busy = 0;
        let mut bodies_busy: Vec<String> = Vec::new();
        for w in workers {
            match w.join().expect("worker panicked") {
                Ok((200, _)) => ok += 1,
                Ok((503, body)) => {
                    busy += 1;
                    bodies_busy.push(body);
                }
                Ok((status, body)) => panic!("unexpected status {status} body={body}"),
                Err(e) => panic!("http_get error: {e}"),
            }
        }
        // We can't guarantee EXACTLY `cap` succeed and `8` 503 — the
        // recv loop is a real concurrent system and a request can win
        // a slot after another handler exits. But we MUST see at least
        // one 503 (the cap was exceeded) and the total is conserved.
        assert_eq!(ok + busy, total, "every request must terminate");
        assert!(
            busy >= 1,
            "at least one request must hit the cap; ok={ok} busy={busy}"
        );
        assert!(
            ok <= MAX_CONCURRENT_HANDLERS + total / 4,
            "succeeded count {ok} unreasonably high — the cap may not be enforced"
        );
        // Every 503 body is the canonical envelope.
        for body in &bodies_busy {
            let v: serde_json::Value = serde_json::from_str(body).unwrap_or_else(|_| {
                panic!("503 body is not JSON: {body}");
            });
            assert!(
                v["error"].as_str().unwrap_or("").contains("busy"),
                "503 envelope should mention busy: {body}"
            );
        }
        shutdown.store(true, Ordering::SeqCst);
        // Drain handlers + recv-loop teardown. Inside the drain budget
        // (~5 s) this returns cleanly even with up-to-cap in-flight
        // sleepers still parked.
        handle.join().expect("recv loop thread join");
    }

    /// v0.35 CP-2 / CON-06 — raising the shutdown flag drains the
    /// recv loop within ~250 ms (the `recv_timeout` poll window) plus
    /// the in-flight drain. With no in-flight handlers the drain
    /// returns on the first poll, so the total budget is just the
    /// poll window. Pre-fix the loop had no shutdown poll at all (it
    /// blocked in `incoming_requests` until the next request landed).
    #[test]
    fn shutdown_flag_drains_recv_loop_promptly() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let wiki = tmp.path().to_path_buf();
        let (port, shutdown, handle) =
            spawn_test_server(|port| test_state("127.0.0.1", port, wiki));
        // Touch the server with one healthy request so we know it's
        // actually running — guards against a "shutdown bound to a
        // dead loop" false-pass.
        let (status, _) = http_get(port, "/health").expect("warmup /health");
        assert_eq!(status, 200);

        shutdown.store(true, Ordering::SeqCst);
        let started = Instant::now();
        handle.join().expect("recv loop thread join");
        let elapsed = started.elapsed();
        // Budget: 250 ms poll window + drain (instant with no in-flight)
        // + scheduler slack. 1 s is comfortable but still well below
        // the pre-fix unbounded blocking time.
        assert!(
            elapsed < Duration::from_secs(1),
            "shutdown took {elapsed:?} — should be ≤ ~250 ms poll + epsilon"
        );
    }
}
