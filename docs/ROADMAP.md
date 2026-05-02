# Roadmap

Estado consolidado del backlog. Cada release tiene su sección con items resueltos.

**Última actualización**: 2026-05-02 — v0.6.0 shipped. Todos los ítems
implementables sin dependencias externas están en producción. Los 4
items restantes están todos genuinamente bloqueados (ver "Items
bloqueados").

---

## Items bloqueados / fuera de alcance

| # | Item | Bloqueador real |
|---|---|---|
| B1 | Dogfooding self-hosted `.wiki/` | Necesita que el maintainer corra `claude setup-token` para que el `claude --print` subprocess autentique. La self-hosted wiki sigue en `213ac99` (anterior a v0.1.0). Es 1 acción manual; sin ella no hay ingest LLM-driven. |
| B2 | `AnthropicEmbeddingsProvider` | Anthropic no publicó embeddings API al momento de este commit. Cuando lo haga, agregar es ~80 LOC en `coral-runner::embeddings` (mismo molde que `OpenAIProvider`). Hoy `OpenAIProvider` cubre el caso "no Voyage". |
| B3 | sqlite-vec migration | Diferido en [ADR 0006](adr/0006-local-semantic-search-storage.md) hasta que una wiki cruce ~5k pages y la latencia del JSON in-memory empiece a doler. Premature shipearlo ahora. |
| B4 | `orchestra-ingest` reference repo | Repo separado, fuera del scope de este repo. Issue #12 cerrado pero el follow-up nunca arrancó. Crear cuando alguien pida una demo end-to-end de Coral en una microservice "real". |

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

---

## v0.7+ — speculative

Lista para no perder ideas. Todos sin commitment hasta que alguien pida
explícitamente, o hasta que un consumer real demuestre la necesidad.

- **`coral lint --fix` (no-LLM)**: pure-rule auto-fix for things that
  don't need judgment — trim whitespace, normalize wikilink syntax,
  fix YAML key ordering. Could ship as `coral fmt`.
- **`coral consolidate --apply` outbound-rewrite**: today merge moves
  bodies but leaves wikilinks pointing at retired sources broken
  (relies on lint to surface). A `--rewrite-links` flag would mass-
  patch every page that linked to a source.
- **`coral search --algorithm bm25`**: BM25 alternative to TF-IDF;
  modest precision improvement on 100+ page wikis.
- **Per-rule lint policies**: today every issue feeds the same
  `lint-auto-fix` prompt. Route `BrokenWikilink` to a wikilink-specific
  prompt that has access to the full slug list, etc.
- **Source-suggestion pass**: a separate LLM call that proposes
  `sources:` paths from `git ls-files` output (higher-risk than the
  current capped auto-fix scope; needs its own prompt + tests).
- **Confidence-from-coverage**: if `sources:` cite files that no
  longer exist, auto-downgrade confidence by a fixed step. Pure rule,
  no LLM.
- **`coral init --template <X>`**: starter wikis tailored to project
  type (Rust microservice, React app, ML pipeline, etc.).
- **Coverage in CI** (`cargo-llvm-cov` + Codecov badge).
- **`coral lint --json` schema** versioned at `docs/schemas/lint.schema.json`
  (mirrors what stats already does).
- **Real-API smoke test orchestration in CI**: secrets management for
  `VOYAGE_API_KEY`, `OPENAI_API_KEY`, `LLAMA_MODEL`, `CLAUDE_CODE_OAUTH_TOKEN`
  so the 8 `--ignored` tests run on a periodic schedule.
- **HTTP-based runners** (vLLM, Ollama HTTP, OpenAI Responses): the
  Runner trait shape supports them; just no impl yet.

---

## Cómo trabajar este backlog

1. **Una sesión = un item.** PRs chicos = revisión rápida.
2. **Cada item arranca con dogfooding** — antes de tocar código, intentar el flujo end-to-end con la herramienta actual y anotar dónde se rompe.
3. **Cada item termina con changelog entry + tests que demuestren el gate.**
4. **Items v0.7+ se promueven a un release específico sólo cuando alguien (consumer o maintainer) los pide explícitamente**.

---

## Admin pendiente (no son features)

- [x] Verificar que `release.yml` produjo binarios para todas las releases — confirmado en v0.3.2 verification (3 tarballs + 3 SHA256 por release).
- [ ] Correr los 8 tests `--ignored` (smokes reales) al menos una vez por release; idealmente parte de `release.yml`. Necesita secrets management.
- [ ] Reabrir GH issue #12 (orchestra-ingest) o crear v0.7+ milestone si volvemos a priorizarlo.
