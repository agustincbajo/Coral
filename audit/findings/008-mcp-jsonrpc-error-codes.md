---
title: "mcp: every non-parse, non-version error returns JSON-RPC code -32601 (Method not found) regardless of cause — violates JSON-RPC 2.0 §5.1"
severity: Medium
labels: bug, mcp, spec-conformance
confidence: 5
cross_validated_by: [mcp-audit-agent, direct-code-read]
---

## Summary

`crates/coral-mcp/src/server.rs:277-280`:

```rust
Some(match result {
    Ok(value) => ok_response(request.id, value),
    Err(message) => error_response(request.id, -32601, &message),
})
```

Every error path that returns `Err(String)` from a method handler
collapses to JSON-RPC code `-32601 Method not found`. This includes:

| Actual error                              | Should be (per JSON-RPC 2.0 §5.1)             |
|-------------------------------------------|-----------------------------------------------|
| `"missing required parameter `uri`"`      | `-32602 Invalid params`                       |
| `"invalid cursor 'garbage'"`              | `-32602`                                      |
| `"cursor offset N exceeds list length M"` | `-32602`                                      |
| `"resource not found: <uri>"`             | server-defined (MCP convention: `-32002`)     |
| `"unknown prompt: <name>"`                | server-defined `-32000..-32099`               |
| `"tool 'up' requires --allow-write-tools"`| server-defined (gating)                       |
| `"unknown method: <m>"`                   | `-32601` (the only case where it is correct)  |

Outside of the literal parse error (`-32700`) and JSON-RPC version
mismatch (`-32600`), the server never emits `-32602`, `-32603`, or
anything in the server-defined range. A strict MCP client that
branches on `error.code` to decide whether to retry, refresh
discovery, or surface to the user has no way to distinguish "method
exists but params are wrong" from "method does not exist."

## Why it matters

JSON-RPC 2.0 §5.1 explicitly maps error semantics to code ranges
specifically so clients don't need to grep error messages. MCP
clients (Claude Code, Cursor, Continue, Cline, etc.) are likely
permissive about this today but a future stricter client would
mis-handle these. More immediate: tool-call errors that *should*
surface to the user via the `result.content` envelope with
`isError: true` (per MCP) are escaped into the JSON-RPC envelope
error and may be treated by the client as protocol-level failures
(see related finding #M4 in the raw MCP audit:
`crates/coral-mcp/src/server.rs:417` `ToolCallResult::Error =>
Err(message)`).

## Suggested fix

Introduce a typed error at the handler boundary:

```rust
enum HandlerError {
    InvalidParams(String),
    NotFound(String),     // resource/prompt not found
    Gated(String),        // --allow-write-tools off
    ToolError(String),    // tool ran and returned error — see below
    Internal(String),
}
```

Map to codes:

| Variant            | JSON-RPC code        |
|--------------------|----------------------|
| `InvalidParams`    | `-32602`             |
| `NotFound`         | `-32002` (resource)  |
| `Gated`            | `-32001`             |
| `ToolError`        | route to `result` w/ `isError: true`, not envelope error |
| `Internal`         | `-32603`             |

For `tools/call` specifically: per MCP spec, errors from a tool *that
ran* should be reported as `result: { isError: true, content: [...] }`,
not as a JSON-RPC envelope error. The envelope error is reserved for
protocol-level failures (method missing, params malformed). The
current code at `server.rs:417` (`ToolCallResult::Error => Err(message)`)
routes tool errors through the envelope error path, then through the
`-32601` collapse, double-violating the spec.

Add tests that exercise each error path and assert the code matches
the variant.

## Cross-validation

MCP agent flagged this; I verified the bug at `server.rs:277-280` and
the tool-call routing at `server.rs:417`. The same agent also pinned
the negative result that notifications (`request.id` is None) correctly
suppress error responses per JSON-RPC §4.1 — so that part of the spec
is satisfied.
