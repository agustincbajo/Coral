# PRD — Coral v0.24 → v0.30: Performance, Garantía de Testing, Contención Multi-Repo, Moat Competitivo

**Versión del documento:** 1.2
**Fecha:** 2026-05-09
**Autor:** Agustín Bajo
**Estado:** Borrador
**Versiones objetivo:** Coral v0.24.x → v0.30.x

> **Cambio v1.0 → v1.1:** §2A (posicionamiento), §6.5–6.7 (FR-MOAT/RAG/AGENT), §13 (anti-features), §14 (research apéndice). **Cambio v1.1 → v1.2:** capa estratégica completa — §2B (North Star + Wedge), §6.8 (FR-KILLER: time-travel, PR enhancer, MCP preview, semantic code diff, auto-migration de consumers), §11A (top 10 killer features ranked + 60-second demos), §12A (GTM plan + beachhead + canales), §12B (enterprise readiness: SBOM, SLSA L3, OpenSSF, air-gap), §12C (ecosystem & community), §15 (demo scripts 90s/5min/self-guided). Tighten en §3.2, §6.4, §6.6, §11.1–11.4. Filosofía core intacta.

---

## 1. Resumen ejecutivo

Coral es hoy un binario Rust (~6.3 MB) que mantiene wikis estructuradas en Markdown+Git, orquesta entornos multi-repo, ejecuta tests funcionales y expone todo vía MCP. Este PRD lleva a Coral del estado v0.23.3 a v0.30 en 3 fases.

**Wedge** (la primera y única razón por la que un dev instala Coral): `coral test guarantee --can-i-deploy <env>` — un comando que en < 5 min responde **verde / amarillo / rojo** combinando contract drift cross-repo + tests + coverage + flake-rate, con MCP push subscriptions que avisan al agente IDE en < 1s cuando algo cambia. **Ningún competidor responde esa pregunta hoy**: Pact Broker la responde solo para HTTP-pact (no contracts gRPC/AsyncAPI/SQL, no tests-as-evidence, requiere broker server); Nx affected sabe qué tests correr pero no si un deploy es seguro; Cursor/Cody/Aider tienen contexto de código pero cero awareness de drift de interfaces.

**North Star**: **TTTC = Time-To-Trusted-Context** — segundos desde un cambio de interfaz en repo A hasta que un agente operando en repo B tiene contexto correcto (drift detectado, blast-radius computado, wiki actualizado, página drift accesible vía MCP). Target v0.24.0: < 5 s. Target v0.30: < 1 s. Esta métrica es propia de Coral, ningún otro tool la mide porque ninguno cubre la primitiva.

**Cinco movimientos estratégicos** (research §14, ranking §11A) que se refuerzan: (1) **MCP cachado + subs** (FR-PERF-1, FR-IFACE-7, M1.1+M1.12) — 50-100× tools/call + push drift; (2) **`guarantee --can-i-deploy`** (FR-TEST-1, M1.5) — wedge; (3) **Interface Contract Layer multi-formato** (FR-IFACE-1..9, M1.9+M2.1) — HTTP+gRPC+AsyncAPI+SQL+ENV + RRF hybrid + dual-level; (4) **Killer features Git-native** (FR-KILLER-1..8, §6.8) — wiki time-travel, semantic code diff, mcp preview, pr-enhance, migrate-consumers; (5) **Anti-features como identidad** (§13).

Distribución (§12A): brew + GitHub Action + MCP Registry + Cursor "Add" + Claude Skills marketplace. Sustainability: MIT forever + sponsorware Coral Cloud. Enterprise (§12B): SBOM + SLSA L3 + OpenSSF Scorecard ≥ 8 + audit log + air-gap-first.

---

## 2. Contexto y problemas

### 2.1 Problemas observados en v0.23.3

| # | Problema | Evidencia |
|---|---|---|
| P1 | El servidor MCP re-lee y re-parsea TODO el wiki en cada `tools/call`. Con 10 calls/s sobre 500 páginas: 5000 parses/s. | [`crates/coral-cli/src/commands/mcp.rs:261-267`](../crates/coral-cli/src/commands/mcp.rs), [`crates/coral-mcp/src/resources.rs:147-160`](../crates/coral-mcp/src/resources.rs) |
| P2 | TF-IDF/BM25 re-tokeniza el corpus completo en cada query (~100k allocs por query en wiki de 200 páginas). | [`crates/coral-core/src/search.rs:39-47, 144-152`](../crates/coral-core/src/search.rs) |
| P3 | `coral lint` corre el regex de wikilinks 3× por página (3 checks que cada uno itera todas las páginas). | [`crates/coral-lint/src/structural.rs:22, 43, 373`](../crates/coral-lint/src/structural.rs) |
| P4 | `coral query` carga TODAS las páginas en RAM y solo usa 40 (~92% del trabajo I/O+parse desperdiciado en wiki de 500). | [`crates/coral-cli/src/commands/query.rs:43, 55`](../crates/coral-cli/src/commands/query.rs) |
| P5 | `coral test` ejecuta secuencialmente — `ParallelismHint` declarado pero ignorado. | [`crates/coral-test/src/orchestrator.rs:166-170`](../crates/coral-test/src/orchestrator.rs) |
| P6 | No hay un comando paraguas que responda "¿este producto está garantizado verde?" El usuario debe correr 5 comandos e interpretar 5 outputs. | N/A — gap |
| P7 | `coral contract check` solo lee `paths.<path>.<method>.responses.<code>`. **Ignora `requestBody`, `parameters`, `components.schemas`** — un campo agregado/removido al body es invisible. | [`crates/coral-test/src/contract_check.rs:306-353`](../crates/coral-test/src/contract_check.rs) |
| P8 | No hay propagación automática del drift al agente. Cero MCP resources expuestos para interfaces. Cero file-watcher. | [`crates/coral-mcp/src/resources.rs:91-130`](../crates/coral-mcp/src/resources.rs) (catálogo de 6 resources, ninguno de interfaces) |
| P9 | Cero soporte para gRPC/protobuf, AsyncAPI/Kafka, DB schemas como interfaces tracked. | N/A — gap |
| P10 | `--affected --since` está prometido en el código pero no implementado. Productos multi-repo grandes corren todo o nada. | [`crates/coral-cli/src/commands/filters.rs:4`](../crates/coral-cli/src/commands/filters.rs) (docstring), `tool_affected_repos` retorna mock |
| P11 | `coral query` no distingue queries low-level (entidad/módulo) de high-level (flujo/concepto) — TF-IDF las trata uniformemente. | Análisis comparativo vs LightRAG |
| P12 | Sin tracking histórico de flakes, sin baseline de performance, sin mutation testing, sin coverage tracking de OpenAPI. | N/A — gap |

### 2.2 Tres temas que agrupan los problemas

- **Performance** (P1–P5): Coral es funcionalmente correcto pero subóptimo. Latencia de `tools/call` MCP y `coral query` son los hot paths más afectados.
- **Garantía de testing** (P6, P10, P12): Coral ejecuta tests bien pero no garantiza cobertura ni regresiones, y no agrega un veredicto único.
- **Contención multi-repo** (P7–P9): Coral detecta drift en superficie HTTP+status, pero invisibilidad total en cambios de schema, eventos, gRPC, DB. Sin propagación al agente.

---

## 2A. Posicionamiento competitivo

Análisis ortogonal por dimensión vs los competidores primarios y adyacentes (research detallado en §14). Para cada dimensión: ¿Coral va a ser **igual**, **mejor**, o **intencionalmente NO competir** y por qué?

### 2A.1 Tabla de superioridad

| Dimensión | LightRAG | GraphRAG | Mem0 | Zep | Sourcegraph Cody | Continue.dev | Cursor | Aider | Nx/Turborepo | Bazel | Pact/Schemathesis | **Coral v0.30 objetivo** |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **Single binary, no servidor** | ❌ Python service | ❌ Python pipeline | ❌ SaaS+cloud | ❌ SaaS+graph DB | ❌ SaaS enterprise | ✅ IDE plugin | ❌ App | ✅ CLI | ✅ Node CLI | ✅ daemon | ❌ broker server | ✅ **Mejor — Rust binary < 7 MB** |
| **Plain Markdown en Git, auditable** | ❌ JSON+graph storage | ❌ Parquet+JSON | ❌ vector store | ❌ Neo4j | ⚠️ Code only | ❌ embeddings DB | ❌ embeddings DB | ⚠️ Tags only | N/A | N/A | ⚠️ JSON pacts | ✅ **Único — Markdown+frontmatter+wikilinks** |
| **Determinismo + reproducibilidad** | ⚠️ LLM in retrieval | ⚠️ LLM in summary | ⚠️ LLM merge | ⚠️ LLM dedup | ⚠️ Embeddings drift | ⚠️ Embeddings drift | ⚠️ Embeddings drift | ⚠️ ctags only | ✅ Hash-based | ✅ Hermetic | ⚠️ Stateful | ✅ **Mejor — content_hash + bibliotecario SCHEMA** |
| **MCP-first nativo** | ❌ HTTP API | ❌ Library | ⚠️ MCP recent | ⚠️ MCP recent | ⚠️ MCP outbound | ✅ MCP inbound | ⚠️ MCP plugin | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ✅ **Mejor — 6+ resources, 8+ tools, subscriptions reales** |
| **Multi-repo orchestration** | ❌ Single corpus | ❌ Single corpus | ❌ N/A | ❌ N/A | ✅ 250k repos | ⚠️ Multi-folder | ⚠️ Multi-folder | ⚠️ Per-repo | ✅ Monorepo | ✅ Monorepo+ext | ⚠️ N/A | ✅ **Mejor — `coral.toml` + `--affected --since` cross-repo** |
| **Knowledge Graph / structured wiki** | ✅ KG via LLM | ✅ KG via LLM + Leiden | ⚠️ Vector first | ✅ Bi-temporal KG | ⚠️ Code symbols | ❌ N/A | ❌ N/A | ⚠️ Symbol map | ❌ N/A | ❌ N/A | ❌ N/A | ✅ **Diferente — humano-curado, frontmatter explícito, no LLM-generated** |
| **Dual-level retrieval (entity vs synthesis)** | ✅ Native | ⚠️ Local/global | ❌ N/A | ⚠️ Hops | ⚠️ Search only | ❌ N/A | ❌ N/A | ❌ N/A | N/A | N/A | N/A | ✅ **Igualar — FR-RAG-3** |
| **Hybrid retrieval (BM25 + vector + RRF)** | ⚠️ Hybrid | ❌ Vector | ⚠️ Vector | ⚠️ Hops | ✅ Code search | ⚠️ Embeddings | ⚠️ Embeddings | ❌ TF-IDF | N/A | N/A | N/A | ✅ **Mejor — RRF + dual-level + reranker opt-in** |
| **Bi-temporal awareness (cambios validez)** | ❌ N/A | ❌ N/A | ❌ N/A | ✅ Native | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ✅ **Mejor — wiki-as-git evidencia natural; `frontmatter.validity_window`** |
| **Affected detection cross-repo** | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ⚠️ Per-search | ❌ N/A | ⚠️ Manual | ⚠️ git diff | ✅ Project graph | ✅ rdeps query | ❌ N/A | ✅ **Igualar Nx — content_hash + DFS reverso + symbol-aware** |
| **Contract testing (HTTP+gRPC+events+DB)** | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ⚠️ HTTP only mostly | ✅ **Mejor — 4 tipos + can-i-deploy + Microcks-style** |
| **Goldset eval + RAGAs metrics** | ⚠️ UltraDomain | ⚠️ Comparison | ⚠️ DMR | ✅ DMR | N/A | N/A | N/A | N/A | N/A | N/A | ✅ Property | ✅ **Igualar — `coral query --eval`** |
| **Mutation testing integrado** | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ✅ **Único — wrapper sobre cargo-mutants + Stryker** |
| **Late Chunking (Jina)** | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | N/A | N/A | N/A | ✅ **Adoptar opt-in — FR-RAG-2** |
| **Multimodal (PDF/imagen/tabla)** | ✅ via RAG-Anything | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | ❌ N/A | N/A | N/A | N/A | ❌ **Intencionalmente NO compite — opt-in ingest de docs solamente, sin imágenes/audio. Ver §13** |
| **WebUI / visualization** | ✅ Server UI | ✅ CLI UI | ✅ SaaS UI | ✅ SaaS UI | ✅ Web | ❌ N/A | ✅ App | ❌ N/A | ✅ Cloud | ✅ Cloud | ✅ Broker | ⚠️ **Opt-in feature flag — preserva single-binary base** |
| **Open source + permisivo** | ✅ MIT | ✅ MIT | ✅ Apache | ✅ Apache | ❌ Source-available | ✅ Apache | ❌ Closed | ✅ Apache | ✅ MIT | ✅ Apache | ✅ MIT | ✅ **Mantener** |

### 2A.2 Moats (lo que NADIE más combina)

Los moats de Coral son la **intersección** de propiedades, no cualquiera por separado. Concretamente, Coral es el único producto en el mercado que combina **todas** estas:

1. **Single Rust binary < 10 MB** + **MCP-first nativo con resource subscriptions reales** + **multi-repo orchestration vía `coral.toml`/`coral.lock`** + **plain Markdown en Git como storage canónico** + **wiki bibliotecario LLM determinístico (SCHEMA-driven, no free-form generation)** + **interface contract layer multi-format (HTTP+gRPC+AsyncAPI+SQL+ENV)** + **garantía verde/roja unificada con `--can-i-deploy`** + **affected-detection cross-repo content-hash-based** + **goldset evaluation + mutation testing integrados**.

Cada competidor tiene **algunas** de estas, **ninguno** las tiene **todas**. Esa intersección es el moat. El PRD apunta a hacer cada propiedad individual ≥ paridad con el mejor del mercado en su nicho, mientras se mantiene la combinación.

### 2A.3 Donde Coral NO compite (intencional)

- **No** compite con **Mem0/Zep/Letta** en memoria conversacional para chatbots o personalización end-user — Coral es "memoria del producto/codebase" no "memoria del usuario".
- **No** compite con **Cursor/Devin/Cody** en ser-el-IDE-mismo — Coral es la **capa de contexto** que esos IDEs consumen vía MCP.
- **No** compite con **GraphRAG/RAG-Anything** en multimodal genérico (imágenes/audio/video) — Coral asume documentos técnicos textuales.
- **No** compite con **Bazel** en hermeticidad de builds — Coral confía en el toolchain del repo, no lo reproduce.

Ver §13 para anti-features y diferenciadores intencionales con justificación.

---

## 2B. North Star, Wedge y funnel de adopción

### 2B.1 North Star — Time-To-Trusted-Context (TTTC)

**Definición**: segundos desde commit en repo A (cambio de interfaz) hasta que un agente IDE conectado a repo B lee `coral://contract-drift/latest` con drift reflejado + blast-radius + wiki drift page generada.

**Targets P95**: v0.24.0 < 5s (mtime cache + ingest manual) → v0.26.0 < 2s (watcher + subscriptions push, M2.3+M2.4) → v0.30 < 1s (incremental hashing + symbol-aware blast-radius).

**Por qué TTTC vs DAU/retention**: DAU es vanity en single-author OSS. Activation (`coral init`) es necesaria pero no suficiente. TTTC mide la **propiedad emergente** que ningún otro tool tiene — si baja consistentemente, **el moat se construye**.

**Métricas secundarias**: Activation (% installs con `guarantee` exitoso < 10 min), Habit (% con MCP en config persistente IDE semana 2), Advocacy (NPS "recomendarías a tu Tech Lead" + stars/issues ratio mes 1).

### 2B.2 Wedge — el ÚNICO killer feature para primeros 1000 usuarios

**`coral test guarantee --can-i-deploy <env>` + MCP push de drift al agente**.

**Justificación con research (§14)**:
- **uv (Astral)** wedge: 8-115× faster + drop-in. Coral wedge: "el comando que responde la pregunta única que nadie más responde" — semántica, no speed.
- **Pact Broker `can-i-deploy`** es la inspiración semántica directa, pero requiere broker server, solo HTTP-pact, no integra evidence de tests/coverage. Coral lo hace file-based + single-binary + multi-formato.
- **Cursor 2.0** captó "the IDE itself"; Coral va por la **capa de contexto** que esos IDEs consumen.

**Por qué este wedge sobre alternativas**: time-travel (FR-KILLER-1) es viral pero no resuelve dolor; cache 50× (FR-PERF-1) es invisible hasta que estás adentro; RRF (FR-RAG-1) es tabla stakes. **Veredicto verde/rojo + drift push** es ortogonal a competidores y ataca dolor #1 (Tech Lead sin certeza).

### 2B.3 Adoption funnel

| Etapa | Objetivo | Tiempo P50 | Tactic |
|---|---|---|---|
| **Trial** | install + `coral init` exitoso | < 2 min | `curl coral.dev/install.sh\|sh` (Astral-style) + `brew install coral` |
| **First value** | primer `guarantee` con veredicto | < 10 min | README 5-min quickstart + repo fixture + GIF 30s del wedge |
| **Habit** | MCP en config persistente IDE | semana 1 | `coral mcp install --client claude-code\|cursor\|continue` |
| **Advocacy** | tweet/Show HN/star/PR | semana 2-4 | conf talk + sponsorware tier + "powered by Coral" badge |

Target trial→advocacy total: < 30 días para primeros 100 users.

---

## 3. Objetivos (Goals)

### 3.1 Objetivos primarios

| ID | Objetivo | Métrica de éxito |
|---|---|---|
| G1 | Reducir latencia mediana de `tools/call` MCP en wikis ≥ 100 páginas | De ~50ms a < 1ms (50× mejora) |
| G2 | Reducir latencia de `coral query` y `coral search` | De baseline actual a 5-10× más rápido en wikis ≥ 500 páginas |
| G3 | Habilitar veredicto único de garantía del producto vía `coral test guarantee` | El comando responde verde/amarillo/rojo en < 5min sobre el repo de Coral mismo (dogfood) |
| G4 | Detectar 100% de breaking changes en HTTP request bodies, response schemas, parameters | Cobertura medible: para 50 cambios sintéticos al OpenAPI de un fixture, el detector encuentra ≥ 48/50 |
| G5 | Propagar drift de interfaces al agente vía MCP automáticamente | Tras modificar `.coral/contracts/http.openapi.yaml`, el resource `coral://contract-drift/latest` refleja el cambio en < 5 segundos sin intervención humana |
| G6 | Implementar `--affected --since <ref>` para test impact analysis | El comando reduce el tiempo de CI en repo multi-servicio en ≥ 50% para PRs que tocan 1-2 archivos |

### 3.2 Objetivos secundarios

- **G7 RAG**: dual-level + RRF; precision@5 ≥ +15% vs BM25 Y ≥ vector simultáneamente (BENCH-1).
- **G8 Runners**: `TestKind::Contract` (Pact) + `TestKind::Event` (AsyncAPI/Kafka) operativos sobre fixtures.
- **G9 Diff semántico**: oasdiff/buf/asyncapi-diff/atlas integrados via subprocess (FR-IFACE-6).
- **G10 Test histórico**: flakes + perf baseline + mutation budget gate operativos.
- **G11 Symbol map**: `wiki bootstrap --from-symbols` < 30s sobre repo 50k LOC, ≥ 80% pub items.
- **G12 Goldset eval**: RAGAs metrics + regression gate (NFR-11).
- **G13 Killer features (§6.8)**: time-travel + diff + mcp preview + pr-enhance + migrate-consumers, cada uno con demo 60s sin broker.

---

## 4. Non-goals

Lista canónica en §13. Sumario: no RAG multimodal completo (Coral consume vía MCP); no Neo4j/MongoDB/Postgres como default (pgvector solo opt-in); no WebUI first-class; no LLM-gen sin SCHEMA; no memoria conversacional; no broker obligatorio; no `unsafe` propio (NFR-14); no break BC v0.15 (NFR-1).

---

## 5. Personas y casos de uso

### 5.1 Personas

- **P1 — Dev en proyecto multi-repo**: trabaja con 3-10 microservicios. Quiere que su agente (Claude Code/Cursor) sepa cuándo cambia una interfaz en otro repo. Hoy debe correr `contract check` manualmente.
- **P2 — Tech Lead / Staff Engineer**: quiere un veredicto único antes de deploy. Hoy interpreta 5 outputs separados.
- **P3 — Maintainer de Coral**: quiere que `cargo test` siga verde, que el binario no crezca > 10% por release, y que las nuevas features no comprometan determinismo ni seguridad.
- **P4 — Usuario de wiki single-repo (v0.15)**: quiere que su flujo siga funcionando. **No debe romperse.**

### 5.2 Casos de uso (user stories)

- **UC1 Performance MCP** (P1): wiki 800p, 50 `tools/call`/sesión. Hoy 50ms/call = 2.5s perdidos. Después: < 1ms = 50ms total.
- **UC2 Veredicto único + can-i-deploy** (P2): pre-release corre `coral test guarantee --strict --can-i-deploy production`. En < 5 min ve verify+contract+smoke+coverage+flake-rate cross-repo + cruz pact-style con `.coral/deployments.json`. Verde solo si todas las parejas (consumer, provider) verificaron Y candidate no introduce breaking vs lo desplegado.
- **UC3 Drift de schema + push** (P1): edita `http.openapi.yaml` agregando required field. Sin recargar, Claude Code en repo `worker` ve "BREAKING: affects worker, billing" via `coral://contract-drift/latest` (subscriptions push, < 1s, cero polling).
- **UC4 Test impact analysis** (P1): PR toca 2 archivos en `auth`. CI con `--affected --since=main` corre 47 tests (consumidores transitivos) en lugar de 1200. CI: 90s vs 18min.
- **UC5 Query dual-level + RRF** (P3): `coral query "AuthService::verify_token"` (entity → BM25 sobre slug/title + filter `type=module`); `coral query "cómo fluye auth end-to-end"` (synthesis → embeddings sobre body + filter `type=flow|concept` + expand-graph 1-hop). RRF combina ambos.
- **UC6 Bootstrap repo legacy** (P1): repo 50k LOC sin wiki. `coral wiki bootstrap --from-symbols`. tree-sitter extrae symbols → draft `confidence: 0.4` revisable.
- **UC7 Mutation budget gate** (P3): `--mutation-budget 80%` falla guarantee si kill rate cae bajo threshold.
- **UC8 Wiki time-travel** (P2): `coral wiki at v1.2.0 cat AuthService` para audit/incident review/ADR context. Trivial dado wiki en Markdown+Git plano.

---

## 6. Requisitos funcionales

### 6.1 Performance

| ID | Requisito | Prioridad |
|---|---|---|
| FR-PERF-1 | El servidor MCP debe cachear el wiki en RAM con invalidación por `mtime` del directorio `.wiki/`. | P0 |
| FR-PERF-2 | Opcional fase 2: el servidor MCP debe usar un file watcher (`notify` crate) para invalidación incremental fina. | P1 |
| FR-PERF-3 | Coral debe persistir un índice de búsqueda tokenizado (`.coral-search-index.bin`, formato bincode/rkyv) keyed por content_hash, invalidando con la misma estrategia que `WalkCache`. | P0 |
| FR-PERF-4 | El lint debe pre-computar `outbound_links()` una sola vez por página y compartirlo entre los checks que lo necesiten. | P1 |
| FR-PERF-5 | `coral query` debe leer solo el `index.md`, seleccionar top-N páginas (por confidence o TF-IDF rápido), y solo entonces hacer `Page::from_file` sobre ellas. | P1 |
| FR-PERF-6 | `EmbeddingsProvider::embed_batch` debe aceptar `&[&str]` en lugar de `&[String]` para evitar clones. | P2 |
| FR-PERF-7 | Coral debe usar `mimalloc` o `jemalloc` como global allocator (con feature flag para desactivar). | P2 |
| FR-PERF-8 | `STOPWORDS` debe ser un `HashSet` (lazy via `OnceLock`), no un slice con `.contains()` lineal. | P2 |
| FR-PERF-9 | `coral ingest` debe paralelizar el plan con rayon (excepto el upsert final del index, que mantiene `flock`). | P1 |

### 6.2 Garantía de testing

| ID | Requisito | Prioridad |
|---|---|---|
| FR-TEST-1 | **`coral test guarantee [--strict] [--can-i-deploy <env>] [--mutation-budget <pct>]`** — orquesta verify + contract check + test (smoke/property/recorded) + coverage + flake-rate + (opt) `--can-i-deploy` Pact-style cruzando `.coral/deployments.json` vs contracts verificados + (opt) mutation gate. Veredicto único verde/amarillo/rojo. Diferenciador vs Pact Broker: file-based sin servidor + 4 tipos de interfaz (HTTP/gRPC/AsyncAPI/SQL). | P0 |
| FR-TEST-2 | Implementar `coral test coverage [--format markdown\|json]` que cruza `discover_openapi_in_project` contra los TestCases existentes y reporta gaps. | P0 |
| FR-TEST-3 | **`--affected --since <ref>`** content-hash-based (Nx/Turborepo-style): compara con `.coral/affected-cache.json` del último build verde; DFS reverso `depends_on` → tests filtrados. `--paranoid` cae a "correr todo" si grafo unsafe. | P0 |
| FR-TEST-4 | Honor `ParallelismHint` en `orchestrator.rs:166-170`: `Isolated` → `rayon::par_iter`, `Sequential` → preservar orden. | P1 |
| FR-TEST-5 | `JunitOutput::render` debe agregar un `<testsuite>` por repo (con campo `repo: Option<String>` en `TestCase`). | P1 |
| FR-TEST-6 | Implementar `coral test flakes [--last N]` con histórico en `.coral/test-history.jsonl` (schema versionado, similar a `monitor/run.rs`). | P1 |
| FR-TEST-7 | Implementar `coral test perf [--baseline FILE] [--threshold-p95-ms N]` extendiendo `HttpEvidence` con `latency_ms` y comparator vs baseline checked-in. | P2 |
| FR-TEST-8 | Wirear `TestKind::Contract` con runner Pact-style: consumer publica pact, provider verifica vía HTTP con datos reales. `--can-i-deploy` como gate de PR. | P2 |
| FR-TEST-9 | Wirear `TestKind::Event` con Testcontainers Kafka, assertion sobre topic + schema. | P2 |
| FR-TEST-10 | (Largo plazo) Wirear `TestKind::E2eBrowser` (Playwright) y `TestKind::Trace` (assertions sobre OTel spans). | P3 |
| FR-TEST-11 | (Largo plazo) Integrar mutation testing: wrapper sobre `cargo-mutants` (Rust) y Stryker (JS/TS). | P3 |

### 6.3 Contención multi-repo (Interface Contract Layer)

| ID | Requisito | Prioridad |
|---|---|---|
| FR-IFACE-1 | Convención `<repo>/.coral/contracts/` con archivos: `http.openapi.yaml`, `events.asyncapi.yaml`, `proto/*.proto`, `db/schema.sql`, `env.contract.yaml`. Si existe, gana sobre los heurísticos actuales; si no, fallback al comportamiento v0.23 (BC). | P0 |
| FR-IFACE-2 | Extender el parser de OpenAPI para leer **`requestBody`, `parameters`, `components.schemas`**, no solo `responses.<code>`. Cierra el caso "campo agregado al body queda invisible". | P0 |
| FR-IFACE-3 | Agregar `Interface` al enum `PageType` en `frontmatter.rs:11-26` con frontmatter: `kind` (http_endpoint\|grpc_method\|event_topic\|db_table\|env_var), `provider_repo`, `consumers`, `spec_path`, `schema_hash`. | P0 |
| FR-IFACE-4 | Crear `coral-interface` crate con comandos: `coral interface ingest`, `coral interface diff [--from <ref>] [--to <ref>]`, `coral interface blast-radius <id>`, `coral interface watch` (daemon). | P0 |
| FR-IFACE-5 | Persistir `.coral/contracts.lock` con hashes semánticos canonicalizados (anti-falsos-negativos por reorder de keys YAML). | P1 |
| FR-IFACE-6 | Diff semántico delegando a binarios externos (cero deps Rust nuevas): `oasdiff` (HTTP), `buf breaking` (protobuf), `asyncapi diff` (eventos), `atlas schema diff` (SQL). Detectar via `which` con error accionable si faltan. | P0 |
| FR-IFACE-7 | MCP resources: `coral://interfaces/_index`, `coral://interfaces/<repo>/<id>`, `coral://contract-drift/latest`, `coral://interface-graph`. Cada uno con `"subscribe": true` y `notifications/resources/updated` (MCP spec 2025-11-25, §14). Coral entre los primeros MCP servers en ejercitar subscriptions reales. | P0 |
| FR-IFACE-8 | Extender el catálogo de MCP tools con: `interface_diff`, `blast_radius`, `find_consumers_of`, `contract_check`. | P1 |
| FR-IFACE-9 | **Loop contención automático**: (a) inyectar `pending_drifts` en `tools/list`+`resources/read` (fallback clientes sin subscribe), (b) emitir `notifications/resources/list_changed` (clientes con subscribe). TTL 60s. Watcher (FR-PERF-2) propaga sin polling. | P0 |
| FR-IFACE-10 | Extender `coral export-agents` para incluir sección "Cross-repo interface contracts" en `agents.md`/`claude.md`/`cursor-rules` — cierra el caso "agente sin MCP". | P1 |
| FR-IFACE-11 | Hooks git: pre-commit corre `interface diff --strict` si cambian archivos bajo `.coral/contracts/`; post-merge regenera blast-radius y escribe wiki page tipo `log` con cambios. | P2 |
| FR-IFACE-12 | (Inspirado en LightRAG) `coral query` con dual-level routing: detectar entity-level vs synthesis-level por señales en query y rutear a la estrategia adecuada. | P2 |
| FR-IFACE-13 | (Inspirado en LightRAG) `coral consolidate --gc` para detectar páginas huérfanas, wikilinks rotos tras `archive`, backlinks bidireccionales rotos. | P2 |

### 6.4 Inspirados en LightRAG (opt-ins)

| ID | Requisito | Prioridad |
|---|---|---|
| FR-LRAG-1 | `coral query --expand-graph N` — tras hit en una página, expandir N hops por backlinks/wikilinks. | P2 |
| FR-LRAG-2 | (Opt-in) `coral ingest --include-docs` para `docs/*.pdf` textuales. Sin OCR, sin imágenes (AF-4). | P3 |
| FR-LRAG-3 | (Opt-in, feature `webui`) `coral wiki serve` con grafo D3 — NO en binario base (AF-7). | P3 |
| FR-LRAG-4 | (Opt-in) Backend pgvector para wikis 10k+ páginas (AF parcial — solo embeddings, no metadata). | P3 |

### 6.5 Moats / diferenciadores estratégicos (FR-MOAT)

Diferenciadores intencionales que conforman el moat (ver §2A.2). Cada uno justificado por un competidor que **no** lo combina con las otras propiedades.

| ID | Requisito | Prioridad |
|---|---|---|
| FR-MOAT-1 | **Storage abstraction traits** — `WikiStorage`, `EmbeddingsStorage`, `IndexStorage` en `coral-core` con defaults JSON+SQLite. Otros backends (Neo4j, pgvector) viven en crates terceros. Coral mantiene 2 backends en el binario, no 6 (vs LightRAG). | P1 |
| FR-MOAT-2 | **Repo Symbol Map (Aider-style)** — `coral wiki bootstrap --from-symbols` usa `tree-sitter-language-pack` (parsers on-demand para Rust/TS/JS/Py/Go/Java/Ruby) para draft `type: module, confidence: 0.4`. Diferenciador vs Aider: Coral persiste output como Markdown auditable, no cache opaco. | P1 |
| FR-MOAT-3 | **Bi-temporal awareness (Zep-style)** — frontmatter `validity_window: { from, to }` + `superseded_by`. Coral usa Git+frontmatter, no Neo4j — auditabilidad temporal es subproducto natural de versionar Markdown. | P2 |
| FR-MOAT-4 | **Contract registry multi-format (Apicurio-style)** — `.coral/contracts/` soporta Avro + JSON Schema además de OpenAPI/AsyncAPI/proto/SQL/env. File-based, sin servidor. | P2 |
| FR-MOAT-5 | **Wiki humano-curado** — rechazo explícito de LLM-generation en runtime sin bibliotecario+SCHEMA+`confidence<1.0`. Diferencia clave vs LightRAG/GraphRAG/Cognee (KG free-form, no auditable byte-a-byte). Ver NFR-4, §13. | P0 |
| FR-MOAT-6 | **MCP sampling-back** — razonamiento secundario via MCP sampling cuando host lo soporta, no via cliente LLM embebido. Coral no se acopla a proveedor LLM en runtime. | P2 |
| FR-MOAT-7 | **`coral export-skill`** — autodetecta features y emite bundles SKILL.md (`coral-wiki-skill`, `coral-test-skill`, `coral-interface-skill`). Permite a otros agentes consumir comportamiento Coral via skill, no solo MCP. | P3 |
| FR-MOAT-8 | **Spectral-style governance** — `.coral/rules/*.yaml` con reglas custom sobre wiki+contracts. Sintaxis compatible Spectral rulesets. Coral delega lint OpenAPI a Spectral si instalado. | P2 |

### 6.6 RAG techniques 2024-2025 (FR-RAG)

| ID | Requisito | Prioridad |
|---|---|---|
| FR-RAG-1 | **Hybrid retrieval RRF** — `search` y `query` combinan BM25 + embeddings via RRF (k=60). Determinístico, ~50 LOC, cero deps. Fuente: Cormack 2009, OpenSearch 2.19. Coral expone componentes BM25/vector/RRF por separado (debuggeable, no black-box). | P0 |
| FR-RAG-2 | **Late Chunking opt-in** — para páginas > 4000 tokens, si embeddings model lo soporta (Voyage-3, Jina-v3): embed doc entero primero, mean-pool después. Provider declara `supports_late_chunking: true`. Fuente: Jina 2409.04701. | P2 |
| FR-RAG-3 | **Dual-level routing determinístico (no LLM)** — heurística: `::` o PascalCase → entity-level (`type=module|interface`, BM25-weighted); "cómo/flujo/end-to-end" → synthesis (`type=flow|concept|policy`, expand-graph N=2). Default ambos paralelo + RRF. Diferenciador vs LightRAG (que hace LLM call para keyword extraction). | P1 |
| FR-RAG-4 | **CRAG-lite** (`--verify`) — heurística determinística: si top-K hits no comparten wikilinks/`topic` frontmatter, re-tirar con `--expand-graph 2`. Sin LLM por default. Fuente: [arXiv:2401.15884](https://arxiv.org/abs/2401.15884). | P2 |
| FR-RAG-5 | **Reranker opt-in** — `EmbeddingsProvider::rerank` que delega a Voyage rerank-2.5 endpoint si `CORAL_RERANKER_ENABLED=true`. Sin model embebido. | P3 |
| FR-RAG-6 | **Goldset RAGAs eval** — `coral query --eval --goldset` reporta `context_precision@k`, `context_recall@k`, `mrr`. Goldset en `.coral/goldset.jsonl`. Bloquea PRs con regression > 2pp (NFR-11). | P1 |
| FR-RAG-7 | **`tantivy` opt-in** (feature `tantivy`) — wikis 5000+ páginas, queries < 1ms. Agrega ~3 MB → opt-in para preservar single-binary base (NFR-2). | P3 |
| FR-RAG-8 | **HyDE opt-in** (`--hyde`) — genera doc hipotético vía MCP sampling/LLM client, embeddea eso. Default off (1 LLM call/query). | P3 |

### 6.7 Agent context engineering (FR-AGENT)

| ID | Requisito | Prioridad |
|---|---|---|
| FR-AGENT-1 | **`agents.txt` + `llms.txt`** generados por `coral export-agents` — spec [llmstxt.org](https://llmstxt.org/) emergente. | P1 |
| FR-AGENT-2 | **Progressive disclosure en MCP tools** — Anthropic context engineering pattern (descripción corta + parameters mínimos). Coral hoy es verbose. Refactorizar. | P2 |
| FR-AGENT-3 | **Memory tool compat** — `.wiki/` usable directamente como Anthropic Memory tool, sin transformación. | P2 |
| FR-AGENT-4 | **MCP Tasks (experimental)** — `guarantee` puede devolver task handle en vez de bloquear conexión MCP. | P3 |
| FR-AGENT-5 | **Sampling outbound** — bibliotecario subagent usa MCP sampling cuando cliente lo soporte (no requiere API keys propias). | P3 |

### 6.8 Killer features de "wow" (FR-KILLER)

Derivadas de propiedades únicas (Git-native + content-hash + blast-radius + SCHEMA + MCP-first). Trivial dado el stack actual, imposible de copiar sin replicar primitivas. Demo de 60s c/u en §15.

| ID | Requisito | Prioridad |
|---|---|---|
| FR-KILLER-1 | **`coral wiki at <git-ref>`** — time-travel del wiki vía `git show <ref>:.wiki/<slug>.md`. Casos: auditoría compliance ("políticas activas 2026-Q3"), debugging incidentes, ADR review. Ningún competidor lo tiene porque ninguno guarda wiki en Markdown+Git plano. Diferenciador de identidad. | P0 |
| FR-KILLER-2 | **`coral diff <ref>`** semántico de código (no contracts) — usa symbol map (FR-MOAT-2) para identificar funciones públicas modificadas + consumers afectados cross-repo. Diferente de FR-IFACE-4 (que opera sobre archivos de contrato). | P1 |
| FR-KILLER-3 | **`coral mcp preview`** — dry-run que imprime resources/tools como vería un agente, sin necesidad de IDE. Baja la barrera "instalá IDE primero". | P0 |
| FR-KILLER-4 | **`coral pr-enhance` GitHub App** (feature `github-app`, opt-in) — comenta drift+blast-radius+coverage delta en PRs. Reusa `coral pr-comment` + Webhook + `octocrab`. Setup container/Cloud Run/Lambda en < 10 min con template. | P1 |
| FR-KILLER-5 | **`coral migrate-consumers <interface>`** (feature `consumer-migration`, opt-in `--apply`) — auto-PRs draft en consumers tras breaking change. Combina FR-IFACE-4 (blast-radius) + FR-MOAT-6 (sampling) + symbol map. | P2 |
| FR-KILLER-6 | **`coral diff narrative`** — post-merge hook que el bibliotecario auto-genera página `log` con cambios. Combina FR-IFACE-11 + SCHEMA. | P2 |
| FR-KILLER-7 | **`coral test gap-suggest`** — extiende FR-TEST-2 con generación determinística de skeletons Hurl/YAML por endpoint sin TestCase. De "hay gap" a "tengo .hurl listo". Sin LLM. | P1 |
| FR-KILLER-8 | **`coral scaffold module --like <slug>`** — wiki-driven scaffolding determinístico. Si LLM disponible (sampling MCP), bibliotecario refina. | P3 |

Restricción: ninguna FR-KILLER rompe §13 ni filosofía core. FR-KILLER-4/5 opt-in detrás de feature flags `github-app`/`consumer-migration`.

---

## 7. Requisitos no funcionales

| ID | Requisito |
|---|---|
| NFR-1 | **Backward compatibility**: el job `bc-regression` (6 fixtures v0.15) debe seguir verde. |
| NFR-2 | **Tamaño del binario**: el binario release no debe crecer > 10% (de ~6.3 MB a ≤ 7.0 MB) **sin** features opt-in. |
| NFR-3 | **Tests**: cada nueva feature debe agregar tests unitarios + integración. La línea base de ~1124 tests debe seguir verde. |
| NFR-4 | **Determinismo**: cero generación de páginas wiki por LLM en runtime sin pasar por el bibliotecario subagent + SCHEMA. |
| NFR-5 | **Seguridad**: nuevas integraciones de subprocess (oasdiff, buf, atlas) deben usar `--` separator y validar paths con `is_safe_filename_slug`. Sin `unsafe`. Sin `panic!` en hot paths. |
| NFR-6 | **MSRV**: Rust 1.85 mínimo. No subir a menos que haya justificación documentada. |
| NFR-7 | **Dependencias nuevas**: cada nueva dep en `Cargo.toml` debe tener comentario de justificación (siguiendo el patrón existente en `Cargo.toml:30-72`). |
| NFR-8 | **Dogfooding**: cada feature nueva debe demostrarse usando Coral sobre el repo de Coral mismo. |
| NFR-9 | **Documentación**: cada comando nuevo agrega entrada en `docs/USAGE.md`, sección en README, y ejemplo en `examples/`. |
| NFR-10 | **Performance regression gate**: agregar 4 nuevos `criterion` benches (`mcp_dispatch_bench`, `lint_bench`, `search_bm25_bench`, `embeddings_search_bench`) y bloquear PRs con regresión > 10%. |
| NFR-11 | **Goldset regression gate**: bench de retrieval (FR-RAG-6) corre en CI; precision@5 y recall@10 sobre goldset interno no pueden caer > 2pp entre commits. Documenta cambios > 2pp en CHANGELOG. |
| NFR-12 | **MCP spec lockfile**: el repo declara explícitamente la versión de MCP spec implementada (`MCP_SPEC_VERSION = "2025-11-25"`) en `crates/coral-mcp/src/lib.rs` y un test de integración valida shape de `initialize` response. Cuando aparezca una nueva spec, se trata como bump de versión deliberado (no automático). |
| NFR-13 | **Subprocess detection UX**: cuando un binario externo falta, error incluye comando install por OS + feature flag skip + link doc. |
| NFR-14 | **No `unsafe` propio** (excepciones: deps auditadas notify/mimalloc, listadas en `docs/SECURITY.md`). |
| NFR-15 | **Reproducibilidad cross-platform**: outputs byte-idénticos Mac/Linux/Windows-WSL para mismo input+lockfile. Test `cross_platform_reproducibility`. |
| NFR-16 | **Supply chain** (ver §12B): SBOM CycloneDX + cosign + SLSA L3 (`slsa-framework/slsa-github-generator`) + SHA-256 + OpenSSF Scorecard ≥ 8. |
| NFR-17 | **Air-gap-first** (§12B): cada feature opera sin Internet salvo endpoint LLM/embeddings. Tests `airgap_smoke` con outbound bloqueado. |
| NFR-18 | **Audit log JSONL versionado** (§12B): `{ts, actor, command, args_redacted, exit_code, duration_ms, hashes_in/out, mcp_session_id?}`. PII redacted. SOC 2 CC7.2. |

---

## 8. Plan de ejecución

3 fases: Fase 1 v0.24-v0.25 (4-6 sem), Fase 2 v0.26-v0.28 (8-12 sem), Fase 3 v0.29-v0.30 (8-12 sem).

### 8.2 Fase 1 — v0.24.x → v0.25.x — alto-impacto/menor-esfuerzo + 3 killer features ★

| Hito | Versión | Tareas | Esfuerzo | Paths principales |
|---|---|---|---|---|
| **M1.1** MCP cachado | v0.24.0 | (a) Cache `Arc<RwLock<Option<(SystemTime, Vec<Page>)>>>` en `CoralToolDispatcher` y `WikiResourceProvider` con invalidación por mtime. (b) Bench `mcp_dispatch_bench`. | 4-6h + 2h bench | [`crates/coral-cli/src/commands/mcp.rs`](../crates/coral-cli/src/commands/mcp.rs), [`crates/coral-mcp/src/resources.rs`](../crates/coral-mcp/src/resources.rs) |
| **M1.2** Lint pre-compute | v0.24.0 | Cambiar firma de los 3 checks que llaman `outbound_links()` para aceptar `&[(Page, Vec<String>)]` precomputado. | 2-3h | [`crates/coral-lint/src/structural.rs`](../crates/coral-lint/src/structural.rs) |
| **M1.3** Quick wins allocator + STOPWORDS | v0.24.0 | (a) Agregar `mimalloc = "0.1"` con `#[global_allocator]`. (b) `STOPWORDS` → `OnceLock<HashSet<&'static str>>`. (c) `OnceLock<AHashMap>` para tool-kind lookup. | 2-3h total | [`crates/coral-cli/src/main.rs`](../crates/coral-cli/src/main.rs), [`crates/coral-core/src/search.rs`](../crates/coral-core/src/search.rs), [`crates/coral-mcp/src/server.rs`](../crates/coral-mcp/src/server.rs) |
| **M1.4** `coral query` top-N | v0.24.1 | Leer index primero, seleccionar top-40 por confidence o TF-IDF rápido sobre titles, luego `Page::from_file`. | 3-4h | [`crates/coral-cli/src/commands/query.rs`](../crates/coral-cli/src/commands/query.rs) |
| **M1.5** `coral test guarantee` | v0.25.0 | Comando paraguas que orquesta verify + contract check + test (smoke/property/recorded) + coverage + flake-rate. Output verde/amarillo/rojo. | 12-16h | Nuevo: `crates/coral-cli/src/commands/test/guarantee.rs` |
| **M1.6** `coral test coverage` | v0.25.0 | Cruza `discover_openapi_in_project()` vs TestCases. Reporta gaps. Formatos markdown/json. | 6-8h | Nuevo: `crates/coral-cli/src/commands/test/coverage.rs` |
| **M1.7** `--affected --since` | v0.25.0 | Implementar el walk: `git diff` → archivos → repos afectados → DFS reverso. Wirear desde MCP `tool_affected_repos`. | 8-10h | [`crates/coral-cli/src/commands/filters.rs`](../crates/coral-cli/src/commands/filters.rs), [`crates/coral-cli/src/commands/mcp.rs:409-430`](../crates/coral-cli/src/commands/mcp.rs), nuevo `crates/coral-core/src/project/affected.rs` |
| **M1.8** Honor `ParallelismHint` + JUnit per-repo | v0.25.1 | (a) En `orchestrator.rs:166-170`, ramificar por hint. (b) `TestCase` agrega campo `repo: Option<String>`. (c) `JunitOutput::render` emite `<testsuites>` con `<testsuite>` por repo. | 4-6h | [`crates/coral-test/src/orchestrator.rs`](../crates/coral-test/src/orchestrator.rs), [`crates/coral-test/src/report.rs`](../crates/coral-test/src/report.rs) |
| **M1.9** Interface Contract Layer base | v0.25.x | (a) Crear `crates/coral-interface/`. (b) Convención `.coral/contracts/`. (c) Extender parser OpenAPI para leer `requestBody`+`parameters`+`components.schemas`. (d) Agregar `PageType::Interface`. (e) Comando `coral interface ingest`. | 24-32h | Nuevo: `crates/coral-interface/`, [`crates/coral-core/src/frontmatter.rs`](../crates/coral-core/src/frontmatter.rs), [`crates/coral-test/src/contract_check.rs`](../crates/coral-test/src/contract_check.rs) |
| **M1.10** RRF hybrid retrieval | v0.25.x | Implementar combinator RRF (k=60) para `coral search` y `coral query`. Test contra goldset interno (M3.6 anticipated). Bench `search_hybrid_bench`. **Cumple FR-RAG-1**. | 6-8h | [`crates/coral-core/src/search.rs`](../crates/coral-core/src/search.rs), nuevo `crates/coral-core/src/search/rrf.rs` |
| **M1.11** Storage abstraction trait | v0.25.x | Definir traits `WikiStorage`, `EmbeddingsStorage`, `IndexStorage`. Refactor JSON + SQLite implementations. **Cumple FR-MOAT-1**. Sin regresiones en BC tests. | 8-10h | [`crates/coral-core/src/storage.rs`](../crates/coral-core/src/storage.rs) |
| **M1.12** MCP spec 2025-11-25 + subscriptions | v0.25.x | Bumpear MCP spec a 2025-11-25. Implementar `notifications/resources/list_changed` y `resources/subscribe` para `coral://contract-drift/latest`. **Cumple FR-IFACE-7, FR-IFACE-9, NFR-12**. ★ Killer #2 (§11A). | 10-14h | [`crates/coral-mcp/src/server.rs`](../crates/coral-mcp/src/server.rs), [`crates/coral-mcp/src/resources.rs`](../crates/coral-mcp/src/resources.rs) |
| **M1.13** `coral wiki at <git-ref>` | v0.24.1 | Comando que lee el wiki en cualquier ref Git via `git show <ref>:.wiki/<slug>.md` + parse. Subcomandos `cat`, `ls`, `query`. Cero deps nuevas. **Cumple FR-KILLER-1**. ★ Killer #3 (§11A). | 4-6h | Nuevo: `crates/coral-cli/src/commands/wiki/at.rs` |
| **M1.14** `coral mcp preview` + `coral test gap-suggest` | v0.25.0 | (a) `mcp preview` imprime resources/tools como vería un agente sin instalar IDE; (b) `gap-suggest` extiende coverage con generación de skeletons Hurl/YAML por endpoint sin TestCase. **Cumple FR-KILLER-3 + FR-KILLER-7**. | 6-8h + 6-8h | [`crates/coral-cli/src/commands/mcp.rs`](../crates/coral-cli/src/commands/mcp.rs), [`crates/coral-cli/src/commands/test/coverage.rs`](../crates/coral-cli/src/commands/test/coverage.rs) |
| **M1.15** Supply chain hardening | v0.25.x | (a) `cargo cyclonedx` en `release.yml` emite SBOM. (b) `slsa-framework/slsa-github-generator` Generic Generator workflow. (c) cosign signing. (d) `ossf/scorecard-action` con badge en README. **Cumple NFR-16**. | 4-6h | `.github/workflows/release.yml`, `.github/workflows/scorecard.yml` |

**Salida Fase 1**: MCP < 1ms; `guarantee --can-i-deploy` + `coverage` + `gap-suggest`; `--affected` content-hash; interface layer base + RRF + storage abstraction; MCP subs push; **3 killer features ★ (M1.5/M1.12/M1.13) demoables (§15)**; `mcp preview` pre-IDE; supply chain (SBOM/SLSA/Scorecard).

### 8.3 Fase 2 — v0.26.x → v0.28.x — TestKinds + diff semántico + dual-level + bi-temporal

| Hito | Versión | Tareas | Esfuerzo |
|---|---|---|---|
| **M2.1** Diff semántico real | v0.26.0 | Integrar `oasdiff` (HTTP), `buf breaking` (protobuf), `asyncapi diff` (eventos), `atlas schema diff` (SQL) como subprocess. Normalizar a `SemanticChange { kind, path, breaking }`. Detectar binarios via `which`. | 16-20h |
| **M2.2** MCP resources de interfaces + injector | v0.26.0 | Exponer `coral://interfaces/_index`, `coral://interfaces/<repo>/<id>`, `coral://contract-drift/latest`, `coral://interface-graph`. Inyectar `pending_drifts` en `tools/list` y `resources/read`. | 12-16h |
| **M2.3** `coral interface watch` daemon | v0.26.1 | `notify` crate, emitir MCP `notifications/resources/list_changed`, regenerar `contracts.lock` incremental. | 8-12h |
| **M2.4** MCP stateful + file watcher | v0.27.0 | Refactor del cache de M1.1 para usar `notify`, invalidación incremental fina, `Arc<RwLock<WikiState>>` compartido. | 8-12h |
| **M2.5** `TestKind::Contract` (Pact) | v0.27.0 | Runner Pact-style. Consumer publica, provider verifica. `--can-i-deploy`. | 16-20h |
| **M2.6** `TestKind::Event` (AsyncAPI/Kafka) | v0.27.1 | Testcontainers Kafka en `coral-env`. Runner análogo a HTTP pero sobre topics. | 16-20h |
| **M2.7** `coral test flakes` | v0.27.x | Histórico JSONL `.coral/test-history.jsonl`. Reporta flake_rate por case_id. Marcar `quarantined` con flake_rate > 0.2. | 8-10h |
| **M2.8** `coral test perf` | v0.28.0 | Extender `HttpEvidence` con `latency_ms`. Baseline `.coral/perf-baseline.json`. Falla si p95 regression > threshold. | 10-12h |
| **M2.9** Dual-level query routing | v0.28.0 | Heurística para detectar entity-level vs synthesis-level. Rutear a estrategia adecuada (frontmatter filter vs subgrafo). | 8-12h |
| **M2.10** `coral consolidate --gc` | v0.28.x | Detectar páginas huérfanas, wikilinks rotos tras archive, backlinks bidireccionales rotos. | 6-8h |
| **M2.11** `coral query --expand-graph N` | v0.28.x | Tras hit, expandir N hops por backlinks/wikilinks. | 4-6h |
| **M2.12** Repo Symbol Map | v0.27.x | Comando `coral wiki bootstrap --from-symbols` con `tree-sitter-language-pack`. Parsers descargados on-demand (no en binario base). Genera draft de wiki con `confidence: 0.4`. **Cumple FR-MOAT-2**. | 14-18h |
| **M2.13** Goldset eval básico | v0.27.x | `coral query --eval --goldset` con métricas precision@k/recall@k/MRR (RAGAs-inspired). **Cumple FR-RAG-6, NFR-11**. | 8-10h |
| **M2.14** Governance rules + llms.txt export | v0.28.x | (a) `.coral/rules/*.yaml` Spectral-compatible (FR-MOAT-8). (b) `coral export-agents` emite `llms.txt` (FR-AGENT-1). | 14-20h total |
| **M2.16** Bi-temporal frontmatter (validity_window) | v0.28.x | Soporte de frontmatter `validity_window` y `superseded_by` para páginas tipo `decision\|policy\|interface`. Coral query lo respeta vía filter `--at <timestamp>`. **Cumple FR-MOAT-3**. | 8-12h |
| **M2.17** `coral diff <ref>` semantic + `coral pr-enhance` | v0.27.x | (a) `diff` semántico de código con symbol-aware blast-radius (FR-KILLER-2). (b) GitHub App `pr-enhance` opt-in detrás de feature flag `github-app` (FR-KILLER-4). | 10-14h + 16-20h |
| **M2.18** `coral diff narrative` (auto wiki page post-merge) | v0.28.x | Post-merge hook que el bibliotecario subagent dispara para auto-generar página `log` con cambios. **Cumple FR-KILLER-6**. | 6-10h |

### 8.4 Fase 3 — v0.29.x → v0.30.x — opt-ins, mutation, browser, trace

| Hito | Versión | Tareas | Esfuerzo |
|---|---|---|---|
| **M3.1** Índice de búsqueda persistente | v0.29.0 | `.coral-search-index.bin` (bincode/rkyv), invalidación por content_hash. Sub-millisegundo BM25 en wikis 5000+. | 12-20h |
| **M3.2** Vocabulario interned con `Arc<str>` | v0.29.0 | Refactor de `Vec<String>` a `Vec<TokenId>` con `AHashMap<Arc<str>, TokenId>`. | 6-8h |
| **M3.3** Mutation testing wrapper | v0.29.x | Wrapper sobre `cargo-mutants` y Stryker. Reporta survivor mutants → tests débiles. | 12-16h |
| **M3.4** `TestKind::E2eBrowser` (Playwright) | v0.30.0 | Playwright runner para flujos UI cross-service. | 16-20h |
| **M3.5** `TestKind::Trace` (OTel) | v0.30.0 | Assertions sobre OpenTelemetry spans. | 12-16h |
| **M3.6** `coral migrate-consumers` + `coral scaffold module --like` | v0.30.x | (a) Auto-PRs draft en consumer repos cuando hay breaking change (FR-KILLER-5). (b) Wiki-driven scaffolding (FR-KILLER-8). Ambos opt-in. | 16-20h + 6-8h |
| **M3.7** `coral ingest --include-docs` | v0.30.x | Extraer texto de `docs/*.pdf` como `type: reference`. Opt-in. | 10-12h |
| **M3.8** Backend pgvector opt-in | v0.30.x | `CORAL_EMBEDDINGS_BACKEND=pgvector`. Solo opt-in para wikis 10k+. | 12-16h |
| **M3.9** `coral wiki serve` opt-in (feature `webui`) | v0.30.x | Servidor HTTP local con grafo D3/Mermaid. Detrás de feature flag para preservar single-binary base. | 16-24h |
| **M3.10** RAG opt-ins (todos detrás de feature flags / env vars) | v0.30.x | (a) `tantivy` BM25 backend (FR-RAG-7) wikis 5000+ — 10-14h. (b) Late Chunking en `EmbeddingsProvider` (FR-RAG-2) — 8-12h. (c) CRAG-lite `--verify` (FR-RAG-4) — 6-10h. (d) HyDE `--hyde` (FR-RAG-8) — 6-10h. (e) Voyage rerank-2.5 (FR-RAG-5) — 6-8h. | 36-54h total |
| **M3.11** Mutation budget gate + export-skill autoextend + MCP Tasks | v0.30.x | (a) `--mutation-budget` extiende M3.3 (FR-TEST-11). (b) `coral export-skill` autodetect (FR-MOAT-7). (c) MCP Tasks handle (FR-AGENT-4). | 18-24h total |

**Salida Fase 3**: search sub-ms wikis grandes, mutation+budget gate, runners 100% TestKind, eval rigurosa, RAG opt-ins (tantivy/pgvector/webui/late-chunking/CRAG/HyDE/reranker) sin comprometer core, multi-skill export.

---

## 8A. Sprint plan ejecutable — start mañana (2026-05-10)

Roadmap día-por-día para llegar a **v0.24.0 release el domingo 2026-06-21** (6 semanas). Asume single-author con disponibilidad ~4-6h/día efectivos.

### 8A.1 Day 0 — domingo 2026-05-10 (prep, ≤ 2h)

Hacer ANTES de tocar código. Setup pesa más que el primer commit.

- [ ] Crear milestone GitHub `v0.24.0` con due date `2026-06-21`
- [ ] Crear 12 issues, una por hito: M1.1, M1.2, M1.3, M1.4, M1.5, M1.6, M1.7, M1.8, M1.9, M1.12, M1.13, M1.15 (linkeadas al milestone)
- [ ] Crear gh project board "Coral v0.24.0 sprint" con columnas Backlog/Doing/Review/Done; mover los 12 issues a Backlog
- [ ] Capturar baseline benchmarks: `cargo bench --workspace 2>&1 | tee target/criterion/baseline-v0.23.3.txt`
- [ ] Verificar verde: `cargo test --workspace --release && cargo clippy --workspace -- -D warnings`
- [ ] Crear branch base: `git checkout -b feat/v0.24.0-staging` (los PRs se mergean acá; al final, PR único de staging → main)
- [ ] Reservar slot calendario: lunes-viernes 8:00–12:00 + sábados 10:00–13:00, etiqueta "v0.24.0 sprint"

### 8A.2 Sprint 1 — 2026-05-11 → 2026-05-17 — warmup + ★ killer #3 (wiki at)

Semana de confianza con codebase. El primer killer feature es el más liviano (M1.13, 4-6h) y demoable solo, perfecto para construir momentum.

| Día | Tarea | Hito | Esfuerzo | Branch / PR |
|---|---|---|---|---|
| Lun 05-11 AM | M1.3a `mimalloc` global allocator en `coral-cli/src/main.rs` | M1.3 | 1h | `perf/mimalloc` → PR #1 |
| Lun 05-11 PM | M1.3b `STOPWORDS` → `OnceLock<HashSet<&'static str>>` | M1.3 | 30m | `perf/stopwords` → PR #2 |
| Lun 05-11 PM | M1.3c tool-kind lookup → `OnceLock<AHashMap>` | M1.3 | 1h | `perf/tool-kind` → PR #3 |
| Mar 05-12 | M1.2 pre-compute `outbound_links()` una vez en lint | M1.2 | 2-3h | `perf/lint-precompute` → PR #4 |
| Mié 05-13 AM | M1.13 design + skeleton `crates/coral-cli/src/commands/wiki/at.rs` | M1.13 | 2h | `feat/wiki-at` |
| Jue 05-14 | M1.13 implementar `cat`, `ls`, `query` subcomandos via `git show <ref>:.wiki/<slug>.md` | M1.13 | 3h | (cont.) |
| Vie 05-15 AM | M1.13 tests integración + assert_cmd CLI tests | M1.13 | 2h | (cont.) |
| Vie 05-15 PM | Grabar GIF demo 30s `coral wiki at HEAD~10 query "auth"` → `docs/demos/wiki-at.gif` | M1.13 | 1h | → PR #5 |

**Definition of done sprint 1**: 5 PRs merged. ★ killer #3 demoable. Bench mimalloc ≥ 5% improvement vs baseline.

### 8A.3 Sprint 2 — 2026-05-18 → 2026-05-31 — MCP cache + ★ killer #2 (subs) + interface scaffold

Foco en bloque crítico: M1.1 (cache) y M1.9 (interface scaffold) son blockers para M1.12 (subs). Por eso 2 semanas.

| Días | Tarea | Hito | Esfuerzo |
|---|---|---|---|
| Lun-Mar 05-18/19 | M1.1 cache `Arc<RwLock<Option<(SystemTime, Vec<Page>)>>>` en `CoralToolDispatcher` + `WikiResourceProvider` con invalidación por mtime de `.wiki/` | M1.1 | 4-6h |
| Mié 05-20 | M1.1 bench `mcp_dispatch_bench` con criterion + verificar 50× speedup en wiki ≥ 100 páginas | M1.1 | 2h |
| Jue-Vie 05-21/22 | M1.9 fase A — `cargo new crates/coral-interface --lib` + parser OpenAPI extendido (`requestBody.content.application/json.schema` + `parameters` + `components.schemas`) | M1.9 partial | 8h |
| Lun-Mar 05-25/26 | M1.9 fase B — agregar `PageType::Interface` en `frontmatter.rs:11-26` + comando `coral interface ingest` mínimo (sin `contracts.lock` aún) | M1.9 partial | 8h |
| Mié 05-27 | M1.12 fase A — bumpear MCP `PROTOCOL_VERSION` a `2025-11-25` en `crates/coral-mcp/src/lib.rs` + handshake versión + tests retrocompat | M1.12 | 4h |
| Jue 05-28 | M1.12 fase B — implementar `notifications/resources/list_changed` server-push para `coral://contract-drift/latest` | M1.12 | 4h |
| Vie-Dom 05-29/31 | M1.12 fase C — `resources/subscribe` + tracking de subscriptions con TTL 60s + tests E2E con cliente MCP mock | M1.12 | 4-6h |

**Definition of done sprint 2**: 7 PRs merged (#6-#12). ★ killer #2 demoable. Latencia mediana `tools/call` < 1ms en wiki de 500 páginas (medible vs baseline).

### 8A.4 Sprint 3 — 2026-06-01 → 2026-06-14 — coverage + affected + ★ killer #1 (guarantee)

El killer feature más complejo. Requiere coverage (M1.6) y affected (M1.7) como inputs.

| Días | Tarea | Hito | Esfuerzo |
|---|---|---|---|
| Lun-Mar 06-01/02 | M1.6 `coral test coverage` cruzando `discover_openapi_in_project()` vs TestCases existentes; output markdown + json | M1.6 | 6-8h |
| Mié-Vie 06-03/05 | M1.7 `--affected --since <ref>` con DFS reverso de `depends_on` en `coral.toml`; wirear desde `tool_affected_repos` MCP | M1.7 | 8-10h |
| Sáb 06-06 | Buffer / catch-up / refactor | — | 4h |
| Lun-Mar 06-08/09 | M1.5 fase A — comando `guarantee` orquestador secuencial: verify → contract check → test --kind smoke | M1.5 | 6h |
| Mié-Jue 06-10/11 | M1.5 fase B — agregar coverage gap (M1.6) + flake-rate placeholder + composer de veredicto verde/amarillo/rojo con razones específicas | M1.5 | 6h |
| Vie 06-12 | M1.5 fase C — flag `--can-i-deploy <env>` con semánticos pact-style (todos los consumers verified contra esta versión) | M1.5 | 4h |
| Sáb-Dom 06-13/14 | M1.5 tests + dogfood sobre repo Coral mismo + grabar demo 60s | M1.5 | 4h |

**Definition of done sprint 3**: 5 PRs merged (#13-#17). ★ killer #1 demoable sobre el repo Coral mismo. **Las 3 ★ killer features funcionando.**

### 8A.5 Sprint 4 — 2026-06-15 → 2026-06-21 — polish + supply chain + release v0.24.0

| Día | Tarea | Hito | Esfuerzo |
|---|---|---|---|
| Lun 06-15 | M1.4 `coral query` top-N (lee index primero, selecciona top-40 por confidence, luego `Page::from_file`) | M1.4 | 3-4h |
| Mar 06-16 | M1.8 honor `ParallelismHint` en `orchestrator.rs:166-170` + JUnit per-repo (`<testsuites>` con `<testsuite repo="..." />`) | M1.8 | 4-6h |
| Mié 06-17 | M1.15 supply chain — `cargo-cyclonedx` SBOM + `slsa-framework/slsa-github-generator` workflow + cosign + `ossf/scorecard-action` badge | M1.15 | 4-6h |
| Jue 06-18 | Update `README.md` (nueva sección "What's new in v0.24") + `docs/USAGE.md` (entries para `wiki at`, `test guarantee --can-i-deploy`, `mcp serve` con subs) + `CHANGELOG.md` | docs | 3h |
| Vie 06-19 | Grabar demo 90s wedge + demo 5min conf talk (siguiendo §15 scripts) → `docs/demos/v0.24-wedge.mp4`, `v0.24-talk.mp4` | docs | 2h |
| Sáb 06-20 | Pre-release: `cargo release v0.24.0-rc.1 --execute` + smoke test cross-platform (linux x64, macos aarch64, macos x64) | release | 2-3h |
| Dom 06-21 | Release v0.24.0: tag + changelog + Show HN draft + tweet thread + RustConf 2026 CFP submit | release | 2h |

**Definition of done sprint 4**: 5 PRs merged (#18-#22). v0.24.0 publicado en GitHub Releases con SBOM, SLSA L3 attestation, cosign signature, OpenSSF Scorecard badge ≥ 7.0.

### 8A.6 Definition of Done — v0.24.0 (release gate)

Marcar verde sólo si TODO esto pasa:

- [ ] Las 3 ★ killer features (M1.5, M1.12, M1.13) demoables sobre el repo Coral mismo
- [ ] `coral test guarantee --can-i-deploy local` corre sobre Coral en ≤ 5min y devuelve verde
- [ ] `coral wiki at HEAD~10 query "MCP server"` devuelve resultados consistentes con el wiki histórico
- [ ] Editar `repos/<svc>/.coral/contracts/http.openapi.yaml` y `coral://contract-drift/latest` se actualiza en ≤ 5s sin polling (medido con cliente MCP de test)
- [ ] **TTTC P95 ≤ 5s** medido con script reproducible en `scripts/measure-tttc.sh`
- [ ] Tests verde: ~1124 originales + nuevos (target ≥ 1180)
- [ ] `bc-regression` job verde (6 fixtures v0.15)
- [ ] `cargo bloat --release` confirma binario ≤ 7.0 MB sin features opt-in
- [ ] SBOM CycloneDX adjunto a release artifact (`coral-v0.24.0.cdx.json`)
- [ ] OpenSSF Scorecard score ≥ 7.0 (badge en README)
- [ ] cosign signature verificable (`cosign verify-blob coral`)
- [ ] README sección "What's new" + 3 quickstarts (wiki at, test guarantee, mcp subs)
- [ ] Demo 90s wedge + 5min talk grabados en `docs/demos/`
- [ ] CHANGELOG con sección v0.24.0 narrativa (no sólo lista de commits)
- [ ] Show HN draft escrito (no publicado aún) en `docs/launch/show-hn.md`

### 8A.7 PR sequence (orden recomendado, 22 PRs en 6 semanas)

PRs pequeños, mergeables independientemente, ≤ 500 líneas net cada uno. Cada PR cierra su issue.

```
Semana 1 (warmup + killer #3):
  PR #1   perf: mimalloc global allocator                                    (M1.3a, 1h)
  PR #2   perf: STOPWORDS → OnceLock<HashSet>                                (M1.3b, 30m)
  PR #3   perf: tool-kind lookup → OnceLock<AHashMap>                        (M1.3c, 1h)
  PR #4   perf(lint): pre-compute outbound_links                             (M1.2, 2-3h)
  PR #5   feat(wiki): coral wiki at <git-ref> [★ killer #3]                  (M1.13, 4-6h)

Semana 2-3 (cache + interface + killer #2):
  PR #6   perf(mcp): cache wiki in RAM with mtime invalidation               (M1.1a, 4-6h)
  PR #7   bench(mcp): mcp_dispatch_bench criterion                           (M1.1b, 2h)
  PR #8   feat(interface): scaffold crate + extended OpenAPI parser          (M1.9a, 8h)
  PR #9   feat(interface): PageType::Interface + ingest command              (M1.9b, 8h)
  PR #10  feat(mcp): bump spec 2025-11-25 + version handshake                (M1.12a, 4h)
  PR #11  feat(mcp): notifications/resources/list_changed server-push        (M1.12b, 4h)
  PR #12  feat(mcp): resources/subscribe with TTL 60s [★ killer #2]          (M1.12c, 4-6h)

Semana 4-5 (coverage + affected + killer #1):
  PR #13  feat(test): coverage report (markdown + json)                      (M1.6, 6-8h)
  PR #14  feat(test): --affected --since with DFS reverse                    (M1.7, 8-10h)
  PR #15  feat(test): guarantee orchestrator skeleton                        (M1.5a, 6h)
  PR #16  feat(test): guarantee verdict + reasons                            (M1.5b, 6h)
  PR #17  feat(test): --can-i-deploy <env> flag [★ killer #1]                (M1.5c, 4h)

Semana 6 (polish + release):
  PR #18  perf(query): top-N selection                                       (M1.4, 3-4h)
  PR #19  feat(test): honor ParallelismHint + per-repo JUnit                 (M1.8, 4-6h)
  PR #20  ci(release): SBOM + SLSA + cosign + Scorecard                      (M1.15, 4-6h)
  PR #21  docs: README + USAGE + CHANGELOG for v0.24.0                       (3h)
  PR #22  release: v0.24.0                                                   (2h)
```

**Total: 22 PRs / ~88-110h trabajo / 6 semanas calendario.** A 4-6h/día efectivos = realista para single-author con full-time disponibility. Buffer ~30% para imprevistos absorbido en sábados libres y sprint 3 día sábado catch-up.

### 8A.8 Mañana 2026-05-10 — si solo tenés 4 horas

Si por algún motivo no podés full sprint, hacé esto mañana en 4h y al menos arrancás con momentum:

| Hora | Tarea |
|---|---|
| 1 | Day 0 prep (§8A.1) — milestones + 12 issues + project board + baseline benchmarks |
| 2 | PR #1 (mimalloc) — agregar dep + `#[global_allocator]` en `crates/coral-cli/src/main.rs` + bench check |
| 3 | M1.13 day 1 — design en papel + crear `crates/coral-cli/src/commands/wiki/mod.rs` + skeleton `at.rs` |
| 4 | M1.13 day 1 (cont.) — implementar `coral wiki at <ref> cat <slug>` mínimo via `Command::new("git").args(["show", ...])` |

Resultado mañana al final del día: setup completo + 1 PR mergeado + ★ killer #3 50% (cat funcional, faltan ls/query). Suficiente para confirmar que el plan es ejecutable.

### 8A.9 Riesgos específicos del sprint y fallback

| Riesgo | Trigger | Fallback |
|---|---|---|
| M1.9 (interface scaffold) toma más de 16h en Sprint 2 | Lunes 05-25 sin terminar fase A | Cortar scope: solo `requestBody` + `components.schemas` v0.24, dejar `parameters` para v0.24.1 |
| M1.12 subscriptions complican concurrencia con cache M1.1 | Tests E2E inestables semana 05-29 | Implementar polling fallback `--mcp-subs=poll` flag opt-in, ship subs como experimental |
| M1.5 guarantee depende de M1.7 affected y M1.7 atrasa | Sprint 3 fin de semana 1 sin M1.7 done | M1.5 corre full suite (sin affected) inicialmente; affected se integra en v0.24.1 |
| Bench mcp_dispatch_bench no muestra 50× | Bench rojo en Sprint 2 | Profile con `samply` o `cargo-flamegraph`, atacar el hot path real, ajustar target a "≥ 10×" honesto en CHANGELOG |
| Single-author burnout antes de Sprint 4 | Cualquier semana sin PRs por > 3 días | Ship v0.24.0-beta solo con M1.13 + M1.12 + M1.1; M1.5 (guarantee) se difiere a v0.24.1 |

### 8A.10 v0.25.x → v0.30.x — visión post-v0.24.0

v0.24.0 release el 2026-06-21. Después:

- **v0.25.x** (jul-ago 2026): completar Fase 1 (M1.10 RRF, M1.11 storage abstraction, M1.14 mcp preview + gap-suggest)
- **v0.26-v0.28** (sep-nov 2026): Fase 2 completa (TestKinds Pact + Event, dual-level routing, bi-temporal, diff narrative, pr-enhance, symbol map)
- **v0.29-v0.30** (dic 2026 - feb 2027): Fase 3 completa (RAG opt-ins, mutation budget, browser/trace runners, migrate-consumers, scaffold)

Cada release ulterior sigue el mismo patrón: 6 semanas / sprint plan ejecutable / ≤ 25 PRs / DoD explícito / killer feature destacable / SBOM+SLSA+Scorecard mantenidos.

---

## 9. Dependencias y orden de ejecución

```
Fase 1:
  M1.1  (MCP cache) ───────────┐
  M1.2  (lint) ────────────────┤
  M1.3  (allocator+stopwords) ─┤
  M1.4  (query top-N) ─────────┤
  M1.8  (parallelism+junit) ───┤
                               ├──► Fase 1 perf done
  M1.5  (test guarantee) ◄── M1.6 (coverage) + M1.7 (affected)
  M1.9  (interface base) ──────► bloquea Fase 2 §M2.1–M2.3
  M1.10 (RRF) ◄── M1.4 (top-N infra)
  M1.11 (storage abstraction) — independiente, mejor antes que M3.x
  M1.12 (MCP spec 2025-11-25 + subs) ◄── M1.1 (cache) + M1.9 (resources)

Fase 2:
  M2.1  (diff semántico) ◄── M1.9 (interface base)
  M2.2  (MCP resources) ◄── M1.1 + M1.12 + M2.1
  M2.3  (watch daemon) ◄── M2.2
  M2.4  (MCP stateful) ◄── M1.1 + M2.3
  M2.5  (Contract Pact) ◄── M1.5 (guarantee orchestration)
  M2.6  (Event Kafka) ◄── M1.5
  M2.7  (flakes) ◄── M1.8 (per-repo report)
  M2.8  (perf) ◄── M2.7 (history infrastructure)
  M2.9  (dual-level) ◄── M1.4 + M1.10 (RRF infra)
  M2.10, M2.11 — independientes
  M2.12 (symbol map) — independiente; consume tree-sitter-language-pack
  M2.13 (goldset eval) ◄── M1.10 (RRF) + M2.9 (dual-level)
  M2.14 (governance rules) ◄── M1.9 (interface base)
  M2.15 (llms.txt export) — independiente
  M2.16 (bi-temporal) ◄── M1.9 (PageType::Interface frontmatter)

Fase 3:
  M3.1  (search index persist) ◄── M2.9 + M1.10 (dual-level + RRF)
  M3.2  (interned vocab) ◄── M3.1
  M3.3  (mutation) — independiente; M3.4, M3.5 (browser, trace) — independientes
  M3.6  (migrate-consumers + scaffold) ◄── M2.12 (symbol map) + M1.12 (sampling)
  M3.7, M3.8 (docs ingest, pgvector) ◄── M1.11 (storage abstraction)
  M3.9  (wiki serve) ◄── M2.10 (gc) para grafo limpio
  M3.10 (RAG opt-ins consolidados) ◄── M1.11 + M2.13 + M1.12 (componentes diversos)
  M3.11 (mutation budget + export-skill + MCP Tasks) ◄── M3.3 + M1.12
```

**Ruta crítica:** M1.1 → M1.12 → M1.9 → M2.1 → M2.2 → M2.4 → M2.9 → M3.1. M1.12 + M2.9 reflejan que diferenciación competitiva depende de tenerlos antes que competidores.

---

## 10. Riesgos y mitigaciones

| # | Riesgo | Probabilidad | Impacto | Mitigación |
|---|---|---|---|---|
| R1 | El cache MCP introduce bugs de concurrencia (race entre lectura del agente y escritura del file watcher). | Media | Alto | Implementar primero con mtime-poll (M1.1) — más simple. Pasar al watcher (M2.4) solo cuando el cache esté estable. Tests con `loom` para concurrencia. |
| R2 | Los binarios externos (oasdiff, buf, atlas) no están disponibles en el sistema del usuario. | Alta | Medio | Detectar via `which` con error claro: "Install with `brew install oasdiff` or skip this check with `--no-semantic-diff`". Documentar en `docs/INSTALL.md`. CI debería instalarlos. |
| R3 | `--affected` produce falsos negativos (no incluye un test que SÍ debería correr). | Media | Alto | Implementar `--affected --paranoid` que cae a "correr todo" si la heurística no está segura. Tests unitarios sobre el grafo `depends_on` invertido. |
| R4 | El nuevo `PageType::Interface` rompe wikis v0.23. | Baja | Alto | Serde rechaza variantes desconocidas con error claro. Bumpar la `wiki-schema-version` y proveer `coral migrate-wiki`. BC test `bc-regression` con fixture pre-migration. |
| R5 | El binario crece > 10% por las nuevas features. | Media | Medio | Mover features pesadas (mimalloc, webui, pgvector) detrás de feature flags. Medir tamaño en CI por release. |
| R6 | Determinismo se erosiona con caches/watchers; `pending_drifts` injector daña UX agent; subprocess deps (oasdiff/buf/atlas) crean fricción install. | Media | Medio-Alto | Tests "doble run = same output" en CI (NFR-4); TTL 60s+opt-out env var+tamaño máx; bundling pre-compiled en releases (R10) + fallback diff sintáctico. |
| R7 | Single-author, esfuerzos > 6 meses-persona. | Alta | Alto | Priorizar P0+P1. P2/P3 = "otra release window". Fase 1 sola ya entregable. |
| R8 | **MCP spec evoluciona rápido (2025-03→06→11)**, Coral implementa 2025-11-25 — próxima rev podría romper subs. | Alta | Alto | NFR-12 (lockfile + integration test). Tracking issue mensual sobre [modelcontextprotocol](https://github.com/modelcontextprotocol/modelcontextprotocol). Mapping `coral-feature → mcp-primitive` en docs. |
| R9 | **Competidor publica "MCP for code intelligence" canónico antes que Coral termine FR-IFACE-7..9 + FR-MOAT-2** (Sourcegraph MCP outbound, Cursor 2.0 plugin). | Media | Alto | M1.12 antes de v0.25.0 si posible. Posicionamiento: single-binary + plain Markdown como propiedades únicas vs Sourcegraph SaaS. |
| R10 | **Goldset interno drift hacia tests fáciles** → false confidence. | Media | Medio | Versionado en Git, CODEOWNERS review en PRs que tocan goldset, cross-check periódico con UltraDomain mini-subset / HotpotQA. |
| R11 | **`wiki bootstrap --from-symbols` genera wikis low-quality** → erosiona confianza en wiki. | Media | Alto | Output siempre `confidence: 0.4` + banner "auto-generated, review". `coral lint` falla si > 70% páginas con confidence < 1.0 sin flag explícito. |
| R12 | **MCP subs ignoradas por mayoría de clientes; storage abstraction abre puerta a forks Neo4j; Skill bundle format cambia; tree-sitter-language-pack pesado.** | Baja-Media | Bajo-Medio | Fallback `pending_drifts`; default JSON+SQLite en core, otros backends en crates terceros; integration test shape de bundle; parsers tree-sitter on-demand pinned en lock. |
| R13 | **Wedge fracasa**: dev sin OpenAPI/proto contracts experimenta menos value. | Media | Alto | Marketing claro "designed for teams with contracts". Activation flow detecta ausencia de `.coral/contracts/` y sugiere fixture template. Beachhead (§12A.1) exige contracts. |
| R14 | **Sponsorware no llega a $30k/año en 12 meses** → bus factor. | Media | Alto | Decision rule §12A.6. Plan B: GitHub Sponsors Matched + reach Anthropic/OpenAI/Voyage por sponsorships estratégicos. Plan C: CNCF Sandbox mes 18. |

---

## 11. Métricas de éxito y telemetría

### 11.1 Targets v0.24 → v0.30

**North Star TTTC P95**: < 5s → < 1s. **Activation** (% installs primer `guarantee` < 10 min): 60% → 80%. **Habit** (% MCP persistente semana 1): 30% → 60%. **Performance** `tools/call` < 1ms → < 0.5ms (vs ~50ms baseline). **Testing** `guarantee` Coral repo P95: < 5 min → < 3 min. **Multi-repo** recall ≥ 96% → ≥ 99% (4 tipos HTTP/gRPC/AsyncAPI/SQL → +Avro/JSON Schema). **Calidad**: ~1124 tests verde, 0 BC regressions, 0 cargo audit. **Binario**: ≤ 7.0 → ≤ 8.5 MB. **Supply chain**: Scorecard ≥ 7 → ≥ 8.5.

### 11.5 Benchmarks competitivos (verificables, goldset checked-in)

BENCH-1 RRF ≥ BM25 ≥ vector P@5 (OpenSearch). BENCH-2 `tools/call` < 1ms vs LightRAG MCP > 50ms. BENCH-3 `query --eval` precision@5 ≥ 0.7, recall@10 ≥ 0.85 (RAGAs). BENCH-4 `guarantee --affected` P95 < 5 min (vs Pact+Schemathesis 8-15 min). BENCH-5 `--affected` 10-svc fixture ≥ 50% reduction (Nx ref 60-80%). BENCH-6 `interface diff` 50 cambios OpenAPI ≥ 96% recall. BENCH-7 mutation kill kernel ≥ 80%. BENCH-8 TTTC P95 < 1s (LSP watchers 100-500ms ref). BENCH-9 `wiki bootstrap` Rust 50k LOC < 30s + ≥ 80% pub items (Aider ~10s/30k ref).

---

## 11A. Killer features ranked + 60-second demos (impacto × inverso-esfuerzo)

| # | Feature | FR/Hito | Esfuerzo | Demo 60s |
|---|---|---|---|---|
| 1 ★ | `guarantee --can-i-deploy` | FR-TEST-1, M1.5 | 12-16h | "Pre-deploy. 4 min. Verde/amarillo/rojo con razón exacta. Sin broker, sin SaaS." |
| 2 ★ | MCP push drift via subscriptions | FR-IFACE-7+9, M1.12 | 10-14h | "Edito OpenAPI. Sin recargar Claude Code, próxima respuesta menciona drift. < 1s. Cero polling." |
| 3 ★ | `coral wiki at <git-ref>` | FR-KILLER-1 | 4-6h | "Leo wiki como existía hace 6 meses. Imposible en otras tools — ninguna guarda wiki en Markdown+Git plano." |
| 4 | `coral mcp preview` | FR-KILLER-3 | 3-5h | "Veo qué resources/tools verá un agente sin instalar IDE. Baja barrera de evaluación." |
| 5 | `coral test gap-suggest` | FR-KILLER-7 | 6-8h | "De gap a `.hurl` listo en 30s. Templates determinísticos por response code." |
| 6 | `coral pr-enhance` GitHub App | FR-KILLER-4 | 16-20h | "PR abre. App comenta drift+blast-radius+coverage delta. Engineer no abre CLI. Viral por team." |
| 7 | `coral diff <ref>` semantic | FR-KILLER-2 | 10-14h | "3 funciones públicas cambiaron, afectan worker+billing. Sin Sourcegraph." |
| 8 | `wiki bootstrap --from-symbols` | FR-MOAT-2, M2.12 | 14-18h | "50k LOC legacy → 30s después draft 80% pub items documentados." |
| 9 | `coral migrate-consumers` | FR-KILLER-5 | 16-20h | "Breaking change → PRs draft en consumers con patch. Cierra loop drift→fix." |
| 10 | Hybrid RRF + dual-level | FR-RAG-1+3 | 14-20h | "Entity vs synthesis routing + RRF. Mejor relevancia que search puro." |

**Las 3 ★ definen v0.24.0** (26-36h total). Sin ellas Coral es "mejor v0.23"; con ellas Coral tiene wedge defendible. Resto en M1.4–M3.x (§9).

---

## 12. Apéndices

### A. Decisiones explícitas

- **Cero deps Rust nuevas para diff semántico**: subprocess (oasdiff/buf/asyncapi-diff/atlas).
- **`notify` única dep nueva** (M2.3) — ~500 KB.
- **`mimalloc`** opt-in (default activado, desactivable).
- **`PageType::Interface`** requiere bump de wiki-schema-version + `coral migrate-wiki`.
- **`tree-sitter-language-pack`** on-demand (cache `~/.coral/parsers/`).
- **`tantivy`** feature flag opt-in (~3 MB).
- **`bincode` evitado** (RUSTSEC-2025-0141); usar `postcard`/`rkyv`.

### B. Post-v0.30 (futuro, no en este PRD)

Multi-modal completo, backends ≠ Markdown+Git, LLM-gen wiki sin SCHEMA, WebUI first-class, memoria conversacional.

---

## 12A. Go-to-Market plan

### 12A.1 Beachhead market

**Equipos Rust o TypeScript de 10-50 devs, ≥ 3 servicios con al menos 1 contract (OpenAPI/proto), Claude Code o Cursor ya en uso por ≥ 30% del team.**

Por qué: < 10 devs no sienten dolor de drift; > 50 ya tienen platform team con Bazel/Buck. Rust ecosystem valora single-binary (uv/ripgrep); TS valora drop-in (bun/biome). Coral tiene ambas. Si dev ya escribió "TODO: avisar al consumer cuando cambie esto", es Coral lead.

**Anti-beachhead v0.24-v0.26**: Python-mostly sin Rust/TS (uv cubre velocity), single-repo (wedge es multi), regulated-only (primero §12B), Java/Kotlin (Bazel/Maven ya cubren).

### 12A.2 Wedge campaign — primeros 90 días post-v0.24.0

- **Día 0 Show HN**: title "Coral, a single binary that tells your agent when an API contract drifts across repos". GIF 30s del wedge. README < 200 palabras + 5-min quickstart. Lanzamiento martes/miércoles 8am PT. Author "happy to take questions" comment en 30 min.
- **Días 1-30 content seeding**: Astral-style tutorial post ("How we made `can-i-deploy` work without a broker"). Submit a Lobsters, r/rust, r/programming, dev.to, Hashnode. 3 GIFs Twitter/X (killer features 1-3). Outreach 10 newsletters: Pragmatic Engineer, Bytes, Console.dev, This Week in Rust, AI Engineer Podcast.
- **Días 31-90 design partners + conferences**: 5-10 design partners por email directo (§12A.5). Submit MCP Registry + Cursor marketplace. Apply talks: RustConf, MCP Dev Summit, AI Engineer Summit, KubeCon NA. Title: "TTTC: closing the multi-repo loop for AI agents".

### 12A.3 Distribution channels

| Canal | Prioridad | Acción concreta v0.24 | Métrica de éxito |
|---|---|---|---|
| **Homebrew tap** | P0 | propio tap (no core hasta 75 stars+30 forks); `brew tap coral-cli/coral` | accesible D0 |
| **`curl install.sh`** | P0 | Astral-style: arch detect + SHA256 + `~/.coral/bin` | < 30s end-to-end |
| **GitHub Action** | P0 | `coral-cli/coral-action@v1` default `guarantee` con PR annotations | trending Code quality 30 días |
| **MCP Registry** | P1 | submit `registry.modelcontextprotocol.io` (preview 2025/GA late) + DNS verify `coral.dev` | featured list MCP Dev Summit 2026 |
| **Cursor "Add"** | P1 | botón web + `coral mcp install --client cursor` | 1-click install docs |
| **VS Code MCP** | P2 | extensión thin → marketplace | top 50 categoría MCP 90 días |
| **Claude Skills mktplace** | P1 | publicar `coral-{wiki,test,interface}-skill` + cookbook | referenciado en talk Anthropic |
| **Docker/OCI** | P2 | `ghcr.io/coral-cli/coral:0.24.0` | pulls > 10k en 90 días |

### 12A.4 Conferences (top 5) + 3 demos

1. **AI Engineer Summit** (jun 2026 SF) — track "Agents at Work!" + MCP. Talk "TTTC: closing the multi-repo loop". 2. **MCP Dev Summit NA** (otoño 2026, KubeCon-colocated) — subscriptions+sampling production case study. 3. **RustConf 2026** (sept 8-11 Montreal) — "Single-binary CLI as platform". 4. **KubeCon NA 2026** (nov 9-12 SLC) — Platform Engineering Day, "Governance multi-repo sin SaaS". 5. **GitHub Universe 2026** — dev tools track, demo GitHub App+Action.

Demos en §15 (A wedge 90s, B conf 5min, C self-guided fixture repo).

### 12A.5 Design partners — cómo conseguir 5-10 antes de v0.25.0

Lista 30 prospects via GitHub scrape (multi-repo OpenAPI/proto + ≥ 5 contributors + indicadores de Claude Code/Cursor). Email cold < 100 palabras, asunto "early access — comando que tu Tech Lead va a querer", 1 GIF + CTA 30-min Zoom. Onboarding promise: setup 30 min + Slack channel + priority issues + Coral Cloud free tier 1-2 años garantizados. Quid pro quo: "trusted by" + logo en talks + co-author blog post. Funnel: 30 emails → 10 calls → 5 partners activos. Éxito: 3+ corriendo `coral mcp serve` persistentemente en CI/IDE.

### 12A.6 Pricing / sustainability

**Modelo recomendado**: **MIT forever (binario) + sponsorware tier "Coral Cloud" (dashboard opcional, post v0.25.0) + comercial-friendly skill bundles de terceros**.

- **Binario MIT forever**: moat son primitivas locales (filesystem+Git+MCP+single-binary). Hosted no agrega defensibility, agrega fricción (lecciones Tailscale, Coder).
- **Sponsorware Coral Cloud**: dashboard `coral.dev` agrega content_hashes + metadata cross-repos (no código). GitHub Sponsors: $50/mes (logo+early skills) + $500/mes team plan (dashboard+priority support). Target 12 meses: 50 sponsors ≈ $40k/año (Caleb Porzio playbook).
- **NO open core comercial pro tier**: rompería "single binary tiene todo lo importante". Sentry funciona pero tiene 50+ engineers; Coral no lo sostiene single-author.
- **NO 100% donations**: pattern Lerna/Babel demostró que matan proyectos cuando author cambia de empleo. Sponsorware crea gradiente de valor sin paywall.
- **Skill bundles comerciales de terceros**: modelo VSCode Extensions Marketplace. Coral no claim ownership.

**Decision rule v1**: en 12 meses post-launch, si sponsors > $30k/año Y ≥ 3 design partners en producción → invertir en Coral Cloud. Si no, mantener OSS-puro y considerar foundation (CNCF Sandbox / Apache Incubator) en mes 18 (§12C).

---

## 12B. Enterprise readiness layer

Para vender a empresas regulated (post-beachhead, target v0.27+), el binario emite **evidence automática de compliance**, sin tooling externo.

### 12B.1 Supply chain (NFR-16)

| Concepto | Implementación | Justificación |
|---|---|---|
| **SBOM CycloneDX 1.6** | `cargo cyclonedx` en `release.yml`, bundle en GitHub Releases | NIST SSDF, EU CRA 2027 |
| **SLSA Build Level 3** | `slsa-framework/slsa-github-generator` Generic Generator reusable workflow | Non-forgeable provenance, cosign-verifiable |
| **Cosign signing** | `cosign sign-blob` binarios+provenance, key en `coral.dev/.well-known/cosign.pub` | Verificable offline |
| **OpenSSF Scorecard** | `ossf/scorecard-action`, target ≥ 7 v0.24 / ≥ 8.5 v0.30 | Branch protection, signed releases, SAST, fuzz |

### 12B.2 Audit logging (NFR-18)

`audit.log.jsonl` append-only con Merkle-style chain (cada línea hash de la anterior, detecta tampering). Cubre **SOC 2 CC7.2** (system monitoring) + **CC8.1** (change mgmt: `actor`, `command`, `git_ref_before/after`) + **GDPR Art. 30** (frontmatter `pii: true` activa redaction). Exportable a syslog/JSONL/OTel.

### 12B.3 Air-gap (NFR-17)

- **Default offline**: `--offline` salta llamadas LLM/embeddings; cae a TF-IDF/BM25.
- **Bundle**: `coral bundle --output coral-airgap.tar.gz` empaqueta binario + SBOM + parsers tree-sitter + fixtures.
- **Mirror docs**: `docs/AIRGAPPED.md`. Binario base sin deps de network.
- **Subprocess detection**: NFR-13 mensajes claros con path offline.

### 12B.4 Compliance posture

| Framework | Cómo ayuda Coral | Evidence automática |
|---|---|---|
| **SOC 2 Type II** | CC7.2/CC8.1/CC1.x | `audit.log.jsonl` + Git history + SBOM + provenance |
| **GDPR** | `pii: true` + `validity_window` + redaction | Audit log redacted, export PII-flagged |
| **HIPAA** | Accountability layer (Coral no procesa PHI) | `audit.log.jsonl` |
| **ISO 27001 A.12.1/A.18.1** | Logging + compliance | OpenSSF Scorecard + SLSA |
| **NIST SSDF PS.3/PW.4** | Provenance + SBOM + dep update | Outputs CI |
| **EU CRA 2027** | SBOM + vuln disclosure | CycloneDX + `coral.dev/security` |

### 12B.5 Lo que Coral NO promete

No SAML/SCIM/SSO en binario (Git provee auth via remote; Coral Cloud sí lo tendrá). No RBAC propio (Git permissions del repo). No secrets management (env vars + `.env`; Vault hook post-v0.30). No FedRAMP/CMMC cert (Coral provee evidence; cert es del deployment cliente).

---

## 12C. Ecosystem & Community strategy

### 12C.1 Plugin/extension points (sin fork del binario)

1. **TestKind plugins out-of-tree**: trait `TestRunner` + bin externo `coral-test-<kind>`. Discovered via `which`. Ejemplos: `coral-test-grpc-stress`, `coral-test-chaos-pumba`, `coral-test-load-k6`.
2. **Storage backends out-of-tree (FR-MOAT-1)**: traits `WikiStorage`/`EmbeddingsStorage`/`IndexStorage`. JSON+SQLite en core; pgvector/Neo4j/Qdrant/Weaviate en crates terceros via `CORAL_STORAGE_PROVIDER`. NO en binario (NFR-2).
3. **Skill bundles (FR-MOAT-7)**: `coral skill install <github-repo>` (feature `skill-marketplace`). Federation-first, no registry centralizado.

### 12C.2 Contributor onboarding

Target: **issue → first PR merged < 1 día** para 50%+ first-timers. Tactics: 10 `good-first-issue` curados con entry-point file + expected diff size; `cargo run --example contribute-walkthrough` que automatiza branch+fixture+tests+PR; pre-commit hooks (fmt, clippy, nextest, bc-regression smoke); mentor pairing para PRs > 50 LOC.

### 12C.3 Sponsor targets

Mes 6: 10 sponsors → ~$5k/año. Mes 12: 50 → ~$40k/año (single-author part-time). Mes 24: 200 sponsors o 5 team plans ($500/mes) → ~$60k/año + Coral Cloud paid tier.

### 12C.4 Governance roadmap

- **Mes 0-12 BDFL** (Agustín Bajo). NFR-7 + §13 son contratos públicos.
- **Mes 12-24 maintainer team (3-5)**: top contributors con merge access, consenso lazy.
- **Mes 24+ foundation candidate** (CNCF Sandbox / Apache Incubator / LF AI&Data). Trigger: ≥ 1 enterprise design partner en producción + ≥ 5 maintainers activos. Premature foundation distrae en single-author phase.

### 12C.5 Vertical skill marketplace

Bundles verticales (FR-MOAT-7) MIT/Apache-2.0: `coral-skill-fintech` (PCI-DSS), `coral-skill-healthtech` (HIPAA + `phi: true` redaction), `coral-skill-infra` (k8s manifests + Helm como contracts), `coral-skill-llm-product` (prompt registry, eval suites, telemetry). Comercial (proprietary): vía `--bundle-source <git-url>`.

---

## 13. Anti-features y diferenciadores intencionales

Esta sección codifica explícitamente las features que Coral **no** va a implementar, aunque competidores las tengan. El objetivo: convertir non-goals en **identidad** y eliminar ambigüedad para contributors externos sobre qué PRs se rechazan.

### 13.1 Anti-features

| # | Anti-feature | Competidor que la tiene | Por qué Coral NO la implementa |
|---|---|---|---|
| AF-1 | **LLM-driven knowledge graph extraction como source-of-truth** | LightRAG, GraphRAG, Cognee | Romperia determinismo (FR-MOAT-5, NFR-4). El wiki Coral es **humano-curado** o promovido por bibliotecario subagent **con SCHEMA**. La diferencia clave: en LightRAG/GraphRAG, **no podés auditar byte-a-byte** por qué un nodo del KG existe. En Coral, cada página tiene un commit Git y un author. |
| AF-2 | **Community detection (Leiden) auto-aplicado** | GraphRAG | Coral puede **sugerir** clusters (`coral consolidate --suggest-topics`), pero nunca los persiste sin aprobación. Razón: clustering automático produce labels semi-arbitrarios que terminan siendo basura técnica difícil de mantener. |
| AF-3 | **Servidor obligatorio (broker, registry, daemon-only)** | Pact Broker, Buf BSR, Apicurio Registry, Sourcegraph Cloud | Single binary + file-based defaults. Cualquier broker es opt-in (`--broker URL`). Coral debe ser **utilizable offline en una laptop** sin servicios externos. |
| AF-4 | **Multimodal completo (imágenes, audio, video)** | RAG-Anything, GraphRAG (con extension), LlamaParse | Es una pelea perdida vs HKUDS RAG-Anything que ya integra MinerU/Docling. Coral **consume** esos vía MCP. Incluso `coral ingest --include-docs` (FR-LRAG-2) es texto-only. |
| AF-5 | **Memoria conversacional / personalización por usuario** | Mem0, Letta, Zep | Coral es memoria del **codebase/producto**, no del **usuario**. Si un team quiere personalización end-user, usa Mem0 + Coral simultáneamente vía MCP. |
| AF-6 | **Embeddings model embebido en el binario** | LangChain (con local models opt-in), llama.cpp wrappers | Coral solo invoca **endpoints** (Voyage, OpenAI, Anthropic) o cae a TF-IDF/BM25 local. Embebir un modelo agregaría ~500 MB-2 GB al binario. |
| AF-7 | **WebUI / dashboard como first-class citizen** | LightRAG Server, Sourcegraph, Mem0 dashboard, Zep dashboard | `coral wiki serve` es opt-in detrás de `--features webui`. El binario base sigue siendo CLI + MCP server. UI no debe ser camino crítico. |
| AF-8 | **Hermetic builds estilo Bazel** | Bazel, Buck2, Pants | Coral confía en el toolchain del repo (Cargo, npm, pip). No reproduce el toolchain. Razón: hermeticidad agrega 10× complejidad y limita el set de proyectos compatibles. |
| AF-9 | **IDE plugin / extensión propia** | Sourcegraph Cody, Continue.dev, Cursor | Coral expone **MCP**. Cualquier IDE que soporte MCP (Claude Code, Cursor, Cline, Continue, Goose, Codex) consume Coral automáticamente. No reinventamos la integración. |
| AF-10 | **Schema-on-write vía LLM (auto-clasificación de docs)** | Cognee ECL pipelines, LightRAG entity extraction | Coral exige frontmatter explícito (`type:`, `status:`, `confidence:`). Si el bibliotecario subagent lo asigna, lo hace con SCHEMA y el resultado queda en `.coral/wiki-suggestions.jsonl` para revisión humana, **no aplicado directamente**. |
| AF-11 | **Telemetría obligatoria con cloud collector** | Sentry-style runtime telemetría | Coral genera logs locales (JSONL); si el usuario quiere telemetría agregada, exporta a su propio observability stack. Cero phone-home por default. |
| AF-12 | **`unsafe` propio para optimizaciones aggressive** | Algunos crates de search high-perf | Tantivy/notify/mimalloc tienen `unsafe` auditado upstream — Coral lo acepta. Pero **el código de Coral mismo** no introduce `unsafe`. Auditable con `cargo geiger`. |
| AF-13 | **Cliente HTTP/RPC propio embebido** | Cody, Continue (cliente de provider) | Coral usa `reqwest` para llamadas LLM/embeddings (ya en el dep tree). Sin protocolos custom propios. |
| AF-14 | **`coral` ejecuta código del proyecto** (build, run, test orquestado por Coral mismo) | Bazel, Nx run-targets | Coral **invoca** el toolchain del proyecto (`cargo test`, `npm run build`) pero no lo reemplaza. `coral test` corre tests Coral-funcionales (Hurl, OpenAPI fuzzing, Pact, etc.), no `cargo test`. |

### 13.2 Cómo se enforce-an estas anti-features

- **PR template** incluye checklist: "¿este PR introduce alguna anti-feature de §13? Si sí, no se merge."
- **`docs/CONTRIBUTING.md`** documenta cada anti-feature con ejemplos de PRs rechazados.
- **CI gate**: `cargo geiger` falla en PRs que agregan `unsafe` propio (AF-12). `cargo bloat` falla si binario crece > 10% (NFR-2). Test de smoke valida que `coral mcp serve` arranca sin features opt-in.

### 13.3 Lo que Coral promete a sus competidores

- Coral **no** quiere reemplazarte si vos sos **LightRAG/GraphRAG/Cognee** — querés ese caso (multimodal, KG generalista). Te consumimos vía MCP.
- Coral **no** quiere reemplazarte si vos sos **Sourcegraph/Cursor/Cody** — sos el IDE/code-search. Coral te alimenta de contexto vía MCP.
- Coral **no** quiere reemplazarte si vos sos **Mem0/Zep/Letta** — sos memoria de usuario. Coral es memoria del producto.
- Coral **sí** quiere reemplazarte si vos sos **un wiki abandonado en Confluence/Notion/Obsidian sin agente** y un equipo sin garantía verde de que sus contratos están vivos.

---

## 14. Apéndice de research (2026-05-09)

URLs consultadas durante la investigación de v1.1, agrupadas por tema. Cada una se cita en el FR/NFR/sección que la usa.

### 14.1 LightRAG (competidor primario)

| URL | Aporta |
|---|---|
| [arXiv:2410.05779v3](https://arxiv.org/html/2410.05779v3) | Paper completo: dual-level retrieval con keyword extraction prompts, evaluación UltraDomain (agriculture/CS/legal/mix), métricas (Comprehensiveness/Diversity/Empowerment/Overall), incremental update via set union, win rates 67-85% vs NaiveRAG. Citado en FR-RAG-3, FR-LRAG-1, BENCH-1. |
| [github.com/HKUDS/LightRAG](https://github.com/HKUDS/LightRAG) | Storage backends (NetworkX, Neo4j, PostgreSQL, MongoDB, JSON, OpenSearch — multi-backend pattern). Server WebUI con grafo. RAG-Anything integration. Citado en FR-MOAT-1, AF-1, AF-4. |
| [github.com/HKUDS/RAG-Anything](https://github.com/HKUDS/RAG-Anything) | Multimodal pipeline (MinerU/Docling, multimodal KG). Citado en AF-4, R13. |

### 14.2 GraphRAG y community detection

| URL | Aporta |
|---|---|
| [microsoft.github.io/graphrag/index/default_dataflow/](https://microsoft.github.io/graphrag/index/default_dataflow/) | Dataflow + community detection con Leiden hierarchical. Citado en AF-2. |
| [arXiv:2404.16130v2](https://arxiv.org/html/2404.16130v2) | Original GraphRAG paper "From Local to Global" — 50-70% improvement over vector RAG en preguntas globales. |
| [github.com/microsoft/graphrag/discussions/1128](https://github.com/microsoft/graphrag/discussions/1128) | Hierarchical levels en community detection con Leiden. |
| [crates.io/crates/fa-leiden-cd](https://crates.io/crates/fa-leiden-cd) | Implementación Rust de Leiden con minimal deps — referencia para `coral consolidate --suggest-topics` opt-in. |

### 14.3 Memory frameworks (Mem0, Letta, Zep, Cognee)

| URL | Aporta |
|---|---|
| [arXiv:2501.13956](https://arxiv.org/abs/2501.13956) | Zep paper: temporal knowledge graph con bi-temporal (event time T + ingestion time T'). DMR benchmark 94.8% vs MemGPT 93.4%. Citado en FR-MOAT-3, AF-5. |
| [github.com/getzep/graphiti](https://github.com/getzep/graphiti) | Implementación de Graphiti — referencia para bi-temporal awareness sin Neo4j. |
| [cognee.ai/blog/deep-dives/ontology-ai-memory](https://www.cognee.ai/blog/deep-dives/ontology-ai-memory) | Cognee ontology RDF approach. Citado en AF-1, AF-10. |
| [letta.com/blog/benchmarking-ai-agent-memory](https://www.letta.com/blog/benchmarking-ai-agent-memory) | Comparación memoria vs filesystem. |

### 14.4 Coding agents (Sourcegraph, Continue, Cursor, Aider, Codex)

| URL | Aporta |
|---|---|
| [sourcegraph.com/blog/how-cody-provides-remote-repository-context](https://sourcegraph.com/blog/how-cody-provides-remote-repository-context) | Cody multi-repo context (3 layers: local file, local repo, remote repo). 250k repos, 10M LOC scale. Citado en R12. |
| [aider.chat/docs/repomap.html](https://aider.chat/docs/repomap.html) | Aider repo map con tree-sitter + graph ranking. Citado en FR-MOAT-2, BENCH-10. |
| [aider.chat/2023/10/22/repomap.html](https://aider.chat/2023/10/22/repomap.html) | Detalle del algoritmo de ranking. |
| [docs.continue.dev/guides/codebase-documentation-awareness](https://docs.continue.dev/guides/codebase-documentation-awareness) | Continue context retrieval, MCP integration. |
| [cursor.com/blog/2-0](https://cursor.com/blog/2-0) | Cursor 2.0 + Composer multi-agent, git worktrees. Citado en R12. |
| [openai.com/index/introducing-codex/](https://openai.com/index/introducing-codex/) | Codex SWE-bench Verified ~80%. |

### 14.5 Multi-repo orchestration (Nx, Bazel, Turborepo, Moon)

| URL | Aporta |
|---|---|
| [nx.dev/docs/features/ci-features/affected](https://nx.dev/docs/features/ci-features/affected) | Nx affected: file-system + project graph + git history. Citado en FR-TEST-3, BENCH-5. |
| [nx.dev/docs/concepts/mental-model](https://nx.dev/docs/concepts/mental-model) | Nx project graph mental model. |
| [bazel.build/query/language](https://bazel.build/query/language) | Bazel query language for cross-repo deps. |
| [turborepo.dev/docs/core-concepts/remote-caching](https://turborepo.dev/docs/core-concepts/remote-caching) | Turborepo content-aware hashing. Citado en FR-TEST-3. |
| [moonrepo.dev/moon](https://moonrepo.dev/moon) | Moon polyglot project graph. |

### 14.6 Contract testing & API governance

| URL | Aporta |
|---|---|
| [docs.pact.io/pact_broker/can_i_deploy](https://docs.pact.io/pact_broker/can_i_deploy) | Pact `can-i-deploy` semantics (matrix of consumer×provider versions). Citado en FR-TEST-1, UC8. |
| [github.com/pact-foundation/pact_broker](https://github.com/pact-foundation/pact_broker) | Implementación Pact Broker. |
| [schemathesis.io/](https://schemathesis.io/) | Property-based fuzzing OpenAPI. Citado en relación con FR-PROP existente. |
| [github.com/schemathesis/schemathesis](https://github.com/schemathesis/schemathesis) | Stateful testing details. |
| [github.com/oasdiff/oasdiff](https://github.com/oasdiff/oasdiff) | 450+ breaking change rules. Citado en FR-IFACE-6. |
| [stoplight.io/open-source/spectral](https://stoplight.io/open-source/spectral) | Spectral rules. Citado en FR-MOAT-8. |
| [openapis.org/arazzo-specification](https://www.openapis.org/arazzo-specification) | Arazzo workflow spec — futuro para FR-IFACE extensions post-v0.30. |
| [apicur.io/registry](https://www.apicur.io/registry/docs/apicurio-registry/3.0.x/getting-started/assembly-intro-to-the-registry.html) | Apicurio multi-format support. Citado en FR-MOAT-4. |
| [buf.build/docs/breaking/](https://buf.build/docs/breaking/) | 53 breaking change rules para protobuf. Citado en FR-IFACE-6. |
| [microcks.io](https://microcks.io/) | Microcks contract testing CNCF. Inspiración para `coral test guarantee` infrastructure. |

### 14.7 MCP spec 2025-11-25

| URL | Aporta |
|---|---|
| [modelcontextprotocol.io/specification/2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25) | Spec actual: elicitation, sampling, resource subscriptions, async tasks. Citado en FR-IFACE-7, FR-AGENT-4, NFR-12, R11. |
| [workos.com/blog/mcp-2025-11-25-spec-update](https://workos.com/blog/mcp-2025-11-25-spec-update) | Highlights del bump 2025-11-25. |
| [pulsemcp.com/posts/mcp-client-capabilities-gap](https://www.pulsemcp.com/posts/mcp-client-capabilities-gap) | Gap entre spec y client adoption — justifica fallback `pending_drifts` (FR-IFACE-9). Citado en R17. |
| [github.com/orgs/modelcontextprotocol/discussions/391](https://github.com/orgs/modelcontextprotocol/discussions/391) | Discusión sobre push notifications. |

### 14.8 RAG techniques 2024-2025

| URL | Aporta |
|---|---|
| [arxiv.org/abs/2409.04701](https://arxiv.org/abs/2409.04701) | Late Chunking paper (Jina). Citado en FR-RAG-2. |
| [jina.ai/news/late-chunking-in-long-context-embedding-models/](https://jina.ai/news/late-chunking-in-long-context-embedding-models/) | Late Chunking implementación. |
| [arxiv.org/abs/2401.18059](https://arxiv.org/abs/2401.18059) | RAPTOR hierarchical clustering — referencia para FR-LRAG-1 (expand-graph). |
| [arxiv.org/abs/2401.15884](https://arxiv.org/abs/2401.15884) | CRAG (corrective RAG). Citado en FR-RAG-4. |
| [glaforge.dev/posts/2026/02/10/](https://glaforge.dev/posts/2026/02/10/advanced-rag-understanding-reciprocal-rank-fusion-in-hybrid-search/) | RRF deep dive. Citado en FR-RAG-1. |
| [opensearch.org/blog/introducing-reciprocal-rank-fusion-hybrid-search/](https://opensearch.org/blog/introducing-reciprocal-rank-fusion-hybrid-search/) | RRF en OpenSearch 2.19. Citado en BENCH-1. |
| [docs.haystack.deepset.ai/docs/hypothetical-document-embeddings-hyde](https://docs.haystack.deepset.ai/docs/hypothetical-document-embeddings-hyde) | HyDE en Haystack. Citado en FR-RAG-8. |
| [blog.voyageai.com/2025/08/11/rerank-2-5/](https://blog.voyageai.com/2025/08/11/rerank-2-5/) | Voyage rerank-2.5 instruction-following. Citado en FR-RAG-5. |
| [docs.ragas.io/en/stable/concepts/metrics/available_metrics/](https://docs.ragas.io/en/stable/concepts/metrics/available_metrics/) | RAGAs metrics. Citado en FR-RAG-6. |
| [weaviate.io/blog/late-interaction-overview](https://weaviate.io/blog/late-interaction-overview) | ColBERT/ColPali — referencia futura post-v0.30. |

### 14.9 Performance — Rust-specific

| URL | Aporta |
|---|---|
| [github.com/quickwit-oss/tantivy](https://github.com/quickwit-oss/tantivy) | Tantivy BM25 ~6× más rápido vs Lucene. Citado en FR-RAG-7. |
| [docs.rs/tantivy](https://docs.rs/tantivy) | API de Tantivy. |
| [crates.io/crates/bm25](https://crates.io/crates/bm25) | bm25 crate (BM25 embedder + scorer + search engine). |
| [github.com/lightonai/bm25x](https://github.com/lightonai/bm25x) | bm25x — streaming/mmap variant. |
| [github.com/rkyv/rkyv](https://github.com/rkyv/rkyv) | rkyv zero-copy. Citado en M3.1. |
| [david.kolo.ski/blog/rkyv-is-faster-than/](https://david.kolo.ski/blog/rkyv-is-faster-than/) | rkyv vs bincode/postcard/capnp/flatbuffers benchmarks. |
| [github.com/microsoft/mimalloc](https://github.com/microsoft/mimalloc) | mimalloc. Citado en FR-PERF-7, M1.3. |
| [github.com/notify-rs/notify](https://github.com/notify-rs/notify) | notify cross-platform. Citado en M2.3, M2.4. |
| [crates.io/crates/tree-sitter-language-pack](https://crates.io/crates/tree-sitter-language-pack) | 305 parsers on-demand. Citado en FR-MOAT-2. |
| [docs.rs/tree-sitter](https://docs.rs/tree-sitter) | API tree-sitter Rust. |

### 14.10 Testing best practices 2025

| URL | Aporta |
|---|---|
| [mutants.rs/](https://mutants.rs/) | cargo-mutants. Citado en FR-TEST-11, M3.3. |
| [github.com/sourcefrog/cargo-mutants](https://github.com/sourcefrog/cargo-mutants) | --shard, --re, --file filters. |
| [stryker-mutator.io/](https://stryker-mutator.io/) | Stryker JS/TS. Citado en FR-TEST-11. |
| [dagger.io/](https://dagger.io/) | Dagger programmable CI. Inspiración para `coral test guarantee` orchestration. |
| [testcontainers.com/modules/kafka/](https://testcontainers.com/modules/kafka/) | Kafka module. Citado en M2.6. |
| [github.com/testcontainers/testcontainers-rs](https://github.com/testcontainers/testcontainers-rs) | testcontainers-rs API para FR-TEST-9. |

### 14.11 AI-era developer experience

| URL | Aporta |
|---|---|
| [www.anthropic.com/engineering/effective-context-engineering-for-ai-agents](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents) | Context engineering, structured note-taking, memory tool. Citado en FR-AGENT-2, FR-AGENT-3. |
| [platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool) | Memory tool API. Citado en FR-AGENT-3. |
| [latent.space/p/s3](https://www.latent.space/p/s3) | Karpathy Software 3.0 talk transcript. Citado en FR-AGENT-1, en intro de §14.11. |
| [block.xyz/inside/block-open-source-introduces-codename-goose](https://block.xyz/inside/block-open-source-introduces-codename-goose) | Goose agent loop (Rust + MCP). Inspiración para FR-MOAT-7 (multi-agent compatibility). |
| [github.com/aaif-goose/goose](https://github.com/aaif-goose/goose) | Goose codebase. |

### 14.12 Tabla feature → fuente → adoptado en

| Feature | Fuente principal | Adoptado en |
|---|---|---|
| Dual-level retrieval | LightRAG arxiv 2410.05779 | FR-RAG-3 (heurística determinística, no LLM call) |
| Reciprocal Rank Fusion | Cormack 2009; OpenSearch 2.19 | FR-RAG-1, M1.10 |
| Late Chunking | Jina 2409.04701 | FR-RAG-2, M3.11 |
| HyDE | Haystack docs | FR-RAG-8, M3.13 |
| CRAG | arxiv 2401.15884 | FR-RAG-4, M3.12 |
| RAGAs metrics | docs.ragas.io | FR-RAG-6, M2.13, NFR-11 |
| Voyage rerank-2.5 | blog.voyageai.com | FR-RAG-5, M3.15 |
| Pact `can-i-deploy` | docs.pact.io | FR-TEST-1, UC8 |
| Schemathesis property-based | schemathesis.io | (ya existente FR-PROP) |
| oasdiff 450+ rules | github.com/oasdiff | FR-IFACE-6 |
| buf breaking 53 rules | buf.build/docs/breaking | FR-IFACE-6 |
| Apicurio multi-format | apicur.io/registry | FR-MOAT-4 |
| Spectral custom rules | stoplight.io/spectral | FR-MOAT-8, M2.14 |
| Aider repo map (tree-sitter) | aider.chat/docs/repomap | FR-MOAT-2, M2.12 |
| Sourcegraph multi-repo (3 layers) | sourcegraph.com/blog | (ya existente, refina §2A) |
| Cursor 2.0 multi-agent (worktrees) | cursor.com/blog/2-0 | R12 risk awareness only |
| MCP 2025-11-25 subscriptions | modelcontextprotocol.io | FR-IFACE-7, FR-IFACE-9, M1.12 |
| MCP sampling | modelcontextprotocol.io | FR-MOAT-6, FR-AGENT-5 |
| MCP Tasks (experimental) | modelcontextprotocol.io | FR-AGENT-4, M3.17 |
| Anthropic Memory tool | platform.claude.com/.../memory-tool | FR-AGENT-3 |
| Anthropic context engineering | anthropic.com/engineering | FR-AGENT-2 |
| Karpathy Software 3.0 / llms.txt | latent.space/p/s3 | FR-AGENT-1, M2.15 |
| Storage abstraction (4 backends) | LightRAG | FR-MOAT-1, M1.11 (limit a 2 backends Coral, no 4) |
| Bi-temporal awareness | Zep arxiv 2501.13956 | FR-MOAT-3, M2.16 (sin Neo4j) |
| Nx affected (content-hash) | nx.dev/.../affected | FR-TEST-3 (refinamiento) |
| Turborepo content-aware hashing | turborepo.dev/.../caching | FR-TEST-3 |
| cargo-mutants | mutants.rs | FR-TEST-11, M3.3, M3.14 |
| Stryker | stryker-mutator.io | FR-TEST-11 |
| tantivy | github.com/quickwit-oss/tantivy | FR-RAG-7, M3.10 (opt-in only) |
| rkyv | github.com/rkyv/rkyv | M3.1 |
| notify | github.com/notify-rs/notify | M2.3, M2.4 |
| tree-sitter-language-pack | crates.io/.../tree-sitter-language-pack | FR-MOAT-2, M2.12 |
| Dagger.io programmable CI | dagger.io | inspiración orchestration `coral test guarantee` (no dep) |
| Goose Rust agent loop | block.xyz/.../goose | FR-MOAT-7 multi-agent compatibility |
| Microcks AsyncAPI/Kafka | microcks.io | inspiración FR-TEST-9 (no dep) |

---

## 15. Demo scripts — wedge 90s + conf talk 5min + self-guided

Repo fixture: `coral-cli/coral-demo-multirepo` con 3 servicios (`api`, `worker`, `billing`) + OpenAPI cross-refs.

### 15.1 Demo A — Wedge 90s (asset central)

```text
[0:00–0:10] Split: api/ con http.openapi.yaml abierto + terminal.
            Voz: "Antes de cada deploy, ¿cómo sabés que tu producto está verde?"
            $ coral test guarantee --can-i-deploy production
            Output streaming: ✅ verify, ✅ contract check, ⏳ smoke (3/47)...
[0:10–0:30] Edit en vivo: agregar campo required `email` al body de POST /users. Save.
[0:30–0:45] Foco en worker/ + Claude Code (sin recargar). Chat: "actualizá cliente POST /users".
            Claude Code: "BREAKING change detectado via coral://contract-drift/latest:
            required field `email` added. Affects worker, billing. Diff: ..."
            Voz: "Agente recibió drift en < 1s. Cero polling."
[0:45–1:15] Terminal: guarantee terminó. ❌ ROJO — "POST /users body schema breaking,
            consumer worker not yet updated". Voz: "Cruza contracts + tests + coverage.
            Verde solo si todos los pares verificaron. HTTP+gRPC+AsyncAPI+SQL. Local."
[1:15–1:30] Slide final: "coral test guarantee --can-i-deploy <env> | brew install coral | coral.dev"
```

Viralidad: (a) problema real multi-repo, (b) wow moment claro (agente ve drift solo), (c) CTA concreto, (d) 90s = nativo X/LinkedIn/HN.

### 15.2 Demo B — Conf talk 5min (AI Eng Summit / RustConf / KubeCon / MCP Dev Summit)

- **0:00–0:30 Hook**: "Quién acá tuvo incidente por cambio en repo A que rompió B?" Polling.
- **0:30–1:00 Problem**: 3 servicios + 2 contracts + 1 PR = 6 estados. Pact (solo HTTP), Nx (solo tests), Cursor (solo código). Nadie cierra el loop.
- **1:00–2:30 Demo A inline** sobre slides.
- **2:30–3:30 Under the hood**: MCP subscriptions 2025-11-25, content-hash affected (Nx-style), `can-i-deploy` Pact-style file-based, frontmatter SCHEMA + bibliotecario.
- **3:30–4:30 TTTC**: gráfico P95 v0.23→v0.30. "Métrica que define si un agente tiene contexto correcto cross-repo."
- **4:30–5:00 CTA**: brew install + GitHub + "design partners apertura". QR.

### 15.3 Demo C — Self-guided (`coral-demo-multirepo` README)

```text
1. git clone github.com/coral-cli/coral-demo-multirepo && ./setup.sh
2. coral mcp preview                        # resources/tools sin IDE
3. coral test guarantee --can-i-deploy demo # verde
4. ./break-it.sh && coral test guarantee --can-i-deploy demo  # rojo, razón clara
5. coral wiki at HEAD~3 cat AuthService     # time-travel
6. (Opt) coral mcp install --client claude-code → pedir "actualizá cliente POST /users"
```

### 15.4 Conversion targets

Demo A → 10% trial install en 7 días. Demo B → 5% design-partner inquiry post-talk. Demo C → 20% cloners corren ≥3 comandos. Telemetry opt-in via `CORAL_TELEMETRY=1` (default off).

---

**Fin del documento.**
