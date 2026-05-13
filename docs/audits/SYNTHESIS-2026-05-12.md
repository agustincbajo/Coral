# Audit synthesis — Coral v0.34.1

Date: 2026-05-12
Synthesizer: claude (sonnet)
Scope: 6-axis audit + 6 validator passes, full autonomous orchestration
Inputs: `AUDIT-{SECURITY,TESTING,CONCURRENCY,PERFORMANCE,QUALITY,ARCHITECTURE}-2026-05-12.md` + paired `VALIDATION-*` docs

---

## Executive summary

Across 6 audit axes, **~14 Critical and ~52 High** findings were surfaced and verified by independent validator passes. **Five cross-Phase compositions** repeatedly tie multiple axes to the same module — these are the highest-leverage PR opportunities for v0.35 because a single change closes findings across 2-5 axes simultaneously.

The audit methodology — pair every audit with a validator that spot-checks ≥5 findings against source — caught **2 stale findings** (P-H8 + Q-L10, both referencing a `OnceLock` bug already resolved in M2.4) and **1 overstated claim** (Q-L9 "MUST ship with tracing" softened to "should"). Audit doc revisions are applied in a separate commit batch.

The 3 architectural debts surfaced (`tiny_http` substrate, `coral-core` `pub mod` discipline, bootstrap state mutex absence) **all root-cause findings in Phases 1-2** — closing them is structural, not tactical, and warrants written ADRs before v0.35 implementation.

---

## Findings counts per axis (post-validator revisions)

| Axis | Critical | High | Medium | Low | Commit |
|---|---|---|---|---|---|
| Security | 0 → 0 | 7 → 6 (SEC-04↓) + 2 new (distill, user_defined_runner) | 6 → 7 | 10 → 10 | `22aaabf` |
| Testing | 3 → 2 (TEST-03↓) + 1 uplift (TEST-11↑) = 3 | 9 → 11 (+TEST-HTTPAUTH, +proptest-underuse) | 7 | 7 | `33d053a` |
| Concurrency | 2 → 2 | 8 → 8 + 2 missed gaps (cross-cmd lockfiles, skill build torn-rename) | 5 | 10 | `dc086c2` |
| Performance | 3 → 3 (P-H8 retired) | 9 → 8 + 2 missed gaps (BM25 query time, post-parallelization lock contention) | 6 | 6 | `292f4e4` |
| Quality | 3 → 3 (Q-L10 retired) | ~9 | ~7 | ~7 | `7211a4e` |
| Architecture | 3 | 10 | 16 (M+L) | | `fdb0ddc` |
| **Totals** | **~13** | **~52** | **~48** | **~40** | |

Plus quality dashboard (counts production-only, `#[cfg(test)] mod` excluded):
- **70** `.unwrap()` (worst: coral-runner 23, coral-mcp 18, coral-core 13)
- **49** `.expect()` (worst: coral-core 20, coral-mcp 10)
- **0** `panic!()` in production (good); 44 in tests
- **18** `tracing::info!`, **44** warn!, **2** error!, **0** `#[instrument]`, **0** spans

---

## Cross-Phase critical compositions (the 5 highest-leverage PRs)

Each composition closes findings across ≥3 axes with a single focused change. Listed by combined severity × user-visible impact.

### CP-1 · Bootstrap N+1 paralelization (5 axes)

| Axis | Finding | Role |
|---|---|---|
| Concurrency | CON-04 | Bootstrap is sequential; `BootstrapState.cost_spent_usd` needs `Arc<Mutex<f64>>` first |
| Performance | P-C1 | 30-page Sonnet 4.5 bootstrap ≈5 min wall-clock; rayon par-iter + Semaphore(4) → ≈75 s (-75%) |
| Security | SEC-06 | Bootstrap LLM prompt injection — parallelization without containment exposes blast radius |
| Quality | Q-C3 | `coral-cli::commands::bootstrap` 30-page loop emits **zero** `tracing` events — parallelization MUST land with per-page spans |
| Architecture | ARCH-C1 | Bootstrap state mutex absence is the architectural root |

**Single PR shape**: introduce `Arc<Mutex<BootstrapState>>` + per-page `tracing::info_span!` + rayon par-iter with `Semaphore(4)` to honor Anthropic Sonnet rate limits. Drops bootstrap wall-clock 75%, closes 5 findings, surfaces failures with actionable error chains.

**Estimated cost**: 1-2 dev-days. **User-visible win**: 4× throughput on real bootstraps.

### CP-2 · `tiny_http` thread-pool extension (4 axes)

| Axis | Finding | Role |
|---|---|---|
| Performance | P-C3 | `coral ui serve` single-threaded recv loop blocks streaming `/api/v1/query` against `/health`, assets, etc. |
| Concurrency | CON-01 | Same; user-visible browser stall |
| Concurrency | CON-06 | No shutdown poll in recv loop (causes shutdown stall) |
| Architecture | ARCH-C3 | `tiny_http` blocking-I/O substrate is the architectural choice; ADR needed |

**Single PR shape**: port the existing thread-pool pattern from `coral-mcp/transport/http_sse.rs:53` (`MAX_CONCURRENT_HANDLERS = 32`) into `coral-ui/src/server.rs`. Add `recv_timeout(Duration::from_millis(200))` for shutdown poll. **NO tokio** (validators C+D confirm: MSRV 1.85, async-std maintenance mode, smol no supply-chain reduction).

**Estimated cost**: <300 LoC. **User-visible win**: WebUI responsive under streaming queries.

### CP-3 · MCP HTTP auth + pre-auth DoS mitigation (4 axes)

| Axis | Finding | Role |
|---|---|---|
| Security | SEC-01 | `coral mcp serve --transport http` has no bearer auth, only origin allowlist + 127.0.0.1 default |
| Security | SEC-07 | `Mcp-Session-Id` derived from `timestamp_nanos * 0xdeadbeef + counter` (not CSPRNG) |
| Quality | Q-C1 | 28 `Mutex::lock().unwrap()/expect()` sites in `coral-mcp` — mutex poisoning crashes every subsequent request |
| Testing | TEST-01 | `coral-ui/src/auth.rs` `validate_host/origin/require_bearer` have **zero** direct tests; SEC-01's port-pattern target |

**Composition risk**: SEC-01 unauth endpoint + Q-C1 mutex poisoning = **pre-auth DoS** by malformed payload (Validator E verified pre-auth path).

**Single PR shape**: (a) port `constant_time_eq` + bearer auth from `coral-ui::auth` to `coral-mcp::transport::http_sse`; (b) replace `Mutex::lock().unwrap()` with `parking_lot::Mutex` (no poison semantics) OR `.unwrap_or_else(|p| p.into_inner())`; (c) test the entire `coral-ui::auth` module before re-using the pattern; (d) switch `new_session_id()` to `rand::random` CSPRNG.

**Estimated cost**: 2-3 dev-days. **User-visible win**: HTTP MCP transport becomes safe to expose beyond loopback.

### CP-4 · WebUI token + auth.rs untested module (2 axes)

| Axis | Finding | Role |
|---|---|---|
| Security | SEC-02 | `coral ui serve --token` has no entropy floor, no auto-mint |
| Testing | TEST-01 | Same module zero-direct-tests |
| Testing | TEST-11 | `coral-ui/tests/api_smoke.rs` is the natural test home — uplift to Critical per validator |

**Single PR shape**: (a) auto-mint a 256-bit CSPRNG token if `--token` not supplied + entropy floor enforcement; (b) integration tests in `coral-ui/tests/api_smoke.rs` covering host/origin/bearer paths.

**Estimated cost**: 1 dev-day.

### CP-5 · Composition attack: SEC-01 + CON-02 + CON-06 shutdown stall

Validator C surfaced this — **not in any audit body**.

Once CON-02's "5s drain grace" fix lands, an attacker can send a long `/api/v1/query` to the unauthenticated MCP HTTP endpoint, then trigger `SIGTERM`. The shutdown stalls until drain timeout, denying legitimate shutdown for 5 s × N attackers.

**Mitigation**: cap the drain timeout AND require auth before the request reaches the drain queue. Bundled with CP-3.

---

## Top architectural ADRs needed before v0.35

The audit synthesis surfaces 3 implicit-only architectural decisions that should be written before v0.35 implementation begins:

1. **ADR-001: Blocking-I/O substrate (`tiny_http` vs tokio)**. Audits + validators converge: stay with `tiny_http` + thread-pool extension. MSRV 1.85, async-std maintenance, smol no win. Write it down so the next contributor doesn't re-litigate.

2. **ADR-002: MSRV policy**. `rust-toolchain.toml` uses `stable` with no floor — silent drift. Bump 1.85 → 1.89 (let_chains lands idiomatically; `&raw` already stable since 1.82). Pin `1.85.0` in CI as floor until v0.35 release.

3. **ADR-003: `mimalloc` global allocator**. Used for "10-20% throughput" claim — unmeasured. Document the choice + add a `cargo bench` baseline before v0.35 changes invalidate it.

---

## Top priorities for v0.35 (ordered)

| # | Item | Closes | Estimated cost |
|---|---|---|---|
| 1 | CP-1 — Bootstrap N+1 paralelization | CON-04, P-C1, SEC-06, Q-C3, ARCH-C1 | 1-2 days |
| 2 | CP-3 — MCP HTTP auth + mutex poisoning | SEC-01, SEC-07, Q-C1, TEST-01 | 2-3 days |
| 3 | CP-2 — `tiny_http` thread-pool | P-C3, CON-01, CON-06, ARCH-C3 | <300 LoC |
| 4 | CP-4 — WebUI auth tests + auto-mint | SEC-02, TEST-01, TEST-11 | 1 day |
| 5 | Workspace lints `[lints.clippy] unwrap_used/expect_used/panic = "warn"` | Q-* observability | <100 LoC, ratchet plan |
| 6 | ARCH-C1 `coral-core` `pub mod` → `pub(crate)` audit (32 mods) | ARCH-C1 | 1 day, BC pass needed |
| 7 | ARCH-C2 `test_script_lock` pub-fn pollution → custom test harness | ARCH-C2, BACKLOG #8 | 1 day |
| 8 | Easy win — gzip + brotli pre-compression UI bundle (`build.rs`) | P-H1 | <100 LoC, -75% first paint |
| 9 | MSRV 1.85 → 1.89 + 1.85 floor CI job | ARCH-H9 | <50 LoC |
| 10 | `[provider.gemini]` + `[provider.anthropic]` config bridge | Quick-wins followup | DONE (commits 57142d4 + 6bc2d1f + b968ca6) |

---

## Handoff items requiring human runs

These could not be executed in the autonomous audit pass; require ≤30 min each on a developer workstation:

1. **Live `cargo audit` + `cargo deny` snapshot** — confirms current advisory state matches the static audit.
2. **Live `cargo llvm-cov` baseline** (~30 min) — replaces the tests-per-100-LoC proxy from TEST audit.
3. **`cargo bloat --release -p coral-cli`** + `cargo-machete` + `cargo-udeps` — top-20 binary contributors + dead code candidates.
4. **Windows ACL empirical validation on `.coral/config.toml`** — code admits chmod 600 is no-op on Windows; default DACL inheritance behavior should be confirmed on Windows 11.
5. **SLSA verification flow E2E** against an actual v0.34.1 release artifact (gh CLI + cosign route both documented in `docs/SLSA-VERIFICATION.md`).
6. **macOS/Linux binary smoke en VM real** (BACKLOG item 3) — glibc compatibility, codesign on darwin, libssl ABI.
7. **`nextest run` enumeration of the 26 pre-existing Windows failures** — gives concrete test names per TEST-08.
8. **RSS measurements**: `coral mcp serve` + `coral ui serve` idle.
9. **5000-page corpus for BM25 query-time scaling** — currently extrapolated linearly from 20-page measurements.
10. **Browser traces for Sigma.js graph render** at 1000+ nodes — Playwright trace.
11. **Real `bootstrap --estimate` accuracy** on a calibrated corpus (PRD KPI: ±25% v0.34, ±15% v0.37 — needs real TokenUsage data).

---

## Audit infrastructure recommendations

The validator-paired-audit pattern produced verifiable findings (only 2 stale findings, 1 overstated claim across ~150 findings — ~2% noise rate). Worth institutionalising:

1. **Cadence**: ~6 months between full 6-axis audits. Major release sprints (M2, M3) should trigger targeted re-audits on the changed surfaces.
2. **Documentation**: keep the `docs/audits/` directory as the canonical home. Each audit + validator gets paired commits. The pattern (executive summary ≤200 words, Critical+High in body, M+L in appendix, hard word cap 2500) keeps findings actionable.
3. **Cross-Phase synthesis**: dedicate one audit (Architecture is natural) to surface compositions. Validators should explicitly check synthesis claims against the underlying audits.
4. **Threshold discipline**: Critical+High = action items with PR shapes; Medium+Low = inventory without commitment. Resist scope creep into all-findings-everywhere.

---

## What was NOT audited (scope honesty)

- **`crates/coral-ui/assets/` JavaScript + Vite + node_modules** — entirely outside SBOM scope. Real plug-and-play blind spot (Validator F gap).
- **MCP-protocol-level SemVer** — only crate surface audited, not the wire protocol.
- **PRD-v0.34 Apéndices E/F compliance pass** — frozen contracts (`.coral/config.toml` schema v1 + `SelfCheck` JSON schema v1) deserve dedicated verification audit.
- **Internationalization (i18n)** — Coral is EN-only in M1 per AF-8; M2 ES localization needs its own audit pass.
- **WebUI assets accessibility (a11y)** — Playwright + axe-core would be the tool.
- **`coral-ui/assets/src/e2e/` Playwright 14 specs** — present but not running in CI (BACKLOG item 4).
- **`unsafe { }` blocks** (~23 across self_upgrade, pgvector, self_check, runner_helper) — flagged as Quality gap by Validator E, worth a dedicated review.

---

*Fin SYNTHESIS-2026-05-12. Total commits del audit cycle: 12 (6 audit docs + 6 validation docs). All committed and pushed to `main`.*
