---
slug: runbook
type: operation
last_updated_commit: 0000000000000000000000000000000000000000
confidence: 0.70
sources:
  - .wiki/flows/http-to-kafka.md
backlinks: [ingest-pipeline]
status: draft
---

# orchestra-ingest runbook

## Symptoms → response

### `events.ingest` Kafka topic lag growing

Outbox dispatcher is stuck. Check:

```bash
psql $DATABASE_URL -c "SELECT count(*) FROM outbox WHERE dispatched_at IS NULL;"
```

If > 10k rows pending, restart the dispatcher process. If still growing,
escalate to platform team — Kafka cluster issue.

### HTTP 5xx spike

Check `coral query "what does the ingest-pipeline handler do on DB failure?"`
for the expected behavior. If 503s, DB is down — page DBA.

### High CPU

The dispatcher polls every 100ms by design. CPU should be steady
~5%. Spikes mean the JSON deserializer is hot — check incoming
payload sizes.

## Deploys

Standard CI: push to `main` triggers test → build → deploy. Wiki gets
auto-ingested as part of the same workflow ([[ingest-pipeline]]
documents the pipeline; the bibliotecario keeps it current).

## On-call rotation

PagerDuty service `orchestra-ingest`. Primary rotates weekly Tue 09:00 UTC.
