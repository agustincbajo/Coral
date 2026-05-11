//! Stdio MCP transport — one JSON-RPC envelope per line on stdin,
//! one response (or none, for notifications) on stdout. Stderr is
//! reserved for server-side logs.
//!
//! Lifted out of `crate::server` in v0.21.1 so the HTTP/SSE transport
//! could share the same `McpHandler::handle_line` dispatcher without
//! the stdio loop staying tangled with the JSON-RPC core. See
//! [`crate::transport`] for the multi-transport overview.
//!
//! v0.24: the loop now also drains a notification channel after each
//! request-response cycle. This enables MCP resource subscriptions —
//! the server can push `notifications/resources/updated` and
//! `notifications/resources/list_changed` to the client without waiting
//! for a request.

use crate::server::McpHandler;
use std::io::{BufRead, Write};
use std::sync::mpsc;

/// Run the stdio loop. Reads one JSON-RPC message per line until
/// stdin closes. Each response is written to stdout followed by a
/// newline. Notifications (requests with no `id`) are dispatched for
/// side effects but no response is emitted to stdout, per JSON-RPC
/// 2.0 §4.1.
///
/// v0.24: after writing each response, the loop drains any pending
/// push notifications that were enqueued by `notify_resource_updated`
/// or `notify_resources_list_changed`. This gives the MCP client a
/// chance to see resource-change events interleaved with normal
/// responses on the same stdio stream.
pub fn serve_stdio(handler: &McpHandler) -> std::io::Result<()> {
    let (tx, rx) = mpsc::channel();
    handler.set_notification_sender(tx);

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
        // Drain any pending push notifications enqueued during
        // handler dispatch (e.g. resource subscription updates).
        while let Ok(notification) = rx.try_recv() {
            let serialized =
                serde_json::to_string(&notification).unwrap_or_else(|_| "{}".into());
            writeln!(handle, "{serialized}")?;
            handle.flush()?;
        }
    }
    Ok(())
}
