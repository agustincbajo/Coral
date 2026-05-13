# Validation: CP-2 tiny_http thread-pool en coral-ui — v0.35 sprint
Date: 2026-05-13
Validator: claude (sonnet)
Targets: 3 commits 37cb770..65ff4d9

## Verdict

APPROVED. 0 hallucinations: every claim in the dev brief reproduces against
the working tree. Patterns mirror the CP-3 cleaned `coral-mcp::http_sse`
substrate; backwards-compat surface intact; CP-4 (api_smoke landed at
`coral-cli/tests/ui_api_smoke.rs`) does not touch the same files.

## Spot-check

| Aspect | Status | Notes |
|---|---|---|
| `MAX_CONCURRENT_HANDLERS = 32` | OK | `crates/coral-ui/src/server.rs:52` |
| `Arc<AtomicUsize>` + `fetch_add`/`fetch_sub` semaphore | OK | `server.rs:169`, `:181`, `:183`, `:202` |
| RAII `ActiveGuard` decrement-on-Drop | OK | `server.rs:239-245`, used at `:192` |
| `std::thread::Builder::new().name(...).spawn(...)` per req | OK | `server.rs:189-194` (named handler thread, parity with coral-mcp `:317-319`) |
| `recv_timeout(Duration::from_millis(250))` poll | OK | `server.rs:176`. Pre-existing — confirmed by commit body (`"the recv loop's existing 250 ms recv_timeout poll"`) |
| `SHUTDOWN_DRAIN_TIMEOUT = Duration::from_secs(5)` | OK | `server.rs:60` |
| `SHUTDOWN_DRAIN_POLL = Duration::from_millis(50)` | OK | `server.rs:65` |
| Drain loop polls `active.load() > 0` with deadline | OK | `server.rs:219-233` (`Acquire` load, monotonic — comment matches behaviour) |
| `BUSY_BODY = r#"{"error":"server busy: too many concurrent requests"}"#` | OK | `server.rs:71` |
| 503 Content-Type `application/json` | OK | `respond_busy` uses `json_header()` (`server.rs:250-255` + `:459-462`) |
| 503 status code | OK | `with_status_code(503)` (`server.rs:252`) |
| `pub(crate) fn run_recv_loop` extraction | OK | `server.rs:162` — visibility scoped, no public API leak |
| `#[cfg(test)] /_test/slow` route | OK | `server.rs:350-359`, attribute gates the match arm; production build cannot route there (falls through to `ApiError::NotFound`, `server.rs:407`) |
| `spawn_test_server` state-builder closure | OK | `server.rs:633-651`, port-0 + caller-supplied `make_state(port)` so `check_host` aligns with the OS-picked port |

## Measurements

| Test | Claim | Measured |
|---|---|---|
| `health_not_blocked_by_concurrent_slow_handler` | `<400ms` for 8 concurrent `/health` while slow handler sleeps 500ms | PASS; total test wall-clock 0.732s (incl. 500ms slow sleep + 100ms warmup). Internal `elapsed < 400ms` budget held — assert fires on the 8-worker fan-in, not the test envelope. |
| Pre-fix serialization claim (~4s) | Mathematically sound: 1 thread × (500ms slow + 8 × ~ε ms /health) serialized = ≈ 500–600ms wait, **not** 4500ms. The dev brief's "4500ms serial" arithmetic assumes 8 × 500ms which doesn't apply (only one slow handler). The qualitative claim — pre-fix would serialize past the 400ms budget — is correct; the headline number is overstated. (Not a blocker — fix is real, 0.732s test PASS confirms.) |
| `concurrent_cap_returns_503_above_threshold` | ≥1 503 + total conserved | PASS in 0.629s. Asserts `ok + busy == cap + 8` (line 780) and `busy >= 1` (line 782). Every 503 body re-parsed as JSON, `"busy"` substring asserted (lines 790-798). |
| `shutdown_flag_drains_recv_loop_promptly` | Recv loop exits in `< 1s` after flag set | PASS in 0.367s (250ms `recv_timeout` poll + instant drain with no in-flight). |
| `busy_body_is_canonical_error_envelope` | `BUSY_BODY` parses, has `error` string containing `"busy"` | PASS. |
| `active_guard_decrements_on_{normal_drop,panic_unwind}` | RAII decrement holds across panic | PASS both. |

Full suite: **69/69 PASS** under `cargo nextest run -p coral-ui` (1.277s wall). +6 new CP-2 tests vs. CP-1 baseline (63), matches dev claim.

## Cross-pattern consistency (vs coral-mcp::http_sse)

Direct comparison (`coral-mcp/src/transport/http_sse.rs:54`, `:285-345` vs `coral-ui/src/server.rs:52`, `:162-245`):

- `MAX_CONCURRENT_HANDLERS = 32` — **identical constant + doc rationale (macOS 256 ulimit)**.
- Pre-increment / check / decrement-on-overflow ordering — **identical** (`fetch_add(1, SeqCst) + 1` then `> cap` then `fetch_sub`).
- `ActiveGuard(Arc<AtomicUsize>)` + `Drop` impl with `fetch_sub(1, SeqCst)` — **identical struct shape**.
- `std::thread::Builder::new().name(...).spawn(move || { let _guard = ActiveGuard(...); ... })` — **identical spawn shape**, only the thread name differs (`coral-ui-http` vs `coral-mcp-http`).
- 503 envelope string — **byte-identical** (`{"error":"server busy: too many concurrent requests"}`).
- **One real divergence**: coral-mcp's `respond_simple` declares `Content-Type: text/plain; charset=utf-8` for the busy body (`http_sse.rs:311`) while coral-ui uses `application/json` via `json_header()`. coral-ui's choice is **correct** — the body is JSON; coral-mcp's `text/plain` for a JSON envelope is a minor latent inconsistency in the older surface (out of scope here, flagged for future cleanup, NOT a CP-2 blocker).
- Two real differences vs. coral-mcp, both intentional and justified: coral-ui adds (1) a `recv_timeout`-driven shutdown poll (coral-mcp uses `incoming_requests()` blocking — different lifecycle), and (2) a graceful drain budget (`SHUTDOWN_DRAIN_TIMEOUT`).

"coral-ui-local pattern (NO shared module)" decision: audit confirms duplicated surface is ~7 lines of meaningfully shared structure (constant + `ActiveGuard` struct + Drop impl). Dev brief's "3 lines" claim is undercount but the call (keep local) is defensible — extracting a module here would force public API on the busy-body string + cap constant for marginal DRY benefit. Acceptable as-is; flag for a v0.40 shared `coral_core::http_pool` module if a third caller appears.

## Workspace state

- `cargo nextest run -p coral-ui`: **69/69 PASS** (1.277s).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo fmt --check -p coral-ui`: clean.
- `cargo fmt --check -p coral-cli`: clean. (Validator-CP3's prior `commands/ui.rs` fmt flag has since been resolved by CP-4's SEC-02 touch — fmt status confirmed at HEAD.)
- `65ff4d9` style commit: confirmed it fixes `doc-lazy-continuation` lint regressions introduced by the multi-paragraph doc-comments in `37cb770` + applies rustfmt; no semantic changes.

## Cross-audit closure

- **P-C3** (`/health` head-of-line block, AUDIT-PERFORMANCE): closed by `health_not_blocked_by_concurrent_slow_handler` (0.732s w/ 500ms parked slow handler).
- **CON-01** (single-threaded dispatch, AUDIT-CONCURRENCY): closed by thread-spawn + `ActiveGuard` + 503 cap.
- **CON-06** (no graceful shutdown drain, AUDIT-CONCURRENCY): closed by 5s drain budget + 50ms poll loop + `shutdown_flag_drains_recv_loop_promptly` test.
- **ARCH-C3** (tiny_http stay vs. swap): implicitly validated. The thread-pool port is the v0.35 mitigation; substrate-swap remains a v0.40 question, consistent with CP-1 validation.

## CP-4 overlap check

`44047f2` CP-4 adds only `crates/coral-cli/tests/ui_api_smoke.rs` (449 lines, NEW file). It does **not** touch `crates/coral-ui/src/server.rs`. `run_recv_loop` is `pub(crate)` so CP-4's e2e tests reach the dispatch loop through the public `serve(ServeConfig)` entry point unchanged. No file-level or API-level overlap. CP-2's revert of an earlier `commands/ui.rs` fmt-touch (to avoid stomping CP-4 SEC-02 work) confirmed: that file has no diff vs. CP-4 expectations and fmt-check passes.

## Recommendation

APPROVED. Merge as-is. Two cosmetic gaps worth a future low-priority pass (NOT blockers):

1. The "4500ms serial" pre-fix figure in the dev brief is mathematically off (8 × /health does NOT serialize to 8 × 500ms when only one /_test/slow is parked) — the test commentary at `server.rs:730-733` is the authoritative version and gets the budget right. Suggest updating future briefs to mirror that comment.
2. The coral-mcp side's `text/plain` Content-Type for the 503 JSON envelope is now visibly out of step with coral-ui's `application/json`. Track for a v0.36 housekeeping item; coral-ui's choice is the correct one.

No revisions required for the CP-2 commits themselves.
