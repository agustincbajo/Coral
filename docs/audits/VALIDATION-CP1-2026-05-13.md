# Validation: CP-1 Bootstrap parallelization — v0.35 sprint

Date: 2026-05-13
Validator: claude (opus 4.7, 1M ctx)
Targets: 6 commits 604da88..44d0a8c (all in `crates/coral-cli/src/commands/bootstrap/mod.rs`)

## Verdict

**APPROVED_WITH_REVISIONS** — code is correct, tests pass, the two-phase
collect-then-apply design is sound, but two dev-facing claims are
**overstated** (4× speedup, 210+ tracing events). Neither blocks ship.

## Spot-check results

| Aspect | Status | Notes |
|---|---|---|
| collect-then-apply avoids mutex | **VERIFIED** | Phase B (`pool.install(\|\| ... par_iter ... )` at `mod.rs:390-419`) only reads `state.plan[i]` (immutable) and calls `render_page_body` (pure: no `&mut self`, returns owned `(String, Option<TokenUsage>)`). No `state.pages[i] = ...`, no `state.cost_spent_usd += ...`, no `state.save_atomic`. Workers genuinely don't touch shared state. Phase C is sequential. **No mutex needed.** |
| 4× speedup measured | **OVERSTATED** | Dev claim "4× speedup (mock 12 pages × 40ms: 480ms→120ms)". `parallel_apply_honors_thread_pool_cap` test (12 pages × 40ms sleep) clocks **0.21–0.36s** in nextest, not 120ms. Theoretical lower bound is 3 batches × 40ms = 120ms; observed 213ms means ≈ **2.25× wall-clock speedup over 480ms serial**, not 4×. Architecture supports up to 4× when per-page latency dominates (real Sonnet calls @ 5s each → 30 pages: 150s serial vs ~40s parallel ≈ 3.75×), but the **test does not assert any speedup ratio**. Recommend: either re-word the claim ("up to 4× when worker latency dominates fixed overhead") or add a serial-baseline comparison test. |
| SEC-06 injection check covers threat | **VERIFIED (with caveat)** | `coral_lint::structural::check_injection` (lint/structural.rs:269-316) scans for `<\|system\|>`, `</user>`, `<\|assistant\|>`, `Authorization:`, `Bearer `, `x-api-key`, long base64 runs, U+202E bidi-override and tag chars. Called at `mod.rs:497` post-`build_page`, pre-`page.write()`. The test `injection_check_blocks_suspicious_body` confirms `<\|system\|>` triggers refusal. **Caveat**: original SEC-06 threat in audit is "LLM-generated markdown is parsed by frontmatter without injection check" — the actual fix runs *after* `build_page` (which already parsed/built frontmatter). Frontmatter-poisoning via injected YAML keys at the head of the body is not covered (the check looks inside `page.body`, not at frontmatter that the LLM may have prepended). Still closes the documented threat in the audit table (pages with prompt-injection markers do not land on disk), but the frontmatter-injection sub-threat would need a separate check. Acceptable for v0.35 ship. |
| 210+ tracing events / 30 spans | **UNDER (significantly)** | Actual count in `bootstrap/mod.rs`: 1× `#[tracing::instrument]` (`apply_pages`) + **3× `tracing::info_span!`** (page, page.render, page.apply) + **13× event macros** (`tracing::info!`/`warn!`/`error!`/`debug!`). Per-page execution emits ~3 spans + ~3–4 events. For 30 pages: ~90 spans + ~110 events (90 per-page + ~20 outer). The "210+ events + 30 spans" claim is **off by ~2×**. Q-C3 (replace `eprintln!` with tracing on the per-page failure path) is still **closed** — the failing-page paths now emit `tracing::error!` with `error = %e` and `tracing::warn!` for skip. The instrumentation count is healthy and adequate for `RUST_LOG=coral=debug` triage; only the marketing number is wrong. |

## Cost-gate trade-off assessment

Dev correctly documents the worst-case overspend at `mod.rs:447-451` (3 in-flight pages can complete after the gate decision before the next batch is started, bounding overspend at `(BOOTSTRAP_MAX_PARALLEL - 1) × per-page-cost ≈ 3 × $0.02 = $0.06`).

- **`max_cost_midflight_halts_and_marks_partial` passes** under the new parallel apply path (verified live: 0.207s, exit code 2, state.partial=true). ✓
- The gate now checks **after** Phase B returns (post-collect), so up to BOOTSTRAP_MAX_PARALLEL pages may have already been paid for. Code comment at `mod.rs:447-451` clearly calls this out and links to the P-H4 v0.36 follow-up.
- **No dedicated overspend test.** A test that asserts "overspend ≤ 3 × projected" would be valuable but its absence does not block ship — the bound is structurally guaranteed by the par-iter cap.
- $0.06 worst-case is acceptable for a $1–$5 typical bootstrap; documented; deferred fix tracked.

**Verdict: acceptable for v0.35.**

## Resume safety

- `resume_completes_pending_pages_under_parallel_path` passes (0.210s). Two-page seed state with both `Pending` runs through Phase B, both land Completed, no plan call. ✓
- `resume_skips_completed_pages_and_finishes_pending` (pre-CP-1) still passes. ✓
- Lockfile: `_lock = BootstrapLock::acquire(&root)?` at both `mod.rs:162` (resume path) and `mod.rs:233` (fresh apply). Held across the parallel section because `_lock` lives in the outer scope. ✓
- `InProgress` from a crashed prior run: not explicitly tested. The classification loop at `mod.rs:313` skips only `Completed`; `InProgress` pages re-enter `to_render` and get a fresh runner call. Idempotent on the page write side (atomic_write), but a duplicate cost charge accumulates on resume. Minor; documented at `coral bootstrap --resume` UX level.
- Plan re-use: `BootstrapState::load` deserializes the full plan; resume never re-calls the planner (asserted in `resume_completes_pending_pages_under_parallel_path:2027`). ✓

## Cross-audit closure verification

| Finding | Audit doc | Closure verdict |
|---|---|---|
| CON-04 | AUDIT-CONCURRENCY-2026-05-12.md:19 | **CLOSED** — cost gate is now read+written from the sequential Phase C only. No race possible because Phase B never reads `cost_spent_usd`. Note: original audit prescribed `Arc<Mutex<BootstrapState>>`; dev chose a stricter "no shared state" pattern, which is *better* than the audit ask. |
| P-C1 | AUDIT-PERFORMANCE-2026-05-12.md:18 | **CLOSED** — rayon par-iter with dedicated `ThreadPoolBuilder` (4 workers, named `bootstrap-worker-{i}`) at `mod.rs:377`. Audit explicitly asks for "rayon par_iter over state.pages with semaphore-bounded N=4 in-flight" — delivered. Speedup claim is overstated (see above) but the mechanism is correct. |
| SEC-06 | AUDIT-SECURITY-2026-05-12.md:29 | **CLOSED (primary threat)** — injection-marker bodies are rejected at ingest time. Audit ask was "Run `coral_lint::structural::check_injection` over each LLM-emitted body before `build_page` accepts it" — implementation runs it after `build_page` but before `page.write()`, which is equivalent for the threat (poisoned content never lands on disk). |
| Q-C3 | AUDIT-QUALITY-2026-05-12.md:20 | **CLOSED** — per-page failure path now uses `tracing::error!(error = %e, ...)` at `mod.rs:410-414` and `mod.rs:551` for completion. `info_span!("page", idx, slug, page_type)` wraps each iteration. Audit ask delivered. (Event-count marketing claim aside.) |
| ARCH-C1 | AUDIT-ARCHITECTURE-2026-05-12.md:18 | **MISMATCH / NOT CLOSED HERE** — original ARCH-C1 is **"`coral-core` exposes 33 `pub mod`"** (SemVer surface reduction). Dev marks ARCH-C1 closed via the "collect-then-apply pattern avoids mutex" decision. That maps to **architectural debt #3** in `AUDIT-ARCHITECTURE-2026-05-12.md:113` ("Bootstrap state mutex"), which is **a discussion-level concern, not a finding row**. The actual `ARCH-C1` finding (`pub mod` surface) is **untouched by CP-1** — no `coral-core/src/lib.rs` edits, no `pub(crate)` demotions. Recommend the dev re-tag this closure as "architectural debt #3" rather than "ARCH-C1". |

## Workspace state

- `cargo check --workspace`: **clean** (0.41s)
- `cargo clippy --workspace --all-targets -- -D warnings`: **clean** (7.71s)
- `cargo fmt --check`: **clean** (no output)
- `cargo nextest run -p coral-cli bootstrap::`: **34/34 passed** (496ms)
- `cargo nextest run --workspace --no-fail-fast`: **1874 passed / 26 failed / 19 skipped** — all 26 failures are pre-existing Windows env issues (release_flow `.sh` tests, gemini/local echo-substitute tests, template_validation, multi_repo git scenarios, coral-stats schema). **Zero failures touch bootstrap surface.**

## Recommendation

Ship CP-1 in v0.35 with the following non-blocking revisions in the changelog / commit messages:

1. **Re-word "4× speedup"** to "up to 4× when per-page latency > fixed overhead; ~2.25× measured on the mock 40ms-per-page test." The architecture supports 4× on real Sonnet timings (5s per call), but unconditional claims overshoot the data.
2. **Re-tag ARCH-C1 closure as "architectural debt #3"** in CHANGELOG / commit footers. The actual ARCH-C1 (33 pub mod surface) is not touched by CP-1.
3. **Re-state tracing instrumentation** more accurately: "~3 spans + 3–4 events per page → ~100–130 events for a 30-page bootstrap", not "210+ events + 30 spans".
4. **Optional follow-up**: add a regression test that asserts cost-overspend bound ≤ `(BOOTSTRAP_MAX_PARALLEL - 1) × per-page-projected`. Not blocking.

Findings tally: **5 of 5 listed closed** in the spirit dev claims, with one mis-tagged (ARCH-C1 → debt#3) and two marketing claims overstated. All structural fixes land correctly; nothing is misimplemented.
