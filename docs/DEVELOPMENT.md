# Coral Development Mechanics

How to keep your Coral checkout fast, lean, and reproducible.

This document is the canonical reference for the workspace's build /
test / disk hygiene posture. Every choice below was measured during
the v0.32.x WebUI sprint where uncontrolled `cargo build` cycles
inflated `target/` to **38 GB on a single SSD** before the cleanup
mechanics described here were adopted.

If you're a new contributor: run `scripts/dev-setup.sh` (Linux/macOS)
or `scripts/dev-setup.ps1` (Windows) once. Everything below is
already wired in.

---

## TL;DR

```bash
# One-time bootstrap (installs cargo-sweep, sccache, cargo-nextest;
# wires `~/.cargo/config.toml` to use sccache as the rustc wrapper):
./scripts/dev-setup.sh           # Linux / macOS
./scripts/dev-setup.ps1          # Windows (PowerShell)

# Daily routine — same as before:
cargo build --release
cargo test --workspace

# Weekly hygiene (cron / scheduled task, also runnable on demand):
cargo sweep --time 7             # delete artifacts not touched in 7 days
```

If `du -sh target/` ever creeps past 10 GB, run `cargo sweep
--installed`. If past 25 GB, run `cargo clean` and rebuild. Neither
loses data — both rebuild from your source tree.

---

## What's already wired into the repo

These ship with the checkout — no per-contributor setup needed:

### `Cargo.toml` — `[profile.dev] debug = "line-tables-only"`

Default `debug = "full"` produces ~200 MB binaries per test fixture
because of DWARF symbols. `line-tables-only` keeps panic backtraces
and test failure locations readable while dropping per-variable
tables, taking each binary down to ~80 MB (**-60%**).

Variables in `gdb` / `lldb` may show as "<optimized out>". For a
CLI / server crate that's acceptable; if you need full debug info
temporarily, override with `RUSTFLAGS="-C debuginfo=2" cargo build`.

### `.cargo/config.toml` — `[build] incremental = false`

Incremental compilation caches live in
`target/<profile>/incremental/` and **never get garbage-collected**.
During a long session that touches many feature combinations they
balloon. Off here matches CI behavior (`CARGO_INCREMENTAL: 0` in
`.github/workflows/ci.yml`).

First build after `cargo clean` is ~30% slower; every subsequent
build in the same session is unaffected because cargo's in-process
query cache covers that.

Override per-command if you really want incremental:

```bash
CARGO_INCREMENTAL=1 cargo build
```

---

## Tooling — installed by `scripts/dev-setup.*`

### `cargo-sweep` — periodic artifact cleanup

Borra los artifacts que **no usó el último build**, sin tocar lo que
sí. Idempotente; corre cuando quieras.

```bash
cargo sweep --installed          # remove unused artifacts (recommended)
cargo sweep --time 7             # remove anything not touched in 7 days
cargo sweep --maxsize 5G         # keep target/ ≤ 5 GB
```

**Empirical numbers** from the v0.32.x sprint:

| Command | Frequency | Recovered |
|---|---|---|
| `cargo sweep --time 7` | weekly | 0.5–3 GB |
| `cargo sweep --installed` | when feel pressure | 5–15 GB |
| `cargo clean` | nuclear | everything; next build ~3 min |

### `sccache` — cross-project rustc cache

Caches `rustc` output keyed by source + flags. Builds across
branches / projects / `cargo clean` cycles **reuse** the cache.
Adds ~10 GB of its own cache in `~/.cache/sccache/` (configurable
via `SCCACHE_CACHE_SIZE`), but that's bounded — unlike `target/`
which grows unbounded.

`scripts/dev-setup.*` adds the wrapper to `~/.cargo/config.toml`:

```toml
[build]
rustc-wrapper = "sccache"
```

Check hit rate at any time:

```bash
sccache --show-stats
```

A healthy hit rate after a week is ≥ 60%. If it's < 30%, your work
is mostly novel compilations and sccache isn't pulling weight; you
can drop it via `unset RUSTC_WRAPPER` or removing the line.

### `cargo-nextest` — faster test runner

Drop-in replacement for `cargo test` that's ~40% faster and
produces less intermediate state:

```bash
cargo nextest run --workspace
```

The CI workflow can stay on `cargo test` — `nextest` is a developer
quality-of-life tool, not a contract change.

---

## Workspace hygiene rules

### 1. Don't run `--all-features` locally

Each feature combination leaves its own artifacts in `target/`.
`--all-features` activates them all at once and inflates the
artifact pile.

```bash
# ❌ Local: avoid
cargo build --workspace --all-features

# ✅ Local: test the specific feature set you're modifying
cargo build --features "ui webui"

# ✅ CI: --all-features is the gate (`.github/workflows/ci.yml`)
```

### 2. Prefer `cargo nextest` over `cargo test` for iteration

Faster wallclock + smaller `target/` footprint.

### 3. Don't shotgun `touch` on source files

`touch crates/coral-ui/src/static_assets.rs` invalidates the
incremental cache for that crate + every downstream consumer. Useful
when you need to force a rebuild after editing an embedded asset
(e.g. the SPA bundle), but **don't** make it a habit. Each touch is
~200 MB of new artifacts when downstream crates re-link.

### 4. Run `cargo sweep` after long sessions

If you've been iterating for hours with many `cargo build` /
`cargo test` calls, a quick `cargo sweep --installed` at the end
prevents accumulation.

### 5. `cargo clean` between major feature work

When switching from "M1 work" to "M2 work" (or any context with
substantially different code), `cargo clean` is cheap insurance:
~3 minute rebuild, predictable disk state.

---

## Disk budget targets

For the Coral workspace on a standard contributor's machine:

| Path | Healthy | Concerning | Action |
|---|---|---|---|
| `target/` | < 5 GB | 5–15 GB | `cargo sweep --installed` |
| `target/` | — | > 15 GB | `cargo clean` |
| `~/.cargo/registry/` | < 1 GB | 1–3 GB | `cargo cache --autoclean` |
| `~/.cache/sccache/` (Linux/macOS) | 10 GB | > 15 GB | adjust `SCCACHE_CACHE_SIZE` |
| `%LOCALAPPDATA%\Mozilla\sccache\` (Windows) | 10 GB | > 15 GB | adjust `SCCACHE_CACHE_SIZE` |
| `crates/coral-ui/assets/src/node_modules/` | ~150 MB | > 500 MB | `rm -rf && npm ci` |

`cargo cache --autoclean` is from the optional `cargo-cache` tool —
install with `cargo install cargo-cache`.

---

## CI vs local — what's different

| | Local default | CI |
|---|---|---|
| `incremental` | `false` (this repo) | `false` (`CARGO_INCREMENTAL=0`) |
| `--all-features` | avoid | always |
| `RUST_TEST_THREADS` | unset (parallel) | `1` for `test` + `coverage` |
| Profile | `dev` (debug = line-tables-only) | `release` for matrix |
| Test runner | `cargo test` or `cargo nextest` | `cargo test` |

The intent is: **local is fast iteration, CI is the gate.** Local
won't catch every CI failure (e.g. `--all-features` exclusive
modules), but the loop time is short enough that you'll see it on
the PR.

---

## Why this matters — empirical record

During the v0.32.x WebUI sprint (~24 commits in one session
delivering M1 + v0.32.1 + v0.32.2 + v0.32.3 + v0.33.0):

```
Before this mechanics: target/ grew to 38.0 GB
After cargo-sweep + cargo clean: 0 bytes
After one --release build with the new profile: ~1.0 GB
```

The combination of `incremental = false` + `debug =
"line-tables-only"` + periodic `cargo sweep` keeps a long session's
`target/` to **5–8 GB** on average instead of 30+ GB, with builds
~40% faster once `sccache` warms up.

---

## Troubleshooting

### "Build is much slower than I remember"

Probably `incremental = false` + cold `sccache`. Either:

- Live with the ~30% slowdown on the first build of the day (then
  fast for the rest of the session via cargo's in-process cache).
- Override with `CARGO_INCREMENTAL=1 cargo build` for hot iteration
  loops on a single file.
- Make sure `sccache --show-stats` shows non-zero hit rate after
  a few builds. If zero, check `rustc-wrapper` is set in
  `~/.cargo/config.toml`.

### "`target/` is back to 30+ GB"

`cargo sweep --installed`, then if still pressured, `cargo clean`.
If this happens every week, your workflow is hitting an edge case
this doc didn't anticipate — open an issue with the output of
`du -h --max-depth=2 target/ | sort -hr | head -20`.

### "Can I just use the default settings?"

Yes — none of this is mandatory. The repo will compile fine with
`incremental = true` overridden in your `~/.cargo/config.toml` and
no sccache. You'll just need more disk and more patience.

---

## Removing the mechanics

If you want to undo the per-contributor part of this setup:

```bash
# Remove the sccache wrapper from your global cargo config:
sed -i '/^rustc-wrapper = "sccache"/d' ~/.cargo/config.toml

# Uninstall the tools:
cargo uninstall cargo-sweep sccache cargo-nextest

# (Optional) Force incremental back on for Coral specifically by
# overriding the repo's .cargo/config.toml in your shell:
export CARGO_INCREMENTAL=1
```

The repo-side parts (`[profile.dev]` in `Cargo.toml`,
`.cargo/config.toml`) are shared across contributors and should be
proposed via PR if you want to revise them.
