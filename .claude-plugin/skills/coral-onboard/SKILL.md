---
name: coral-onboard
description: Guide a developer who is new to a codebase. Use when the user says they're new to the repo, asks for an onboarding path, a guided tour, a "where do I start", a "give me the lay of the land", a "what should I read first", or any variant of getting up to speed on an unfamiliar codebase. Walks them through the Coral wiki's curated entry points, the `coral onboard` subcommand if applicable, and (for multi-repo projects) the project graph.
allowed-tools: Bash(coral:*), Bash(ls:*), Bash(test:*), Bash(git:*)
---

# Coral onboard

The user is new to this codebase and wants an entry point. Coral wikis are designed for exactly this: a small set of curated pages a human (or an LLM) can read top-to-bottom to grok the repo in 10-30 minutes.

## Step 1 — confirm there's something to onboard from

```bash
coral status
```

Three outcomes:

- **No `.wiki/`** — there's no curated onboarding material. Two options:
  1. Offer to invoke the `coral-bootstrap` skill so the user gets a wiki first (best for projects they'll work in for more than a day).
  2. Fall back to a manual tour: read `README.md`, list top-level directories, read the entry point (`main.rs` / `src/index.ts` / `cmd/<name>/main.go` / `app.py` / `manage.py`), and summarize.

- **`.wiki/` exists but is stale or empty** — offer to run `coral ingest --apply` first (cheap, only updates changed pages).

- **`.wiki/` exists with content** — proceed.

## Step 2 — use `coral onboard` if available

```bash
coral onboard
```

This subcommand (when the wiki has it scaffolded) prints a curated reading order: the 5-10 wiki pages a new contributor should read first, in order, with one-line rationales. It's the wiki's own answer to "where do I start". If it returns useful output, paraphrase it for the user with the slugs as clickable file paths (`.wiki/<repo>/<slug>.md`).

If `coral onboard` is empty or errors, fall back to step 3.

## Step 3 — read the wiki index by hand

Read `coral://wiki/_index` via the MCP `coral` server. Look for pages with:

- `type: overview` or `type: architecture` — high-level
- High `confidence` — pages the LLM was sure of
- High `backlinks` count — pages other pages reference, i.e. central concepts

Pick 3-7 pages and recommend a reading order. For each, give the slug, one sentence on what it covers, and what it sets up for the next page.

## Step 4 — for multi-repo projects, render the graph

If there's a `coral.toml` at the repo root, this is a multi-repo project. Show the graph:

```bash
coral project graph --format mermaid
```

The Mermaid output renders inline in Markdown — paste it into your answer. It shows which repo depends on which, which the user needs to mentally compose before any single repo's wiki makes sense.

Also recommend `coral project doctor` so they can see whether all the repos in the manifest are actually cloned and in-sync locally.

## Step 5 — point at follow-up resources

Wrap up with:

- *"After you've read those pages, ask me conceptual questions and I'll use the `coral-query` skill to ground my answers in the wiki."*
- *"If you spot something in the wiki that's wrong, edit the page in `.wiki/<slug>.md` directly and run `coral lint` — the wiki is meant to be human-curatable."*
- *"For the daily-use loop after you're onboarded: `coral status` is the dashboard."*

## Tone

This is the user's first 30 minutes in a strange codebase. Keep recommendations short. Don't dump 20 file paths at them. Three to seven pages, in order, with rationales — more is overwhelming.
