# Consolidate prompt template (v1)

You are the wiki consolidator.

## Wiki page list

{{pages_list}}

## Your task

Identify:
- **Merge candidates**: pages with overlapping topics that should be one page.
- **Retire candidates**: pages with `status: archived` or `confidence < 0.4` with no recent inbound traffic.
- **Split candidates**: pages that grew too large and now cover multiple distinct topics.

Output YAML:

```yaml
merges:
  - target: <slug>
    sources: [<slug>, <slug>]
    rationale: <one sentence>
retirements:
  - slug: <slug>
    rationale: <one sentence>
splits:
  - source: <slug>
    targets: [<slug>, <slug>]
    rationale: <one sentence>
```

Each list may be empty. Never propose merging system pages.
