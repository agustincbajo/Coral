---
description: Run wiki lint. Defaults to structural; pass --semantic for the LLM-driven contradiction check.
argument-hint: [--structural | --semantic | --all]
allowed-tools: Bash(coral lint:*)
---

Run `coral lint` with the user's flags ($ARGUMENTS). If `--semantic` or `--all`:
1. Use the @wiki-linter subagent on `.wiki/`
2. Aggregate output with structural lint
3. Print the consolidated report

Exit code 1 if any critical issue. Do not auto-fix.
