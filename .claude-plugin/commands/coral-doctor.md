---
description: Deterministic Coral diagnostics + provider config. Runs `coral doctor` and offers fixes for warnings. Always available — does NOT depend on the LLM to decide whether to invoke. Use this when the auto-invoked `coral-doctor` skill is not triggering or you want to skip the conversational layer.
allowed-tools: Bash(coral:*), Bash(ls:*), Bash(test:*)
disable-model-invocation: true
---

# /coral:coral-doctor

The deterministic version of the `coral-doctor` skill. The user typed `/coral:coral-doctor` because they want diagnostics on demand — no NLP guessing, no "Claude decided not to invoke".

## What this does

1. Run `coral doctor` (top-level, NOT `coral project doctor`). This in-process runs the `coral self-check` probes and prints a human-readable report covering:
   - Coral binary status (in PATH, version, platform).
   - Wiki presence + page count.
   - CLAUDE.md presence + whether the Coral routing section is installed.
   - `claude` CLI status (PATH + version).
   - Configured providers (Anthropic / Gemini / Ollama / claude CLI).
   - Warnings (with `fix:` lines that are exact, copy-pasteable commands).
   - Suggestions (`coral bootstrap --estimate`, `/coral:coral-doctor`, etc.).
2. If `providers_configured` is empty, suggest the wizard:
   ```
   coral doctor --wizard
   ```
   The wizard is interactive — it requires a TTY (the slash command runs in one). Walks the user through 4 paths and writes `.coral/config.toml` only after a successful 1-token ping against the API endpoint.
3. For each `warning`, show the `fix:` line and ask if the user wants Claude to run it. Show the literal command — users copy/paste.
4. If everything is clean: tell the user to try `/coral:coral-bootstrap` next.

## Why deterministic

The auto-invoked `coral-doctor` skill is the primary entry point. This slash command is the **fallback** documented in `CLAUDE.md` (FR-ONB-25): *"if Claude doesn't suggest a Coral action, type `/coral:coral-doctor` — it always works"*. It's the contract that closes the "Claude ignored my CLAUDE.md routing" risk (PRD R16).

## Not for

- Code-content questions ("how does X work") — that's `coral query` (via the `coral-query` skill).
- Multi-repo manifest health — that's `coral project doctor`, a different command.
