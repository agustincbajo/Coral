# Validation: Performance audit — Coral v0.34.1
Date: 2026-05-12
Validator: claude (sonnet)
Target: `docs/audits/AUDIT-PERFORMANCE-2026-05-12.md`

## Verdict

**APPROVED_WITH_REVISIONS**

Four of five spot-checked findings verify cleanly with exact file:line
precision. One High (P-H8) is stale: it cites a `OnceLock` cache bug in
`WikiResourceProvider` that the **M2.4 refactor already resolved** —
the current code uses `Arc<RwLock<WikiState>>` with a `mark_dirty()`
invalidation path (`crates/coral-mcp/src/state.rs:10-12`,
`resources.rs:215-226`). The auditor read a stale source comment in
`watcher.rs:7-13` and treated it as current state. One stale-but-not-
hallucinated finding does not meet the REVISE_REQUIRED threshold (3+),
but P-H8 should be downgraded to Low or rewritten before the
remediation PR lands.

## Spot-check (5 findings)

| ID | Status | Notes |
|----|--------|-------|
| P-C1 | VERIFIED | `bootstrap/mod.rs:300-328` is a for-loop with sequential `runner.run(&page_prompt)` at L310. Plan entries carry no declared dependencies; the embarrassing-parallel claim holds. Measurement `~5-15s × N` per Sonnet page is conservative — real-world tier-2 Anthropic latency is 8-20s for a 1.5K-token prompt with cache writes. The proposed `rayon::par_iter` + Semaphore(4) fix is realistic; provider Tier-2 rate limits (~10 in-flight Anthropic, ~5 Gemini) bound the parallelism, and the audit correctly recognizes this in the "Semaphore-bound" framing rather than naive unbounded `par_iter`. |
| P-C2 | VERIFIED | `cost.rs:163-182` confirmed: `estimate_cost_from_tokens` has only two arms (`input_rate`, `output_rate`); no `cache_creation_input_tokens` / `cache_read_input_tokens` fields. Test at L191-198 asserts $18/MTok for 1M+1M which would fail to update on the cached-prompt path. 5-8× overstate on repeating base prompt is mathematically sound: 1500-token base × 30 pages × ($3/MTok cache-write 1.25×) → ~$0.17 cached vs $1.35 flat-billed. |
| P-C3 | VERIFIED | `server.rs:101-111`: `match server.recv_timeout(250ms) { Ok(Some(req)) => handle(&state, req) }` is exactly synchronous-inline. `handle(&state, req)` (L115+) does not spawn. State is `Arc<AppState>` so the proposed thread-pool fix is straightforward. **Note**: this finding is the SAME bottleneck as CON-01 (Concurrency Critical). |
| P-H1 | VERIFIED | `static_assets.rs:43-71`: response builds `mime` from extension and sets only `cache` header. No `Content-Encoding` and no `.br`/`.gz` sibling lookup. `dist/assets` totals are real (`909.2 KB` matches my own `Get-ChildItem`). gzip/brotli is single-PR, validated win — arguably underclassified as High (should be Critical given UX impact + trivial implementation). |
| P-H3 | VERIFIED | `atomic.rs:73-89`: `f.write_all → f.flush` — NO `sync_all`. Confirmed asymmetry vs `atomic_write_bytes` (L146-148) which DOES chain `.and_then(\|_\| f.sync_all())`. Power-loss durability gap is real. |

Counts: VERIFIED 5 / OVERSTATED 0 / UNDERSTATED 0 / HALLUCINATED 0.

Additional ad-hoc check on P-H5: `libcoral_cli-*.rlib = 15.25 MB` confirmed locally — audit's binary-size proxy is faithful.

Additional ad-hoc check on P-H8 (outside the 5-finding budget but
relevant): **STALE**. The fix the audit describes ("Swap `OnceLock` →
`RwLock<Option<Arc<Vec<Page>>>>`") was already landed in the M2.4
refactor (see `state.rs:1-12` module doc). The current `OnceLock`
references in the source tree are either (a) tool-kind static map
(`server.rs:640-642`, unrelated) or (b) stale comments in `watcher.rs`
that pre-date M2.4. The audit reads documentation as code.

## Cross-Phase reinforcement (Concurrency + Performance Phase 2)

| Concurrency | Performance | Combined recommendation |
|---|---|---|
| CON-01 (Critical) | P-C3 (Critical) | **Single combined Critical PR.** Both findings point at `crates/coral-ui/src/server.rs:101-111`. CON-01 frames it as a head-of-line blocking *correctness* problem (one slow LLM stream starves all chrome); P-C3 frames the same code as a *latency* problem. Same fix: 8-slot static-Semaphore thread-pool around `handle(&state, req)`. Critical + Critical → top-1 Phase 2 PR. |
| CON-04 (High) | P-C1 (Critical) | **Combined Critical, must land together.** CON-04 says "any future parallelization of bootstrap needs `Arc<Mutex<cost_spent_usd>>` + atomic gate read"; P-C1 says "parallelize bootstrap NOW for 4× speedup". P-C1's `Semaphore`-bounded `par_iter` MUST include CON-04's mutex-wrapped state, or the cost cap is racy. The two audits are not contradictory — they are halves of the same PR. P-C1 carries the user-visible motivation (5min→75s); CON-04 carries the safety prerequisite. **CON-04's "High" severity is understated when paired with the Critical perf motivation.** |
| CON-06 (High) | (none direct) | CON-06 (`coral mcp serve` recv loop has no shutdown poll) has no direct perf finding but lives in the same `tiny_http` family as P-C3 / CON-01. Worth bundling into the same thread-pool PR. |
| CON-L08 | M6 | Both note `fsync`/`recv_timeout` latency floor on Windows NTFS (~5 ms). Consistent. |

**Cross-Phase ↔ Testing**: No PERF benchmark is asserted as a CI gate. The hyperfine `--warmup 5 --runs 20` budget (`.github/workflows/ci.yml:209-232`) is the only enforced perf SLO and it gates only cold-start, not bootstrap throughput, BM25 query latency, or UI first-paint. A regression on P-C1 or P-H1 would not be caught by current CI. Recommend Phase 2 add a non-blocking `criterion` benchmark ratchet job.

**Cross-Phase ↔ Security**: P-C3 / CON-01's thread-pool fix MUST preserve the `validate_host` / `validate_origin` / `require_bearer` checks (`auth.rs:24/96/62`) that TEST-01 already flagged as test-uncovered. If the per-request thread spawn moves auth out-of-band, the loopback-only guarantee can regress silently. Land the TEST-01 `auth_e2e.rs` integration test BEFORE the thread-pool PR.

## Measurement methodology

The seven declared handoffs are **mostly legitimate but partially
self-inflicted**:

- `hyperfine on Linux/macOS`: legitimate — no host available, CI already enforces budget.
- `cargo bloat`: **partially legitimate**. The audit proxies via `Get-ChildItem target/release/deps/*.rlib` sums. This is a valid *upper-bound* proxy for build cost (CI runtime) but NOT for shipped-binary symbol weight — `cargo bloat` reports `__TEXT` size of the LINKED binary, which LTO+strip flattens drastically. The audit conflates these in P-H5 ("392 MB rlib → 14.3 MB binary thanks to LTO"). Build-cost gating from rlib totals is fair; binary-cost gating is not. A `cargo install cargo-bloat` (~30s) was within budget and should have been done.
- `cargo-llvm-lines`: legitimate (install + run ~3 min, would have changed the audit's medium findings only).
- `cargo-machete`: **self-inflicted** — `cargo install cargo-machete` is ~20s; would have grounded the M3/M4 "could be feature-gated" claims in actual unused-dep detection rather than guesswork.
- `5000-page corpus`: legitimate — out of audit scope to synthesize.
- `Sigma.js graph trace`: legitimate (browser required).
- `RSS measurements`: **self-inflicted** — `Get-Process coral | Select-Object WS,PM` after `coral mcp serve` startup is a 10-second measurement.

**Extrapolations**:
- P-H2's "73 ms / 20 pages → 18 s / 5000 pages linear" is defensible for lint (structural checks are O(N) over pages with no cross-page state).
- P-H9's "30-80 ms SHA-256 on 5000×4KB" is correct order-of-magnitude (SHA-256 on modern x86 runs ~500 MB/s without AES-NI accel; 20 MB ≈ 40 ms).
- BM25 query-time scaling is NOT extrapolated — only load time. **Gap**.

**Sampling**: n=10-20 per measurement with no warmup discipline on PowerShell `Stopwatch` is single-shot grade. The hyperfine baseline in CI is more rigorous and the audit defers to it correctly. Windows-only measurement extrapolated to "Linux 12 MB" via prior `BACKLOG` references is acceptable as best-effort but explicitly flagged.

## Gaps not caught

1. **BM25 query latency vs cold-load**: audit measures `coral search "wiki"` mean=15.68ms but does not decompose `index_load + query_eval`. The interesting scaling axis (query latency at 5000-page wiki) is unmeasured. P-H9 implies query is amortized, but if the SHA-256 gate fires on each invocation, query path inherits 30-80 ms floor at scale. Severity Medium.
2. **Lock contention as perf**: Phase 2 Concurrency covers correctness of locks; PERF does not measure the *latency* cost of `with_exclusive_lock` on hot paths. `atomic_write_string`+lock at `bootstrap/mod.rs:302` (`state.save_atomic`) runs every page — on a 30-page bootstrap that is 30× lock-acquire+fsync+release, ~150-300 ms total on Windows. Once P-C1 parallelizes, this becomes lock contention. Should be a High to land WITH P-C1.
3. **`mimalloc` 10-20% claim**: cited in executive summary, never measured. Trivially verifiable via `MIMALLOC_DISABLE=1` env-flag rerun of `coral self-check --quick`.
4. **`coral self-check --quick` budget vs stale local binary**: handoff note acknowledges local binary is v0.34.0 but the audit reports `51 ms mean` against the local binary anyway. CI hyperfine measurements would have produced a fresh number against actual v0.34.1.
5. **Compilation perf**: `cargo build --release` cold/warm wall-clock not measured. CI shows ~6-8 min cold on Windows runners; the proptest / rustls / windows-sys feature-gating in P-H5 motivates this but the audit doesn't quantify the CI-cycle delta.

## Severity discipline

- **3 Critical for perf is justifiable**. P-C1 (5min→75s bootstrap) and P-C3 (head-of-line blocking) are unambiguously UX-blocking under realistic v0.34 use. P-C2 (cost overstate) is a Critical *correctness* claim (PRD §10 KPI is published) rather than perf, but the audit's classification is defensible because it directly invalidates a public KPI.
- **P-H1 (gzip) is arguably misclassified as High**. 75% wire reduction on the first-paint critical path, implementable in <1 day with `vite-plugin-compression`. Should be Critical — the audit even bundles it with P-C3 in Top-3 #3, signaling the auditor knows it's a top-priority win.
- **P-H8 should be Low (stale code reading)** per the cross-check above.

## Recommendation

Land P-C1 + CON-04 as a **single Critical PR** (parallelization + cost-mutex). Land P-C3 + CON-01 + CON-06 as a **second Critical PR** (tiny_http thread-pool, both UI and MCP servers). Land P-H1 + P-C2 as a fast-follow (each ~1 day). Defer P-H8 pending a re-read of `state.rs` and `resources.rs` against the M2.4 refactor; it may already be CLOSED. Verify the M6/CON-L08 `recv_timeout(250ms)` floor before any latency-sensitive UI work.
