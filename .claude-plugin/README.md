# Coral plugin for Claude Code

This directory is the Claude Code plugin for [Coral](https://github.com/agustincbajo/Coral). It bundles:

- **Three auto-invoked skills** — `coral-bootstrap`, `coral-query`, `coral-onboard`. Claude picks them up from natural-language requests like *"set up Coral for this repo"*, *"how does authentication work"*, or *"I'm new to this codebase, where do I start"*.
- **Two slash commands** — `/coral:coral-bootstrap` and `/coral:coral-status` for explicit manual control.
- **The Coral MCP server** — automatically registers `coral mcp serve --transport stdio` so Claude can read your repo's wiki, manifest, lockfile, and test results across sessions.

## Install (from inside Claude Code)

```
/plugin marketplace add agustincbajo/Coral
/plugin install coral@coral
```

Then `/reload-plugins` to activate.

## Prerequisite — `coral` on PATH

The plugin registers an MCP server that invokes `coral`. You need the binary installed first. Three ways:

1. **One-line installer** (Linux/macOS/Windows) — fetches the latest release and places the binary on PATH:

   ```bash
   # Linux / macOS
   curl -fsSL https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.sh | bash
   ```

   ```powershell
   # Windows
   iwr -useb https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.ps1 | iex
   ```

2. **`cargo install`** (any platform with a Rust toolchain):

   ```bash
   cargo install --locked --git https://github.com/agustincbajo/Coral --tag v0.30.0 coral-cli
   ```

3. **Download a release tarball** manually from <https://github.com/agustincbajo/Coral/releases> and unpack `coral` (or `coral.exe`) onto your PATH.

Verify with `coral --version`. If `which coral` (or `where coral` on Windows) returns nothing, the plugin's MCP server will fail to start — Claude Code surfaces that in `/plugin` → Errors.

## What it does

After install, try any of these inside Claude Code:

- *"Set up Coral for this repo."* → `coral-bootstrap` skill walks the install (and confirms before the paid `bootstrap --apply` step).
- *"How does X work in this codebase?"* → `coral-query` skill reads `coral://wiki/_index` and the `query` MCP tool before grepping.
- *"I'm new here, where do I start?"* → `coral-onboard` skill walks the curated reading order.
- `/coral:coral-status` → dashboard summary.
- `/coral:coral-bootstrap` → manual bootstrap with explicit gating on the paid step.

Skills cost zero tokens until they're invoked — Claude only loads the body when the description matches what you asked.

## Upgrade

```
/plugin update coral@coral
/reload-plugins
```

The plugin's version (declared in `plugin.json` and `marketplace.json`) tracks the Coral binary's minor version. If you upgrade the binary but not the plugin (or vice versa) and something breaks, check `/plugin` → Errors and `coral --version` against the plugin's declared version.

## Files in this directory

```
.claude-plugin/
├── plugin.json                              # plugin manifest + MCP server registration
├── marketplace.json                         # makes the repo installable as a marketplace
├── README.md                                # this file
├── skills/
│   ├── coral-bootstrap/SKILL.md             # auto-invoked: setup
│   ├── coral-query/SKILL.md                 # auto-invoked: conceptual questions
│   └── coral-onboard/SKILL.md               # auto-invoked: new-to-repo
└── commands/
    ├── coral-bootstrap.md                   # /coral:coral-bootstrap
    └── coral-status.md                      # /coral:coral-status
```

For the full Coral docs (subcommand reference, wiki schema, multi-repo manifest, MCP tool catalog), see the main repo README: <https://github.com/agustincbajo/Coral>.
