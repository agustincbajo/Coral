---
slug: why-rust
type: synthesis
last_updated_commit: 213ac997cf61ad89610b3cfbe40af05e6b7fa8a8
confidence: 0.9
sources:
  - docs/adr/0001-rust-cli-architecture.md
backlinks:
  - cli
  - core
status: reviewed
---

# Why Rust for Coral

Coral could have been written in bash, Python, or Go. We chose Rust. The full decision is in [ADR 0001](../../docs/adr/0001-rust-cli-architecture.md); this is the synthesis.

## Drivers

1. **Test surface.** Each crate is independently testable with zero external dependencies. `coral-core` has 68 unit tests that run in <100ms.
2. **Strong types.** `Confidence::try_new` rejects out-of-range values at the type boundary. `PageType` and `Status` are exhaustive enums that never let an invalid string through.
3. **Single binary.** `cargo install --git` produces one statically-linked binary <8MB stripped. No runtime deps for end users.
4. **Parallelism.** `rayon::par_iter` makes parallel page walks trivial. Bash and Python would need GNU parallel or asyncio gymnastics.
5. **Performance.** A 500-page wiki scans + parses in <50ms. The cold-start of `coral lint --structural` is <100ms.

## Trade-offs accepted

- Rust toolchain heavier than bash. Mitigated by `rust-toolchain.toml` pinning.
- Iteration on prompts is slightly slower than editing a `.md` file standalone. Mitigated by versioning prompts in `template/prompts/` and reloading at runtime — no rebuild for prompt changes.

## Alternatives we ruled out

- **Bash scripts (Karpathy original)**: too hard to test, too platform-fragile.
- **Python CLI**: similar ergonomics but heavier deploys (no static binary), and we wanted to match the rest of the user's stack.
- **Go**: comparable to Rust on most axes. Rust won on the type-safety + ergonomic ADT support (enums with data), which we use heavily for `LintIssue`, `RunnerError`, `PageType`, `Status`.

## Confidence

This decision is `0.9` confidence because we've shipped v0.1.0 and the Rust choice never blocked us. If we hit a wall (e.g., needing Python ML libs in-process for embeddings), we'd revisit — but bridging to Python via subprocess is cheap.
