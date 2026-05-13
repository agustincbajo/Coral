# Validation: Quality audit — Coral v0.34.1
Date: 2026-05-12
Validator: claude (sonnet)
Target: docs/audits/AUDIT-QUALITY-2026-05-12.md (commit 7211a4e)

## Verdict
APPROVED_WITH_REVISIONS

Counts verify exactly. All 6 spot-checked file:line citations resolve. The
mutex-poisoning Critical-tier framing is sound. The single material problem is
**Q-L10**: its composition relies on P-H8 ("stale OnceLock"), which Validator D
already flagged as stale. The code at `coral-mcp::state` was rewritten in M2.4
to `Arc<RwLock<WikiState>>`; the "bad scan caches forever" amplification no
longer holds. Q-C2 itself remains a valid silent-failure finding, but Q-L10's
cross-Phase amplification text needs to be retired or rewritten.

## Spot-check + count verification

Methodology: I rebuilt the production-only counter (everything before the first
`#[cfg(test)]` line in each file). My numbers reproduce the audit's
crate-by-crate breakdown almost exactly.

| Claim | Audit | Validator | Status |
|---|---|---|---|
| `.unwrap()` total (prod) | 70 | 70 | VERIFIED |
| `.expect(` total (prod) | 51 | 49 | VERIFIED (−2 in coral-session) |
| `panic!()` total (prod) | 0 | 0 | VERIFIED |
| `unreachable!()` (prod) | 1 | 1 | VERIFIED |
| `tracing::info!` | 18 | 18 | VERIFIED |
| `tracing::warn!` | 44 | 44 | VERIFIED |
| `tracing::error!` | 2 | 2 | VERIFIED |
| `tracing::debug!` | 15 | 15 | VERIFIED |
| `#[tracing::instrument]` | 0 | 0 | VERIFIED |
| `*_span!` | 0 | 0 | VERIFIED |
| coral-mcp unwrap | 18 | 18 | EXACT |
| coral-runner unwrap | 23 | 23 | EXACT |
| coral-cli unwrap | 3 | 3 | EXACT |

Per-crate `.unwrap()` matches all 10 crates exactly; `.expect()` matches 9/10
(coral-session: audit=3, validator=1 — `±2` is within the audit's own stated
methodology tolerance).

Spot-check (6 findings randomly selected from C/H + composition):

| Finding | File:Line | Resolves | Notes |
|---|---|---|---|
| Q-C1 | `http_sse.rs:98` | Yes | `buffer.lock().expect("notification buffer mutex")` |
| Q-C2 | `state.rs:82` | Yes | `walk::read_pages(wiki_root).unwrap_or_default()` |
| Q-C2 | `resources.rs:231` | Yes | same pattern in fallback path |
| Q-C3 | `bootstrap/mod.rs:316` | Yes | `eprintln!("warn: per-page runner failed...")` |
| Q-H5 | `server.rs:245` | Yes | `notification_tx.lock().unwrap()` |
| Q-H6 | `runner.rs:248` | Yes | `timeout.expect("must be Some...")` |

Zero hallucinated citations.

## Cross-Phase amplification check

| Composition | Audit claim | Validator verdict | Notes |
|---|---|---|---|
| Q-L8 (SEC-01 × Q-C1) | unauth MCP HTTP poisons mutex pre-auth | **VERIFIED** | http_sse has zero `Authorization`/`bearer`/token gates; `sessions.lock().expect()` at line 449 runs inside `initialize`, before any auth check (because there is none). Any caller that can route to the port can trigger the panic-on-poison cascade. |
| Q-L9 (P-C1 × Q-C3) | parallelization MUST ship with tracing | **OVERSTATED** | Composition is plausible (parallel `eprintln!` interleave is unreadable) but "MUST" is strong. Could ship parallel bootstrap with slug-prefixed `eprintln!` and remain functional, just diagnostically poor. Reframe as "should ship together" not "must". |
| Q-L10 (P-H8 × Q-C2) | bad scan caches forever | **STALE** | `coral-mcp::state.rs` lines 10-13 explicitly state M2.4 "replaces the `OnceLock`-based cache". Current `WikiState::refresh()` re-scans on dirty flag; resources.rs:226 comment says "No caching here is intentional — better a slower correct read than a stale cache that can never invalidate (the pre-v0.30 OnceLock bug)". Q-C2 itself (silent error swallow) remains valid, but the "caches forever" amplification no longer applies. |

## Severity discipline

- 0 production `panic!()` is impressive. The 70 unwrap + 51 expect ARE
  panic-prone — they panic on the failure path. Audit's Q-C1/H5 correctly
  isolate the 28 mutex sites as the highest-risk category (concurrency-amplified
  and reachable via unauth HTTP).
- No High findings should be promoted to Critical. Q-C1's framing already
  captures the worst case.
- **Q-L10 should be downgraded from "Cross-Phase amplification" to a plain
  Low note about Q-C2** (silent error swallow on read_pages). The
  cross-Phase part is dead.

## Counts methodology

The audit's stated brace-tracking method is fragile (string literals like
`"not json{{{"` corrupt depth counting), but it converged on the right
numbers because most Coral files have a single `#[cfg(test)] mod tests` at
the bottom — splitting at the first `#[cfg(test)]` line gives identical
production counts. My replication of that simpler split agrees within ±2.

No false positives observed in spot-check. No false negatives in the
panic/tracing axis. `?` operators with `Context` lost are not counted by
either of us — that's a separate "error-chain quality" axis the audit
acknowledges only via Q-H7 (no actionable hints on common failures).

## Gaps not caught

1. **`unsafe { ... }` blocks (23 sites)** — not mentioned by the audit. Most
   are Rust 2024-mandated `unsafe { env::set_var }` (pgvector.rs, self_check.rs,
   runner_helper.rs) plus Win32 FFI in self_upgrade.rs (`MoveFileExW`). All
   appear correctly scoped, but a quality audit that counts `.unwrap()` should
   at minimum acknowledge the unsafe surface area (~23 sites) and confirm
   each is safe-by-construction. Particularly self_upgrade's `MoveFileExW`
   path — error handling around it is unwrap-free but the unsafe block's
   precondition (valid wide strings) is implicit.

2. **`?`-chain test coverage cross-axis** — the audit counts tracing density
   but does not cross-reference against the TEST audit's error-path coverage.
   `coral-cli` has good `anyhow::Context` discipline but no measurement of
   how many of those `?` paths have a test that exercises the error branch.
   A `?` that silently propagates a wrong error type is invisible to both
   panic-counting and tracing-counting.

## Recommendation

APPROVED_WITH_REVISIONS. Required edits before this audit ships:

1. **Mark Q-L10 as stale** in the Findings table. Replace the cross-Phase
   amplification text with a one-liner noting M2.4 closed P-H8 and that the
   Q-C2 silent-swallow remains as an independent Low/Med (not Critical-tier).
2. **Soften Q-L9** from "MUST ship with tracing" to "should ship together
   with tracing" — accurate to the diagnostic-quality, not correctness,
   nature of the amplification.
3. **Add an `unsafe { ... }` row** to the Quality dashboard (count: ~23) or
   an explicit "Out of scope" note in §Methodology. Right now it looks like
   the audit denies unsafe exists.
4. **Update orchestrator/Critical-tier list** to drop Q-L10 and downgrade
   Q-L9; only Q-L8 (SEC-01 × Q-C1) remains a true Critical-tier composition.

The audit's analytical core — mutex-poisoning under SEC-01, bootstrap
observability gap, lack of `#[instrument]` — is solid, sharp, and actionable.
The proposed "Top-3 next actions" (workspace clippy lints, parking_lot/
poisoning-tolerant mutex, `#[instrument]` on the 6 long-running ops) are
correctly prioritized and will pay back well above their ETA.

Word count: ~720.
