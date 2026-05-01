# ADR 0006 — Local semantic search storage

**Date:** 2026-04-30 (last updated 2026-05-01)
**Status:** accepted (v0.2 = TF-IDF stub; v0.3.1 = JSON-backed embeddings opt-in; sqlite-vec deferred to v0.4)

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

## v0.3.1 update — JSON storage, sqlite-vec deferred to v0.4

The v0.3.0 release deferred embeddings entirely (see CHANGELOG). v0.3.1 ships them, but with a simpler backend than this ADR originally proposed.

**What changed:** the storage layer landed as a single JSON file (`<wiki_root>/.coral-embeddings.json`) instead of `sqlite-vec`. Both the cache shape and the `Provider` field are schema-versioned. The CLI surface (`coral search --engine embeddings`) matches what this ADR contemplated.

**Why JSON, not sqlite-vec:**
- A wiki under ~5k pages fits in a JSON file with no measurable load-time cost (read, parse, in-memory cosine over `Vec<f32>`).
- sqlite-vec pulls in a native dep that complicates cross-platform builds (Coral ships as a single static binary today).
- `coral` already shells to `curl` for HTTP (see `notion-push`); doing the same for the Voyage call kept the binary lean and avoided pulling `reqwest` + `tokio` into a sync CLI.

**Provider:** Voyage AI `voyage-3` only. The provider field is reserved for a future trait, but the v0.3.1 path is hard-wired to Voyage. `--embeddings-model` lets users override the model name, which Voyage reads server-side.

**Default:** `tfidf`. Embeddings are strictly opt-in via `--engine embeddings`. No API key required for the default path — Coral keeps working offline.

**When sqlite-vec lands:** when a wiki crosses ~5k pages and the JSON load starts to dominate `coral search` latency, we'll migrate to sqlite-vec under the same `EmbeddingsIndex` API. The schema-versioning logic already invalidates stale on-disk format.
