# Roadmap

Estado consolidado del backlog. Cada release tiene su sección con items resueltos.

**Última actualización**: 2026-05-02 — v0.15.0 shipped. 15 releases this session (v0.3.2 → v0.15.0). 602 tests + 16 ignored. Todo lo implementable sin LLM access está en producción.

---

## Items bloqueados / fuera de alcance

| # | Item | Bloqueador real |
|---|---|---|
| B1 | Dogfooding self-hosted `.wiki/` | **Doble blocker**: (a) el maintainer tiene que correr `claude setup-token` interactivamente — el sandbox NO permite que el agente lo haga porque OAuth flows crean auth state persistente; (b) si el token se pega en chat (intentado en esta sesión), el sandbox bloquea su uso porque embeber tokens chat-leak en env vars de subprocesses es leak surface adicional. La self-hosted wiki sigue en `213ac99` (anterior a v0.1.0). Workaround: maintainer corre `claude setup-token` en su terminal local + corre `coral ingest --apply` ahí. Plan listo en `~/.claude/plans/tuve-que-cancelar-sesiones-rippling-cray.md`. |
| B2 | `AnthropicEmbeddingsProvider` real | v0.13.0 envió un stub speculative (`AnthropicProvider` con endpoint placeholder + warning). Cuando Anthropic publique el endpoint real basta con cambiar 2 constants y 1 path. |
| B3 | sqlite-vec migration | v0.13.0 introdujo `SqliteEmbeddingsIndex` opt-in (rusqlite + bundled SQLite, sin C-extension `sqlite-vec`). Cosine similarity es pure-Rust por ahora; al cruzar ~5k pages la migración a `sqlite-vec` será una reemplazo de UDF, no del schema. Diferido en [ADR 0006](adr/0006-local-semantic-search-storage.md). |
| B4 | ~~Concurrencia WikiLog/Index~~ | **Cerrado en v0.14 + v0.15**: v0.14 envió `atomic_write_string` (torn-write safety) + `WikiLog::append_atomic` (POSIX O_APPEND single-entry). v0.15 envió `with_exclusive_lock` (cross-process flock) y wireó `coral ingest`/`bootstrap` por dentro. Concurrency tests upgraded para asertar entries == N y errors == 0. Stress 50× threads green. |

---

## Releases shipped

### v0.1.0 — initial release

Cargo workspace con 5 crates, 10 subcomandos, embedded skill bundle, 3
composite GH actions, 150 tests + 3 ignored.

### v0.2.0 + v0.2.1 — patch series

`bootstrap`/`ingest --apply`, walk skips system files, CHANGELOG +
cargo-release wiring, streaming `coral query`, `coral search` (TF-IDF),
Hermes quality gate, local prompt overrides, GeminiRunner stub, Notion
sync, `coral sync --remote`, per-file pinning, fine-tune dataset,
perf docs, stats coverage, `coral notion-push`, `ClaudeRunner` streaming
timeout.

### v0.3.x — embeddings + cache + dogfooding fixes

- v0.3.0: mtime-cached frontmatter parsing + LLM-driven Q/A pairs.
- v0.3.1: embeddings-backed search via Voyage AI (`coral search --engine embeddings`).
- v0.3.2: 3 dogfooding fixes — UTF-8 search panic, runner auth UX, CWD_LOCK race.

### v0.4.0 — multi-provider runners

| # | Item | Estado |
|---|---|---|
| 1 | `EmbeddingsProvider` trait + Voyage as one impl | ✅ |
| 2 | `OpenAIProvider` second real impl | ✅ |
| 3 | Real `GeminiRunner` (not a stub) | ✅ |
| 4 | Auth setup section in README | ✅ |
| 5 | Telemetry on `coral query` | ✅ |
| 6 | `coral notion-push --apply` semantics | ✅ |
| 7 | `coral query` token streaming polish | ✅ (via runner unification in v0.5) |

### v0.5.0 — apply-flow + streaming + docs

| # | Item | Estado |
|---|---|---|
| 1 | `coral validate-pin` | ✅ |
| 2 | `coral lint --staged` (pre-commit hook) | ✅ |
| 3 | `embeddings-cache` composite GH action | ✅ |
| 4 | `coral diff <slugA> <slugB>` (structural) | ✅ |
| 5 | `coral export --format html` (single-file static site) | ✅ |
| 6 | `LocalRunner` (llama.cpp) | ✅ |
| 7 | `coral lint --auto-fix [--apply]` | ✅ |
| 8 | `coral consolidate --apply` (retire path) | ✅ |
| 9 | `coral onboard --apply` | ✅ |
| 10 | Streaming runner unification (Gemini + Local) | ✅ |
| 11 | USAGE.md fully refreshed | ✅ |

### v0.6.0 — quality + apply-flow extension + CI hardening

| # | Item | Estado |
|---|---|---|
| 1 | 4 new structural lint checks (`CommitNotInGit`, `SourceNotFound`, `ArchivedPageLinked`, `UnknownExtraField`) | ✅ |
| 2 | `coral diff --semantic` (LLM-driven contradictions) | ✅ |
| 3 | `coral consolidate --apply` extended to merges + splits | ✅ |
| 4 | `criterion` benchmarks for 5 hot paths | ✅ |
| 5 | `cargo-audit` + `cargo-deny` CI jobs | ✅ |
| 6 | ADR 0008 (multi-provider runner+embeddings traits) | ✅ |
| 7 | ADR 0009 (auto-fix scope + YAML plan shape) | ✅ |
| 8 | `SCHEMA.base.md` aligned with 10 PageType variants | ✅ |
| 9 | Parallelized embeddings batching across rayon | ✅ |
| 10 | `KNOWN_PROMPTS` registers `qa-pairs` / `lint-auto-fix` / `diff-semantic` + ships their templates | ✅ |

### v0.7.0 — BM25 + rewrite-links + prompt registry polish

| # | Item | Estado |
|---|---|---|
| 1 | `coral search --algorithm bm25` (Okapi BM25 alternative to TF-IDF) | ✅ |
| 2 | `coral consolidate --apply --rewrite-links` (mass-patch outbound wikilinks) | ✅ |
| 3 | Embedded prompt templates for `diff-semantic` + `lint-auto-fix` | ✅ |

### v0.8.0 — lint --severity + JSON schema + coverage CI

| # | Item | Estado |
|---|---|---|
| 1 | `coral lint --severity <critical\|warning\|info\|all>` filter | ✅ |
| 2 | `docs/schemas/lint.schema.json` (drift-guard tested) | ✅ |
| 3 | Coverage CI job (`cargo-llvm-cov`, lcov artifact) | ✅ |

### v0.8.1 — test infrastructure + executable tutorial

| # | Item | Estado |
|---|---|---|
| 1 | `docs/TUTORIAL.md` — every output captured from the real binary | ✅ |
| 2 | proptest harnesses for lint / search / wikilinks / frontmatter (31 props) | ✅ |
| 3 | insta snapshot tests for 11 deterministic CLI outputs | ✅ |

### v0.9.0 — stats extension

| # | Item | Estado |
|---|---|---|
| 1 | `pages_without_sources_count` + `oldest_commit_age_pages` + `pages_by_confidence_bucket` on `StatsReport` | ✅ |
| 2 | 3 more snapshot tests (validate-pin / lint --severity variants) | ✅ |

### v0.10.0 — lint --rule filter + error path tests

| # | Item | Estado |
|---|---|---|
| 1 | `coral lint --rule <CODE>` per-LintCode allowlist (composes with --severity) | ✅ |
| 2 | RunnerError + EmbeddingsError Display assertions + non-streaming timeout | ✅ |

### v0.11.0 — HttpRunner

| # | Item | Estado |
|---|---|---|
| 1 | `HttpRunner` — 5th `Runner` impl, OpenAI-compatible chat-completions endpoint (vLLM, Ollama, OpenAI, etc.) | ✅ |
| 2 | `--provider http` flag (env vars `CORAL_HTTP_ENDPOINT` + `CORAL_HTTP_API_KEY`) | ✅ |

### v0.12.0 — six-feature batch

| # | Item | Estado |
|---|---|---|
| 1 | `coral export html --multi` (per-page HTML + extracted CSS for static hosting) | ✅ |
| 2 | `coral status --watch` (live dashboard refresh, atomic frame switch, configurable interval) | ✅ |
| 3 | `coral lint --fix` dedup + EOL normalization (CRLF→LF, trailing whitespace) | ✅ |
| 4 | Per-rule auto-fix prompt routing (`broken-wikilink`, `low-confidence` → dedicated prompts) | ✅ |
| 5 | Cross-runner contract test suite (5 tests × every Runner impl) | ✅ |
| 6 | Concurrency test suite (7 tests; load+modify+save race documented as v0.14 design item) | ✅ |

### v0.13.0 — orchestra-ingest example + storage backend

| # | Item | Estado |
|---|---|---|
| 1 | `examples/orchestra-ingest/` reference repo (placeholder microservice + .wiki seed + 3 GH workflow jobs pinned to v0.12.0) | ✅ |
| 2 | `SqliteEmbeddingsIndex` opt-in backend (`CORAL_EMBEDDINGS_BACKEND=sqlite`, rusqlite + bundled SQLite, pure-Rust cosine) | ✅ |
| 3 | `AnthropicProvider` speculative stub (placeholder endpoint, dim 1024, ready when Anthropic ships embeddings API) | ✅ |
| 4 | `coral lint --suggest-sources` LLM-driven source proposal pass | ✅ |
| 5 | 7 `#[ignore]` stress tests against synthetic 200-page wiki | ✅ |
| 6 | docs/USAGE.md refresh (--fix, --rule, --suggest-sources, --watch, --multi, sqlite backend) | ✅ |
| 7 | Flaky `chunked_parallel_actually_uses_multiple_threads` stabilized | ✅ |

### v0.14.0 — atomic file writes (torn-write safety)

| # | Item | Estado |
|---|---|---|
| 1 | `WikiLog::append_atomic(path, op, summary)` — POSIX `O_APPEND` race-free single-entry append. Coral CLI commands switched. | ✅ |
| 2 | `coral_core::atomic::atomic_write_string(path, content)` — temp-file + rename for torn-write safety. Wired into `Page::write`, `WikiLog::save`, `EmbeddingsIndex::save`, all CLI `.wiki/` writers. | ✅ |
| 3 | 50-writer × 50-reader stress test pinning that no reader ever observes a torn write. | ✅ |

### v0.14.1 — confidence-from-coverage rule

| # | Item | Estado |
|---|---|---|
| 1 | `coral lint --fix` `confidence-from-coverage` rule (no-LLM): downgrades confidence by 0.20 (floored at 0.30) when sources don't resolve on disk. Idempotent at floor. | ✅ |
| 2 | Concurrency-model section in docs/USAGE.md. | ✅ |

### v0.15.0 — cross-process file locking (lost-update safety)

| # | Item | Estado |
|---|---|---|
| 1 | `coral_core::atomic::with_exclusive_lock(path, closure)` — `flock(2)` advisory exclusive lock on `<path>.lock`. fs4 dep added. MSRV stays at 1.85 via UFCS. | ✅ |
| 2 | `coral ingest` and `coral bootstrap` index writes wrapped in the lock. Closes the lost-update race documented in v0.13's concurrency.rs. | ✅ |
| 3 | Stress: 50 threads × increment-shared-counter, 100% land. | ✅ |

---

## v0.16+ — speculative

Items fuera del current scope. Sin commitment hasta que alguien pida
explícitamente, o hasta que un consumer real demuestre la necesidad.

- **`RunnerError` UX bug**: las variantes `NotFound` / `AuthFailed` / `NonZeroExit` / `Timeout` / `Io` hardcodean "claude" en sus mensajes. Cuando el usuario corre `coral query --provider local` y el binario falta, se imprime "claude binary not found" en vez del binario que efectivamente faltó. Fix: parameterizar el binary name por variante (cada Runner attaches su propio nombre antes de propagar). v0.16 candidate.
- **sqlite-vec C-extension migration**: hoy `SqliteEmbeddingsIndex`
  hace cosine en pure-Rust. Al cruzar ~5k pages la query empieza a
  doler; cambiar UDF a sqlite-vec mantiene el schema.
- **`coral init --template <X>`**: starter wikis tailored to project
  type (Rust microservice, React app, ML pipeline, etc.).
- **Coverage badge en README**: el job `coverage` en `ci.yml` ya corre
  `cargo-llvm-cov` y sube `lcov.info` como artifact. Falta sólo
  publicar a Codecov + agregar badge.
- **Real-API smoke test orchestration en CI**: secrets management para
  `VOYAGE_API_KEY`, `OPENAI_API_KEY`, `LLAMA_MODEL`,
  `CLAUDE_CODE_OAUTH_TOKEN`, así los 15 `--ignored` tests corren en
  schedule periódico.
- **AnthropicProvider real**: cambiar el placeholder a endpoint real
  cuando Anthropic publique el embeddings API.
- **Cross-process integration test for `with_exclusive_lock`**: hoy
  el stress test es 50 threads en UN proceso. flock(2) funciona igual
  cross-process pero un test que spawn N coral subprocesses (vía un
  hidden test-only subcommand o helper binary) probaría el contrato
  end-to-end al límite del proceso.

---

## Cómo trabajar este backlog

1. **Una sesión = un item.** PRs chicos = revisión rápida.
2. **Cada item arranca con dogfooding** — antes de tocar código, intentar el flujo end-to-end con la herramienta actual y anotar dónde se rompe.
3. **Cada item termina con changelog entry + tests que demuestren el gate.**
4. **Items v0.7+ se promueven a un release específico sólo cuando alguien (consumer o maintainer) los pide explícitamente**.

---

## Admin pendiente (no son features)

- [x] Verificar que `release.yml` produjo binarios para todas las releases — confirmado v0.3.2 → v0.15.0 (3 tarballs + 3 SHA256 cada una).
- [x] `examples/orchestra-ingest/` shipped en v0.13.0 (cierra issue #12 con ejemplo dentro del repo en vez de repo separado).
- [x] Confidence-from-coverage rule (v0.7+ speculative item) shipped en v0.14.1.
- [x] Concurrencia file-locking (v0.14 design item) shipped en v0.14 + v0.15.
- [ ] Correr los 16 tests `--ignored` (smokes reales + stress + sync) al menos una vez por release; idealmente parte de `release.yml`. Necesita secrets management para `VOYAGE_API_KEY` / `OPENAI_API_KEY` / `LLAMA_MODEL` / `CLAUDE_CODE_OAUTH_TOKEN`.
- [ ] Self-hosted dogfooding: maintainer corre `claude setup-token` localmente + `coral ingest --apply` para traer `.wiki/` desde commit `213ac99` hasta HEAD (15 releases worth of catch-up). Plan listo en `~/.claude/plans/tuve-que-cancelar-sesiones-rippling-cray.md`.
- [ ] Publicar Codecov badge (CI ya genera `lcov.info`).
- [ ] Fix `RunnerError` UX bug — error messages mention "claude" even with `--provider local|gemini|http` (v0.16 candidate).
