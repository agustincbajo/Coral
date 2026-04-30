---
slug: wikilink-extraction
type: concept
last_updated_commit: 213ac997cf61ad89610b3cfbe40af05e6b7fa8a8
confidence: 0.9
sources:
  - crates/coral-core/src/wikilinks.rs
backlinks:
  - core
  - lint
status: verified
---

# Wikilink extraction

`coral_core::wikilinks::extract(content) -> Vec<String>` returns the targets of double-bracket wikilink references in document order, deduplicated. The reference syntax is `[[page]]`, optionally with `#anchor` or `|alias` suffixes (see [[page]]).

## Edge cases handled

- **Anchor stripping**: `[[page#fields]]` → `"page"` (see [[page]]).
- **Alias stripping**: `[[page|the Page entity]]` → `"page"`.
- **Whitespace trimming**: `[[  page  ]]` → `"page"`.
- **Code fences**: wikilinks inside ` ``` ` fenced blocks are ignored.
- **Escapes**: `\[[escaped]]` is ignored (preceding backslash).
- **Empty targets**: `[[]]` and `[[ ]]` are filtered out.

## Why this matters

The structural lint check `check_broken_wikilinks` in [[lint]] depends on this being correct. A buggy extractor either flags valid links as broken or misses real broken ones — both are loud failures during ingest.

Tested with 11 unit tests covering every edge case above.

## Implementation note

The regex is `\[\[([^\]\n]+)\]\]`, compiled once via `OnceLock<Regex>` (lazy static via `std`, no `lazy_static!` crate).

The line-by-line walk for the code-fence guard is cheap; we don't try to parse Markdown — we just track the boolean state across triple-backtick lines.
