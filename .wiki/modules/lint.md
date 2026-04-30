---
slug: lint
type: module
last_updated_commit: 213ac997cf61ad89610b3cfbe40af05e6b7fa8a8
confidence: 0.9
sources:
  - crates/coral-lint/src/
backlinks:
  - cli
  - core
  - runner
status: verified
---

# `coral-lint` — structural + semantic checks

Lives at `crates/coral-lint`. 25 unit tests.

## Two layers

**Structural** (deterministic, no LLM, runs in `<50ms`):

- `check_broken_wikilinks` — every wikilink resolves to a known page slug.
- `check_orphan_pages` — no page (except system types) has zero inbound backlinks.
- `check_low_confidence` — pages below 0.6 (warning) / 0.3 (critical).
- `check_high_confidence_without_sources` — claimed confidence ≥ 0.6 without verifiable sources is a Warning.
- `check_stale_status` — surfaces explicit `status: stale` markings.

These run in parallel via `rayon::par_iter` over a fixed list of `fn` pointers. See [[lint-checks]].

**Semantic** (uses the LLM via [[runner]]):

- Builds a wiki snapshot context (slug + type + body excerpt per page).
- Sends it to the runner with a strict format request: one line per issue, `severity:slug:message`.
- Parses the response. `NONE` means no issues.
- Runner errors surface as a single Critical issue.

See [[ingest-cycle]] for how this fits into the maintenance loop.
