# Using Coral with Claude Code

Coral exposes the wiki + manifest + lockfile + test results as a [Model Context Protocol](https://modelcontextprotocol.io/) server. Claude Code connects to it via stdio and reads structured project context across sessions — no custom plugin, no copy-paste.

## Setup (1 minute)

1. Make sure `coral` is on your `$PATH`:
   ```bash
   coral --version    # should print 0.19.0 or newer
   ```

2. In your project directory, register Coral as an MCP server. Either edit `.claude/mcp.json` directly:

   ```json
   {
     "mcpServers": {
       "coral": {
         "command": "coral",
         "args": ["mcp", "serve"]
       }
     }
   }
   ```

   …or use the Claude Code CLI:

   ```bash
   claude mcp add coral coral mcp serve
   ```

3. Restart Claude Code (or `/mcp reload`).

That's it. From now on Claude Code can call:

| Tool | What it does |
|---|---|
| `query(q, repo?, tag?)` | Streamed LLM answer using the wiki as context (read-only). |
| `search(q)` | TF-IDF full-text search across the wiki, no LLM. |
| `find_backlinks(slug)` | List wiki slugs that link to a given slug. |
| `affected_repos(since)` | List repos whose SHA changed since a git ref (transitively walks `depends_on`). |
| `verify(env?)` | Run liveness healthchecks against the running environment. |

Claude Code can also read these resources on demand:

- `coral://manifest` — `coral.toml` parsed to JSON
- `coral://lock` — resolved SHAs
- `coral://graph` — repo dependency graph
- `coral://wiki/_index` — aggregated wiki listing
- `coral://stats` — wiki health stats
- `coral://test-report/latest` — last test run

Plus the templated prompts `prompts/onboard?profile`, `prompts/cross_repo_trace?flow`, and `prompts/code_review?repo&pr_number`.

## Read-only by default

Coral's MCP server defaults to read-only mode (`--read-only` is on unless you pass `--allow-write-tools`). This blocks the three write tools (`run_test`, `up`, `down`) so an agent that's been compromised by prompt injection in your wiki can't escalate to running arbitrary tests or tearing down your dev environment.

Per the PRD risk #25 (MCP server as exfiltration vector), if you do enable write tools, every invocation is logged to `.coral/audit.log` with timestamp, tool name, arguments, and response. Review it before re-enabling write tools in shared environments.

## What about Cursor / Continue / Cline?

Same flow — they all speak MCP. For Cursor you'd add it to the MCP config in settings; for Continue, add it to `~/.continue/config.yaml` under `mcpServers`. The Coral side is identical: `coral mcp serve`.

For non-MCP-speaking clients (or paste-into-chat workflows), use `coral export-agents` instead — it generates `AGENTS.md` / `CLAUDE.md` / `.cursor/rules/coral.mdc` / `.github/copilot-instructions.md` / `llms.txt` deterministically from `coral.toml`. See the README's [export-agents reference](../README.md#ai-ecosystem-layer-v019) for the full table.

## Troubleshooting

### Claude Code says "tool 'X' is not wired in this build"

The default v0.19 build ships `coral-mcp::NoOpDispatcher` — the resources/list and prompts/get endpoints work but `tools/call` returns scripted "skip" responses. Tool wiring (delegating to `coral query` / `coral search` / `coral verify`) lands in v0.19.x. Until then: use the resources to read context, not the tools.

### The MCP server starts but Claude Code doesn't see it

Check Claude Code's MCP logs (Cmd+Shift+P → "Show MCP logs" or equivalent). Common causes:
- `coral` not on `$PATH` for the shell Claude Code spawns
- `coral.toml` parse error — try `coral status` first to verify the project loads
- macOS Gatekeeper holding the binary — `xattr -d com.apple.quarantine $(which coral)`

### Resources/read returns -32601 "resource not found"

That's the v0.19 wave-1 stub for `WikiResourceProvider::read`. Resources/list works (returns the catalog), but the actual content read lands in v0.19.x with the wave-2 wiring. If you need wiki content right now, use `coral query` directly from the terminal or `coral context-build --query "X" > prompt.md` to paste into chat.
