# Roadmap

Estado consolidado del backlog. Cada release tiene su sección con items resueltos.

**Última actualización**: 2026-05-02 — v0.13.0 shipped. 13 releases this session (v0.3.2 → v0.13.0). 583 tests + 15 ignored. Todo lo implementable sin LLM access está en producción.

---

## Items bloqueados / fuera de alcance

| # | Item | Bloqueador real |
|---|---|---|
| B1 | Dogfooding self-hosted `.wiki/` | **Doble blocker**: (a) el maintainer tiene que correr `claude setup-token` interactivamente — el sandbox NO permite que el agente lo haga porque OAuth flows crean auth state persistente; (b) si el token se pega en chat (intentado en esta sesión), el sandbox bloquea su uso porque embeber tokens chat-leak en env vars de subprocesses es leak surface adicional. La self-hosted wiki sigue en `213ac99` (anterior a v0.1.0). Workaround: maintainer corre `claude setup-token` en su terminal local + corre `coral ingest --apply` ahí. Plan listo en `~/.claude/plans/tuve-que-cancelar-sesiones-rippling-cray.md`. |
| B2 | `AnthropicEmbeddingsProvider` real | v0.13.0 envió un stub speculative (`AnthropicProvider` con endpoint placeholder + warning). Cuando Anthropic publique el endpoint real basta con cambiar 2 constants y 1 path. |
| B3 | sqlite-vec migration | v0.13.0 introdujo `SqliteEmbeddingsIndex` opt-in (rusqlite + bundled SQLite, sin C-extension `sqlite-vec`). Cosine similarity es pure-Rust por ahora; al cruzar ~5k pages la migración a `sqlite-vec` será una reemplazo de UDF, no del schema. Diferido en [ADR 0006](adr/0006-local-semantic-search-storage.md). |
| B4 | Concurrencia WikiLog/Index | `crates/coral-core/tests/concurrency.rs` documenta una race condition: `WikiLog::append` + `WikiIndex::upsert` hacen load+modify+save sin lock. En 10 threads concurrentes sólo persisten ~2/10 entries. Marcado como design item de v0.14: hace falta file-locking (`fs2` o `fcntl`) o switch a SQLite-backed index/log. |

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

---

## v0.14+ — speculative

Items fuera del current scope. Sin commitment hasta que alguien pida
explícitamente, o hasta que un consumer real demuestre la necesidad.

- **WikiLog/Index file-locking**: documentado en
  `crates/coral-core/tests/concurrency.rs`. Bajo concurrencia (~10
  threads), `append`+`upsert` pierde entries por race load+modify+save.
  Fix: `fs2` advisory locks o switch a SQLite-backed log/index.
- **sqlite-vec C-extension migration**: hoy `SqliteEmbeddingsIndex`
  hace cosine en pure-Rust. Al cruzar ~5k pages la query empieza a
  doler; cambiar UDF a sqlite-vec mantiene el schema.
- **`coral consolidate --apply` outbound-rewrite**: hoy merge mueve
  bodies pero deja wikilinks rotos. `--rewrite-links` haría mass-patch.
- **`coral search --algorithm bm25`**: alternativa a TF-IDF; modest
  precision improvement en wikis grandes.
- **Confidence-from-coverage**: si `sources:` cita files que ya no
  existen, auto-downgrade confidence. Pure rule, no LLM.
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

---

## Cómo trabajar este backlog

1. **Una sesión = un item.** PRs chicos = revisión rápida.
2. **Cada item arranca con dogfooding** — antes de tocar código, intentar el flujo end-to-end con la herramienta actual y anotar dónde se rompe.
3. **Cada item termina con changelog entry + tests que demuestren el gate.**
4. **Items v0.7+ se promueven a un release específico sólo cuando alguien (consumer o maintainer) los pide explícitamente**.

---

## Admin pendiente (no son features)

- [x] Verificar que `release.yml` produjo binarios para todas las releases — confirmado v0.3.2 → v0.13.0 (3 tarballs + 3 SHA256 cada una).
- [x] `examples/orchestra-ingest/` shipped en v0.13.0 (cierra issue #12 con ejemplo dentro del repo en vez de repo separado).
- [ ] Correr los 15 tests `--ignored` (smokes reales + stress) al menos una vez por release; idealmente parte de `release.yml`. Necesita secrets management para `VOYAGE_API_KEY` / `OPENAI_API_KEY` / `LLAMA_MODEL` / `CLAUDE_CODE_OAUTH_TOKEN`.
- [ ] Self-hosted dogfooding: maintainer corre `claude setup-token` localmente + `coral ingest --apply` para traer `.wiki/` desde commit `213ac99` hasta HEAD (5 releases worth of catch-up). Plan listo en `~/.claude/plans/tuve-que-cancelar-sesiones-rippling-cray.md`.
- [ ] Publicar Codecov badge (CI ya genera `lcov.info`).
