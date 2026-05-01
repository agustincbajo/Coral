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
coral bootstrap [--model <id>]
```

- Walks the repo (skips `.git/`, `target/`, `.wiki/`, `node_modules/`, `.idea/`, `.vscode/`).
- Sends a truncated file listing (max 200 paths) to the LLM via `claude --print`.
- Prints a YAML suggestion of 5–15 page slugs (type + rationale).
- **In v0.1, does not write pages.** Apply suggestions manually.

---

## `coral ingest`

Incremental ingest from `last_commit` to `HEAD`.

```bash
coral ingest [--from <sha>] [--model <id>]
```

- Reads `last_commit` from `.wiki/index.md` unless `--from` is given.
- Runs `git diff --name-status <from>..HEAD`, sends the summary to the LLM.
- Prints a YAML plan of `{slug, action, rationale}` items where `action ∈ {create, update, retire}`.
- **In v0.1, does not write pages.**

---

## `coral query`

Ask the wiki a question.

```bash
coral query "How is an order created?" [--model <id>]
```

- Walks `.wiki/`, builds a snapshot context (truncated to 40 pages, 200 chars each).
- Sends question + context + citation instructions to the LLM.
- Prints the answer to stdout (cites slugs as `[[wikilink]]`).

---

## `coral lint`

Run structural and/or semantic lint.

```bash
coral lint [--structural] [--semantic] [--all] [--format markdown|json]
```

- `--structural` (default if no flag): broken wikilinks, orphans, low confidence, high confidence without sources, stale status. Pure deterministic checks.
- `--semantic`: requires the LLM. Surfaces contradictions, obsolete claims, confidence/sources mismatches.
- `--all`: both.
- Exit code `1` if any **critical** issue; `0` otherwise.

---

## `coral consolidate`

Suggest page consolidations.

```bash
coral consolidate [--model <id>]
```

- Lists all page slugs (truncated to 80) and asks the LLM for merge / retire / split candidates.
- Prints a YAML proposal. **Does not apply changes.**

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
coral onboard [--profile "backend dev" | "data engineer" | "PM" | "on-call"] [--model <id>]
```

- Sends the wiki page list + profile to the LLM.
- Prints a Markdown ordered list of 5–10 pages with rationales.

---

## `coral search`

Semantic search over the wiki.

> **Not implemented in v0.1.** Returns exit code 2. Coming in v0.2 via local embeddings (sqlite-vec or qmd).
