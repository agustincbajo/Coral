# PRD — Coral v0.34: Zero-Friction Onboarding via Claude Code

**Versión del documento:** 1.1 (post-review independiente)
**Fecha:** 2026-05-12
**Autor:** Agustín Bajo
**Estado:** Borrador validado
**Versiones objetivo:** Coral v0.34.0 (M1, 6–8 semanas) → v0.34.x patches → v0.35.0 (M2 marketplace polish + daemon + wizard)

**Cambios v1.0 → v1.1** (post review):
1. **Re-arquitectado FR-ONB-9** para usar `SessionStart` hook (que SÍ existe en Claude Code — verificado contra docs oficiales) en lugar de auto-invocación NLP amplia. Resuelve elegante y deterministicamente F4.
2. **§3.1 actualizado** con tabla de Claude Code primitives reusables, incluyendo hooks.
3. **FR-ONB-4** corregido: chain `&&` NO funciona en Claude Code. Tres líneas separadas, no una.
4. **FR-ONB-20** (`installCommand` en marketplace.json) **eliminado** — el field no existe en el schema oficial.
5. **§7.6 sync release.yml** añade guardia explícita anti-loop: `[skip ci]` en commit message + filter en `ci.yml` `on.push`.
6. **F2 sync manual pre-M1** movido a un step de **prep** (1 commit antes del sprint).
7. **`coral ui daemon` + `project init --wizard` movidos a M2 (v0.35.0).** Windows daemon no es trivial (`daemonize` crate no soporta Windows). M1 mantiene `coral ui serve --background` simple via `nohup`/`Start-Process`.
8. **O4 (cost estimation)** bajado de ±15% a **±25%** en M1; ±15% es target M2 con `n≥30` calibration data en lugar de `n=5`.
9. **Timeline M1 ampliado** de 3–4 → **6–8 semanas** para un dev.
10. **§11 Decisión #1 resuelta**: install.sh imprime 3 líneas separadas que el usuario pega; NO auto-registra el plugin desde fuera de Claude Code (riesgo de mutar `~/.claude.json` directamente). En M2 evaluar `claude plugin install --headless` si Anthropic lo expone.
11. **§6.3 tabla explícita** `coral-onboard` (existente, mantener) vs `coral-onboard-prompt` (nuevo). Comportamiento claro de cada uno.
**Predecesor:** [PRD-v0.32-webui.md](PRD-v0.32-webui.md) — la WebUI existe, ahora hay que llevar a usuarios hacia ella sin fricción.

---

## 1. Resumen ejecutivo

Coral v0.33.0 ya tiene **todo el producto**: binario único, 4 binarios cross-platform en releases, plugin de Claude Code con 4 skills + 2 slash commands, WebUI moderna, REST API, MCP server, contract checking, multi-repo. Lo que falta es **el camino del usuario**: hoy, un desarrollador nuevo necesita **al menos 3 actos desacoplados** (instalar binary fuera de Claude Code, instalar plugin dentro de Claude Code, pedirle a Claude que arranque el setup) — con **10 fricciones documentadas** (F1–F10 en el [reconnaissance interno](#apendice-a-fricciones-actuales-detectadas)) entre las que sobresalen: el binario no se auto-instala, no hay detección automática de "este repo no tiene wiki", el costo LLM del bootstrap es opaco, y multi-repo requiere editar `coral.toml` a mano.

Este PRD lleva el onboarding a **un solo comando + Claude conduce el resto**:

```
# El usuario solo escribe ESTO. Todo lo demás Claude lo guía.
curl -fsSL https://coral.dev/install | bash
```

A partir de ahí, en el siguiente prompt en Claude Code:
- Claude detecta el binario, registra el plugin, valida prerequisitos, ofrece bootstrap con cost-estimate determinista, levanta la WebUI, y guía multi-repo con un wizard interactivo.

**Wedge** (la única razón que justifica este sprint): **time-to-first-wiki-query ≤ 5 minutos** sobre un repo desconocido, sin que el usuario abra documentación.

Hoy ese tiempo es **20–40 minutos** y requiere leer el README, decidir provider LLM, escribir `coral.toml` por defecto, manejar errores silenciosos. Anthropic acaba de probar con los LSP plugins (gopls, rust-analyzer, pyright) que el patrón "plugin + binary externo" es aceptable; lo que distingue a un plugin bien hecho de uno mediocre es **cuán inteligente es la primera skill**.

**Cuatro principios no negociables:**

1. **Una sola línea de install para el usuario.** El binario, el plugin y la marketplace se configuran con UN `curl | bash`. Cualquier descarga adicional la hace Claude por el usuario, con confirmación.
2. **Skills detectan, no asumen.** Cada skill chequea su precondición antes de ejecutar (¿coral en PATH? ¿claude CLI en PATH? ¿es git repo? ¿tiene wiki? ¿tiene coral.toml?). Cuando falta algo, ofrece el comando exacto para corregirlo, no un mensaje genérico.
3. **Cost transparency.** Cualquier comando que gaste tokens (bootstrap, ingest, query) muestra el costo estimado **antes** de correr, y Claude pide confirmación cuando supera un umbral configurable (default $0.10).
4. **Loop cerrado.** El último paso del onboarding deja al usuario con (a) wiki bootstrapeada, (b) WebUI corriendo en el browser, (c) MCP server registrado, (d) un primer query funcionando. No "instalar y descubrir qué hacer"; "instalar y ya estás trabajando".

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

**Pasos manuales hoy: 4 (1, 3, 4, 5)** que el usuario escribe sin guía. **Friction points internos: 6 (F1–F6 abajo)**.

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
2. **`scripts/install.sh` también registra el plugin** (vía `gh` CLI o un endpoint Claude Code), de manera que el `curl | bash` deje todo wired.
3. **`coral self-check`** comando nuevo del binario que verifica todo el entorno y reporta JSON estructurado (para el skill).

---

## 3. Posicionamiento vs alternativas

| Approach | Pros | Cons | Coral elige |
|---|---|---|---|
| **A. "App store style"** — usuario descarga marketplace, click "install", todo se hace solo | UX óptimo, modelo conocido | **Anthropic no soporta auto-binary-install**; el plugin no puede ejecutar arbitrary code en install | ❌ |
| **B. Status quo extendido** — README más claro + mejores mensajes de error | Cero infra nueva | No cierra la fricción real (F1 sigue) | ❌ |
| **C. Hybrid: install.sh hace todo, skills detectan estado, doctor cierra el loop** | Una línea de install + Claude guía después | Más complejidad inicial en `install.sh`; requiere comunicación con Claude Code (¿es factible?) | ✅ |
| **D. Coral Cloud** — instalar nada localmente, todo en SaaS | Cero fricción local | Rompe el anti-feature §13 del PRD anterior ("no SaaS multi-tenant"); cambia el producto | ❌ |

**Decisión: opción C.** Approach híbrido — script potente + skills inteligentes + doctor command.

### 3.1 Reusar lo que Anthropic ya construyó

| Capacidad de Claude Code | Usamos para |
|---|---|
| `extraKnownMarketplaces` en `.claude/settings.json` | Auto-añadir marketplace de Coral cuando el repo se abre (si scope = project) |
| Plugin auto-update | Plugin se mantiene sincronizado con el binario via release.yml |
| MCP server `env` block | Inyectar `RUST_LOG`, `CORAL_PROVIDER`, etc. sin que el user los configure |
| Skill auto-invocation por NLP | Mantenemos las 4 skills + agregamos 2 (doctor, wizard) |
| `/reload-plugins` post-install | Documentado para auto-correr en el doctor flow |
| `disable-model-invocation: true` en slash commands | Slash commands deterministicos, sin gasto LLM |

### 3.2 Primitives de Claude Code que usamos / no usamos

| Primitive | Documentación | Lo usamos para |
|---|---|---|
| **`SessionStart` hook** | [hooks reference](https://code.claude.com/docs/en/plugins-reference) — listado verbatim | **Resuelve F4**: ejecutar `coral self-check` cuando se abre el repo, sin requerir prompt del usuario |
| **`UserPromptSubmit` hook** | idem | NO en M1 (lo que necesitábamos lo cubre SessionStart) |
| **`PreToolUse` hook** | idem | NO en M1 (no bloqueamos Bash calls) |
| `mcpServers` block en plugin.json | reference + Coral hoy | Registramos `coral mcp serve --transport stdio` (sin cambios) |
| Skills auto-invocables (NLP triggers) | reference | Las 4 existentes + 1 nueva (`coral-doctor`) |
| Slash commands con `disable-model-invocation` | reference | `/coral:coral-doctor` (nuevo) determinístico |
| `extraKnownMarketplaces` en `.claude/settings.json` proyecto-scope | [settings docs](https://code.claude.com/docs/en/settings) — confirmado verbatim | Auto-añadir marketplace por proyecto cuando el repo se clone con esa config |
| `${CLAUDE_PLUGIN_ROOT}` + `${CLAUDE_PROJECT_DIR}` env vars en hooks | reference | Path resolution determinístico en `SessionStart` hook script |

| Primitive que NO existe (verificado) | Workaround |
|---|---|
| **Auto-install de binario desde el plugin** | `install.sh` separado (estándar de la industria — todos los LSP plugins lo hacen así) |
| **Chain de slash commands con `&&`** | `install.sh` imprime 3 líneas, el user las pega de a una |
| **`installCommand` field en `marketplace.json`** | El campo no existe en el schema; el README + `description` del marketplace contienen las instrucciones |
| **CLI externa `claude plugin install --headless`** | No documentada; investigar para M2 si Anthropic la publica |

### 3.3 Por qué `SessionStart` hook es el cambio arquitectural más importante

La v1.0 de este PRD proponía una skill auto-invocable (`coral-onboard-prompt`) con triggers NLP amplios para detectar "primera sesión en repo sin wiki". El reviewer puntualizó (correctamente) que eso es **un router intentando colarse en cada prompt** — falsos positivos inevitables, y nomenclatura confusa (`coral-onboard` vs `coral-onboard-prompt`).

La solución correcta es el hook `SessionStart`, documentado verbatim:

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

El script `scripts/on-session-start.sh` que envía el plugin Coral:

```bash
#!/usr/bin/env bash
# Coral plugin SessionStart hook.
# Runs when a Claude Code session begins or resumes.
# Reports environment status as JSON; Claude's main agent reads it.

if ! command -v coral >/dev/null 2>&1; then
  printf '{"coral_status": "binary_missing", "suggestion": "run scripts/install.sh"}'
  exit 0
fi

cd "${CLAUDE_PROJECT_DIR:-$PWD}" || exit 0
coral self-check --format=json --quick
exit 0
```

Esto:
1. Es determinístico (no NLP).
2. Se ejecuta UNA vez al abrir la sesión, no en cada prompt.
3. Su output queda disponible para el contexto principal de Claude, que puede entonces invocar las skills correctas.
4. No requiere una skill `coral-onboard-prompt` separada; las existentes (`coral-bootstrap`, `coral-query`, `coral-ui`, `coral-onboard`) siguen siendo las que el usuario invoca explícitamente.

**El cambio neto**: en lugar de añadir una skill 5ta confusa, agregamos UN hook + UN script bash + reutilizamos las skills existentes que ya saben qué hacer.

---

## 4. Objetivos y no-objetivos

### 4.1 Objetivos

- **O1** — `curl install.sh | bash` instala binario + sugiere o auto-registra el plugin de marketplace en una sola pasada. ≤ 60 segundos end-to-end en una conexión normal.
- **O2** — Después del install, el primer prompt del usuario en Claude Code dispara una skill que valida el entorno y arranca el bootstrap (con cost confirmation determinista). No requiere que el usuario sepa qué comando invocar.
- **O3** — `coral self-check` (nuevo) reporta JSON estructurado con TODO el estado del entorno: binary version, PATH, claude CLI presence, git repo, wiki status, coral.toml, MCP server health. Usado por `coral-doctor` skill y por humanos.
- **O4** — Cost estimation **determinista** (no rango): `coral bootstrap --estimate` calcula tokens basado en LOC + filetypes + LLM provider config sin gastar tokens. Margen de error ≤ ±15%.
- **O5** — Multi-repo wizard (`coral project init --wizard`) interactivo: Claude pregunta nombre, repos, tags, depends-on, genera `coral.toml` validado.
- **O6** — Post-bootstrap: la última acción del happy path es **levantar la WebUI en background** + abrir browser + dejar el MCP server corriendo. El usuario ya está trabajando con Coral.
- **O7** — `time-to-first-wiki-query` ≤ 5 minutos sobre un repo de 10k LOC, mediado por reloj de pared, desde `curl install.sh | bash` hasta la primera respuesta de `coral query`.

### 4.2 No-objetivos (anti-features)

- **N1** — **No habrá auto-install del binario desde el plugin**. Anthropic no lo soporta; intentar workarounds vía hooks dudosos rompe el plugin marketplace policy. El `install.sh` separado es la única vía.
- **N2** — **No habrá Coral Cloud**. Sigue siendo local-first. Si futuro PRD lo introduce, no está en este sprint.
- **N3** — **No habrá auto-bootstrap sin confirmación**. Bootstrap gasta dinero; el costo se muestra siempre. El usuario aprueba siempre.
- **N4** — **No habrá telemetría**. Ningún `phone home`. Ni siquiera "anonymous usage stats". Cero.
- **N5** — **No habrá UI propia para gestionar plugin/marketplace**. Reusamos `/plugin` de Claude Code.
- **N6** — **No habrá soporte oficial para Cursor/Continue/Cline en este sprint**. El MCP server técnicamente funciona con ellos pero el onboarding está optimizado para Claude Code. Cross-agent onboarding es M3 trabajo.

---

## 5. Personas y casos de uso

| Persona | Contexto | Outcome esperado |
|---|---|---|
| **Solo dev, repo nuevo** | Acaba de clonar un repo en su laptop. Nunca usó Coral. | En ≤5 min: wiki bootstrapeada, WebUI abierta en browser, primer `coral query` respondido. |
| **Solo dev, repo existente con `.wiki/`** | Clona un repo de su empresa que ya usa Coral. | El plugin detecta `.wiki/` y NO bootstrapea de nuevo; arranca WebUI + MCP, listo. |
| **Team lead, monorepo grande (50+ servicios)** | Quiere setup multi-repo `coral.toml` + WebUI compartida. | Wizard interactivo guía la generación de `coral.toml`; bootstrap muestra costo agregado **antes** de aceptar; WebUI sirve `/graph` con todos los servicios. |
| **CI/CD operator** | Quiere correr `coral test guarantee --can-i-deploy` en GitHub Actions sin instalar plugins ni binarios manualmente. | Action de GH wraps el install.sh + invoca el comando; no requiere onboarding interactivo. |
| **Existing v0.33.0 user** | Ya tiene Coral instalado, está mirrando el upgrade. | `coral self-check` reporta v0.33→v0.34 disponible; `coral self-upgrade` lo hace en un comando. |

---

## 6. Requisitos funcionales

Codificación: `FR-ONB-<n>`. Cada FR mapea a un mecanismo + a una persona.

### 6.1 Install script unificado (M1)

- **FR-ONB-1** — `scripts/install.sh` y `install.ps1` post-install **detectan si Claude Code está instalado** y, si sí, imprimen instrucciones más cortas. Si Claude Code NO está, sugieren install link + cómo continuar.
- **FR-ONB-2** — `install.sh` acepta `--skip-plugin-instructions` para CI (silencioso).
- **FR-ONB-3** — `install.sh` acepta `--version vX.Y.Z` para pin (ya existe; documentar).
- **FR-ONB-4** — Al final del install, imprime las **3 líneas que el usuario debe pegar en Claude Code** (Claude Code no soporta chain con `&&` — verificado contra docs oficiales). El install.sh los formatea como un único bloque copy-pasteable:
  ```
  📋 Next: paste these three lines into Claude Code (one at a time):

      /plugin marketplace add agustincbajo/Coral
      /plugin install coral@coral
      /reload-plugins

  Alternatively, if you control this repo:
      add `extraKnownMarketplaces` to .claude/settings.json
      so collaborators auto-register the marketplace when they clone.
  ```
  El install también escribe un fichero `.coral/claude-paste.txt` con las mismas 3 líneas, para copy-paste desde un editor.
- **FR-ONB-5** — `plugin.json` y `marketplace.json` se sincronizan automáticamente en cada release vía `release.yml` (hoy son manuales y desincronizados, F2). Job nuevo: `update-plugin-version`.

### 6.2 `coral self-check` y `coral-doctor` skill (M1)

- **FR-ONB-6** — `coral self-check [--format=json|text]` comando nuevo del binario que reporta:
  ```json
  {
    "coral_version": "0.34.0",
    "binary_path": "/usr/local/bin/coral",
    "claude_cli": {"installed": true, "path": "/usr/local/bin/claude", "version": "1.6"},
    "in_path": true,
    "is_git_repo": true,
    "wiki_present": false,
    "coral_toml_present": false,
    "mcp_server_reachable": null,  // null = not tested; true/false if --test-mcp
    "platform": "windows/x86_64",
    "warnings": ["claude CLI not found; bootstrap requires it"]
  }
  ```
- **FR-ONB-7** — `coral-doctor` skill nueva en `.claude-plugin/skills/coral-doctor/SKILL.md`. Trigger amplio: cualquier error reportado por Claude Code Errors tab que contenga "coral", o el primer prompt del usuario cuando el plugin acaba de ser instalado. Flujo:
  1. Ejecuta `coral self-check --format=json`.
  2. Por cada warning/falla, ofrece el comando exacto para corregir.
  3. Si todo OK, sugiere "tu próximo paso es `/coral:coral-bootstrap`".
- **FR-ONB-8** — `coral-doctor` también valida la WebUI (`coral ui serve --no-open --port 38400 &` + `curl /health` + kill). Si falla, reporta razón.

### 6.3 Smart skills + SessionStart hook (M1)

**Skills antes/después** (claridad post-review):

| Skill / Hook | Estado v0.33.0 | Estado v0.34.0 | Cambio |
|---|---|---|---|
| `SessionStart` hook | no existe | **NUEVO** — corre `coral self-check --quick` al abrir Claude Code | Detección determinística de estado del entorno |
| `coral-bootstrap` skill | existe (rango de costo) | **actualizada** — siempre `--estimate` first, número concreto | Cost transparency |
| `coral-query` skill | existe | sin cambios | — |
| `coral-onboard` skill | existe (recomienda orden de lectura) | sin cambios | — |
| `coral-ui` skill | existe (foreground) | **actualizada** — spawnea background usando `nohup`/`Start-Process` | No-block UX |
| `coral-doctor` skill | no existe | **NUEVA** — wraps `coral self-check`, propone fixes | Cierra F1, F3 |
| `/coral:coral-doctor` slash command | no existe | **NUEVO** — versión determinística del skill | Power-user shortcut |

No se crea `coral-onboard-prompt` (era una pieza redundante en v1.0 de este PRD; el `SessionStart` hook + skills existentes cubren el flujo completamente).

- **FR-ONB-9** — Hook `SessionStart` ejecuta `scripts/on-session-start.sh` del plugin (ver §3.3). El script invoca `coral self-check --format=json --quick` y emite el JSON. Claude Code lo recibe en contexto y, basado en `wiki_present` / `coral_toml_present` / `warnings`, decide qué skill ofrecer al usuario:
  - `wiki_present == false` → sugiere `/coral:coral-bootstrap`
  - `warnings` no vacío → sugiere `/coral:coral-doctor`
  - `wiki_present == true && coral_toml_present == false` → ofrece `coral-onboard` (sin cambios)
  - todo OK → ofrece `coral-query` / `coral-ui` según contexto
- **FR-ONB-10** — Skill `coral-bootstrap` (actualizada): SIEMPRE ejecuta `coral bootstrap --estimate` ANTES de pedir confirmación. Muestra: "Estimated cost: $0.X (M tokens, N pages, provider: claude-sonnet) — margin of error: ±25%". Confirmación con número concreto, no rango.
- **FR-ONB-11** — Skill `coral-ui` (actualizada): después de levantar la UI, **no bloquea Claude**. Spawnea background usando `nohup coral ui serve --no-open --port 3838 > ~/.coral/ui.log 2>&1 &` (Linux/macOS) o `Start-Process -WindowStyle Hidden coral 'ui','serve','--no-open','--port','3838'` (Windows). Documenta `pkill -f "coral ui serve"` para detener. **`coral ui daemon` proper se movió a M2** porque `daemonize` crate no soporta Windows y hacer una abstracción cross-platform en M1 es scope creep.

### 6.4 Cost estimation determinista (M1)

- **FR-ONB-12** — `coral bootstrap --estimate` (nuevo flag) reporta:
  ```
  Repo size: 10,247 LOC across 142 files (78 .rs, 31 .ts, 15 .md, ...)
  Estimated pages: 47 (avg 1 per significant module)
  Estimated tokens: ~120,000 input + ~80,000 output
  Provider: claude-sonnet-4.5 (via claude CLI)
  Estimated cost: $0.42 (±15%)
  ```
- **FR-ONB-13** — Cálculo basado en heurísticas: LOC bucketed por tipo, prompts versionados con tokens conocidos, output_per_page calibrado en runs reales. Heurísticas en `crates/coral-cli/src/commands/bootstrap.rs::estimate_cost`.
- **FR-ONB-14** — Umbrales de confirmación configurables en `.coral/config.toml`:
  ```toml
  [bootstrap]
  auto_confirm_under_usd = 0.10  # default
  warn_threshold_usd = 1.00      # red warning
  ```

### 6.5 Multi-repo wizard (M2 — moved out of M1)

`coral project init --wizard` y la generación interactiva de `coral.toml` se movieron a v0.35.0 para mantener M1 entregable en 6–8 semanas. Multi-repo en M1 sigue funcionando con el flujo actual: `coral project new <name>` + editar `coral.toml` a mano + `coral project add` (CLI con flags, no interactivo).

La justificación: F6 ("multi-repo manual editing") existe pero afecta a una minoría de usuarios (team leads de monorepos) en comparación con F1–F5 que afectan al primer prompt de **cualquier** usuario nuevo. M1 prioriza fix de F1–F5, M2 hace F6.

### 6.6 Post-bootstrap automation (M1)

- **FR-ONB-17** — Después de un bootstrap exitoso, `coral-bootstrap` skill ofrece:
  1. Levantar WebUI en background usando `nohup`/`Start-Process` (default: yes, configurable via `auto_serve_ui = false` en `.coral/config.toml`).
  2. Sugerir snippet de pre-commit hook que invoque `coral ingest --apply --staged` (no instalar automáticamente).
  3. Sugerir snippet de GitHub Actions que invoque `coral test guarantee --can-i-deploy`.
- **FR-ONB-18** — `coral ui daemon` proper (start/stop/status con PID file) **movido a M2 (v0.35.0)**. Razón: `daemonize` crate no soporta Windows; cross-platform daemon requiere abstracción custom no trivial. M1 usa los workarounds de FR-ONB-11 (nohup/Start-Process); funcionan pero son menos elegantes.

### 6.7 Plugin marketplace polish (M2)

- **FR-ONB-19** — `marketplace.json` declara `keywords`, `category` y describe prerequisitos en `description` (el field `requires` no existe en el schema oficial — verificado).
- **FR-ONB-20** — ~~`installCommand` field~~ — **eliminado**. El field no existe en `marketplace.json` schema (verificado contra docs Anthropic). En su lugar, la `description` del marketplace incluye el `curl install.sh | bash` con formato `markdown`.
- **FR-ONB-21** — README del repo tiene una **sección "Getting Started in 60 seconds"** al tope con el video o GIF del flow completo.
- **FR-ONB-22** — Submission a Anthropic's official marketplace (https://claude.ai/settings/plugins/submit) — investigación previa requerida sobre el proceso.

### 6.8 Cross-platform smoke (M1)

- **FR-ONB-23** — Tests E2E del onboarding flow en CI matrix Linux + macOS + Windows: install → register plugin (mock Claude Code with `~/.claude.json` injection) → invoke skill via `claude --print` (CLI mode) → assert wiki created.
- **FR-ONB-24** — GitHub Action reusable: `coral-onboard-action` que setup-instala Coral en un CI runner. Permite a otros repos hacer `uses: agustincbajo/Coral/.github/actions/onboard@v0.34.0`.

---

## 7. Arquitectura

```
┌────────────────────────────────────────────────────────────────────┐
│  El usuario: 1 comando                                              │
│                                                                     │
│    curl -fsSL https://coral.dev/install | bash                      │
│                                                                     │
└──────────────────────────────┬─────────────────────────────────────┘
                               │
                               ▼
┌────────────────────────────────────────────────────────────────────┐
│  install.sh / install.ps1                                           │
│  (FR-ONB-1 → 5)                                                     │
│                                                                     │
│  1. Detect platform                                                 │
│  2. Download coral binary v0.34.0                                   │
│  3. Verify SHA-256                                                  │
│  4. Place on PATH                                                   │
│  5. Detect Claude Code installation                                 │
│  6. Print single-line plugin install snippet                        │
└──────────────────────────────┬─────────────────────────────────────┘
                               │
                               ▼  (user copy-pastes ONE line into Claude Code)
┌────────────────────────────────────────────────────────────────────┐
│  Claude Code                                                        │
│  /plugin marketplace add agustincbajo/Coral && /plugin install …   │
│                                                                     │
│  Plugin loaded with:                                                │
│    • mcpServers: { coral: { command: "coral", args: [...] } }       │
│    • skills: bootstrap, query, onboard, ui, doctor (NEW),           │
│              onboard-prompt (NEW)                                    │
│    • commands: /coral:bootstrap, /coral:status, /coral:doctor (NEW) │
└──────────────────────────────┬─────────────────────────────────────┘
                               │
                               ▼  (user opens a repo, asks anything)
┌────────────────────────────────────────────────────────────────────┐
│  Skill: coral-onboard-prompt  (FR-ONB-9)                           │
│                                                                     │
│  1. coral self-check --format=json                                  │
│  2. Branch:                                                         │
│     a. wiki exists?    → suggest `coral-query` or `coral-ui`        │
│     b. wiki missing?   → invoke `coral-bootstrap` flow              │
│     c. binary issue?   → invoke `coral-doctor`                      │
│     d. multi-repo hint detected? → suggest `project init --wizard`  │
└──────────────────────────────┬─────────────────────────────────────┘
                               │
                               ▼
┌────────────────────────────────────────────────────────────────────┐
│  Skill: coral-bootstrap (updated, FR-ONB-10)                        │
│                                                                     │
│  1. coral init                                                      │
│  2. coral bootstrap --estimate  ←──── NEW: deterministic $$         │
│     → "$0.42 ± 15%, 47 pages, ~200k tokens"                         │
│  3. Confirm with user (single number, not range)                    │
│  4. coral bootstrap --apply                                         │
│  5. Spawn `coral ui serve --background` (FR-ONB-17/18)              │
│  6. Suggest CI integration snippets                                 │
│  7. Done. WebUI open in user's browser.                             │
└────────────────────────────────────────────────────────────────────┘
```

### 7.1 Nuevo subcomando: `coral self-check`

**Crate**: `coral-cli` (nuevo handler).

**Output schema** (JSON):

```rust
#[derive(Serialize)]
pub struct SelfCheck {
    pub coral_version: String,
    pub binary_path: PathBuf,
    pub in_path: bool,
    pub git_repo: Option<GitRepoInfo>,
    pub wiki: Option<WikiInfo>,
    pub coral_toml: Option<ManifestInfo>,
    pub claude_cli: Option<ClaudeCli>,
    pub mcp_server: Option<McpHealth>,  // populated only if --test-mcp
    pub ui_server: Option<UiHealth>,    // populated only if --test-ui
    pub platform: PlatformInfo,
    pub warnings: Vec<Warning>,
    pub suggestions: Vec<Suggestion>,
}

#[derive(Serialize)]
pub struct Suggestion {
    pub kind: SuggestionKind,  // "install_claude_cli", "run_bootstrap", "fix_path", ...
    pub command: String,        // exact command to run
    pub explanation: String,    // 1-line context
}
```

### 7.2 Nuevo subcomando: `coral bootstrap --estimate`

Reusa la lógica existente de `coral-cli/src/commands/bootstrap.rs` que ya hace dry-run, pero añade un módulo `estimate_cost.rs`:

```rust
pub fn estimate_cost(plan: &BootstrapPlan, provider: &Provider) -> CostEstimate {
    let input_tokens = plan.entries.iter().map(|e| {
        // Heurística por tipo:
        // - Source files: LOC * 4 + 500 (prompt overhead)
        // - Markdown: tokens existentes + 200
        // - OpenAPI: schema * 6 + 800
        token_estimate_per_entry(e)
    }).sum();
    let output_tokens = plan.entries.len() * AVG_OUTPUT_PER_PAGE;
    let usd = match provider {
        Provider::Claude => input_tokens * 0.003 / 1000.0 + output_tokens * 0.015 / 1000.0,
        Provider::Gemini => ...,
        Provider::Local => 0.0,
    };
    CostEstimate { input_tokens, output_tokens, usd, margin_of_error_pct: 15 }
}
```

Calibración: medir 5 runs reales y ajustar las constantes hasta ±15% sea verdadero.

### 7.3 Nueva skill: `coral-doctor`

Path: `.claude-plugin/skills/coral-doctor/SKILL.md`

Behavior:
1. Llama `coral self-check --format=json` (vía Bash tool).
2. Parsea el output.
3. Por cada `warning` o falta de prerequisite, prepara una respuesta estructurada:
   ```
   Coral needs <X> to <Y>. To fix:

     $ <exact command>

   This will <consequence in 1 sentence>. Want me to run it? (y/n)
   ```
4. Si el usuario acepta cada uno, ejecuta y re-runs `coral self-check`.
5. Cuando todo está verde, devuelve "Coral is ready. Try `/coral:coral-bootstrap` next."

### 7.4 Nueva skill: `coral-onboard-prompt`

Auto-invocable con triggers muy amplios — efectivamente, cualquier prompt del usuario en una sesión donde el plugin está recién instalado **y** el repo actual no tiene wiki configurada.

Su trabajo no es ejecutar; es **enrutar**: ofrecer al usuario uno de 3 caminos (bootstrap si nuevo, query si ya hay wiki, doctor si algo está roto). Como un router en HTTP terms.

### 7.5 `coral ui daemon` (subcomando nuevo)

```bash
coral ui daemon start [--port 3838] [--token TOKEN]   # fork + detach + write PID
coral ui daemon stop                                   # SIGTERM via PID file
coral ui daemon status                                 # alive? PID? port? uptime?
```

Implementación: `daemonize` crate (Linux/macOS); en Windows, `CreateProcess` con `DETACHED_PROCESS` flag. PID file en `~/.coral/ui.pid`. Lockfile via `fs4` (ya en deps).

### 7.6 Sincronización de versión plugin↔binary (FR-ONB-5)

**Step de prep (ejecutado UNA vez antes de M1 — no es parte del sprint)**: sincronización manual de `plugin.json` (v0.32.3 actual) y `marketplace.json` (v0.30.0 actual) a v0.33.0 (el binary actual). Esto fixea la desincronización legacy para usuarios entre v0.33.0 y v0.34.0.

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

**Guardia anti-loop** (critical):
1. El commit tag `[skip ci]` está en el subject line del commit message. GitHub Actions reconoce este pattern por default y NO dispara el workflow para ese commit (`ci.yml` + `ui-build.yml` ya lo soportan implícitamente; el doc oficial: https://docs.github.com/en/actions/managing-workflow-runs/skipping-workflow-runs).
2. Adicionalmente, en `ci.yml` agregamos paths-ignore explícito para el job `release-only`:
   ```yaml
   on:
     push:
       branches: [main]
       paths-ignore:
         - '.claude-plugin/**'
   ```
   (Solo cambiamos paths-ignore en main; los PRs siguen corriendo todo.)
3. El push del workflow usa `GITHUB_TOKEN` con scope default (no PAT) — `release.yml` no triggerea otros workflows con GITHUB_TOKEN por restricción explícita de GitHub Actions.

**Verificación post-release**: el commit `[skip ci]` aparece en main pero NINGÚN otro workflow se dispara. Si por algún motivo se dispara, el rollback es revert del commit + tag de hotfix.

---

## 8. Stack y dependencias

### Backend (Rust)
- Sin nuevas dependencies. Reusamos `serde`, `clap`, `chrono` ya en workspace.
- `daemonize` crate para `coral ui daemon` (Linux/macOS): ~25 KB, single-purpose. Opt-in feature flag `daemon`.
- Windows: `winapi`/`windows-sys` ya en tree via `signal-hook` transitive.

### Frontend (SPA)
- Sin cambios en este sprint. La SPA de v0.32+ ya funciona.

### Skills (Markdown)
- Sin nuevas dependencies. Markdown puro.

### Scripts
- `install.sh` / `install.ps1` modificados — añaden detección de Claude Code, mensaje formateado de un solo line.

---

## 9. Fases / hitos

### Prep (1 commit, antes de M1)
- Sincronizar `plugin.json` (v0.32.3) y `marketplace.json` (v0.30.0) → v0.33.0. Una sola commit manual + push.

### M1 — v0.34.0 (6–8 semanas, 1 dev)

| Semanas | Entregables |
|---|---|
| 1–2 | `coral self-check` subcommand (Rust crate `coral-cli`) + `SessionStart` hook script + tests unitarios. Verificar Bash/PowerShell scripts cross-platform. |
| 2–3 | `coral bootstrap --estimate` con heurísticas locales calibradas (n=10 runs reales mínimo). Update `coral-bootstrap` skill. Bucketed model por tipo de archivo. |
| 3–4 | Nueva skill `coral-doctor` + slash command `/coral:coral-doctor`. `SessionStart` hook integrado en `plugin.json`. Update `coral-ui` skill con background spawn (nohup/Start-Process). |
| 4–5 | Sync versions automation en `release.yml` con guardia `[skip ci]`. Update `install.sh` / `install.ps1` con detección Claude Code + 3-line copy-paste output. |
| 5–6 | E2E tests cross-platform via `claude --print` mock injection. Matrix Linux + macOS + Windows. |
| 6–8 | Docs ("Getting Started in 60 seconds" section), GIF/video, README updates, release v0.34.0. Buffer para issues que surjan en cross-platform tests. |

### M2 — v0.35.0 (4–5 semanas)

- `coral project init --wizard` interactivo (dialoguer crate).
- `coral ui daemon start/stop/status` cross-platform (custom Windows abstraction).
- `coral bootstrap --estimate` calibrado a ±15% con n≥30 datapoints.
- Submission a Anthropic official marketplace.
- `coral-onboard-action` reusable GitHub Action.

### M3 — v0.36.0+ (futuro)

- Cross-agent onboarding optimization (Cursor, Continue, Cline).
- `coral mcp serve --transport http` shared-MCP default.
- Coral Cloud (PRD separado).

---

## 10. Métricas de éxito

| KPI | Baseline (v0.33.0) | Target M1 | Target M3 |
|---|---|---|---|
| Time-to-first-wiki-query, repo 10k LOC, conexión normal, claude-sonnet provider | ~30 min (manual reading) | **≤ 10 min** (incluye install 30s + plugin paste 30s + first prompt 15s + bootstrap LLM ~5min + UI spin 5s + first query ~30s; bootstrap es el dominante) | ≤ 3 min (con local-LLM provider) |
| Manual commands entre `curl install.sh` y wiki funcional | 4 separadas, sin guía | **3 paste-lines (no chain)** + Claude conduce el resto via SessionStart hook | 2 lines |
| `coral bootstrap` cost variance (estimate vs actual) | N/A (rango opaco) | **±25%** con n=10 calibration runs | ±15% con n≥30 |
| `coral self-check` cobertura de issues | N/A | **8 checks** (binary, claude CLI, git, wiki, manifest, MCP, UI, platform) | +3 nuevos |
| Test coverage del nuevo código en `coral-cli` | N/A | **≥ 75%** | ≥ 85% |
| Skills nuevas funcionales E2E en Claude Code mock | 0 | **1 (doctor)** + 1 hook (SessionStart) + 1 slash command (`/coral:coral-doctor`) | + wizard, + daemon control |

> **Assumptions del KPI principal "≤10 min":** binary install 30s, paste-3-lines 30s, first user prompt to Claude 15s, `coral init` 1s, `coral bootstrap --estimate` 2s, user confirmation 30s, `coral bootstrap --apply` 3–5 min sobre repo 10k LOC con `claude-sonnet-4.5` (escala con tamaño y rate limits del provider), UI spin 5s, first `coral query` 5–30s. El bootstrap es 80% del tiempo y no es comprimible localmente; el target ≤3 min de M3 asume local-LLM provider o cache pre-built shipped con el plugin (idea futura).

---

## 11. Decisiones resueltas (post-review)

1. ~~**¿`install.sh` auto-registra el plugin?**~~ → **RESUELTO: NO en M1.** Mutar `~/.claude.json` directamente desde un script externo es frágil + viola separation of concerns. Las 3 paste lines son aceptables. En M2 evaluamos si Anthropic publica `claude plugin install --headless`.

2. ~~**¿`coral-onboard-prompt` skill triggers amplios?**~~ → **RESUELTO: la skill se eliminó.** Usamos `SessionStart` hook (que sí existe en Claude Code, verificado) en lugar de auto-invocación NLP. Detección determinística, sin falsos positivos.

3. ~~**¿`coral bootstrap --estimate` llama al LLM?**~~ → **RESUELTO: NO.** Heurísticas locales. M1 target ±25% con `n=10`; M2 calibra a ±15% con `n≥30` y opt-in feedback loop (sin telemetría).

4. ~~**¿`coral ui daemon` feature-gated?**~~ → **RESUELTO: out of M1.** `daemonize` crate no soporta Windows; abstracción custom es scope creep. M1 usa `nohup`/`Start-Process`. M2 hace daemon proper.

Decisiones aún abiertas (a resolver en M1 kickoff):

5. **¿`SessionStart` hook script en `scripts/on-session-start.sh` o inline en `plugin.json`?**
   - **Inline JSON**: ~200 chars; sin archivo extra; harder to debug.
   - **Archivo separado**: legible; debuggable con `bash -x`; un archivo más en `.claude-plugin/scripts/`.
   - **Recomendación tentativa:** archivo separado (mejor DX para contributors).

6. **¿`extraKnownMarketplaces` en `.claude/settings.json` del repo Coral mismo, para que clonar el repo auto-añada el marketplace?**
   - **Pro:** dogfooding + onboarding más simple para contributors.
   - **Contra:** archivo nuevo en el repo; puede ser ruidoso si Claude Code prompts al user al abrir el repo.
   - **Recomendación tentativa:** sí, con un comentario claro de "this is for Coral contributors; remove if forking".

---

## 12. Riesgos y mitigaciones

| # | Riesgo | Probabilidad | Impacto | Mitigación |
|---|---|---|---|---|
| R1 | Anthropic cambia el plugin schema durante el sprint | B | M | Pin a `code.claude.com/docs` snapshot; subscribe a release notes; tests E2E corren contra Claude Code real, no mock |
| R2 | `coral bootstrap --estimate` se equivoca > ±30% en repos no calibrados | M | M | Mensaje claro "esto es estimación, no factura"; usuario siempre confirma; añadir feedback loop en `~/.coral/actual-vs-estimate.jsonl` |
| R3 | `coral-onboard-prompt` skill genera false positives (auto-invocable demasiado amplio) | M | M | Trigger restrictivo (primera sesión, repo sin wiki); test E2E que verifique que NO se invoca cuando wiki existe |
| R4 | `coral ui daemon` fail en Windows (cross-platform process management) | M | M | Implementar y testear PRIMERO en Windows (donde es más complejo); fallback explícito "daemon mode not supported on this platform → fg" |
| R5 | Sincronización plugin↔binary versions en release.yml falla | B | A | El step de sync corre ANTES del cargo build; falla del job aborta el release (no se publica binary desincronizado) |
| R6 | Marketplace de Anthropic rechaza Coral por security/policy | B | A | Conversación con Anthropic ANTES de submission; documentar threat model + security model |
| R7 | Usuarios reportan que el flow es "más complejo, no menos" | M | A | A/B test con 5 usuarios reales antes del release; medir TTV con cronómetro |

---

## 13. Anti-features (explícitos)

- **AF-1** — No habrá `phone home` telemetry. Cero datos enviados.
- **AF-2** — No habrá auto-bootstrap sin confirmación de costo (incluso si `auto_confirm_under_usd` está configurado, la primera vez que un usuario lo activa hay un prompt).
- **AF-3** — No habrá download/install automático del binary desde el plugin (Anthropic policy + security).
- **AF-4** — No habrá UI propia para gestionar el plugin (usamos `/plugin` de Claude Code).
- **AF-5** — No habrá Coral Cloud en este sprint.
- **AF-6** — No habrá cross-agent onboarding optimizado (Cursor/Continue/Cline) — M3 trabajo.
- **AF-7** — No habrá WYSIWYG editor para `coral.toml` — sigue siendo CLI/TOML.

---

## 14. Apéndice A: Fricciones actuales detectadas (F1–F10)

Ver §2.2 arriba.

---

## 15. Apéndice B: Plan de implementación detallado M1

```
Semana 1 — coral self-check + sync versions
  - Crear crates/coral-cli/src/commands/self_check.rs
  - Subcommand definition en clap (main.rs)
  - JSON schema definido en serde structs
  - 6 unit tests: binary detection, git repo, wiki present/absent, claude CLI present/absent, MCP health, UI health
  - release.yml: añadir step sync-plugin-manifests con jq
  - Sync plugin.json + marketplace.json a v0.34.0 (manual una vez)

Semana 2 — Cost estimation + bootstrap skill update
  - Crear crates/coral-cli/src/commands/bootstrap/estimate.rs
  - Heurísticas: token_estimate_per_entry(entry: &Entry) -> u64
  - Calibration data: 5 runs reales (small Rust, mid TS, large Python, monorepo, sample OpenAPI)
  - --estimate flag en clap
  - Skill .claude-plugin/skills/coral-bootstrap/SKILL.md updated: SIEMPRE --estimate antes de --apply
  - Mensaje formato: "$X.YZ (±15%), N páginas, M tokens, provider: P"

Semana 3 — Doctor + onboard-prompt skills + ui daemon
  - .claude-plugin/skills/coral-doctor/SKILL.md (new)
  - .claude-plugin/skills/coral-onboard-prompt/SKILL.md (new)
  - .claude-plugin/commands/coral-doctor.md (slash command)
  - coral-cli/src/commands/ui_daemon.rs (new): start/stop/status subcommands
  - Daemonize crate added with feature flag
  - PID file management at ~/.coral/ui.pid
  - 4 unit tests: spawn detached, write PID, kill via PID, status alive/dead

Semana 4 — Cross-platform tests + wizard + release
  - E2E test via claude --print + mock JSON injection
  - Test matrix: ubuntu-latest, macos-latest, windows-latest
  - coral project init --wizard subcommand (interactive prompts via dialoguer crate)
  - Validation: schema check, lock generation, project doctor at end
  - Docs: README "Getting Started in 60 seconds" section
  - Release notes draft
  - Tag v0.34.0 → release.yml runs
```

---

## 16. Criterios de aceptación (DoD M1)

1. `curl install.sh | bash` (Linux/macOS) o `iwr install.ps1 | iex` (Windows) instala el binario v0.34.0 y al final imprime las **3 líneas copy-pasteables** + escribe `.coral/claude-paste.txt` con el mismo contenido.
2. Esas 3 líneas (pegadas de a una en Claude Code) dejan el plugin operativo.
3. Al abrir Claude Code en un repo SIN wiki, el **`SessionStart` hook** ejecuta `coral self-check --quick` automáticamente; Claude propone arrancar `coral-bootstrap` sin que el usuario tenga que escribir nada específico.
4. `coral bootstrap --estimate` muestra costo en USD con **±25% de margen empírico** verificable contra 10 runs de calibración.
5. Después de aceptar el bootstrap, la WebUI se levanta en background (`nohup`/`Start-Process`) y abre browser en `/pages`.
6. `coral self-check --format=json` devuelve estructura completa con `warnings` y `suggestions` accionables, cubriendo 8 checks documentados.
7. La nueva skill `coral-doctor` + slash command `/coral:coral-doctor` están registrados y funcionan E2E.
8. CI matrix Linux + macOS + Windows todos verdes con tests E2E del onboarding via `claude --print` mock.
9. `plugin.json` y `marketplace.json` ambos en v0.34.0, sincronizados automáticamente por `release.yml` con guardia anti-loop `[skip ci]`.
10. README tiene sección "Getting Started in 60 seconds" con GIF/video.
11. **BC sagrada**: todas las skills/commands existentes (`coral-bootstrap`, `coral-query`, `coral-onboard`, `coral-ui`) siguen funcionando idénticas a v0.33.0; lo nuevo es aditivo.
12. **Items movidos a M2 (NO bloquean M1)**: `coral ui daemon`, `coral project init --wizard`, ±15% estimate accuracy, marketplace submission a Anthropic.

---

*Fin del PRD v1.1 — incorpora 1 fix mayor + 9 fixes menores del review independiente:*
*(mayor) Re-arquitectura usando `SessionStart` hook en lugar de skill auto-invocable amplia. El hook existe en Claude Code (verificado contra docs oficiales) y resuelve F4 sin falsos positivos.*
*(menor 1) FR-ONB-4 corregido: 3 líneas separadas, no chain `&&` (no soportado).*
*(menor 2) FR-ONB-20 eliminado: `installCommand` field no existe en marketplace.json schema.*
*(menor 3) §3.2 reestructurado con tabla de primitives reusables / no disponibles.*
*(menor 4) §7.6 commit-back loop guardia explícita con `[skip ci]` + paths-ignore.*
*(menor 5) Sync v0.32.3→v0.33.0 movido a prep (1 commit antes de M1), no a Semana 1.*
*(menor 6) `coral ui daemon` + `project init --wizard` movidos a M2 (Windows daemon es scope creep).*
*(menor 7) O4 bajado de ±15% a ±25% en M1; ±15% es target M2 con n≥30 datapoints.*
*(menor 8) Timeline M1 ampliado de 3–4 → 6–8 semanas para un dev.*
*(menor 9) Tabla explícita skills antes/después; `coral-onboard-prompt` eliminado.*

*PRD v1.0 (mismo día): primera redacción + review independiente con findings 1 crítico + 4 cuestionables + 7 huecos.*
