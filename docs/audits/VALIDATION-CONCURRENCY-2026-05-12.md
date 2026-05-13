# Validation: Concurrency audit — Coral v0.34.1
Date: 2026-05-12
Validator: claude (sonnet)
Target: docs/audits/AUDIT-CONCURRENCY-2026-05-12.md

## Verdict

**APPROVED_WITH_REVISIONS** — 0 hallucinations on the spot-check (all 5 findings verified at the cited file:line), the tokio decision is well-grounded, and the audit explicitly carries the CON-09 ↔ SEC-M03 cross-reference. The revisions are scope: two real concurrency surfaces (`coral monitor up` SIGINT-vs-append, `coral skill build` zip writer flock) are absent, and one cross-audit amplification (SEC-01 + CON-06 + CON-02) is not connected.

## Spot-check results (5 findings)

| ID | Status | Notes |
|----|--------|-------|
| CON-01 (Critical) | **VERIFIED** | `crates/coral-ui/src/server.rs:106-107` confirmed: `match server.recv_timeout(...) { Ok(Some(req)) => handle(&state, req) }` runs inline on the recv thread. No thread spawn, no pool. SPA mount-time concurrent GETs serialize. Proposed fix (8-slot semaphore mirroring `MAX_CONCURRENT_HANDLERS=32` at `http_sse.rs:53`) is correct shape; `AppState` is already `Arc`, `Request` is `Send`. Fix does not introduce new races. |
| CON-02 (Critical) | **VERIFIED** | `crates/coral-cli/src/commands/mcp.rs:332-339` confirmed: 150 ms `thread::sleep` then `process::exit(0)`. The `active: Arc<AtomicUsize>` is exposed at `http_sse.rs:53-54` and incremented at L209 — perfect drain target. Note the audit text says "exposed at http_sse.rs:114" which is **slightly off** — the constant `MAX_CONCURRENT_HANDLERS` is at L53 and the `AtomicUsize` field lives in the `HttpTransport` struct around L65; line number drift on a single citation, fix shape unaffected. |
| CON-03 (High) | **VERIFIED** | `monitor/run.rs:117-126` confirmed: `OpenOptions::append(true) + writeln! + sync_all` with no `with_exclusive_lock`. Windows `_O_APPEND` race scenario is correct per MSDN. Two parallel monitors on the same env+monitor name corrupt the JSONL. Fix is mechanical. |
| CON-05 (High) | **VERIFIED** | `http_sse.rs:209-233` confirmed: `active.fetch_add` precedes `Builder::spawn`; on spawn failure (L229) the `Request` is moved into the closure and silently dropped without `respond_simple`. Client times out 30s+. The proposed reorder is minimal. |
| CON-09 (High) | **VERIFIED** | `self_register_marketplace.rs:203-219` confirmed: `std::fs::copy` for backup at L207 runs BEFORE `with_exclusive_lock` at L219. Second parallel invocation backs up the already-patched file. Same surface as SEC-M03 in `AUDIT-SECURITY-2026-05-12.md:66`. ~5 LoC fix as claimed. |

Bonus sanity sample: CON-04 cost-accumulator race at `bootstrap/mod.rs:288, 383` confirmed; CON-06 `for request in server.incoming_requests()` at `http_sse.rs:201` confirmed (no shutdown poll); `MAX_CONCURRENT_HANDLERS` at `http_sse.rs:53` confirmed.

## Cross-audit reinforcement matrix (Phase 1 + Phase 2)

| Concurrency finding | Phase 1 finding | Combined impact |
|---|---|---|
| **CON-09** (backup-before-lock) | **SEC-M03** (`AUDIT-SECURITY-2026-05-12.md:66`) | **Captured.** The audit calls this out in CON-09's "SEC-M03 cross-reference" sentence and explains the severity uplift (backup is the only rollback). Cleanly handled. |
| **CON-M03** (session-id collisions) | **SEC-07** (`http_sse.rs:820-840`) | **Captured.** Explicitly referenced in the Medium appendix. |
| **CON-06 + CON-02** (no shutdown poll, 150 ms exit) | **SEC-01** (unauth MCP HTTP POST) | **MISSED amplification.** With SEC-01 unfixed and the server bound off-loopback, an unauthenticated attacker can spam `tools/call` to keep `active` high and turn CON-02's 150 ms exit window into either a forced truncation (bad UX) or — once the audit's proposed drain loop with `CORAL_MCP_SHUTDOWN_GRACE_MS=5000` lands — a 5s graceful-shutdown denial. Worth a one-line note in CON-02 saying "drain bound must coexist with SEC-01 to avoid attacker-induced shutdown stall." |
| **CON-06** (in-flight tool calls truncated at exit) | **TEST-07** (stdio MCP not exercised E2E) — adjacent | TEST-07 covers the secure-by-default stdio transport; HTTP transport's shutdown drain (CON-02/06) is what actually needs an integration test. Audit does not flag this. |
| **CON-01** (UI single-threaded) | **TEST-01** (auth.rs guards untested) | Same crate (`coral-ui`). A thread-pool refactor of `server.rs` touches the auth call sites; landing CON-01 without first landing TEST-01 risks a silent auth regression. Not called out. |

## Tokio decision review

**Validated.** The audit's "no tokio in v0.34.x" recommendation is sound:

1. **MSRV 1.85** is confirmed in workspace `Cargo.toml`; tokio 1.x compiles, so MSRV is not a *blocker* — but the audit's actual argument is dep-tree weight, which holds: a `tokio` + `hyper` + `bytes` + `pin-project-lite` + `tower` switch from `tiny_http` would roughly double link time and triple supply-chain surface, against zero current async-I/O need.
2. **Single-binary plug-and-play distribution goal** (per project memory) is materially harder with an async runtime stack — installers must verify rustls + ring + tokio-rustls work on every target triple.
3. **Both Criticals fix in <300 LoC of `std::thread + semaphore`.** Confirmed: CON-01 is ~50 LoC (lift the `handle(state, req)` call inside a spawn behind a `parking_lot::Mutex<usize>` permit), CON-02 is ~40 LoC (replace sleep with a `while active.load > 0` bounded loop). The `MAX_CONCURRENT_HANDLERS = 32` pattern at `http_sse.rs:53` is directly portable to `coral-ui/src/server.rs` — same `Arc<AtomicUsize>` shape, same `ActiveGuard` Drop helper, just narrower cap (8–16 sufficient for SPA load).
4. **Alternative runtimes**: `async-std` is in long-term maintenance with stalled releases (development effectively paused 2024+); `smol` is alive but still pulls `polling`/`async-io`/`async-task` and would not reduce the dep-tree argument materially. Neither is a credible alternative to `tokio` for v0.34.x. Audit's framing is correct.

The one nuance worth adding: the audit treats the semaphore pattern as trivially extrapolable. For MCP it works because handlers are short and stateless. For `coral ui serve`, the `/api/v1/query` streaming-LLM handler is **long-lived** — 30s+ — which means the 8-slot cap acts as a hard concurrency ceiling for the entire SPA. Acceptable for v0.34 (single-user), but worth a one-line caveat tying it to the v0.37 multi-user roadmap.

## Gaps not caught

1. **`coral monitor up` SIGINT-during-append**: `monitor/up.rs` registers SIGINT/SIGTERM via `signal_hook::flag::register` (L390-401) and polls `shutdown_flag()` in the wiki-ingest loop. The race window: a SIGINT arrives mid-`append_run` (between `writeln!` at `run.rs:124` and `sync_all` at `run.rs:125`). Linux: `O_APPEND` write is atomic at the byte level, so SIGKILL after `writeln!` returns leaves the line on disk but un-fsync'd; on a hard host crash within the next ~5s the line is lost. CON-L08 mentions the fsync but not the signal-vs-fsync race. **Flag as missed Low.**
2. **`coral skill build` zip writing**: `crates/coral-cli/src/commands/skill.rs:336-380` uses `tempfile::NamedTempFile + persist` for atomic rename **without** `with_exclusive_lock`. Two parallel `coral skill build --version X` invocations torn-rename the dist zip silently. Lower probability than CON-M01 (cache torn-rename) but same class. **Flag as missed Low.**
3. **`coral test` parallel test rule execution**: `coral-test/src/orchestrator.rs:177, 195` uses `par_iter` and `into_par_iter`. The audit mentions in CON-06 that "`coral_test::run_test_suite_filtered` is itself rayon-parallel" but never audits this surface for cross-rule contention on shared state (e.g. `body_tempfile` paths, fixture directories). **Flag as missed Medium.**
4. **`coral ingest --apply` concurrent with `coral bootstrap`**: ingest locks `.wiki/index.md` at `ingest.rs:259`, bootstrap locks `.bootstrap-state.json.lock` via `BootstrapLock::acquire` at `state.rs:299-320`. These are **different lockfiles** — ingest writing `index.md` while bootstrap is mid-page can corrupt `index.upsert` ordering. The audit's executive summary claims "every persistent write ... goes through `with_exclusive_lock`" but the writes target different sentinels, so locks don't serialise across these two commands. **Flag as missed High.**
5. **`BootstrapState` reader-side**: The audit calls for `Arc<Mutex<f64>>` BEFORE parallelization (CON-04) but does not address whether a concurrent reader (telemetry / `coral doctor`) sees torn JSON mid-write. Answer: no — `atomic_write_string` uses tempfile + rename, which is atomic on POSIX and `MoveFileEx` on Windows, so readers see either old or new. Audit's implicit assumption is correct but worth one explicit sentence.

## Severity discipline observations

- 2 Critical is well-calibrated. CON-01 produces user-visible SPA freeze on every LLM stream — high-frequency, high-impact. CON-02 produces truncated JSON-RPC responses on every Ctrl-C with an active `coral test` — visible in any non-trivial MCP session.
- **CON-04 should arguably be Critical, not High**, *conditional on* the v0.35 milestone landing `--workers N`. Today it's serial-only and the race is latent. The audit correctly flags this as "not safe under any future parallelization." Severity is fine for v0.34.1; promote on the v0.35 ratchet.
- **CON-09 + SEC-M03 combined** is correctly uplifted to High by the audit (SEC-M03 was Medium in isolation). This is a model of cross-audit reasoning.

## Recommendation

Land the audit as-is. Open four follow-up issues against the Concurrency backlog: (1) one-line caveat in CON-02 about the SEC-01 amplification, (2) add `coral monitor up` signal-fsync race as CON-L11, (3) add `coral skill build` torn-zip as CON-L12, (4) add ingest-vs-bootstrap cross-lockfile race as CON-M06 (worth promoting to High depending on user-visibility). Pair the CON-09 PR with the SEC-M03 fix (same diff, ~5 LoC).
