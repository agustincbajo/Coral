# Changelog

All notable changes to Coral are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.1] - 2026-05-01

### Added

- **Embeddings-backed search** (`coral search --engine embeddings`): semantic similarity via Voyage AI `voyage-3`. Embeddings cached at `<wiki_root>/.coral-embeddings.json` (schema v1, mtime-keyed per slug, dimension-aware). Only changed pages are re-embedded between runs. `--reindex` forces a full rebuild. `--embeddings-model` overrides the default `voyage-3`. Requires `VOYAGE_API_KEY` env var. Falls back to a clear error when missing. TF-IDF (`--engine tfidf`) remains the default — no API key, works offline.
- **`coral_core::embeddings::EmbeddingsIndex`**: new module with cosine-similarity search, prune-by-live-slugs, JSON load/save, schema versioning. 9 unit tests.
- **Voyage provider** at `coral_cli::commands::voyage`: shells to curl (same pattern as `notion-push`), batches input into 128-item chunks (Voyage's limit), parses by `index` field for ordering safety, surfaces curl/HTTP errors with full stdout for debugging. 2 unit tests + 1 ignored real-API smoke.
- **`coral init` `.gitignore`** also lists `.coral-embeddings.json` so the cache stays out of source control alongside `.coral-cache.json`.

### Changed

- **ADR 0006** updated with the v0.3.1 status: embeddings now ship in JSON storage; sqlite-vec migration is deferred to v0.4 if/when wiki size pressures the JSON format (~5k pages).

## [0.3.0] - 2026-05-01

### Added

- **mtime-cached frontmatter parsing**: new `coral_core::cache::WalkCache` persists parsed `Frontmatter` keyed by file mtime in `<wiki_root>/.coral-cache.json`. `walk::read_pages` consults the cache before YAML parsing — files whose mtime hasn't changed since the previous walk skip the deserialization step, with body re-extraction handled by a new pure helper `frontmatter::body_after_frontmatter`. Wikis ≥200 pages should see ~30 % faster `coral lint` / `coral stats`. Schema-versioned (`SCHEMA_VERSION = 1`) — future bumps invalidate stale caches automatically. `coral init` now writes `<wiki_root>/.gitignore` with a `.coral-cache.json` entry to keep the cache out of source control. Cache writes are best-effort: a failure to persist the cache logs a warning but does not fail the walk.
- **`coral export --format jsonl --qa`**: invokes the runner per page with a new `qa-pairs` system prompt and emits 3–5 `{"slug","prompt","completion"}` lines per page for fine-tuning datasets. Malformed runner output is skipped with a warning. Add `--provider gemini --model gemini-2.5-flash` for a cheaper batch run. Override the system prompt at `<cwd>/prompts/qa-pairs.md` (priority: local override > embedded `template/prompts/qa-pairs.md` > hardcoded `QA_FALLBACK`). Default jsonl behavior (stub prompt, no runner) is unchanged.

### Deferred to v0.3.1

- **sqlite-vec embeddings search** (originally part of v0.3 roadmap): kept as a separate sprint because it requires API-key management for an embedding provider (Voyage / Anthropic when shipped) plus end-to-end testing against a real provider. TF-IDF in v0.2+ stays as the search default.

## [0.2.1] - 2026-05-01

### Added

- **`coral notion-push`**: thin wrapper over `coral export --format notion-json` that POSTs each page to a Notion database via curl. Reads `NOTION_TOKEN` + `CORAL_NOTION_DB` env vars or flags. `--type` filter, `--dry-run` preview. Wired with 4 unit tests + 2 integration tests (no-token failure, dry-run does not call curl).
- **`ClaudeRunner::run_streaming` honors `prompt.timeout`** (was a TODO in v0.2). Reader runs in a separate thread; main loop waits with `recv_timeout` and kills the child + cleans up if the deadline elapses. New non-`#[ignore]` test `claude_runner_streaming_timeout_kills_child` invokes `/usr/bin/yes` (writes forever, ignores args) with a 200 ms deadline and asserts `RunnerError::Timeout` returns within 2 s.

### Documentation

- **SCHEMA.base.md** explicit wikilinks section: `[[X]]` resolves by frontmatter slug, NOT by `[[type/slug]]`. Lint flags broken links if you use the prefixed form. Documents the convention with a comparison table and notes that `#anchor` / `|alias` suffixes still resolve by the part before `#` / `|`. New `template_validation` test asserts the section is present.

## [0.2.0] - 2026-05-01

### Added

- **`bootstrap`/`ingest --apply`** (issue #1): both LLM-driven subcommands now mutate `.wiki/` when invoked with `--apply`. They parse the runner's YAML response (`Plan { plan: [PlanEntry { slug, action, type, confidence, body, ... }] }`), write pages via `Page::write()`, upsert entries into `index.md`, append `log.md`. Default behavior remains `--dry-run` (print plan, no mutations) for safety. Malformed YAML prints raw output and exits 1.
- **`walk` skips top-level system files** (issue #2): the wiki walker now skips `index.md` and `log.md` at the wiki root in addition to the existing `SCHEMA.md`/`README.md` skip. Eliminates the `WARN skipping page … missing field 'slug'` noise on every `coral lint` and `coral stats` invocation. Subdirectory files like `concepts/index.md` still parse normally.
- **CHANGELOG.md + cargo-release wiring** (issue #3): adopted Keep a Changelog 1.1.0 format with backfilled `[0.1.0]` entry. `release.toml` configures `cargo-release` to rotate `[Unreleased]` → `[X.Y.Z] - {date}` and update compare-links automatically. `release-checklist.md` updated.
- **Streaming `coral query`** (issue #4): `Runner` trait gained `run_streaming(prompt, &mut FnMut(&str))` with a default impl that calls `run()` and emits one chunk. `ClaudeRunner` overrides to read stdout line-by-line via `BufReader::read_line`. `MockRunner::push_ok_chunked(Vec<&str>)` enables tests. The `coral query` subcommand prints chunks as they arrive instead of buffering.
- **Prompt overrides** (issue #7): every LLM subcommand (`bootstrap`, `ingest`, `query`, `lint --semantic`, `consolidate`, `onboard`) now resolves its system prompt with priority `<cwd>/prompts/<name>.md` > embedded `template/prompts/<name>.md` > hardcoded fallback. New `coral prompts list` subcommand prints a table of each prompt's resolved source.
- **GeminiRunner** (issue #8): alternative LLM provider, opt-in via `--provider gemini` on any LLM subcommand or the `CORAL_PROVIDER=gemini` env var. v0.2 ships a stub that shells to a `gemini` CLI binary; if absent, returns `RunnerError::NotFound`.
- **`coral search`** (issue #5): TF-IDF ranking over slug + body across all wiki pages. `--limit` / `--format markdown|json` flags. Pure Rust, no embeddings, no API key — works offline. v0.3 will switch to embeddings (Voyage / Anthropic) per [ADR 0006](docs/adr/0006-local-semantic-search-storage.md). The CLI surface stays stable on upgrade.
- **Hermes quality gate** (issue #6): opt-in composite action (`.github/actions/validate`) and `wiki-validator` subagent (template/agents) that runs an independent LLM to verify wiki/auto-ingest PRs against their cited sources before merge. Configurable `min_pages_to_validate` threshold to keep token spend predictable on small PRs.
- **`coral sync --remote <tag>`** (issue #10): pulls the `template/` directory from any tagged Coral release via `git clone --depth=1 --branch=<tag>`. No new Rust deps — shells to `git`. Without `--remote`, behavior is unchanged: only the embedded bundle is used and a mismatched `--version` aborts. Passing `--remote` without `--version` errors fast.
- **`.coral-pins.toml` per-file pinning** (issue #11): `coral sync --pin <path>=<version>` and `--unpin <path>` flags persist into a TOML file at the repo root with a `default` version + an optional `[pins]` map. Backwards compatible with the legacy `.coral-template-version` single-line marker — when only the legacy file exists, `Pins::load` migrates it on the fly. The legacy marker is kept in sync so existing tooling that reads it still works.
- **`docs/PERF.md`** (issue #14): documented baselines, hyperfine methodology, profiling tips, and the release-profile config. README links to it from a new "Performance" section.
- **`coral export`** (issues #9 + #13): single subcommand with four output formats (`markdown-bundle`, `json`, `notion-json`, `jsonl`) for shipping the wiki to downstream consumers. Replaces what would have been per-target subcommands (Notion sync, fine-tune dataset) with a unified exporter. `--type` filters by page type, `--out` writes to a file. Decision rationale in [ADR 0007](docs/adr/0007-unified-export-vs-per-target-commands.md).
- **`coral-stats` JsonSchema** (issue #15): `StatsReport` derives `JsonSchema` (`schemars 0.8`), new `json_schema()` method, generated schema committed at `docs/schemas/stats.schema.json`. 5 additional unit tests cover self-link, no-outbound, perf 500-page baseline, schema validity, JSON roundtrip.
- **2 new ADRs**: [0006](docs/adr/0006-local-semantic-search-storage.md) (TF-IDF stub vs v0.3 embeddings) and [0007](docs/adr/0007-unified-export-vs-per-target-commands.md) (single `coral export` vs per-target commands).

### Changed

- **`[profile.release]`**: added `panic = "abort"` to shave ~50 KB off the stripped binary and skip unwinding tables. CLI panics are unrecoverable anyway.
- **`prompt_loader`**: added `load_or_fallback_in(cwd, …)` and `list_prompts_in(cwd, …)` variants that take an explicit working directory. Fixes a flaky test that raced against `set_current_dir` calls in other test binaries. The default `load_or_fallback` / `list_prompts` wrappers preserve the original API for production callers.

### Closed issues

#1, #2, #3, #4, #5, #6, #7, #8, #9, #10, #11, #13, #14, #15. (#12 — orchestra-ingest consumer repo — tracked separately.)

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

[Unreleased]: https://github.com/agustincbajo/Coral/compare/v0.3.1...HEAD
[0.3.1]: https://github.com/agustincbajo/Coral/releases/tag/v0.3.1
[0.3.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.3.0
[0.2.1]: https://github.com/agustincbajo/Coral/releases/tag/v0.2.1
[0.2.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.2.0
[0.1.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.1.0
