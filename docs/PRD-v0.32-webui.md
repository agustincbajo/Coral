# PRD — Coral v0.32: WebUI estilo LightRAG (`coral ui serve`)

**Versión del documento:** 1.2 (borrador post-2da-review)
**Fecha:** 2026-05-11
**Autor:** Agustín Bajo
**Estado:** Borrador — incorpora fixes del review independiente (versiones deps marcadas como snapshot, cuenta de tamaño rehecha, spike SSE como gate de semana 1, default-on justificado, timeline M1 ampliado a 8–10 semanas, auth bearer adelantada a M1)
**Versiones objetivo:** Coral v0.32.0 (M1) → v0.33.0 (M2) → v0.34.0 (M3)
**Predecesor:** [PRD-v0.24-evolution.md](PRD-v0.24-evolution.md) §2A.1 (fila *WebUI / visualization* — "opt-in feature flag, preserva single-binary base") — este PRD revisa parcialmente esa decisión: el feature *existe* y es opt-out a nivel `cargo install` (justificación en §11.1), pero **a nivel ejecución** sigue siendo opt-in (sólo se levanta con `coral ui serve`; nunca corre implícito).

---

## 1. Resumen ejecutivo

Coral v0.31.1 expone su superficie por tres canales: **CLI** (42 subcomandos), **MCP server** (8 recursos + 10 tools + 3 prompts) y un **servidor HTTP local muy básico** (`coral wiki serve`, [crates/coral-cli/src/commands/serve.rs](../crates/coral-cli/src/commands/serve.rs), `tiny_http` bloqueante, HTML preformateado en `GET /page/<slug>`, grafo Mermaid en `GET /graph`). Funcional pero estéticamente fósil: no hay búsqueda interactiva, ni filtros por `PageType`/`Status`/`confidence`, ni un grafo navegable, ni un playground de `coral query`.

[LightRAG](https://github.com/HKUDS/LightRAG) resuelve este problema con un **SPA React 19 + Vite + Tailwind + shadcn**, estado en Zustand, grafo en **Sigma.js + Graphology**, servida por FastAPI vía un `SmartStaticFiles` que inyecta config runtime y montada en `/webui`. El **bundle compilado va commiteado al repo** (`lightrag/api/webui/`) para que el end-user no necesite Node ni Bun.

Este PRD adapta ese patrón a Coral preservando las invariantes del proyecto: **un solo binario Rust, sin runtime de Node en el host de producción, distribución por `cargo install` o plugin de Claude Code, backward-compat sagrada**.

**Wedge de la WebUI** (la razón única por la que un dev abriría `http://localhost:3838` en vez de quedarse en el CLI): **explorar visualmente el grafo de wikilinks de un wiki multi-repo y disparar `coral query` desde un playground, con feedback de `status`/`confidence`/`valid_from/valid_to` codificado en color**. Ningún competidor (LightRAG incluido) muestra bi-temporalidad ni status workflow sobre un grafo de conocimiento Git-native.

**Tres principios no negociables:**

1. **Single-binary preserved**: el bundle JS+CSS+HTML se *embebe* en el binario `coral` vía `include_dir!` (mismo patrón que el `template/` actual, ADR-0003). Sin assets externos. Sin Node en producción.
2. **Opt-in, no default**: se invoca con `coral ui serve`, separado de `coral wiki serve` (que se mantiene como fallback minimalista y como API de lectura pública). El comando `wiki serve` no se rompe.
3. **MCP + REST = misma superficie semántica**: cada endpoint REST que crea este PRD se construye sobre los mismos `coral-core` builders que ya alimentan MCP. Cero duplicación de lógica.

---

## 2. Contexto y problemas

### 2.1 Estado actual de la UI de Coral

| # | Síntoma | Evidencia |
|---|---|---|
| U1 | `coral wiki serve` renderiza Markdown como `<pre>` preformateado, sin parser MD a HTML. | [crates/coral-cli/src/commands/serve.rs](../crates/coral-cli/src/commands/serve.rs) |
| U2 | El "grafo" es Mermaid embebido en `<pre class="mermaid">`. No es interactivo (no se pueden filtrar nodos, no hay zoom semántico, no hay layouts force-directed). | `GET /graph` en el mismo archivo |
| U3 | Sin búsqueda. Sin filtros por `PageType`, `Status`, `confidence ∈ [0,1]`, ni por `valid_from`/`valid_to`. | Gap — el modelo `Frontmatter` ya soporta todo esto en [crates/coral-core/src/page.rs](../crates/coral-core/src/page.rs) pero la UI no lo expone. |
| U4 | Sin playground para `coral query`. El usuario debe abrir un terminal aparte para consultar el wiki via LLM. | Gap |
| U5 | Sin visualizador del manifiesto multi-repo (`coral.toml` / `coral.lock`). Sin vista de pipeline. | Gap |
| U6 | Sin vista de drift de interfaces (FR-IFACE del PRD anterior), aunque MCP ya tiene `contract_status` y `affected_repos`. | Gap |
| U7 | Sin auth / sin distinción read-only vs write-tools, aunque MCP ya tiene `--allow-write-tools`. | Gap |
| U8 | Sin i18n. El público objetivo de Coral incluye dev hispanohablantes y enterprise EU. | Gap |

### 2.2 ¿Por qué ahora?

- **v0.31.1 cerró el ciclo 5 de auditoría**: el core Rust está estable y BC-garantizado. Es el momento estructural correcto para añadir una capa de presentación sin riesgo de regresión.
- **El ecosistema visualmente cercano (LightRAG, GraphRAG, Zep) acelera el listón**: cuando un dev compara herramientas, las que carecen de UI moderna son percibidas como "menos serias", incluso si funcionalmente son superiores.
- **El modelo de datos de Coral es más rico que el de LightRAG y nadie lo ve**: `Status` cíclico (Draft→Reviewed→Verified→Stale→Archived), `confidence`, bi-temporal (`valid_from`/`valid_to`), `superseded_by`, `PageType` (15 variantes). Una UI bien diseñada convierte estas propiedades en una ventaja de marketing inmediata.

---

## 3. Posicionamiento vs LightRAG

Análisis ortogonal — qué adoptar, qué cambiar, qué intencionalmente NO copiar.

| Dimensión | LightRAG | Coral v0.32 objetivo | Decisión |
|---|---|---|---|
| Framework | React 19 + Vite + TS | **React 19 + Vite + TS** | Igualar — ecosistema más amplio y tipado fuerte |
| UI primitives | Radix + Tailwind + shadcn | **Radix + Tailwind + shadcn** | Igualar — la combinación es estándar de facto |
| Estado | Zustand | **Zustand** | Igualar — minimalista, sin Redux/RTK |
| Routing | React Router v7 | **React Router v7** | Igualar |
| HTTP | Axios | **fetch + TanStack Query** | **Cambiar** — TanStack Query da cache + retries + invalidation sin reinventar; bundle más pequeño que Axios |
| i18n | i18next | **i18next con bundles `en` + `es`** | Igualar + 1 idioma extra (el público objetivo de Coral) |
| Grafo | Sigma.js + Graphology + react-sigma | **Sigma.js + Graphology + react-sigma** | Igualar — es el stack más maduro para WebGL graph rendering |
| Markdown | react-markdown + KaTeX + Mermaid | **react-markdown + Mermaid** (sin KaTeX en M1) | Reducir — Coral no escribe mucho LaTeX; añadir KaTeX si surge demanda |
| Servidor estático | FastAPI `SmartStaticFiles` con inyección de config en runtime | **Rust + `tiny_http` con inyección equivalente** vía placeholder `<!-- __CORAL_RUNTIME_CONFIG__ -->` en `index.html` | Adaptar — mismo patrón, distinto runtime |
| Distribución del bundle | Pre-buildeado y commiteado a `lightrag/api/webui/` | **Embebido en el binario `coral` vía `include_dir!`** en `crates/coral-ui/assets/dist/` | **Cambiar** — el binario sigue siendo único, no hay carpeta `webui/` externa que sincronizar |
| Build del bundle | `bun run build` manual | **CI builda + commitea** el bundle a `crates/coral-ui/assets/dist/` antes de `cargo build --release` | Adaptar — `cargo build` no requiere Bun, sólo lee bytes embebidos |
| Auth | login + JWT en cookie | **Token bearer en header**, opcional, en local sin auth | Adaptar — Coral es local-first por defecto |
| Multimodal (PDF, imágenes, audio) | RAG-Anything | **NO** (anti-feature, ver §13) | Diferenciar |
| Bi-temporal UI | ❌ N/A | **Sí — slider de `valid_from`/`valid_to` para "ver el wiki como estaba en fecha X"** | **Diferenciar** — feature única, alimentada por `frontmatter.valid_from`/`superseded_by` ya existentes |
| Status / Confidence overlay | ❌ N/A | **Sí — color por status, opacidad por confidence en el grafo** | **Diferenciar** |

---

## 4. Objetivos y no-objetivos

### 4.1 Objetivos

- **O1** — Servir una SPA moderna en `http://localhost:3838` con 4 vistas core: *Pages*, *Graph*, *Query*, *Manifest*. (M1)
- **O2** — Mantener el binario `coral` como un solo archivo `.exe`/ELF/Mach-O sin assets externos. Tamaño objetivo: **≤ 8.5 MB stripped en M1, ≤ 10 MB en M3** (vs 6.3 MB actual; cuenta detallada en §7.6). CI enforcement gate en **8.0 MB** para mantener buffer real de ~0.5 MB ante regresiones de tree-shaking o dependencias nuevas.
- **O3** — Exponer un **REST API estable** (`/api/v1/*`) que sea estrictamente isomórfico a los recursos MCP. Cero divergencia semántica entre los dos canales.
- **O4** — Mantener `coral wiki serve` retrocompatible. El nuevo comando es `coral ui serve` (subcomando paralelo).
- **O5** — Distribuir vía `cargo install --features ui` o `cargo install` con `ui` en `default-features`. **Decisión pendiente:** ¿default-features incluye `ui`? (ver §11.1)
- **O6** — Build del frontend automatizado en CI (`.github/workflows/release.yml`). El desarrollador local que no toca el frontend no necesita Node/Bun instalado.
- **O7** — i18n con `en` (default) + `es` desde el día 1.

### 4.2 No-objetivos (anti-features explícitos, ver §13 para razonamiento)

- **N1** — No habrá modo SaaS multi-tenant. La UI es local-first o desplegable en el mismo host que `coral`.
- **N2** — No habrá editor WYSIWYG de Markdown. Coral es Git-native; el wiki se edita en el IDE del usuario.
- **N3** — No habrá ingesta de PDFs/imágenes/audio en M1–M3 (LightRAG sí lo tiene via RAG-Anything; Coral lo cede deliberadamente — ver PRD-v0.24 §13).
- **N4** — No habrá panel de administración de usuarios/roles en M1. Auth básica con un solo token bearer; RBAC se posterga a v0.35+.
- **N5** — No habrá SSR ni Next.js. SPA pura embebida.
- **N6** — No habrá WebSocket en M1. Polling + invalidation de TanStack Query es suficiente para el caudal de eventos esperado. SSE opcional en M3 si el caso de uso de "wiki rebuilt, refetch" lo justifica.

---

## 5. Personas y casos de uso

| Persona | Rol | Caso de uso primario |
|---|---|---|
| **Onboarding dev** | Nuevo en el equipo, día 1 | Abre `coral ui serve`, navega el grafo del wiki para entender la arquitectura sin leer 30 archivos Markdown |
| **Architect** | Senior, multi-repo lead | Usa el slider bi-temporal para ver cómo evolucionó la sección de "Auth" entre Q1 y Q3 |
| **Tech writer** | Mantiene el wiki | Filtra páginas por `status = stale` + `confidence < 0.5` para priorizar revisión |
| **PM / Non-dev** | Stakeholder ocasional | Lanza un `coral query` desde el playground sin abrir terminal: "¿cómo procesa pagos el servicio X?" |
| **Reviewer** | Code review de PR cross-repo | Abre la vista de `affected_repos --since main` para ver qué wikis tocó el PR |

---

## 6. Requisitos funcionales

Codificación: `FR-UI-<n>`. Cada FR mapea a un endpoint REST + una vista o componente.

### 6.1 Vista *Pages* (M1)

- **FR-UI-1** — Listado de páginas paginado (50/página), columnas: `slug`, `page_type`, `status`, `confidence`, `generated_at`, `repo`. Ordenable por cada columna.
- **FR-UI-2** — Filtros laterales: `page_type` (multiselect, 15 valores), `status` (multiselect, 6 valores), `confidence range` (slider doble [0.0, 1.0]), `repo` (multiselect, derivado de `coral.toml`), `has backlinks` (toggle), `valid at` (date picker para queries bi-temporales).
- **FR-UI-3** — Búsqueda full-text con debounce 200 ms, vía el backend BM25 existente (`coral-core::search`). El input dispara `GET /api/v1/search?q=...&limit=20` que envuelve `tool_search`.
- **FR-UI-4** — Vista detalle de página: Markdown renderizado con `react-markdown` (no `<pre>`), frontmatter en panel lateral, lista de backlinks clickeables, badge de status con color (Draft=gris, Reviewed=azul, Verified=verde, Stale=amarillo, Archived=rojo claro, Reference=violeta).

### 6.2 Vista *Graph* (M1)

- **FR-UI-5** — Render del grafo de wikilinks con **Sigma.js + Graphology**. Nodos = páginas; aristas = wikilinks dirigidos.
- **FR-UI-6** — Color de nodo por `status`; tamaño por número de backlinks; opacidad por `confidence`.
- **FR-UI-7** — Layouts seleccionables: **ForceAtlas2** (default), **circular**, **circlepack**, **noverlap**. Mismo set que LightRAG.
- **FR-UI-8** — Click en nodo → panel lateral con preview del Markdown + botón "abrir página completa". Doble-click → focus + 1-hop neighborhood.
- **FR-UI-9** — Slider bi-temporal `valid at`: oculta nodos cuyo `valid_from > t` o `valid_to < t`. **Feature diferenciadora vs LightRAG.**
- **FR-UI-10** — Botón "export PNG/SVG" — Sigma soporta esto nativo, integración trivial.

### 6.3 Vista *Query* (M1)

- **FR-UI-11** — Playground tipo chat para `coral query`. Input multi-línea + botón Send + selector de `mode` (`local`, `global`, `hybrid` — mapeados a los modes de `coral-core::query`). **Streaming nativo soportado**: el trait `Runner::run_streaming` ya existe en [crates/coral-runner/src/runner.rs:247](../crates/coral-runner/src/runner.rs) y `ClaudeRunner` lo implementa línea por línea sobre `stdout`. El handler REST envuelve ese callback en eventos SSE. Si el spike SSE de semana 1 (ver §15) falla, el fallback es respuesta completa POST con `Transfer-Encoding: chunked` + polling, sin perder funcionalidad.
- **FR-UI-12** — Cada respuesta muestra fuentes (slugs citados) clickeables que abren la vista *Pages* en split.
- **FR-UI-13** — Historial de queries en la sesión (no persistido en disco en M1).
- **FR-UI-14** — Warning visible: "Esta query gastará tokens del proveedor LLM configurado en `coral.toml`". (Cumple el requisito del PRD-v0.24 sobre acciones costosas.)

### 6.4 Vista *Manifest* (M1)

- **FR-UI-15** — Render del `coral.toml` parseado: repos, runners configurados, manifest schema version.
- **FR-UI-16** — Render del `coral.lock`: hashes de páginas, fecha de último ingest, contadores por repo.
- **FR-UI-17** — Botón "abrir archivo en editor" usa el handler `vscode://` / `idea://` configurable.

### 6.5 Vista *Interfaces / Drift* (M2)

- **FR-UI-18** — Lista de interfaces tracked (HTTP, gRPC, AsyncAPI, SQL) — alimentada por `tool_list_interfaces`.
- **FR-UI-19** — Vista de drift por repo, con código de color rojo/amarillo/verde. Diff inline de schemas.
- **FR-UI-20** — Vista de `affected_repos --since <ref>`: dado un git ref, qué repos consumidores están afectados.

### 6.6 Vista *Guarantee / Can-I-Deploy* (M3)

- **FR-UI-21** — Página dedicada al wedge del PRD-v0.24: `coral test guarantee --can-i-deploy <env>` con widget de semáforo grande y panel de evidencia (contracts + tests + coverage + flake-rate).

### 6.7 Auth, bind y permisos

- **FR-UI-22 (M1)** — Por defecto el servidor bindea a `127.0.0.1`. Bindear a `0.0.0.0` o a una IP no-loopback **requiere obligatoriamente** `--token <secret>` (o `CORAL_UI_TOKEN` env var). Sin token + bind no-loopback → `coral ui serve` aborta con error claro.
- **FR-UI-23 (M1)** — `POST /api/v1/query` exige token bearer desde el día 1 **incluso en bind loopback**. Razón: la query gasta tokens LLM del usuario; cualquier página web que el usuario visite en el mismo navegador podría dispararla via `fetch` a `127.0.0.1:3838`. Header `Origin` checked contra `bind` exacto + token mitiga DNS rebinding.
- **FR-UI-24 (M1)** — Read-only endpoints (`/api/v1/pages*`, `/api/v1/graph`, `/api/v1/manifest`, `/api/v1/lock`, `/api/v1/stats`, `/api/v1/search`, `/health`) accesibles sin token en bind loopback.
- **FR-UI-25 (M2)** — Endpoints sensibles (`/api/v1/tools/up`, `/api/v1/tools/down`, `/api/v1/tools/run_test`) están detrás de **dos gates simultáneos**: flag `--allow-write-tools` igual que en MCP **y** token bearer válido. Sin alguno → 403.
- **FR-UI-26 (M1)** — Path traversal: el handler de `/api/v1/pages/:repo/:slug` valida `slug` contra el regex `^[a-z0-9][a-z0-9-_/]*$` (ya usado en `coral-core` para slug allowlist) y rechaza `..`, `\`, leading `/`. El `repo` se valida contra la lista de `coral.toml`.

### 6.8 i18n (M1)

- **FR-UI-27** — Bundles `en` y `es` cargados lazy. Idioma autodetectado del header `Accept-Language` + override manual persistido en `localStorage`. Convención de keys: `<feature>.<surface>.<element>` (ej. `pages.list.empty_state`, `graph.controls.layout`). Script `bun run lint:i18n` detecta keys huérfanas en `assets/src/` y bloquea CI si fallan.

---

## 7. Arquitectura técnica

### 7.1 Nuevo crate: `coral-ui`

```
crates/coral-ui/
├── Cargo.toml                  # dependencias: tiny_http, serde_json, mime_guess, include_dir
├── build.rs                    # opcional: verifica que assets/dist/ existe
├── src/
│   ├── lib.rs                  # punto de entrada: serve(port, opts) -> Result<()>
│   ├── server.rs               # loop tiny_http + router
│   ├── routes/
│   │   ├── mod.rs              # registry
│   │   ├── pages.rs            # GET /api/v1/pages, /api/v1/pages/:slug
│   │   ├── search.rs           # GET /api/v1/search?q=
│   │   ├── graph.rs            # GET /api/v1/graph
│   │   ├── query.rs            # POST /api/v1/query (SSE stream)
│   │   ├── manifest.rs         # GET /api/v1/manifest
│   │   ├── interfaces.rs       # GET /api/v1/interfaces  (M2)
│   │   └── tools.rs            # POST /api/v1/tools/<name>  (M2+, gated por --allow-write-tools)
│   ├── auth.rs                 # bearer token middleware
│   ├── static_assets.rs        # include_dir!(assets/dist) + MIME guess + runtime config injection
│   └── error.rs                # error → JSON envelope
└── assets/
    ├── src/                    # SPA React/TS (carpeta hermana del crate, NO en src/)
    │   ├── package.json
    │   ├── vite.config.ts
    │   ├── tsconfig.json
    │   ├── index.html
    │   └── src/
    │       ├── main.tsx
    │       ├── App.tsx
    │       ├── routes/
    │       ├── features/{pages,graph,query,manifest,interfaces}/
    │       ├── components/ui/   # shadcn
    │       ├── api/             # TanStack Query hooks
    │       ├── stores/          # Zustand
    │       ├── locales/{en,es}.json
    │       └── lib/
    └── dist/                   # OUTPUT de vite build — committed a git, gitattributes linguist-generated
        ├── index.html          # con placeholder <!-- __CORAL_RUNTIME_CONFIG__ -->
        ├── assets/*.{js,css,svg,woff2}
        └── locales/*.json
```

**Decisión:** `assets/src/` y `assets/dist/` ambos en el repo. El primero para desarrollo del frontend, el segundo para embebido. `.gitattributes` marca `assets/dist/**` como `linguist-generated` para no contaminar el detector de lenguaje.

### 7.2 Flujo de build

```
dev local (frontend):
  cd crates/coral-ui/assets/src && bun run dev  → vite dev server :5173 con proxy a :3838

dev local (Rust, sin tocar frontend):
  cargo build  → lee assets/dist/ con include_dir!

CI release:
  1) actions/setup-bun
  2) cd crates/coral-ui/assets/src && bun install --frozen-lockfile && bun run build
  3) git diff --exit-code crates/coral-ui/assets/dist/ || error "dist out of sync, commit it"
  4) cargo build --release --features ui
```

**Garantía**: el step 3 falla la CI si un PR de Rust olvidó incluir el `dist/` actualizado. Es el equivalente de `git diff Cargo.lock --exit-code` que ya usamos.

### 7.3 REST API v1

| Método | Path | MCP equivalente | Notas |
|---|---|---|---|
| GET | `/api/v1/pages` | `coral://wiki/_index` | Query params: `page_type`, `status`, `confidence_min`, `confidence_max`, `repo`, `valid_at`, `q`, `limit`, `offset` |
| GET | `/api/v1/pages/:repo/:slug` | `coral://wiki/<repo>/<slug>` | Devuelve frontmatter + body + backlinks computados |
| GET | `/api/v1/search?q=` | `tool_search` | BM25, top-K |
| GET | `/api/v1/graph` | derivado de wikilinks | Query: `repo`, `valid_at`, `max_nodes` (default 500, cap 5000) |
| POST | `/api/v1/query` | `tool_query` | Body: `{q, mode}`; respuesta SSE con `event: token` + `event: source` + `event: done` |
| GET | `/api/v1/manifest` | `coral://manifest` | |
| GET | `/api/v1/lock` | `coral://lock` | |
| GET | `/api/v1/stats` | `coral://stats` | |
| GET | `/api/v1/interfaces` | `tool_list_interfaces` | M2 |
| GET | `/api/v1/contract_status` | `tool_contract_status` | M2 |
| GET | `/api/v1/affected?since=<ref>` | `tool_affected_repos` | M2 |
| POST | `/api/v1/tools/verify` | `tool_verify` | M2 |
| POST | `/api/v1/tools/up` | `tool_up` | M2+, gated |
| POST | `/api/v1/tools/down` | `tool_down` | M2+, gated |
| POST | `/api/v1/tools/run_test` | `tool_run_test` | M2+, gated |
| GET | `/api/v1/guarantee?env=<env>` | `tool_guarantee` | M3 |
| GET | `/health` | — | Igual que `wiki serve` actual |

**Convenciones:**
- Todas las respuestas son JSON `{data, meta, error?}`.
- Paginación por `limit`/`offset` + `meta.total`, `meta.next_offset`.
- Errores tipados: `{error: {code, message, hint?}}` con códigos como `WIKI_NOT_FOUND`, `INVALID_FILTER`, `LLM_NOT_CONFIGURED`, `WRITE_TOOLS_DISABLED`.

### 7.4 ¿Tokio o `tiny_http` síncrono?

`tiny_http` es bloqueante y un thread-per-request. Para una UI local con un solo usuario humano + un IDE polling cada par de segundos esto es **perfectamente suficiente** — y mantiene la invariante de no arrastrar `tokio` a `coral-core`. **Decisión:** mantener `tiny_http`. Si en v0.35+ aparece un caso de uso con cientos de conexiones simultáneas (SaaS / multiusuario), se reevalúa.

SSE sobre `tiny_http` es viable: `Response` con `chunked` encoding + content-type `text/event-stream`. El runner LLM (Claude CLI) ya escribe stdout linea-por-linea, mapear eso a SSE es ~30 líneas.

### 7.5 Inyección de config en runtime

`index.html` contiene:

```html
<script>window.__CORAL_CONFIG__ = /* __CORAL_RUNTIME_CONFIG__ */ null;</script>
```

El handler de `GET /` reemplaza ese comentario por:

```json
{"apiBase":"/api/v1","authRequired":true,"writeToolsEnabled":false,"version":"0.32.0","defaultLocale":"en"}
```

Esto permite servir la UI bajo proxy con prefijo (`/coral/`) sin rebuilds, mismo patrón que `SmartStaticFiles` de LightRAG.

### 7.6 Tamaño del bundle (presupuesto)

Dos columnas: **gzipped** (lo que ve un browser sobre la red) y **on-disk uncompressed** (lo que `include_dir!` embebe literalmente en el binario, byte por byte).

| Pieza | Gzipped | On-disk (raw JS/CSS) |
|---|---|---|
| React 19 + React DOM | ~45 KB | ~145 KB |
| React Router v7 | ~12 KB | ~40 KB |
| TanStack Query | ~12 KB | ~38 KB |
| Zustand | ~3 KB | ~9 KB |
| Tailwind output purged | ~15 KB | ~55 KB |
| shadcn (~20 componentes usados) | ~25 KB | ~80 KB |
| Sigma.js + Graphology + react-sigma + 2 layouts | ~120 KB | ~410 KB |
| react-markdown + rehype-sanitize + remark-gfm | ~35 KB | ~115 KB |
| Mermaid (lazy chunk) | ~150 KB | ~480 KB |
| i18next + react-i18next | ~15 KB | ~50 KB |
| Bundles locales (en + es) JSON | ~5 KB | ~15 KB |
| Lucide icons (tree-shaken, ~30 íconos) | ~10 KB | ~35 KB |
| App code | ~40 KB | ~130 KB |
| HTML + assets misc (fonts, favicon) | ~10 KB | ~30 KB |
| **Total bundle on-disk uncompressed** | — | **~1.63 MB** |
| Bundle pre-compresión brotli (servido como `Content-Encoding: br`) | — | **~420 KB on-disk** |

**Decisión de embed:** los assets se almacenan **pre-comprimidos en brotli** dentro de `assets/dist/` y `include_dir!` los embebe así. El servidor `tiny_http` los devuelve con `Content-Encoding: br` sin descomprimir en runtime — el browser hace el work. Para clientes sin soporte brotli (extremadamente raros en 2026), se incluye también la variante `.gz` (~480 KB total).

**Delta sobre binario base v0.31.1 (6.3 MB stripped):**
- Compresión brotli: +0.42 MB → **6.72 MB binario final**
- Compresión gzip fallback: +0.48 MB → **7.20 MB**
- Sin brotli/gzip embedding (raw): +1.63 MB → **7.93 MB**

**O2 actualizado:**
- Target M1: **≤ 8.5 MB stripped** (asume brotli embed → 6.72 MB; deja 1.78 MB de buffer real para slippage de deps + variabilidad cross-platform de stripping)
- CI enforcement gate: **8.0 MB** (deja 0.5 MB headroom antes de alarma)
- Target M3: **≤ 10 MB** (espacio para Sigma layouts adicionales, vista Guarantee, dark theme)

### 7.7 Distribución y plug-and-play

- **Feature flag `ui` en `coral-cli/Cargo.toml`**: default-on en releases, default-off en `cargo install --no-default-features` para usuarios que quieran un binario aún más pequeño.
- **Sin Node ni Bun en el host de producción.** El bundle ya está en el binario.
- **Plugin Claude Code**: `.claude-plugin/plugin.json` ya empaqueta el binario. Se añade una skill `coral-ui` que enseña a Claude *cuándo* sugerir abrir la UI (ej. "el usuario pregunta sobre la arquitectura general" → respuesta: "ejecutar `coral ui serve` y abrir el grafo en localhost:3838").
- **Windows first-class**: el binario embebido funciona idénticamente en Windows. CI ya produce `x86_64-pc-windows-msvc` desde v0.31.0; añadir job de smoke-test que arranque `coral ui serve --no-open` y haga `curl /health` cross-platform.

---

## 8. Stack y dependencias

### 8.1 Frontend (`crates/coral-ui/assets/src/package.json`)

> ⚠️ **Snapshot orientativo, no normativo.** Las versiones exactas se congelan en la primera semana del M1 vía `bun add` + `bun.lock`. Lo que sigue es la intención (familias de versiones probadas en LightRAG + ajustes para shadcn compat).

```json
{
  "dependencies": {
    "react": "^19",
    "react-dom": "^19",
    "react-router-dom": "^7",
    "@tanstack/react-query": "^5",
    "zustand": "^5",
    "sigma": "^3",
    "@react-sigma/core": "^5",
    "graphology": "^0.26",
    "graphology-layout-forceatlas2": "^0.10",
    "graphology-layout-noverlap": "^0.4",
    "react-markdown": "^9",
    "rehype-sanitize": "^6",
    "remark-gfm": "^4",
    "mermaid": "^11",
    "i18next": "^23",
    "react-i18next": "^15",
    "tailwindcss": "~3.4",
    "lucide-react": "^0.4",
    "@radix-ui/react-*": "latest (~1.x familia)"
  },
  "devDependencies": {
    "vite": "^7",
    "typescript": "^5",
    "@vitejs/plugin-react": "^5"
  }
}
```

**Notas de selección:**
- **Tailwind se mantiene en 3.4 LTS** (no 4.x) porque shadcn/ui aún no soporta oficialmente Tailwind 4 en todos sus componentes (oxide/lightningcss rompe varios primitives). Reevaluar en M3 cuando shadcn cierre la migración.
- **`rehype-sanitize` obligatorio** sobre `react-markdown` — mitigación de R4 (XSS persistido en wiki).
- **i18next 23 + react-i18next 15** son las majors estables actuales; las "^26"/"^16" del v1.0 del PRD eran proyecciones futuras erradas.

### 8.2 Backend (`crates/coral-ui/Cargo.toml`)

```toml
[dependencies]
tiny_http = "0.12"          # ya en workspace
serde = { workspace = true }
serde_json = { workspace = true }
include_dir = "0.7"          # NUEVA — embebe assets/dist/
mime_guess = "2"             # NUEVA — tipos MIME para static
coral-core = { workspace = true }
coral-mcp  = { workspace = true }   # reusa builders de recursos/tools
coral-stats = { workspace = true }
```

Sin tokio, sin hyper, sin axum. Mantenemos la disciplina del PRD-v0.24.

---

## 9. Fases / hitos

> **Nota timeline (post-review):** la v1.0 del PRD planteó M1 en 4–6 semanas. El review independiente identificó esa estimación como sub-estimada ~50% para un solo desarrollador asumiendo cero días perdidos en debugging Vite/include_dir/tiny_http SSE/cross-platform. Las estimaciones revisadas asumen 1 dev full-time con buffer realista de ~25%.

### M1 — v0.32.0 (8–10 semanas)
- Semana 1: **spike técnicos gate** (ver §15) — SSE sobre tiny_http, include_dir + brotli serving, baselines de KPI medidos en repo real
- Semanas 2–3: crate `coral-ui` con `serve()`, routing tiny_http, REST read-only completo, auth bearer mínima (FR-UI-22..26)
- Semana 4: bootstrap frontend (Vite + Tailwind 3.4 + shadcn + i18n)
- Semanas 5–6: vistas Pages + Graph (Sigma + ForceAtlas2 + slider bi-temporal)
- Semana 7: vistas Query (SSE streaming) + Manifest
- Semana 8: CI pipeline (bun build + drift check), tests integración Rust ≥ 70%, smoke test cross-platform (Linux/macOS/Windows)
- Semanas 9–10: documentación (`docs/UI.md` + README + 2 screenshots), buffer para regresiones, release v0.32.0

### M2 — v0.33.0 (5–6 semanas)
- Vistas Interfaces / Drift / Affected (depende de FR-IFACE del PRD-v0.24)
- Write-tools gating (`/api/v1/tools/up|down|run_test`)
- Tests E2E con Playwright en matrix Linux + Windows en CI
- Artefacto release `coral-*-nocli.tar.gz` (sin UI) para air-gap

### M3 — v0.34.0 (4–5 semanas)
- Vista Guarantee / Can-I-Deploy (depende de FR-TEST-1 del PRD-v0.24)
- SSE para "wiki rebuilt, refetch" (notifications push-style)
- Export del grafo (PNG/SVG/GraphML)
- Theme dark/light + persistencia en `localStorage`
- Reevaluar migración a Tailwind 4 si shadcn ya cerró su migración

---

## 10. Métricas de éxito

> Las celdas marcadas **`(S3)`** son baselines a medir en el spike S3 de semana 1 antes de firmar los targets numéricos. Los targets aquí listados son la *intención inicial*; el spike S3 puede mover ±30% antes del kickoff de semana 2.

| KPI | Baseline (v0.31) | Target M1 | Target M3 |
|---|---|---|---|
| Tamaño del binario (stripped, Linux x86_64) | 6.3 MB (medido) | ≤ 8 MB | ≤ 9 MB |
| Tiempo de cold-boot del binario (`coral ui serve` → `/health` 200) | `(S3)` | ≤ 300 ms | ≤ 150 ms |
| Time-to-first-paint en el browser (SPA inicial) | `(S3)` | ≤ 1.2 s (Linux), ≤ 1.8 s (Windows con AV) | ≤ 600 ms |
| Latencia p95 `GET /api/v1/pages?limit=50` sobre wiki de 500 páginas | `(S3)` | ≤ 80 ms | ≤ 30 ms |
| Latencia p95 `GET /api/v1/graph?max_nodes=500` | `(S3)` | ≤ 200 ms | ≤ 80 ms |
| FCP del grafo Sigma con 500 nodos | `(S3 via S4)` | ≤ 2.5 s | ≤ 1 s |
| Time-to-first-token en `/api/v1/query` (stream SSE) | `(S3, depende del runner)` | ≤ 2 s | ≤ 1 s |
| Cobertura del REST API por tests de integración Rust | 0% | ≥ 70% | ≥ 90% |
| Lighthouse Performance score (build prod, Chromium desktop) | n/a | ≥ 85 | ≥ 92 |
| `% MCP resources isomórficos a REST` | 0% | 100% (read) | 100% (read+write) |
| Cap server-side de nodos por `GET /api/v1/graph` | n/a | 500 default, 5000 max | 1000 default, 10000 max |

---

## 11. Decisiones abiertas

1. **¿`ui` en default-features de `coral-cli`?**
   - **Pro:** UX out-of-the-box — `cargo install coral-cli` da CLI + UI lista. Plug-and-play (memoria `project_plug_and_play`) exige cero pasos manuales.
   - **Contra:** ~0.42 MB extra (brotli embed, §7.6) para usuarios CLI-only o CI runners.
   - **Tensión con predecesor:** PRD-v0.24 §2A.1 dijo "opt-in feature flag, preserva single-binary base". Este PRD **reinterpreta esa decisión**: "opt-in" ahí se refería a *correr* la UI (sigue siendo cierto — la UI sólo se levanta con `coral ui serve`, jamás implícitamente), no a *empaquetarla*. El single-binary se preserva en ambos casos.
   - **Recomendación:** sí, default-on. Quien quiera el binario mínimo usa `cargo install coral-cli --no-default-features --features mcp,cli`. La descarga de release prebuilt incluye UI por default; para air-gap o entornos minimalistas habrá un artefacto `coral-x86_64-...-nocli.tar.gz` desde M2.

2. **¿Bun o pnpm/npm para el frontend?**
   - **Pro Bun:** velocidad de install (~3-5× pnpm), un solo binario, lo que usa LightRAG.
   - **Contra Bun:** menos ubicuo en Windows CI, pero `oven-sh/setup-bun` cubre el caso.
   - **Recomendación:** Bun, igual que LightRAG. Riesgo bajo, beneficio claro en CI.

3. **¿Mermaid keeps su lugar o se reemplaza por Sigma para todo?**
   - **Pro Mermaid:** el wiki existente tiene fences ` ```mermaid ` autoría humana; renderizarlos es esperado.
   - **Pro Sigma-only:** un solo motor de visualización, bundle más pequeño.
   - **Recomendación:** ambos. Mermaid lazy-loaded para fences manuales; Sigma para el grafo computado de wikilinks.

4. **¿`coral wiki serve` se deprecia?**
   - **Recomendación:** **no en v0.32**. Convive — es el "fallback minimalista" sin JS. Reevaluar en v0.40.

5. **¿`POST /api/v1/query` requiere CSRF / origin check?**
   - **Recomendación:** sí — `Origin` debe matchear el bind. Sin esto, una página web maliciosa abierta en el navegador del usuario puede gastar tokens LLM via `coral` corriendo en localhost.

---

## 12. Riesgos y mitigaciones

| # | Riesgo | Probabilidad | Impacto | Mitigación |
|---|---|---|---|---|
| R1 | El bundle JS rompe el budget de tamaño binario | M | M | Tree-shaking estricto + lazy-load de Mermaid + budget enforced en CI (`stat assets/dist/index.js < 200KB gzipped`) |
| R2 | `tiny_http` síncrono no aguanta SSE estable | M | M | Spike de prueba en M1 con un endpoint mock; fallback a polling si falla |
| R3 | El `dist/` commiteado genera merge conflicts crónicos | A | B | `.gitattributes` marca `linguist-generated=true merge=ours` (aceptar la versión del branch que mergea; CI post-merge corre `bun run build` y commitea el rebuild si hay diff). Procedimiento documentado en `CONTRIBUTING.md`. **Consistente con §15 semana 8.** |
| R4 | Inyección XSS via Markdown del wiki | B | A | `react-markdown` con `rehype-sanitize` por defecto; CSP estricta en el header `Content-Security-Policy` |
| R5 | Same-origin trust roto (DNS rebinding) o CORS misconfig si se bindea no-loopback | M | A | (a) bind por defecto `127.0.0.1` — same-origin, no aplica CORS; (b) en bind no-loopback, token obligatorio (FR-UI-22) + `Access-Control-Allow-Origin: <Origin exacto, validado contra allowlist>` nunca `*`; (c) `Host` header check contra `127.0.0.1`/`localhost` mitiga DNS rebinding; (d) token bearer obligatorio para `POST /api/v1/query` aún en loopback (FR-UI-23) |
| R6 | Frontend dev local sin Bun bloquea a contributores Rust | A | B | `cargo build` no requiere Bun (lee `dist/` precompilado). Sólo quien toca `assets/src/` instala Bun. Documentado en `CONTRIBUTING.md` |
| R7 | Sigma.js no escala a wikis de >5000 nodos | M | M | Cap server-side en `max_nodes` (default 500) + UI de "zoom out muestra clusters" diferida a v0.34+ |
| R8 | El placeholder de runtime config falla con CDNs que cachean `index.html` | B | M | Header `Cache-Control: no-cache` para `index.html`, `max-age=31536000, immutable` para `/assets/*` (mismo patrón que LightRAG) |
| R9 | Windows en CI tarda más en el build de Vite y bloquea releases | M | B | Build de Vite sólo en runner Linux; el `dist/` resultante se reutiliza para todos los targets cross-compile |
| R10 | v0.32.0 sale con un bug crítico en la UI y necesita yank | B | A | **Procedimiento de rollback:** (a) `cargo yank --version 0.32.0 coral-cli`; (b) usuarios siguen pudiendo `cargo install --locked --git ... --tag v0.31.1 coral-cli` para volver a v0.31.1; (c) plugin Claude Code: bump del plugin a v0.31.1 vía `/plugin update coral@coral`; (d) hotfix en branch `release/0.32.x` → v0.32.1 sólo con el fix mínimo; (e) BC sagrada garantiza que el wiki/`.coral-lock` no quedan corruptos al downgradear |

---

## 13. Anti-features (explícitos)

Decisiones de **NO hacer** algo, con justificación.

- **AF-1 — No editor WYSIWYG.** El wiki es Git-native; editarlo desde la UI rompe la auditoría de commits y crea una superficie de seguridad (XSS persistente). Coral asume que el editor del usuario (VS Code, Cursor, IDEA) es el camino feliz.
- **AF-2 — No SaaS / multi-tenant.** La UI corre local o detrás del firewall del usuario. Mantenemos la promesa de "tus datos nunca salen de tu máquina salvo cuando *tú* llamas a un LLM".
- **AF-3 — No ingesta multimodal en la UI.** Coherente con AF de PRD-v0.24. Si llega, será un comando CLI separado, no un drag-and-drop.
- **AF-4 — No analytics / telemetría opt-out.** Cero telemetría. Si quisiéramos métricas de uso, sería opt-in explícito y documentado.
- **AF-5 — No "Coral Cloud" UI prematura.** Aunque PRD-v0.24 §12A menciona sponsorware Coral Cloud, este PRD se limita al producto local. Cloud se diseña en su propio PRD.

---

## 14. Apéndice A: mapeo recurso/MCP ↔ vista UI

| Recurso MCP / Tool | Vista UI | Componente React | Endpoint REST |
|---|---|---|---|
| `coral://wiki/<repo>/<slug>` | Pages › Detail | `<PageDetail/>` | `GET /api/v1/pages/:repo/:slug` |
| `coral://wiki/_index` | Pages › List | `<PagesList/>` | `GET /api/v1/pages` |
| `coral://manifest` | Manifest | `<ManifestView/>` | `GET /api/v1/manifest` |
| `coral://lock` | Manifest › Lock tab | `<LockView/>` | `GET /api/v1/lock` |
| `coral://stats` | Sidebar widget | `<StatsWidget/>` | `GET /api/v1/stats` |
| `tool_query` | Query | `<QueryPlayground/>` | `POST /api/v1/query` |
| `tool_search` | Header search | `<GlobalSearch/>` | `GET /api/v1/search` |
| `tool_find_backlinks` | Pages › Detail › Backlinks panel | `<BacklinksPanel/>` | derivado de `pages/:slug` |
| `tool_affected_repos` | Interfaces › Affected | `<AffectedView/>` (M2) | `GET /api/v1/affected` |
| `tool_verify` | Manifest › Verify button | `<VerifyButton/>` (M2) | `POST /api/v1/tools/verify` |
| `tool_list_interfaces` | Interfaces › List | `<InterfacesList/>` (M2) | `GET /api/v1/interfaces` |
| `tool_contract_status` | Interfaces › Drift | `<DriftView/>` (M2) | `GET /api/v1/contract_status` |
| `tool_run_test`, `tool_up`, `tool_down` | Env panel | `<EnvPanel/>` (M2+, gated) | `POST /api/v1/tools/*` |
| derivado de wikilinks | Graph | `<GraphView/>` | `GET /api/v1/graph` |

---

## 15. Apéndice B: secuencia de implementación detallada (M1)

### Semana 1 — Spikes técnicos (gate go/no-go)

Estos spikes deben validarse antes de continuar. Si alguno falla, el plan se reajusta antes de invertir en frontend.

| Spike | Pregunta a responder | Criterio de éxito |
|---|---|---|
| S1 | ¿`tiny_http` soporta SSE estable con chunked encoding sin parchear? | Server demo escribe 100 eventos `data: ...\n\n` a un cliente `curl -N`, sin buffering. Mide latency p95 < 50ms. |
| S2 | ¿`include_dir!` + brotli precomprimido sirve en runtime correctamente? | Demo embebe un `index.html.br` de 50 KB, lo devuelve con `Content-Encoding: br`, Chrome lo decodifica. |
| S3 | Baseline real de los KPIs de §10 | Medir cold-boot de `coral wiki serve`, latencia `coral search`, build size actual en repo Coral mismo (que ya tiene un wiki). Reemplazar las "—" de §10 con números reales. |
| S4 | ¿Sigma renderiza 500 nodos reales del wiki Coral con ForceAtlas2 sub-2s? | Notebook standalone (`bun run dev` en un Vite throw-away) que carga JSON de los wikilinks de `.wiki/`. |
| S5 | Tailwind 3.4 + shadcn + Vite 7 + React 19: ¿stack instala sin warnings críticos? | `bun install --frozen-lockfile` + `bun run build` produce bundle < 500 KB gzipped. |

**Decisión gate:** si S1 falla, FR-UI-11 streaming pasa a M2 y M1 usa polling. Si S2 falla, los assets se embeben sin compresión (delta binario +1.2 MB, todavía dentro de 8.5 MB target). Si S5 falla, se reevalúa el stack (posible fallback: pnpm + Tailwind 3.4 + Vite 5).

**Re-baseline del timeline:** si cualquier spike falla con un fallback no trivial (S1 → polling requiere rediseño del playground; S5 → cambio de bundler), el timeline M1 se renegocia con el usuario antes de continuar con semana 2. Estimación de impacto por fallo: S1 +1 semana, S2 0 semanas, S3 0 semanas (afecta sólo KPI targets), S4 +0.5 semana, S5 +1.5 semanas. Worst case acumulado: M1 → 11–12 semanas.

### Semanas 2–3 — Backend Rust
- Crate `coral-ui` (lib + Cargo.toml + estructura de routes)
- Subcomando `coral ui serve [--port 3838] [--bind 127.0.0.1] [--no-open] [--token <t>] [--allow-write-tools]` en coral-cli (clap)
- `include_dir!` sobre carpeta dummy con `index.html` placeholder + brotli serving validado
- Auth bearer middleware + Origin check + Host header check (FR-UI-22..26)
- REST read-only completo: `GET /api/v1/{pages,pages/:r/:s,search,graph,manifest,lock,stats,health}`
- Tests de integración Rust contra wiki fixture (cobertura objetivo: 70%)
- BC test: `coral wiki serve` sigue funcionando idéntico (test `bc_regression` extendido)

### Semana 4 — Frontend bootstrap
- Vite + Tailwind 3.4 + shadcn install (CLI) + Radix
- TanStack Query setup + cliente API tipado (generación de tipos desde response samples)
- Zustand stores: `useFiltersStore`, `useGraphStateStore`, `useAuthStore`
- Routing React Router v7: `/pages`, `/graph`, `/query`, `/manifest`
- i18n setup en/es + script `lint:i18n`

### Semanas 5–6 — Vistas Pages + Graph
- `<PagesList/>` + filtros laterales + paginación + sort
- `<PageDetail/>` + Markdown renderer + rehype-sanitize + badge status + backlinks panel
- `<GraphView/>` con Sigma + ForceAtlas2 + color por status + size por degree + opacity por confidence
- Slider bi-temporal en `<GraphView/>` con re-query a `/api/v1/graph?valid_at=`
- Lazy import Mermaid (sólo si una página tiene fence ` ```mermaid `)

### Semana 7 — Query + Manifest
- SSE endpoint `POST /api/v1/query` (envuelve `Runner::run_streaming`)
- `<QueryPlayground/>` con streaming via `EventSource` polyfill (POST EventSource necesita polyfill o `fetch` + `ReadableStream`)
- `<ManifestView/>` + `<LockView/>` + botón "abrir en editor"
- Warning de costo LLM (FR-UI-14)

### Semana 8 — CI + cross-platform
- Workflow `bun build` antes de `cargo build --release`
- Drift check: `git diff --exit-code crates/coral-ui/assets/dist/`
- `.gitattributes`: `crates/coral-ui/assets/dist/** linguist-generated=true merge=ours`
- Smoke tests cross-platform: `coral ui serve --no-open --port 38400 &` + `curl /health` + `curl -H "Authorization: Bearer $T" /api/v1/pages | jq` en Ubuntu + macOS + Windows
- Verificar tamaño binario contra O2 en CI (`stat coral.exe < 9_000_000`)

### Semanas 9–10 — Polish + release
- Documentación: `docs/UI.md` + sección "WebUI" en README
- 2 screenshots en `docs/assets/` (vista Pages, vista Graph con slider bi-temporal)
- Plugin Claude Code: skill `coral-ui` actualizado
- CHANGELOG entry detallada
- Release v0.32.0 con notas que mencionen explícitamente:
  - BC: `coral wiki serve` no cambió
  - Binario único, sin runtime de Node
  - 4 vistas + i18n + auth bearer
  - Cómo desactivar UI con `cargo install --no-default-features --features mcp,cli`

---

## 16. Criterios de aceptación (Definition of Done para M1)

1. `cargo install --git ... --tag v0.32.0 coral-cli` produce un binario ≤ 8 MB stripped que incluye la UI.
2. `coral ui serve` arranca el servidor en < 300 ms, abre el navegador automáticamente (salvo `--no-open`) y muestra la vista Pages con datos del wiki actual.
3. Las 4 vistas (Pages, Graph, Query, Manifest) son navegables y traducidas en/es. Switching de idioma persiste en `localStorage`.
4. El grafo renderiza 500 nodos con ForceAtlas2 en < 2.5 s y soporta zoom/pan suave. Slider bi-temporal funcional.
5. `POST /api/v1/query` con token bearer streamea tokens del LLM configurado en `coral.toml` vía SSE (o polling si S1 falló).
6. CI bloquea PRs donde `crates/coral-ui/assets/dist/` está fuera de sync con `assets/src/`.
7. Lighthouse Performance ≥ 85 en build de producción, **Chromium ≥ 120 y Firefox ≥ 121** (Safari best-effort, no bloqueante para release).
8. `cargo install --no-default-features --features mcp,cli` produce un binario sin UI ≤ 6.5 MB.
9. Cobertura de tests de integración del REST API ≥ 70% (medido con `cargo llvm-cov` sobre `coral-ui::routes`).
10. README actualizado con sección "WebUI" + 2 screenshots (Pages, Graph).
11. **Backward compat sagrada**: `coral wiki serve` sigue funcionando idéntico a v0.31.1. El test `bc_regression` extendido lo verifica (mismas rutas, mismos códigos, mismos bodies HTML).
12. Plugin Claude Code (`.claude-plugin/`) actualizado con una skill que enseña a Claude cuándo sugerir `coral ui serve` (ej. usuario pregunta sobre la arquitectura general del wiki).
13. **Seguridad smoke test**: tests verifican (a) `POST /api/v1/query` sin token → 401; (b) `GET /api/v1/pages/../../etc/passwd` → 400 con `INVALID_FILTER`; (c) bind a `0.0.0.0` sin `--token` → error abort; (d) `Host: evil.com` header → 400.
14. CHANGELOG y README mencionan explícitamente: "v0.32.0 introduces `coral ui serve` — opt-in modern WebUI. The legacy `coral wiki serve` continues working unchanged."

---

*Fin del PRD v1.2 — incorpora segunda ronda de fixes:*
*(a) merge driver R3 alineado a `merge=ours` consistente con §15 semana 8;*
*(b) O2 target subido a 8.5 MB con CI gate en 8.0 MB para buffer real;*
*(c) re-baseline de timeline si los spikes fallan (worst case 11–12 semanas);*
*(d) R10 nuevo: procedimiento de rollback explícito ante yank de v0.32.0.*

*PRD v1.1 (2026-05-11): primera ronda de fixes del review independiente:*
*(1) versiones de deps marcadas como snapshot + corregidas las claramente erradas (Tailwind 3.4 LTS, i18next 23/15, Vite 7, lucide 0.x);*
*(2) §7.6 rehecho con cuenta de tamaño coherente con O2 (delta brotli +0.42 MB → ≤ 8 MB);*
*(3) spike SSE/include_dir/Sigma/baselines movido a semana 1 como gate go/no-go;*
*(4) §11.1 justifica el cambio default-on vs predecesor opt-in;*
*(5) timeline M1 ampliado 4–6 → 8–10 semanas; M2 3–4 → 5–6; M3 2–3 → 4–5;*
*(6) auth bearer adelantada a M1 (FR-UI-22..26) con path-traversal mitigation explícito;*
*(7) R5 reescrito separando same-origin loopback de bind no-loopback con Host header check anti DNS-rebinding;*
*(8) §10 KPIs marcan baselines pendientes de medición en S3;*
*(9) DoD §16 añade tests de seguridad + matriz navegadores + bc_regression extendido.*
