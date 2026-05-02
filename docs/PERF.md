# Performance

Coral aims for sub-100 ms cold-start for `init`, `lint --structural`, and `stats` on a 200-page wiki. The CLI is single-binary, statically linked, and reads everything from disk + an embedded template, so there's no IO beyond the local filesystem.

## Measured baselines (v0.1.0, M2 Pro)

| Operation | Wiki size | Time (debug) | Time (release) |
|---|---|---|---|
| `coral init` | empty | ~30 ms | ~10 ms |
| `coral lint --structural` | 14 pages | ~80 ms | ~25 ms |
| `coral stats` | 14 pages | ~70 ms | ~20 ms |
| `coral sync` | embedded template | ~40 ms | ~15 ms |

Numbers are illustrative — re-run `hyperfine` on your hardware before quoting them anywhere.

## Methodology

```bash
cargo install hyperfine
hyperfine \
  --warmup 3 \
  --runs 20 \
  --export-markdown perf-results.md \
  'coral lint --structural --wiki-root /tmp/test-wiki' \
  'coral stats --wiki-root /tmp/test-wiki' \
  'coral sync'
```

Run from a release build (`cargo install --path crates/coral-cli` or `cargo build --release`) to get representative numbers.

## Profiling

```bash
cargo install flamegraph
sudo cargo flamegraph --release --bin coral -- lint --structural
# Open flamegraph.svg in a browser; the wide red boxes are the hot paths.
```

On macOS use `sample` or `cargo instruments` (Xcode) instead of `cargo flamegraph` if perf events are not available.

## Known hot paths (v0.1)

1. `walk::list_page_paths` — walkdir + filter chain. Already parallel via rayon.
2. `frontmatter::parse` — `serde_yaml_ng::from_str` per page. Could be cached by `last_modified` mtime in v0.3.
3. `wikilinks::extract` — regex. Already lazy-compiled via `OnceLock`.

## Compiler tweaks

Active in `[profile.release]`:

- `lto = "thin"` — cross-crate inlining.
- `codegen-units = 1` — maximum optimization (slower compile).
- `strip = true` — removes debug info from the binary.
- `panic = "abort"` — saves ~50 KB and skips unwinding on panic. Fine for a CLI; if a panic ever fires we want the process to die hard.
- `opt-level = 3`.

The `regex` crate auto-detects SIMD on aarch64 (Apple Silicon) and amd64. No extra config needed.

## Benchmarks

`hyperfine` measures the CLI end-to-end; for sub-millisecond hot paths we use
`criterion` micro-benchmarks. Each crate ships its own `benches/` directory
and the workspace runner exercises all of them at once:

```bash
cargo bench --workspace
```

Results are written to `target/criterion/<bench-id>/` along with HTML reports.
The top-level summary is at `target/criterion/report/index.html` — open it in
a browser to see throughput, distribution, and run-over-run regression
comparisons. Criterion automatically diffs against the previous run, so a
typical workflow is:

1. Run `cargo bench --workspace` on `main` to capture a baseline.
2. Make a change.
3. Run `cargo bench --workspace` again — the report shows percentage deltas
   and flags statistically significant regressions or improvements.

For a sanity-only pass that runs every bench exactly once (no measurement),
use the criterion `--test` mode — useful in CI to catch broken benches without
paying the full bench wall-clock:

```bash
cargo bench --workspace -- --test
```

Current benches:

| Bench id | Crate | What it measures |
|---|---|---|
| `search/100_pages_2_token_query` | `coral-core` | TF-IDF over 100-page corpus, 2-token query |
| `wikilinks/extract_50_links` | `coral-core` | Regex extraction of 50 wikilinks from a body |
| `frontmatter/parse_5_field_block` | `coral-core` | `serde_yaml_ng::from_str` for a typical frontmatter |
| `walk/read_pages_100_pages_4_subdirs` | `coral-core` | End-to-end disk walk + parse for 100 pages |
| `lint/run_structural_100_pages` | `coral-lint` | All 7 pure structural checks against 100 pages |

The two context-aware lint checks (`commit_in_git`, `source_exists`) are not
benched here — they shell out to git / hit the filesystem and need their own
fixture story.

## Future work

- mimalloc / jemalloc allocator (issue follow-up).
- Cache parsed frontmatter by mtime (issue follow-up).
- Embeddings-backed search (issue #5 → v0.3).
