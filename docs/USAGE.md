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
coral bootstrap [--model <id>] [--provider claude|gemini]
```

- Walks the repo (skips `.git/`, `target/`, `.wiki/`, `node_modules/`, `.idea/`, `.vscode/`).
- Sends a truncated file listing (max 200 paths) to the LLM via `claude --print`.
- Prints a YAML suggestion of 5–15 page slugs (type + rationale).
- **In v0.1, does not write pages.** Apply suggestions manually.
- `--provider` (or `CORAL_PROVIDER` env): `claude` (default) or `gemini` (shells to a `gemini` CLI binary).

---

## `coral ingest`

Incremental ingest from `last_commit` to `HEAD`.

```bash
coral ingest [--from <sha>] [--model <id>] [--provider claude|gemini]
```

- Reads `last_commit` from `.wiki/index.md` unless `--from` is given.
- Runs `git diff --name-status <from>..HEAD`, sends the summary to the LLM.
- Prints a YAML plan of `{slug, action, rationale}` items where `action ∈ {create, update, retire}`.
- **In v0.1, does not write pages.**
- `--provider` (or `CORAL_PROVIDER` env): `claude` (default) or `gemini`.

---

## `coral query`

Ask the wiki a question.

```bash
coral query "How is an order created?" [--model <id>] [--provider claude|gemini]
```

- Walks `.wiki/`, builds a snapshot context (truncated to 40 pages, 200 chars each).
- Sends question + context + citation instructions to the LLM.
- Prints the answer to stdout (cites slugs as `[[wikilink]]`).
- `--provider` (or `CORAL_PROVIDER` env): `claude` (default) or `gemini`.

---

## `coral lint`

Run structural and/or semantic lint.

```bash
coral lint [--structural] [--semantic] [--all] [--format markdown|json] [--provider claude|gemini]
```

- `--structural` (default if no flag): broken wikilinks, orphans, low confidence, high confidence without sources, stale status. Pure deterministic checks.
- `--semantic`: requires the LLM. Surfaces contradictions, obsolete claims, confidence/sources mismatches.
- `--all`: both.
- `--provider` (or `CORAL_PROVIDER` env): used by `--semantic` only. `claude` (default) or `gemini`.
- Exit code `1` if any **critical** issue; `0` otherwise.

---

## `coral consolidate`

Suggest page consolidations.

```bash
coral consolidate [--model <id>] [--provider claude|gemini]
```

- Lists all page slugs (truncated to 80) and asks the LLM for merge / retire / split candidates.
- Prints a YAML proposal. **Does not apply changes.**
- `--provider` (or `CORAL_PROVIDER` env): `claude` (default) or `gemini`.

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
coral onboard [--profile "backend dev" | "data engineer" | "PM" | "on-call"] [--model <id>] [--provider claude|gemini]
```

- Sends the wiki page list + profile to the LLM.
- Prints a Markdown ordered list of 5–10 pages with rationales.
- `--provider` (or `CORAL_PROVIDER` env): `claude` (default) or `gemini`.

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

TF-IDF search over the wiki.

```bash
coral search "<query>" [--limit <N>] [--format markdown|json]
```

- Tokenizes slug + body, scores via TF-IDF, returns the top-N pages (default 5).
- Stopwords (English + Spanish) filtered out. Single-character tokens dropped.
- Snippets are 200-char windows around the first matching token.
- `--format json` for downstream tooling.
- v0.2 ships TF-IDF (deterministic, no API key). v0.3 will switch to embeddings (Voyage / Anthropic) — see [ADR 0006](./adr/0006-local-semantic-search-storage.md). The CLI surface stays the same on upgrade.

---

## `coral export`

Export the wiki to various target formats.

```bash
coral export [--format markdown-bundle|json|notion-json|jsonl] [--out FILE] [--type TYPE]...
```

- `markdown-bundle` (default): single Markdown file with all pages concatenated. Useful for printing or feeding to another LLM as context.
- `json`: raw JSON array, one object per page (`slug`, `type`, `confidence`, `sources`, `backlinks`, `body`, ...).
- `notion-json`: array of Notion API `POST /v1/pages` request bodies, ready to be `curl`-posted to a Notion database. Set `parent.database_id` from your config.
- `jsonl`: one JSON object per line — `{slug, body, prompt}` — a stub starting point for fine-tuning datasets. v0.3 will add LLM-generated Q/A pairs.

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
coral export --format jsonl --out wiki-dataset.jsonl
```

---

## `coral notion-push`

Push wiki pages directly to a Notion database. Thin wrapper over `coral export --format notion-json` + curl.

```bash
coral notion-push [--token <TOKEN>] [--database <DB_ID>] [--type <TYPE>] [--dry-run]
```

Env vars (alternative to flags):
- `NOTION_TOKEN` — Notion integration token (required).
- `CORAL_NOTION_DB` — target database id (required).

Filter by page type with `--type concept --type module` (repeatable).

`--dry-run` prints what would be pushed without calling Notion.

Exit code: `0` if all pages POST cleanly (HTTP 2xx), `1` otherwise.

### Setup

1. Create an internal integration at https://www.notion.so/my-integrations — copy the secret as `NOTION_TOKEN`.
2. Create a database with these properties: `Name` (title), `Type` (select), `Status` (select), `Confidence` (number).
3. Share the database with your integration. Copy the database id (32-char hex from the URL).
4. `export NOTION_TOKEN=secret_…` and `export CORAL_NOTION_DB=…`. Run `coral notion-push --dry-run` to verify.

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
