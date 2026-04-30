# ADR 0002 — Claude CLI subprocess vs Anthropic API

**Date:** 2026-04-30  
**Status:** accepted

## Context

Coral needs to call an LLM. The two obvious paths:

- **A) Anthropic API directly** — HTTP POST with `ANTHROPIC_API_KEY`, parse JSON response, manage retries/timeouts.
- **B) `claude` CLI subprocess** — shell out to `claude --print`, capture stdout, no API key.

The user explicitly chose B for both local dev and CI (via `claude-code-action`).

## Decision

`coral-runner` shells out to `claude --print` via `std::process::Command`. The OAuth-based authentication is delegated to the `claude` CLI's own session management. CI uses `anthropics/claude-code-action@v1` with `CLAUDE_CODE_OAUTH_TOKEN` (org-level GitHub secret).

`Prompt::system` becomes `--append-system-prompt`, `Prompt::model` becomes `--model`, `Prompt::user` is the final positional arg. Optional wall-clock timeout via poll-based `try_wait` loop.

## Consequences

**Positive:**
- **Zero API key sprawl.** A single OAuth token at the org level covers N consumer repos.
- **Bills go to the user's Claude subscription**, not a separate API account.
- **The CLI handles auth, retries, model routing, and rate-limiting.** Coral focuses on prompt construction and output parsing.
- **No Anthropic SDK dependency.** Smaller binary, fewer transitive crates.
- **Subagent files (`.claude/agents/*.md`) are honored automatically** when the user invokes `/wiki-ingest` interactively in their Claude Code session.

**Negative:**
- **Requires `claude` in `PATH`.** `coral init`, `coral lint --structural`, `coral stats`, and `coral sync` work without it; `bootstrap`, `ingest`, `query`, `consolidate`, `onboard`, and `lint --semantic` need it.
- **Subprocess overhead (~50–200ms cold start)** is noticeable but tolerable for non-interactive batch jobs.
- **Streaming is not exposed** in v0.1 (stdout is captured fully before return). Future work: stream stdout to the user's terminal for `query`.
- **Error semantics are coarser** than direct API: we get exit code + stderr; not structured error types.

## Alternatives considered

- **Direct API with `reqwest` + `serde_json`**: rejected to avoid API-key infra and to align with the user's existing Claude Code subscription.
- **MCP server**: too complex for v0.1; would require running a separate process and managing IPC.
- **Multi-provider abstraction (Claude + Gemini + Groq)**: deferred. The `Runner` trait already allows swapping; a future `GeminiRunner` is one new file.
