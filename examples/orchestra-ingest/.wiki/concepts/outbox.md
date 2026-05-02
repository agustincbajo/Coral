---
slug: outbox
type: concept
last_updated_commit: 0000000000000000000000000000000000000000
confidence: 0.85
sources:
  - src/main.rs
backlinks: [ingest-pipeline, http-to-kafka]
status: reviewed
---

# Outbox pattern

Used by [[ingest-pipeline]] to guarantee at-least-once delivery to
Kafka. Each incoming event is written to a local outbox table inside
the same database transaction as the business write. A background
dispatcher polls the outbox and emits to Kafka.

The pattern decouples the HTTP handler's success path from Kafka
availability — the handler only needs the local DB to be up.

See [[http-to-kafka]] for how the dispatcher is wired.
