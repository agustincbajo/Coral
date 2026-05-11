# Raw MCP audit (agent a7b05f4bb3569d764)

## H1 — Stale data: WikiState wired to watcher but NOT to read path
Files: crates/coral-mcp/src/resources.rs:73-86,174-189; crates/coral-mcp/src/watcher.rs:9-13,84-91; crates/coral-mcp/src/state.rs
WikiResourceProvider::read_pages uses OnceLock<Vec<Page>>; watcher marks dirty but read_pages doesn't consult WikiState.

## H2 — README/code drift on counts
README says 6 resources / 5 read-only tools. Code: static_catalog → 8 resources, read_only → 7 tools.
Files: README.md:85,779; crates/coral-mcp/src/tools.rs:37-87; crates/coral-mcp/src/resources.rs:101-152

## H3 — JSON-RPC error codes collapse to -32601
crates/coral-mcp/src/server.rs:277-280 returns -32601 for all non-parse/non-version errors (invalid params, gate-denied, not-found, cursor out of range).

## H4 — Write-tool gate message leaks existence
crates/coral-mcp/src/server.rs:401-405 returns "tool 'up' requires --allow-write-tools" wrapped in -32601.

## H5 — initialize detection uses substring on body
crates/coral-mcp/src/transport/http_sse.rs:348-358 contains("\"initialize\"") false-positives on tools/call payloads carrying that token.

## M1 — POST /mcp no Content-Type validation
crates/coral-mcp/src/transport/http_sse.rs:285-378

## M2 — GET /mcp: no Last-Event-ID, no notification draining
crates/coral-mcp/src/transport/http_sse.rs:384-420. HTTP/SSE clients never receive push notifications despite subscribe:true. Only stdio drains notifications.

## M3 — Audit log rotation not crash-safe
crates/coral-cli/src/commands/mcp.rs:428-445. Two non-atomic syscalls without fsync; race between concurrent rotators; concurrent appends between metadata check and rename can drop entries.

## M4 — tools/call errors route via JSON-RPC envelope
crates/coral-mcp/src/server.rs:417. MCP spec: tool errors should be result.isError, not envelope error.

## M5 — Concurrency cap TOCTOU (fetch_add then check)
crates/coral-mcp/src/transport/http_sse.rs:131-141

## L1 — is_origin_allowed("") returns true
crates/coral-mcp/src/transport/http_sse.rs:570-573

## L2 — test-report/latest mimeType always application/json, body may be XML
crates/coral-mcp/src/resources.rs:459-466

## OK
- L3 Notifications correct (id: null only on parse error)
- L4 Unknown method notification suppression correct
