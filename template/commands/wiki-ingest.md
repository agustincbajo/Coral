---
description: Run an incremental wiki ingest. Reads the last_commit anchor in .wiki/index.md and updates pages affected by the diff to HEAD.
argument-hint: [optional: --from <sha>]
allowed-tools: Bash(coral ingest:*), Bash(git status:*)
---

Use the @wiki-bibliotecario subagent to run an incremental ingest from `.wiki/index.md`'s `last_commit` to `HEAD`.

If the user provided `--from <sha>`, use that as the start commit instead.

After the agent reports the proposed changes, summarize:
- How many pages would be created / updated / retired
- Whether `index.md` and `log.md` would change
- Any stale pages flagged

Do not commit. Leave the changes staged for the user to review.
