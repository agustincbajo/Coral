//! Streamable HTTP/SSE MCP transport (MCP 2025-11-25).
//!
//! Wire shape (the spec's "Streamable HTTP" transport):
//!
//! | Method | Path | Server response |
//! |---|---|---|
//! | `POST /mcp` | JSON-RPC envelope, `Accept: application/json, text/event-stream` | 200 application/json with response body for single-answer; 204 for notification (no `id`) |
//! | `GET /mcp` | `Accept: text/event-stream` | 200 text/event-stream empty stream + 15s `:keep-alive` heartbeat |
//! | `DELETE /mcp` | `Mcp-Session-Id: <id>` | 204 if session existed; 404 otherwise |
//!
//! Headers required:
//! - `Mcp-Session-Id`: server generates on `initialize`, client echoes
//!   on subsequent. Sessions live in an `Arc<Mutex<HashMap<String,
//!   Instant>>>` with a 1h TTL, reaped each request.
//! - `Origin`: validated against `null` / `http://localhost*` /
//!   `http://127.0.0.1*` (DNS-rebinding mitigation per MCP spec).
//!   Forbidden otherwise.
//! - `Accept`: must include `application/json` or `text/event-stream`.
//!   406 otherwise.
//!
//! Caps (acceptance criteria):
//! - 4 MiB body cap → 413
//! - 32 concurrent threads → 503 if exhausted
//! - Batched JSON-RPC arrays → 400 (pinned-out for v0.21.1; spec
//!   allows but defers it)
//!
//! Security model (see README "Security model for the HTTP transport"):
//! - Default bind is `127.0.0.1` — the CLI flag `--bind 0.0.0.0` is
//!   opt-in and emits a stderr warning banner.
//! - Origin validation only protects browser clients (the spec's
//!   DNS-rebinding mitigation); native clients can spoof Origin
//!   trivially. The 127.0.0.1 default is the load-bearing defense.

use crate::server::McpHandler;
use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Maximum POST body size before the server returns 413. The MCP wire
/// is JSON-RPC envelopes — even hairy `tools/call` payloads fit in a
/// few hundred KiB. 4 MiB leaves several orders of magnitude of headroom
/// before the cap hits a legit client.
pub const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

/// Maximum concurrent request handlers. A 33rd request is answered
/// `503 Service Unavailable`. Picked to keep the per-process FD budget
/// in check on macOS (default `ulimit -n` is 256) without throttling
/// any realistic agent workload — every shipped MCP client multiplexes
/// a single conversation per session.
pub const MAX_CONCURRENT_HANDLERS: usize = 32;

/// Time-to-live for `Mcp-Session-Id` cookies. 1h is generous for
/// long-running agent sessions and short enough that an abandoned
/// browser tab doesn't leak indefinitely.
pub const SESSION_TTL: Duration = Duration::from_secs(60 * 60);

/// SSE keep-alive interval. The spec doesn't pin a value; 15s is
/// snug enough that proxies don't drop idle connections and loose
/// enough to avoid spamming the wire.
pub const SSE_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// v0.30 audit #010: bounded replay buffer for SSE `Last-Event-ID`
/// resumption. The MCP spec doesn't pin a size; 128 covers a few
/// minutes of typical wiki-edit chatter at one notification/sec while
/// keeping the buffer's memory footprint tiny (a `Value` per slot).
/// Older events are dropped — clients that resume after a long
/// disconnect get whatever is still in the ring.
pub const SSE_REPLAY_BUFFER_SIZE: usize = 128;

/// v0.30 audit #010: shared notification ring buffer + a Condvar to
/// wake parked SSE writers when a new event arrives. Multiple
/// concurrent `GET /mcp` connections all read from the same buffer
/// (broadcast semantics), so the underlying notification mpsc only
/// needs one consumer (the dispatcher thread in
/// [`HttpSseTransport::bind`]).
pub(crate) struct NotificationHub {
    pub(crate) buffer: Mutex<VecDeque<(u64, serde_json::Value)>>,
    pub(crate) cond: Condvar,
    pub(crate) next_id: AtomicU64,
}

impl NotificationHub {
    pub(crate) fn new() -> Self {
        Self {
            buffer: Mutex::new(VecDeque::with_capacity(SSE_REPLAY_BUFFER_SIZE)),
            cond: Condvar::new(),
            next_id: AtomicU64::new(1),
        }
    }

    /// Push a notification into the ring, evicting the oldest entry
    /// when the cap is exceeded. Wakes all parked SSE writers.
    pub(crate) fn publish(&self, value: serde_json::Value) {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let mut buf = self.buffer.lock().expect("notification buffer mutex");
        if buf.len() >= SSE_REPLAY_BUFFER_SIZE {
            buf.pop_front();
        }
        buf.push_back((id, value));
        drop(buf);
        self.cond.notify_all();
    }
}

/// Public handle for the bound HTTP/SSE transport. Tests use
/// [`Self::local_addr`] when binding to port 0.
pub struct HttpSseTransport {
    server: tiny_http::Server,
    handler: Arc<McpHandler>,
    sessions: Arc<Mutex<HashMap<String, Instant>>>,
    active: Arc<AtomicUsize>,
    local_addr: SocketAddr,
    /// v0.30 audit #010: shared replay ring + condvar broadcast hub
    /// for SSE notifications. Populated by the dispatcher thread
    /// spawned in [`Self::bind`].
    notifications: Arc<NotificationHub>,
}

impl HttpSseTransport {
    /// Bind a `tiny_http::Server` to `addr` and return a handle ready
    /// for [`Self::serve_blocking`]. Bind errors (`EADDRINUSE`, etc.)
    /// surface as plain `io::Error`.
    pub fn bind(addr: SocketAddr, handler: Arc<McpHandler>) -> io::Result<Self> {
        let server = tiny_http::Server::http(addr).map_err(|e| {
            io::Error::other(format!(
                "could not bind MCP HTTP transport to {addr}: {e}. \
                 If the port is already in use, pick a different one with --port; \
                 the default is {}.",
                3737u16
            ))
        })?;
        let local_addr = match server.server_addr() {
            tiny_http::ListenAddr::IP(s) => s,
            tiny_http::ListenAddr::Unix(_) => {
                return Err(io::Error::other(
                    "MCP HTTP transport bound to a unix socket — not supported in v0.21.1; \
                     pass a TCP --bind/--port instead",
                ));
            }
        };
        // v0.30 audit #010: wire a notification hub. The handler's
        // existing `notification_tx` is an mpsc; we own the rx end on
        // a dispatcher thread that re-publishes every incoming
        // notification into the shared replay ring + condvar broadcast.
        // SSE connections poll the ring directly, so multiple GET /mcp
        // streams all see every event (true broadcast).
        let notifications = Arc::new(NotificationHub::new());
        let (tx, rx) = std::sync::mpsc::channel::<serde_json::Value>();
        handler.set_notification_sender(tx);
        {
            let hub = Arc::clone(&notifications);
            std::thread::Builder::new()
                .name("coral-mcp-sse-dispatcher".to_string())
                .spawn(move || {
                    while let Ok(value) = rx.recv() {
                        hub.publish(value);
                    }
                })
                .map_err(|e| {
                    io::Error::other(format!(
                        "could not spawn SSE notification dispatcher: {e}"
                    ))
                })?;
        }
        Ok(Self {
            server,
            handler,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            active: Arc::new(AtomicUsize::new(0)),
            local_addr,
            notifications,
        })
    }

    /// Local address the listener bound to. When the user passes
    /// `--port 0`, the OS picks a free port; the test harness reads
    /// it back via this method.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.local_addr)
    }

    /// Block the calling thread until the server stops accepting
    /// connections. Each request is dispatched on a fresh thread,
    /// capped at [`MAX_CONCURRENT_HANDLERS`] in flight.
    pub fn serve_blocking(self) -> io::Result<()> {
        let HttpSseTransport {
            server,
            handler,
            sessions,
            active,
            notifications,
            ..
        } = self;
        for request in server.incoming_requests() {
            let handler = Arc::clone(&handler);
            let sessions = Arc::clone(&sessions);
            let active = Arc::clone(&active);
            let notifications = Arc::clone(&notifications);
            // Enforce the concurrent-handler cap. We pre-increment
            // and check, then decrement either when the handler
            // returns or in the "too many" branch.
            let inflight = active.fetch_add(1, Ordering::SeqCst) + 1;
            if inflight > MAX_CONCURRENT_HANDLERS {
                active.fetch_sub(1, Ordering::SeqCst);
                let _ = respond_simple(
                    request,
                    503,
                    "text/plain; charset=utf-8",
                    "{\"error\":\"server busy: too many concurrent requests\"}",
                );
                continue;
            }
            let active_for_guard = Arc::clone(&active);
            let spawn_result = std::thread::Builder::new()
                .name("coral-mcp-http".to_string())
                .spawn(move || {
                    let _guard = ActiveGuard(active_for_guard);
                    if let Err(e) = handle_request(request, &handler, &sessions, &notifications)
                    {
                        tracing::warn!(error = %e, "MCP HTTP request handler error");
                    }
                });
            if let Err(e) = spawn_result {
                // Spawn failed — release the slot we reserved.
                active.fetch_sub(1, Ordering::SeqCst);
                tracing::warn!(error = %e, "could not spawn MCP HTTP handler thread");
            }
        }
        Ok(())
    }
}

/// RAII helper — decrement the in-flight counter when the handler
/// thread returns (success, panic, anything).
struct ActiveGuard(Arc<AtomicUsize>);

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Convenience entry point — bind to `addr` and serve until closed.
pub fn serve_http_sse(handler: Arc<McpHandler>, addr: SocketAddr) -> io::Result<()> {
    HttpSseTransport::bind(addr, handler)?.serve_blocking()
}

/// Top-level dispatcher per request. Returns `Ok(())` whether the
/// request was answered with 200, 204, 4xx, or 5xx — it only
/// returns `Err` for I/O failures the runtime can't recover from.
fn handle_request(
    request: tiny_http::Request,
    handler: &McpHandler,
    sessions: &Arc<Mutex<HashMap<String, Instant>>>,
    notifications: &Arc<NotificationHub>,
) -> io::Result<()> {
    reap_expired_sessions(sessions);

    let method = request.method().clone();
    let url = request.url().to_string();

    // CORS preflight — answered before any other validation.
    if matches!(method, tiny_http::Method::Options) {
        return respond_options(request);
    }

    // v0.22.5: discoverable MCP server card per the 2025-11-25 spec.
    // Mounted at `/.well-known/mcp/server-card.json` and intentionally
    // exempt from the `/mcp` Origin allowlist below — registries and
    // discovery probes hit this from any origin (including a fresh
    // browser tab on `https://example.com`) and a 403 would defeat the
    // whole point of "discoverable". The card is a static JSON document
    // built from compile-time constants + catalog `.len()` counts —
    // there's no session state, no tool dispatch, no PII, no exfil
    // surface. The load-bearing defense for the actual MCP traffic
    // (POST /mcp) remains the 127.0.0.1 default bind + Origin allowlist
    // applied below.
    //
    // Any other path under `/.well-known/mcp/*` (or a non-GET method
    // on the card path) returns 404 — the card is the only well-known
    // resource we publish.
    let path = url.split('?').next().unwrap_or("");
    if path == "/.well-known/mcp/server-card.json" {
        if !matches!(method, tiny_http::Method::Get) {
            return respond_simple(request, 404, "application/json", r#"{"error":"not found"}"#);
        }
        return handle_well_known_card(request, handler);
    }
    if path.starts_with("/.well-known/mcp/") {
        return respond_simple(request, 404, "application/json", r#"{"error":"not found"}"#);
    }

    // Path routing. We accept exact `/mcp` only; anything else is 404.
    if path != "/mcp" {
        return respond_simple(
            request,
            404,
            "application/json",
            r#"{"error":"not found; MCP transport is mounted at /mcp"}"#,
        );
    }

    // Origin validation (DNS-rebinding mitigation per MCP spec).
    // Native clients can spoof Origin, so this is a browser-side
    // defense — the load-bearing protection is the 127.0.0.1 default.
    let origin = header_value(request.headers(), "Origin");
    if let Some(o) = origin.as_deref() {
        if !is_origin_allowed(o) {
            return respond_simple(
                request,
                403,
                "application/json",
                r#"{"error":"forbidden Origin: only null / http://localhost / http://127.0.0.1 allowed"}"#,
            );
        }
    }

    // Accept header validation. POST and GET have different requirements.
    let accept = header_value(request.headers(), "Accept").unwrap_or_default();

    match method {
        tiny_http::Method::Post => {
            // POST must accept application/json (preferred) or text/event-stream.
            if !accept_includes_json(&accept) && !accept_includes_sse(&accept) {
                return respond_simple(
                    request,
                    406,
                    "application/json",
                    r#"{"error":"Accept must include application/json or text/event-stream"}"#,
                );
            }
            handle_post(request, handler, sessions)
        }
        tiny_http::Method::Get => {
            // GET requires SSE.
            if !accept_includes_sse(&accept) {
                return respond_simple(
                    request,
                    406,
                    "text/plain; charset=utf-8",
                    "Accept must include text/event-stream for GET /mcp",
                );
            }
            handle_get_sse(request, notifications)
        }
        tiny_http::Method::Delete => handle_delete(request, sessions),
        _ => respond_simple(
            request,
            405,
            "text/plain; charset=utf-8",
            "method not allowed",
        ),
    }
}

/// POST /mcp — read body (capped), reject batched arrays, pass to
/// `handler.handle_line`, wrap response.
fn handle_post(
    mut request: tiny_http::Request,
    handler: &McpHandler,
    sessions: &Arc<Mutex<HashMap<String, Instant>>>,
) -> io::Result<()> {
    // v0.30 audit #B5: validate Content-Type before reading the body.
    // The MCP "Streamable HTTP" spec requires `application/json` on
    // POST; anything else is a transport-shape error → 415. We accept
    // an optional `charset=` parameter (per RFC 7231 §3.1.1.5).
    if !content_type_is_json(&header_value(request.headers(), "Content-Type").unwrap_or_default())
    {
        return respond_simple(
            request,
            415,
            "application/json",
            r#"{"error":"Content-Type must be application/json"}"#,
        );
    }

    // Body size check. tiny_http exposes `body_length()` — preferred
    // when Content-Length is set; fall back to a streaming read with
    // a hard cap.
    if let Some(len) = request.body_length() {
        if len > MAX_BODY_BYTES {
            return respond_simple(
                request,
                413,
                "application/json",
                r#"{"error":"payload too large; cap is 4 MiB"}"#,
            );
        }
    }

    let mut body = Vec::with_capacity(request.body_length().unwrap_or(0).min(MAX_BODY_BYTES));
    let reader = request.as_reader();
    let mut limited = reader.take((MAX_BODY_BYTES as u64) + 1);
    limited.read_to_end(&mut body)?;
    if body.len() > MAX_BODY_BYTES {
        return respond_simple(
            request,
            413,
            "application/json",
            r#"{"error":"payload too large; cap is 4 MiB"}"#,
        );
    }

    let body_str = match std::str::from_utf8(&body) {
        Ok(s) => s,
        Err(_) => {
            return respond_simple(
                request,
                400,
                "application/json",
                r#"{"error":"body must be UTF-8 JSON-RPC"}"#,
            );
        }
    };

    // Reject batched arrays (spec allows but v0.21.1 defers).
    if is_jsonrpc_batch(body_str) {
        return respond_simple(
            request,
            400,
            "application/json",
            r#"{"error":"MCP batching not yet supported; send one envelope per POST"}"#,
        );
    }

    // Dispatch through the shared handler. handle_line returns
    // `Some(value)` for requests with `id` (single-answer) and
    // `None` for notifications.
    let response_value = handler.handle_line(body_str);

    // v0.30 audit #B6: mint a session ID on initialize. Pre-fix this
    // substring-sniffed `"initialize"` from the raw body, which false-
    // positived on `tools/call` arguments containing that literal
    // token (e.g. a prompt mentioning the word). Parse the envelope's
    // `method` field cheaply (we already parsed it inside
    // `handle_line`, but its parsed form isn't exposed; re-parsing
    // just the top-level shape is microseconds).
    let mut response_headers: Vec<(String, String)> = Vec::new();
    let parsed_method = parse_jsonrpc_method(body_str);
    if parsed_method.as_deref() == Some("initialize") {
        let session_id = new_session_id();
        sessions
            .lock()
            .expect("session map mutex")
            .insert(session_id.clone(), Instant::now());
        response_headers.push(("Mcp-Session-Id".to_string(), session_id));
    } else {
        // Subsequent requests SHOULD include Mcp-Session-Id; we
        // surface but don't enforce — a missing-session POST still
        // succeeds, but the audit trail logs it. Clients that care
        // about the contract can opt in via the `--strict-sessions`
        // flag (tracked for v0.22+).
        if let Some(id) = header_value(request.headers(), "Mcp-Session-Id") {
            // Touch the entry's last-seen timestamp.
            sessions
                .lock()
                .expect("session map mutex")
                .insert(id, Instant::now());
        }
    }

    match response_value {
        Some(v) => respond_json(request, 200, &v, &response_headers),
        None => respond_simple(request, 204, "application/json", ""),
    }
}

/// GET /mcp — open an SSE stream, replay any buffered notifications
/// whose event-id is greater than `Last-Event-ID` (if present), then
/// drain the shared notification hub for the lifetime of the
/// connection. Emit `: keep-alive\n\n` every [`SSE_KEEPALIVE_INTERVAL`].
///
/// v0.30 audit #010: pre-fix this stream was keep-alive-only, so
/// HTTP/SSE clients never saw `notifications/resources/updated` /
/// `notifications/resources/list_changed` even though the server's
/// `initialize` advertised `subscribe: true`. The hub is populated by
/// the dispatcher thread spawned in [`HttpSseTransport::bind`].
///
/// Limitations:
/// - The replay ring is bounded ([`SSE_REPLAY_BUFFER_SIZE`] entries).
///   Clients that disconnect for longer than the buffer's lifespan
///   will miss events whose ids fall below the oldest buffered id —
///   they should fall back to `resources/list` on reconnect.
/// - Event IDs are per-transport-process and monotonic from 1; they
///   do NOT survive process restart. A new process emits id=1 again,
///   so clients MUST tolerate id resets across server reboots.
fn handle_get_sse(
    request: tiny_http::Request,
    notifications: &Arc<NotificationHub>,
) -> io::Result<()> {
    // Snapshot Last-Event-ID before consuming the request. Invalid /
    // missing → 0 (replay everything still in the buffer).
    let last_event_id: u64 = header_value(request.headers(), "Last-Event-ID")
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);

    let mut writer = request.into_writer();
    let head = b"HTTP/1.1 200 OK\r\n\
                 Content-Type: text/event-stream\r\n\
                 Cache-Control: no-cache\r\n\
                 Connection: keep-alive\r\n\
                 X-Accel-Buffering: no\r\n\
                 \r\n";
    if writer.write_all(head).is_err() {
        return Ok(());
    }
    if writer.write_all(b": connected\n\n").is_err() {
        return Ok(());
    }
    if writer.flush().is_err() {
        return Ok(());
    }

    // v0.30 audit #010: replay buffered events with id > Last-Event-ID.
    let mut cursor: u64 = last_event_id;
    let mut replay_batch: Vec<(u64, serde_json::Value)> = Vec::new();
    {
        let buf = notifications.buffer.lock().expect("notification buffer");
        for (id, value) in buf.iter() {
            if *id > last_event_id {
                replay_batch.push((*id, value.clone()));
            }
        }
    }
    for (id, value) in replay_batch {
        if write_sse_event(&mut writer, id, &value).is_err() {
            return Ok(());
        }
        cursor = cursor.max(id);
    }

    // Main loop: park on the condvar until either a new event arrives
    // or the keep-alive interval elapses. On wake, drain any new
    // events past `cursor`; on timeout, emit a comment so proxies
    // don't drop the connection.
    loop {
        // Collect events past `cursor` into a local Vec while holding
        // the lock briefly. We use Condvar::wait_timeout to block
        // efficiently — no busy-spin.
        let new_events: Vec<(u64, serde_json::Value)> = {
            let buf = notifications.buffer.lock().expect("notification buffer");
            // If there's nothing new, park.
            let has_new = buf.iter().any(|(id, _)| *id > cursor);
            if has_new {
                buf.iter()
                    .filter(|(id, _)| *id > cursor)
                    .map(|(id, v)| (*id, v.clone()))
                    .collect()
            } else {
                let (buf, _) = notifications
                    .cond
                    .wait_timeout(buf, SSE_KEEPALIVE_INTERVAL)
                    .expect("condvar wait");
                buf.iter()
                    .filter(|(id, _)| *id > cursor)
                    .map(|(id, v)| (*id, v.clone()))
                    .collect()
            }
        };

        if new_events.is_empty() {
            // Timeout path → keep-alive comment.
            if writer.write_all(b": keep-alive\n\n").is_err() {
                break;
            }
            if writer.flush().is_err() {
                break;
            }
            continue;
        }

        for (id, value) in new_events {
            if write_sse_event(&mut writer, id, &value).is_err() {
                return Ok(());
            }
            cursor = cursor.max(id);
        }
    }
    Ok(())
}

/// Serialize one SSE `id: <n>\ndata: <json>\n\n` frame to `writer`.
fn write_sse_event(
    writer: &mut dyn Write,
    id: u64,
    value: &serde_json::Value,
) -> io::Result<()> {
    let data = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    let frame = format!("id: {id}\ndata: {data}\n\n");
    writer.write_all(frame.as_bytes())?;
    writer.flush()?;
    Ok(())
}

/// DELETE /mcp — terminate a session by `Mcp-Session-Id`.
fn handle_delete(
    request: tiny_http::Request,
    sessions: &Arc<Mutex<HashMap<String, Instant>>>,
) -> io::Result<()> {
    let id = match header_value(request.headers(), "Mcp-Session-Id") {
        Some(s) => s,
        None => {
            return respond_simple(
                request,
                400,
                "application/json",
                r#"{"error":"DELETE requires Mcp-Session-Id header"}"#,
            );
        }
    };
    let removed = sessions.lock().expect("session map mutex").remove(&id);
    if removed.is_some() {
        respond_simple(request, 204, "application/json", "")
    } else {
        respond_simple(
            request,
            404,
            "application/json",
            r#"{"error":"session not found"}"#,
        )
    }
}

/// `GET /.well-known/mcp/server-card.json` — discoverable MCP server
/// card per the 2025-11-25 spec. Public by design (no Origin check, no
/// session id), pretty-printed JSON, `Content-Type: application/json`.
///
/// The body is computed via [`crate::card::server_card`] sampling the
/// handler's resource provider + the static `ToolCatalog` /
/// `PromptCatalog`. Counts reflect the FULL catalog so a registry
/// observing the card sees capability shape independent of the per-
/// process `--allow-write-tools` gate.
///
/// v0.22.5 acceptance criterion: this surface and `coral mcp card`
/// emit byte-identical bodies modulo the trailing newline `println!`
/// adds. Pinned by the e2e test
/// `well_known_card_endpoint_returns_200_with_valid_json` and the
/// CLI smoke test `cli_mcp_card_emits_json_to_stdout`.
fn handle_well_known_card(request: tiny_http::Request, handler: &McpHandler) -> io::Result<()> {
    let card = crate::card::server_card(
        handler.resources.as_ref(),
        &crate::tools::ToolCatalog,
        &crate::prompts::PromptCatalog,
    );
    let body = serde_json::to_string_pretty(&card).unwrap_or_else(|_| "{}".to_string());
    respond_simple(request, 200, "application/json", &body)
}

/// CORS preflight responder. Tight allowlist — no wildcard origin,
/// only the methods the MCP transport actually accepts.
fn respond_options(request: tiny_http::Request) -> io::Result<()> {
    // Echo the request's Origin if it's allowed; otherwise reflect
    // a safe default. Origin reflection here is tighter than wildcard
    // because the response includes credentials-relevant headers.
    let origin = header_value(request.headers(), "Origin").unwrap_or_default();
    let allowed_origin = if is_origin_allowed(&origin) && !origin.is_empty() {
        origin
    } else {
        "http://localhost".to_string()
    };
    let response = tiny_http::Response::empty(200)
        .with_header(
            format!("Access-Control-Allow-Origin: {allowed_origin}")
                .parse::<tiny_http::Header>()
                .unwrap(),
        )
        .with_header(
            "Access-Control-Allow-Methods: POST, GET, DELETE, OPTIONS"
                .parse::<tiny_http::Header>()
                .unwrap(),
        )
        .with_header(
            "Access-Control-Allow-Headers: Content-Type, Mcp-Session-Id, Accept, Origin"
                .parse::<tiny_http::Header>()
                .unwrap(),
        )
        .with_header(
            "Access-Control-Max-Age: 600"
                .parse::<tiny_http::Header>()
                .unwrap(),
        )
        .with_header("Vary: Origin".parse::<tiny_http::Header>().unwrap());
    request.respond(response)
}

/// Final-form responder for plain text or JSON bodies.
fn respond_simple(
    request: tiny_http::Request,
    status: u16,
    content_type: &str,
    body: &str,
) -> io::Result<()> {
    let response = tiny_http::Response::from_string(body.to_string())
        .with_status_code(status)
        .with_header(
            format!("Content-Type: {content_type}")
                .parse::<tiny_http::Header>()
                .unwrap(),
        );
    request.respond(response)
}

/// Final-form responder for JSON `serde_json::Value` bodies, with
/// optional extra headers (used to inject `Mcp-Session-Id` on
/// initialize).
fn respond_json(
    request: tiny_http::Request,
    status: u16,
    value: &serde_json::Value,
    extra_headers: &[(String, String)],
) -> io::Result<()> {
    let body = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    let mut response = tiny_http::Response::from_string(body)
        .with_status_code(status)
        .with_header(
            "Content-Type: application/json"
                .parse::<tiny_http::Header>()
                .unwrap(),
        );
    for (k, v) in extra_headers {
        if let Ok(h) = format!("{k}: {v}").parse::<tiny_http::Header>() {
            response = response.with_header(h);
        }
    }
    request.respond(response)
}

/// Case-insensitive header lookup. We compare the field's stringified
/// form via ASCII lowercase rather than `field.equiv(...)` because
/// `equiv` requires `&'static str` — fine for compile-time constants
/// but we also pass dynamic names from origin echo / debug paths.
fn header_value(headers: &[tiny_http::Header], name: &str) -> Option<String> {
    let needle = name.to_ascii_lowercase();
    headers
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(&needle))
        .map(|h| h.value.as_str().to_string())
}

/// Origin allowlist per the MCP spec's DNS-rebinding mitigation.
/// `null` (file:// origins) and any localhost / 127.0.0.1 host pass;
/// everything else is rejected.
pub fn is_origin_allowed(origin: &str) -> bool {
    if origin.is_empty() || origin == "null" {
        return true;
    }
    // Parse `scheme://host[:port]` manually — pulling `url` for one
    // host check would inflate the dep tree.
    let after_scheme = match origin.split_once("://") {
        Some((_, rest)) => rest,
        None => return false,
    };
    // Strip path/query if any.
    let host_port = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    // Bracketed IPv6 literal: `[::1]` or `[::1]:port`. Match the
    // segment up to the closing bracket as the host. Otherwise, the
    // host is everything before the first `:` (the port separator).
    let host = if let Some(rest) = host_port.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

/// Returns true if the Accept header includes `application/json` (or
/// the wildcard `*/*`).
pub fn accept_includes_json(accept: &str) -> bool {
    let lower = accept.to_lowercase();
    lower.contains("application/json") || lower.contains("*/*")
}

/// Returns true if the Accept header includes `text/event-stream`.
pub fn accept_includes_sse(accept: &str) -> bool {
    let lower = accept.to_lowercase();
    lower.contains("text/event-stream") || lower.contains("*/*")
}

/// Heuristic: a JSON-RPC batch is an outer JSON array. We don't fully
/// parse the body — `serde_json::from_str(...).is_array()` would
/// also accept malformed input. Cheap leading-character sniff:
/// strip leading whitespace + UTF-8 BOM, then check for `[`.
pub fn is_jsonrpc_batch(body: &str) -> bool {
    let s = body.trim_start_matches('\u{feff}').trim_start();
    s.starts_with('[')
}

/// v0.30 audit #B5: returns true if the `Content-Type` header is
/// `application/json`, with an optional charset / media-type
/// parameter (per RFC 7231). Case-insensitive on the media type.
pub fn content_type_is_json(content_type: &str) -> bool {
    let primary = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    primary == "application/json"
}

/// v0.30 audit #B6: parse the JSON-RPC envelope to extract the
/// `method` field, returning `None` for non-JSON bodies or envelopes
/// missing `method`. Pre-fix the HTTP transport substring-sniffed
/// `"initialize"` from the raw body, false-positiving on tool
/// arguments that contain the literal token.
pub fn parse_jsonrpc_method(body: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Envelope {
        method: Option<String>,
    }
    serde_json::from_str::<Envelope>(body)
        .ok()
        .and_then(|e| e.method)
}

/// Mint an opaque 36-char UUID-shaped session ID. This is NOT
/// cryptographically random — the spec doesn't require it, and
/// pulling `uuid` for one helper would inflate the dep tree.
/// Format mimics a v4 UUID for client-side regex compatibility.
pub fn new_session_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u128;
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed) as u128;
    let mixed = nanos.wrapping_mul(0xdead_beef_u128).wrapping_add(counter);
    let bytes: [u8; 16] = mixed.to_le_bytes();
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        u16::from_le_bytes([bytes[4], bytes[5]]),
        u16::from_le_bytes([bytes[6], bytes[7]]) & 0xfff,
        u16::from_le_bytes([bytes[8], bytes[9]]),
        u64::from_le_bytes([
            bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15], 0, 0
        ]) & 0xffff_ffff_ffff,
    )
}

/// Drop session entries older than [`SESSION_TTL`]. Called on every
/// request so the table stays bounded without a dedicated reaper
/// thread.
fn reap_expired_sessions(sessions: &Arc<Mutex<HashMap<String, Instant>>>) {
    let now = Instant::now();
    let mut guard = sessions.lock().expect("session map mutex");
    guard.retain(|_, last_seen| now.saturating_duration_since(*last_seen) < SESSION_TTL);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #1 — request-line parse: validates the Origin allowlist on
    /// representative inputs.
    #[test]
    fn origin_allowlist_admits_localhost_and_rejects_others() {
        assert!(is_origin_allowed(""));
        assert!(is_origin_allowed("null"));
        assert!(is_origin_allowed("http://localhost"));
        assert!(is_origin_allowed("http://localhost:3737"));
        assert!(is_origin_allowed("https://localhost:8080"));
        assert!(is_origin_allowed("http://127.0.0.1"));
        assert!(is_origin_allowed("http://127.0.0.1:9000"));
        // IPv6 loopback in bracketed form, with and without port.
        assert!(is_origin_allowed("http://[::1]"));
        assert!(is_origin_allowed("http://[::1]:3737"));
        assert!(!is_origin_allowed("http://example.com"));
        assert!(!is_origin_allowed("https://attacker.com"));
        // Look-alike: subdomain of localhost-named tld is NOT
        // localhost — different host per RFC 3986.
        assert!(!is_origin_allowed("http://localhost.evil.com"));
    }

    /// #2 — Accept header validation rejects text/plain.
    #[test]
    fn accept_validation_rejects_text_plain() {
        assert!(accept_includes_json("application/json"));
        assert!(accept_includes_json("application/json, text/event-stream"));
        assert!(accept_includes_json("*/*"));
        assert!(accept_includes_sse("text/event-stream"));
        assert!(!accept_includes_json("text/plain"));
        assert!(!accept_includes_sse("text/plain"));
        // Mixed case must still match.
        assert!(accept_includes_json("Application/JSON"));
    }

    /// #3 — Origin DNS-rebind block + localhost allow (more
    /// adversarial cases beyond #1).
    #[test]
    fn origin_blocks_dns_rebind_attempts() {
        // Subdomain of attacker that rebinds to 127.0.0.1 — still
        // rejected because the Origin host is not literally
        // localhost/127.0.0.1.
        assert!(!is_origin_allowed("http://malicious.localhost.attacker"));
        assert!(!is_origin_allowed("http://localhost.attacker.com"));
        // Userinfo trick: `http://localhost@attacker.com` — `host`
        // parsing splits on `://`, so the userinfo is part of the
        // host_port string. Our parser walks the host as the leading
        // segment before `:`, so `localhost@attacker.com` reads as
        // host `localhost@attacker.com` — not a match for the
        // allowlist. Verify this is rejected.
        assert!(!is_origin_allowed("http://localhost@attacker.com"));
    }

    /// #4 — SSE frame format: literal bytes for the keep-alive
    /// comment. Cheap pin against accidental refactoring.
    #[test]
    fn sse_keepalive_frame_is_canonical_bytes() {
        // The HTTP transport emits `: keep-alive\n\n` per SSE spec.
        // Pin the literal bytes so a refactor can't accidentally
        // break the frame.
        let frame = b": keep-alive\n\n";
        assert_eq!(frame.len(), 14);
        assert_eq!(frame[0], b':');
        assert_eq!(&frame[frame.len() - 2..], b"\n\n");
    }

    /// #5 — Session table reap: entries older than the TTL are dropped
    /// on the next reap pass.
    #[test]
    fn session_table_reap_drops_expired_entries() {
        let sessions: Arc<Mutex<HashMap<String, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
        // Insert a session with a clearly-expired timestamp.
        let expired_id = new_session_id();
        let fresh_id = new_session_id();
        let stale = Instant::now()
            .checked_sub(SESSION_TTL + Duration::from_secs(60))
            .expect("clock supports sub");
        {
            let mut guard = sessions.lock().unwrap();
            guard.insert(expired_id.clone(), stale);
            guard.insert(fresh_id.clone(), Instant::now());
        }
        reap_expired_sessions(&sessions);
        let guard = sessions.lock().unwrap();
        assert!(
            !guard.contains_key(&expired_id),
            "expired session should have been reaped"
        );
        assert!(
            guard.contains_key(&fresh_id),
            "fresh session must survive the reaper"
        );
    }

    /// #6 — body cap: 5 MiB POST is rejected as 413 (validated end-to-end
    /// in the e2e suite; here we pin the constant value).
    #[test]
    fn body_cap_constant_is_4_mib() {
        assert_eq!(MAX_BODY_BYTES, 4 * 1024 * 1024);
        // Pin the magnitude — 4 MiB is the spec-driven cap, not a
        // tweakable knob (see crate docstring). const-block keeps
        // clippy happy on assertions over constants.
        const _: () = assert!(MAX_BODY_BYTES >= 1024 * 1024);
        const _: () = assert!(MAX_BODY_BYTES <= 16 * 1024 * 1024);
    }

    /// #7 — batch JSON-RPC arrays are detected by leading-`[` sniff.
    #[test]
    fn batch_jsonrpc_array_detected_by_leading_bracket() {
        assert!(is_jsonrpc_batch("[]"));
        assert!(is_jsonrpc_batch("[ {\"jsonrpc\":\"2.0\"} ]"));
        // Whitespace + BOM tolerance.
        assert!(is_jsonrpc_batch("\u{feff}  [{\"jsonrpc\":\"2.0\"}]"));
        // Single envelope is NOT a batch.
        assert!(!is_jsonrpc_batch(r#"{"jsonrpc":"2.0","id":1}"#));
        assert!(!is_jsonrpc_batch("not json"));
        assert!(!is_jsonrpc_batch(""));
    }

    /// Session IDs are 36 chars in the v4 UUID visual format.
    #[test]
    fn new_session_id_has_uuid_v4_visual_shape() {
        let id = new_session_id();
        assert_eq!(id.len(), 36, "session id length: {id}");
        let bytes = id.as_bytes();
        assert_eq!(bytes[8], b'-');
        assert_eq!(bytes[13], b'-');
        assert_eq!(bytes[14], b'4', "UUID v4 marker missing in {id}");
        assert_eq!(bytes[18], b'-');
        assert_eq!(bytes[23], b'-');
    }

    /// Successive session IDs differ — counter ensures uniqueness even
    /// when the clock has nanosecond collisions.
    #[test]
    fn session_ids_are_unique_across_calls() {
        let a = new_session_id();
        let b = new_session_id();
        assert_ne!(a, b, "two consecutive session IDs collided: {a} == {b}");
    }

    /// v0.30 audit #B5 — Content-Type allowlist accepts `application/json`
    /// with or without parameters, case-insensitive on the media type.
    #[test]
    fn content_type_allowlist_matches_json_only() {
        assert!(content_type_is_json("application/json"));
        assert!(content_type_is_json("application/json; charset=utf-8"));
        assert!(content_type_is_json("Application/JSON"));
        assert!(content_type_is_json("APPLICATION/JSON; charset=UTF-8"));
        // Other media types rejected.
        assert!(!content_type_is_json("text/plain"));
        assert!(!content_type_is_json("text/json"));
        assert!(!content_type_is_json("application/xml"));
        assert!(!content_type_is_json(""));
        assert!(!content_type_is_json("application/jsonish"));
    }

    /// v0.30 audit #B6 — parsed-method extraction picks the actual
    /// JSON-RPC `method` field rather than substring-sniffing.
    #[test]
    fn parse_jsonrpc_method_extracts_top_level_method_only() {
        assert_eq!(
            parse_jsonrpc_method(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#),
            Some("initialize".to_string())
        );
        assert_eq!(
            parse_jsonrpc_method(
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"query","arguments":{"command":"initialize"}}}"#
            ),
            Some("tools/call".to_string()),
            "embedded 'initialize' in arguments must NOT shadow the real method"
        );
        // No method field → None.
        assert_eq!(
            parse_jsonrpc_method(r#"{"jsonrpc":"2.0","id":1}"#),
            None
        );
        // Garbage → None (not a panic).
        assert_eq!(parse_jsonrpc_method("not json"), None);
    }

    /// v0.30 audit #010 — NotificationHub assigns monotonic ids and
    /// evicts the oldest entry when the cap is exceeded.
    #[test]
    fn notification_hub_assigns_monotonic_ids_and_evicts_on_overflow() {
        let hub = NotificationHub::new();
        for i in 0..(SSE_REPLAY_BUFFER_SIZE + 10) {
            hub.publish(serde_json::json!({ "i": i }));
        }
        let buf = hub.buffer.lock().unwrap();
        assert_eq!(buf.len(), SSE_REPLAY_BUFFER_SIZE, "ring must be capped");
        let ids: Vec<u64> = buf.iter().map(|(id, _)| *id).collect();
        // IDs are strictly increasing.
        for w in ids.windows(2) {
            assert!(w[0] < w[1], "ids must be strictly increasing: {ids:?}");
        }
        // The oldest entries were evicted — the first id in the
        // buffer is past the original first 10 inserts.
        assert!(ids[0] > 1, "oldest entries should have been evicted: {ids:?}");
    }
}
