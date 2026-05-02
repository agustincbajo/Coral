# orchestra-ingest wiki — SCHEMA

This SCHEMA extends [Coral's base SCHEMA](../../template/schema/SCHEMA.base.md) with project-specific rules.

## Project context

orchestra-ingest is a placeholder HTTP service that ingests events,
deduplicates them via the [[outbox]] pattern, and forwards to Kafka.
Pages should reflect this domain.

## Page-type conventions

In addition to Coral's base types:

- `modules/` — one per top-level module under `src/`.
- `flows/` — one per ingestion path (HTTP → outbox → Kafka).
- `operations/` — runbooks for on-call.

## Confidence thresholds (project override)

- `0.7+` requires at least one `sources:` entry.
- `0.5–0.7` is acceptable for synthesis pages.
- `<0.5` automatically tagged `status: draft` (lint --fix will enforce).

## Naming

- Page slugs are kebab-case: `ingest-pipeline`, not `ingest_pipeline`.
- Wikilinks always by slug only: `[[outbox]]`, never `[[concepts/outbox]]`.

See the base SCHEMA for everything else.
