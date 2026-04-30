# Coral Wiki — SCHEMA (base)

This is the base SCHEMA shipped with Coral v0.1. Customize it for your repo.

## Page types

- `module/`   — vertical slice (one feature per page)
- `concept/`  — reusable abstraction
- `entity/`   — domain type with invariants
- `flow/`     — request flow / sequence
- `decision/` — link to docs/adr/
- `synthesis/`— comparative pages, "why we chose X"
- `operation/`— runbooks
- `source/`   — external doc references
- `gap/`      — what's missing (curated by lint)

## Required frontmatter

```yaml
slug: <kebab-case>
type: module | concept | entity | flow | decision | synthesis | operation | source | gap
last_updated_commit: <git sha>
confidence: 0.0..1.0
sources: [paths]
backlinks: [[other-page]]
status: draft | reviewed | verified | stale | archived
```

## Rules of thumb

- HEAD wins: if the wiki contradicts the code, the code is right → mark stale.
- A new page must link to ≥2 existing pages and be linked by ≥1.
- Never delete pages; archive by moving to `.wiki/_archive/`.
- Decisions never duplicate ADR content; link only.
