---
slug: search
type: concept
last_updated_commit: 721050563f1ed29954b279fe334bf6bc8c8e2c34
confidence: 0.92
sources: []
backlinks: []
status: draft
generated_at: 2026-05-02T23:48:12.483062+00:00
---

# Search

`coral search <query>` ranks wiki pages against a free-text query.
Two engines are available, selected with `--engine`:

## TF-IDF / BM25 (default, offline)

Implemented in `coral-core/src/search.rs`. No API key required.

- `--algorithm tfidf` (default): classic TF-IDF with IDF smoothing and
  sqrt-length normalization.
- `--algorithm bm25`: BM25 with `k1=1.5`, `b=0.75`. Better precision on
  wikis > 100 pages; the saturation constant prevents a single high-freq
  term from dominating the score.

Tokenization: lowercase, alphanumeric, single-char tokens dropped, small
English+Spanish stopword list removed.

## Embeddings (semantic, online)

Selected with `--engine embeddings`. Requires an API key for the chosen
provider (`--embeddings-provider voyage|openai|anthropic`).

Vectors are stored in `<wiki_root>/.coral-embeddings.json`
(`EmbeddingsIndex`; schema-versioned, mtime-keyed per slug). The file is
auto-added to `.gitignore` by `coral init`. On `--reindex` the whole
index is rebuilt; otherwise only stale slugs are re-embedded.

Default provider: **Voyage AI** (`voyage-3`, dim=1024, via `curl`).

## Output

`--format markdown` (default) or `--format json`. `--limit` caps the
result count (default 5).

## Related

- [[embeddings]] — provider trait and vector cache details
- [[core]] — `search` and `embeddings` modules live here
- [[cli]] — `SearchArgs` wired in `commands/search.rs`
