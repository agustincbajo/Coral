---
description: Run `coral status` in the current repo and summarize the dashboard for the user.
allowed-tools: Bash(coral status:*)
disable-model-invocation: true
---

# /coral:coral-status

Run `coral status` and summarize the output.

```bash
coral status
```

Then give the user a 3-5 line summary:

- Wiki state: page count, last ingest timestamp, drift status.
- Lint status: pass / fail count from the structural and semantic checks.
- Multi-repo: if there's a `coral.toml`, the per-repo state from `coral project doctor`.
- Any immediate actions the user should take (e.g. *"`.wiki/` is 12 commits stale — run `coral ingest --apply` to refresh"*).

Keep it terse. The user can re-read the full output above your summary if they want details.

If `coral status` exits non-zero (no `.wiki/`, no `coral` binary on PATH, etc.), say so plainly and point at the `coral-bootstrap` skill or the install instructions at https://github.com/agustincbajo/Coral#install.
