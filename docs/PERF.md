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

## Future work

- mimalloc / jemalloc allocator (issue follow-up).
- Cache parsed frontmatter by mtime (issue follow-up).
- Embeddings-backed search (issue #5 → v0.3).
