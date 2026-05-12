---
name: coral-ui
description: Suggest launching the Coral WebUI (`coral ui serve`) when the user wants to *see* or *navigate* the wiki visually — explore the knowledge graph of wikilinks, scrub through bi-temporal history with the slider, browse pages with filters, or run an interactive LLM query playground in a browser. Use when the user says "show me", "open the wiki", "visualize", "browse", "let me see the graph", "I want to explore", or asks for an overview that benefits from a graph. Do NOT use for line-level code questions or one-shot lookups — those belong to `coral-query`. Always confirm before running because `coral ui serve` is a long-lived foreground process.
allowed-tools: Bash(coral:*), Bash(ls:*), Bash(test:*)
---

# Coral UI

The user wants to **see** the wiki, not just query it. There is a
modern WebUI shipped inside the `coral` binary (v0.32.0+) that opens in
their browser. Suggest it when:

- They ask for an *overview*, *architecture map*, *project shape*, or
  *graph* — Sigma-rendered wikilinks tell a story that text can't.
- They mention *bi-temporal* needs ("how did this look in March?",
  "show me the wiki at the previous release tag"). The Graph view has
  a slider on `valid_from` / `valid_to` that no other tool surfaces.
- They want to *interactively explore* — filtering pages by status,
  type, confidence, with instant repaint.
- They want a *playground* for `coral query` without a terminal.

## Step 1 — confirm prerequisites

Check that the repo has a wiki:

```bash
test -d .wiki && coral status
```

If `.wiki/` doesn't exist or is empty, suggest the `coral-bootstrap`
skill first. `coral ui serve` fails fast in that case anyway.

## Step 2 — confirm with the user before running

`coral ui serve` is a long-lived foreground process. It binds
`127.0.0.1:3838` by default. **Ask the user once** before running:

> "I can open the Coral WebUI in your browser — it'll start a local
> server on http://localhost:3838. Want me to launch it?"

If they confirm, run it as a long-running background process. The
binary opens the browser automatically.

```bash
coral ui serve
# (or `coral ui serve --no-open` if they prefer to open the URL themselves)
```

## Step 3 — guide them to the relevant view

Based on their question, point at the right route:

| User intent                                    | URL                           |
| ---------------------------------------------- | ----------------------------- |
| "Show me the architecture / overview"          | `http://localhost:3838/graph` |
| "Let me browse / filter pages"                 | `http://localhost:3838/pages` |
| "What does this look like at version X?"       | `/graph` → use the time slider |
| "Run a quick LLM query without opening a term" | `/query` (needs `--token`)    |
| "Show me the manifest / coral.toml"            | `/manifest`                   |

## Step 4 — for queries from the UI, mint a token

`POST /api/v1/query` spends LLM credits. The UI requires a bearer token
even on loopback. If the user wants the Query playground:

```bash
export CORAL_UI_TOKEN="$(uuidgen)"
coral ui serve --token "$CORAL_UI_TOKEN"
```

Then walk them through pasting the token into the lock-icon dialog in
the top-right of the UI. The token is stored in `localStorage` and
re-used.

## When NOT to suggest the WebUI

- The user asked a *specific* question like "how does jwt-validation
  work" — use the `coral-query` skill instead. It's faster than
  context-switching to a browser.
- The user is on a remote machine over SSH without port forwarding —
  the WebUI is local-only.
- The user said "no GUI / keep it in the terminal".

## Stopping the server

When the user is done, Ctrl-C cleanly shuts down the server (SIGINT
handler is installed). If they ask you to stop it from a tool
invocation, kill the process.

## Backward compat note

`coral wiki serve` (the legacy v0.25.0 HTML/Mermaid view) still works
and is a one-page fallback if the user has an old binary or wants
something even simpler. `coral ui serve` is the new structured
surface; they coexist.
