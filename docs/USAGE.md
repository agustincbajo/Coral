# Usage

Reference for every Coral subcommand. Run `coral <cmd> --help` for the canonical clap-generated help.

## Global flags

| Flag | Default | Description |
|---|---|---|
| `--wiki-root <PATH>` | `.wiki/` | Override the wiki root directory. |
| `--quiet` | off | Suppress non-error output. |
| `--verbose` | off | Set `RUST_LOG=coral=debug,info` if not already set. |

---

## `coral init`

Initialize a `.wiki/` directory in the current Git repo.

```bash
coral init [--force]
```

- Creates `SCHEMA.md` (from the embedded base SCHEMA), `index.md` (with current HEAD as `last_commit`), `log.md` (with the bootstrap entry), and 9 type subdirectories (`modules/`, `concepts/`, `entities/`, `flows/`, `decisions/`, `synthesis/`, `operations/`, `sources/`, `gaps/`).
- **Idempotent**: re-running skips existing files unless `--force` is passed.
- `--force` re-creates all top-level files (DESTRUCTIVE for `index.md` / `log.md`).

---

## `coral bootstrap`

Compile the initial wiki from `HEAD`.

```bash
coral bootstrap [--apply] [--model <id>] [--provider claude|gemini|local]
```

- Walks the repo (skips `.git/`, `target/`, `.wiki/`, `node_modules/`, `.idea/`, `.vscode/`).
- Sends a truncated file listing (max 200 paths) to the LLM via `claude --print`.
- **Default is dry-run**: prints the YAML plan of 5–15 page slugs (type + rationale + body).
- Pass `--apply` to write the pages, upsert `.wiki/index.md`, append `.wiki/log.md`.
- `--provider` (or `CORAL_PROVIDER` env): `claude` (default) | `gemini` | `local` (llama.cpp).

---

## `coral ingest`

Incremental ingest from `last_commit` to `HEAD`.

```bash
coral ingest [--from <sha>] [--apply] [--model <id>] [--provider claude|gemini|local]
```

- Reads `last_commit` from `.wiki/index.md` unless `--from` is given.
- Runs `git diff --name-status <from>..HEAD`, sends the summary to the LLM.
- **Default is dry-run**: prints the YAML plan of `{slug, action, type?, body?, rationale}` where `action ∈ {create, update, retire}`.
- `--apply` mutates `.wiki/`: writes new pages, bumps `last_updated_commit` on updates, marks retired pages `status: stale`, updates `index.md`, appends `log.md`.
- `--provider` (or `CORAL_PROVIDER` env): `claude` (default) | `gemini` | `local`.

---

## `coral query`

Ask the wiki a question.

```bash
coral query "How is an order created?" [--model <id>] [--provider claude|gemini|local]
```

- Walks `.wiki/`, builds a snapshot context (truncated to 40 pages, 200 chars each).
- Sends question + context + citation instructions to the LLM.
- Streams the answer to stdout (cites slugs as `[[wikilink]]`).
- Telemetry: `RUST_LOG=coral=info coral query "..."` emits `coral query: starting` / `coral query: completed` events with `pages_in_context`, `model`, `duration_ms`, `chunks`, `output_chars`.
- `--provider` (or `CORAL_PROVIDER` env): `claude` (default) | `gemini` | `local`.

---

## `coral lint`

Run structural and/or semantic lint.

```bash
coral lint [--structural] [--semantic] [--all] [--staged] [--auto-fix [--apply]] \
           [--format markdown|json] [--provider claude|gemini|local]
```

- `--structural` (default if no flag): broken wikilinks, orphans, low confidence, high confidence without sources, stale status. Pure deterministic checks.
- `--semantic`: requires the LLM. Surfaces contradictions, obsolete claims, confidence/sources mismatches.
- `--all`: both.
- `--staged`: pre-commit-hook mode. Loads every page (so the graph stays intact for orphan / wikilink checks) but filters the report to issues whose `page` is in `git diff --cached --name-only`. Workspace-level issues (no `page`) are kept. Use to gate commits that touch the wiki.
- `--auto-fix`: after lint runs, the LLM proposes fixes (downgrade confidence, mark stale, append italic note). Default is **dry-run** (prints YAML plan); `--apply` writes changes back. Capped scope — cannot rewrite whole bodies or invent sources. Override the system prompt at `<cwd>/prompts/lint-auto-fix.md`.
- `--provider` (or `CORAL_PROVIDER` env): used by `--semantic` and `--auto-fix`. `claude` (default) | `gemini` | `local`.
- Exit code `1` if any **critical** issue; `0` otherwise. With `--staged`, only critical issues touching staged files trigger `1`.

### Pre-commit hook example

```bash
# .git/hooks/pre-commit
#!/usr/bin/env bash
exec coral lint --structural --staged
```

---

## `coral consolidate`

Suggest page consolidations.

```bash
coral consolidate [--apply] [--model <id>] [--provider claude|gemini|local]
```

- Lists all page slugs (truncated to 80) and asks the LLM for `merges:`, `retirements:`, `splits:` candidates (YAML).
- **Default is dry-run**: prints the proposal.
- `--apply`: marks every `retirements[].slug` as `status: stale`. `merges[]` and `splits[]` are surfaced as warnings — they need human review (body merging / partitioning isn't safely automated).
- `--provider` (or `CORAL_PROVIDER` env): `claude` (default) | `gemini` | `local`.

---

## `coral stats`

Wiki health dashboard.

```bash
coral stats [--format markdown|json]
```

- Counts pages by type and status, computes confidence stats, lists orphan candidates (excluding system page types).
- `stats --format json` produces output that validates against [`docs/schemas/stats.schema.json`](./schemas/stats.schema.json). Use `jq` or downstream tooling to parse.

---

## `coral sync`

Lay the embedded template into the current directory.

```bash
coral sync [--version <V>] [--force]
```

- Extracts `template/` into `<cwd>/template/`.
- Writes `.coral-template-version` at `<cwd>` root.
- `--version` must match the running binary's version (in v0.1, no remote download).
- `--force` overwrites existing template files.

---

## `coral onboard`

Generate a tailored reading path.

```bash
coral onboard [--profile "backend dev" | "data engineer" | "PM" | "on-call"] [--apply] \
              [--model <id>] [--provider claude|gemini|local]
```

- Sends the wiki page list + profile to the LLM.
- **Default**: prints a Markdown ordered list of 5–10 pages with rationales.
- `--apply`: persists the path as a wiki page at `<wiki>/operations/onboarding-<slug>.md` (slug = profile lowercased + dashed). Re-running with the same profile overwrites; the slug is the dedup key.
- `--provider` (or `CORAL_PROVIDER` env): `claude` (default) | `gemini` | `local`.

---

## `coral prompts list`

Inspect prompt sources (local override, embedded, or fallback).

```bash
coral prompts list
```

- Prints a Markdown table mapping each known prompt name to its resolved source.
- Resolution priority (highest first):
  1. **Local** — `<cwd>/prompts/<name>.md` (if present and readable).
  2. **Embedded** — the file at `template/prompts/<name>.md` baked into the binary via `include_dir`.
  3. **Fallback** — a hardcoded string in the corresponding command source.
- Drop a `prompts/<name>.md` file in your repo to override how Coral talks to the LLM for that subcommand. Known names: `bootstrap`, `ingest`, `query`, `lint-semantic`, `consolidate`, `onboard`.

---

## `coral search`

Search the wiki.

```bash
coral search "<query>" [--engine tfidf|embeddings] \
                       [--embeddings-provider voyage|openai] \
                       [--embeddings-model <id>] \
                       [--reindex] [--limit N] [--format markdown|json]
```

### Engines

- **`tfidf`** (default): pure-Rust TF-IDF over slug + body. No API key, works offline. Suitable for ~500 pages.
- **`embeddings`**: semantic similarity. Pluggable provider (v0.4+):
  - `--embeddings-provider voyage` (default) — Voyage AI `voyage-3` (1024-dim). Requires `VOYAGE_API_KEY`.
  - `--embeddings-provider openai` — OpenAI `text-embedding-3-small` (1536-dim) by default; pass `--embeddings-model text-embedding-3-large` for the 3072-dim variant. Requires `OPENAI_API_KEY`.

  Embeddings are cached at `<wiki_root>/.coral-embeddings.json`, mtime-keyed per slug, dimension-aware. Switching provider/model invalidates the cache automatically.

### Examples

```bash
# Offline TF-IDF (default).
coral search "outbox dispatcher"

# Semantic via Voyage.
export VOYAGE_API_KEY=…
coral search "how does retry work" --engine embeddings

# Semantic via OpenAI.
export OPENAI_API_KEY=…
coral search "how does retry work" --engine embeddings --embeddings-provider openai

# OpenAI's larger model.
coral search "..." --engine embeddings --embeddings-provider openai \
  --embeddings-model text-embedding-3-large

# Force re-embedding (e.g. after model upgrade).
coral search "x" --engine embeddings --reindex

# JSON output for scripting.
coral search "x" --engine embeddings --format json
```

### Notes

- TF-IDF tokenizes slug + body, scores via TF-IDF, returns the top-N pages. Stopwords (English + Spanish) filtered out. Single-character tokens dropped. Snippets are 200-char windows around the first matching token. Snippets clamp to UTF-8 char boundaries (v0.3.2 fix — the previous byte-indexed slicer panicked on em-dashes).
- Embeddings cache is schema-versioned; mtime-keyed per slug. Pages whose mtime didn't change skip re-embedding. Use `--reindex` to force a full rebuild.
- Implementations live in `coral-runner::embeddings::EmbeddingsProvider` — adding a new provider (Anthropic when shipped, etc.) is one new struct.

### Cost

Embedding a 200-page wiki with `voyage-3` ≈ $0.001 (200 pages × ~500 tokens × $0.10/1M). OpenAI `text-embedding-3-small` ≈ $0.001 too. Re-runs only embed changed pages. CI workflows can add the `embeddings-cache` composite action to persist `.coral-embeddings.json` across runs.

See [ADR 0006](./adr/0006-local-semantic-search-storage.md) for the rationale on JSON storage vs sqlite-vec (deferred to a future release).

---

## `coral export`

Export the wiki to various target formats.

```bash
coral export [--format markdown-bundle|json|notion-json|html|jsonl] [--out FILE] \
             [--type TYPE]... [--qa] [--model M] [--provider claude|gemini|local]
```

- `markdown-bundle` (default): single Markdown file with all pages concatenated. Useful for printing or feeding to another LLM as context.
- `json`: raw JSON array, one object per page (`slug`, `type`, `confidence`, `sources`, `backlinks`, `body`, ...).
- `notion-json`: array of Notion API `POST /v1/pages` request bodies, ready to be `curl`-posted to a Notion database. Set `parent.database_id` from your config.
- `html`: single self-contained HTML file. Embedded CSS supports light + dark via `prefers-color-scheme`, sticky sidebar TOC grouped by page type, every page as `<section id="slug">`. `[[wikilinks]]` translate to in-page anchor links (plain / aliased / anchored forms supported). Drop the file on GitHub Pages / S3 / any static host — no build step. New in v0.5.
- `jsonl`: one JSON object per line — `{slug, body, prompt}` (stub) or `{slug, prompt, completion}` (with `--qa`) — a starting point for fine-tuning datasets.

### Notion sync example

```bash
coral export --format notion-json --out notion-bodies.json
# Then for each entry:
jq -c '.[]' notion-bodies.json | while read body; do
  curl -X POST https://api.notion.com/v1/pages \
    -H "Authorization: Bearer $NOTION_TOKEN" \
    -H "Notion-Version: 2022-06-28" \
    -H "Content-Type: application/json" \
    -d "$body"
done
```

### Fine-tuning dataset

```bash
# Stub prompts (one per page, no LLM call):
coral export --format jsonl --out wiki-dataset.jsonl

# LLM-driven Q/A pairs (3-5 per page):
coral export --format jsonl --qa --out wiki-qa.jsonl

# Cheap batch via Gemini:
coral export --format jsonl --qa --provider gemini --model gemini-2.5-flash --out wiki-qa.jsonl
```

With `--qa`, each page is sent to the runner with the `qa-pairs` system prompt (override at `<cwd>/prompts/qa-pairs.md`). The model emits one JSON line per pair (`{"prompt":"...","completion":"..."}`); Coral tags each with the page slug and concatenates the result. Malformed lines are skipped with a warning.

---

## `coral notion-push`

Push wiki pages directly to a Notion database. Thin wrapper over `coral export --format notion-json` + curl.

```bash
coral notion-push [--token <TOKEN>] [--database <DB_ID>] [--type <TYPE>] [--apply]
```

Env vars (alternative to flags):
- `NOTION_TOKEN` — Notion integration token (required).
- `CORAL_NOTION_DB` — target database id (required).

Filter by page type with `--type concept --type module` (repeatable).

**Default is dry-run** (matches `bootstrap`/`ingest` semantics): without `--apply`, the command prints what would be POSTed and exits 0 without calling Notion. Pass `--apply` to actually push.

Exit code: `0` if all pages POST cleanly (HTTP 2xx) or dry-run, `1` otherwise.

### Setup

1. Create an internal integration at https://www.notion.so/my-integrations — copy the secret as `NOTION_TOKEN`.
2. Create a database with these properties: `Name` (title), `Type` (select), `Status` (select), `Confidence` (number).
3. Share the database with your integration. Copy the database id (32-char hex from the URL).
4. `export NOTION_TOKEN=secret_…` and `export CORAL_NOTION_DB=…`. Run `coral notion-push` (no `--apply`) to preview, then add `--apply` to push.

---

## `coral diff`

Compare two wiki pages structurally.

```bash
coral diff <slugA> <slugB> [--format markdown|json]
```

- **Frontmatter delta**: type / status / confidence Δ.
- **Sources arithmetic**: common / only-A / only-B as `BTreeSet`s.
- **Wikilinks arithmetic**: same as sources but for `[[wikilinks]]` extracted from each body.
- **Body length stats**: char counts.

Use cases: spot merge candidates, evaluate retirement, review `wiki/auto-ingest` PRs side-by-side. v0.5 ships the structural diff only; future `--semantic` flag will add LLM-driven contradiction detection.

```bash
# Markdown table side-by-side.
coral diff order checkout

# JSON for tooling.
coral diff order checkout --format json | jq '.wikilinks.common'
```

---

## `coral validate-pin`

Verify every version in `.coral-pins.toml` exists as a tag in the remote Coral repo.

```bash
coral validate-pin [--remote <URL>]
```

- Reads `.coral-pins.toml` (or the legacy `.coral-template-version`).
- One `git ls-remote --tags <url>` call (no clone).
- Reports `✓` per pin / `✗` for any missing tag.
- Exit `0` when clean, `1` if any pin is unresolvable.
- `--remote` overrides the default Coral repo URL (useful for forks / mirrors).

Run as a CI guard before a release that consumes those pins to catch typos cheaply.

---

## CI: embeddings cache

If your CI workflow runs `coral search --engine embeddings`, persist `.coral-embeddings.json` across runs so each build only re-embeds pages whose content changed:

```yaml
- uses: actions/checkout@v4
- uses: agustincbajo/Coral/.github/actions/embeddings-cache@v0.5.0
  with:
    wiki_root: .wiki    # default
- run: coral search --engine embeddings "outbox dispatcher"
  env:
    VOYAGE_API_KEY: ${{ secrets.VOYAGE_API_KEY }}
```

Cache key is `<prefix>-<ref>-<hash of .wiki/**/*.md>` with branch-scoped fallback so a single page edit reuses ~all vectors.

---

## CI: Hermes quality gate

Coral ships an **opt-in** composite action that runs an independent LLM validator (Hermes) against wiki/auto-ingest PRs before merge. The validator (a separate subagent from the bibliotecario, on a different model) reads each changed page in the PR, verifies its `sources:` list resolves and the body doesn't contradict the cited files, and posts a PR review (APPROVE or REQUEST CHANGES).

To enable it, add a job to your workflow that runs on PRs labeled `wiki-auto`:

```yaml
hermes-validate:
  if: contains(github.event.pull_request.labels.*.name, 'wiki-auto')
  runs-on: ubuntu-latest
  permissions:
    contents: read
    pull-requests: write
  steps:
    - uses: actions/checkout@v4
      with:
        fetch-depth: 0
    - uses: agustincbajo/Coral/.github/actions/validate@main
      with:
        claude_code_oauth_token: ${{ secrets.CLAUDE_CODE_OAUTH_TOKEN }}
        pr_number: ${{ github.event.pull_request.number }}
```

The action skips validation when fewer than `min_pages_to_validate` (default 5) pages changed — keeps token spend predictable on small PRs. The subagent definition lives at `template/agents/wiki-validator.md` (sync via `coral sync`).
