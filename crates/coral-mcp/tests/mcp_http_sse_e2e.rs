//! End-to-end tests for the v0.21.1 HTTP/SSE MCP transport.
//!
//! Spec: orchestrator §6 "Phase 4 — Tests" lists 9 integration tests
//! (#8-#16) plus the adversarial trio (#18-#20). All driven via raw
//! `std::net::TcpStream` so the test crate doesn't need a `tiny_http`
//! dep — the wire format is HTTP/1.1 plain text and small enough to
//! hand-craft. Each test binds to port 0 and reads the assigned port
//! back from `HttpSseTransport::local_addr` so they run in parallel
//! without contention.
//!
//! Common pattern:
//! 1. `spawn_server()` — bind a fresh transport on port 0, spawn the
//!    serve loop on a dedicated thread, return the local addr.
//! 2. send a hand-crafted HTTP/1.1 request via TcpStream.
//! 3. parse status + headers + body via `parse_response`.
//!
//! Each test uses a unique port via the OS-assigned port-0 trick;
//! the `serve_blocking` thread leaks on test exit, which is fine for
//! a unit-test binary.

use coral_mcp::transport::HttpSseTransport;
use coral_mcp::{McpHandler, NoOpDispatcher, ServerConfig, Transport, WikiResourceProvider};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

fn make_handler() -> Arc<McpHandler> {
    let cfg = ServerConfig {
        transport: Transport::HttpSse,
        read_only: true,
        allow_write_tools: false,
        port: None,
        bind_addr: None,
        allow_experimental_tasks: false,
    };
    let resources = Arc::new(WikiResourceProvider::new(std::path::PathBuf::from("/tmp")));
    let tools = Arc::new(NoOpDispatcher);
    Arc::new(McpHandler::new(cfg, resources, tools))
}

fn make_handler_with(allow_write: bool) -> Arc<McpHandler> {
    let cfg = ServerConfig {
        transport: Transport::HttpSse,
        read_only: !allow_write,
        allow_write_tools: allow_write,
        port: None,
        bind_addr: None,
        allow_experimental_tasks: false,
    };
    let resources = Arc::new(WikiResourceProvider::new(std::path::PathBuf::from("/tmp")));
    let tools = Arc::new(NoOpDispatcher);
    Arc::new(McpHandler::new(cfg, resources, tools))
}

/// Bind a fresh server on 127.0.0.1:0, return the assigned addr.
/// The serve loop runs on a leaked thread.
fn spawn_server(handler: Arc<McpHandler>) -> SocketAddr {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let transport = HttpSseTransport::bind(addr, handler).expect("bind");
    let local = transport.local_addr().expect("local_addr");
    std::thread::spawn(move || {
        let _ = transport.serve_blocking();
    });
    // Give the OS a moment to fully wire the listener.
    std::thread::sleep(Duration::from_millis(20));
    local
}

/// Send a raw HTTP request and return (status_code, headers, body).
fn send_request(addr: SocketAddr, request: &[u8]) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .expect("write timeout");
    stream.write_all(request).expect("write request");
    stream.flush().expect("flush");
    let mut buf = Vec::with_capacity(8192);
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
    }
    parse_response(&buf)
}

fn parse_response(buf: &[u8]) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let split = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("response missing header/body separator");
    let head = &buf[..split];
    let body = buf[split + 4..].to_vec();
    let head_str = std::str::from_utf8(head).expect("ASCII head");
    let mut lines = head_str.lines();
    let status_line = lines.next().expect("status line");
    let parts: Vec<&str> = status_line.splitn(3, ' ').collect();
    let status: u16 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut headers = Vec::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    (status, headers, body)
}

fn header(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}

/// Build a POST request to /mcp with optional extra headers.
fn build_post(addr: SocketAddr, body: &str, extra_headers: &[&str]) -> Vec<u8> {
    let mut req = format!(
        "POST /mcp HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Content-Type: application/json\r\n\
         Accept: application/json\r\n\
         Content-Length: {}\r\n",
        body.len()
    );
    for h in extra_headers {
        req.push_str(h);
        req.push_str("\r\n");
    }
    req.push_str("Connection: close\r\n\r\n");
    req.push_str(body);
    req.into_bytes()
}

/// #8 — POST initialize round-trip.
#[test]
fn post_initialize_round_trip() {
    let addr = spawn_server(make_handler());
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let req = build_post(addr, body, &[]);
    let (status, headers, body) = send_request(addr, &req);
    assert_eq!(status, 200, "initialize should be 200");
    let session_id = header(&headers, "Mcp-Session-Id").expect("session id header");
    assert_eq!(session_id.len(), 36, "session id should be UUID-shaped");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
    assert_eq!(json["jsonrpc"], "2.0");
    assert_eq!(json["id"], 1);
    assert_eq!(json["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(json["result"]["serverInfo"]["name"], "coral");
}

/// #9 — POST resources/list returns catalog.
#[test]
fn post_resources_list_returns_catalog() {
    let addr = spawn_server(make_handler());
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"resources/list","params":{}}"#;
    let req = build_post(addr, body, &[]);
    let (status, _, resp_body) = send_request(addr, &req);
    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_slice(&resp_body).expect("valid JSON");
    let uris: Vec<&str> = json["result"]["resources"]
        .as_array()
        .expect("resources is array")
        .iter()
        .map(|r| r["uri"].as_str().unwrap_or(""))
        .collect();
    assert!(uris.contains(&"coral://manifest"));
    assert!(uris.contains(&"coral://lock"));
}

/// #10 — POST tools/call write-tool without --allow returns error.
#[test]
fn post_tools_call_write_without_allow_returns_jsonrpc_error() {
    let addr = spawn_server(make_handler());
    let body =
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"up","arguments":{}}}"#;
    let req = build_post(addr, body, &[]);
    let (status, _, resp_body) = send_request(addr, &req);
    // The dispatcher rejection is a JSON-RPC error envelope, not an
    // HTTP 4xx — that's the contract: transport-level errors only for
    // transport-shape problems.
    assert_eq!(status, 200, "JSON-RPC errors flow through 200 body");
    let json: serde_json::Value = serde_json::from_slice(&resp_body).expect("valid JSON");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("--allow-write-tools"),
        "expected --allow-write-tools rejection, got {json}"
    );
}

/// #11 — Mcp-Session-Id pins on initialize, subsequent POST echoes it.
/// (We don't enforce session-id-required after initialize in v0.21.1,
/// per the doc comment in `handle_post`. This test pins the soft
/// contract: initialize mints, the client SHOULD echo, the server
/// touches the entry on subsequent traffic.)
#[test]
fn session_id_minted_on_initialize_and_touched_on_subsequent_posts() {
    let addr = spawn_server(make_handler());
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let req = build_post(addr, body, &[]);
    let (_, headers, _) = send_request(addr, &req);
    let session_id = header(&headers, "Mcp-Session-Id").expect("session id");

    // Subsequent ping with the cookie — server should accept.
    let body2 = r#"{"jsonrpc":"2.0","id":2,"method":"ping","params":{}}"#;
    let session_header = format!("Mcp-Session-Id: {session_id}");
    let req2 = build_post(addr, body2, &[&session_header]);
    let (status2, _, body2) = send_request(addr, &req2);
    assert_eq!(status2, 200);
    let json2: serde_json::Value = serde_json::from_slice(&body2).expect("valid JSON");
    assert_eq!(json2["result"], serde_json::json!({}));
}

/// #12 — DELETE terminates session.
#[test]
fn delete_terminates_session() {
    let addr = spawn_server(make_handler());
    // First, initialize to mint a session.
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let init_req = build_post(addr, body, &[]);
    let (_, headers, _) = send_request(addr, &init_req);
    let session_id = header(&headers, "Mcp-Session-Id").expect("session id");

    // DELETE the session.
    let del_req = format!(
        "DELETE /mcp HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Mcp-Session-Id: {session_id}\r\n\
         Connection: close\r\n\r\n"
    );
    let (status, _, _) = send_request(addr, del_req.as_bytes());
    assert_eq!(status, 204, "DELETE on existing session must be 204");

    // Second DELETE on the same id is 404.
    let (status2, _, _) = send_request(addr, del_req.as_bytes());
    assert_eq!(
        status2, 404,
        "DELETE on already-terminated session must be 404"
    );
}

/// #13 — GET /mcp opens an empty SSE stream + initial keep-alive byte.
/// We close after the first read so we don't hang waiting for the 15s
/// timer.
#[test]
fn get_mcp_returns_sse_stream_with_keepalive_comment() {
    let addr = spawn_server(make_handler());
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("rt");
    let req = format!(
        "GET /mcp HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Accept: text/event-stream\r\n\
         Connection: keep-alive\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).expect("write");
    stream.flush().expect("flush");
    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).expect("read");
    buf.truncate(n);
    let text = String::from_utf8_lossy(&buf);
    assert!(
        text.starts_with("HTTP/1.1 200"),
        "expected 200 status; got: {text:?}"
    );
    assert!(
        text.to_lowercase().contains("text/event-stream"),
        "Content-Type missing text/event-stream: {text}"
    );
    assert!(
        text.contains(": connected"),
        "expected initial SSE comment: {text}"
    );
}

/// #14 — 5 concurrent clients all get responses (no deadlocks).
#[test]
fn five_concurrent_clients_all_succeed() {
    let addr = spawn_server(make_handler());
    let mut handles = Vec::new();
    for i in 0..5 {
        let h = std::thread::spawn(move || {
            let body =
                format!(r#"{{"jsonrpc":"2.0","id":{i},"method":"initialize","params":{{}}}}"#);
            let req = build_post(addr, &body, &[]);
            let (status, _, body) = send_request(addr, &req);
            assert_eq!(status, 200, "client {i} got status {status}");
            let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
            assert_eq!(json["id"], i, "client {i} id mismatch");
        });
        handles.push(h);
    }
    for h in handles {
        h.join().expect("thread join");
    }
}

/// #15 — Malformed JSON-RPC → 200 with JSON-RPC -32700 error envelope.
/// (Transport-level errors only for transport-shape problems; protocol
/// errors flow through the body.)
#[test]
fn malformed_jsonrpc_returns_200_with_minus_32700_envelope() {
    let addr = spawn_server(make_handler());
    let body = "not json at all";
    let req = build_post(addr, body, &[]);
    let (status, _, resp_body) = send_request(addr, &req);
    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_slice(&resp_body).expect("json");
    assert_eq!(json["error"]["code"], -32700);
}

/// #16 — Huge POST → 413, server stays up.
#[test]
fn huge_post_returns_413_server_remains_responsive() {
    let addr = spawn_server(make_handler());
    // 5 MiB of `a` — over the 4 MiB cap.
    let big = "a".repeat(5 * 1024 * 1024);
    let req = build_post(addr, &big, &[]);
    let (status, _, _) = send_request(addr, &req);
    assert_eq!(status, 413, "5 MiB POST must be 413");

    // Server must still respond to a small request.
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}"#;
    let req2 = build_post(addr, body, &[]);
    let (status2, _, _) = send_request(addr, &req2);
    assert_eq!(status2, 200, "server must remain responsive after 413");
}

/// #18 (adversarial) — Origin with a homoglyph-style ASCII spoof
/// (no UTF-8 in the header — tiny_http's header parser is strict
/// ASCII-only and would reject non-ASCII bytes outright before our
/// allowlist runs). The realistic threat is an attacker that
/// registers a legitimate-looking ASCII domain — `localhost.attacker
/// .com` or punycode-rendered ASCII — and tricks the user into
/// clicking. Pin the allowlist behavior here.
#[test]
fn homoglyph_style_ascii_origin_is_blocked() {
    let addr = spawn_server(make_handler());
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    // Punycode ASCII rendering of the Cyrillic-l-o-c-alhost
    // domain — what a browser sends after IDN normalization. The
    // allowlist matches the literal host segment, so any non-
    // localhost host fails closed.
    let cases = [
        "Origin: http://xn--lcalhost-5cf.attacker.com",
        "Origin: http://localhost.attacker.com",
        "Origin: http://127.0.0.1.attacker.com",
    ];
    for origin in cases {
        let req = build_post(addr, body, &[origin]);
        let (status, _, body) = send_request(addr, &req);
        assert_eq!(
            status, 403,
            "homoglyph-style origin {origin:?} must be rejected; got {status}"
        );
        let body_str = String::from_utf8_lossy(&body);
        assert!(
            body_str.contains("forbidden Origin"),
            "expected forbidden-origin message for {origin:?}: {body_str}"
        );
    }
}

/// #19 (adversarial) — bind to already-bound port → friendly error.
#[test]
fn bind_to_already_bound_port_returns_friendly_error() {
    // Bind once.
    let addr_zero = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let first = HttpSseTransport::bind(addr_zero, make_handler()).expect("first bind");
    let local = first.local_addr().expect("local addr");
    std::thread::spawn(move || {
        let _ = first.serve_blocking();
    });
    std::thread::sleep(Duration::from_millis(20));

    // Try to bind a second server to the same port — must return
    // an `io::Error` with a friendly message.
    let result = HttpSseTransport::bind(local, make_handler());
    let err = match result {
        Ok(_) => panic!("second bind should fail"),
        Err(e) => e,
    };
    let msg = err.to_string();
    assert!(
        msg.contains("could not bind") && msg.contains(&local.to_string()),
        "expected friendly bind error mentioning the port {local}: {msg}"
    );
}

/// #17 (CLI smoke duplicate) — direct allow-write dispatch reaches the
/// dispatcher when the gate is open. Sanity that the `read_only` /
/// `allow_write_tools` matrix flows through the HTTP transport, not
/// just stdio.
#[test]
fn allow_write_tools_gate_reaches_dispatcher_over_http() {
    let addr = spawn_server(make_handler_with(true));
    let body =
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"up","arguments":{}}}"#;
    let req = build_post(addr, body, &[]);
    let (status, _, resp_body) = send_request(addr, &req);
    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_slice(&resp_body).expect("json");
    // No JSON-RPC error — the call passed the gate. Dispatcher
    // returns Skip (NoOpDispatcher) but that's an Ok result.
    assert!(
        json["error"].is_null(),
        "write tool with --allow-write-tools must reach dispatcher: {json}"
    );
}

/// Edge case — Accept header with only `text/plain` returns 406.
#[test]
fn accept_text_plain_only_returns_406() {
    let addr = spawn_server(make_handler());
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    // We need to override Accept manually; build_post hardcodes
    // application/json. Build by hand.
    let req = format!(
        "POST /mcp HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Content-Type: application/json\r\n\
         Accept: text/plain\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    let (status, _, _) = send_request(addr, req.as_bytes());
    assert_eq!(status, 406, "text/plain only must yield 406");
}

/// Edge case — OPTIONS preflight succeeds with CORS headers.
#[test]
fn options_preflight_returns_200_with_cors_headers() {
    let addr = spawn_server(make_handler());
    let req = format!(
        "OPTIONS /mcp HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Origin: http://localhost:3000\r\n\
         Access-Control-Request-Method: POST\r\n\
         Connection: close\r\n\r\n"
    );
    let (status, headers, _) = send_request(addr, req.as_bytes());
    assert_eq!(status, 200);
    let allow_methods = header(&headers, "Access-Control-Allow-Methods").unwrap_or_default();
    assert!(allow_methods.contains("POST"));
    assert!(allow_methods.contains("DELETE"));
    let allow_origin = header(&headers, "Access-Control-Allow-Origin").unwrap_or_default();
    assert!(
        allow_origin.contains("localhost"),
        "expected localhost in Access-Control-Allow-Origin: {allow_origin}"
    );
}

/// Edge case — unknown path returns 404 with JSON body.
#[test]
fn unknown_path_returns_404() {
    let addr = spawn_server(make_handler());
    let req = format!(
        "GET /not-mcp HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Accept: application/json\r\n\
         Connection: close\r\n\r\n"
    );
    let (status, _, _) = send_request(addr, req.as_bytes());
    assert_eq!(status, 404);
}

/// v0.22.5 — `GET /.well-known/mcp/server-card.json` returns 200 with
/// valid JSON matching the spec D1 schema. The card is public — no
/// Origin allowlist, no Mcp-Session-Id requirement — so this test
/// deliberately omits both. Acceptance criteria #1, #2, #3, #4 all
/// covered here.
#[test]
fn well_known_card_endpoint_returns_200_with_valid_json() {
    let addr = spawn_server(make_handler());
    let req = format!(
        "GET /.well-known/mcp/server-card.json HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Accept: application/json\r\n\
         Connection: close\r\n\r\n"
    );
    let (status, headers, body) = send_request(addr, req.as_bytes());
    assert_eq!(status, 200, "card endpoint must be 200");
    let content_type = header(&headers, "Content-Type").unwrap_or_default();
    assert!(
        content_type.starts_with("application/json"),
        "Content-Type must be application/json, got: {content_type:?}"
    );
    let json: serde_json::Value =
        serde_json::from_slice(&body).expect("card body must be valid JSON");
    // AC #3: name + version + protocolVersion top-level fields.
    assert_eq!(json["name"], "coral");
    assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(json["protocolVersion"], "2025-11-25");
    // AC #4: capability counts match the catalog .len(). We use
    // ToolCatalog::all().len() because the card reports the FULL
    // catalog independent of --allow-write-tools.
    let tools_count = json["capabilities"]["tools"]["count"]
        .as_u64()
        .expect("tools.count is integer");
    assert_eq!(
        tools_count as usize,
        coral_mcp::ToolCatalog::all().len(),
        "card tools count must equal ToolCatalog::all().len()"
    );
    let prompts_count = json["capabilities"]["prompts"]["count"]
        .as_u64()
        .expect("prompts.count is integer");
    assert_eq!(
        prompts_count as usize,
        coral_mcp::PromptCatalog::list().len()
    );
    // AC #8: the endpoint accepts cross-origin GETs. Re-issue with a
    // disallowed Origin and confirm it still 200s. Without the new
    // route shape this would 403 because the legacy `/mcp` Origin
    // allowlist would have fired.
    let req2 = format!(
        "GET /.well-known/mcp/server-card.json HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Origin: https://attacker.example.com\r\n\
         Accept: application/json\r\n\
         Connection: close\r\n\r\n"
    );
    let (status2, _, _) = send_request(addr, req2.as_bytes());
    assert_eq!(
        status2, 200,
        "card endpoint must accept cross-origin GETs (no Origin allowlist on the public card)"
    );
}

/// v0.30 audit #B5 — POST with non-JSON Content-Type is rejected as
/// 415 Unsupported Media Type. The MCP "Streamable HTTP" spec
/// requires `application/json` on POST; anything else is a
/// transport-shape error.
#[test]
fn post_with_text_plain_content_type_returns_415() {
    let addr = spawn_server(make_handler());
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let req = format!(
        "POST /mcp HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Content-Type: text/plain\r\n\
         Accept: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    let (status, _, resp_body) = send_request(addr, req.as_bytes());
    assert_eq!(status, 415, "text/plain Content-Type must be 415");
    let json: serde_json::Value =
        serde_json::from_slice(&resp_body).expect("415 body must be JSON");
    assert!(
        json["error"]
            .as_str()
            .unwrap_or("")
            .contains("application/json"),
        "415 body should mention application/json: {json}"
    );
}

/// v0.30 audit #B5 — POST with `application/json; charset=utf-8` is
/// accepted (RFC 7231 allows the charset parameter).
#[test]
fn post_with_json_content_type_and_charset_param_is_accepted() {
    let addr = spawn_server(make_handler());
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let req = format!(
        "POST /mcp HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Content-Type: application/json; charset=utf-8\r\n\
         Accept: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    let (status, _, _) = send_request(addr, req.as_bytes());
    assert_eq!(
        status, 200,
        "application/json with charset must be accepted"
    );
}

/// v0.30 audit #B5 — POST without any Content-Type header is rejected
/// as 415 (the MCP spec requires the header to be present).
#[test]
fn post_with_missing_content_type_returns_415() {
    let addr = spawn_server(make_handler());
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let req = format!(
        "POST /mcp HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Accept: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    let (status, _, _) = send_request(addr, req.as_bytes());
    assert_eq!(status, 415, "missing Content-Type must be 415");
}

/// v0.30 audit #B6 — a `tools/call` whose arguments contain the
/// literal token `"initialize"` MUST NOT mint a new session. Pre-fix
/// the substring sniff false-positived here, churning new session
/// IDs on every tool call that happened to mention the word.
#[test]
fn tools_call_with_initialize_in_arguments_does_not_mint_session() {
    let addr = spawn_server(make_handler());
    // Arguments embed `"command":"initialize"` — the OLD sniff would
    // match `"initialize"` and mint a session. Post-fix the parsed
    // `method` field is `tools/call`, so no session header on the
    // response.
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"query","arguments":{"command":"initialize"}}}"#;
    let req = build_post(addr, body, &[]);
    let (status, headers, _) = send_request(addr, &req);
    assert_eq!(status, 200);
    assert!(
        header(&headers, "Mcp-Session-Id").is_none(),
        "tools/call with 'initialize' in args must NOT mint a session header: {headers:?}"
    );
}

/// v0.30 audit #010 — GET /mcp streams notifications pushed through
/// `notify_resources_list_changed`. The dispatcher thread spawned in
/// `bind()` re-publishes mpsc events into the shared replay ring;
/// GET /mcp drains them. Pre-fix the GET stream was keep-alive-only.
#[test]
fn get_mcp_emits_sse_frame_for_pushed_notification() {
    let handler = make_handler();
    let addr = spawn_server(Arc::clone(&handler));
    // Subscribe so the notify_resource_updated path also fires for
    // an exact URI match (resources_list_changed is unconditional,
    // we use it for simplicity).
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("rt");
    let req = format!(
        "GET /mcp HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Accept: text/event-stream\r\n\
         Connection: keep-alive\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).expect("write");
    stream.flush().expect("flush");
    // Read the initial chunk (HTTP head + ': connected\n\n').
    let mut prelude = vec![0u8; 1024];
    let n = stream.read(&mut prelude).expect("initial read");
    prelude.truncate(n);
    let text = String::from_utf8_lossy(&prelude);
    assert!(
        text.starts_with("HTTP/1.1 200") && text.contains(": connected"),
        "expected SSE prelude: {text}"
    );

    // Push a notification through the handler — the dispatcher
    // thread inside HttpSseTransport::bind republishes it into the
    // shared replay ring, which our GET stream drains.
    handler.notify_resources_list_changed();

    // Read until we see a `data:` frame containing the method name.
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).expect("notification read");
    buf.truncate(n);
    let text = String::from_utf8_lossy(&buf);
    assert!(text.contains("data:"), "expected SSE data frame: {text:?}");
    assert!(
        text.contains("notifications/resources/list_changed"),
        "SSE frame must carry the list_changed notification: {text:?}"
    );
    // Frame must carry an `id:` line so clients can resume via
    // Last-Event-ID.
    assert!(text.contains("id:"), "SSE frame must include id: {text:?}");
}

/// v0.22.5 — anything else under `/.well-known/mcp/*` (or a non-GET
/// method on the card path) returns 404. Pins the contract that the
/// card is the only well-known resource we publish, and that the new
/// branch doesn't accidentally shadow `/mcp` traffic. Acceptance
/// criterion #7.
#[test]
fn well_known_unknown_path_returns_404() {
    let addr = spawn_server(make_handler());
    // 1. Unknown sibling under /.well-known/mcp/ returns 404.
    let req = format!(
        "GET /.well-known/mcp/something-else HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Accept: application/json\r\n\
         Connection: close\r\n\r\n"
    );
    let (status, _, _) = send_request(addr, req.as_bytes());
    assert_eq!(
        status, 404,
        "unknown well-known path must be 404; got {status}"
    );
    // 2. Non-GET on the card path is 404 (we don't accept POST/DELETE
    //    on the card — it's a static read-only resource).
    let req2 = format!(
        "POST /.well-known/mcp/server-card.json HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Accept: application/json\r\n\
         Content-Length: 0\r\n\
         Connection: close\r\n\r\n"
    );
    let (status2, _, _) = send_request(addr, req2.as_bytes());
    assert_eq!(
        status2, 404,
        "POST on /.well-known/mcp/server-card.json must be 404; got {status2}"
    );
    // 3. The card route MUST NOT have shadowed /mcp — sanity check the
    //    real endpoint still works after the new branch lands.
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}"#;
    let mcp_req = build_post(addr, body, &[]);
    let (mcp_status, _, _) = send_request(addr, &mcp_req);
    assert_eq!(
        mcp_status, 200,
        "/mcp must still respond 200 after well-known route landed"
    );
}

// ─── v0.35 SEC-01 / CP-3 / CP-5 — bearer-auth e2e tests ───────────────────

use coral_mcp::transport::AuthConfig;

/// Bind a server with bearer auth enabled. Mirrors `spawn_server` but
/// goes through `HttpSseTransport::bind_with_auth` so the dispatcher
/// enforces the token on every `/mcp` request.
fn spawn_auth_server(handler: Arc<McpHandler>, token: &str) -> SocketAddr {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let auth = AuthConfig {
        expected_token: Some(token.to_string()),
        bind_label: "127.0.0.1".to_string(),
    };
    let transport = coral_mcp::transport::HttpSseTransport::bind_with_auth(addr, handler, auth)
        .expect("bind_with_auth");
    let local = transport.local_addr().expect("local_addr");
    std::thread::spawn(move || {
        let _ = transport.serve_blocking();
    });
    std::thread::sleep(Duration::from_millis(20));
    local
}

/// SEC-01 #1 — POST without `Authorization` header → 401.
#[test]
fn sec01_post_without_token_returns_401() {
    let token = "test-token-abc";
    let addr = spawn_auth_server(make_handler(), token);
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let req = build_post(addr, body, &[]);
    let (status, _, resp_body) = send_request(addr, &req);
    assert_eq!(
        status,
        401,
        "POST without bearer must be 401, got body={}",
        String::from_utf8_lossy(&resp_body)
    );
    let json: serde_json::Value = serde_json::from_slice(&resp_body).expect("valid JSON");
    assert!(
        json["error"]
            .as_str()
            .unwrap_or("")
            .contains("unauthorized"),
        "401 body should mention unauthorized: {json}"
    );
}

/// SEC-01 #2 — POST with the WRONG bearer → 401.
#[test]
fn sec01_post_with_wrong_token_returns_401() {
    let addr = spawn_auth_server(make_handler(), "secret-token");
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let req = build_post(addr, body, &["Authorization: Bearer wrong-token"]);
    let (status, _, _) = send_request(addr, &req);
    assert_eq!(status, 401, "wrong bearer must be 401");
}

/// SEC-01 #3 — POST with the WRONG header shape (Basic auth) → 401.
#[test]
fn sec01_post_with_basic_auth_returns_401() {
    let addr = spawn_auth_server(make_handler(), "secret-token");
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let req = build_post(addr, body, &["Authorization: Basic dXNlcjpwYXNz"]);
    let (status, _, _) = send_request(addr, &req);
    assert_eq!(status, 401, "Basic auth must be 401 (we require Bearer)");
}

/// SEC-01 #4 — POST with the CORRECT bearer → 200, session id minted.
#[test]
fn sec01_post_with_correct_token_returns_200_and_mints_session() {
    let token = "the-real-token";
    let addr = spawn_auth_server(make_handler(), token);
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let header_line = format!("Authorization: Bearer {token}");
    let req = build_post(addr, body, &[&header_line]);
    let (status, headers, resp_body) = send_request(addr, &req);
    assert_eq!(status, 200, "valid bearer must be 200");
    assert!(
        header(&headers, "Mcp-Session-Id").is_some(),
        "initialize must still mint a session id post-auth"
    );
    let json: serde_json::Value = serde_json::from_slice(&resp_body).expect("valid JSON");
    assert_eq!(json["result"]["protocolVersion"], "2025-11-25");
}

/// SEC-01 #5 — POST with the bearer in lowercase `bearer` casing also
/// accepted (matches RFC 6750 and the coral-ui pattern).
#[test]
fn sec01_post_with_lowercase_bearer_also_accepted() {
    let token = "lower-case-token";
    let addr = spawn_auth_server(make_handler(), token);
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let header_line = format!("Authorization: bearer {token}");
    let req = build_post(addr, body, &[&header_line]);
    let (status, _, _) = send_request(addr, &req);
    assert_eq!(status, 200, "lowercase bearer must be 200");
}

/// SEC-01 #6 — GET /mcp (SSE) without bearer → 401.
#[test]
fn sec01_get_sse_without_token_returns_401() {
    let addr = spawn_auth_server(make_handler(), "secret-token");
    let req = format!(
        "GET /mcp HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Accept: text/event-stream\r\n\
         Connection: close\r\n\r\n",
    );
    let (status, _, _) = send_request(addr, req.as_bytes());
    assert_eq!(status, 401, "SSE GET without bearer must be 401");
}

/// SEC-01 #7 — DELETE /mcp without bearer → 401.
#[test]
fn sec01_delete_without_token_returns_401() {
    let addr = spawn_auth_server(make_handler(), "secret-token");
    let req = format!(
        "DELETE /mcp HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Mcp-Session-Id: some-id\r\n\
         Connection: close\r\n\r\n",
    );
    let (status, _, _) = send_request(addr, req.as_bytes());
    assert_eq!(status, 401, "DELETE without bearer must be 401");
}

/// SEC-01 #8 — the `/.well-known/mcp/server-card.json` discovery
/// endpoint MUST stay public even when bearer auth is enabled. Pin
/// the carve-out so a future refactor doesn't accidentally hide it
/// behind the gate (registries probe cross-origin and a 401 there
/// would defeat the point of "discoverable").
#[test]
fn sec01_well_known_card_stays_public_when_auth_enabled() {
    let addr = spawn_auth_server(make_handler(), "secret-token");
    let req = format!(
        "GET /.well-known/mcp/server-card.json HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Connection: close\r\n\r\n",
    );
    let (status, _, body) = send_request(addr, req.as_bytes());
    assert_eq!(
        status,
        200,
        "well-known card must remain public; got body={}",
        String::from_utf8_lossy(&body)
    );
    // Sanity: the body is valid JSON.
    serde_json::from_slice::<serde_json::Value>(&body).expect("card body must be JSON");
}

/// SEC-01 #9 — CORS preflight (OPTIONS) MUST bypass the bearer check
/// so browsers can probe the allow-headers list before they're able to
/// supply the Authorization header on the real request.
#[test]
fn sec01_options_preflight_bypasses_bearer() {
    let addr = spawn_auth_server(make_handler(), "secret-token");
    let req = format!(
        "OPTIONS /mcp HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Origin: http://localhost\r\n\
         Access-Control-Request-Method: POST\r\n\
         Access-Control-Request-Headers: Authorization, Content-Type\r\n\
         Connection: close\r\n\r\n",
    );
    let (status, headers, _) = send_request(addr, req.as_bytes());
    assert_eq!(status, 200, "OPTIONS preflight must succeed without bearer");
    let allow_headers = header(&headers, "Access-Control-Allow-Headers").unwrap_or_default();
    // The allow-headers list must include something the real request
    // will need; pin the contract here so an accidental tightening of
    // the CORS reply doesn't silently lock out browser clients.
    assert!(
        allow_headers.contains("Content-Type") || allow_headers.contains("content-type"),
        "preflight should advertise Content-Type: {allow_headers}"
    );
}

/// SEC-01 #10 — `bind_with_auth` REFUSES a non-loopback bind without a
/// token (defense-in-depth — `serve()` should catch this at startup,
/// but the transport-layer guard is what actually closes the gap if a
/// library consumer instantiates `HttpSseTransport` directly).
#[test]
fn sec01_bind_with_auth_rejects_external_bind_without_token() {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let auth = AuthConfig {
        expected_token: None,
        bind_label: "0.0.0.0".to_string(),
    };
    let result = coral_mcp::transport::HttpSseTransport::bind_with_auth(addr, make_handler(), auth);
    assert!(
        result.is_err(),
        "non-loopback bind without token must error at bind time"
    );
    let msg = result.err().unwrap().to_string();
    assert!(
        msg.contains("non-loopback") || msg.contains("0.0.0.0"),
        "error message should explain the bind/token mismatch: {msg}"
    );
}

/// SEC-01 #11 / Q-C1 composition — after a malformed pre-auth payload
/// that previously could panic a handler, the server stays responsive
/// to legitimate requests. Pre-fix, the std-mutex poisoning from any
/// handler panic would kill every subsequent request. With both the
/// parking_lot migration AND the pre-auth early-401 in place, an
/// unauthenticated nonsense payload should be a no-op for the rest of
/// the dispatch table.
#[test]
fn sec01_unauth_garbage_does_not_break_subsequent_auth_requests() {
    let token = "real-token";
    let addr = spawn_auth_server(make_handler(), token);

    // Round 1: garbage POST without bearer. Body is intentionally
    // invalid (would have tripped the JSON parser pre-auth had it
    // been allowed in); the early 401 means it never reaches the
    // dispatcher.
    let garbage = "\u{0}\u{0}\u{0}\u{0}NOT-JSON";
    let req1 = build_post(addr, garbage, &[]);
    let (status1, _, _) = send_request(addr, &req1);
    assert_eq!(status1, 401, "round 1 garbage must be 401");

    // Round 2: a legit authed request right after.
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let header_line = format!("Authorization: Bearer {token}");
    let req2 = build_post(addr, body, &[&header_line]);
    let (status2, _, _) = send_request(addr, &req2);
    assert_eq!(
        status2, 200,
        "round 2 must succeed — auth + parking_lot together close the pre-auth DoS"
    );
}
