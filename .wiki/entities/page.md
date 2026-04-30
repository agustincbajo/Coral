---
slug: page
type: entity
last_updated_commit: 213ac997cf61ad89610b3cfbe40af05e6b7fa8a8
confidence: 0.95
sources:
  - crates/coral-core/src/page.rs
  - crates/coral-core/src/frontmatter.rs
backlinks:
  - core
  - wiki-index
status: verified
---

# `Page`

The central entity in Coral. Lives at `coral_core::page::Page`.

```rust
pub struct Page {
    pub path: PathBuf,
    pub frontmatter: Frontmatter,
    pub body: String,
}
```

## Invariants

- `frontmatter` is fully parsed (no raw YAML strings).
- `body` is the text **after** the closing `---` line of the frontmatter, with one canonical leading blank line stripped.
- `path` is a real filesystem path or a tracking handle (used for error messages even when the Page is built from in-memory content).

## Lifecycle methods

- `Page::from_file(path)` — read + parse.
- `Page::from_content(text, path)` — parse from a string (path used only for errors).
- `Page::to_string()` — serialize back to a Markdown document.
- `Page::write()` — atomically write `to_string()` to `self.path`, creating parent dirs if needed.
- `Page::outbound_links()` — wikilinks discovered in body ∪ `frontmatter.backlinks`, sorted + deduped.
- `Page::bump_last_commit(sha)` — set `frontmatter.last_updated_commit`.
- `Page::add_backlink(slug)` — idempotent insert into `backlinks`.

## Frontmatter contract

Defined in `coral_core::frontmatter::Frontmatter`. Required fields: `slug`, `type`, `last_updated_commit`, `confidence`, `status`. Optional: `sources`, `backlinks`, `generated_at`. Unknown keys land in `extra: BTreeMap<String, Value>` for consumer-SCHEMA extensions.
