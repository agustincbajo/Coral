---
slug: ingest-pipeline
type: module
last_updated_commit: 0000000000000000000000000000000000000000
confidence: 0.80
sources:
  - src/main.rs
backlinks: [http-to-kafka, runbook]
status: reviewed
---

# Ingest pipeline

The orchestra-ingest pipeline accepts HTTP POSTs of event payloads,
deduplicates them via the [[outbox]] pattern, and forwards each event
to a Kafka topic.

See [[http-to-kafka]] for the end-to-end flow and [[runbook]] for the
on-call runbook.

## Module layout

```
src/
├── main.rs        — bootstrap + config
├── http/          — request handlers (placeholder)
├── outbox/        — dedupe + buffering
└── kafka/         — producer wrapping
```

This is a SKELETON — actual handler code is intentionally empty.
