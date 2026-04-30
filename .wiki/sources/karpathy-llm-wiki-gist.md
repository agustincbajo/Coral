---
slug: karpathy-llm-wiki-gist
type: source
last_updated_commit: 213ac997cf61ad89610b3cfbe40af05e6b7fa8a8
confidence: 0.95
sources:
  - https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f
  - https://x.com/karpathy/status/<original-tweet-2026-04-03>
backlinks:
  - karpathy-wiki
status: reviewed
---

# Karpathy LLM Wiki gist

The canonical reference for the pattern Coral implements.

- **Author:** Andrej Karpathy.
- **Published:** 2026-04-03 (X post). Gist `karpathy/442a6bf555914893e9891c11519de94f` published the next day.
- **Title:** "LLM Wiki" (gist file: `llm-wiki.md`).
- **Stars at v0.1 cut:** ~5,000 in the first week.

## Key claims

1. RAG is wrong for personal/project knowledge bases — the LLM redescubre relations from scratch on every query, doesn't accumulate, and chunking destroys structure.
2. Alternative: the LLM compiles raw sources into an interconnected Markdown wiki, **once per source**, and queries the pre-synthesized wiki.
3. Karpathy reported ~100 articles / ~400,000 words on a research topic generated this way in a few weeks, with no manual writing.

## How Coral diverges

- Karpathy's gist describes the pattern manually with Obsidian as the editor. Coral makes it operational for a Git repo with automation: hooks, GH Actions, CI.
- Coral adds the explicit `confidence` + `sources` requirement enforced by lint. Karpathy's pattern relies on LLM judgment alone.
- Coral adds the `Runner` trait abstraction so all LLM-touching code is testable. Karpathy's bash scripts are not.

## Derived works tracked

- `cablate/llm-atomic-wiki` — atom layer + two-layer Lint + topic branches.
- `rohitg00/llm-wiki-v2` — extension with Git hooks + lifecycle + retention decay.
- `Yysun/git-wiki` — DEV.to writeup translating to a codebase. Shipped April 12, 2026.
- `Pratiyush/llm-wiki` — full implementation with 16 lint rules + 5-state lifecycle + Auto Dream consolidation.

## How to read the gist

The gist is short (~200 lines). The 4 canonical operations (ingest, query, lint, consolidate) and the 3-layer architecture (raw, wiki, schema) are the core. Read it cover-to-cover before contributing to Coral.
