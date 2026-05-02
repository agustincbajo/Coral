# Roadmap

Un solo lugar para ver qué viene. Cada item tiene **prioridad**, **tamaño**, y un **estado** (✅ shipped, ⏸️ blocked, pendiente). Items sin gate no se considerarán "done".

**Última actualización**: 2026-05-01 — todos los P0 entregados (#1, #2, #3, #4); P1 #6, #7, #8 entregados; #5 (dogfooding) bloqueado en `claude setup-token`. v0.4 listo para release. Ver CHANGELOG `[Unreleased]`.

## v0.4.0 — multi-provider runners

Honra lo prometido en docs (multi-provider) y ataca la deuda real del stub de Gemini + el hard-wire a Voyage.

### P0 — bloqueantes para release

| # | Título | Tamaño | Estado |
|---|---|---|---|
| 1 | **`EmbeddingsProvider` trait** + reorganización de `voyage` como una impl | M | ✅ shipped en commit `998fbfb` |
| 2 | **`OpenAIProvider`** como segunda impl real (Anthropic embeddings se difiere a v0.5 hasta que Anthropic publique la API) | M | ✅ shipped — `coral search --embeddings-provider openai`, smoke real `#[ignore]` |
| 3 | **`GeminiRunner` real** (no stub) — argv propio (`-p`, `-m`, system prepended) en vez de wrappear ClaudeRunner | L | ✅ shipped — 7 tests sobre la argv real, smoke real `#[ignore]` |
| 4 | **Documentación de auth para CI / Claude Code** | S | ✅ shipped en README "Auth setup" section |

### P1 — quality of life

| # | Título | Tamaño | Estado |
|---|---|---|---|
| 5 | **Dogfooding effectivo de `.wiki/`** — bring `.wiki/` desde 213ac99 → HEAD usando `coral bootstrap --apply` o `coral ingest --apply` por chunks de release | M | ⏸️ bloqueado en auth de `claude` subprocess — necesita `claude setup-token` |
| 6 | ~~**`PageType::Reference`**~~ — ya existe en [crates/coral-core/src/frontmatter.rs:25](crates/coral-core/src/frontmatter.rs:25) | — | ✅ ya estaba shipped |
| 7 | **Telemetría básica de `coral query`** — duración, tokens, modelo, snippet del top hit usado | S | ✅ shipped (duración, chunks, output_chars, model, pages_in_context) — visible con `RUST_LOG=coral=info` |
| 8 | **`coral notion-push --dry-run` por defecto** + `--apply` explícito (consistencia con `bootstrap`/`ingest`) | XS | ✅ shipped en commit `cd9b1f8` |

### P2 — opportunistic

| # | Título | Tamaño | Gate |
|---|---|---|---|
| 9 | **sqlite-vec migration** (sigue diferida desde v0.3.1, ADR 0006) | L | Solo si una wiki cruza ~5k pages y `coral search --engine embeddings` se vuelve > 200ms cold; ADR 0006 update |
| 10 | **`orchestra-ingest` reference repo** (issue #12 cerrado pero el repo no existe) | L | Repo separado en GH org; recibe `ingest` action en cada push; demuestra Coral en un microservicio "real" |

## v0.5+ — ideas tentativas (sin commitment)

Lista para no perder ideas que no entran ahora. **Items entregados anticipadamente** se marcan ✅ y caen del backlog.

- **Local llama.cpp runner** — `LocalRunner` que use un binario `llama-cli`. Útil para offline / ahorrar costos en lint nocturno.
- **`coral diff <pageA> <pageB>`** — mostrar contradicciones entre 2 versiones de la misma página o entre 2 wikis sincronizados.
- **`coral lint --auto-fix`** — el LLM bumpea `confidence`, mueve a `_archive/`, completa `sources` automáticamente para issues sencillos.
- ✅ **Hooks pre-commit** — `coral lint --staged` corre lint completo pero filtra issues a los `.wiki/**/*.md` staged. Shipped en `[Unreleased]`.
- **`coral export --format html`** — sitio estático navegable (mdbook backend?) para hospedar la wiki como docs públicas.
- ✅ **Embeddings caching en CI** — composite action `embeddings-cache` con `actions/cache@v4`, key branch-scoped. Shipped en `[Unreleased]`.
- ✅ **`coral validate-pin`** — verifica via `git ls-remote --tags` que las versiones pinneadas existan. Shipped en `[Unreleased]`.

## Cómo trabajar este backlog

1. **Una sesión = un item P0 a la vez.** No mezclar items para mantener PRs digeribles.
2. **Cada item arranca con dogfooding** — antes de tocar código, intentar el flujo end-to-end del item con la herramienta actual y anotar dónde se rompe. Eso valida o ajusta el gate.
3. **Cada item termina con changelog entry + test que demuestre el gate**.
4. **Items P2 se promueven a P1 sólo cuando alguien (vos o un consumer) los pide explícitamente** — no anticipar demanda.

## Pendientes administrativos (no son features)

- [ ] Reabrir GH issue #12 (orchestra-ingest) o crear v0.4 milestone con los P0/P1 de arriba.
- [ ] Verificar que `release.yml` produjo binarios para v0.3.0/v0.3.1/v0.3.2 (ningún checkpoint manual hasta ahora).
- [ ] Correr los 4 tests `--ignored` (smokes contra `claude`/`git`/`gemini` reales) al menos una vez por release; idealmente parte de `release.yml`.
