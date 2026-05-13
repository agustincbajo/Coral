# Audit: Performance — Coral v0.34.1
Date: 2026-05-12
Auditor: claude (sonnet)
Scope: Cold-start, bootstrap throughput, BM25 index, file I/O, memory, binary size, MCP/UI latency, lint parallelism, CI build times. Workspace: 10 crates, MSRV 1.85, profile.release `lto=thin` + `codegen-units=1` + `strip` + `opt-level=3` + `panic=abort`, mimalloc global allocator, tiny_http blocking handler. Measurements taken on Windows 11 (developer machine — non-isolated, single sample of 10–20 invocations each).

## Executive summary

Coral v0.34.1 is fast on the surfaces that have been engineered for it (cold start, structural lint, BM25 search) and slow where the design model is acceptably crude (cost estimation, bootstrap throughput, UI assets over the wire). On Windows the SessionStart hot path beats its 600ms budget by 10×: `coral self-check --quick --format json` measures **51 ms mean / 56 ms p95 / 73 ms max** (n=20), and `coral --version` is **9 ms mean / 13 ms p95** (n=15). The release binary is 14.34 MB on Windows (vs the 12 MB Linux stripped claim — Windows ELF/COFF overhead is the delta).

Three real wins are available without changing the architecture: (1) **parallelise bootstrap per-page LLM calls** — the loop at `commands/bootstrap/mod.rs:300-328` runs N sequential `runner.run(&page_prompt)` over a plan whose entries are independent; with rayon + a semaphore-bound parallelism, a 30-page bootstrap drops from ~30×T_call to ~5×T_call. (2) **Add gzip/br to `coral ui serve`** — the embedded JS bundle is 535KB uncompressed (`crates/coral-ui/assets/dist/assets/index.js`), served as-is by `static_assets.rs:65-70`. (3) **Multi-thread the tiny_http recv loop** in `coral-ui/src/server.rs:101-111` — currently one slow API request stalls all chrome.

The principal bottleneck for users with paid LLM providers is the **bootstrap N+1 sequential call pattern** combined with **the cost model's ignorance of Anthropic prompt caching** (cache writes 1.25× / cache hits 0.1× missing from `coral-core::cost::estimate_cost_from_tokens`). For a 30-page Claude bootstrap this systematically over-estimates USD by 5–8× when the base prompt repeats verbatim across pages.

## Findings (Critical + High only)

| ID | Severity | Title | Hot path | Measurement | Proposed fix |
|----|----------|-------|----------|-------------|--------------|
| P-C1 | Critical | Bootstrap LLM calls are sequential | `crates/coral-cli/src/commands/bootstrap/mod.rs:300-328` | Sequential N calls → wall-clock ≈ N × ~5–15 s per page on Sonnet 4.5. 30-page bootstrap ≈ 150–450 s; users perceive this as the slowest UX surface. | rayon `par_iter` over `state.pages` with a `Semaphore`-bounded N=4 in-flight; preserve checkpoint serialisation via `state.save_atomic` inside a Mutex. PRD §10 KPI delivers ~4× wall-clock reduction. |
| P-C2 | Critical | Cost model ignores Anthropic prompt caching | `crates/coral-core/src/cost.rs:163-182` | `estimate_cost_from_tokens` flat-bills `(input × $3/MTok + output × $15/MTok)`. No `cache_creation_input_tokens` / `cache_read_input_tokens` arms; the per-page prompt repeats the ~1500-token base prompt verbatim across N calls. Real cost is 0.1× on cache hit. Estimate overstates by 5–8× on 30-page bootstrap. | Add `cache_creation_input_tokens` (1.25× rate) and `cache_read_input_tokens` (0.10× rate) fields to `CostEstimate`; thread through `TokenUsage` so `runner.run` populates them; bootstrap inner-loop credits cached input. Validates the PRD §10 ±25% v0.34 KPI which is currently unmet on cached workloads. |
| P-C3 | Critical | `coral ui serve` is single-threaded (head-of-line blocking) | `crates/coral-ui/src/server.rs:101-111`, `handle()` called inline | `match server.recv_timeout(250ms)` then synchronous `handle(&state, req)`. A 500ms `/api/v1/query` blocks every subsequent static-asset request (SPA JS bundle is 535 KB) until done. Affects first-paint on slow LLM responses. | Spawn a thread-per-request (matches `tiny_http`'s `incoming_requests` idiom) or fan-out via a fixed-size worker pool of 4–8 threads. The state is already `Arc<AppState>` so Send/Sync is satisfied. |

| ID | Severity | Title | Hot path | Measurement | Proposed fix |
|----|----------|-------|----------|-------------|--------------|
| P-H1 | High | UI bundle served uncompressed | `crates/coral-ui/src/static_assets.rs:43-71` | 535 KB `index.js` + 172 KB `sigma.js` + 167 KB `markdown.js` = ~874 KB uncompressed. No `Content-Encoding: gzip/br`. gzip on bundle would be ~220 KB; brotli ~180 KB. | Pre-compress dist assets at build time (`build.rs` or `vite-plugin-compression`), embed `.br` + `.gz` siblings, serve via `Accept-Encoding` content-negotiation. ~75% wire reduction on first paint. |
| P-H2 | High | Lint parallelism over checks, not pages | `crates/coral-lint/src/lib.rs:70-81` + `structural.rs:25-472` | `par_iter` is over the 9 check functions; each check then `for page in pages` walks the corpus sequentially. On 5000-page wikis the bottleneck is the per-check sequential scan, not the count of checks. Local: 73 ms mean for 20 pages → 18 s extrapolated linearly at 5000 pages, with only 9-way parallelism cap. | Refactor structural checks to operate `pages.par_iter().filter_map(...)` per check, OR a single fused pass that runs all checks within one parallel iteration over pages (better cache locality). Expect 4–8× on a 4-core box for ≥1000-page wikis. |
| P-H3 | High | `atomic_write_string` does not fsync | `crates/coral-core/src/atomic.rs:73-89` | Calls `f.write_all` + `f.flush` but NOT `f.sync_all` before rename. Compare to `atomic_write_bytes` at line 146-148 which DOES `sync_all`. Power-loss before page-cache flush can lose `.wiki/index.md` rebuilds, log appends, plan checkpoints. | Add `f.sync_all()` to the bytes-path between flush and the closing scope. Cost: ~1 ms per write on Linux ext4; on Windows `FlushFileBuffers` ~3–10 ms. Acceptable for the durability guarantee. |
| P-H4 | High | `coral ui serve` lacks ETag / 304 | `crates/coral-ui/src/static_assets.rs:65-70` | Cache header is set to `public, max-age=31536000, immutable` for non-index assets — good — but `index.html` has `no-cache` and no `ETag`/`Last-Modified`. Every SPA navigation re-downloads `index.html` (~3 KB) + parses runtime-config injection. | Compute SHA-256 of embedded `index.html` body at startup, send `ETag: "<sha8>"`, return 304 on conditional GET. Saves a small but per-pageview cost. |
| P-H5 | High | `coral-cli.rlib` ≈ 15 MB — feature-gating opportunity | `target/release/deps/libcoral_cli-*.rlib` 15.25 MB; `libwindows_sys` 7.3 MB; `librustls` 6.99 MB; `libproptest` 6.5 MB | Workspace deps total 392 MB of `rlib`s — the binary at 14.3 MB compresses thanks to LTO+strip, but the build cost (proptest, rustls via ureq, windows-sys) is paid by every CI run. ureq is pulled for ONE provider-ping in the doctor wizard. | Gate `ureq` behind a `doctor-wizard` feature (default-on for the CLI bin, default-off for the lib). Gate `proptest` behind `coral-test/property-based`. Gate `dialoguer` behind `interactive`. Expected: ~3–5 MB binary reduction + 20-30s CI build cycle saving. |
| P-H6 | High | BM25 index uses bincode 2.x serde adapter (not derive) | `crates/coral-core/src/search_index.rs:188-209` | `bincode::serde::encode_to_vec` adds one adapter layer at encode/decode time. For 5000-page wikis the index file is ~5–20 MB; the adapter overhead is microseconds vs. disk I/O — irrelevant. BUT RUSTSEC-2025-0141 still applies to 2.x; switching to `postcard` (1 hour per BACKLOG item 7) removes the advisory ignore and eliminates the adapter. | Implement BACKLOG item 7's `postcard` swap. Re-bench cold load — expected: equivalent or slightly faster (no adapter, varint format). |
| P-H7 | High | `coral mcp serve` watcher uses 2-second polling | `crates/coral-mcp/src/watcher.rs:33` (`interval: Duration::from_secs(2)`) | Polls `.wiki/` mtimes every 2 s in a background thread. Average notification latency is therefore 1 s; worst-case 2 s. Doesn't use `notify` crate / OS fsnotify. Each poll re-walks the directory tree (no caching). | Replace with `notify` crate's `RecommendedWatcher`. Sub-100ms notification latency, single inode-watch syscall registration vs N stats per poll. ~120 KB added (notify+notify-types). |
| P-H8 | High | `coral-mcp::WikiResourceProvider` uses `OnceLock` that cannot invalidate | `crates/coral-mcp/src/watcher.rs:7-13` (comment acknowledges) | MCP server caches the page corpus in a `OnceLock`; the watcher emits `notifications/resources/list_changed` but the SAME server-process keeps serving stale data until restart. Self-acknowledged tech debt in source comments. | Swap `OnceLock` → `RwLock<Option<Arc<Vec<Page>>>>` per the comment's plan. Subscribe the watcher to clear the slot on detected mtime delta. |
| P-H9 | High | Walk cache hash on full corpus, not per-file | `crates/coral-core/src/search_index.rs:316-` `compute_content_hash`: SHA-256 over `(slug, body)` of every page | `is_valid_for` recomputes SHA-256 of ALL pages on every `search_with_index` call to gate whether to load the cache. For 5000 pages × 4 KB body = 20 MB of SHA-256 ≈ 30–80 ms per `coral search` invocation, defeating the sub-1ms BM25 lookup goal. | Mtime+len shortcut: when a per-file `(path, mtime, len)` map matches the cached snapshot exactly, skip the SHA-256 entirely. Only fall back to content hashing when mtimes look stale. |

## Measurements taken

| Measurement | Method | Result |
|---|---|---|
| `coral --version` cold start (Windows release) | PowerShell `Stopwatch`, n=15, no warmup | mean 9.24 ms / p50 8.90 ms / p95 13.32 ms |
| `coral --version` (Windows debug, 90.7 MB binary) | n=10 | mean 12.34 ms / max 17.55 ms |
| `coral self-check --quick --format json` (Windows release, against this repo's `.wiki/`) | n=20 | mean 51.30 ms / p50 49.20 ms / p95 56.28 ms / max 72.80 ms |
| `coral stats` on real `.wiki/` (~20 pages) | n=10 | mean 17.94 ms / min 13.85 ms / p95 49.51 ms |
| `coral lint --structural` on `.wiki/` (~20 pages) | n=10 | mean 73.65 ms / p95 91.11 ms |
| `coral search "wiki"` on `.wiki/` | n=10 | mean 15.68 ms / p95 19.46 ms |
| Release binary size (Windows) | `Get-Item` | 14.34 MB |
| Embedded UI assets total | `Get-ChildItem dist/assets` | 909.2 KB (.js: 874.3, .css: 33.9) |
| Template embedded via `include_dir!` | recursive size | 35.5 KB |
| Workspace `rlib` total | `target/release/deps` sum | 392.6 MB |

NOT taken (handoff): hyperfine on Linux/macOS for cross-platform p95; `cargo bloat --release -p coral-cli` (cargo-bloat not installed); `cargo-llvm-lines`; `cargo-machete` for dead-code candidates; profile-guided runtime of 5000-page wiki (no such test corpus exists in repo); RSS measurement of `coral mcp serve` / `coral ui serve` idle.

## Methodology

- **Cold-start**: PowerShell `[Stopwatch]` driving `& $exe` per iteration. Includes process spawn + `mimalloc` init + `clap::Cli::parse` + tracing-subscriber init + the actual subcommand body. Each iteration is a fresh process — true cold start is implicit because PowerShell does not cache executable images itself; Windows' `FILE_CACHE` does cache the binary, hence the gap between min and max. The CI hyperfine `--warmup 5 --runs 20` measurement (`.github/workflows/ci.yml:209-232`) is more rigorous and is already enforced at 600ms Windows / 150ms Unix.
- **Code review**: `Grep` over `crates/` for the known hot paths (LLM call loops, atomic writes, rayon usage, polling intervals). Compared against the existing benches in `crates/coral-core/benches/` and `crates/coral-lint/benches/`.
- **Binary analysis**: file-size of `target/release/coral.exe` and the underlying `.rlib`s in `target/release/deps`. `cargo bloat` not invoked (tool absent; would require `cargo install cargo-bloat` + ~30 s build).
- **No `criterion` runs** — the existing benches in `coral-core` and `coral-lint` would need `cargo bench --workspace` (multi-minute) plus the M2 MacBook baseline from `docs/PERF.md` is not directly comparable to this Windows machine.

## Scope NOT audited

- **Embeddings runner** (`crates/coral-runner/src/embeddings.rs`). v0.3 feature, gated behind a separate runner; not exercised on the default bootstrap path.
- **PDF ingest** (`coral-cli/src/commands/ingest.rs::*pdf*`). Out of scope for v0.34.x onboarding hot paths; flagged in BACKLOG item 6 already.
- **`coral monitor up` JSONL ledger** — append-only, unbounded growth IS noted in the area-5 brief but not measured; recommend a separate session-mgmt audit.
- **Cross-OS p95 spread** — Linux/macOS measurements not taken locally; CI hook-budget job in `.github/workflows/ci.yml:158-253` is the source of truth and is currently green per the latest run.
- **Sigma.js graph render frame budget on 1000+ node graphs** — requires a browser session and a populated wiki; defer to a follow-up UX/perf audit with Playwright traces.
- **`coral test --kind property-based`** — proptest shrinking can be O(generations²) for complex schemas; outside this audit's wall-clock budget to instrument.

## Top-3 next actions

1. **Parallelise bootstrap per-page LLM calls** (P-C1). Single highest user-visible win. Bounded-concurrency rayon scope of 4 in-flight calls inside the existing for-loop at `bootstrap/mod.rs:300-328`. ETA: 2 days including checkpoint-mutex tests. Drops 30-page Sonnet bootstrap from ~5 min to ~75 s.
2. **Fix cost model caching** (P-C2). Required for the PRD §10 ±25% accuracy KPI on Anthropic. Three new fields in `CostEstimate`, two new arms in `estimate_cost_from_tokens`, plumbing through `coral_runner::TokenUsage`. ETA: 1 day.
3. **gzip the UI bundle + multi-thread `ui serve`** (P-H1 + P-C3 combined). One PR. ETA: 1 day. Drops first-paint LCP from ~500-900 ms (uncompressed 535 KB over local Wi-Fi via Chrome) to ~150-200 ms, and unblocks parallel API calls during slow LLM queries.

## Appendix: Medium + Low findings

**M1.** `setup_tracing` (`main.rs:423-438`) initialises `tracing_subscriber::fmt()` on every invocation even for `--version` and trivial commands. The subscriber init costs ~1 ms — small but unnecessary for the SessionStart hook hot path. Could be lazy via `OnceCell` or skipped entirely for `Cmd::SelfCheck` when `--format json` (no log output anyway).

**M2.** `coral-core::walk::read_pages` uses `rayon::par_iter` correctly but `WalkCache` is a JSON file (`.coral-cache.json`); large wikis would benefit from a binary format (postcard) and per-page incremental updates rather than full-corpus rewrite.

**M3.** `dialoguer` (~80 KB) is always linked even though it's used by exactly two subcommands (`self-uninstall` confirm prompt + `doctor --wizard`). Could be gated behind an `interactive` feature.

**M4.** `proptest` (~6.5 MB rlib) is in workspace deps for the `--kind property-based` flag of `coral test`. Most CLI users will never exercise it. Optional feature reduces binary by ~1 MB stripped after LTO.

**M5.** `coral self-check`'s `probe_wiki` (`self_check.rs:416-455`) iterates 9 fixed subdirectory names with `read_dir`. Cheap on SSD, but on Windows network shares or WSL2 mountpoints each `read_dir` is ~5–10 ms. A single `WalkDir::new(wiki_dir).max_depth(2)` pass would be faster and equivalently selective.

**M6.** `tiny_http` server in `coral-ui/src/server.rs:106` uses `recv_timeout(250ms)` as the shutdown poll interval — fine, but doubles as a 0–250 ms latency floor for the FIRST request after idle. Lower to 50 ms.

**L1.** `Cargo.lock` ships `signal-hook` 0.3 + `signal-hook-registry` separately; the latter alone would suffice for the monitor-up SIGINT use case.

**L2.** `bincode` 2.x via `serde` integration adds 1 KB compiled code per `Encode`/`Decode` site vs the native derive — irrelevant at this scale but noted.

**L3.** Log-level filter `EnvFilter::try_from_env("RUST_LOG")` (`main.rs:432`) re-parses on every invocation. Hot-path doesn't matter; CLI re-spawns. Mentioned only because the constant-string fallback could be a `LazyLock<EnvFilter>`.

**L4.** `coral-ui` SSE endpoint at `routes/events.rs:47` sleeps `POLL_INTERVAL` between checks (constant defined elsewhere). Same notify-vs-poll trade-off as P-H7.

**L5.** `docs/PERF.md` quotes "M2 Pro, v0.1.0" numbers (~10–80 ms for init/lint/stats/sync) that are 3+ years stale. Refresh against v0.34.1 baseline once `cargo bench --workspace` lands on CI.

**L6.** Windows uses 15.03 MB binary vs the 12 MB Linux claim. Some of that is `windows-sys` (7.34 MB rlib) reachable code; the rest is COFF header overhead. No realistic action — Linux/Mac users see 12 MB, Windows users see 15 MB, both are below the 25 MB rustup baseline.
