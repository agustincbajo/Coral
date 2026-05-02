# Roadmap

Un solo lugar para ver qué viene. Cada item tiene **prioridad**, **tamaño**, y un **estado** (✅ shipped, ⏸️ blocked, pendiente). Items sin gate no se considerarán "done".

**Última actualización**: 2026-05-01 — v0.4 P0 #1 + #4 entregados, P1 #6 + #8 entregados. Ver CHANGELOG `[Unreleased]`.

## v0.4.0 — multi-provider runners

Honra lo prometido en docs (multi-provider) y ataca la deuda real del stub de Gemini + el hard-wire a Voyage.

### P0 — bloqueantes para release

| # | Título | Tamaño | Estado |
|---|---|---|---|
| 1 | **`EmbeddingsProvider` trait** + reorganización de `voyage` como una impl | M | ✅ shipped en commit `998fbfb` |
| 2 | **`AnthropicEmbeddingsProvider`** (cuando Anthropic publique la API) o **`OpenAIEmbeddingsProvider`** como segunda impl real | M | ⏸️ bloqueado en credenciales / API real — pendiente |
| 3 | **`GeminiRunner` real** (no stub) — usar el `gemini` CLI real con flags propios o caer a Vertex AI API | L | ⏸️ bloqueado en `gemini` CLI instalado para smoke real — pendiente |
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

Lista para no perder ideas que no entran ahora.

- **Local llama.cpp runner** — `LocalRunner` que use un binario `llama-cli`. Útil para offline / ahorrar costos en lint nocturno.
- **`coral diff <pageA> <pageB>`** — mostrar contradicciones entre 2 versiones de la misma página o entre 2 wikis sincronizados.
- **`coral lint --auto-fix`** — el LLM bumpea `confidence`, mueve a `_archive/`, completa `sources` automáticamente para issues sencillos.
- **Hooks pre-commit** — `coral lint --structural --staged` corre solo sobre los `.wiki/**/*.md` cambiados.
- **`coral export --format html`** — sitio estático navegable (mdbook backend?) para hospedar la wiki como docs públicas.
- **Embeddings caching en CI** — la GH action de ingest reusa cache entre runs vía `actions/cache`.
- **`coral validate-pin`** — verifica que las versiones pinneadas en `.coral-pins.toml` existen como tags.

## Cómo trabajar este backlog

1. **Una sesión = un item P0 a la vez.** No mezclar items para mantener PRs digeribles.
2. **Cada item arranca con dogfooding** — antes de tocar código, intentar el flujo end-to-end del item con la herramienta actual y anotar dónde se rompe. Eso valida o ajusta el gate.
3. **Cada item termina con changelog entry + test que demuestre el gate**.
4. **Items P2 se promueven a P1 sólo cuando alguien (vos o un consumer) los pide explícitamente** — no anticipar demanda.

## Pendientes administrativos (no son features)

- [ ] Reabrir GH issue #12 (orchestra-ingest) o crear v0.4 milestone con los P0/P1 de arriba.
- [ ] Verificar que `release.yml` produjo binarios para v0.3.0/v0.3.1/v0.3.2 (ningún checkpoint manual hasta ahora).
- [ ] Correr los 4 tests `--ignored` (smokes contra `claude`/`git`/`gemini` reales) al menos una vez por release; idealmente parte de `release.yml`.
