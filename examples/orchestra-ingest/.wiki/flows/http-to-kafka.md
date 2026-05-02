---
slug: http-to-kafka
type: flow
last_updated_commit: 0000000000000000000000000000000000000000
confidence: 0.75
sources:
  - src/main.rs
backlinks: [ingest-pipeline, outbox]
status: reviewed
---

# HTTP → Kafka flow

End-to-end ingestion path:

1. Client POSTs to `/v1/events`.
2. [[ingest-pipeline]] handler validates schema, writes the event to
   the local DB AND to the [[outbox]] table in one transaction.
3. Handler returns `202 Accepted` with the event id.
4. The outbox dispatcher (background tokio task) polls the outbox
   table every 100ms, batches up to 50 events, and emits them to the
   Kafka topic `events.ingest`.
5. On Kafka success, the dispatcher marks the rows as
   `dispatched_at = now`.
6. On Kafka failure, the dispatcher leaves the rows for the next
   poll. Idempotency in the consumer side handles duplicates.

## Failure modes

- DB down → handler returns 503.
- Kafka down → outbox grows; dispatcher retries indefinitely.
- Both down → 503; events are dropped from the client's perspective
  (no retry on the server).

See [[runbook]] for the on-call response.
