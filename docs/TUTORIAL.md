# Tutorial: Coral in 5 minutes (no LLM auth required)

This walkthrough exercises every **deterministic** Coral subcommand
against a synthetic 4-page wiki. No `claude setup-token`, no
`VOYAGE_API_KEY`, no network — just the binary + 4 markdown files
you'll create by hand. The LLM-driven subcommands (`bootstrap`,
`ingest`, `query`, `consolidate`, `lint --auto-fix`, `diff --semantic`,
`onboard`) need their own auth and are covered in
[USAGE.md](USAGE.md).

> **Why this exists:** to let a new user feel Coral end-to-end before
> wiring it up to their real LLM provider. Every output block below
> is real, captured against the v0.8.0 binary on macOS.

## Prereqs

```bash
git clone https://github.com/agustincbajo/Coral && cd Coral
cargo build --release -p coral-cli
export CORAL=$PWD/target/release/coral
$CORAL --version          # → coral 0.8.0
```

## 1. Initialize the wiki

```bash
mkdir -p ~/coral-tutorial && cd ~/coral-tutorial
$CORAL init
```

```
✔ `.wiki/` initialized at .wiki
```

What got created:

```
.wiki/
├── .gitignore     # excludes .coral-cache.json + .coral-embeddings.json
├── SCHEMA.md      # the bibliotecario contract — extend per repo
├── index.md       # catalog with last_commit anchor
├── log.md         # append-only operation log
├── modules/
├── concepts/
├── entities/
├── flows/
├── decisions/
├── synthesis/
├── operations/
├── sources/
└── gaps/
```

## 2. Seed 4 pages

We'll create a tiny synthetic wiki for a fake order-processing
microservice. In a real workflow `coral bootstrap --apply` would do
this for you (LLM-driven from the actual code), but here we write
them by hand to keep auth out of the loop.

```bash
cat > .wiki/modules/order.md <<'EOF'
---
slug: order
type: module
last_updated_commit: 1234567890abcdef1234567890abcdef12345678
confidence: 0.85
sources:
  - src/order/handler.rs
backlinks: [outbox]
status: reviewed
---

# Order

Order creation flow. Receives POST /orders, validates, persists via the
[[outbox]] pattern, returns 202.

See [[checkout-flow]] for end-to-end behavior.
EOF
```

```bash
cat > .wiki/concepts/outbox.md <<'EOF'
---
slug: outbox
type: concept
last_updated_commit: 1234567890abcdef1234567890abcdef12345678
confidence: 0.9
sources:
  - src/outbox/dispatcher.rs
backlinks: [order]
status: verified
---

# Outbox pattern

Guarantees at-least-once delivery by writing intent to a local outbox
table inside the same database transaction as the business write. A
background dispatcher polls the outbox and emits to the message bus.

Used by [[order]] to publish OrderCreated events.
EOF
```

```bash
cat > .wiki/flows/checkout-flow.md <<'EOF'
---
slug: checkout-flow
type: flow
last_updated_commit: 1234567890abcdef1234567890abcdef12345678
confidence: 0.7
sources:
  - src/checkout/saga.rs
backlinks: []
status: reviewed
---

# Checkout flow

End-to-end: cart → payment → [[order]]. Spans 3 services. The order
service uses [[outbox]] for the OrderCreated event downstream.
EOF
```

```bash
cat > .wiki/concepts/idempotency.md <<'EOF'
---
slug: idempotency
type: concept
last_updated_commit: 1234567890abcdef1234567890abcdef12345678
confidence: 0.5
sources: []
backlinks: []
status: draft
---

# Idempotency

A re-submitted request must produce the same observable result as the
first. Implemented via request-id deduplication.
EOF
```

We seeded 2 things deliberately broken so lint surfaces them:
- `idempotency` is a draft with `confidence: 0.5` and no inbound
  backlinks → triggers `LowConfidence` + `OrphanPage`.
- All four pages cite source paths that don't exist on disk →
  triggers `SourceNotFound` for each.

## 3. Lint — structural checks

```bash
$CORAL lint --structural
```

```
# Lint report

- 🚨 Critical: 0
- ⚠️ Warning: 5
- ℹ️ Info: 0

## ⚠️ Warning

- **LowConfidence** in `.wiki/concepts/idempotency.md`: Confidence 0.5 below threshold 0.6
- **OrphanPage** in `.wiki/concepts/idempotency.md`: Page 'idempotency' has no inbound backlinks
- **SourceNotFound** in `.wiki/concepts/outbox.md`: Source path 'src/outbox/dispatcher.rs' for page 'outbox' not found on disk
- **SourceNotFound** in `.wiki/flows/checkout-flow.md`: Source path 'src/checkout/saga.rs' for page 'checkout-flow' not found on disk
- **SourceNotFound** in `.wiki/modules/order.md`: Source path 'src/order/handler.rs' for page 'order' not found on disk
```

Exit code is `0` because there are no `Critical` issues. Use
`--severity critical` to filter when wiring CI:

```bash
$CORAL lint --severity critical --format json
```

```json
{
  "issues": []
}
```

## 4. Stats — health snapshot

```bash
$CORAL stats
```

```
# Wiki stats

- Total pages: 4
- By type:
  - concept: 2
  - flow: 1
  - module: 1
- By status:
  - draft: 1
  - reviewed: 2
  - verified: 1
- Confidence: avg 0.74 (min 0.50, max 0.90)
- Low confidence (<0.6): 1
- Critical low confidence (<0.3): 0
- Stale pages: 0
- Archived pages: 0
- Total outbound links: 5
- Orphan candidates: 1 (idempotency)
```

JSON form is schema-validated against
[`docs/schemas/stats.schema.json`](schemas/stats.schema.json):

```bash
$CORAL stats --format json | jq '.total_pages, .by_type'
```

## 5. Search — TF-IDF and BM25

```bash
$CORAL search "outbox dispatcher"
```

```
# Search results for: outbox dispatcher

- **[[outbox]]** (score: 1.243)
  # Outbox pattern
  Guarantees at-least-once delivery by writing intent to a local outbox
  table inside the same database transaction as the business write. A
  background dispatcher polls the outbox and e

- **[[checkout-flow]]** (score: 0.288)
  ns 3 services. The order service uses [[outbox]] for the OrderCreated
  event downstream.

- **[[order]]** (score: 0.267)
  /orders, validates, persists via the [[outbox]] pattern, returns 202.
  See [[checkout-flow]] for end-to-end behavior.

_(Offline tfidf ranking. Pass `--algorithm bm25` (or `tfidf`) to switch
ranking, or `--engine embeddings` for semantic search via Voyage AI.)_
```

The same query with **BM25** (v0.7+):

```bash
$CORAL search "outbox dispatcher" --algorithm bm25
```

Same ranking on this tiny corpus, slightly different scores
(1.614 / 0.383 / 0.359). On 100+ page wikis BM25 generally has
better precision than TF-IDF cosine.

## 6. Diff — compare two pages structurally

```bash
$CORAL diff order outbox
```

```
# Diff: `order` ↔ `outbox`

## Frontmatter

| field | `order` | `outbox` |
|---|---|---|
| type | `module` | `concept` | ⚠️ differ
| status | `reviewed` | `verified` | ⚠️ differ
| confidence Δ | — | — | +0.05 |
| body chars | 162 | 276 |

## Sources

### Only in `order` (1)
- src/order/handler.rs

### Only in `outbox` (1)
- src/outbox/dispatcher.rs

## Wikilinks

### Only in `order` (2)
- checkout-flow
- outbox

### Only in `outbox` (1)
- order
```

Useful for spotting merge candidates, evaluating retirement, or
reviewing `wiki/auto-ingest` PRs side-by-side. With LLM auth, add
`--semantic` for contradiction detection (covered in
[USAGE.md](USAGE.md)).

## 7. Export — to a single-file HTML site

```bash
$CORAL export --format html --out ~/coral-tutorial.html
open ~/coral-tutorial.html      # macOS — opens in your browser
```

What you get: a self-contained HTML file with:
- Embedded CSS (light + dark via `prefers-color-scheme`).
- Sticky sidebar TOC grouped by page type.
- Each page rendered as a `<section id="slug">` so wikilinks become
  in-page anchor links.
- No build step, no JS — drop on GitHub Pages / S3 / any static host.

Other formats: `markdown-bundle` (single .md), `json` (raw page array),
`notion-json` (Notion API request bodies), `jsonl` (one page per line,
optional LLM-driven Q/A pairs via `--qa`).

## 8. Validate template-version pins

If you've ever run `coral sync --pin agents/x=v0.5.0`, you can verify
those versions exist as tags in the remote Coral repo:

```bash
$CORAL validate-pin
```

For a fresh install with no `.coral-pins.toml`:

```
no .coral-pins.toml or .coral-template-version present at .; nothing to validate
```

Once you have pins, the output is `✓` per resolvable version, `✗` per
missing tag, exit 1 if any are unresolvable.

## What's next

To unlock the LLM-driven half of Coral:

1. `claude setup-token` (one-time — see [README "Auth setup"](../README.md#auth-setup)).
2. `$CORAL bootstrap --apply` — let the bibliotecario propose pages from
   your actual code.
3. `$CORAL ingest --apply` — keep the wiki in sync as you push commits.
4. `$CORAL query "..."` — ask the wiki questions; cites slugs as
   `[[wikilinks]]`.

Or:
- `coral lint --auto-fix --apply` — LLM proposes structural fixes.
- `coral consolidate --apply --rewrite-links` — merge redundant pages
  AND patch every outbound wikilink in one pass.
- `coral diff <a> <b> --semantic` — surface contradictions between two
  pages.

The full reference is [USAGE.md](USAGE.md). The `template/` directory
ships 4 Claude Code subagents (`wiki-bibliotecario`, `wiki-linter`,
`wiki-consolidator`, `wiki-onboarder`) plus 9 prompt templates that
every LLM subcommand resolves via the `coral prompts list` chain.
