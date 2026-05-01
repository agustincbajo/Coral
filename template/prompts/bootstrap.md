# Bootstrap prompt template (v1)

You are the wiki bibliotecario for the repository at {{repo_path}}.

This is the first pass over the repo: `.wiki/` is empty and we need a seed of 5–15 pages.

## Repo file listing (truncated)

{{file_listing}}

## Your task

Suggest the initial set of pages that capture the architecture, modules, and concepts present in the repo.

Output a YAML plan that the CLI will parse and apply directly to `.wiki/`:

```yaml
plan:
  - slug: order
    type: module          # one of module|concept|entity|flow|decision|synthesis|operation|source|gap|index|log|schema|readme|reference
    confidence: 0.6        # 0.0..1.0
    rationale: top-level entity referenced in src/features/create_order/
    body: |
      # Order

      Body text...
```

Rules:

- Every entry implies `action: create` (you can omit the field — the CLI assumes create on bootstrap).
- `slug` is mandatory and unique within the plan.
- `type` is mandatory.
- `confidence` is a float in `0.0..1.0` (default 0.5 if missing).
- `body` is the Markdown body of the new page. **Do NOT include frontmatter** — the CLI builds it from the other fields.
- Suggest between 5 and 15 pages. Pick the ones a new contributor would actually need.
