# PRD — Coral v0.34: Zero-Friction Onboarding via Claude Code

**Versión del documento:** 1.2 (post second-pass review independiente)
**Fecha:** 2026-05-12
**Autor:** Agustín Bajo
**Estado:** Borrador validado, 2 iteraciones
**Versiones objetivo:** Coral v0.34.0 (M1, 6–8 semanas) → v0.34.x patches → v0.35.0 (M2 marketplace polish + daemon + wizard + i18n)

**Cambios v1.1 → v1.2** (second-pass review, todos verificados contra docs oficiales de Claude Code):

*Críticos:*
1. **Re-arquitectura del "primer-prompt-en-blanco"**. Verificado en `code.claude.com/docs/en/hooks.md`: `SessionStart` hook stdout se inyecta como **contexto silencioso**, no produce mensaje proactivo del asistente. No existe ningún mecanismo documentado para que un plugin haga hablar a Claude antes del primer prompt del user. Solución: combo `SessionStart` hook (estado dinámico) + **`CLAUDE.md` template** (instrucciones de routing estáticas, provisto por `coral init`). Cualquier prompt del user dispara la respuesta correcta de Claude. Ver §3.3 y FR-ONB-25.
2. **`--with-claude-config` opt-in committed**. Decisión #6 (era "tentativa sí") ahora es "committed sí". `install.sh --with-claude-config` parchea `.claude/settings.json` del proyecto con `extraKnownMarketplaces` (patch idempotente + backup atómico). Baja onboarding de 6 actos a 2. Default off por security. Ver FR-ONB-26.
3. **Provider mini-wizard para users sin `claude` CLI**. `coral-doctor` ofrece 4 paths (Anthropic API key directa, Gemini, Ollama local, install claude CLI) cuando no detecta provider configurado. Sin esto, users que descubren Coral antes de tener Claude Code quedan varados. Ver FR-ONB-27, FR-ONB-28.

*Medianos:*
4. `coral bootstrap --max-cost=USD` con abort mid-flight. Cost **upper-bound** mostrado en estimate. FR-ONB-29.
5. `coral bootstrap --resume` movido de M2 a M1. Trust-killer si bootstrap falla a mitad y user pierde $0.30. FR-ONB-30.
6. `SessionStart` hook budget **<100ms documentado** + early-exit si `coral` no en PATH (<10ms). FR-ONB-9.
7. Windows specifics: Defender SmartScreen, PATH refresh, WSL detection. FR-ONB-31.
8. `coral self-upgrade` + `update_available` en self-check JSON. FR-ONB-32.
9. Repos grandes (estimate > $5): mensaje específico con `--max-pages=N --priority=high` hint. FR-ONB-12.
10. `coral self-uninstall` para confidence-to-try. FR-ONB-33.

*Huecos cerrados:*
11. §3.4 nuevo: análisis competitivo (rustup, Stripe CLI, Vercel CLI, Supabase CLI, Prisma).
12. AF-8 nuevo: M1 EN-only en scripts/CLI messages; i18n a M2.
13. §11 decisión #3 ampliada: calibración `--estimate` con **opt-in manual** `coral feedback submit` (no telemetry, no auto-send).
14. WebUI empty-state coaching diferido explícitamente a M2 (FR-ONB-24).

**Predecesor:** [PRD-v0.32-webui.md](PRD-v0.32-webui.md) — la WebUI existe, ahora hay que llevar a usuarios hacia ella sin fricción.

Changelog v1.0 → v1.1 (first-pass review) movido a Apéndice D.

---

## 1. Resumen ejecutivo

Coral v0.33.0 ya tiene **todo el producto**: binario único, 4 binarios cross-platform, plugin Claude Code con 4 skills + 2 slash commands, WebUI, REST API, MCP server, contract checking, multi-repo. Lo que falta es **el camino del usuario**: hoy, un desarrollador nuevo necesita **6 actos desacoplados** (instalar binary fuera de Claude Code, abrir Claude Code, marketplace add, plugin install, reload, primer prompt) — con **10 fricciones documentadas** (F1–F10) entre las que sobresalen: el binario no se auto-instala, no hay detección automática de "este repo no tiene wiki", el costo LLM del bootstrap es opaco, users sin `claude` CLI quedan varados.

Este PRD lleva el onboarding a **2 actos en el happy path con `--with-claude-config`**:

```bash
# Acto 1: el usuario corre
curl -fsSL https://coral.dev/install | bash -s -- --with-claude-config

# Acto 2: abre Claude Code en su repo y escribe CUALQUIER COSA
#   ("hola", "qué es este repo", "help", "/coral:coral-bootstrap"...)
#
# Claude lee CLAUDE.md (provisto por el plugin) + el SessionStart hook
# context, y responde proponiendo el siguiente paso correcto.
```

Sin `--with-claude-config` (default por security): **3 actos** (install + paste 3 lines + cualquier prompt).

**Wedge** (la única razón que justifica este sprint): **time-to-first-wiki-query ≤ 10 minutos** sobre un repo desconocido, sin que el usuario abra documentación. Hoy ese tiempo es **20–40 minutos**.

**Insight clave verificado** en `code.claude.com/docs/en/hooks.md`: Claude Code **no tiene mecanismo** para que un plugin haga hablar al asistente proactivamente al abrir la sesión. El `SessionStart` hook solo inyecta contexto silencioso. La única forma de "Claude conduce desde el primer prompt" es `CLAUDE.md` en el repo + cualquier input del user dispara la respuesta correcta vía routing instructions documentadas allí.

**Cuatro principios no negociables:**

1. **Plug-and-play máximo del install.** Un solo `curl | bash`. El `--with-claude-config` opt-in (parchea `.claude/settings.json` con `extraKnownMarketplaces` + backup atómico) baja a 2 actos. Default sin esto: 3 actos.
2. **Skills detectan, no asumen.** Cada skill chequea su precondición antes de ejecutar (¿coral en PATH? ¿claude CLI? ¿git repo? ¿wiki? ¿coral.toml? ¿provider configurado?). Cuando falta algo, ofrece el comando exacto o un mini-wizard interactivo.
3. **Cost transparency con techo duro.** Cualquier comando que gaste tokens muestra el costo estimado **con upper-bound** antes de correr; el flag `--max-cost=USD` aborta mid-flight si se excede.
4. **Loop cerrado y recoverable.** El último paso deja al usuario con (a) wiki bootstrapeada con checkpoints (`--resume` si falla), (b) WebUI corriendo, (c) MCP server registrado, (d) primer query funcionando.

---

## 2. Contexto y problemas (F1–F10)

### 2.1 Estado actual del flujo `happy path`

El usuario debe ejecutar manualmente, en este orden:

| # | Acción del usuario | Falla si... | Mensaje de error |
|---|---|---|---|
| 1 | `curl install.sh \| bash` (fuera de Claude Code) | sin `curl`, sin `shasum`, sin permisos | "command not found" — falla mute |
| 2 | Abrir Claude Code en el repo | sin Claude Code instalado | obvio pero no documentado |
| 3 | `/plugin marketplace add agustincbajo/Coral` | sin internet, marketplace privada | error genérico Claude Code |
| 4 | `/plugin install coral@coral` | binario `coral` no en PATH | **silent fail** — plugin instala pero MCP falla; user ve "Errors" tab vacío |
| 5 | `/reload-plugins` | — | — |
| 6 | Pedir "set up Coral for this repo" (texto natural) | el trigger no matchea exactamente | usuario no sabe qué decir |
| 7 | Skill `coral-bootstrap` ejecuta `coral init` | no es `.git` repo | ya hace check, OK |
| 8 | Skill pausa para confirmar costo LLM | usuario no entiende qué va a costar | estimado es **rango** ($0.02–$5), no número |
| 9 | `coral bootstrap --apply` | sin `claude` CLI en PATH | mensaje claro, OK |
| 10 | Esperar 30s–5min según tamaño del repo | — | — |
| 11 | Pedir "show me the architecture" | — | — |
| 12 | Skill `coral-ui` ejecuta `coral ui serve` (foreground) | el usuario tiene que esperar; si Ctrl-C, muere | aceptable |
| 13 | Browser abre `localhost:3838` | — | — |

**Pasos manuales hoy: 6 (1, 2, 3, 4, 5, 6)** entre el descubrimiento del producto y el primer prompt que da respuesta útil. **Friction points internos: 6 (F1–F6 abajo)**.

> **v1.2 target**: 6 actos → **2 con `--with-claude-config`** (install + cualquier prompt) o **3 sin él** (install + paste-3-lines + cualquier prompt).

### 2.2 Fricciones detectadas (F1–F10)

| # | Síntoma | Evidencia | Severidad |
|---|---|---|---|
| **F1** | El binario `coral` se instala fuera de Claude Code; el plugin falla silenciosamente si no está en PATH | El `mcpServers` block de `plugin.json` invoca `coral` y crashea con error genérico cuando no existe | 🔴 alta |
| **F2** | `plugin.json` declara v0.32.3, `marketplace.json` declara v0.30.0 — desincronización | Comparar headers — el release.yml no toca `.claude-plugin/`, solo `Cargo.toml` | 🟡 media |
| **F3** | `install.ps1` modifica `HKCU\Environment\Path` pero shells abiertas no lo ven hasta nueva sesión | Behavior estándar de Windows, pero el script no avisa al usuario | 🟡 media |
| **F4** | No hay detección automática de "este repo no tiene `.wiki/`" cuando el usuario abre Claude Code | Las skills tienen triggers basados en NLP de la pregunta del usuario; nada inspecciona el repo al entrar | 🔴 alta |
| **F5** | El costo LLM del bootstrap es opaco — rango `$0.02–$5` sin determinar antes de pagar | `coral bootstrap --dry-run` muestra la list de páginas pero no estima tokens | 🟡 media |
| **F6** | Multi-repo (`coral.toml`) no se genera con wizard — el usuario edita TOML a mano | `coral project new` es CLI no-interactivo; pregunta valores en flags | 🟡 media |
| **F7** | `coral ui serve` es foreground; Ctrl-C lo mata, no hay daemon mode | Asumido del diseño de `tiny_http` | 🟢 baja (aceptable) |
| **F8** | Multi-agent (Claude Code + Cursor + Continue) cada uno arranca su propio `coral mcp serve` stdio | stdio transport no es compartible; cada cliente abre el suyo | 🟢 baja |
| **F9** | `.coral/config.toml` no se crea por `coral init`; el usuario debe escribirlo si quiere provider ≠ Claude | `coral init` solo crea `.wiki/`; el config está documentado pero no scaffold | 🟡 media |
| **F10** | Sin guía post-bootstrap: el usuario no sabe que debe agregar `coral ingest --apply` a su loop de desarrollo | El skill termina con "wiki ready"; no menciona cron, git hook, CI integration | 🟡 media |

### 2.3 Por qué Anthropic-style plugins fallan suave

El [doc oficial](https://code.claude.com/docs/en/discover-plugins) lista **11 LSP plugins** (clangd, gopls, pyright, rust-analyzer, etc.) que siguen exactamente el mismo patrón: plugin + binary externo. La doc admite:

> *"If you see `Executable not found in $PATH` in the `/plugin` Errors tab after installing a plugin, install the required binary from the table above."*

Es decir, **el estándar de la industria es: "el plugin falla y muestra un error en la tab Errors"**. Coral puede hacerlo mejor:

1. **Skill `coral-doctor` (nueva)** que se auto-invoca cuando el plugin reporta MCP error → diagnostica y propone fix exacto.
2. **`scripts/install.sh --with-claude-config`** parchea `.claude/settings.json` con `extraKnownMarketplaces`, dejando la marketplace pre-registrada.
3. **`coral self-check`** comando nuevo del binario que verifica todo el entorno y reporta JSON estructurado (para el skill).

---

## 3. Posicionamiento vs alternativas

| Approach | Pros | Cons | Coral elige |
|---|---|---|---|
| **A. "App store style"** — usuario descarga marketplace, click "install", todo se hace solo | UX óptimo, modelo conocido | **Anthropic no soporta auto-binary-install**; el plugin no puede ejecutar arbitrary code en install | ❌ |
| **B. Status quo extendido** — README más claro + mejores mensajes de error | Cero infra nueva | No cierra la fricción real (F1 sigue) | ❌ |
| **C. Hybrid: install.sh hace todo (con opt-in claude-config), skills detectan estado + CLAUDE.md rutea, doctor cierra el loop** | 2-3 actos + Claude guía después | Más complejidad inicial en `install.sh` y en routing | ✅ |
| **D. Coral Cloud** — instalar nada localmente, todo en SaaS | Cero fricción local | Rompe el anti-feature §13 del PRD anterior ("no SaaS multi-tenant"); cambia el producto | ❌ |

**Decisión: opción C.** Approach híbrido — script potente + skills inteligentes + CLAUDE.md routing + doctor command.

### 3.1 Reusar lo que Anthropic ya construyó

| Capacidad de Claude Code | Usamos para |
|---|---|
| `extraKnownMarketplaces` en `.claude/settings.json` | **CRÍTICO**: `install.sh --with-claude-config` lo escribe → repo recién instalado tiene marketplace auto-registrada |
| Plugin auto-update | Plugin se mantiene sincronizado con el binario via release.yml |
| MCP server `env` block | Inyectar `RUST_LOG`, `CORAL_PROVIDER`, etc. sin que el user los configure |
| Skill auto-invocation por NLP | Mantenemos las 4 skills + agregamos 1 (`coral-doctor`) |
| **`CLAUDE.md` en repo root** | **CRÍTICO**: el único mecanismo documentado para que Claude "sepa cómo responder" antes del primer prompt. El plugin lo provee via `coral init` template. |
| `/reload-plugins` post-install | Documentado para auto-correr en el doctor flow |
| `disable-model-invocation: true` en slash commands | Slash commands deterministicos, sin gasto LLM |

### 3.2 Primitives de Claude Code que usamos / no usamos

| Primitive | Documentación | Lo usamos para |
|---|---|---|
| **`SessionStart` hook** | [hooks reference](https://code.claude.com/docs/en/hooks) — *"stdout is added as context that Claude can see and act on"* (silencioso, no produce mensaje proactivo) | Reportar estado **dinámico** (wiki_present, warnings, providers_configured) que Claude lee al responder al primer prompt del user |
| **`CLAUDE.md` en repo** | [memory docs](https://code.claude.com/docs/en/memory) — cargado automáticamente al abrir el repo | Llevar las **instrucciones estáticas de routing** ("si user dice X, sugerí Y"). `coral init` lo crea/append-safe. |
| **`UserPromptSubmit` hook** | [hooks reference](https://code.claude.com/docs/en/hooks) | NO en M1 (lo que necesitábamos lo cubre SessionStart + CLAUDE.md) |
| **`PreToolUse` hook** | idem | NO en M1 (no bloqueamos Bash calls) |
| `mcpServers` block en plugin.json | reference + Coral hoy | Registramos `coral mcp serve --transport stdio` (sin cambios) |
| Skills auto-invocables (NLP triggers) | reference | Las 4 existentes + 1 nueva (`coral-doctor`) |
| Slash commands con `disable-model-invocation` | reference | `/coral:coral-doctor` (nuevo) determinístico |
| `extraKnownMarketplaces` en `.claude/settings.json` proyecto-scope | [settings docs](https://code.claude.com/docs/en/settings) — confirmado verbatim | `install.sh --with-claude-config` lo escribe; también en el propio repo Coral para dogfooding |
| `${CLAUDE_PLUGIN_ROOT}` + `${CLAUDE_PROJECT_DIR}` env vars en hooks | reference | Path resolution determinístico en `SessionStart` hook script |

| Primitive que NO existe (verificado en docs) | Workaround |
|---|---|
| **Auto-install de binario desde el plugin** | `install.sh` separado (estándar industria — todos los LSP plugins lo hacen así) |
| **Plugin que hace hablar a Claude antes del primer prompt** | `CLAUDE.md` + cualquier prompt del user dispara la respuesta routeada (verificado v1.2) |
| **Chain de slash commands con `&&`** | `install.sh` imprime 3 líneas, user las pega de a una. Con `--with-claude-config` se evita por completo. |
| **`installCommand` field en `marketplace.json`** | El campo no existe en el schema; el README + `description` contienen las instrucciones |
| **CLI externa `claude plugin install --headless`** | No documentada; investigar para M2 si Anthropic la publica |

### 3.3 Por qué el combo `CLAUDE.md` + `SessionStart` hook resuelve el "primer-prompt-vacío"

**Hallazgo crítico del second-pass review** (verificado en `code.claude.com/docs/en/hooks.md`):

> *"For most events, stdout is written to the debug log but not shown in the transcript. The exceptions are `UserPromptSubmit`, `UserPromptExpansion`, and `SessionStart`, where stdout is added as context that Claude can see and act on."*

Es decir: el `SessionStart` hook **NO produce un mensaje proactivo del asistente**. Su stdout queda en contexto, pero Claude solo "habla" cuando el usuario manda un prompt. La v1.1 de este PRD asumía que el hook bastaba para arrancar la conversación — error.

**Solución v1.2:** combo de dos mecanismos complementarios.

**1) `SessionStart` hook (estado dinámico, silencioso, <100ms):**

```json
{
  "hooks": {
    "SessionStart": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "\"${CLAUDE_PLUGIN_ROOT}/scripts/on-session-start.sh\""
          }
        ]
      }
    ]
  }
}
```

```bash
#!/usr/bin/env bash
# scripts/on-session-start.sh
# Budget: <100ms. Early-exit cuando coral no está en PATH.

command -v coral >/dev/null 2>&1 || {
  printf '{"coral_status":"binary_missing","suggestion":"run scripts/install.sh"}'
  exit 0
}

cd "${CLAUDE_PROJECT_DIR:-$PWD}" || exit 0
timeout 5 coral self-check --format=json --quick 2>/dev/null \
  || printf '{"coral_status":"check_failed"}'
exit 0
```

El JSON resultante queda en contexto: Claude sabe `wiki_present`, `providers_configured`, `warnings`, `update_available`, etc.

**2) `CLAUDE.md` en repo root (instrucciones estáticas de routing):**

`coral init` genera (append-safe) un `CLAUDE.md` con:

```markdown
# CLAUDE.md — Coral-enabled repo

This repository uses Coral for AI-readable architecture documentation.

## Routing instructions (for Claude Code)

When the user types ANYTHING in this repo (greeting, question, command):
1. Check the SessionStart hook context (Coral self-check JSON).
2. If coral_status == "binary_missing" → suggest the user run `scripts/install.sh`.
3. If wiki_present == false → suggest `/coral:coral-bootstrap`.
4. If warnings non-empty → suggest `/coral:coral-doctor`.
5. If wiki_present == true and the user asks about code → invoke coral-query skill.
6. If the user wants the UI → invoke coral-ui skill (background spawn).

## Quick reference

- `/coral:coral-bootstrap` — generate wiki for this repo (with cost confirmation)
- `/coral:coral-doctor` — diagnose environment / fix provider config
- `coral query "..."` — ask the wiki (via MCP, automatic in Claude Code)
- `coral ui serve --no-open` — start WebUI on port 3838
```

**Por qué funciona:**

1. `CLAUDE.md` se carga automáticamente al abrir Claude Code en el repo (documentado en memory docs).
2. `SessionStart` hook se ejecuta también al abrir; su JSON va a contexto.
3. El user escribe **cualquier cosa** (`hola`, `qué es esto`, `help`, una pregunta sobre el código). Ese prompt llega a Claude con (a) las routing instructions del CLAUDE.md + (b) el JSON del hook en contexto.
4. Claude responde según el routing.

**Append-safety:** si el repo ya tiene `CLAUDE.md`, `coral init` añade una sección `## Coral routing` al final si no existe; si ya existe, no-op. Nunca sobrescribe contenido del user.

**Bonus:** este patrón se compone con `extraKnownMarketplaces` (FR-ONB-26): el repo Coral mismo lo usa para dogfooding — clonar el repo Coral en Claude Code auto-registra la marketplace.

### 3.4 Análisis competitivo: ¿dónde está Coral vs developer tooling de clase mundial?

| Tool | Onboarding ideal (acts) | Notas |
|---|---|---|
| **rustup** (instalar Rust) | `curl ... | sh` → 1 acto | Self-contained; instala toolchain + cargo + rustc |
| **Stripe CLI** | `brew install stripe/stripe-cli/stripe` + `stripe login` → 2 actos | Browser-based OAuth; login pre-fills API key |
| **Vercel CLI** | `npm i -g vercel` + `vercel` (login en el primer run) → 2 actos | Login inline |
| **Supabase CLI** | `brew install supabase/tap/supabase` + `supabase init` → 2 actos | Local-first; init crea project skeleton |
| **Prisma** | `npx prisma init` → 1 acto (asume npm en el repo) | Genera schema + .env template |
| **Coral v0.33.0** | 6 actos (ver §2.1) | Plugin + binary externo + Claude Code interaction |
| **Coral v0.34.0 target** | **2 actos** (con `--with-claude-config`) **/ 3 actos** (default) | Paridad con la mejor clase de Anthropic plugin (LSP plugins listados en `discover-plugins.md` requieren 3) |

Coral pertenece a una clase distinta porque depende de Claude Code (no es una tool standalone con su propio CLI auth flow). La comparación honesta:

- **Stripe/Vercel/Supabase**: 2 actos porque controlan su propio auth + tienen un comando `init` que es el primer prompt. Coral tendría 2 actos con `--with-claude-config` (install hace TODO el setup del lado de plugin/marketplace; el "primer prompt" en Claude Code reemplaza el `init` standalone).
- **rustup**: 1 acto porque no hay agente involucrado. Coral por diseño requiere un agente (Claude Code / Cursor / Continue). No es comparable.

Lo que Coral hace **mejor que los LSP plugins de Anthropic**:
- `coral-doctor` skill que diagnostica + ofrece fixes (los LSP plugins solo dicen "executable not found").
- `coral bootstrap --estimate` con cost upfront (los LSP plugins no gastan tokens, no aplica).
- `CLAUDE.md` routing → el user no tiene que recordar nombres de skills.

Lo que Coral aprende de Stripe/Vercel:
- `--with-claude-config` opt-in (analogous to `stripe login`).
- Mini-wizard cuando falta credentials (analogous to `vercel` first-run prompt).
- Self-upgrade in-place (`stripe upgrade`).
- Self-uninstall clean (confianza para probar).

---

## 4. Objetivos y no-objetivos

### 4.1 Objetivos

- **O1** — `curl install.sh | bash -s -- --with-claude-config` (opt-in) instala binario + parchea `.claude/settings.json` del proyecto con `extraKnownMarketplaces` (backup atómico + idempotente). ≤ 60 segundos.
- **O1b** — Sin `--with-claude-config` (default por security), `install.sh` instala binario y imprime las 3 paste lines + escribe `.coral/claude-paste.txt`. ≤ 60 segundos.
- **O2** — Después del install, **cualquier prompt** del usuario en Claude Code (incluyendo "hola") dispara una respuesta de Claude que propone el siguiente paso correcto. Mecanismo: `CLAUDE.md` provisto por el plugin + `SessionStart` hook context.
- **O3** — `coral self-check --format=json` reporta TODO el estado del entorno incluyendo `providers_available`, `providers_configured`, `update_available`, `claude_cli`, `mcp_server`, `wiki`, `coral_toml`, `claude_md`.
- **O4** — Cost estimation **determinista**: `coral bootstrap --estimate` calcula tokens basado en LOC + filetypes + provider config sin gastar tokens. Muestra **upper-bound**. Margen ±25% en M1, ±15% en M2.
- **O5** — `coral bootstrap --max-cost=USD` aborta mid-flight si se excede. `--resume` retoma desde checkpoint persistido en `.wiki/.bootstrap-state.json`.
- **O6** — `coral-doctor` con mini-wizard de provider (4 opciones) cuando `providers_configured == []`.
- **O7** — Post-bootstrap: WebUI en background (nohup/Start-Process) + MCP server + primer query funcionando.
- **O8** — `time-to-first-wiki-query` ≤ 10 minutos sobre repo 10k LOC, conexión normal, claude-sonnet provider.

### 4.2 No-objetivos (anti-features)

- **N1** — **No habrá auto-install del binario desde el plugin**. Anthropic policy + security.
- **N2** — **No habrá Coral Cloud** en este sprint.
- **N3** — **No habrá auto-bootstrap sin confirmación de costo**. Bootstrap gasta dinero; el costo se muestra siempre, con upper-bound. El usuario aprueba siempre.
- **N4** — **No habrá telemetría**. Cero `phone home`. La calibración del estimate usa opt-in `coral feedback submit` (manual paste a un GitHub Discussion).
- **N5** — **No habrá UI propia para gestionar plugin/marketplace**. Reusamos `/plugin` de Claude Code.
- **N6** — **No habrá soporte oficial para Cursor/Continue/Cline en este sprint**. MCP server técnicamente funciona con ellos; onboarding optimizado para Claude Code. Cross-agent → M3.

---

## 5. Personas y casos de uso

| Persona | Contexto | Outcome esperado |
|---|---|---|
| **Solo dev, repo nuevo, Claude Code ya instalado** | Acaba de clonar. Nunca usó Coral. `claude` CLI configurado. | ≤10 min: wiki bootstrapeada, WebUI abierta, primer `coral query` respondido. |
| **Solo dev, descubre Coral por blog post, SIN Claude Code** | Quiere probar Coral pero recién va a instalar el ecosistema. | `install.sh` detecta ausencia de `claude` CLI → `coral-doctor` ofrece mini-wizard de provider (Anthropic API directa / Gemini / Ollama / claude CLI install). User elige y avanza. **No queda varado.** |
| **Solo dev, repo existente con `.wiki/`** | Clona repo de su empresa que ya usa Coral. | Plugin detecta `.wiki/` (via SessionStart) y NO bootstrapea; arranca WebUI + MCP, listo. |
| **Team lead, monorepo grande (50+ servicios)** | Setup multi-repo `coral.toml` + WebUI compartida. | M1: CLI no-interactivo (`coral project new` + flags + edit `coral.toml`). M2: wizard interactivo. |
| **CI/CD operator** | Quiere `coral test guarantee --can-i-deploy` en GH Actions. | Action wraps el install.sh + invoca comando; no requiere onboarding interactivo. |
| **Existing v0.33.0 user** | Ya tiene Coral; mira upgrade. | `coral self-check` reporta `update_available: "0.34.0"`; `coral self-upgrade` lo hace en un comando. |

---

## 6. Requisitos funcionales

Codificación: `FR-ONB-<n>`. Cada FR mapea a un mecanismo + a una persona.

### 6.1 Install script unificado (M1)

- **FR-ONB-1** — `scripts/install.sh` y `install.ps1` post-install detectan si Claude Code está instalado y, si sí, imprimen instrucciones más cortas. Si Claude Code NO está, sugieren install link + cómo continuar (incluyendo el path "sin claude CLI" via FR-ONB-27).
- **FR-ONB-2** — `install.sh` acepta `--skip-plugin-instructions` para CI (silencioso).
- **FR-ONB-3** — `install.sh` acepta `--version vX.Y.Z` para pin (ya existe; documentar).
- **FR-ONB-4** — Al final del install **sin** `--with-claude-config`, imprime las **3 líneas paste**:
  ```
  📋 Next: paste these three lines into Claude Code (one at a time):

      /plugin marketplace add agustincbajo/Coral
      /plugin install coral@coral
      /reload-plugins

  Then type anything in Claude Code — Coral's CLAUDE.md will guide it.
  ```
  Con `--with-claude-config`: solo imprime:
  ```
  ✅ Coral installed + marketplace registered.
  Open Claude Code in your repo and type anything to get started.
  ```
  El install también escribe `.coral/claude-paste.txt` con las 3 líneas para copy-paste desde un editor.
- **FR-ONB-5** — `plugin.json` y `marketplace.json` se sincronizan automáticamente en cada release vía `release.yml`.
- **FR-ONB-26 (NUEVO)** — `install.sh --with-claude-config` flag opt-in:
  1. Localiza `.claude/settings.json` del proyecto actual (`$PWD`). Si no existe, lo crea con `{}`.
  2. Backup atómico: copia a `.claude/settings.json.coral-backup-<ISO8601>`.
  3. Parsea JSON con `serde_json` (no string manipulation).
  4. Si la key `extraKnownMarketplaces` no existe, la crea como array.
  5. Si `agustincbajo/Coral` ya está, no-op.
  6. Si no está, lo añade al array.
  7. Escribe JSON de vuelta con `serde_json::to_writer_pretty`.
  8. Logs el path del backup al stdout: `*Backup at .claude/settings.json.coral-backup-2026-05-12T19:34:11Z. Restore with: mv <that> .claude/settings.json*`.
  9. Si el JSON parse falla (archivo corrupto), aborta con mensaje claro y no-touch.
  
  El flag es opt-in por security: el user expresa consentimiento explícito para que el script toque su config.

### 6.2 `coral self-check` y `coral-doctor` skill (M1)

- **FR-ONB-6** — `coral self-check [--format=json|text] [--quick]` comando nuevo:
  ```json
  {
    "coral_version": "0.34.0",
    "binary_path": "/usr/local/bin/coral",
    "claude_cli": {"installed": true, "path": "/usr/local/bin/claude", "version": "1.6"},
    "providers_available": ["claude_cli", "anthropic_api_key", "ollama"],
    "providers_configured": ["claude_cli"],
    "update_available": "0.35.0",
    "in_path": true,
    "is_git_repo": true,
    "wiki_present": false,
    "coral_toml_present": false,
    "claude_md_present": true,
    "mcp_server_reachable": null,
    "ui_server_reachable": null,
    "platform": "windows/x86_64",
    "warnings": ["claude CLI not found; provider wizard suggested"],
    "suggestions": [{"kind":"run_doctor","command":"/coral:coral-doctor","explanation":"Configure a provider to enable bootstrap."}]
  }
  ```
  Flag `--quick` salta MCP probe, UI health, git fetch, update-available check; target <100ms.
- **FR-ONB-7** — `coral-doctor` skill nueva en `.claude-plugin/skills/coral-doctor/SKILL.md`. Trigger: cualquier error reportado por Claude Code Errors tab que contenga "coral", o cuando el JSON del SessionStart hook tiene `warnings` no-vacío. Flujo:
  1. Ejecuta `coral self-check --format=json`.
  2. Por cada warning, ofrece el comando exacto para corregir.
  3. **Si `providers_configured` está vacío → lanza el mini-wizard FR-ONB-27**.
  4. Si todo OK, sugiere "tu próximo paso es `/coral:coral-bootstrap`".
- **FR-ONB-8** — `coral-doctor` valida la WebUI (`coral ui serve --no-open --port 38400 &` + `curl /health` + kill). Si falla, reporta razón.

### 6.3 Smart skills + SessionStart hook + CLAUDE.md (M1)

**Mecanismos antes/después:**

| Mecanismo | Estado v0.33.0 | Estado v0.34.0 | Cambio |
|---|---|---|---|
| `SessionStart` hook | no existe | **NUEVO** — corre `coral self-check --quick`; stdout silencioso a contexto | Estado dinámico determinístico |
| `CLAUDE.md` template en repo | no existe | **NUEVO** — `coral init` lo crea (append-safe) con routing instructions | "Claude responde correctamente al primer prompt" |
| `coral-bootstrap` skill | existe (rango de costo) | **actualizada** — `--estimate` first + upper-bound + `--max-cost` + `--resume` hints | Cost transparency + recovery |
| `coral-query` skill | existe | sin cambios | — |
| `coral-onboard` skill | existe (recomienda orden de lectura) | sin cambios | — |
| `coral-ui` skill | existe (fg) | **actualizada** — background spawn vía `nohup`/`Start-Process` | No-block UX |
| `coral-doctor` skill | no existe | **NUEVA** — self-check + provider mini-wizard | Cierra F1, F3, sin-claude-CLI |
| `/coral:coral-doctor` slash command | no existe | **NUEVO** — versión determinística | Power-user shortcut |

- **FR-ONB-9** — Hook `SessionStart` ejecuta `${CLAUDE_PLUGIN_ROOT}/scripts/on-session-start.sh`. **Budget <100ms** (CI verifica). Comportamiento:
  - Early-exit si `coral` no en PATH (típico <10ms).
  - `--quick` flag salta probes lentos (MCP, UI, git fetch, update-available).
  - `timeout 5` como hard cap.
  - Output JSON estructurado; Claude lo lee y combina con `CLAUDE.md` para rutear.

- **FR-ONB-25 (NUEVO)** — `coral init` **genera `CLAUDE.md`** en el repo con instrucciones de routing (ver §3.3). **Append-safe**: si ya existe, añade sección `## Coral routing` al final si no está; si está, no-op.

- **FR-ONB-10** — Skill `coral-bootstrap` (actualizada): SIEMPRE ejecuta `coral bootstrap --estimate` ANTES de pedir confirmación. Mensaje:
  ```
  Estimated cost: $0.42 (up to $0.53 — margin ±25%)
  Pages: 47 | Tokens: ~120k input + ~80k output
  Provider: claude-sonnet-4.5

  Want me to run it? Options:
    yes                                    run with no cap
    yes --max-cost=0.50                    abort mid-flight if exceeded
    yes --max-pages=20                     limit scope (useful for huge repos)
    cancel                                 abort
  ```
- **FR-ONB-11** — Skill `coral-ui` (actualizada): background spawn via `nohup coral ui serve --no-open --port 3838 > ~/.coral/ui.log 2>&1 &` (Linux/macOS) o `Start-Process -WindowStyle Hidden coral 'ui','serve','--no-open','--port','3838'` (Windows). Documenta `pkill -f "coral ui serve"` para detener. **`coral ui daemon` proper movido a M2** (FR-ONB-18).

### 6.4 Cost estimation determinista + max-cost + resume (M1)

- **FR-ONB-12** — `coral bootstrap --estimate` muestra **upper-bound + heurísticas + sugerencia para repos grandes**:
  ```
  Repo size: 10,247 LOC across 142 files (78 .rs, 31 .ts, 15 .md, ...)
  Estimated pages: 47
  Estimated tokens: ~120,000 input + ~80,000 output
  Provider: claude-sonnet-4.5
  Estimated cost: $0.42 (up to $0.53 — margin ±25%)
  ```
  Si `estimate.upper_bound > $5` (configurable):
  ```
  ⚠️  This is a large repo (estimate > $5). Consider starting with:

      coral bootstrap --apply --max-pages=50 --priority=high

  This bootstraps the 50 most-referenced modules first. You can run again
  later with --resume to continue or re-run without --max-pages to do all.
  ```
- **FR-ONB-13** — Cálculo basado en heurísticas: LOC bucketed por tipo, prompts versionados con tokens conocidos, output_per_page calibrado en runs reales. Heurísticas en `crates/coral-cli/src/commands/bootstrap/estimate.rs`.
- **FR-ONB-14** — Umbrales de confirmación configurables en `.coral/config.toml`:
  ```toml
  [bootstrap]
  auto_confirm_under_usd = 0.10
  warn_threshold_usd = 1.00
  big_repo_threshold_usd = 5.00
  ```
- **FR-ONB-29 (NUEVO)** — `coral bootstrap --max-cost=USD`:
  1. Estimate corre primero; si `estimate.upper_bound > max_cost` → abort antes de pagar nada con mensaje claro:
     ```
     Estimated upper bound ($0.53) exceeds --max-cost ($0.50).
     Try: --max-pages=N or remove --max-cost.
     ```
  2. Si `estimate.upper_bound ≤ max_cost` pero el actual cost mid-flight (suma running) excede `max_cost`: skip remaining pages, mark `.wiki/.bootstrap-state.json` con `partial: true`, exit con código 2 + mensaje "Stopped at $X.XX. Run `coral bootstrap --resume` to continue."
- **FR-ONB-30 (NUEVO)** — `coral bootstrap --resume`:
  - Checkpoint en `.wiki/.bootstrap-state.json` cada página completada (schema-versioned).
  - `--resume` lee el state, skipea páginas con `status: completed`, retoma desde la primera `pending`.
  - Compatible con `--max-cost` (se acumula al cost ya pagado).
  - Si schema del checkpoint cambió entre versiones → aborta con mensaje claro: "*Checkpoint schema v1, binary expects v2. Run `coral bootstrap --apply --force` to start over.*"

### 6.5 Multi-repo wizard (M2 — moved out of M1, con justificación)

`coral project init --wizard` y generación interactiva de `coral.toml` se movieron a v0.35.0. M1 mantiene flow CLI no-interactivo (`coral project new <name>` + flags + editar `coral.toml` + `coral project add`).

**Justificación cuantitativa**: la persona "Team lead monorepo" es **~5-10% del TAM** esperado (estimación; recalibrar con feedback v0.33.0). Las personas afectadas por F1-F5 son **~80-90%** (cualquier user nuevo). M1 prioriza el bottleneck más amplio. M2 hace el wizard cuando ya hay base de usuarios para feedback real sobre el flow exacto.

### 6.6 Post-bootstrap automation (M1)

- **FR-ONB-17** — Después de bootstrap exitoso, `coral-bootstrap` skill ofrece:
  1. Levantar WebUI en background (default: yes; configurable via `auto_serve_ui = false`).
  2. Sugerir snippet de pre-commit hook que invoque `coral ingest --apply --staged` (no instala automáticamente).
  3. Sugerir snippet de GitHub Actions que invoque `coral test guarantee --can-i-deploy`.
- **FR-ONB-18** — `coral ui daemon` proper (start/stop/status con PID file) **movido a M2 (v0.35.0)**. Razón: `daemonize` crate no soporta Windows; cross-platform daemon requiere abstracción custom no trivial.
  
  **Degradation aceptable en M1** (con `nohup`/`Start-Process`):
  - ❌ No hay auto-restart si crashea.
  - ❌ No hay PID file consistente cross-platform.
  - ❌ User debe re-spawnear manualmente tras reboot.
  - ✅ Funcional para el use-case típico (arrancar UI 1×/sesión).
  - ✅ Detener via `pkill -f "coral ui serve"` (Linux/macOS) o Task Manager (Windows).
  - ✅ Documentado en SKILL.md de `coral-ui`.

### 6.7 Plugin marketplace polish (M2)

- **FR-ONB-19** — `marketplace.json` declara `keywords`, `category` y describe prerequisitos en `description`.
- **FR-ONB-20** — ~~`installCommand` field~~ **eliminado** (no existe en schema).
- **FR-ONB-21** — README del repo: **sección "Getting Started in 60 seconds"** al tope con video o GIF.
- **FR-ONB-22** — Submission a Anthropic's official marketplace (`claude.ai/settings/plugins/submit`) — research previo.
- **FR-ONB-24 (NUEVO, M2)** — WebUI empty-state coaching: si `/pages` está vacía al primer load, mostrar inline "First time? Try these queries: …". SPA-only change; diferido a M2 por scope.

### 6.8 Cross-platform + maintenance (M1)

- **FR-ONB-23** — Tests E2E del onboarding flow en CI matrix Linux + macOS + Windows: install → register plugin (mock Claude Code con `~/.claude.json` injection) → invoke skill via `claude --print` → assert wiki created.
- **FR-ONB-31 (NUEVO)** — **Windows-specific friction mitigation**:
  - `install.ps1` imprime al final, en amarillo:
    ```
    ⚠ Windows Defender SmartScreen may block `coral.exe` on first run.
      If so: right-click → Properties → check "Unblock" → OK.
      We are working on code signing for v0.35.
    ```
  - `install.ps1` después de mutar PATH imprime:
    ```
    ⚠ PATH updated for new sessions. Open a NEW PowerShell window
      (current shell still has old PATH).
    ```
  - `install.sh` detecta WSL2 (lee `/proc/version` containing "microsoft") y, si está en WSL2, imprime:
    ```
    ⚠ Detected WSL2. Coral binary installed for Linux.
      If you use Claude Code on Windows host (not in WSL),
      install the Windows binary instead via install.ps1.
    ```
- **FR-ONB-32 (NUEVO)** — `coral self-upgrade`:
  ```bash
  coral self-upgrade [--version vX.Y.Z] [--check-only]
  ```
  - Default: latest same-major (v0.34.x → v0.34.y).
  - `--check-only`: solo reporta `update_available`.
  - Major bumps (0.34 → 0.35) **requieren install.sh explícito** (anti-feature AF-9 — evita data-corruption silenciosa si schemas cambian).
  - Implementación: re-ejecuta install script con `--version` target, in-place sobre el binary.
  - Self-upgrade NO toca el plugin (Claude Code lo auto-actualiza vía marketplace).
- **FR-ONB-33 (NUEVO)** — `coral self-uninstall`:
  ```bash
  coral self-uninstall [--keep-data]
  ```
  - Remueve binary del PATH.
  - Remueve `~/.coral/` (config + logs).
  - Con `--keep-data`: mantiene `~/.coral/`.
  - NO toca `.wiki/` del repo (es del repo, no del binary).
  - Imprime: `*Plugin still registered in Claude Code. Remove with `/plugin uninstall coral@coral`.*`

---

## 7. Arquitectura

```
┌────────────────────────────────────────────────────────────────────┐
│  El usuario: 1 comando (2 actos con --with-claude-config)          │
│                                                                     │
│    curl -fsSL https://coral.dev/install | bash \                    │
│      -s -- --with-claude-config                                     │
│                                                                     │
└──────────────────────────────┬─────────────────────────────────────┘
                               │
                               ▼
┌────────────────────────────────────────────────────────────────────┐
│  install.sh / install.ps1  (FR-ONB-1..5, 26, 31)                    │
│                                                                     │
│  1. Detect platform                                                 │
│  2. Download coral binary v0.34.0                                   │
│  3. Verify SHA-256                                                  │
│  4. Place on PATH                                                   │
│  5. Detect Claude Code installation                                 │
│  6. If --with-claude-config:                                        │
│       parchea .claude/settings.json con extraKnownMarketplaces      │
│       (backup atómico, idempotente, serde_json)                     │
│  7. Print final instructions (1 line if --with-claude-config,       │
│     3 paste lines otherwise)                                        │
│  8. Windows: Defender hint, PATH-new-shell hint                     │
│  9. WSL2: warning                                                    │
└──────────────────────────────┬─────────────────────────────────────┘
                               │
                               ▼  (user types ANYTHING in Claude Code)
┌────────────────────────────────────────────────────────────────────┐
│  Claude Code abre el repo                                           │
│                                                                     │
│   • CLAUDE.md (provisto por coral init, FR-ONB-25) cargado          │
│   • SessionStart hook ejecuta scripts/on-session-start.sh           │
│     → coral self-check --format=json --quick                        │
│     → stdout JSON al contexto (silencioso)                          │
│                                                                     │
│   User escribe lo que sea: "hola", "qué es esto", "/coral:..."      │
└──────────────────────────────┬─────────────────────────────────────┘
                               │
                               ▼  Claude routea según CLAUDE.md + hook
┌────────────────────────────────────────────────────────────────────┐
│  Branching                                                          │
│                                                                     │
│  coral_status == "binary_missing" → suggest install.sh              │
│  wiki_present == false            → suggest /coral:coral-bootstrap  │
│  providers_configured == []       → /coral:coral-doctor + wizard    │
│  warnings non-empty               → /coral:coral-doctor             │
│  wiki_present && user asks code   → invoke coral-query              │
│  user wants UI                    → invoke coral-ui (background)    │
└──────────────────────────────┬─────────────────────────────────────┘
                               │
                               ▼
┌────────────────────────────────────────────────────────────────────┐
│  Skill: coral-bootstrap  (FR-ONB-10, 12, 29, 30)                    │
│                                                                     │
│  1. coral self-check (provider configured?)                         │
│       → no? handoff to coral-doctor mini-wizard                     │
│  2. coral init  (creates .wiki/ + CLAUDE.md append-safe)            │
│  3. coral bootstrap --estimate                                      │
│     → "$0.42 (up to $0.53), 47 pages, 200k tokens"                  │
│     → if upper-bound > $5: suggest --max-pages=50 --priority=high   │
│  4. Confirm (yes / yes --max-cost=X / yes --max-pages=N / cancel)   │
│  5. coral bootstrap --apply [--max-cost=X] [--max-pages=N]          │
│     → checkpoints en .wiki/.bootstrap-state.json                    │
│     → if interrupted: `coral bootstrap --resume`                    │
│  6. Spawn `coral ui serve --background` (FR-ONB-17, 18)             │
│  7. Suggest CI integration snippets                                  │
│  8. Done. WebUI in browser. MCP server up. coral query works.       │
└────────────────────────────────────────────────────────────────────┘
```

### 7.1 Nuevo subcomando: `coral self-check`

**Crate**: `coral-cli` (nuevo handler).

```rust
#[derive(Serialize)]
pub struct SelfCheck {
    pub coral_version: String,
    pub binary_path: PathBuf,
    pub in_path: bool,
    pub git_repo: Option<GitRepoInfo>,
    pub wiki: Option<WikiInfo>,
    pub coral_toml: Option<ManifestInfo>,
    pub claude_md: Option<ClaudeMdInfo>,
    pub claude_cli: Option<ClaudeCli>,
    pub providers_available: Vec<ProviderId>,   // detected on system
    pub providers_configured: Vec<ProviderId>,  // in .coral/config.toml
    pub update_available: Option<String>,        // upstream version if newer
    pub mcp_server: Option<McpHealth>,
    pub ui_server: Option<UiHealth>,
    pub platform: PlatformInfo,
    pub warnings: Vec<Warning>,
    pub suggestions: Vec<Suggestion>,
}

#[derive(Serialize)]
pub struct Suggestion {
    pub kind: SuggestionKind,
    pub command: String,
    pub explanation: String,
}
```

### 7.2 Nuevo subcomando: `coral bootstrap --estimate`

```rust
pub fn estimate_cost(plan: &BootstrapPlan, provider: &Provider) -> CostEstimate {
    let input_tokens = plan.entries.iter().map(|e| {
        token_estimate_per_entry(e)
    }).sum();
    let output_tokens = plan.entries.len() * AVG_OUTPUT_PER_PAGE;
    let usd = match provider {
        Provider::Claude => input_tokens * 0.003 / 1000.0 + output_tokens * 0.015 / 1000.0,
        Provider::Gemini => ...,
        Provider::Local => 0.0,
    };
    CostEstimate {
        input_tokens,
        output_tokens,
        usd_estimate: usd,
        usd_upper_bound: usd * 1.25,  // ±25% en M1
        margin_of_error_pct: 25,
    }
}
```

Calibración: medir **10 runs reales** en M1 (small Rust, mid TS, large Python, monorepo, OpenAPI, Ollama variant, etc.).

### 7.3 Nueva skill: `coral-doctor` + provider mini-wizard

Path: `.claude-plugin/skills/coral-doctor/SKILL.md` (ver SKILL.md target literal en Apéndice C).

Behavior:
1. Llama `coral self-check --format=json` (vía Bash tool).
2. Parsea el output.
3. Si `providers_configured == []`, lanza mini-wizard FR-ONB-27.
4. Por cada `warning` o falta de prerequisite, prepara una respuesta estructurada:
   ```
   Coral needs <X> to <Y>. To fix:

     $ <exact command>

   This will <consequence in 1 sentence>. Want me to run it? (y/n)
   ```
5. Si el usuario acepta cada uno, ejecuta y re-runs `coral self-check`.
6. Cuando todo está verde, devuelve "Coral is ready. Try `/coral:coral-bootstrap` next."

### 7.4 Provider mini-wizard (FR-ONB-27 + FR-ONB-28)

Lanzado por `coral-doctor` cuando `providers_configured == []`:

```
Coral needs an LLM provider to bootstrap your wiki. Choose:

  [1] Anthropic API key directly
      (paste key now, stored in .coral/config.toml with 0600 perms)
  [2] Gemini API
      (paste key)
  [3] Ollama (local LLM, no API key, slower)
      (requires Ollama installed; will pull llama3.1:8b if needed)
  [4] Install claude CLI (Anthropic's official CLI)
      (browser link; run this wizard again after install)

Pick a number (1-4) or "skip" to abort:
```

Implementación:
- **Opciones 1-2**: pegar key, escribir `.coral/config.toml` con `[provider]` section, chmod 600 en el file. Verifica con un 1-token ping al provider.
- **Opción 3**: chequea `ollama` en PATH; si no, ofrece "Install Ollama: https://ollama.com → run again". Si sí, pull `llama3.1:8b` (default) si no está, escribir config.
- **Opción 4**: imprime "Install claude CLI: https://claude.ai/code → run again".

**FR-ONB-28** — Ollama path testeado E2E en M1:
- Test fixture: bootstrap mini-repo (50 LOC) con Ollama.
- Acceptable: slower (5min vs 30s) pero funcional.
- Document caveats: quality < claude-sonnet, page count menor.

### 7.5 `coral ui daemon` (subcomando — M2, NO M1)

[movido a v0.35.0 — ver FR-ONB-18]

### 7.6 Sincronización de versión plugin↔binary (FR-ONB-5)

**Step de prep (1 commit antes de M1)**: sincronización manual de `plugin.json` (v0.32.3 actual) y `marketplace.json` (v0.30.0 actual) a v0.33.0.

**Step automatizado en `release.yml`** (M1):

```yaml
- name: Sync plugin manifests
  run: |
    set -euo pipefail
    V="${RELEASE_TAG#v}"
    jq --arg v "$V" '.version = $v' \
       .claude-plugin/plugin.json > .tmp && mv .tmp .claude-plugin/plugin.json
    jq --arg v "$V" '.plugins[0].version = $v' \
       .claude-plugin/marketplace.json > .tmp && mv .tmp .claude-plugin/marketplace.json
    git diff --quiet .claude-plugin/ || {
      git config user.name "github-actions[bot]"
      git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
      git add .claude-plugin/
      git commit -m "ci(release): sync plugin manifests to ${RELEASE_TAG} [skip ci]"
      git push origin HEAD:main
    }
```

**Guardia anti-loop**: `[skip ci]` en commit subject + paths-ignore en `ci.yml` para `.claude-plugin/**` en push a main.

---

## 8. Stack y dependencias

### Backend (Rust)
- `dialoguer` crate (M1) — prompts interactivos del provider mini-wizard. Pequeño, single-purpose.
- `serde_json` (ya en deps) — para parche atómico de `.claude/settings.json`.
- `daemonize` crate (M2 — diferido).
- Windows: `winapi`/`windows-sys` ya en tree via `signal-hook` transitive.

### Frontend (SPA)
- Sin cambios en M1. WebUI empty-state coaching (FR-ONB-24) en M2.

### Skills (Markdown)
- Sin nuevas dependencies. Markdown puro.

### Scripts
- `install.sh` / `install.ps1` modificados — añaden `--with-claude-config`, Windows hints, WSL detect.

---

## 9. Fases / hitos

### Prep (1 commit, antes de M1)
- Sincronizar `plugin.json` (v0.32.3) y `marketplace.json` (v0.30.0) → v0.33.0. Una sola commit manual + push.
- Añadir `.claude/settings.json` con `extraKnownMarketplaces` al propio repo Coral (dogfooding).

### M1 — v0.34.0 (6–8 semanas, 1 dev)

| Semanas | Entregables |
|---|---|
| 1–2 | `coral self-check` + `coral self-upgrade` + `coral self-uninstall` subcommands (Rust crate `coral-cli`). `SessionStart` hook script con budget verificado <100ms. Tests unitarios. |
| 2–3 | `coral bootstrap --estimate` con upper-bound + `--max-cost` + `--resume` con checkpoints schema-versioned. Calibración n=10 runs reales. Update skill `coral-bootstrap`. |
| 3–4 | `coral-doctor` skill + provider mini-wizard (dialoguer) + slash command. `coral init` genera `CLAUDE.md` template (append-safe). Update `coral-ui` skill con background spawn. |
| 4–5 | Sync versions automation en `release.yml` con guardia `[skip ci]`. `install.sh --with-claude-config` (parche atómico con backup). `install.ps1` Windows hints. `install.sh` WSL detect. |
| 5–6 | E2E tests cross-platform Linux + macOS + Windows via `claude --print` mock. Ollama path validation E2E. |
| 6–8 | Docs "Getting Started in 60 seconds" + GIF/video. README updates. Release v0.34.0. Buffer para cross-platform issues. |

### M2 — v0.35.0 (4–5 semanas)

- `coral project init --wizard` interactivo (dialoguer).
- `coral ui daemon start/stop/status` cross-platform.
- `coral bootstrap --estimate` calibrado a ±15% con n≥30.
- Submission a Anthropic official marketplace.
- `coral-onboard-action` reusable GitHub Action.
- **WebUI empty-state coaching (FR-ONB-24)**.
- **i18n del onboarding (EN + ES)** en scripts + SKILL.md + CLI messages.

### M3 — v0.36.0+ (futuro)

- Cross-agent onboarding optimization (Cursor, Continue, Cline).
- `coral mcp serve --transport http` shared-MCP default.
- Coral Cloud (PRD separado).
- Code signing en Windows (Defender clean).

---

## 10. Métricas de éxito

| KPI | Baseline (v0.33.0) | Target M1 | Target M3 |
|---|---|---|---|
| Time-to-first-wiki-query, repo 10k LOC, conexión normal, claude-sonnet provider | ~30 min (manual reading) | **≤ 10 min** (incluye install 30s + paste 30s + first prompt 15s + bootstrap LLM ~5min + UI spin 5s + first query ~30s; bootstrap es dominante) | ≤ 3 min (con local-LLM provider) |
| Manual acts entre `curl install.sh` y wiki funcional | 6 | **2 con `--with-claude-config`** / **3 sin él** | 1 |
| Users que arrancan sin `claude` CLI y completan onboarding | N/A (varados) | **≥ 80%** (vía provider mini-wizard) | ≥ 95% |
| `coral bootstrap` cost variance (estimate vs actual) | N/A (rango opaco) | **±25%** con n=10 calibration runs | ±15% con n≥30 |
| `coral self-check` cobertura | N/A | **10 checks** (binary, claude CLI, providers, update, git, wiki, manifest, MCP, UI, platform, CLAUDE.md) | +3 |
| Test coverage del nuevo código en `coral-cli` | N/A | **≥ 75%** | ≥ 85% |
| Skills nuevas funcionales E2E en Claude Code mock | 0 | **1 skill (doctor)** + 1 hook (SessionStart) + 1 slash command (`/coral:coral-doctor`) + CLAUDE.md template | + wizard, + daemon control |
| `coral bootstrap --resume` recovers de crash o max-cost-hit | N/A | **✅ M1** | — |
| `coral self-upgrade` works same-major in-place | N/A | **✅ M1** | — |
| `coral self-uninstall` deja sistema limpio | N/A | **✅ M1** | — |
| `SessionStart` hook overhead p95 | N/A | **< 100ms** | < 50ms |

> **Assumptions del KPI "≤10 min":** binary install 30s, paste-3-lines (o auto-config) 30s, first user prompt 15s, `coral init` 1s, `coral bootstrap --estimate` 2s, user confirmation 30s, `coral bootstrap --apply` 3–5 min sobre repo 10k LOC con `claude-sonnet-4.5`, UI spin 5s, first `coral query` 5–30s. Bootstrap es 80% del tiempo y no es comprimible localmente; el target ≤3 min de M3 asume local-LLM provider o cache pre-built shipped con el plugin.

---

## 11. Decisiones resueltas (post second-pass review)

1. **`install.sh` auto-registra plugin?** → **RESUELTO: opt-in via `--with-claude-config`**. Default off (security). Parche idempotente con `serde_json` + backup atómico al directorio del archivo. Justificación: `extraKnownMarketplaces` es la forma documentada de Claude Code para project-scope marketplaces; herramientas similares (rustup, cargo install, oh-my-zsh) hacen patches similares de configs. El "risk" original era especulativo; con backup + idempotencia + flag explícito, la security tradeoff es aceptable.

2. **`coral-onboard-prompt` skill triggers amplios?** → **RESUELTO: la skill se eliminó (v1.1)**. Usamos `SessionStart` hook (silencioso) + **`CLAUDE.md` template** (v1.2 fix). El combo cubre estado dinámico (hook) e instrucciones de routing estáticas (CLAUDE.md) — y `CLAUDE.md` es el único mecanismo documentado para que Claude responda correctamente al primer prompt sin que el user tenga que invocar una skill específica por nombre.

3. **`coral bootstrap --estimate` llama al LLM?** → **RESUELTO: NO**. Heurísticas locales. M1 target ±25% con `n=10`; M2 calibra a ±15% con `n≥30` vía **opt-in `coral feedback submit`**: el comando imprime un JSON con (a) repo size + filetypes, (b) estimate generado, (c) actual cost de ese run. El user copia el JSON y lo pega en un GitHub Discussion del repo Coral. **No telemetry, no auto-send.** Calibración crowd-sourced explícita.

4. **`coral ui daemon` feature-gated?** → **RESUELTO: M2**. `daemonize` crate no soporta Windows; abstracción custom es scope creep. M1 usa `nohup`/`Start-Process` con degradation documentada (FR-ONB-18).

5. **`SessionStart` hook script inline o archivo?** → **RESUELTO: archivo separado** (`${CLAUDE_PLUGIN_ROOT}/scripts/on-session-start.sh`). Mejor DX para contributors + `bash -x` debuggable + reusable para tests.

6. **`extraKnownMarketplaces` en repo Coral mismo?** → **RESUELTO: sí**, en `.claude/settings.json` con comentario "for Coral contributors; remove if forking". Dogfooding + onboarding más simple para PRs.

7. **`disable-model-invocation` en `coral-doctor`?** → **RESUELTO: NO disable** (sigue model-invocable). La doctor flow puede beneficiarse de Claude juzgando contexto ambiguo. Costos por invocation: ~500 tokens. Aceptable. El slash command `/coral:coral-doctor` SÍ es determinístico para users que quieren shortcut explícito sin gasto LLM.

---

## 12. Riesgos y mitigaciones

| # | Riesgo | Probabilidad | Impacto | Mitigación |
|---|---|---|---|---|
| R1 | Anthropic cambia el plugin schema durante el sprint | B | M | Pin a `code.claude.com/docs` snapshot; subscribe a release notes; tests E2E corren contra Claude Code real, no mock |
| R2 | `coral bootstrap --estimate` se equivoca > ±30% en repos no calibrados | M | M | Mensaje claro "esto es estimación, no factura"; user siempre confirma; añadir `--max-cost` para techo duro; calibración crowd-sourced opt-in |
| R3 | `coral-doctor` skill genera false positives (auto-invocable demasiado amplio) | M | M | Trigger restrictivo a errores con "coral" + warnings del SessionStart hook; test E2E que verifique NO se invoca cuando todo OK |
| R4 | `coral ui daemon` fail en Windows | — | — | Movido a M2; M1 usa workaround documentado |
| R5 | Sincronización plugin↔binary versions en release.yml falla | B | A | Step corre ANTES del cargo build; falla del job aborta el release |
| R6 | Marketplace de Anthropic rechaza Coral por security/policy | B | A | Conversación con Anthropic ANTES de submission (M2); documentar threat model + security model |
| R7 | Usuarios reportan que el flow es "más complejo, no menos" | M | A | A/B test con 5 usuarios reales antes del release; medir TTV con cronómetro |
| R8 (NUEVO) | `--with-claude-config` patch corrompe `.claude/settings.json` (parse buggy) | B | A | `serde_json` para parse+write, no string append; backup atómico antes; test fixtures con malformed JSON; documentar revert en el output del script (`mv .claude/settings.json.coral-backup-* .claude/settings.json`) |
| R9 (NUEVO) | Ollama path falla en M1 (E2E test inestable) | M | M | Test fixture mini-repo; si Ollama no instalado en runner, skip test con mensaje claro; documentar "Ollama experimental in M1, prod-ready in M2" |
| R10 (NUEVO) | `coral bootstrap --resume` corrompe `.wiki/` si checkpoint schema cambia entre versiones | B | A | Schema-versioned checkpoint JSON; `--resume` aborta con mensaje claro si schema desactualizada; require misma versión de binary |
| R11 (NUEVO) | `CLAUDE.md` ya existe en el repo y `coral init` rompe contenido del user | B | A | `coral init` detecta CLAUDE.md existente; **append** sección `## Coral routing` solo si no existe; nunca sobrescribir; test fixture con CLAUDE.md pre-existente |

---

## 13. Anti-features (explícitos)

- **AF-1** — No habrá `phone home` telemetry. Cero datos enviados.
- **AF-2** — No habrá auto-bootstrap sin confirmación de costo (incluso si `auto_confirm_under_usd` está configurado, la primera vez hay un prompt).
- **AF-3** — No habrá download/install automático del binary desde el plugin (Anthropic policy + security).
- **AF-4** — No habrá UI propia para gestionar el plugin (usamos `/plugin` de Claude Code).
- **AF-5** — No habrá Coral Cloud en este sprint.
- **AF-6** — No habrá cross-agent onboarding optimizado (Cursor/Continue/Cline) — M3 trabajo.
- **AF-7** — No habrá WYSIWYG editor para `coral.toml` — sigue siendo CLI/TOML.
- **AF-8 (NUEVO)** — **M1 EN-only en onboarding scripts/CLI messages/SKILL.md**. La WebUI ya tiene i18n (EN+ES) desde v0.32. Los scripts (`install.sh`, `install.ps1`, error messages del binary), los SKILL.md de las skills, y los CLI prompts del provider mini-wizard siguen en inglés en M1. ES para todo el onboarding es M2.
- **AF-9 (NUEVO)** — **No habrá `coral self-upgrade --force-major`** (cross-major sin install.sh). Evita data-corruption silenciosa si schemas cambian entre majors. Major bumps requieren el install.sh explícito.

---

## 14. Apéndice A: Fricciones actuales detectadas (F1–F10)

Ver §2.2 arriba.

---

## 15. Apéndice B: Plan de implementación detallado M1

```
Semana 1 — self-check + self-upgrade + self-uninstall + sync versions
  - Crear crates/coral-cli/src/commands/self_check.rs
  - Crear crates/coral-cli/src/commands/self_upgrade.rs
  - Crear crates/coral-cli/src/commands/self_uninstall.rs
  - Subcommand definitions en clap (main.rs)
  - JSON schema con providers_available, providers_configured, update_available, claude_md_present
  - 10 unit tests
  - release.yml: añadir step sync-plugin-manifests con jq + [skip ci] guard
  - Sync plugin.json + marketplace.json a v0.34.0 (manual una vez)

Semana 2 — Cost estimation + max-cost + resume + bootstrap skill update
  - Crear crates/coral-cli/src/commands/bootstrap/estimate.rs
  - Heurísticas: token_estimate_per_entry(entry) -> u64
  - upper_bound calculation (estimate * 1.25 en M1)
  - --max-cost flag con abort mid-flight
  - --resume flag con checkpoints schema-versioned en .wiki/.bootstrap-state.json
  - Calibration runs (10): small Rust, mid TS, large Python, monorepo, OpenAPI, Ollama variant, etc.
  - Skill .claude-plugin/skills/coral-bootstrap/SKILL.md updated
  - Mensaje formato con upper-bound + sugerencias para repos grandes

Semana 3 — Doctor + provider wizard + CLAUDE.md + ui background
  - .claude-plugin/skills/coral-doctor/SKILL.md (new)
  - .claude-plugin/commands/coral-doctor.md (slash command)
  - Provider mini-wizard via dialoguer (4 paths: Anthropic key, Gemini key, Ollama, claude CLI install)
  - coral init: write CLAUDE.md template (append-safe with test fixtures)
  - coral-ui skill: background spawn (nohup / Start-Process)
  - SessionStart hook: scripts/on-session-start.sh con budget <100ms verificado en CI

Semana 4 — Windows specifics + install.sh --with-claude-config + cross-platform tests
  - install.sh: --with-claude-config flag, JSON patch atómico con serde_json, backup
  - install.ps1: WindowsDefender hint, PATH-needs-new-shell hint
  - install.sh: WSL2 detection (lee /proc/version)
  - extraKnownMarketplaces en Coral's own .claude/settings.json (dogfooding)
  - E2E test via claude --print mock + matrix

Semana 5–6 — Ollama path validation + buffer + docs
  - Test fixture: 50-LOC repo bootstrap con Ollama
  - --provider=ollama E2E (skip si Ollama no instalado en CI runner)
  - Docs: README "Getting Started in 60 seconds" + GIF/video
  - Cross-platform smoke runs

Semana 7–8 — Release + buffer
  - Release notes draft
  - Tag v0.34.0 → release.yml runs
  - Hotfix buffer
```

---

## 16. Criterios de aceptación (DoD M1)

1. `curl install.sh | bash` (sin flags) instala el binario v0.34.0 e imprime las **3 paste lines** + escribe `.coral/claude-paste.txt`. Tiempo total ≤ 60s.
2. `curl install.sh | bash -s -- --with-claude-config` adicionalmente parchea `.claude/settings.json` del proyecto actual con `extraKnownMarketplaces` (backup atómico al directorio, idempotente). Imprime path del backup.
3. Después del install + paste (o solo install con `--with-claude-config`), abrir Claude Code en el repo + **cualquier prompt del user** (incluyendo "hola") dispara la respuesta correcta de Claude routeada por **`CLAUDE.md`** (provista por `coral init`) + **`SessionStart` hook context**.
4. `coral bootstrap --estimate` muestra costo con **upper-bound explícito** y margen ±25% verificable contra 10 calibration runs reales.
5. `coral bootstrap --max-cost=USD` aborta antes de pagar si `estimate.upper_bound > max-cost`, o aborta mid-flight con checkpoint si actual cost > max-cost; `coral bootstrap --resume` retoma desde checkpoint.
6. `coral self-check --format=json` cubre **10 checks** incluyendo `providers_available`, `providers_configured`, `update_available`, `claude_md_present`.
7. `coral-doctor` skill: si `providers_configured == []`, ofrece mini-wizard de 4 paths (Anthropic API key / Gemini / Ollama / claude CLI install). Ollama path testeado E2E con mini-fixture.
8. `coral self-upgrade` upgrade same-major in-place; `coral self-uninstall` deja sistema limpio con `--keep-data` opt-out.
9. `SessionStart` hook ejecuta en **<100ms p95** medido en CI matrix; early exit cuando binary no en PATH (<10ms).
10. WebUI background spawn (`nohup`/`Start-Process`) funcional en Linux/macOS/Windows; degradation documentada en SKILL.md de `coral-ui`.
11. Windows-specific friction mitigation en `install.ps1`: Defender SmartScreen hint + new-shell PATH hint. `install.sh` detecta WSL2 y advierte.
12. CI matrix Linux + macOS + Windows todos verdes con E2E onboarding via `claude --print` mock.
13. `plugin.json` y `marketplace.json` ambos en v0.34.0, sincronizados automáticamente por `release.yml` con `[skip ci]` guard.
14. README tiene sección "Getting Started in 60 seconds" con GIF/video.
15. **`CLAUDE.md` template provisto por `coral init`** es append-safe (no sobrescribe existente; añade sección `## Coral routing` solo si no está; no-op si ya está).
16. **BC sagrada**: todas las skills/commands existentes (`coral-bootstrap`, `coral-query`, `coral-onboard`, `coral-ui`) siguen funcionando idénticas a v0.33.0; lo nuevo es aditivo.
17. **Items movidos a M2 (NO bloquean M1)**: `coral ui daemon`, `coral project init --wizard`, ±15% estimate accuracy, marketplace submission a Anthropic, WebUI empty-state coaching, i18n ES.

---

## 17. Apéndice C: SKILL.md target literal (coral-bootstrap actualizado)

```markdown
---
name: coral-bootstrap
description: Bootstrap the Coral wiki for the current repository, with cost confirmation.
triggers:
  - bootstrap coral
  - set up coral
  - coral init
  - generate wiki
  - this repo doesn't have a wiki
disable-model-invocation: false
---

# Coral bootstrap

Run a cost-confirmed wiki bootstrap for the current repo.

## Steps

1. Run `coral self-check --format=json --quick`. Parse the JSON.
2. If `wiki_present == true`, ask user "Wiki already exists. Re-bootstrap? (y/n)". If no, exit.
3. If `providers_configured == []`, hand off to `coral-doctor` skill (it has the provider mini-wizard). Do NOT continue here.
4. Run `coral bootstrap --estimate`. Capture stdout.
5. Show the user:
   - Estimated cost with upper-bound: "$0.42 (up to $0.53)"
   - Pages count
   - Provider used
   - If `estimate.upper_bound > $5`: suggestion for large repos with `--max-pages=50 --priority=high`
6. Ask: "Run? Options:
     - yes
     - yes --max-cost=X
     - yes --max-pages=N
     - cancel"
7. On confirm, run `coral bootstrap --apply [--max-cost=X] [--max-pages=N]`.
8. If interrupted/failed mid-flight: mention `coral bootstrap --resume`. Checkpoints are in `.wiki/.bootstrap-state.json`.
9. On success:
   - Suggest spawning the WebUI: invoke `coral-ui` skill (background spawn).
   - Mention "Your wiki is in `.wiki/`. Try queries like 'show me the architecture' or open http://localhost:3838/pages."
   - Suggest CI integration snippet (pre-commit hook, GitHub Actions).

## Failure modes

- `coral` not in PATH → suggest `/coral:coral-doctor`.
- No provider configured → hand off to `coral-doctor` (which has the wizard).
- `--apply` fails mid-flight → mention `--resume`.
- Estimate upper-bound exceeds `--max-cost` → mention `--max-pages` to limit scope.
```

---

## 18. Apéndice D: Changelog v1.0 → v1.1 (first-pass review, 2026-05-12)

1. **Re-arquitectado FR-ONB-9** para usar `SessionStart` hook (que SÍ existe en Claude Code — verificado contra docs oficiales) en lugar de auto-invocación NLP amplia. Resuelve elegante y deterministicamente F4. [v1.2 nota: el hook es silencioso; v1.2 lo combina con CLAUDE.md.]
2. **§3.1 actualizado** con tabla de Claude Code primitives reusables, incluyendo hooks.
3. **FR-ONB-4** corregido: chain `&&` NO funciona en Claude Code. Tres líneas separadas, no una.
4. **FR-ONB-20** (`installCommand` en marketplace.json) **eliminado** — el field no existe en el schema oficial.
5. **§7.6 sync release.yml** añade guardia explícita anti-loop: `[skip ci]` en commit message + filter en `ci.yml` `on.push`.
6. **F2 sync manual pre-M1** movido a un step de **prep** (1 commit antes del sprint).
7. **`coral ui daemon` + `project init --wizard` movidos a M2 (v0.35.0).** Windows daemon no es trivial (`daemonize` crate no soporta Windows).
8. **O4 (cost estimation)** bajado de ±15% a **±25%** en M1; ±15% es target M2 con `n≥30` calibration data.
9. **Timeline M1 ampliado** de 3–4 → **6–8 semanas** para un dev.
10. **§11 Decisión #1 resuelta**: install.sh imprime 3 líneas separadas. [v1.2: ahora opt-in `--with-claude-config` baja a 0 líneas + 1 line install.]
11. **§6.3 tabla explícita** `coral-onboard` (existente, mantener) vs `coral-onboard-prompt` (nuevo). [v1.2: `coral-onboard-prompt` ELIMINADO; reemplazado por CLAUDE.md + hook combo.]

---

*Fin del PRD v1.2 — incorpora 3 críticos + 8 medianos + 4 huecos cerrados del second-pass review independiente.*

*v1.0 → v1.1 (first-pass): Apéndice D.*
*v1.1 → v1.2 (second-pass): cabecera.*
