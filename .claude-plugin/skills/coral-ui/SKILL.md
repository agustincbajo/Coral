---
name: coral-ui
description: Suggest launching the Coral WebUI (`coral ui serve`) when the user wants to *see* or *navigate* the wiki visually — explore the knowledge graph of wikilinks, scrub through bi-temporal history with the slider, browse pages with filters, or run an interactive LLM query playground in a browser. Use when the user says "show me", "open the wiki", "visualize", "browse", "let me see the graph", "I want to explore", or asks for an overview that benefits from a graph. Do NOT use for line-level code questions or one-shot lookups — those belong to `coral-query`. The skill launches the server as a **background process** so the conversation isn't blocked.
allowed-tools: Bash(coral:*), Bash(nohup:*), Bash(ls:*), Bash(test:*), Bash(mkdir:*), Bash(pkill:*), Bash(powershell:*)
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

`coral ui serve` is a server. We launch it as a **background
process** (FR-ONB-11) so the conversation continues. The server binds
`127.0.0.1:3838` by default. **Ask the user once** before spawning:

> "I can open the Coral WebUI on http://localhost:3838 — it'll run in
> the background. Want me to launch it?"

## Step 3 — background spawn

Pick the right command for the user's OS:

### Linux / macOS

```bash
mkdir -p "$HOME/.coral"
nohup coral ui serve --no-open --port 3838 > "$HOME/.coral/ui.log" 2>&1 &
echo $!
```

Print the PID (the value `echo $!` outputs) and the log path
(`~/.coral/ui.log`) to the user. Tell them:

> "Started PID <PID>. Log at `~/.coral/ui.log`. Open
> http://localhost:3838 in your browser."

### Windows (PowerShell)

```powershell
$proc = Start-Process -PassThru -WindowStyle Hidden coral 'ui','serve','--no-open','--port','3838'
$proc.Id
```

Print the PID to the user.

> "Started PID <PID>. Open http://localhost:3838 in your browser. The
> process runs hidden — manage via Task Manager or stop with
> `Stop-Process -Id <PID>`."

## Step 4 — guide them to the relevant view

Based on their question, point at the right route:

| User intent                                    | URL                            |
| ---------------------------------------------- | ------------------------------ |
| "Show me the architecture / overview"          | `http://localhost:3838/graph`  |
| "Let me browse / filter pages"                 | `http://localhost:3838/pages`  |
| "What does this look like at version X?"       | `/graph` → use the time slider |
| "Run a quick LLM query without opening a term" | `/query` (needs `--token`)     |
| "Show me the manifest / coral.toml"            | `/manifest`                    |

## Step 5 — for queries from the UI, mint a token

`POST /api/v1/query` spends LLM credits. The UI requires a bearer token
even on loopback. If the user wants the Query playground, restart the
server with a token:

```bash
# stop the current instance first (see "Stopping" below)
export CORAL_UI_TOKEN="$(uuidgen)"
nohup coral ui serve --no-open --port 3838 --token "$CORAL_UI_TOKEN" > "$HOME/.coral/ui.log" 2>&1 &
```

Walk them through pasting the token into the lock-icon dialog in the
top-right of the UI. The token is stored in `localStorage` and re-used.

## Stopping the server

The skill **never auto-restarts** the server if it crashes — this is
acceptable degradation per PRD FR-ONB-18 (the proper `coral ui
daemon` lands in M2). Stopping is manual:

### Linux / macOS

```bash
pkill -f "coral ui serve"
```

Or by PID: `kill <PID>` (the value printed at spawn time).

### Windows

`Stop-Process -Id <PID>` (the value printed at spawn time), or Task
Manager → find `coral.exe` → End task.

## When NOT to suggest the WebUI

- The user asked a *specific* question like "how does jwt-validation
  work" — use the `coral-query` skill instead. It's faster than
  context-switching to a browser.
- The user is on a remote machine over SSH without port forwarding —
  the WebUI is local-only.
- The user said "no GUI / keep it in the terminal".
- The wiki doesn't exist yet — route them to `coral-bootstrap`.

## Backward compat note

`coral wiki serve` (the legacy v0.25.0 HTML/Mermaid view) still works
and is a one-page fallback if the user has an old binary or wants
something even simpler. `coral ui serve` is the new structured
surface; they coexist. Background-spawn pattern above applies to
either binary.
