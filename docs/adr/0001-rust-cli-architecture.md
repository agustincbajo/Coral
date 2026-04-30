# ADR 0001 — Rust CLI architecture

**Date:** 2026-04-30  
**Status:** accepted

## Context

The original Karpathy LLM Wiki proposal sketches a set of bash scripts (`wiki-bootstrap.sh`, `wiki-ingest.sh`, etc.) that orchestrate the LLM via curl/jq calls to the Anthropic API. That works for a personal vault but has problems for a distributable product:

- Bash scripts are hard to test, hard to refactor, and platform-fragile.
- Each script reinvents YAML frontmatter parsing, wikilink extraction, lint checks.
- No type-safety across the pipeline — drift between scripts is invisible until runtime.

We want a single distributable artifact, fast on cold start, with a tested core.

## Decision

Build Coral as a **Rust workspace with 5 crates + a single `coral` binary**:

- `coral-core` — pure types and parsing (frontmatter, wikilinks, pages, index, log, gitdiff, walk).
- `coral-lint` — structural + semantic lint, depends on `coral-core` and `coral-runner`.
- `coral-runner` — `Runner` trait + `ClaudeRunner` (subprocess) + `MockRunner` (tests).
- `coral-stats` — wiki health computation.
- `coral-cli` — clap dispatcher; depends on the other four.

Edition 2024, rust-version 1.85, deps pinned in workspace `[workspace.dependencies]`.

## Consequences

**Positive:**
- Each crate is independently testable (`coral-core` has 68 unit tests with zero external dependencies).
- Strong types prevent classes of bugs (e.g., `Confidence::try_new` rejects out-of-range values at the type boundary).
- Single `cargo install --git ... coral-cli` for end users; one binary <8MB stripped.
- Parallelization via `rayon` is straightforward in Rust (impossible in bash).
- The `Runner` trait makes every LLM-touching path test-friendly via `MockRunner`.

**Negative:**
- Rust toolchain is heavier than bash for casual contributors. Mitigated by `rust-toolchain.toml` pinning.
- Iteration on prompts is slightly slower than editing a `.md` file standalone — but prompts are versioned in `template/prompts/` and reloaded at runtime, so no rebuild needed for prompt changes.

## Alternatives considered

- **Pure bash scripts (Karpathy original)**: rejected for the reasons above.
- **Python CLI**: would be similar in ergonomics; rejected to match the rest of the user's stack (Rust) and to get a single statically-linked binary.
- **Single monolithic crate**: rejected — separation of concerns paid off in testing and made the multi-agent dev flow tractable (each agent owned one crate at a time).
