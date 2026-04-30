---
slug: lint-checks
type: concept
last_updated_commit: 213ac997cf61ad89610b3cfbe40af05e6b7fa8a8
confidence: 0.9
sources:
  - crates/coral-lint/src/structural.rs
  - crates/coral-lint/src/report.rs
backlinks:
  - lint
status: verified
---

# Lint checks

The 5 deterministic structural checks in [[lint]]. Each is a pure `fn(&[Page]) -> Vec<LintIssue>`.

## `check_broken_wikilinks` (Critical)

Builds a `HashSet` of known slugs. For every page, calls `outbound_links()` and emits a Critical issue per link whose target isn't in the set.

## `check_orphan_pages` (Warning)

Builds an inbound-count map. For every page (excluding system types `Index`, `Log`, `Schema`, `Readme`), if its slug never appears as a wikilink target or `backlinks` entry from any other page, it's an orphan.

System types are exempt because they're roots by design — `index.md` doesn't need to be referenced from anywhere.

## `check_low_confidence` (Critical or Warning)

- `confidence < 0.3` → Critical (the page is barely useful).
- `confidence < 0.6` → Warning.
- Pages with `status: reference` (examples, fixtures) are exempt.

## `check_high_confidence_without_sources` (Warning)

If `confidence ≥ 0.6` and `sources` is empty, the lint flags it. You can't claim high confidence without a verifiable source. Anti-pattern from [[karpathy-wiki]].

## `check_stale_status` (Info)

Surfaces pages explicitly marked `status: stale`. This is informational — it doesn't fail CI, but it's visible to anyone running `coral lint`.

## Aggregation

`run_structural(&[Page])` invokes all 5 in parallel via `rayon` and returns a single `LintReport` sorted deterministically (severity → page → message). The CLI's exit code is `1` if any Critical, `0` otherwise.
