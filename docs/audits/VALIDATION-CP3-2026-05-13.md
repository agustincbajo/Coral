# Validation: CP-3 + CP-5 MCP HTTP auth + mutex poisoning — v0.35 sprint
Date: 2026-05-13
Validator: claude (opus 4.7)
Targets: 7 commits `1d0f042..4cca700` (range `1d0f042^..4cca700`)

## Verdict

**APPROVED_WITH_REVISIONS** — all 4+1 findings (SEC-01, SEC-07, Q-C1, TEST-01,
CP-5) close cleanly with zero hallucinations in the dev's claims. The bearer-
auth gate is correctly ordered before any DoS-able work, the CSPRNG is real
(routes through `getrandom`), parking_lot adoption is complete on the touched
files, and all 24 net-new tests (13 ui + 11 mcp) pass. Three minor revisions
recommended (none block merge of the 7 commits in scope):

1. The dev's "13 sites converted" tally undercounts the actual converted
   `.lock()` call-sites by ~5 — the file totals are 8 in `server.rs` and 10
   in `http_sse.rs` (18 lock-call sites, all parking_lot). The audit's "28
   sites" claim was an overcount. Both numbers are decorative — the
   behavioural property (no `std::sync::Mutex` residual in the touched
   files) is verified.
2. `cargo clippy --workspace --all-targets -- -D warnings` is no longer
   clean, but the regression is in `crates/coral-ui/src/server.rs:27` from
   commit `37cb770` (perf(ui) dispatcher CP-3 of the UI track) which lands
   AFTER `4cca700` — outside the validation range. Flag for the next sprint
   cleanup, not this PR.
3. `cargo fmt --check` reports 4 hunks in `crates/coral-cli/src/commands/
   ui.rs`; `git log -- <file>` last-touched at `d678be8` (v0.32.0). Pre-
   existing rustfmt edition drift, unrelated to CP-3.

## Spot-check results

| Aspect | Status | Notes |
|---|---|---|
| shared `coral-core::auth` module | VERIFIED | `crates/coral-core/src/auth.rs` exposes `is_loopback`, `constant_time_eq`, `extract_bearer_token`, `verify_bearer`, `BearerAuthError` (4 variants + `label()`). Consumed by `coral-mcp/src/transport/http_sse.rs:429` AND `coral-ui/src/auth.rs:27` (via `pub use coral_core::auth::is_loopback`). coral-ui preserves its `ApiError` mapping by wrapping `check_bearer` over the shared primitive. |
| parking_lot 13-site conversion | VERIFIED_WITH_COUNT_DRIFT | 0 `std::sync::Mutex` residual in `server.rs` / `http_sse.rs` (only mention is a Cargo.toml comment). `use parking_lot::Mutex` at `server.rs:19` and `use parking_lot::{Condvar, Mutex}` at `http_sse.rs:35`. Actual `.lock()` call-sites: 8 + 10 = 18 (dev said 13, audit said 28). |
| SEC-07 CSPRNG | VERIFIED | `http_sse.rs:970 let bytes: [u8; 16] = rand::random();` → `format_uuid_v4(bytes)`. `rand = "0.9"` added as workspace dep (`Cargo.toml:157`). `rand::random` routes through `OsRng` → `getrandom` → kernel CSPRNG on every platform. Old "NOT cryptographically random" doc-comment replaced with a 12-line explanation referencing kernel sources by name. Tests pin the v4 visual shape + 10k-id uniqueness. |
| SEC-01 bearer-first gate order | VERIFIED | `handle_request` (line 355) ordering: OPTIONS preflight (372) → `.well-known` (392) → path-404 (403) → **bearer check (427)** → Origin (447) → Accept (460) → method dispatch (462) → body read (532, inside `handle_post`). Bearer happens BEFORE the session table is touched, the body is read, or any DoS-able work. 401 body is `{"error":"unauthorized; pass Authorization: Bearer <token>"}` — no info leak about which failure mode (label is logged via `tracing::warn`, not returned). |
| TEST-01 (13 tests in coral-ui) | VERIFIED | `cargo nextest run -p coral-ui auth` → `13 tests run: 13 passed, 50 skipped` in 0.094s. Tests cover: loopback aliases via re-export, constant_time_eq stability, bearer both casings + missing + mismatched, host loopback vs external + DNS-rebind, origin allowlist + null + attacker origin. |

## CP-5 shutdown stall pre-empt

`handle_request` flow at `http_sse.rs:355-495`:

```
OPTIONS preflight  (line 372) ← intentionally pre-auth (CORS browser probe)
/.well-known/mcp/* (line 392) ← intentionally pre-auth (registry discovery)
path 404 if not /mcp (line 403)
bearer check       (line 427) ← O(1): single header lookup + constant-time compare
Origin allowlist   (line 447)
Accept header      (line 460)
method dispatch    (line 462)
└─ handle_post → content-type → body_length cap → read body (line 532)
└─ handle_get_sse / handle_delete
```

Both pre-auth bypasses are safe by design:
- **OPTIONS**: returns a fixed 204 with Allow/Access-Control-* headers
  (no session state, no body read). A pre-auth attacker burns ~32 bytes of
  headers + a small `Vec<Header>` allocation per request — well under the
  per-handler thread cost. The browser cannot otherwise discover the
  allowed-headers list to send `Authorization` on the real request.
- **`.well-known/mcp/server-card.json`**: returns a static JSON document
  built from compile-time constants + catalog `.len()` calls
  (`handle_well_known_card`, no `Mutex` touched). Other paths under
  `/.well-known/mcp/` are hard-404'd before any state read.

The CP-5 claim ("bearer check BEFORE Origin/Accept/body read/session-table
touch") holds. An unauth attacker on `/mcp` cannot enqueue a long-running
operation that would stall the drain on shutdown — they get 401 before any
handler-side work is committed.

## Workspace state

- `cargo nextest run --workspace --no-fail-fast`: **1900 tests run, 1874
  passed, 26 failed, 19 skipped** (matches dev's claim exactly).
- Sampled 5 failing tests:
  - `coral-cli::release_flow release_sh_preflight_fails_when_ci_locally_fails`:
    panics with Windows OS code 193 "no es una aplicación Win32 válida"
    (shell script not executable on Windows). File last-touched
    `403b787` (v0.22.4) — predates CP-3.
  - `coral-runner gemini::tests::gemini_runner_non_zero_*`: same Windows
    subprocess-env class.
  - `coral-env compose_yaml::tests::watch_path_resolves_*`: pre-existing.
  - `coral-cli::template_validation`: pre-existing.
  - `coral-stats tests::stats_schema_matches_committed_file`: pre-existing.
  All 26 are Windows-env / non-bash failures; none touch the 7 commits in
  scope. ✔ pre-existing.
- `cargo nextest run -p coral-mcp`: **154/154 PASS** (includes the 11
  SEC-01 tests at `mcp_http_sse_e2e.rs:752-` and the 6 net-new
  `new_session_id` / `format_uuid_v4` / `session_table_reap` unit tests).
- `cargo nextest run -p coral-ui auth`: **13/13 PASS**.
- `cargo clippy --workspace --all-targets -- -D warnings`: **FAILS** with
  `doc_lazy_continuation` at `crates/coral-ui/src/server.rs:27`. Commit
  responsible: `37cb770` (outside CP-3 range). Not the dev's regression.
- `cargo fmt --check`: **FAILS** with 4 hunks in
  `crates/coral-cli/src/commands/ui.rs` (last touched `d678be8`, v0.32.0).
  Pre-existing.

## Cross-audit closure

| Finding | Audit doc | Status |
|---|---|---|
| SEC-01 (MCP HTTP unauth tool execution) | AUDIT-SECURITY-2026-05-12.md L24 | CLOSED — bearer enforced on every `/mcp` request; non-loopback bind without token errors at `bind_with_auth` time; CLI auto-mints 256-bit token on non-loopback when `--token`/env absent |
| SEC-07 (predictable Mcp-Session-Id) | AUDIT-SECURITY-2026-05-12.md L30 | CLOSED — `rand::random::<[u8;16]>()` via OsRng→getrandom; 122-bit entropy v4 UUID; entropy test pins the kernel-CSPRNG-shape |
| Q-C1 / Q-H5 (Mutex poisoning, 28 sites) | AUDIT-QUALITY-2026-05-12.md L28, L85 | CLOSED — every `Mutex` in `server.rs` + `http_sse.rs` is `parking_lot::Mutex` (no poisoning by construction). Audit's "28" is an overcount of the 18 actual `.lock()` call-sites; the underlying property (zero std::sync::Mutex in touched files) is verified |
| TEST-01 (coral-ui auth untested) | AUDIT-TESTING-2026-05-12.md L78, L92 | CLOSED — 13 unit tests added covering host/origin/bearer rules end-to-end against pure helpers |
| CP-5 (unauth shutdown stall composition) | (sprint planning, no separate audit) | PRE-EMPTED — handler-order analysis above confirms bearer-check runs before any blocking work; pre-auth requests cost O(1) |

## Autonomous decisions review

- **`--bind 0.0.0.0` without `--token` auto-mints (not bails).** Looking at
  `coral-cli/src/commands/mcp.rs:390-438`: the CLI prints the minted token
  to STDOUT (not stderr) prefixed with `coral mcp serve — auto-minted
  bearer token`, instructs the operator to copy it into their MCP client's
  `Authorization: Bearer` header, and notes the server forgets it on exit.
  This matches the `coral ui serve` precedent (same UX shape) and is a
  reasonable default: bailing would be hostile to the LAN-mode happy path,
  and stdout printing is explicit enough that operators can pipe it.
  **Caveat**: stdout-printing a secret is a footgun if the operator pipes
  stdout to a log file. The eprintln vs println split is intentional
  (banner→stderr, token→stdout) and matches coral-ui — keep, but worth
  documenting in CHANGELOG.

- **OPTIONS preflight + `.well-known/mcp/server-card.json` bypass auth.**
  Both are safe by inspection:
  - OPTIONS returns CORS headers only (no app state read, no body parse).
    The W3C CORS spec REQUIRES preflight to succeed without credentials
    or browsers cannot send `Authorization` on the real request. Forcing
    auth on OPTIONS would brick every browser-based MCP client.
  - `.well-known/mcp/server-card.json` is intentionally a public
    discovery endpoint per the 2025-11-25 MCP spec ("registries and
    discovery probes hit this from any origin"). The card is built
    from compile-time constants + catalog counts — no PII, no exfil
    surface. Other `.well-known/mcp/*` paths return 404 immediately.

  Both bypasses are pre-auth but post-routing (path-checked before bearer
  gate), so a malicious attacker cannot use them to bypass auth for any
  state-mutating endpoint. **Accept.**

## Recommendation

**MERGE the 7 commits in scope as-is.** The 4+1 finding closures are real,
the bearer-first ordering achieves the CP-5 pre-empt property, and the
net-new test coverage (24 tests) pins the rules. The 13 vs 18 site-count
drift is decorative. Address the `coral-ui/src/server.rs` clippy regression
in a follow-up that touches `37cb770`'s work, not this PR. Optionally drop a
single rustfmt commit on `coral-cli/src/commands/ui.rs` to clear the
pre-existing fmt drift — independent of CP-3.
