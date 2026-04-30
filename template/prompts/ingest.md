# Ingest prompt template (v1)

You are the wiki bibliotecario for the repository at {{repo_path}}.

The current `.wiki/index.md` has `last_commit: {{last_commit}}`. The current `HEAD` is `{{head_sha}}`.

## Files changed in `{{last_commit}}..{{head_sha}}`

{{diff_summary}}

## Your task

For each changed file, decide:
1. Which `.wiki/` page is affected (or needs to be created).
2. The smallest semantic delta to apply.
3. The new `last_updated_commit`, `confidence`, `sources`, `backlinks`.

Output a YAML plan:

```yaml
plan:
  - page: modules/create-order.md
    action: update
    delta: status_machine_changed_state
    new_confidence: 0.85
    sources_to_add: [src/features/create_order/service.rs]
  - page: concepts/idempotency.md
    action: update
    delta: clarify_uniqueness_constraint
    new_confidence: 0.90
```

Do not write the actual page contents in this output — that's a separate step.
