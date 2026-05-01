# ADR 0006 — Local semantic search storage

**Date:** 2026-04-30  
**Status:** accepted (v0.2 = TF-IDF stub; v0.3 = embeddings)

## Context

`coral search <query>` requires relevance ranking over `.wiki/`. Two approaches:

- **TF-IDF / BM25 in pure Rust** — deterministic, no external deps, no API keys. Works well up to ~500 pages.
- **Embeddings + vector DB** — semantic similarity, robust to paraphrase. Needs an embedding API (Voyage AI, OpenAI, Anthropic when shipped) and a local vector store (sqlite-vec or qmd).

## Decision

**v0.2 ships TF-IDF**. Pure Rust, zero dependencies, ranks pages well enough for the typical "find the page about outbox" query.

**v0.3 will switch to embeddings** stored in sqlite-vec, with TF-IDF as a fallback when offline / when embeddings haven't been generated yet.

## Why TF-IDF for v0.2

- No external API. Coral remains usable without `claude` and without an embedding API key.
- Deterministic. Lint-style tests verify ranking stability.
- Tiny binary footprint (one HashMap-based scorer, ~150 LOC).
- Ships in the v0.2 timeframe; embeddings would slip the schedule.

## Why embeddings for v0.3

- Semantic match: "how is an order made" finds the `create_order` page even if the page never uses the verb "made".
- Better cross-lingual recall.
- Industry standard for >100-page knowledge bases.

Storage candidate: **sqlite-vec** (sqlite extension; same DB file as the wiki cache).
Embedding candidate: **Voyage AI `voyage-3`** (cheap, high quality) — or Anthropic when they ship one. Configurable via `CORAL_EMBED_PROVIDER`.

## Consequences

- v0.2 users get a working `coral search` today.
- v0.3 will be a drop-in replacement: same CLI surface, better recall.
- Downstream (composite actions, scripts) won't have to change at the upgrade.

## Alternatives considered

- **qmd** instead of sqlite-vec: smaller community; sqlite-vec is more battle-tested.
- **BM25 instead of TF-IDF**: minor improvement in quality, not enough to justify the algorithmic complexity for a stub.
- **External vector DB (Pinecone, Qdrant)**: cloud dependency we're avoiding.
