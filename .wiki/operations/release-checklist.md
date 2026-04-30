---
slug: release-checklist
type: operation
last_updated_commit: 213ac997cf61ad89610b3cfbe40af05e6b7fa8a8
confidence: 0.85
sources:
  - .github/workflows/release.yml
  - Cargo.toml
backlinks:
  - cli
status: reviewed
---

# Release checklist

Steps for cutting a Coral release. Owner: maintainer.

## Pre-release

1. **All tests green.** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check`.
2. **Bump version.** Edit `Cargo.toml` `[workspace.package].version` and the `crates/*/Cargo.toml` files (or use `cargo set-version --workspace X.Y.Z`).
3. **Update CHANGELOG.md** (when one exists — first one ships in v0.2).
4. **Run `coral lint --structural`** on this repo's `.wiki/` — must be exit 0.
5. **Commit + push** the version bump.

## Tag and release

```bash
git tag -a v0.X.Y -m "Coral v0.X.Y"
git push --tags
```

The `release.yml` GitHub Action handles the rest:

- Builds for `x86_64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`.
- Creates a GitHub Release with binaries attached.
- (Future) `cargo publish` for crates.io.

## Post-release

- Announce in repo Discussions / X.
- Update `.coral-template-version` in any owned consumer repos.
- Open an issue for any deferred TODO surfaced by the release.

## Rollback

```bash
# delete the bad tag locally and remotely
git tag -d v0.X.Y
git push --delete origin v0.X.Y
# then ship a fix and a new tag
```

GitHub releases are NOT auto-deleted by tag deletion — clean those up manually in the Releases UI.
