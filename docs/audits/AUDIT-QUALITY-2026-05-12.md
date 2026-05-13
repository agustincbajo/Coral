# Audit: Quality (error handling + observability) — Coral v0.34.1
Date: 2026-05-12
Auditor: claude (sonnet)
Scope: 10 workspace crates, production code only (lines outside `#[cfg(test)] mod` blocks). Out of scope: tests, integration fixtures, embedded SPA.

## Executive summary

Coral v0.34.1 production code is **panic-disciplined**: across ~50k lines the auditor counted **70 `.unwrap()`** + **51 `.expect(...)`** + **0 `panic!()`** + **0 `todo!()`** + **1 `unreachable!()`** outside `#[cfg(test)]` blocks. The bulk are safe-by-construction (just-checked `Option`, regex capture groups guaranteed by a preceding match, `OnceLock` regex compilation of literal patterns). The thiserror-vs-anyhow split is correctly modelled: every library crate (`coral-core`, `coral-runner`, `coral-env`, `coral-session`, `coral-test`, `coral-ui::error`) defines a `thiserror::Error` enum; `coral-cli` is uniformly `anyhow::Result` with reasonable `.context()` discipline.

The **observability story is the inverse**: instrumentation is undersized for a tool that drives subprocesses and exposes two HTTP servers. Across the entire workspace there are exactly **2 `tracing::error!`** (both in `coral-ui`), **18 `info!`**, **44 `warn!`**, **15 `debug!`**, **0 `trace!`**, **0 `#[tracing::instrument]`**, **0 `*_span!`**. Bootstrap, MCP, runner subprocess lifecycle, and `self-check` are invisible at `RUST_LOG=debug` — every debug line in a real session comes from `rustls`/`ureq`, never Coral.

Top three wins: (1) instrument the 6 long-running operations with spans + structured fields; (2) replace the 28 mutex-bearing unwrap/expect in `coral-mcp` with poisoning-tolerant accessors so one panicked handler does not kill the server; (3) enable `clippy::unwrap_used` + `clippy::expect_used` as workspace lints to triage the remaining surface. Worst panic-prone module: **`coral-mcp::server` + `coral-mcp::transport::http_sse`** (28 mutex sites that crash on poisoning). Worst observability gap: **`coral-cli::commands::bootstrap`** — the 30-page LLM loop emits zero tracing events; failures surface as `eprintln!("warn: …")` without span context.

## Findings (Critical + High only)

| ID | Severity | Title | File:Line | Category | Proposed fix |
|----|----------|-------|-----------|----------|--------------|
| Q-C1 | Critical | MCP HTTP/stdio: 18 `Mutex::lock().expect(...)` poison-panic on any single panicked handler | `crates/coral-mcp/src/transport/http_sse.rs:98, 449, 462, 523, 546, 558, 612, 847`; `crates/coral-mcp/src/server.rs:245, 536, 547, 579, 593, 611, 620, 632` | panic-prone (concurrency-amplified) | Replace with `.unwrap_or_else(\|p\| p.into_inner())` or `parking_lot::Mutex`. SEC-01 (unauthenticated HTTP) means any remote payload that triggers a panic downstream poisons the lock for every subsequent client. |
| Q-C2 | Critical | `coral-mcp::state::WikiState::scan` silently swallows `read_pages` errors as empty corpus | `crates/coral-mcp/src/state.rs:82`; `crates/coral-mcp/src/resources.rs:231` | silent failure swallow | `coral_core::walk::read_pages(&root).unwrap_or_default()` masks IO/permission/parse errors. Clients see a successful `resources/list` with zero resources. Add `tracing::error!(error = %e, "wiki scan failed")` and propagate. Amplifies P-H8 (stale `OnceLock`): a bad scan caches, restart is the only recovery. |
| Q-C3 | Critical | Bootstrap per-page failure path uses `eprintln!` instead of tracing; error chain lost | `crates/coral-cli/src/commands/bootstrap/mod.rs:316, 334, 609` | observability gap on user-input hot path | The N+1 LLM loop catches `RunnerError`, writes `eprintln!("warn: per-page runner failed for {slug}: {e}")`, `continue`s. No span; `state.pages[i].error` stores `format!("{e}")` losing the chain. On a 30-page bootstrap where pages 5/12/19 fail, the user gets 3 stderr lines and a checkpoint JSON with truncated errors. Wrap in `info_span!("bootstrap.page", slug, idx)` + `warn!(error = %e, error_chain = ?e, "per-page runner failed")`. |

| ID | Severity | Title | File:Line | Category | Proposed fix |
|----|----------|-------|-----------|----------|--------------|
| Q-H1 | High | Zero `#[tracing::instrument]` attributes; zero `*_span!` workspace-wide | `crates/*/src/**` | observability gap | The 6 long-running ops (`bootstrap::run`, `ingest::run`, `query::run`, `ui::serve`, `mcp::serve`, `monitor::up::run`) should carry `#[instrument(skip(state), fields(...))]`. Cost: one attribute per entrypoint. |
| Q-H2 | High | `coral-cli::commands::bootstrap` has zero tracing events | `crates/coral-cli/src/commands/bootstrap/{mod,estimate,state}.rs` | observability on critical path | The single most expensive user-visible op (30 LLM calls, $$) emits only `println!`/`eprintln!`. Add `info!` at entry/exit, `warn!` in catches with structured `slug` + `error`. |
| Q-H3 | High | `RUST_LOG=debug` self-check session emits 21 debug lines, 0 from Coral | live session: `target/release/coral.exe self-check --format=text` | observability gap | All debug lines from `rustls`/`ureq`. `self_check.rs` makes 1 GitHub call, 1 MCP probe, 1 walk, 9 directory probes; none instrumented. Triage of "self-check slow" reports impossible from logs. |
| Q-H4 | High | No `clippy::unwrap_used` / `expect_used` / `panic` lints workspace-wide | `Cargo.toml` (no `[workspace.lints]`); `crates/*/src/lib.rs` (no `#![deny(...)]`) | panic discipline | Add `[workspace.lints.clippy]` block at `warn` level. Surfaces ~120 production sites for triage without blocking CI. Safe-by-construction sites can `#[allow(...)]`. |
| Q-H5 | High | 10 `Mutex::lock().unwrap()` in `coral-mcp::server` mirror the HTTP-transport problem on the stdio path | `crates/coral-mcp/src/server.rs:245, 536, 547, 579, 593, 611, 620, 632, 659, 672` | panic-prone | `notification_tx.lock().unwrap()`, `subscriptions.lock().unwrap()`, `tasks.lock().unwrap()` — every JSON-RPC method has one. Stdio is the default transport. Same fix as Q-C1. |
| Q-H6 | High | Subprocess timeout path re-`expect`s a value already checked; misleading panic message | `crates/coral-runner/src/runner.rs:248` (`timeout.expect("must be Some to hit RecvTimeoutError::Timeout")`) | panic-prone (low likelihood, high blast) | Safe today (only reachable when `remaining` came from `Some(t)`), but a future refactor adding a second `RecvTimeoutError` source silently introduces a panic. Capture `t` into a local before `recv_timeout`. |
| Q-H7 | High | No actionable hint for 3 common end-user failures: wiki missing, runner absent, key absent | `crates/coral-core/src/error.rs` (no hint method); `crates/coral-cli/src/main.rs:367` | error chain unactionable | `RunnerError::NotFound` has an excellent multi-line hint (`runner.rs:12-17`). `CoralError::Io` does not — a `coral query` without `.wiki/` dies with `io error at .wiki: …` and no `coral init` suggestion. Adopt the `hint()` pattern from `coral-ui::error::ApiError::hint()` and consume it in `main.rs`. |
| Q-H8 | High | `scrub_secrets` regex compile via `.expect` is brittle to future refactor | `crates/coral-runner/src/runner.rs:167` | safe today; brittle | Hardcoded regex is safe in v0.34.1 but SEC-M02 asks to extend it. One bad character class = panic in every error envelope. Add a unit test that exercises the lazy init. |
| Q-H9 | High | `info!` density uneven; library crates emit zero info! at lifecycle events | workspace counts: cli=15, core=1, env=0, lint=0, mcp=0, runner=0, session=0, stats=0, test=1, ui=1 | observability gap | `mcp::serve` and `runner::run` are long-running and emit no high-level events. Current pattern is `println!` — bypasses `RUST_LOG` filtering and pollutes stdout for `--format=json` callers. Add `info!` on bind/listen/shutdown/provider invocation. |
| Q-H10 | High | "Error occurred but recovered" silently disappears | `crates/coral-runner/src/runner.rs:259`; `crates/coral-cli/src/commands/self_check.rs:621, 622, 708, 709` (`let _ = child.kill()`, `let _ = child.wait()`) | silent failure swallow | These are deliberate process-cleanup-on-failure paths but never logged. Wrap with `if let Err(e) = ... { debug!(error = %e, "child cleanup failed (best-effort)") }`. Triages "self-check hangs sometimes on Windows" reports. |

## Quality dashboard

Production-only counts (lines OUTSIDE `#[cfg(test)] mod tests` blocks). Methodology: brace-tracking Python scanner; numbers may vary ±2 due to multi-line macro handling.

| Crate | `.unwrap()` | `.expect(` | `panic!()` | `unreachable!()` | `info!` | `warn!` | `error!` | `debug!` | `trace!` | `#[instrument]` | `*_span!` |
|---|---|---|---|---|---|---|---|---|---|---|---|
| coral-cli | 3 | 9 | 0 | 1 | 15 | 22 | 0 | 4 | 0 | 0 | 0 |
| coral-core | 13 | 20 | 0 | 0 | 1 | 8 | 0 | 1 | 0 | 0 | 0 |
| coral-env | 8 | 1 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| coral-lint | 0 | 1 | 0 | 0 | 0 | 1 | 0 | 0 | 0 | 0 | 0 |
| coral-mcp | 18 | 10 | 0 | 0 | 0 | 2 | 0 | 1 | 0 | 0 | 0 |
| coral-runner | 23 | 2 | 0 | 0 | 0 | 1 | 0 | 7 | 0 | 0 | 0 |
| coral-session | 0 | 3 | 0 | 0 | 0 | 3 | 0 | 1 | 0 | 0 | 0 |
| coral-stats | 0 | 1 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| coral-test | 5 | 0 | 0 | 0 | 1 | 3 | 0 | 0 | 0 | 0 | 0 |
| coral-ui | 0 | 4 | 0 | 0 | 1 | 4 | **2** | 1 | 0 | 0 | 0 |
| **Total** | **70** | **51** | **0** | **1** | **18** | **44** | **2** | **15** | **0** | **0** | **0** |

Production `panic!()` count is **0** — every `panic!()` lives in test modules. `todo!()` / `unimplemented!()` workspace total: **0**.

Silent-failure patterns (production):
- `let _ = ...`: 115 sites; spot-checked ~30, all deliberate (child cleanup, best-effort writes, drop-impl unlocks). Q-H10 covers the lack of logging.
- `unwrap_or_default()`: 318 sites. The 2 worrying ones (Q-C2) turn wiki-walk failures into empty corpora. The 316 others are TOML/JSON defaults for optional config fields — acceptable.
- `if let Err(_) = ...`: 0 production sites.

## Methodology

Static review only. Tools:

- `Grep` over `crates/*/src/**` for 6 panic-prone constructs and 7 tracing macros.
- Custom Python scanner with brace-depth tracking + `#[cfg(test)] mod {` detection to separate production from test counts. Naïve `wc -l` finds 856 `.unwrap()` in `coral-cli`; scanner correctly reports 3 once test mods exclude.
- Cross-reference against SEC/CON/PERF/TEST audits dated 2026-05-12 for amplification.
- Live session: `target/release/coral.exe self-check --format=text` with `RUST_LOG=debug` — 21 debug lines, 0 from `coral::`.
- Manual triage of every production unwrap/expect: categorised as (a) safe-by-construction, (b) mutex-poisoning, (c) regex-capture-group, (d) lazy-init OnceLock, (e) just-checked Option. The 28 (b) sites in `coral-mcp` drive Q-C1/Q-H5.

Not done: `cargo clippy -- -W clippy::unwrap_used` ground-truth count (Q-H4 wires it); no proptest panic injection of malformed JSON-RPC; no tracing-subscriber overhead measurement.

## Scope NOT audited

- **Test code** (`#[cfg(test)] mod tests` + `tests/*.rs`). Scanner counts 1,673 test-mod unwraps + 481 expects + 44 panics — expected; tests panic on failure by design.
- **`coral-cli::main.rs::setup_tracing`** — covered in P-M1 (perf audit).
- **Embedded SPA** (Vite bundle in `coral-ui/assets/dist/`).
- **`/// `rustdoc coverage on pub items** — informational, deferred. Spot-checked: 10/10 lib.rs files open with `//!` module-doc.
- **`coral-test::mock.rs`** — only linked into integration tests, not real user-input.
- **Deprecation notices on other features** — grep found no others in v0.34.1. The `coral wiki serve` notice (`commands/serve.rs:50-55`) is well-formed: explicit version, removal target, migration path.

## Top-3 next actions

1. **Wire `[workspace.lints.clippy]`** (Q-H4). One TOML block; lints `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented` at `warn`. Surfaces ~120 production sites; ETA 2h for the wire-up + 1d for triage.

2. **Fix mutex-`.expect()` poisoning in `coral-mcp`** (Q-C1 + Q-H5). 28 sites across `server.rs` and `transport/http_sse.rs`. Mechanical change to `.unwrap_or_else(|p| p.into_inner())`. Combined with SEC-01, closes a "single bad request kills the server" amplification. ETA 2h including a panic-injection test.

3. **Instrument the 6 long-running operations** (Q-C3 + Q-H1/H2/H3/H9). `#[instrument]` on `bootstrap::run`, `ingest::run`, `query::run`, `ui::run`, `mcp::run`, `monitor::up::run`. Nested `info_span!` inside the bootstrap per-page loop. Replace `eprintln!` with `info!`/`warn!`/`error!` where the message is diagnostic. Enables `RUST_LOG=coral=debug` triage. ETA 1d.

## Appendix: Medium + Low findings

**Q-M1.** `coral-cli::commands::onboard.rs:153`, `scaffold.rs:106`: `Confidence::try_new(0.7).unwrap()` on literals — safe; flag for a `const_new` constructor.

**Q-M2.** `coral-cli::commands::search.rs:436`: `serde_json::to_string_pretty(&payload).unwrap()` — payload internally constructed, cannot fail. Promote to `expect` with explanatory string.

**Q-M3.** `coral-core::config.rs:473`: `entry.as_table_mut().expect("just checked is_table")` — safe; consider `let Value::Table(t) = entry else { unreachable!() }`.

**Q-M4.** `coral-mcp::server.rs:289, 659, 672`: `serde_json::to_value(JsonRpcResponse {...}).unwrap()` — `JsonRpcResponse` is derived over `String` + `Option<Value>`, cannot fail. Promote to `expect`.

**Q-M5.** `coral-core::symbols.rs` has 8 `caps.get(N).unwrap()` (Rust/TS/Python/Go extractors). All safe-by-construction (after `if let Some(caps) = re.captures(line)`). Add `#[allow(clippy::unwrap_used)]` once Q-H4 lands.

**Q-M6.** `coral-runner::prompt.rs:35`: `Regex::new(...).unwrap()` for template pattern. Literal regex, lazy via `OnceLock`. Safe.

**Q-M7.** `coral-core::log.rs:74-76, 112`: 4 `.expect()` on regex capture groups and `entries.last()` after `push`. All safe; document via comment.

**Q-M8.** `coral-cli::commands::wiki.rs:124`: `.expect("stdin piped")` — spawn configured with `stdin(Stdio::piped())` immediately above. Safe; add `// SAFETY:` comment.

**Q-M9.** `coral-cli::commands::self_upgrade.rs:303`: `unreachable!("nibble was {n}")` — `n` is `byte & 0x0F`, mathematically bounded. Safe.

**Q-M10.** `anyhow` context discipline is good in `coral-cli`. Spot-check exception: `bootstrap/mod.rs:302, 319, 337` call `state.save_atomic(root)?` without per-page context. If the checkpoint write fails mid-loop the user sees a bare `io error` without knowing which page. Add `.with_context(|| format!("checkpoint after page {}", entry.slug))`.

**Q-M11.** `coral-ui::server.rs` uses `anyhow::Result` at the public boundary; `error.rs::ApiError` uses thiserror for the wire protocol. Convention is correct.

**Q-M12.** `coral-session::scrub.rs:598, 616`: `serde_json::from_str(&out).expect("scrubbed JSON must remain parseable")` — strong assertion that prevents leaking corrupted output. Keep.

**Q-L1.** `coral-env::mock.rs` 8 `lock().unwrap()`: test-only-linked. Safe.

**Q-L2.** `coral-runner::mock.rs` 23 `lock().unwrap()`: test-only fixture. Flagged so future production reuse does not inherit poison-panic.

**Q-L3.** `coral-test::perf.rs:93`: `serde_json::to_string_pretty(baseline).unwrap()`. Internal type; fine.

**Q-L4.** Workspace has **zero `trace!` macros**. Hot loops (BM25, structural lint, walk-cache hash) could benefit. Low priority.

**Q-L5.** Module-doc (`//!`) coverage 10/10. Item-doc (`///`) uneven but not Critical. Could enforce `#![warn(missing_docs)]` on pub library APIs once stable.

**Q-L6.** `coral-cli::commands::bootstrap/mod.rs:541`: `std::env::current_dir().expect("cwd")` — fails on Linux when cwd is removed under the process (rare; CI containers). Convert to `?` with `Context`.

**Q-L7.** `coral-cli::commands::sync.rs:60`: `.expect("checked above; --remote requires --version")` — clap-level invariant; document via cross-reference comment.

**Q-L8.** **Cross-Phase amplification.** SEC-01 (no MCP HTTP auth) × Q-C1 (mutex poisoning) = pre-auth DoS. Any remote caller able to reach the HTTP transport can craft a payload that panics in one of the 11 lock-bearing handlers; subsequent legitimate requests poison and panic. Fixing Q-C1 closes this even without SEC-01. This is the most concerning cross-Phase finding.

**Q-L9.** **Cross-Phase amplification.** P-C1 (sequential bootstrap calls) × Q-C3 (no per-page tracing) = a 30-page Sonnet bootstrap that fails on page 19 leaves the user with three stderr lines and no recovery context. The parallelisation work in P-C1 should ship together with the tracing work in Q-C3.

**Q-L10.** **Cross-Phase amplification.** CON-`OnceLock` invalidation (P-H8) × Q-C2 (silent `unwrap_or_default` on `read_pages`) = once a bad scan caches into the `OnceLock`, the server serves empty results indefinitely. Fixing Q-C2 (propagate) helps the user notice; fixing P-H8 (`RwLock<Option<Arc<Vec<Page>>>>`) closes the persistence side.
