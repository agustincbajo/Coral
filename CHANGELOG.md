# Changelog

All notable changes to Coral are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0] - 2026-05-01

### Added

- **`coral consolidate --apply`** ([crates/coral-cli/src/commands/consolidate.rs](crates/coral-cli/src/commands/consolidate.rs)): parses the LLM's YAML proposal into structured `merges:` / `retirements:` / `splits:` arrays and applies the safe subset — every `retirements[].slug` becomes `status: stale`. `merges[]` and `splits[]` are surfaced as warnings (body merging / partitioning isn't safely automated). Default remains dry-run preview. 4 unit tests.
- **`coral onboard --apply`** ([crates/coral-cli/src/commands/onboard.rs](crates/coral-cli/src/commands/onboard.rs)): persists the LLM-generated reading path as a wiki page at `<wiki>/operations/onboarding-<slug>.md` (slug = profile lowercased + dashed; runs with the same profile overwrite). New `profile_to_slug` helper handles spaces, case, special chars. 3 unit tests including slug normalization.

### Changed

- **Streaming runner unification** ([crates/coral-runner/src/runner.rs](crates/coral-runner/src/runner.rs)): extracted `run_streaming_command` helper that ClaudeRunner, GeminiRunner, and LocalRunner all delegate to. GeminiRunner and LocalRunner override `Runner::run_streaming` to use it instead of the trait's default single-chunk fallback — `coral query --provider gemini`/`local` now sees the response token-by-token (when the underlying CLI streams). Timeout + auth-detection semantics are identical across all three runners.

### Documentation

- **USAGE.md fully refreshed** for v0.4 + v0.5: `bootstrap`/`ingest --apply` (drops the stale "v0.1, does not write pages" note), `coral query` telemetry, `coral lint --staged`/`--auto-fix [--apply]`, `coral consolidate --apply`, `coral onboard --apply`, `coral search --embeddings-provider <voyage|openai>`, `coral export --format html`, plus brand-new sections for `coral diff` and `coral validate-pin`. Multi-provider intro now mentions `local` (llama.cpp). New CI section for the embeddings-cache composite action.

### Added (continued)

- **`LocalRunner`** ([crates/coral-runner/src/local.rs](crates/coral-runner/src/local.rs)): third real `Runner` impl alongside Claude and Gemini. Wraps llama.cpp's `llama-cli` (`-p` for prompt, `-m` for `.gguf` model path, `--no-display-prompt`, system prompt prepended). Selected via `--provider local` (or `local`/`llama`/`llama.cpp`). Standing wrapper-script escape hatch through `with_binary` for installs with non-standard flags. 8 unit tests cover argv shape, echo-substitute integration, not-found, non-zero + 1 ignored real-llama smoke (`LLAMA_MODEL` env required).
- **`--provider local` flag** wired in [crates/coral-cli/src/commands/runner_helper.rs](crates/coral-cli/src/commands/runner_helper.rs): `ProviderName::Local` variant + parser aliases. Available on every LLM subcommand (`bootstrap`, `ingest`, `query`, `lint --semantic`, `consolidate`, `onboard`, `export --qa`).
- **`coral lint --auto-fix`** ([crates/coral-cli/src/commands/lint.rs](crates/coral-cli/src/commands/lint.rs)): LLM-driven structural fixes. After lint runs, the runner receives a structured prompt with affected pages + issues and proposes a YAML plan: `{slug, action: update|retire|skip, confidence?, status?, body_append?, rationale}`. Default is **dry-run** (prints the plan); `--apply` writes changes back. Caps the LLM scope: it can downgrade confidence, mark stale, or append a short italic note — but cannot rewrite whole bodies or invent sources. Override the system prompt at `<cwd>/prompts/lint-auto-fix.md`. 4 unit tests cover YAML parsing (with fences + missing-action default-to-skip), apply-on-disk frontmatter+body changes, and retire-marks-stale.

- **`coral diff <slugA> <slugB>`** ([crates/coral-cli/src/commands/diff.rs](crates/coral-cli/src/commands/diff.rs)): structural diff between two wiki pages — frontmatter delta (type / status / confidence), source set arithmetic (common / only-A / only-B), wikilink set arithmetic, body length stats. Markdown or JSON output (`--format json`). Useful for spotting merge candidates, evaluating retirement, or reviewing wiki/auto-ingest PRs. 4 unit tests. (Future: `--semantic` flag for LLM-driven contradiction detection.)
- **`coral export --format html`** ([crates/coral-cli/src/commands/export.rs](crates/coral-cli/src/commands/export.rs)): single-file static HTML site of the wiki — embedded CSS (light + dark via `prefers-color-scheme`), sticky sidebar TOC grouped by page type, every page rendered as a `<section id="slug">`. `[[wikilinks]]` translate to in-page anchor links via a regex preprocessor that handles plain / aliased / anchored forms. New `pulldown-cmark` dep for Markdown→HTML (CommonMark + tables + footnotes + strikethrough + task lists). Drop the file on GitHub Pages / S3 / any static host — no build step. 3 unit tests.

- **`coral validate-pin`** ([crates/coral-cli/src/commands/validate_pin.rs](crates/coral-cli/src/commands/validate_pin.rs)): new subcommand that reads `.coral-pins.toml` (with legacy `.coral-template-version` fallback) and verifies each referenced version exists as a tag in the remote Coral repo via a single `git ls-remote --tags` call (no clone). Reports `✓` per pin / `✗` for any missing tag. Exit `0` when clean, `1` if any pin is unresolvable. `--remote <url>` overrides the default for forks/mirrors. 6 unit tests.
- **`coral lint --staged`**: pre-commit hook mode. Loads every page (graph stays intact for orphan / wikilink checks) but filters the report to issues whose `page` is in `git diff --cached --name-only` plus workspace-level issues (no `page`). Exits non-zero only when a critical issue touches a staged file. 3 unit tests cover staged-path parsing, filter membership, and workspace-level retention.
- **`embeddings-cache` composite action** ([.github/actions/embeddings-cache/action.yml](.github/actions/embeddings-cache/action.yml)): drop-in `actions/cache@v4` wrapper for `.coral-embeddings.json`. Cache key strategy `<prefix>-<ref>-<hashFiles(*.md)>` with branch-scoped fallback so a single-page edit reuses ~all vectors but cross-branch staleness is avoided. README CI section documents usage.

## [0.4.0] - 2026-05-01

### Added

- **`OpenAIProvider`** ([crates/coral-runner/src/embeddings.rs](crates/coral-runner/src/embeddings.rs)): second real `EmbeddingsProvider` impl. Same curl shell-out pattern as Voyage. Constructors `text_embedding_3_small()` (1536-dim, default) and `text_embedding_3_large()` (3072-dim). `coral search --embeddings-provider openai` selects it; needs `OPENAI_API_KEY`. 3 unit tests + 1 ignored real-API smoke.
- **`coral search --embeddings-provider <voyage|openai>`** flag: pick the embeddings provider per invocation. Default `voyage` preserves v0.3.1 behavior. The dimensionality auto-resolves per OpenAI model (`text-embedding-3-large` → 3072, others → 1536).
- **Real `GeminiRunner`** ([crates/coral-runner/src/gemini.rs](crates/coral-runner/src/gemini.rs)): replaces the v0.2 `ClaudeRunner::with_binary("gemini")` stub with a standalone runner that builds its own argv per gemini-cli conventions (`-p` for prompt, `-m` for model, system prompt prepended to user with blank-line separator). Keeps the public API stable (`new()`, `with_binary()`). Surfaces `RunnerError::AuthFailed` on 401-style failures via the shared `combine_outputs` + `is_auth_failure` helpers. 7 unit tests cover argv shape (4), echo-substitute integration (1), not-found (1), non-zero (1) + 1 ignored real-gemini smoke. Streaming uses the trait default (single chunk on completion); incremental streaming is a future improvement.

- **`EmbeddingsProvider` trait** ([crates/coral-runner/src/embeddings.rs](crates/coral-runner/src/embeddings.rs)): mirrors the `Runner` trait pattern but for vector embedding providers. Lets the search command and tests swap providers without recompiling against a specific HTTP shape. Ships with `VoyageProvider` (the prior `coral-cli/commands/voyage` curl shell-out, now an impl) and `MockEmbeddingsProvider` (deterministic in-memory provider for offline tests). 6 unit tests including swap-via-trait-object and a deterministic mock smoke. A second real provider (Anthropic embeddings when shipped, OpenAI text-embedding-3) lands as one new struct in this module.
- **Dedicated `EmbeddingsError` enum** with `AuthFailed`, `ProviderCall`, `Io`, `Parse` variants — surfaces actionable detail without depending on `RunnerError` (which is claude-specific).

- **`coral query` telemetry** ([crates/coral-cli/src/commands/query.rs](crates/coral-cli/src/commands/query.rs)): emits two `tracing::info!` events bracketing the runner call — `coral query: starting` (with `pages_in_context`, `model`, `question_chars`) and `coral query: completed` (with `duration_ms`, `chunks`, `output_chars`, `model`). Visible with `RUST_LOG=coral=info coral query "..."`. No effect on stdout streaming.

### Documentation

- **README "Auth setup" section** ([README.md](README.md)): covers local shell (`claude setup-token`), CI (`CLAUDE_CODE_OAUTH_TOKEN` secret), and the gotcha when running `coral` from inside Claude Code (the parent's `ANTHROPIC_API_KEY` doesn't work in the subprocess; the v0.3.2 actionable error now points users here). Embeddings provider auth (`VOYAGE_API_KEY`) is also documented.

### Changed

- **`coral notion-push` is dry-run by default**; `--apply` is the explicit opt-in to actually POST. Matches `bootstrap`/`ingest` semantics. **BREAKING**: the prior `--dry-run` flag was removed (no longer needed). USAGE.md updated.
- **`coral search --engine embeddings`** now goes through the `EmbeddingsProvider` trait. CLI surface unchanged; behavior identical against Voyage. The factory in `coral-cli/src/commands/search.rs` constructs a `VoyageProvider` from `VOYAGE_API_KEY` + `--embeddings-model`.
- **`coral-cli/src/commands/voyage.rs` deleted** — the curl shell-out lives in `coral-runner::embeddings::VoyageProvider`.

## [0.3.2] - 2026-05-01

### Fixed

- **`coral search` UTF-8 panic** ([crates/coral-core/src/search.rs:103](crates/coral-core/src/search.rs:103)): the snippet builder sliced the page body with raw byte offsets, panicking when `pos.saturating_sub(40)` or `pos + max_len` landed inside a multi-byte char (em-dash, accent, smart quote, emoji). Repro: `coral search "embeddings"` against any wiki containing `—`. Fixed by clamping both ends to the nearest UTF-8 char boundary via new `floor_char_boundary` / `ceil_char_boundary` helpers. Regression test `search_does_not_panic_on_multibyte_chars_near_match` exercises a body with `—` near the match.
- **`ClaudeRunner` silent auth failures** ([crates/coral-runner/src/runner.rs](crates/coral-runner/src/runner.rs)): `claude --print` writes 401 errors to stdout, so the previous code surfaced the user-facing message `error: runner failed: claude exited with code Some(1):` with empty trailing detail. Both `run` and `run_streaming` now combine stdout + stderr via a new `combine_outputs` helper, and a new `RunnerError::AuthFailed` variant is returned when the combined output matches an auth signature (`401`, `authenticate`, `invalid_api_key`). The variant's `Display` shows the actionable hint: "Run `claude setup-token` or export ANTHROPIC_API_KEY in this shell." 2 new unit tests cover the helpers.
- **Test flake `ingest_apply_skips_missing_page_for_update`** ([crates/coral-cli/src/commands/ingest.rs](crates/coral-cli/src/commands/ingest.rs)): `bootstrap.rs` and `ingest.rs` each had their own `static CWD_LOCK: Mutex<()>`, so cross-module tests racing on process cwd would intermittently land in an orphan directory and panic on cwd restore. Unified into a single `commands::CWD_LOCK` shared by all command modules. 5× workspace stress run is green.

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

[Unreleased]: https://github.com/agustincbajo/Coral/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.5.0
[0.4.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.4.0
[0.3.2]: https://github.com/agustincbajo/Coral/releases/tag/v0.3.2
[0.3.1]: https://github.com/agustincbajo/Coral/releases/tag/v0.3.1
[0.3.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.3.0
[0.2.1]: https://github.com/agustincbajo/Coral/releases/tag/v0.2.1
[0.2.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.2.0
[0.1.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.1.0
