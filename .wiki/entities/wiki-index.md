---
slug: wiki-index
type: entity
last_updated_commit: 213ac997cf61ad89610b3cfbe40af05e6b7fa8a8
confidence: 0.9
sources:
  - crates/coral-core/src/index.rs
backlinks:
  - core
  - page
  - ingest-cycle
status: verified
---

# `WikiIndex`

The catalog at `.wiki/index.md`. Anchor for incremental ingest.

```rust
pub struct WikiIndex {
    pub last_commit: String,
    pub generated_at: DateTime<Utc>,
    pub entries: Vec<IndexEntry>,
}
```

Each `IndexEntry`: `slug`, `page_type`, `path`, `confidence`, `status`, `last_updated_commit`.

## Format on disk

```markdown
---
last_commit: 213ac997…
generated_at: 2026-04-30T23:46:02Z
---

# Wiki index

| Type | Slug | Path | Confidence | Status | Last commit |
|------|------|------|------------|--------|-------------|
| module | cli | modules/cli.md | 0.9 | verified | 213ac99 |
| concept | karpathy-wiki | concepts/karpathy-wiki.md | 0.85 | reviewed | 213ac99 |
```

## Why a Markdown table

- Human-readable in any editor.
- Greppable (`grep "stale" .wiki/index.md`).
- Diff-friendly in PRs.
- Parseable by anyone with a regex (no special tooling).

The frontmatter holds the `last_commit` SHA — used by `coral ingest` to know where to start the incremental diff. See [[ingest-cycle]].

## Methods

- `parse(content) -> Result<Self>` — strict parser; tolerates blank tables.
- `to_string() -> Result<String>` — sorts entries deterministically by `(page_type, slug)` for stable diffs.
- `upsert(entry)` — idempotent; last write wins per slug.
- `find(slug)` — Option<&IndexEntry>.
- `bump_last_commit(sha)` — also refreshes `generated_at` to `Utc::now()`.
