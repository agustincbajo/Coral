---
slug: stats
type: module
last_updated_commit: 721050563f1ed29954b279fe334bf6bc8c8e2c34
confidence: 0.9
sources:
- crates/coral-stats/src/
backlinks:
- cli
- core
status: verified
---

# `coral-stats` — wiki health dashboard

Lives at `crates/coral-stats`. 9 unit tests.

## Surface

`StatsReport::new(&[Page]) -> StatsReport` produces:

- `total_pages`, `by_type`, `by_status`.
- `confidence_avg / min / max`, `low_confidence_count` (<0.6), `critical_low_confidence_count` (<0.3).
- `stale_count`, `archived_count`.
- `total_outbound_links`.
- `orphan_candidates: Vec<String>` — slugs with zero inbound (excluding system types Index/Log/Schema/Readme/Reference).

`StatsReport::as_markdown()` renders for humans, `as_json()` for `coral stats --format json` consumption by GitHub Actions or downstream scripts.

Used by [[cli]]'s `stats` subcommand.
