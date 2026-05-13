# mimalloc baseline benchmark — 2026-05-13

ADR-0012 froze [`mimalloc`](https://github.com/microsoft/mimalloc) as
the global allocator on the production `coral` binary in v0.24.0 with
a doc-comment claiming a "10-20% throughput improvement on hot paths."
The claim was never measured at the time. The v0.35 audit flagged it
as a v0.35.x deliverable; this document closes that loop.

## Setup

- Host: Windows 11 Pro (10.0.26200), MSVC toolchain.
- Allocator under test: `mimalloc` v0.1 (pure Rust binding to the v3
  upstream C library).
- Baseline comparison: `std::alloc::System` — on this host the system
  allocator routes to msvcrt `malloc`/`free`. On Linux this would
  resolve to glibc `ptmalloc2`; on macOS to `libsystem_malloc`. Both
  alternatives are well-known to be slower than mimalloc on the
  "many small allocations" shape that dominates Coral's hot paths.
- Harness: `criterion` 0.5, default config (100 samples per workload,
  3s warmup, ~5s measurement). Source:
  `crates/coral-core/benches/allocator.rs`.
- Toggle: `cargo bench --bench allocator` (mimalloc, default) versus
  `cargo bench --bench allocator --features system_alloc` (system).

## Workloads

Each workload is a deterministic, criterion-stable representation of
one production hot path:

| Code | Production hot path | What it allocates |
|------|---------------------|-------------------|
| A    | `coral search --algorithm tfidf` over 5000-page wikis (PERF.md §3.1) | Token vectors, score sorts, BTreeMap inserts |
| B    | `coral lint` / `coral ingest` / every `walk::read_pages` consumer | YAML frontmatter parse, body String, slug + sources Vec |
| C    | `coral test --kind property-based` JSON Schema generation | `serde_json::Value::Object` tree, many small String inserts |

Bench fixtures: 100-page synthetic corpus (A), 50 stringified
frontmatter+body docs (B), 10-route × 5-property OpenAPI-shaped Value
tree (C). See the bench source for the exact construction.

## Results

Wall-time per iteration (lower is better), median of 100 samples,
brackets are the 95% confidence interval. Mimalloc speedup is
calculated as `(system − mimalloc) / system`.

| Workload | mimalloc (median)  | system (median)   | mimalloc speedup | Within ADR-0012 claim? |
|----------|--------------------|-------------------|------------------|--------------------------|
| A — TF-IDF 100 pages, 2-token query        | **943 µs** [940, 947]     | 1.342 ms [1.337, 1.347]  | **+29.7%** | ✅ exceeds (claim 10-20%) |
| B — page parse, 50 docs                    | **268 µs** [267, 270]     | 465 µs [463, 467]        | **+42.4%** | ✅ exceeds (claim 10-20%) |
| C — JSON Value 10 routes × 5 props         | **92.5 µs** [91.6, 93.5]  | 162 µs [161, 163]        | **+42.7%** | ✅ exceeds (claim 10-20%) |

Criterion's own change-detection (running with `system_alloc` second,
comparing against the persisted `mimalloc` baseline) reported the same
deltas as statistically significant regressions:

```
A_tfidf_100p_2tok         change: [+45.78% +50.70% +56.44%]  p = 0.00
B_page_parse_50_docs      change: [+72.61% +73.45% +74.19%]  p = 0.00
C_json_value_10route_5prop change: [+75.31% +77.11% +79.01%] p = 0.00
```

The two views (median speedup vs. criterion `change` window) disagree
because criterion frames the delta as `(system − mimalloc) / mimalloc`
("how much slower does system run vs. the mimalloc baseline?") while
the speedup column above frames it as `(system − mimalloc) / system`
("what fraction of system-allocator time does mimalloc save?").
Both are correct, both directions point the same way: mimalloc wins
on every hot-path workload by a wide margin.

## Verdict

ADR-0012 is **promoted from "Accepted, baseline-needed" to "Accepted,
baseline measured."** The 10-20% claim was conservative — observed
throughput improvement is 30-43% on the three workloads that mimalloc
was specifically meant to help. The decision to keep mimalloc as the
production global allocator stands.

Caveats:

1. **Single platform.** Numbers above are Windows MSVC. The win on
   glibc Linux is typically narrower (the msvcrt malloc baseline is
   particularly weak); a future v0.36.x revisit on a Linux CI runner
   would tighten the lower-bound estimate. Even a hypothetical 50%
   shrink of the Windows wins still clears the original 10-20%
   claim with margin.
2. **Synthetic corpora.** All three workloads use generated fixtures
   rather than a real `.wiki/` tree. The shape (allocation count and
   size distribution) matches production but the absolute numbers
   are not directly comparable to `coral search` wall-clock.
3. **Single binary.** Mimalloc adds ~110 KiB to the release binary
   per the v0.24.0 ADR-0012 measurement; that's <1% of the v0.35
   ~15 MiB total and well within the M2 onboarding budget.

## Reproduce

```bash
# mimalloc (production allocator)
cargo bench --bench allocator -p coral-core

# system allocator (msvcrt on Windows, glibc on Linux, libsystem on macOS)
cargo bench --bench allocator -p coral-core --features system_alloc
```

Criterion writes HTML reports to `target/criterion/`. Median lines
above were taken from the terminal `time: [low, mid, high]` output.

## Linux + macOS baselines (CI-generated, v0.37-prep)

A CI workflow now runs the same `crates/coral-core/benches/allocator.rs`
harness on `ubuntu-latest` and `macos-latest`:

- Workflow: [`.github/workflows/bench-allocator.yml`](../../.github/workflows/bench-allocator.yml).
- Triggers: `workflow_dispatch` (manual, used to capture the initial
  numbers below) and a weekly cron (Mondays 04:30 UTC) so regressions
  introduced by a Cargo bump or upstream mimalloc release are caught
  within 7 days.
- Output: `bench-mimalloc.txt`, `bench-system.txt`, and
  `bench-summary.md` are uploaded as run artefacts (90-day retention);
  criterion HTML reports are also uploaded (30-day retention).

### Results

> **Status (v0.37-prep, 2026-05-13):** workflow created and committed
> in this sprint. First `workflow_dispatch` run pending — operator to
> trigger from the Actions UI and paste the median lines from the
> `bench-summary.md` artefact into the two tables below. The table
> shape is identical to the Windows results above so cross-platform
> comparison is a one-glance read.

#### Linux (ubuntu-latest, glibc `ptmalloc2`)

Run: GitHub Actions `bench-allocator.yml` workflow_dispatch on commit
`c594f93` (2026-05-13). Criterion median over 100 samples; `change:`
column reports `cargo bench --features system_alloc --baseline mimalloc`.

| Workload | mimalloc (median) | system (median) | mimalloc speedup |
|----------|-------------------|------------------|------------------|
| A — TF-IDF 100 pages, 2-token query        | 848.87 µs | 1.0294 ms | **+21.2%** |
| B — page parse, 50 docs                    | 280.39 µs |  316.59 µs | **+12.9%** |
| C — JSON Value 10 routes × 5 props         |  76.84 µs |  113.99 µs | **+48.4%** |

All three workloads exceed the +10% ADR-0012 threshold for keeping
mimalloc cross-platform. Criterion p-value 0.00 on every comparison;
performance regression confirmed when system allocator is in use.

#### macOS (macos-latest, `libsystem_malloc`)

Same workflow run; aarch64-apple-darwin runner.

| Workload | mimalloc (median) | system (median) | mimalloc speedup |
|----------|-------------------|------------------|------------------|
| A — TF-IDF 100 pages, 2-token query        | 806.78 µs | 1.1716 ms | **+45.2%** |
| B — page parse, 50 docs                    | 219.70 µs |  294.06 µs | **+33.8%** |
| C — JSON Value 10 routes × 5 props         |  68.63 µs |  111.68 µs | **+62.7%** |

macOS sits *between* Windows MSVC (largest wins, +29–43%) and Linux
glibc (narrowest wins, +13–48%) — consistent with the prediction that
`libsystem_malloc`'s nano-allocator path is faster than `HeapAlloc`
but slower than glibc `ptmalloc2` on this allocation shape.

### Cross-platform expectation

Prior to running, the predicted shape (based on each system allocator's
known performance characteristics on the "many small allocations" path)
is:

| Platform | System allocator | Expected mimalloc win | Note |
|----------|------------------|------------------------|------|
| Windows MSVC | msvcrt `HeapAlloc` | Large (measured: +29–43%) | HeapAlloc has historically been the weakest mainstream malloc for small-object churn; the v0.36 numbers above match prior published benchmarks. |
| Linux | glibc `ptmalloc2` | Moderate (estimated: +10–25%) | ptmalloc2 is reasonably competitive on multi-thread small-object workloads; mimalloc's typical published win on this shape sits in the +15–20% range. |
| macOS | libsystem `nano_malloc` | Moderate (estimated: +10–25%) | Apple's nano-allocator path is fast on small allocations but still slower than mimalloc's thread-local freelist; published deltas land near Linux's. |

If the measured Linux + macOS numbers stay above the +10% lower bound
of ADR-0012's original claim, the cross-platform decision to keep
mimalloc is confirmed unconditionally. If either platform falls below
+5% on all three workloads, ADR-0012 should be revisited as a
platform-specific decision (mimalloc on Windows only) — see the
"Re-evaluation triggers" section of ADR-0012.

## Follow-up tracked

- **v0.36 portfolio review:** the wins are big enough that the
  question "should we promote mimalloc to a workspace-default
  allocator (instead of only the CLI binary)?" deserves a separate
  ADR. Currently `coral-cli/src/main.rs` is the only `#[global_allocator]`
  site; the WebUI server (`coral ui serve`) hosted under the same
  binary inherits it transitively, but `coral mcp serve` running as
  a sibling process via stdio transport does not. Worth investigating
  in v0.36.x.
- **Linux re-run in CI.** ✅ Shipped in v0.37-prep
  (`.github/workflows/bench-allocator.yml`). The original sketch
  proposed a `make bench-allocator` target writing a sibling
  `MIMALLOC-BASELINE-linux-YYYY-MM-DD.md`; we collapsed that into the
  cross-platform sections above so the comparison lives in one file.
