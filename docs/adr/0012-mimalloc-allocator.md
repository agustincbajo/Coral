# ADR 0012 — `mimalloc` as the global allocator for `coral`

**Date:** 2026-05-13 (decision); 2026-05-13 (baseline measured)
**Status:** **accepted, baseline measured** — see `docs/bench/MIMALLOC-BASELINE-2026-05-13.md`

## Context

`crates/coral-cli/src/main.rs` registers `mimalloc::MiMalloc` as
`#[global_allocator]`. The workspace dependency declaration
(`Cargo.toml`) carries a comment claiming "10-20% throughput on
allocation-heavy workloads (TF-IDF scoring, wiki parsing, large
OpenAPI property-test generation)."

The claim has been in the tree since v0.24.0 (the property-tests
release). It is unmeasured against the system allocator on the
current workspace — there is no `target/criterion/baseline-vs-
mimalloc/` artifact, no comment with numbers, no PR thread linking
to a flamegraph.

Validator Q flagged this as P-X1: the rationale comment makes a
quantitative claim that the test suite cannot demonstrate, and the
40 KB binary growth from `libmimalloc-sys` is real cost we're paying
against an undocumented win.

The question for v0.35 is: **drop mimalloc and revert to the system
allocator, OR keep it and commit to producing the baseline
benchmark?**

## Decision

**Keep `mimalloc` as the global allocator for v0.35, with an
explicit follow-up to produce a quantitative baseline before v0.35.x
changes the relevant allocation hotspots.**

The follow-up is tracked in `BACKLOG.md` under "v0.35.x — mimalloc
baseline benchmark." Until that lands, the comment in `Cargo.toml`
should be read as a hypothesis, not a measurement.

## Rationale

Three forces argue for keeping mimalloc through v0.35:

1. **Removal is more disruptive than keeping.** Switching back to
   the system allocator on macOS / Linux glibc would change the
   memory residency of every `coral` invocation, the page-fault
   pattern of every subprocess fork (`coral runner`), and the
   thread-local arena behavior of every parallel `rayon` job. The
   net effect is unknowable without the same benchmark we're
   missing. Removing it because we lack the benchmark would
   incur the same uncertainty we're trying to avoid.

2. **The hypothesis is plausible.** mimalloc's documented wins on
   allocation-heavy workloads (small-object churn, multi-thread
   contention) match Coral's hot paths:
   - TF-IDF scoring in `coral-core::search_index` allocates a
     `HashMap<String, f32>` per page and intersects them across
     thousands of pages on every `cargo lint` run.
   - Property-test generation in `coral-test` constructs
     short-lived `serde_json::Value` trees by the hundred-thousand.
   - Wiki parsing in `coral-core::page` does
     `pulldown_cmark`-style event streams that produce many
     small `String` slices.
   - Even if the win is at the bottom of the claimed 10-20% band
     (so ~10%), it's still meaningful on the lint hot path.

3. **The cost is small and contained.** ~40 KB binary growth (audit
   F measured), one extra build-dep (`libmimalloc-sys`), and no
   runtime cost — the allocator is registered at startup and never
   touched again. No transitive Rust dep growth (`libmimalloc-sys`
   is a `build.rs` shelling out to a vendored C source tree).

## Alternatives considered

- **Drop mimalloc, use the system allocator.** Cleaner from a
  supply-chain perspective and removes the unverified claim. But
  the residency / contention / fork-cost regressions are
  unmeasured in either direction, so the "cleanup" is just
  trading one unknown for another.
- **Switch to `jemalloc`** (via `tikv-jemallocator`). Bigger
  binary footprint (~250 KB on Linux), better-documented threading
  wins, but breaks on macOS arm64 in some configs and would force
  a platform-specific allocator config in `main.rs`. Same
  benchmark gap as mimalloc; no reason to swap one unverified
  claim for another.
- **Use `tcmalloc` or `snmalloc`.** Same shape as jemalloc,
  smaller community.

## Consequences

- **mimalloc stays in v0.35 and v0.36+.** The Cargo.toml comment
  was preserved verbatim; the v0.36 baseline measurement (see below)
  vindicates it.
- **Baseline benchmark landed in v0.36-prep (2026-05-13).** Three
  representative workloads, criterion harness at
  `crates/coral-core/benches/allocator.rs`, full results captured in
  `docs/bench/MIMALLOC-BASELINE-2026-05-13.md`. Median results
  (Windows 11 / MSVC):

  | Workload | mimalloc | system | mimalloc speedup |
  |----------|----------|--------|------------------|
  | A — TF-IDF 100 pages, 2-token query | 943 µs | 1.342 ms | +29.7% |
  | B — page parse, 50 docs             | 268 µs | 465 µs   | +42.4% |
  | C — JSON Value 10 routes × 5 props  | 92.5 µs| 162 µs   | +42.7% |

  All three workloads exceed the original 10-20% claim with margin.
  Criterion's own change-detection flagged the system-allocator
  variant as a 50–77% regression vs the mimalloc baseline, p = 0.00.

- **Decision trigger met:** the ≥ 10% threshold for "keep mimalloc and
  freeze the claim" fires on every workload. ADR-0012 stays accepted;
  the v0.35 audit's P-X1 finding is closed.

- **Re-evaluation triggers:** revisit only if (a) a future workload
  shows < 5% win and we have evidence that workload is the new hot
  path, or (b) a supply-chain concern emerges (e.g. mimalloc upstream
  becomes unmaintained — currently still actively maintained by
  Microsoft Research).

## References

- v0.24.0 PR introducing `mimalloc` (commit history; no benchmark
  attached).
- Validator Q audit P-X1 (unmeasured allocator claim).
- `Cargo.toml` workspace deps comment on `mimalloc = { version =
  "0.1", default-features = false }`.
- mimalloc upstream benchmarks (Microsoft Research) — useful
  prior, not Coral-specific.
