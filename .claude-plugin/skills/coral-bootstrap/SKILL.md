---
name: coral-bootstrap
description: Scaffold and LLM-compile a Coral wiki for a git repo. Use when the user asks to "set up Coral", "initialize a wiki", "scaffold a wiki", "bootstrap the Coral wiki", "create a .wiki/ directory", "compile a wiki for this repo", or wants a first-time install of Coral in a fresh repo. The skill walks the happy path `coral init` → `coral bootstrap --apply` → `coral query`, and ALWAYS confirms with the user before running `bootstrap --apply` because that step calls an LLM and costs money.
allowed-tools: Bash(coral:*), Bash(ls:*), Bash(test:*)
---

# Coral bootstrap

Set up a Coral wiki in the current git repo. This is the one-time install flow. Subsequent updates use `coral ingest --apply`, not `bootstrap`.

## Preflight — before doing anything

1. **Verify `coral` is on PATH** — run `coral --version`. If it fails, stop and tell the user to install Coral first: `cargo install --locked --git https://github.com/agustincbajo/Coral --tag v0.30.0 coral-cli` or download a release tarball from https://github.com/agustincbajo/Coral/releases. Do NOT attempt to install it for them.

2. **Check for an existing wiki** — run `ls -la .wiki/ 2>/dev/null` (or `test -d .wiki`). If `.wiki/` already exists:
   - Tell the user the wiki is already scaffolded.
   - Ask whether they want to (a) re-run `coral ingest --apply` to refresh, (b) start over (they must `rm -rf .wiki/` themselves first — do NOT delete it for them), or (c) skip bootstrap and just run a query.
   - Do not run `coral init` or `coral bootstrap --apply` again over an existing wiki.

3. **Confirm this is a git repo** — `coral init` requires it. Run `test -d .git` (or `git rev-parse --is-inside-work-tree`). If not, tell the user to `git init` first.

## Happy path

### Step 1 — `coral init` (free, no LLM)

This scaffolds `.wiki/`, writes `SCHEMA.md`, drops the default `.coral/config.toml`, and creates an empty `.wiki/_index.md`. No LLM calls, no network. Safe to run.

```bash
coral init
```

Show the user the output. Confirm the files landed.

### Step 2 — confirm with the user BEFORE running `coral bootstrap --apply`

**This step costs LLM credits.** `coral bootstrap --apply` compiles every Markdown page in `.wiki/` from scratch by calling the user's configured LLM provider (Claude CLI by default; reads `.coral/config.toml`). Cost scales with repo size — small repos are typically a few cents, large monorepos can be several dollars.

Before running it, ask the user:

> *"`coral init` scaffolded the wiki. The next step, `coral bootstrap --apply`, will use your configured LLM (Claude by default — make sure `claude` is on PATH, or set a different provider in `.coral/config.toml`) to compile the initial wiki. This costs real money and runs once per repo. Roughly: a small repo is a few cents, a large monorepo can be several dollars. Do you want me to run it now, or do a `--dry-run` first to see what would happen?"*

Wait for an explicit yes / dry-run / no. Default to dry-run if the user is uncertain.

### Step 3 — run bootstrap (only after confirmation)

```bash
coral bootstrap --apply
```

If the user asked for dry-run first:

```bash
coral bootstrap --dry-run
```

Show the output. If it fails, the most common causes are:
- `claude` (or the configured LLM provider) not on PATH → tell the user how to fix and stop.
- No API key for the embeddings provider → check `.coral/config.toml` and the relevant env var.
- A pre-existing `.wiki/` with stale content → suggest `coral consolidate` rather than re-bootstrapping.

### Step 4 — first query

Suggest one concrete starter question based on what the repo looks like (skim the README or `git ls-files | head` if helpful). Examples:

```bash
coral query "what does this repo do?"
coral query "how does authentication work?"
coral query "where is the entry point?"
```

If the user has Claude Code's `coral` MCP server registered (this plugin handles that automatically), tell them they can now ask the LLM conceptual questions about the repo and the LLM will read `coral://wiki/_index` and call the `query` tool to ground its answers. They don't have to run `coral query` manually anymore.

## After bootstrap

For day-to-day updates as code changes:

```bash
coral ingest --apply       # incremental — only re-compiles pages whose source files changed
coral status               # dashboard: page count, last ingest, lint status, drift
```

Never re-run `coral bootstrap --apply` for incremental updates — it nukes and rebuilds the entire wiki and is wasteful.

## Reference

- Full docs: https://github.com/agustincbajo/Coral
- Subcommand reference: `coral --help` and `coral <cmd> --help`
- Wiki schema: `.wiki/SCHEMA.md` (written by `coral init`)
