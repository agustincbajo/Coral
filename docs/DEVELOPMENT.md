# Coral Development Mechanics

How to keep your Coral checkout fast, lean, and reproducible.

Empirical anchor: during the v0.32.x WebUI sprint, uncontrolled `cargo
build` cycles inflated `target/` to **38 GB on a single SSD**. With the
mechanics below, a comparable session keeps `target/` in the 5–8 GB
range.

---

## TL;DR

```bash
./scripts/dev-setup.sh           # Linux/macOS  (or dev-setup.ps1 on Windows)
cargo build --release
cargo test --workspace

# Weekly (cron-friendly):
cargo sweep --time 7
```

Setup is idempotent. Re-run anytime — costs ~5s when everything is
already in place.

---

## What ships in the repo

### `Cargo.toml` — `[profile.dev] debug = "line-tables-only"`

Each debug binary drops from ~200 MB to ~80 MB (**-60%**). Panic
backtraces and `cargo test` failure locations stay readable; per-
variable DWARF tables don't.

When you need full debug info for `lldb`/`gdb` variable inspection:

```bash
RUSTFLAGS="-C debuginfo=2" cargo build
```

### CI env: `CARGO_INCREMENTAL=0`

`.github/workflows/ci.yml` runs with incremental compilation off so the
job's `target/` doesn't drift across runs. **Local builds keep
incremental on by default** — the inner-dev loop is too sensitive to
the +30% wall-time cost to justify a global override.

The counterweight is `cargo sweep --time 7` once a week. If you want
incremental off locally, add to your `~/.cargo/config.toml`:

```toml
[build]
incremental = false
```

---

## Tooling installed by `scripts/dev-setup.*`

| Tool | What it does | When you use it |
|---|---|---|
| `cargo-sweep` | Deletes unused artifacts | Weekly + when `target/` > 15 GB |
| `sccache` | Cross-project `rustc` cache | Automatic via global `rustc-wrapper` |
| `cargo-nextest` | Faster test runner (~40%) | Daily `cargo nextest run --workspace` |

The setup script wires `rustc-wrapper = "sccache"` into your global
`~/.cargo/config.toml`. It refuses to overwrite a different wrapper
already configured there.

---

## Maintenance commands

| Command | Recovers | Frequency | Trade-off |
|---|---|---|---|
| `cargo sweep --time 7` | 0.5–3 GB | weekly | none — touch-stamp only |
| `cargo sweep --installed` | 5–15 GB | when target/ > 15 GB | next build slower (re-uses installed toolchain only) |
| `cargo clean` | everything | between major feature work | ~3 min next build |
| `cargo cache --autoclean` | 0.3–0.8 GB | monthly | none — registry pruning |

Cache size cap for `sccache`:

```bash
export SCCACHE_CACHE_SIZE=5G   # default 10 GB; set in shell rc
```

Check `sccache` is pulling weight:

```bash
sccache --show-stats   # healthy hit rate after a week ≥ 60%
```

---

## Disk budget

| Path | Healthy | Action when above |
|---|---|---|
| `target/` | < 5 GB | `cargo sweep --installed` |
| `target/` | — | `cargo clean` when > 15 GB |
| `~/.cargo/registry/` | < 1 GB | `cargo cache --autoclean` |
| sccache cache dir | configurable | adjust `SCCACHE_CACHE_SIZE` |
| `crates/coral-ui/assets/src/node_modules/` | ~150 MB | `rm -rf && npm ci` |

`cargo cache` is from the optional `cargo-cache` crate; install with
`cargo install cargo-cache` if you find yourself running it.

---

## Workspace hygiene rules

1. **Don't run `--all-features` locally.** Each feature combination
   leaves its own artifacts. Test the specific feature set you're
   modifying; let CI run the `--all-features` gate.

2. **Prefer `cargo nextest` over `cargo test` for iteration.** Faster
   and produces less intermediate state.

3. **Don't shotgun-`touch` source files.** Each touch invalidates the
   incremental cache for that crate + every downstream consumer. Use
   it deliberately when you need to force a rebuild after editing an
   embedded asset (e.g. the SPA bundle).

4. **`cargo clean` between major feature work.** When switching from
   one milestone to another, cheap insurance.

---

## CI vs local

| | Local default | CI |
|---|---|---|
| `incremental` | on | `CARGO_INCREMENTAL=0` |
| `--all-features` | avoid | always |
| `RUST_TEST_THREADS` | unset | `1` for Test (stable) + Coverage |
| Test runner | `cargo test` or `cargo nextest` | `cargo test` |

Local is fast iteration; CI is the gate. Local won't catch every CI
failure (e.g. `--all-features`-exclusive modules), but the loop time is
short enough that you'll see it on the PR.

---

## v0.34.0 onboarding-stack commands

M1 adds four new subcommands beyond the existing wiki/multi-repo/test
surface. Worth knowing for local iteration on PRs that touch the
plugin, the SessionStart hook, or the install scripts:

| Command                              | What it does                                                                                  | When you use it locally                                                                              |
|--------------------------------------|-----------------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------|
| `coral self-check [--quick] [--full] [--format=json] [--print-schema]` | Diagnostic envelope (PRD App. F) covering binary, providers, wiki, manifest, CLAUDE.md, MCP, UI, update-available.   | Before pushing changes that touch the onboarding surface. `--print-schema` for the CI contract gate. |
| `coral doctor --wizard`              | Interactive 4-path provider mini-wizard (Anthropic / Gemini / Ollama / claude CLI). Writes `.coral/config.toml`. | When testing the provider config write path without going through the plugin.                        |
| `coral self-upgrade [--check-only] [--version vX.Y.Z]` | Replace the running binary with the latest same-major release. Atomic rename on Unix, MoveFileEx on Windows.       | Verifying a new release artifact in-place. `--check-only` is the no-op variant.                      |
| `coral self-uninstall [--keep-data]` | Remove the binary + `~/.coral/` (config + logs). `.wiki/` stays put.                          | Verifying clean-uninstall before tagging a release.                                                  |
| `coral self-register-marketplace`    | Patch the project-scope `.claude/settings.json` so Claude Code already knows about the Coral marketplace.            | What `install.sh --with-claude-config` calls under the hood.                                         |

Each command lives in `crates/coral-cli/src/commands/<name>.rs`. The
`SelfCheck` JSON envelope is a frozen contract (PRD Appendix F) — its
schema is committed at `.ci/self-check-schema.json` and the
`schema-contract` CI step fails on drift. Bump
`SELF_CHECK_SCHEMA_VERSION` in lockstep with any breaking field
change.

## Troubleshooting

**"Build is much slower than I remember"** — `sccache` cold cache.
First build of the day primes it; subsequent builds get hits. Confirm
with `sccache --show-stats`. If hit rate stays < 30% after a week, you
can drop the wrapper (`unset RUSTC_WRAPPER` or remove it from
`~/.cargo/config.toml`).

**"`target/` is back over 15 GB"** — `cargo sweep --installed`, then
`cargo clean` if pressure remains. If this happens every week, file an
issue with `du -h --max-depth=2 target/ | sort -hr | head -20`.

**"I want to disable a piece of this setup"** — every choice is
local-overridable:

- `[profile.dev] debug = ...` — override per-command with
  `RUSTFLAGS="-C debuginfo=2"`.
- `sccache` — `unset RUSTC_WRAPPER` or delete the line from
  `~/.cargo/config.toml`.
- The tools themselves — `cargo uninstall cargo-sweep sccache cargo-nextest`.

The shared parts in the repo (the `Cargo.toml` profile, the CI env var)
require a PR if you want to revise them.
