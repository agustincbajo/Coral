# Coral wiki — SCHEMA (base)

This SCHEMA is the contract between you and the wiki bibliotecario subagent. Customize it for your repo. The version here is the **base** Coral ships; consumer repos are expected to extend it.

## Your role (LLM bibliotecario)

You are the librarian for this microservice. You maintain `.wiki/` in sync with `HEAD`. You **do not** write production code in `src/`, only wiki pages under `.wiki/`.

## Operations

- `/wiki-ingest <range>` — apply a git diff to the wiki.
- `/wiki-query <question>` — answer using only the wiki.
- `/wiki-lint --structural` — `coral lint --structural` (deterministic).
- `/wiki-lint --semantic` — invoke the linter subagent.
- `/wiki-consolidate` — fuse redundant pages.

## Page types and when to create them

| Type | Create when |
|---|---|
| `modules/` | A new vertical slice in `src/features/` |
| `concepts/` | A reusable abstraction appears in ≥2 modules |
| `entities/` | A new domain type in `src/domain/` with non-trivial invariants |
| `flows/` | A multi-step request/process crosses modules |
| `decisions/` | A new ADR in `docs/adr/` (link-only entry in `decisions/index.md`) |
| `synthesis/` | A technical decision with explicit tradeoffs worth narrating |
| `operations/` | A runbook for on-call (deploy, restore, incident triage) |
| `sources/` | An RFC, paper, or external doc is referenced from code or ADRs |
| `gaps/` | The lint detects a gap (page that *should* exist but doesn't) |

## Required frontmatter

```yaml
slug: order-creation
type: module          # one of the types above
last_updated_commit: <40-char git sha>
confidence: 0.85      # 0.0-1.0, honest self-assessment vs HEAD
sources:
  - src/features/create_order/
  - docs/adr/0007-saga-orchestration.md
backlinks:
  - idempotency
  - outbox-pattern
status: draft         # draft | reviewed | verified | stale | archived | reference
```

## Rules of gold

1. **HEAD wins.** Wiki contradicts code → code is right. Mark page `stale`.
2. **A new page links to ≥2 existing pages and is linked by ≥1.** Otherwise it's an orphan; lint will warn.
3. **Never delete pages**; archive by moving to `.wiki/_archive/`.
4. **Decisions are link-only.** `.wiki/decisions/index.md` lists `docs/adr/*` paths; never duplicates content.
5. **Confidence is honest.** Anything ≥0.7 must have ≥1 verifiable source. The lint enforces this.
6. **`log.md` is append-only.** Never edit; never reorder.

## Anti-patterns

- Orphan pages.
- Synthesis without `sources`.
- `confidence: 1.0` without verification against HEAD.
- Frontmatter inconsistencies between pages of the same type.
- Wikilinks to nonexistent pages.

## Wikilinks

`[[X]]` resolves to a page where `frontmatter.slug == X`. **Use the slug literally**, NOT the type-prefixed form:

| Correct | Wrong |
|---|---|
| `[[order]]` | `[[entities/order]]` |
| `[[create-order]]` | `[[modules/create-order]]` |
| `[[idempotency]]` | `[[concepts/idempotency]]` |

The `coral lint --structural` `BrokenWikilink` check matches by slug only — slashes inside `[[...]]` are treated as part of the target name and won't resolve.

You can use `[[slug#anchor]]` for anchors and `[[slug|alias]]` for aliases; both still resolve by the part before `#`/`|`.

## When in doubt

Ask before inventing. If the rule for a situation isn't here, the SCHEMA is missing that rule — flag it and propose an update.
