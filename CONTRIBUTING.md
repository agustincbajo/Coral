# Contributing to Coral

Thanks for the interest. PRs welcome — there are three guardrails plus a few conventions worth knowing up front.

## Guardrails (every PR must pass)

1. **Local CI green before push:**
   ```bash
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace --all-features
   ```
   The CI workflow at `.github/workflows/ci.yml` runs the same gates plus MSRV (Rust 1.85), `bc-regression`, cross-platform smoke (ubuntu + macOS), `cargo deny`, and `cargo audit`.

2. **Backward compatibility is sacred.** v0.15 single-repo workflows must keep working byte-for-byte on every release. Pinned by `crates/coral-cli/tests/bc_regression.rs`; that suite runs as a dedicated CI job. If your PR touches anything in the wiki path (init, ingest, status, lint, search, export, …), add or update the relevant fixture there.

3. **Wiki drift control.** If your PR touches a wiki slug, run `coral lint --all` (and `coral lint --check-spec-vs-server` once that's wired) and update the relevant page.

## Local setup

```bash
git clone https://github.com/agustincbajo/Coral
cd Coral
./scripts/dev-setup.sh           # Linux/macOS — installs cargo-sweep, sccache, cargo-nextest
# or:
./scripts/dev-setup.ps1          # Windows (PowerShell)
cargo build --workspace
cargo test --workspace
```

Rust toolchain pinned to **1.85** in `rust-toolchain.toml`. If your `rustup` is older, `rustup update stable`.

`scripts/dev-setup.*` is idempotent and the only setup step beyond the toolchain. It installs the disk-management tooling (`cargo-sweep`, `sccache`, `cargo-nextest`) and wires `sccache` as your global `rustc` wrapper. The full mechanics — why incremental compilation is off in this repo, when to run `cargo sweep`, what disk budget to target — live in **[`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md)**. Read it once before the first long iteration session; with the mechanics in place `target/` routinely fits in 5–8 GB instead of growing past 30 GB.

For environments + testing development you'll also want:
- `docker compose` v2.22+ (or `podman compose`, or `docker-compose` v1)
- `curl`
- `git` 2.30+

## Architecture orientation

Coral is a Cargo workspace with 8 crates. Each owns one concern; the trait families (`Runner`, `EnvBackend`, `TestRunner`, `ResourceProvider` / `ToolDispatcher`) keep concrete implementations swappable.

```
coral-cli ─→ coral-core ─→ rusqlite, fs4, walkdir, serde, toml, chrono, rayon
          ─→ coral-env  ─→ coral-core
          ─→ coral-test ─→ coral-env, coral-core
          ─→ coral-mcp  ─→ coral-core
          ─→ coral-runner (no internal deps)
          ─→ coral-lint  ─→ coral-core, coral-runner
          ─→ coral-stats ─→ coral-core
```

When you're adding a feature, the question to answer first is **which crate owns it?** New backend / runner / provider goes behind the existing trait — copy the `Mock*` impl shape and the `thiserror`-typed error layout. New CLI subcommand goes in `coral-cli/src/commands/<name>.rs`, exported via `commands::mod.rs`, dispatched from `main.rs`'s `Cmd` enum.

For deeper notes see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Conventions

- **Comments:** explain *why*, not *what*. Don't comment what well-named identifiers already say. Don't reference tasks, PRs, or callers in code comments — those rot. The PR description is for that.
- **Error messages:** actionable. "neither `docker compose` nor `docker-compose` is on PATH" beats "BackendNotFound". Tell the user how to fix it.
- **Tests:** one assertion-shape per test. Avoid sprawling integration tests that fail in 3 different ways at once. The `crates/coral-cli/tests/multi_repo_interface_change.rs` file is a reasonable shape: one scenario per test function.
- **Dependencies:** one new dep per PR. If you need to pull a heavy crate (libcurl FFI, tokio runtime, etc.), open a discussion first — Coral keeps the dep tree slim by design.
- **No emojis in code or commits** unless the user explicitly asked. Output text gets emojis only in CLI status lines (`✔ created …`).

## Commit messages

We follow conventional commits loosely:

```
feat(<scope>): <one-liner>
fix(<scope>): <one-liner>
docs: <one-liner>
test: <one-liner>
chore: <one-liner>
release(vX.Y.Z): <one-liner>
```

Body explains the *why* — what problem the PR addresses and any non-obvious decisions.

## Larger contributions

For new `EnvBackend` (e.g. `KindBackend`), `TestRunner` (e.g. `PropertyBasedRunner`), or MCP transport (e.g. HTTP/SSE), open a [GitHub Discussion](https://github.com/agustincbajo/Coral/discussions) first. The PRD doc tracks design decisions across the v0.16+ evolution and is the source of truth for trade-offs.

## Reporting bugs

Open an [issue](https://github.com/agustincbajo/Coral/issues) with:
- `coral --version`
- The exact command that failed
- Full stderr (with `RUST_LOG=coral=debug,info` if relevant)
- Minimal reproduction (a `coral.toml` + the surrounding files if it's a multi-repo issue)

## License

By contributing, you agree your contributions will be licensed under the [MIT License](LICENSE).
