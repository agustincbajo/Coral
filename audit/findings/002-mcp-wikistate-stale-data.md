---
title: "mcp: WikiState dirty-flag wired to watcher but never consulted on resources/read — server serves stale wiki data until restart"
severity: High
labels: bug, mcp, correctness
confidence: 5
cross_validated_by: [mcp-audit-agent, concurrency-audit-agent, direct-code-read]
---

## Summary

The MCP server advertises `resources: { listChanged: true, subscribe: true }`
(`crates/coral-mcp/src/server.rs:287`) and ships a polling file watcher
(`crates/coral-mcp/src/watcher.rs`) that correctly marks an
`Arc<RwLock<WikiState>>` dirty on filesystem changes. But the resource read
path **never consults `WikiState`**: it uses an unrelated `OnceLock<Vec<Page>>`
inside `WikiResourceProvider` (`crates/coral-mcp/src/resources.rs:73,174-189`)
that is populated on first access and **cannot be invalidated without a
process restart**.

The source comment at `crates/coral-mcp/src/watcher.rs:7-13` admits this
verbatim:

```
**Design note (v0.24 #13):** the current `WikiResourceProvider` uses
`OnceLock` for page caching, which cannot be invalidated without a
process restart. This watcher still provides value: it pushes the
MCP `notifications/resources/list_changed` signal so clients know
*something* changed — even if this server process serves stale data
until restarted. A follow-up PR will swap `OnceLock` → `Mutex<Option<>>`
to enable in-process cache invalidation.
```

The follow-up PR never landed; in `v0.30.0` `WikiState` still has zero
call sites in `server.rs` or `resources.rs` (verified by grep — no hits on
`refresh(`, `is_dirty(`, or `WikiState` in either file).

## Impact

Per the README:

- v0.21+: "MCP `mimeType` matches actual payload per resource (catalog-driven)"
- v0.21+/M2.4 (commit eb715e1): "stateful WikiState with dirty-flag refresh"

The second claim is false in practice. An MCP client that re-fetches
`coral://wiki/<repo>/<slug>` after receiving `notifications/resources/list_changed`
sees the same body it saw on first read. The whole point of the watcher —
keep agents looking at fresh wiki state — is silently defeated. Every
agent integration listed in the README (Claude Code, Cursor, Continue,
Cline, Goose, Codex, Copilot) is affected.

## Repro

1. Start the server:
   ```bash
   coral mcp serve --transport stdio
   ```
2. From a client, call `resources/read` on
   `coral://wiki/<repo>/<some-slug>`. Note the body.
3. In another terminal, edit the page:
   ```bash
   echo "## update" >> .wiki/<repo>/<some-slug>.md
   ```
4. Wait > 2 s (the watcher poll interval).
5. The client receives `notifications/resources/list_changed`.
6. Call `resources/read` again on the same URI.
7. Observe: the body is unchanged.

## Suggested fix

Two-part:

1. Replace `pages_cache: OnceLock<Vec<Page>>` in `WikiResourceProvider`
   with the `Arc<RwLock<WikiState>>` that already exists, or with a
   `Mutex<Option<Vec<Page>>>` per the design note. `read_pages()` should
   check `is_dirty()`; if dirty, call `refresh()` under a write lock;
   otherwise serve cached pages under a read lock.

2. Wire `shared_state(wiki_root)` from `state.rs:88` into the call site
   that constructs `WikiResourceProvider` (likely
   `crates/coral-cli/src/commands/mcp.rs` where the handler is assembled
   for `serve_stdio` / `HttpSseTransport::serve_blocking`).

## Cross-validation

Both the MCP agent and the concurrency agent independently identified
this finding from different starting points (resource provider vs.
state-machine wiring). Direct code-read of `resources.rs:73-189`,
`watcher.rs:7-91`, and `state.rs` confirms.
