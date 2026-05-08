//! Stdio MCP transport — one JSON-RPC envelope per line on stdin,
//! one response (or none, for notifications) on stdout. Stderr is
//! reserved for server-side logs.
//!
//! Lifted out of `crate::server` in v0.21.1 so the HTTP/SSE transport
//! could share the same `McpHandler::handle_line` dispatcher without
//! the stdio loop staying tangled with the JSON-RPC core. See
//! [`crate::transport`] for the multi-transport overview. Behavior is
//! byte-identical to the v0.21.0 stdio loop and pinned by a golden
//! fixture in `crates/coral-mcp/tests/mcp_stdio_golden.rs`.

use crate::server::McpHandler;
use std::io::{BufRead, Write};

/// Run the stdio loop. Reads one JSON-RPC message per line until
/// stdin closes. Each response is written to stdout followed by a
/// newline. Notifications (requests with no `id`) are dispatched for
/// side effects but no response is emitted to stdout, per JSON-RPC
/// 2.0 §4.1.
pub fn serve_stdio(handler: &McpHandler) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        // v0.19.6 audit M3: `handle_line` returns `Option<…>` —
        // `None` for JSON-RPC notifications (no `id` field). Skip
        // emitting anything for notifications so we don't confuse
        // strict JSON-RPC clients that don't expect a response.
        if let Some(response) = handler.handle_line(&line) {
            let serialized = serde_json::to_string(&response).unwrap_or_else(|_| "{}".into());
            writeln!(handle, "{serialized}")?;
            handle.flush()?;
        }
    }
    Ok(())
}
