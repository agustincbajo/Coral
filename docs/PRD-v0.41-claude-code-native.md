# PRD — Coral v0.41: Claude Code-Native Mode + Verbose Observability

**Versión del documento:** 1.0 (draft)
**Fecha:** 2026-05-16
**Autor:** Agustín Bajo (con asistencia)
**Estado:** Borrador para discusión — no implementar hasta sign-off.
**Versión objetivo:** Coral v0.41.0 (M-CCN, 1-2 semanas)

---

## 1. Problema

El BACKLOG #12 (cerrado en v0.40.1+v0.40.2) hizo el **install** autónomo
y dejó mensajes accionables cuando coral corre dentro de Claude Code,
pero NO resolvió el problema de fondo: las operaciones LLM de coral
(`bootstrap`, `query`) **no funcionan** desde una shell de Claude Code
porque el binario `claude` detecta su proceso padre vía macOS Endpoint
Security responsibility tracking y entra en "host-managed mode",
ignorando la credencial del keychain y devolviendo 401.

Hoy el usuario que quiere dogfoodear Coral desde la misma sesión de
Claude Code donde está desarrollando tiene tres opciones, todas malas:

1. Abrir otra Terminal — rompe el flow y duplica contexto.
2. Configurar `[provider.gemini]` o `--provider http` con un endpoint
   OpenAI-compatible — requiere setup independiente, ajeno al journey.
3. Usar las skills (`/coral:coral-bootstrap` etc.) que internamente
   shellan a `coral bootstrap` — **mismo wall**, falla igual.

Adicionalmente, la salida de coral hoy es predominantemente silenciosa
o `tracing::info!`-style key=value JSON-ish que no le dice al usuario
**qué está haciendo** ni **qué obtuvo**. Para una herramienta que está
gastando tokens del usuario (vía Max sub o API key), la opacidad es UX
mala.

## 2. Goals

### Funcionales

- **G1.** `coral bootstrap` (y `query`, y cualquier otra op LLM)
  invocada desde dentro de Claude Code usa la sesión host como motor de
  inferencia. NO spawna `claude` CLI.
- **G2.** Las skills (`coral-bootstrap`, `coral-query`, `coral-onboard`)
  invocadas desde Claude Code completan su trabajo end-to-end usando
  la sesión actual como LLM, sin pegar contra el wall de v0.40.x.
- **G3.** Cada operación de coral emite progreso humano-legible:
  - Línea **antes** de cada paso: "▶ Doing X (model=Y, cwd=Z)..."
  - Línea **después** de cada paso: "✓ Got Y (1.2k in / 850 out tokens,
    $0.0085, 2.4s)" — o "✗ failed: <razón>".
- **G4.** Flag global `--verbose` (y env `CORAL_VERBOSE=1`) eleva el
  progreso para incluir prompt + respuesta crudos, útil para debug y
  observability cuando el usuario quiere ver QUÉ exactamente le pidió
  coral al modelo.
- **G5.** El path legacy (coral desde una Terminal normal, vía
  `claude` CLI o `--provider gemini` o `--provider http`) sigue
  funcionando sin regresión.

### No-funcionales

- **NF1.** Backwards compat: schemas de `.coral/config.toml`,
  `SelfCheck` JSON, exit codes, todos sin cambios.
- **NF2.** No nuevas deps mandatorias para el core. La pieza nueva
  (host-sampling runner) reutiliza el protocolo MCP que coral-mcp ya
  habla.
- **NF3.** Verbose por defecto vs opt-in: progreso *antes/después*
  ON-by-default (es lo que el usuario explícitamente pidió);
  prompt-dump (G4) OFF-by-default, opt-in vía `--verbose` o env.
- **NF4.** Coverage floor existing (65% líneas) no regresa.

## 3. Non-goals

- **NG1.** Telemetría / phone-home. La verbose output va a stderr en la
  máquina del usuario, no se envía a Anthropic ni a Coral.
- **NG2.** Re-arquitectura del modelo de runners (Trait `Runner` queda
  intacto; agregamos un nuevo impl).
- **NG3.** Soporte para Claude Code SDK Python — la integración es vía
  protocolo MCP estándar, no SDK-específica.
- **NG4.** Multi-sesión / multi-host orchestration.
- **NG5.** Cambios al pricing model o cost engine — sólo cambia el
  routing de la inferencia.

## 4. Diseño

### 4.1 Parte A — `HostSamplingRunner`: usar la sesión Claude Code

**Background sobre MCP sampling.** El protocolo MCP define la
operación `sampling/createMessage`: un MCP server (acá coral-mcp)
puede pedirle a su cliente (acá Claude Code) que realice una
inferencia LLM. El cliente decide si aceptar y devuelve el resultado.
Esto es exactamente la inversión que necesitamos: en lugar de coral
spawnear `claude`, coral le pide a Claude Code que haga el llamado.

**Flujo:**

```
┌───────────────┐    coral bootstrap --apply        ┌───────────────┐
│ User en       │  ─────────────────────────────►  │  coral CLI    │
│ Claude Code   │                                  │  (subprocess) │
└───────▲───────┘                                  └───────┬───────┘
        │ tool call: coral_bootstrap_page                  │
        │  body=`# alpha\n\n...`                            │
        │                                                  │
        ▼                                                  ▼
┌───────────────┐  sampling/createMessage          ┌───────────────┐
│ Claude Code   │  ◄─────────────────────────────  │ HostSampling  │
│ (host LLM)    │  ─────────────────────────────►  │ Runner        │
│                                                  │ (in-process)  │
└───────────────┘     response: page body         └───────────────┘
```

**Detección + selección.** En `runner_helper::make_runner`:

1. Si `--provider` está explícito, honrarlo (override manual).
2. Si `CORAL_RUNNER=host-sampling` está seteado, usar
   `HostSamplingRunner`.
3. Si `CLAUDECODE=1` está seteado Y el MCP server tiene una conexión
   activa con sampling capability, **default** a `HostSamplingRunner`.
4. Sino, fallback al runner por proveedor configurado en
   `.coral/config.toml` (path actual v0.34.x).

**Impl:**

```rust
// crates/coral-runner/src/host_sampling.rs (NEW)
pub struct HostSamplingRunner {
    /// MCP transport handle. Resolved at runner construction.
    mcp_client: McpClientHandle,
    /// Preferences forwarded as `modelPreferences` in the sampling
    /// request — costPriority, speedPriority, intelligencePriority.
    prefs: ModelPreferences,
}

impl Runner for HostSamplingRunner {
    fn run(&self, prompt: &Prompt) -> RunnerResult<RunOutput> {
        // 1. Construct sampling/createMessage request from Prompt.
        // 2. Send via mcp_client.
        // 3. Wait for response (timeout via prompt.timeout).
        // 4. Map MCP CreateMessageResult to RunOutput, including
        //    extraction of token usage from the result's usage field.
    }
    fn run_streaming(...) -> RunnerResult<RunOutput> {
        // Streaming via MCP `notifications/progress` — emit on_chunk
        // for each partial.
    }
}
```

**Permissions.** MCP sampling es opt-in: el cliente puede rechazar la
request. En Claude Code, el usuario verá un prompt de consentimiento
("Coral wants to use this session for inference"). Ese prompt forma
parte de la UX deliberadamente — el usuario sabe que su Max sub paga
los tokens.

**Costing.** El cost engine sigue siendo el de coral. Token counts
vienen del response del host. Si el host no reporta usage, fallback a
heurística (lo mismo que LocalRunner hoy).

### 4.2 Parte B — Verbose-by-default observability

**Hoy.** Coral usa `tracing` con info-level por default. La salida
parece `2026-05-16T17:24:29Z INFO wrote SCHEMA.md path=.wiki/SCHEMA.md`
— bien para CI, mala para humanos.

**Nuevo.** Macro `progress!` con dos forms:

```rust
// Antes de la op
progress!(step, "Bootstrapping page {slug}"; model = model);
// Después de la op
progress!(done, step, "page persisted"; cost_usd = 0.0085, tokens_in = 1200, tokens_out = 850, secs = 2.4);
// Error
progress!(fail, step, "claude returned 401"; hint = "..");
```

Renderiza así por default (TTY, no `--quiet`):
```
▶ Bootstrapping page `modules/runner.md`  (model: claude-sonnet-4-5)
✓ done in 2.4s  (1.2k in / 850 out tokens, $0.0085)
```

Y con `--verbose`:
```
▶ Bootstrapping page `modules/runner.md`  (model: claude-sonnet-4-5)
  prompt (1184 tokens):
    System: You are a Coral wiki writer...
    User: Generate the module page for crates/coral-runner/src/runner.rs
          [...file contents pasted...]
  response (850 tokens):
    # Runner
    The `Runner` trait abstracts over the four LLM backends...
✓ done in 2.4s  (1.2k in / 850 out tokens, $0.0085)
```

**Where to thread.** Each runner's `run()` already has the prompt +
response in scope. The verbose dump goes through the new macro at the
runner boundary so EVERY runner (Claude, Gemini, Local, Http,
HostSampling) gets it for free. Cost calculation already exists in
`cost.rs`; just plumb it into the progress event.

**Quiet mode.** `--quiet` (already exists) suppresses progress!. CI
behavior unchanged.

### 4.3 Parte C — Skills update

Las 5 skills actuales asumen que `coral bootstrap`/`query`/etc.
funcionan standalone. Con la Parte A, **no necesitan cambios funcionales** — sólo necesitan agregar
una validación inicial:

```markdown
1. Run `coral self-check --quick --format=json`.
2. If `runner_selected != "host-sampling"` AND `CLAUDECODE=1`, show
   warning: "Coral is about to spawn the `claude` CLI which will fail
   inside Claude Code. Run `export CORAL_RUNNER=host-sampling` or
   continue (and let the L3 gate take over)."
3. ...rest of existing skill body unchanged
```

Net effect: las skills ahora "just work" porque el runner por debajo
ya es el correcto.

## 5. Phased implementation

| Fase | Scope | LOC est. | Time |
|---|---|---|---|
| **P1** | `progress!` macro + plumbing en runners existentes (Parte B excluyendo verbose-prompt-dump) | ~300 | 1d |
| **P2** | `--verbose` flag + prompt-dump (Parte B completa) | ~150 | 0.5d |
| **P3** | `HostSamplingRunner` skeleton + `make_runner` wiring (Parte A sin streaming) | ~500 | 1.5d |
| **P4** | MCP server side: aceptar `sampling/createMessage` y rutearlo (Parte A end-to-end) | ~400 | 1d |
| **P5** | Streaming (`run_streaming` + MCP progress notifications) | ~300 | 1d |
| **P6** | Skills update + docs + CHANGELOG | ~100 | 0.5d |
| **P7** | Tests E2E (HostSampling via mock MCP client) | ~400 | 1d |

**Total: ~6.5 days.** Sized to fit a v0.41 sprint comfortably.

**Phase ordering rationale.** P1+P2 land first because they're
self-contained and give immediate UX win (verbose output) regardless
of host-sampling. P3+P4 are the architectural piece. P5 is streaming
polish. P6+P7 close out.

## 6. Acceptance criteria

- **AC1.** From a Claude Code shell on macOS Sequoia, `coral
  bootstrap --estimate` returns a real cost estimate without invoking
  the `claude` binary. No 401. No host-managed-mode error.
- **AC2.** `coral bootstrap --apply --max-cost=0.10` in a tiny test
  repo from inside Claude Code generates 1-2 pages using the host
  session, deducts the tokens from the host's quota (Max sub or API
  key), and persists pages via the normal `WikiIndex` write path.
- **AC3.** From a plain Terminal, the same commands work via the
  legacy claude_cli path (no regression).
- **AC4.** `coral --verbose bootstrap --estimate` shows per-step
  progress, model used, token counts, costs.
- **AC5.** `coral self-check` reports the selected runner kind in a
  new field `runner_selected: "host_sampling" | "claude_cli" | ...`.
- **AC6.** New tests pass: `host_sampling_runner` unit suite + E2E
  with a mock MCP client.
- **AC7.** Existing 527 workspace tests stay green (no regression).
- **AC8.** Coverage floor 65% holds.

## 7. Risks + open questions

### Riesgos

- **R1.** Claude Code's MCP host implementation might not support
  `sampling/createMessage`. The MCP spec is clear, but optional features
  are optional. Mitigation: P3 ships behind a feature gate; if
  unsupported, fall back gracefully with a clear error pointing to the
  `--provider gemini`/`--provider http` runners.
- **R2.** Sampling permission prompt UX (host-side consent dialog)
  could be intrusive. Mitigation: per-session consent (one prompt per
  Claude Code session, not per coral op). Users can preauthorize via a
  Claude Code setting if Anthropic exposes one.
- **R3.** Cost transparency: token usage from sampling response is
  host-reported and might not be byte-exact with Anthropic's billing.
  Acceptable for estimation; flag this in the progress output ("~"
  prefix for token counts from host-sampling).
- **R4.** Verbose output volume could be overwhelming on big
  bootstraps (50 pages × full prompt dumps). Mitigation: `--verbose`
  caps dump size per call to 2k chars by default;
  `--verbose=full` un-caps.

### Open questions

- **Q1.** **Does Claude Code's MCP host implement `sampling/createMessage`?**
  Need to test empirically before greenlighting P3+P4. Spike: ship a
  toy MCP server with a sampling request, run it from Claude Code, see
  what happens. If unsupported → de-scope to Parts B+C only and revisit
  the host-sampling piece when Anthropic ships it.
- **Q2.** **Default verbose level: just progress, or include 1-line
  per-tool-call summaries?** Leaning toward just progress (G3) and
  letting `--verbose` add the prompt/response detail. But the user
  asked for "siempre te diga qué está haciendo y qué obtiene", so
  default should be MORE than just timing — probably include result
  summaries (page slug created, query answer length, etc.) at the
  default verbosity.
- **Q3.** **Should `--verbose` also apply to read-only ops?** (lint,
  stats, self-check). Probably yes for consistency, even though those
  are already fast and informative.
- **Q4.** **HostSampling + cost gate (`--max-cost`):** when the host
  reports usage, do we trust it for the mid-flight gate, or run our
  own pre-estimation? Suggest: trust host post-hoc, keep pre-estimate
  for gate.

### Validación necesaria antes de implementar

1. Spike (1-2h) confirmando que Claude Code v2.1.140 acepta
   `sampling/createMessage` desde un MCP server stdio.
2. Decisión definitiva sobre Q1 + Q2 antes de empezar P3.
3. Si la spike es negativa, este PRD se reduce a Parts B+C (verbose +
   skills update) y se mantiene "host-sampling" como follow-up.

## 8. Apéndice: superficies de código relevantes

- `crates/coral-runner/src/runner.rs` — `Runner` trait + `ClaudeRunner`.
  Nuevo impl: `HostSamplingRunner` en `host_sampling.rs`.
- `crates/coral-cli/src/commands/runner_helper.rs:195` —
  `make_runner` dispatch. Agregar branch para host-sampling detection.
- `crates/coral-mcp/src/server.rs` — MCP server. Agregar manejo de
  sampling/createMessage como cliente (coral-mcp es el server, pero
  para sampling el server emite requests al host).
- `crates/coral-cli/src/commands/self_check.rs:319` — `probe_*` se
  amplía con `probe_runner_selection` para AC5.
- Macros `progress!` en `coral-core` (nuevo módulo `observability.rs`).
- Plugin skills bajo `.claude-plugin/skills/` — minor edit per Parte C.

---

## Anexo A: Validación contra el spec MCP

(Pegar acá la referencia oficial de `sampling/createMessage` y los
shape de request/response cuando se haga la spike de Q1.)

## Anexo B: comparación con alternativas descartadas

- **AnthropicHttpRunner:** runner nuevo que habla Messages API directo
  a `api.anthropic.com`. Bypassa claude_cli completamente. Descartada
  como solución primaria porque requiere sk-ant-api03 key (no Max sub).
  Acceptable como FOLLOW-UP separado si hay demanda de usuarios.
- **Skill-driven plan/execute split:** la skill genera el plan, el
  usuario aprueba, la skill ejecuta cada página vía el modelo de
  Claude Code, y llama a `coral page upsert` para persistir.
  Equivalente funcional a HostSampling pero requiere reescritura
  significativa de las skills + protocolo skill↔CLI nuevo. Descartada
  por complejidad.
- **Anti-laundering bypass del provenance + claude detection:**
  imposible per nuestra exploración del 2026-05-16 (BACKLOG #12 L4).

---

**Fin del PRD v1.0 draft.** Discusión esperada sobre Q1-Q4 antes de
empezar P1.
