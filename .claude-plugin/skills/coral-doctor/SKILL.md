---
name: coral-doctor
description: Diagnose Coral installation / provider config and offer exact fixes. Auto-invoke when the user says "coral broke", "coral not working", "fix coral", "coral is broken", "what's wrong with coral", "coral doctor", when the SessionStart hook reports warnings, or any prompt mentioning "coral" that touches setup, config, or errors. Walks the user through a 4-path provider wizard (Anthropic API key / Gemini / Ollama / install claude CLI) when no provider is configured. Always shows the exact fix command before running it and asks for consent.
disable-model-invocation: false
allowed-tools: Bash(coral:*), Bash(ls:*), Bash(test:*)
---

# Coral doctor

Diagnose the Coral environment for the current repo and walk the user through a fix for anything broken.

## Flow

### Step 1 — probe the environment

Run:

```bash
coral self-check --format=json
```

Parse the JSON. The schema is pinned at `schema_version: 1` (see `docs/PRD-v0.34-onboarding.md` Appendix F). The fields you care about:

- `coral_status` (`"ok"`, `"binary_missing"`, `"check_failed"`)
- `providers_configured` (array of strings; empty = no provider yet)
- `providers_available` (what the wizard *could* configure)
- `warnings` (array of `{severity, message, action}`)
- `suggestions` (array of `{kind, command, explanation}`)

### Step 2 — empty `providers_configured` → run the wizard

If `providers_configured == []`, the user has no working LLM provider. Tell them what's missing in one sentence and offer the wizard:

> "Coral is installed but no LLM provider is configured. I can run `coral doctor --wizard` — it'll ask you to pick one of: Anthropic API key, Gemini API key, Ollama (local, free), or install the `claude` CLI. Want me to run it? (y/n)"

If they say yes, run:

```bash
coral doctor --wizard
```

This is interactive — Coral handles all the prompts. After it returns, re-run `coral self-check --format=json --quick` to confirm a provider landed in `providers_configured`. If yes, route back to `coral-bootstrap` so they can compile their wiki.

### Step 3 — per-warning offer a fix

For each entry in `warnings[]`, show the user the exact command from `warnings[].action` and ask: *"Want me to run it? (y/n)"*. Examples:

- `coral` not on PATH → `scripts/install.sh` (or `install.ps1` on Windows).
- WebUI not reachable → suggest `coral ui serve --no-open --port 3838 &` (background spawn — the `coral-ui` skill handles this).
- MCP probe failed → suggest `coral mcp serve --transport=stdio` smoke test.

Always show the literal command — never paraphrase. Users copy/paste these.

### Step 4 — everything green

If `warnings == []` and `providers_configured != []`:

> "Coral is ready. Try `/coral:coral-bootstrap` next to compile a wiki for this repo."

## When NOT to run

- The user asked a code-content question (e.g. *"how does jwt-validation work"*). That's `coral-query`, not the doctor. Only invoke the doctor when the user is debugging Coral itself.
- The self-check already came back clean and the user didn't report a problem. Don't pre-emptively run diagnostics.

## Failure modes

- **`coral` not in PATH** → the JSON probe itself fails. Tell the user *"Coral binary not found on PATH. Run `scripts/install.sh` (or `scripts/install.ps1` on Windows) — pre-built binaries land in `~/.coral/bin/`."*
- **`coral self-check` exits non-zero** → that's an internal error (exit 3). Surface stderr and suggest `coral self-check --print-schema` as a sanity probe.
- **Wizard ping fails** (HTTP 401 from Anthropic or Gemini) → the wizard prints `FAILED: HTTP 401 …` and does **not** write the key. Tell the user to double-check the key and re-run.
- **Ollama: no `ollama` on PATH** → wizard prints install URL. Don't try to install Ollama for them.
- **Ollama: `llama3.1:8b` not pulled** → wizard prints `ollama pull llama3.1:8b` and asks them to re-run. The wizard intentionally does **not** auto-pull (4.7 GB download with no progress visible through dialoguer).

## Reference

- Subcommand: `coral doctor --help`.
- The slash command `/coral:coral-doctor` invokes the same flow deterministically (`disable-model-invocation: true`).
- Schema contract: PRD v1.4 Appendix F. Fields are additive — unknown fields MUST be tolerated.
- Wizard rationale: PRD §7.4 (FR-ONB-27 + FR-ONB-28). 4 paths cover both Claude-Code-installed users and developers who discover Coral via a blog post.
