---
title: "mcp: HTTP/SSE transport advertises subscribe: true but GET /mcp never drains notifications nor honors Last-Event-ID — HTTP clients see no push events"
severity: Medium
labels: bug, mcp, http-transport
confidence: 5
cross_validated_by: [mcp-audit-agent]
---

## Summary

The MCP server advertises `resources: { listChanged: true, subscribe:
true }` at `crates/coral-mcp/src/server.rs:287`. Notifications enqueued
via `notification_tx` (`server.rs:564`, `watcher.rs:91`) are correctly
drained by the stdio transport (`crates/coral-mcp/src/transport/stdio.rs:55-62`)
and emitted to the client.

The HTTP/SSE transport (`crates/coral-mcp/src/transport/http_sse.rs`)
does not:

1. `handle_get_sse` at lines 384-420 opens an SSE stream that emits
   only `: keep-alive` comments. No `data:` frames are ever pushed.
2. `Last-Event-ID` header is ignored (no resume buffer).
3. No event-ID emission, so resume couldn't work even if a buffer
   existed.
4. `notification_tx` queues are not drained into the SSE stream.

A client that listens on `GET /mcp` after `POST /mcp` with
`initialize`+`resources/subscribe` will never receive
`notifications/resources/list_changed`. The HTTP/SSE transport is
effectively a heartbeat-only no-op for push notifications, despite
advertising the capability.

## Why it matters

The HTTP/SSE transport is described in the MCP 2025-11-25 spec
("Streamable HTTP") as the network analogue of stdio. Coding agents
that prefer HTTP over stdio (e.g. running Coral as a sidecar
container) will silently lose the entire push-notification channel,
defeating subscriptions and forcing polling by the client.

Compounds with finding #002 (WikiState stale): even if WikiState were
wired up, HTTP clients still wouldn't be told to re-fetch.

## Suggested fix

Wire `notification_rx` into the SSE response stream. On `GET /mcp`,
acquire a per-connection subscriber from a tokio/std broadcast
channel (or a `crossbeam_channel::Receiver` cloned per connection),
emit `data: <json>\n\n` per notification. Optionally implement
`Last-Event-ID` by keeping a bounded VecDeque of last N events.

Until that lands, either:

- Emit a warning at startup when `--transport http` is selected to
  set expectations, OR
- Set `listChanged: false, subscribe: false` in capabilities when
  serving over HTTP so honest clients don't subscribe.

Add an integration test: spawn the HTTP transport, POST `initialize`
+ `resources/subscribe`, open a GET /mcp stream, write to the wiki
from another thread, assert the SSE stream emits a
`notifications/resources/list_changed` frame.

## Cross-validation

MCP agent flagged this. Direct verification not performed in this
audit but the agent's file:line citations (`http_sse.rs:384-420`,
the absence of `notification_rx` references) are consistent with the
SSE-only-keep-alive shape described.
