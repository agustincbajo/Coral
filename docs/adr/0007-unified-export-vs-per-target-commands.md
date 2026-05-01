# ADR 0007 — Unified `coral export` vs per-target commands

**Date:** 2026-04-30  
**Status:** accepted (v0.2)

## Context

Issues #9 (Notion sync) and #13 (fine-tune dataset) both want export
mechanisms. Two designs:

- **Per-target commands**: `coral notion-sync`, `coral export-jsonl`,
  etc. Each tightly integrated with its target API.
- **Unified `coral export --format X`**: one command, multiple format
  outputs, no embedded API knowledge.

## Decision

**Unified `coral export --format X`** for v0.2.

The wiki itself is target-agnostic. Pushing to Notion or fine-tuning
a model both reduce to "give me the data in shape Y and let me POST it".
A unified exporter avoids growing N tightly-coupled subcommands.

## Consequences

**Positive:**
- One command surface to learn. Easier docs.
- Consumers handle the network step (curl, gh CLI, custom scripts).
  Coral stays out of the API-credential-management business.
- New formats (e.g., a vector-store-compatible export) are one new
  match arm in `render_<fmt>`.

**Negative:**
- Less polished UX than a "click here to push to Notion" command.
  v0.3 may add `coral notion-push --token $NOTION_TOKEN` as a thin
  wrapper that calls `coral export --format notion-json` + curls.
- LLM-generated Q/A pairs for the `jsonl` format aren't shipped in
  v0.2 — that needs the runner. Stub prompt is included; v0.3 fills in.

## Alternatives considered

- **Per-target commands**: would need to embed Notion + fine-tune
  knowledge into the binary. Rejected for v0.2.
- **A separate `coral-export` crate**: overkill for ~200 LOC. Module
  inside `coral-cli` is enough.
