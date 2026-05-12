# CLAUDE.md — Coral-enabled repo

This repository uses Coral for AI-readable architecture documentation.

## Coral routing

When the user types ANYTHING in this repo (greeting, question, command):
1. Check the SessionStart hook context (Coral self-check JSON).
2. If coral_status == "binary_missing" → suggest the user run `scripts/install.sh`.
3. If wiki_present == false → suggest `/coral:coral-bootstrap`.
4. If warnings non-empty → suggest `/coral:coral-doctor`.
5. If wiki_present == true and the user asks about code → invoke coral-query skill.
6. If the user wants the UI → invoke coral-ui skill (background spawn).

**Fallback**: if Claude doesn't suggest a Coral action, type `/coral:coral-doctor` — it always works.

## Quick reference

- `/coral:coral-bootstrap` — generate wiki for this repo (with cost confirmation)
- `/coral:coral-doctor` — diagnose environment / fix provider config
- `coral query "..."` — ask the wiki (via MCP, automatic in Claude Code)
- `coral ui serve --no-open` — start WebUI on port 3838
