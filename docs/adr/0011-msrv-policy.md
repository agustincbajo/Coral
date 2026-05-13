# ADR 0011 — MSRV policy: pin via Cargo.toml + CI, not toolchain file

**Date:** 2026-05-13
**Status:** accepted (v0.35)

## Context

Coral promises a minimum supported Rust version (MSRV) so users
running `cargo install --locked coral-cli` on a slightly older
toolchain don't hit "feature `let_chains` is unstable" surprises.

The MSRV historically tracked a "stable - 4" rule. Pre-v0.35 it sat
at 1.85 — chosen because the workspace used `edition = "2024"`,
`let_else`, and `#[diagnostic::on_unimplemented]`, none of which
require anything newer.

Two surfaces declare a toolchain in this repo:

1. **`Cargo.toml`** — `workspace.package.rust-version = "1.85"`.
   This is what `cargo` compares against the resolved feature set
   when a user runs `cargo install`. Cargo refuses to compile when
   the toolchain is below the declared rust-version.
2. **`rust-toolchain.toml`** — `[toolchain] channel = "stable"`.
   This pins what `rustup` installs for repo contributors. We
   deliberately use `stable` (not a fixed minor like `1.85`) so
   contributors get whatever-is-current and CI catches drift, rather
   than every PR pinning a stale toolchain.

The repeatedly-asked question: where should MSRV "live"?

## Decision

**Pin MSRV via `Cargo.toml`'s `rust-version` field and a dedicated
CI job that builds against exactly that minor. Keep
`rust-toolchain.toml` on `stable` for contributor convenience. Never
bump MSRV silently — every bump requires this ADR's update + a
two-version CHANGELOG notice.**

v0.35 Phase C bumps MSRV 1.85 → 1.89. The reasoning is below; the
mechanics (where to update what) are above.

## Rationale

- **Cargo's `rust-version` is the contract.** When a downstream
  consumer (a Linux distro packager, a homelab user pinning to
  rustup 1.87) runs `cargo install --locked coral-cli`, cargo
  compares the toolchain against `Cargo.toml`'s field. That field
  is the wire contract; anything else is internal repo policy.
- **`rust-toolchain.toml` causes drift if used as the MSRV gate.**
  Pinning it to a specific minor means every clone uses that minor,
  even if the MSRV documented in the README is older. CI then tests
  only the pinned minor, not the declared MSRV — and the contract
  silently drifts upward as `cargo` features land in newer minors.
  Validator F flagged this pre-v0.35.
- **CI is the enforcer.** The `Test (MSRV X.Y)` job in
  `.github/workflows/ci.yml` runs `cargo build --workspace --locked`
  using `dtolnay/rust-toolchain@master` pinned to `$CORAL_MSRV`. If
  a future feature use bumps the actual floor without a
  Cargo.toml bump, CI fails with a clear error. The job name is
  hard-coded (GitHub Actions doesn't expand `env.X` in job-level
  `name:` fields — discovered the painful way in v0.22.5).

### Why 1.89 specifically (v0.35 bump)

- **`let_chains` stabilized in 1.88.** Enables `if let Some(x) = …
  && cond` and `while let … && …` without intermediate `match`
  ladders. Coral has 11 sites that benefit (validator F sampled
  `coral-lint::structural`, `coral-runner::shell_lock`,
  `coral-cli::commands::doctor`). Not blocking — the refactor is
  trivial — but the ergonomic floor improves.
- **`&raw const/mut` is NOT new** (validator F correction to the
  audit). It stabilized in 1.82; the audit had cited it as a 1.89
  feature, which would've been a bad reason to bump.
- **No transitive blocks.** Audit F + validator F both ran
  `cargo +1.89 build --workspace --locked` against the current
  Cargo.lock — clean. The tokio/serde/clap/regex/etc. chains all
  build on 1.89.
- **Conservative anchor.** 1.89 (released ~Q4 2025) is far enough
  back that every reasonable Linux distro packager has it
  available, and far enough forward that we don't carry pre-1.88
  workarounds for `let_chains`.

## Alternatives considered

- **Pin `rust-toolchain.toml` to the MSRV minor.** Rejected: forces
  every contributor onto a stale toolchain, masks MSRV drift on PRs.
- **Drop MSRV declaration entirely** (`rust-version` removed, CI
  matrix tests stable only). Rejected: cargo's `--locked` check
  becomes a noop, and packagers lose the install-time contract.
- **Bump MSRV more aggressively** (track stable - 2 or stable - 1).
  Rejected: punishes users on slightly older toolchains for no
  Coral-feature gain. We'll bump when there's a reason; bumping for
  the sake of bumping is churn.

## Consequences

- **Every MSRV bump goes through three places:**
  1. `Cargo.toml` `workspace.package.rust-version`.
  2. `.github/workflows/ci.yml` `CORAL_MSRV` env var.
  3. The job `name:` field (`Test (MSRV X.Y)`) — hard-coded, no
     env expansion in GHA job-level names.
- **CHANGELOG must call out the bump.** Two release notes (the
  bump-release and the next-release) carry an MSRV note so
  packagers see it twice.
- **This ADR must be updated** with the new MSRV minor and the
  rationale (which features unlock, which transitive deps required
  it).
- **Downgrade is hard.** Once we use a 1.89-only feature, going
  back means refactoring the use site. The ADR's bump record is
  the audit trail.

## References

- v0.35 Phase C audit F (transitive-dep MSRV check).
- BACKLOG.md item: "MSRV ratchet plan" (track when 1.90, 1.91 etc.
  are worth the bump).
- Cargo Book §3 "rust-version" — the canonical spec for the field.
