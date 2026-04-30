---
slug: karpathy-wiki
type: concept
last_updated_commit: 213ac997cf61ad89610b3cfbe40af05e6b7fa8a8
confidence: 0.85
sources:
  - https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f
backlinks:
  - cli
  - ingest-cycle
status: reviewed
---

# Karpathy LLM Wiki

The pattern Coral implements. Originally proposed by Andrej Karpathy on April 3, 2026 (X post + gist `karpathy/442a6bf555914893e9891c11519de94f`).

See [[karpathy-llm-wiki-gist]] for the canonical reference and derived works tracked.

## Core idea

Instead of RAG (retrieval-augmented generation, which embeds chunks and searches at query time), the LLM **compiles** raw sources into an interconnected Markdown wiki, **once per source**, and then queries that pre-synthesized wiki.

The wiki is the knowledge. The raw sources are bulk material.

## Why it beats RAG for codebases

| Aspect | RAG | LLM Wiki |
|---|---|---|
| Storage | Vector DB | Markdown in Git |
| State | Stateless per query | Acumulative |
| Lock-in | Vendor DB | Zero — text in Git |
| Auditability | Opaque embeddings | Each page cites `sources` |
| Scale | Millions of docs | ~500 pages before hybrid |
| Cost runtime | Embedding + retrieval per query | Just reading relevant pages |

## How Coral applies it

- `.wiki/` lives in the consumer Git repo.
- Pages have strict frontmatter (`slug`, `type`, `confidence`, `sources`, `backlinks`).
- A bibliotecario subagent (see `template/agents/wiki-bibliotecario.md`) maintains the wiki in sync with `HEAD`.
- Every push triggers an incremental ingest via [[ingest-cycle]].
- HEAD always wins: if the wiki contradicts the code, the page is marked `stale` and revised.
