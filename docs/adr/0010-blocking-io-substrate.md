# ADR 0010 — Blocking I/O substrate for Coral's HTTP surfaces

**Date:** 2026-05-13
**Status:** accepted (v0.35)

## Context

Coral ships two long-lived HTTP-speaking surfaces:

1. `coral mcp serve --transport http` — Streamable HTTP MCP transport
   (POST/GET/DELETE `/mcp`). Lives in `coral-mcp::transport::http_sse`.
2. `coral ui serve` — REST API + embedded SPA. Lives in `coral-ui`.

Both are exposed to local-network clients (loopback by default; opt-in
`--bind 0.0.0.0` for power users). Concurrency expectations are
modest: 1-10 simultaneous editor/IDE clients per developer machine,
upper-bounded by the per-developer-machine workload.

The implementation question, repeatedly raised in audits Q/F and
parallel-Phase reviews, is whether to keep the current blocking-I/O
substrate (`tiny_http` + std threads) or migrate to an async runtime
(tokio / async-std / smol). The async option is conventional in the
Rust ecosystem; the blocking option is what we have and what every
HTTP-touching crate in the workspace currently uses.

This ADR records the v0.35 decision and the reasoning so a future
contributor proposing the swap has the prior context.

## Decision

**Stay with blocking I/O (`tiny_http` + `std::thread`) for both
HTTP surfaces in v0.35 and beyond, until a concrete scaling
requirement forces a revisit.**

The MCP HTTP transport (CP-3) and the WebUI server (CP-2) both use the
same pattern: a `tiny_http::Server` accept loop, one `std::thread`
per request capped by an `AtomicUsize` semaphore (default 32
concurrent handlers), and an `ActiveGuard` RAII drop to release the
slot. Health/cancellation paths fast-fail to keep the cap from
starving observability traffic.

## Rationale

- **MSRV 1.89 + single-binary distribution.** Coral installs as one
  `cargo install --locked coral-cli` binary. tokio drags an Arc'd
  scheduler, a thread-pool, signal-handling, and a wake notification
  fabric into every `coral …` invocation — `coral lint`, `coral
  init`, etc. don't touch HTTP at all and pay zero runtime cost
  today. Audits C and D measured the cold-start regression at ~12 MB
  resident + ~4 MB binary growth for the smallest tokio feature set
  that supports `tokio::net::TcpListener`. The MCP transport's
  measured cold-start (validator C) is 18 ms on a 2025 MacBook;
  tokio's measured cold-start for the equivalent listener is 31 ms.
- **The blocking-I/O surface is small.** `coral mcp serve --transport
  http` parses headers, walks a fixed router, and either reads a
  capped-size POST body or streams an SSE keep-alive. `coral ui
  serve` answers JSON envelopes and serves a baked-in SPA. Neither
  needs back-pressure-aware streaming, async DNS, or any of tokio's
  "long-lived 10k connections" features.
- **The concurrency budget is set by the host, not the runtime.** A
  developer machine running `coral ui serve` will see 1-10
  simultaneous browser tabs. Even an overzealous Claude Code session
  hammering MCP will not exceed the `MAX_CONCURRENT_HANDLERS = 32`
  semaphore cap; the bottleneck is the wiki I/O + the LLM
  round-trip, not the HTTP layer.
- **Supply-chain surface.** tokio brings `mio`, `socket2`,
  `parking_lot` (we already vendor parking_lot — see SEC-Q1), and
  the platform-specific I/O fabric. Each is one more vector for
  RUSTSEC advisories; the audit recommendation was to keep the
  vendor surface tight unless we get a real win.

## Alternatives considered

- **tokio.** The default async runtime in the ecosystem. Rejected
  for the reasons above; the win/cost ratio doesn't make sense for a
  blocking-I/O surface that fits in 600 lines.
- **async-std.** Smaller than tokio but with weaker maintenance
  guarantees (no Foundation-level sponsorship). Same fundamental
  cost as tokio for our workload.
- **smol.** Lightest of the three, but the supply-chain reduction
  vs. `std::thread + tiny_http` is marginal once you count smol's
  `polling`, `async-io`, `async-task`, etc. The "small" claim is
  benchmarked against tokio, not against blocking I/O.
- **Hand-rolled `mio` loop.** Closes the supply-chain gap but
  reinvents what tiny_http already gives us, with bugs we'd own.

## Consequences

- **Thread-per-request stays the model.** CP-2 (coral-ui) and CP-3
  (coral-mcp HTTP) already implement the pattern; CP-2 has full
  integration tests around the 503 cap, drain on shutdown, and
  health-not-blocked-by-slow-handler.
- **No 10k-concurrent-connection scaling.** If a future Coral user
  needs to host the MCP transport behind a multi-tenant gateway with
  10k concurrent SSE clients, the substrate has to be re-evaluated
  — this is a known limit, not a hidden one. Document the cap in
  user-facing docs.
- **Test ergonomics stay simple.** Integration tests spin up
  `tiny_http::Server` on a random port, send a `std::net::TcpStream`,
  and assert on the response bytes. No `#[tokio::test]` machinery,
  no executor injection, no `tokio::time::sleep` mocking.
- **Re-evaluate when:** (a) a credible benchmark shows the
  thread-per-request pattern caps out below the configured
  semaphore, OR (b) we add a streaming/long-poll surface (e.g.
  WebSocket-based RPC) where blocking-I/O semantics are awkward.
  Until then this ADR stays accepted.

## References

- v0.35 Phase A CP-3 audit (validator C/D).
- v0.21.1 `tiny_http` selection rationale in `Cargo.toml` workspace
  deps comment.
- BACKLOG.md item #X (if we ever spin "evaluate async runtime" off
  again, link here).
