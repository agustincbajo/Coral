# Ingest prompt template (v1)

You are the wiki bibliotecario for the repository at {{repo_path}}.

The current `.wiki/index.md` has `last_commit: {{last_commit}}`. The current `HEAD` is `{{head_sha}}`.

## Files changed in `{{last_commit}}..{{head_sha}}`

{{diff_summary}}

## Your task

For each meaningful change, decide:
1. Which `.wiki/` page is affected (or needs to be created or retired).
2. The smallest semantic delta to apply.

Output a YAML plan that the CLI will parse and apply directly to `.wiki/`:

```yaml
plan:
  - slug: order
    action: create | update | retire
    type: module          # required when action=create
    confidence: 0.7        # 0.0..1.0; required when action=create
    rationale: handler signature changed
    body: |
      # Order

      Lorem ipsum...
```

Rules:

- `slug` is mandatory on every entry. It must match an existing page (for `update` / `retire`) or be a new slug (for `create`).
- `action` is one of `create`, `update`, `retire`.
- `type` is required when `action=create`. Allowed values: `module`, `concept`, `entity`, `flow`, `decision`, `synthesis`, `operation`, `source`, `gap`, `index`, `log`, `schema`, `readme`, `reference`.
- `confidence` is a float in `0.0..1.0`. Required for `create` (default 0.5 if missing).
- `body` is the Markdown body of the new page. Required for `create`. **Do NOT include frontmatter** — the CLI builds it from the other fields.
- For `update`, the CLI bumps `last_updated_commit` on the existing page; you only need `slug` + `action: update` + `rationale`.
- For `retire`, the CLI marks the existing page `status: stale`; you only need `slug` + `action: retire` + `rationale`.
- Do not invent slugs that don't fit a real change in the diff.
