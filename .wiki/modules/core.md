---
slug: core
type: module
last_updated_commit: 721050563f1ed29954b279fe334bf6bc8c8e2c34
confidence: 0.95
sources:
- crates/coral-core/src/
backlinks:
- cli
- lint
- page
- wiki-index
status: verified
---

# `coral-core` — types and parsing

Pure Rust crate with zero LLM coupling. Lives at `crates/coral-core`. 68 unit tests + 2 ignored (real-git smoke).

## Modules

- `error.rs` — `CoralError` enum (thiserror) + `Result<T>` alias.
- `frontmatter.rs` — parses YAML frontmatter at the head of a Markdown document. Strict types: `PageType`, `Status`, `Confidence` (0.0..=1.0).
- `wikilinks.rs` — `extract(content) -> Vec<String>`. Skips fenced code blocks and escaped `\[[...]]`. See [[wikilink-extraction]].
- `page.rs` — `Page` = frontmatter + body + path. See [[page]].
- `index.rs` — `WikiIndex` with `last_commit` anchor. See [[wiki-index]].
- `log.rs` — append-only operation log.
- `gitdiff.rs` — shells to `git diff --name-status` (parser is pure + testable; runner is impure).
- `walk.rs` — `rayon`-parallel page reader; skips hidden + `_archive/` + non-`.md`.

## Why zero LLM coupling

Everything in `coral-core` is deterministic and unit-testable. Parsing logic, file walks, model invariants. The LLM lives in [[runner]], the lint that uses the LLM lives in [[lint]], but `coral-core` itself is pure.

This is the layer that future tooling (a Python binding, a WASM build, an editor extension) can consume.
