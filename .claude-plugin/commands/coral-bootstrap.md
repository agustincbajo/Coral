---
description: Manually bootstrap a Coral wiki in the current repo. Runs `coral init` then asks for explicit confirmation before running `coral bootstrap --apply` (which costs LLM credits).
allowed-tools: Bash(coral:*), Bash(ls:*), Bash(test:*)
disable-model-invocation: true
---

# /coral:coral-bootstrap

Explicit, user-driven version of the `coral-bootstrap` skill. Use this when you want manual control instead of letting Claude auto-invoke.

## What this does

1. Run `coral --version` to confirm the binary is on PATH. If not, stop with a clear install pointer.
2. Run `test -d .git` to confirm we're in a git repo. If not, stop and tell the user to `git init` first.
3. Check for an existing `.wiki/` — if it exists, ask the user whether to abort or continue (do not auto-overwrite).
4. Run `coral init` (free, no LLM).
5. **Pause.** Tell the user the next step (`coral bootstrap --apply`) will call their configured LLM provider and cost real money — a few cents for a small repo, several dollars for a large monorepo. Ask whether to:
   - run `coral bootstrap --apply` now,
   - run `coral bootstrap --dry-run` first to preview,
   - or abort.
6. Run whichever the user chose.
7. On success, suggest one concrete starter `coral query "..."` based on the repo's README or top-level layout.

Do NOT skip step 5. The whole point of an explicit slash command is to gate the paid step on user consent.
