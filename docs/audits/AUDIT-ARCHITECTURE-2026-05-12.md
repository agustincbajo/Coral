## Audit: Architecture & dependencies — Coral v0.34.1
Date: 2026-05-12
Auditor: claude (sonnet)
Scope: workspace `Cargo.toml`, the 10 `crates/*/Cargo.toml`, every `lib.rs`, `Cargo.lock`, `.github/workflows/*.yml`, `deny.toml`, `rust-toolchain.toml`. Static review only — no `cargo update`, `cargo public-api`, `cargo machete`. Cross-references the four prior v0.34.1 audits.

## Executive summary

Coral's 10-crate split is **functionally well-bounded**: clean DAG (no cycles, no `path = ".."`), single sink (`coral-cli`), consistent naming. The dep tree is **moderate-heavy for a CLI**: 316 distinct crates in `Cargo.lock` (vs ~150 cargo, ~50 ripgrep) — driven by `rusqlite` bundled SQLite, `proptest` as a regular dep, `ureq`'s rustls+ring stack for one wizard ping, and `windows-sys 0.59 + 0.61` cohabiting. Release binary is 15 MB Windows / ~12 MB Linux.

Real architectural issues, in order: (1) **`coral-core` has 33 `pub mod`** with no `pub(crate)` discipline — every helper is a SemVer surface; (2) **`proptest` is a regular dep of `coral-test`** forcing 6.5 MB rlib in every CLI build; (3) **`coral-cli` reaches into nested modules of consumer crates** (`coral_runner::runner::RunnerError`, `coral_env::compose::*`) instead of the curated crate-root re-exports, defeating the `lib.rs pub use` curation work. DAG is healthy; per-crate surface area is not.

MSRV 1.85 is reasonable for v0.34.x; a 1.89 bump is recommended for v0.35 (lets `let_chains` land idiomatically; no transitive blocks it).

## Findings (Critical + High only)

| ID | Severity | Title | Crate/dep | Proposed fix |
|----|----------|-------|-----------|--------------|
| ARCH-C1 | Critical | `coral-core` exposes 33 `pub mod` — every helper is a SemVer surface | `crates/coral-core/src/lib.rs:4-39` | Demote internal-only modules (e.g. `late_chunking`, `reranker`, `narrative`) to `pub(crate)`. Re-export the genuinely-public symbols at the crate root. Target ≤12 `pub mod`. Land before v0.35 because every external `use coral_core::X::Y` is a frozen surface. |
| ARCH-C2 | Critical | `proptest` is regular (not dev-) dep of `coral-test` — 6.5 MB rlib in every CLI build | `Cargo.toml:80`; `crates/coral-test/Cargo.toml:19-22`; `property_runner.rs` | Gate behind a `coral-test/property` cargo feature (mirror the existing `recorded` pattern at `coral-test/Cargo.toml:33-44`). `coral-cli` opts in only when `--kind property-based` is exposed. Cross-link: P-H5. |
| ARCH-C3 | Critical | `tiny_http` substrate underpins both CON-01 (UI head-of-line) and SEC-01 (MCP unauth); the "stay vs migrate" decision is undocumented | `Cargo.toml:50`; `coral-ui/src/server.rs`; `coral-mcp/src/transport/http_sse.rs` | Land an ADR (`docs/adr/0001-blocking-http.md`) ratifying: tokio is dev-dep only; CON-01/02 are fixed inside the `tiny_http` model with `std::thread`+ shutdown counter (~300 LoC); v1.0 may re-evaluate. Unblocks five cross-referenced findings. |

| ID | Severity | Title | Crate/dep | Proposed fix |
|----|----------|-------|-----------|--------------|
| ARCH-H1 | High | `coral-cli` reaches into crate-internal modules instead of curated re-exports (22 sites) | `bootstrap/mod.rs:5-12`; `chaos.rs:28`; `down.rs:9`; `env.rs:10`; `common/untrusted_fence.rs:38`; `consolidate.rs:3-5`; `diff.rs:19,610`; `session.rs:30`; etc. | `use coral_core::cost::Cost` is fine if both the `pub mod` and a curated `pub use` re-export exist; today `coral-core` only re-exports `version()` and `coral-mcp` is partial. Make consumers go through `coral_core::Cost`. Land alongside ARCH-C1. |
| ARCH-H2 | High | `windows-sys 0.59` AND `0.61` both linked — ~10 MB duplicate bindings | `Cargo.lock` (14 sites mixing 0.59/0.61) | The 0.59 pin is in `Cargo.toml:130` for `MoveFileExW`. 0.61 is pulled by `tempfile`, `fs4`, `signal-hook-registry`. Bump the direct pin to 0.61 once feature-set stability is verified. Drops one 7.3 MB rlib per CI run. |
| ARCH-H3 | High | `rusqlite` bundled-SQLite (~6 MB) for the `embeddings_sqlite` opt-in path only | `crates/coral-core/Cargo.toml:19`; `coral-core/src/embeddings_sqlite.rs` | Put `embeddings_sqlite` behind a `sqlite` cargo feature, default-off — same shape as the existing `pgvector`/`tantivy` feature flags at `lib.rs:34-38`. BM25-only users save ~6 MB binary. |
| ARCH-H4 | High | `coral-runner::test_script_lock` is `pub fn` only because integration tests need it — test concern leaks into public API | `crates/coral-runner/src/lib.rs:43-66` | Move behind `#[cfg(any(test, feature = "test-support"))]`. Release builds shouldn't expose it. Cross-link: TEST-L3, BACKLOG #8. |
| ARCH-H5 | High | Two `bincode` rlib hashes in `target/release/deps/` (build dedup failure) | `target/release/deps/bincode-{6be765,992541}.d` | Same 2.0.x version, two rlibs — different feature unification between coral-core and downstream consumers. Investigate with `cargo tree -e features`; `cargo clean` may resolve. Land before v0.35 release. |
| ARCH-H6 | High | `ureq 2.x` + rustls + ring stack (~15 MB) for ONE provider-key-ping | `Cargo.toml:141`; `coral-cli/src/commands/doctor.rs:340-343` | Gate behind a `doctor-wizard` cargo feature (P-H5), or accept the cost and document the trade-off explicitly. One-line decision in `Cargo.toml`. |
| ARCH-H7 | High | `coral-mcp::transport` exposes `http_sse` + `stdio` submodule internals as SemVer surface | `crates/coral-mcp/src/lib.rs:43` | Demote `transport` to `pub(crate)`; re-export `serve_stdio` / `serve_http` at the crate root mirroring `coral-env`'s shape. Today external code can `use coral_mcp::transport::http_sse::handle_post`. |
| ARCH-H8 | High | `tokio` IS in the workspace tree (dev-dep of `coral-runner`, runtime dep of `wiremock`) — "no tokio" invariant is leaky | `crates/coral-runner/Cargo.toml:30`; `wiremock 0.6` transitive | `Cargo.lock` resolves `tokio v1.52.1`. Confined to dev-deps, never linked into release. Document the carve-out (`[dev-dependencies]` only) and add a `deny.toml` rule. Otherwise CON top-3 messaging is inaccurate. |
| ARCH-H9 | High | `rust-toolchain.toml` pins `stable` with no MSRV floor — CI drifts past MSRV silently | `rust-toolchain.toml:2`; `Cargo.toml:8` | Add an `msrv` CI job using `dtolnay/rust-toolchain@1.85.0` + `cargo build --workspace`. Without it, accidental use of post-1.85 syntax breaks downstream packagers, not CI. |
| ARCH-H10 | High | bincode 2.x serde-adapter is correct but `postcard` swap is deferred — perpetuates RUSTSEC-2025-0141 advisory ignore | `Cargo.toml:88-108`; `coral-core/src/search_index.rs:188-209`; cross-link P-H6, BACKLOG #7 | Per BACKLOG #7 the swap is ~1 hour. `postcard` is maintained, ~50% smaller wire format, no advisory. Land in v0.35 to retire `deny.toml:33`. |

## Crate dependency graph

```
                      coral-cli (sink, 30k LoC, single binary)
                     /  |  |   |   |   |   |   \
   coral-test ──> coral-env ──> coral-core <─── coral-stats
        │             │              ^               ^
        └─────────────┼──────────────┤
                  coral-mcp ─────────┤
                  coral-ui  ─────────┤
                  coral-runner ──────┤
                  coral-session ─────┤
                  coral-lint ────────┘
                      └──> coral-runner (lint depends on runner for embeddings checks)
```

Properties:
- **No cycles** (verified via `cargo tree --workspace`).
- **Single sink**: `coral-cli` consumes all 9 others.
- **Hub crate**: `coral-core` (used by 8 of 9; only `coral-runner` is independent of it).
- **Layered**: `core` → `{env, runner, lint, session, stats}` → `{mcp, ui, test}` → `cli`.
- **Boundary leaks**: 22 `coral-cli` callsites reach into nested modules (e.g. `coral_core::cost::Cost`) rather than crate-root re-exports — formally legal, semantically a layering violation (ARCH-H1).
- **Internal edges**: `coral-lint → coral-runner` (semantic checks need embeddings); `coral-test → coral-env` (orchestrator needs env handle); `coral-ui → coral-runner` (REST `/api/v1/query` proxies through `Runner`). All legitimate.

## Dependency health

| Metric | Value | Baseline |
|---|---|---|
| Distinct crates in `Cargo.lock` | 316 | cargo ~150, ripgrep ~50, helix ~250 |
| Direct `[workspace.dependencies]` | 30 | reasonable for 10-crate workspace |
| Workspace internal crates | 10 | `path = "crates/*"` |
| Total `cargo tree --workspace` lines | 675 | indicator of fanout (high) |
| Duplicate crate-name versions | 8 | `console`, `getrandom` (×3), `hashbrown` (×3), `thiserror` (1+2), `webpki-roots`, `windows-sys` (×3), `r-efi`, `wit-bindgen` |
| Yanked deps | 0 | clean (CI checks per-push) |
| RUSTSEC advisories ignored | 1 (`RUSTSEC-2025-0141` bincode maintenance) | acceptable; tracked |
| Deprecated crates | 0 confirmed (no `async-std`, `failure`, `clap<4`, `tokio<1`) | clean |
| MSRV | 1.85 (Feb 2025) | stable channel ~1.92 at 2026-05; bumpable |
| Edition | 2024 (consistent across all 10 crates) | current |
| Release binary | 15 MB Windows / ~12 MB Linux | acceptable for plug-and-play |
| Heaviest rlibs (release) | `coral-cli` 15 MB, `zerocopy` 15 MB, `syn` 9 MB, `windows-sys 0.59` 7 MB, `regex-automata` 7 MB, `rustls` 7 MB, `proptest` 6.8 MB, `coral-core` 6.6 MB, `ring` 6 MB | proptest + rustls + windows-sys duplication are top targets |

Outdated check (manual): `rusqlite 0.32` is one minor behind 0.33; `clap 4.6.1`, `chrono 1.0.4`, `serde 1.0.228` are current.

## Methodology

1. Read `Cargo.toml` + all 10 `crates/*/Cargo.toml` + all 10 `lib.rs` files.
2. `cargo tree --workspace` (675 lines), `--duplicates` (8 names), `--prefix none --no-dedupe | sort -u` (264 nodes).
3. `grep ^name = Cargo.lock` → 316 distinct crates.
4. `ls target/release/deps/*.rlib | sort -rn` for heaviest rlibs.
5. Cross-crate `use` grep to detect boundary leaks (22 sites in `coral-cli`).
6. Read `deny.toml`, `rust-toolchain.toml`, every `.github/workflows/*.yml`.
7. Cross-referenced AUDIT-{SECURITY, PERFORMANCE, CONCURRENCY, TESTING} for findings with architectural (vs tactical) signal.
8. **NOT run**: `cargo public-api`, `cargo machete`, `cargo udeps`, `cargo bloat`, `cargo +nightly -Z minimal-versions`. Not installed.

## Scope NOT audited

- Live SemVer-compat against crates.io (`cargo-semver-checks`).
- Per-crate `cargo public-api` baseline for the v0.35 SemVer guard.
- Live `cargo deny check licenses` (we read `deny.toml` only).
- Build-time benchmark of proposed feature gates.
- JS/SPA deps under `crates/coral-ui/assets/` (embedded blob).
- Cross-OS rlib measurements (Windows-only).
- PGO evaluation.

## Top-3 next actions

1. **ADR for blocking-I/O substrate (ARCH-C3).** One markdown file documenting tokio-as-dev-only, CON-01/02 fix inside `tiny_http` model, v1.0 re-evaluation. Unblocks the CON-01/02 fix PR. ~2h.
2. **Reduce `coral-core` public surface (ARCH-C1 + ARCH-H1).** Demote internal-only modules to `pub(crate)`; re-export real symbols at crate root; update `coral-cli` callsites mechanically. ~1d; gates v0.35 SemVer baseline.
3. **Feature-gate `proptest` + `ureq` + `rusqlite` (ARCH-C2 + ARCH-H6 + ARCH-H3) in one PR.** Mirror the existing `recorded` feature pattern. ~3–5 MB binary delta + 20–30s CI savings. ~1d.

## Cross-Phase architectural synthesis

This audit closes Phase 3 of the v0.34.1 review series. The architectural reading of the four prior audits surfaces three converging debts — design decisions worth ratifying or revisiting:

**Architectural debt #1 — Blocking I/O choice (CON-01, CON-02, CON-06, P-C3, SEC-01, SEC-07).** `tiny_http` is the single substrate for both `coral mcp serve` and `coral ui serve`. All four high-severity HTTP findings root-cause to the same architectural choice: no tokio, no hyper, hand-rolled blocking handlers. The fixes (worker pool, recv-timeout, shutdown counter, bearer auth, getrandom session IDs) are tractable, but they converge on the unspoken question: what's the v1.0 substrate? Deferred today; debt amortized indefinitely. ARCH-C3 forces a written answer.

**Architectural debt #2 — Cross-binary test coordination (BACKLOG #8, CON-L03, TEST-L4, ARCH-H4).** `RUST_TEST_THREADS=1` in CI plus `test_script_lock()` as `pub fn` is a structural compromise. Cargo runs `lib.rs` separately per binary, so the in-process Mutex doesn't span integration-test binaries. The visible footprint is a `pub fn` whose only purpose is test wiring (ARCH-H4); the architectural fix is either accepting the per-binary mutex permanently or moving to file-based flock for tests.

**Architectural debt #3 — Bootstrap state mutex (CON-04, P-C1, SEC-06).** The N+1 sequential LLM-call pattern is performance debt (P-C1), but the underlying architecture problem is the absence of a state mutex around `cost_spent_usd` + `--max-cost`. SEC-06 (LLM-output-injection check) layers on top so parallelization is also a security-correctness concern. The fix — `Arc<Mutex<BootstrapState>>` + bounded rayon — unblocks all three findings in one PR. Land before v0.35.

## MSRV recommendation

**Bump to 1.89 in v0.35.0.**

- 1.85 (Feb 2025) is ~6 stable releases behind 2026-05.
- No transitive in tree requires more than 1.85 (verified via `Cargo.lock`).
- 1.88 stabilizes `let_chains` (bootstrap planner has 3 candidate sites); 1.89 stabilizes naked functions and `&raw const/mut`.
- Cost of staying on 1.85: `rusqlite 0.34`+ likely bumps MSRV; better to bump at a release boundary than reactively.
- `rust-toolchain.toml` is `stable` with no floor — add a 1.85.0-pinned CI job until the bump lands (ARCH-H9).

## Appendix: Medium + Low findings

- **ARCH-M01 (Medium):** `coral-stats` is a leaf crate with no integration tests AND no parallelism (CON-10, TEST density). Lift `rayon::par_iter` in `crates/coral-stats/src/lib.rs`.
- **ARCH-M02 (Medium):** `coral-env::EnvBackend` trait is `pub` but only `ComposeBackend` ships. Add `tests/contract_env_backend.rs` against `MockBackend` to fix the trait contract before kind/tilt backends land.
- **ARCH-M03 (Medium):** `coral-test` re-exports 40+ symbols — the largest single-crate surface in the workspace. Audit which are consumed by `coral-cli` vs internal. `BrowserRunner`, `TraceRunner`, `EventRunner` are leaf runners with no CLI surface yet.
- **ARCH-M04 (Medium):** `mimalloc` choice is undocumented as ADR (supply-chain weight per SEC-L05). Lift.
- **ARCH-M05 (Medium):** `coral-mcp` does not re-export `transport::ServeOpts` at lib.rs. Mirror `coral-env`'s shape.
- **ARCH-M06 (Medium):** `coral-cli`'s `[lib]` exposes 1 `pub mod` (`commands`) for integration tests per the doc comment. Document this surface as test-only.
- **ARCH-L01 (Low):** `coral-core` features `pgvector` and `tantivy` exist but are unused elsewhere in the workspace.
- **ARCH-L02 (Low):** `playwright-ci.yml.disabled` (89 lines) bit-rots (TEST-05).
- **ARCH-L03 (Low):** `nightly.yml` and `ci.yml` overlap on `cargo audit` + `cargo deny`. De-duplicate.
- **ARCH-L04 (Low):** `serde_yaml_ng = "0.10"` is a fork. Track upstream `serde_yml` as a contingency.
- **ARCH-L05 (Low):** `[profile.release] panic = "abort"`; tests use unwind defaults. Document the divergence.
- **ARCH-L06 (Low):** `tracing-subscriber` features = `env-filter` only. Lift `json` when v0.35 monitor JSON-emit lands.
- **ARCH-L07 (Low):** `Cargo.toml` mixes inline external deps with internal `coral-*` deps. Split into `# === External ===` / `# === Internal ===` sections.
- **ARCH-L08 (Low):** No `[workspace.lints]` block. Lift alongside v0.35 MSRV bump.
- **ARCH-L09 (Low):** `coral-cli` carries both `webui` (legacy) + `ui` (new) feature flags default-on. Sunset `webui` per BACKLOG.
- **ARCH-L10 (Low):** `release.toml` not read; cargo-release config drift vs `release.yml` is a separate concern.
