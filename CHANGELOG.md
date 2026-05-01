# Changelog

All notable changes to Coral are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Prompt overrides** (issue #7): every LLM subcommand (`bootstrap`, `ingest`, `query`, `lint --semantic`, `consolidate`, `onboard`) now resolves its system prompt with priority `<cwd>/prompts/<name>.md` > embedded `template/prompts/<name>.md` > hardcoded fallback. New `coral prompts list` subcommand prints a table of each prompt's resolved source.
- **GeminiRunner** (issue #8): alternative LLM provider, opt-in via `--provider gemini` on any LLM subcommand or the `CORAL_PROVIDER=gemini` env var. v0.2 ships a stub that shells to a `gemini` CLI binary; if absent, returns `RunnerError::NotFound`.
- **`coral search`** (issue #5): TF-IDF ranking over slug + body across all wiki pages. `--limit` / `--format markdown|json` flags. Pure Rust, no embeddings, no API key — works offline. v0.3 will switch to embeddings (Voyage / Anthropic) per [ADR 0006](docs/adr/0006-local-semantic-search-storage.md). The CLI surface stays stable on upgrade.
- **Hermes quality gate** (issue #6): opt-in composite action (`.github/actions/validate`) and `wiki-validator` subagent (template/agents) that runs an independent LLM to verify wiki/auto-ingest PRs against their cited sources before merge. Configurable `min_pages_to_validate` threshold to keep token spend predictable on small PRs.
- **`coral sync --remote <tag>`** (issue #10): pulls the `template/` directory from any tagged Coral release via `git clone --depth=1 --branch=<tag>`. No new Rust deps — shells to `git`. Without `--remote`, behavior is unchanged: only the embedded bundle is used and a mismatched `--version` aborts. Passing `--remote` without `--version` errors fast.
- **`.coral-pins.toml` per-file pinning** (issue #11): `coral sync --pin <path>=<version>` and `--unpin <path>` flags persist into a TOML file at the repo root with a `default` version + an optional `[pins]` map. Backwards compatible with the legacy `.coral-template-version` single-line marker — when only the legacy file exists, `Pins::load` migrates it on the fly. The legacy marker is kept in sync so existing tooling that reads it still works.
- **`docs/PERF.md`** (issue #14): documented baselines, hyperfine methodology, profiling tips, and the release-profile config. README links to it from a new "Performance" section.
- **`coral export`** (issues #9 + #13): single subcommand with four output formats (`markdown-bundle`, `json`, `notion-json`, `jsonl`) for shipping the wiki to downstream consumers. Replaces what would have been per-target subcommands (Notion sync, fine-tune dataset) with a unified exporter. `--type` filters by page type, `--out` writes to a file. Decision rationale in [ADR 0007](docs/adr/0007-unified-export-vs-per-target-commands.md).

### Changed

- **`[profile.release]`**: added `panic = "abort"` to shave ~50 KB off the stripped binary and skip unwinding tables. CLI panics are unrecoverable anyway.

## [0.1.0] - 2026-04-30

### Added

- Cargo workspace with 5 crates: `coral-cli`, `coral-core`, `coral-lint`, `coral-runner`, `coral-stats`.
- `coral` CLI binary with 10 subcommands (init, bootstrap, ingest, query, lint, consolidate, stats, sync, onboard, search).
- Frontmatter parsing with `Frontmatter`, `PageType`, `Status`, `Confidence` types.
- Wikilink extraction with code-fence and escape handling.
- `Page`, `WikiIndex`, `WikiLog` data model with idempotent operations.
- `gitdiff` parser + runner (shells to `git diff --name-status`).
- `walk::read_pages` rayon-parallel page reader.
- 5 structural lint checks: broken wikilinks, orphan pages, low confidence, high confidence without sources, stale status.
- `Runner` trait + `ClaudeRunner` (subprocess wrapper) + `MockRunner` (testing).
- `PromptBuilder` with `{{var}}` substitution.
- `StatsReport` with markdown + JSON renderers.
- Embedded `template/` bundle: 4 subagents, 4 slash commands, 4 prompt templates, base SCHEMA, GH workflow template.
- 3 composite GitHub Actions: ingest, lint, consolidate.
- Multi-agent build pipeline (orchestrator/coder/tester loop).
- 150 tests + 3 ignored. Binary 2.8MB stripped.

### Documentation

- README, INSTALL, USAGE, ARCHITECTURE.
- 5 ADRs: Rust CLI architecture, Claude CLI vs API, template via include_dir, multi-agent flow, versioning + sync.
- Self-hosted `.wiki/` with 14 seed pages (cli/core/lint/runner/stats modules + concepts + entities + flow + decisions + synthesis + operations + sources).

[Unreleased]: https://github.com/agustincbajo/Coral/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.1.0
