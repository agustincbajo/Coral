# ADR 0003 — Template distribution via `include_dir!`

**Date:** 2026-04-30  
**Status:** accepted

## Context

Coral ships a skill bundle (4 subagents, 4 slash commands, 4 prompts, base SCHEMA, GH workflow template) that consumer repos extract via `coral sync`. Three options for distribution:

- **A) Git submodule** — consumer repo includes Coral as a submodule, copies files manually.
- **B) Remote download at runtime** — `coral sync` does `git clone --depth=1 --branch=$VERSION` to a tmpdir, rsyncs files in.
- **C) Embedded in the binary** — `include_dir!` macro bakes the entire `template/` tree into the binary at compile time.

## Decision

**Option C: `include_dir!`.**

```rust
static TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../template");
```

`coral sync` walks the embedded `Dir` and writes each entry to `<cwd>/template/<relative>`. Writes a `.coral-template-version` marker at `cwd` root.

## Consequences

**Positive:**
- **Single binary, zero filesystem dependencies.** A user can `cargo install` Coral on a fresh machine and `coral sync` works without network access.
- **Atomic version pinning.** The `template/` content is byte-identical to the binary's version. No drift possible.
- **Sync is fast** (~ms, just memory writes) — no `git clone` round-trip.
- **No GitHub auth needed at sync time.** Works on locked-down CI runners.

**Negative:**
- **Binary size grows with template size.** Currently the bundle adds ~25KB; if the template grows past ~1MB we may revisit.
- **Updating the template requires a Coral release.** A typo in a prompt means: PR → merge → tag → consumer runs `cargo install`. Mitigation: prompts can be overridden locally in the consumer repo (Coral falls back to the embedded version only when local is missing — TODO for v0.2).
- **Consumers can't pull "latest main" without rebuilding.** A `--version main` flag would need a remote-download fallback. Not done in v0.1.

## Alternatives considered

- **Submodule**: rejected — submodules are operationally painful, especially for transitive consumers.
- **Remote download at runtime**: deferred. Will be added when the first consumer asks for it; the implementation is a 30-line shell-out to `git clone` + rsync.
- **Hybrid (embedded for offline + remote for `--version main`)**: best long-term, but unnecessary complexity for v0.1.
