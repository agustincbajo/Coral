---
title: "docs: README MCP resource/tool counts drift from code — claims 6 resources/5 read-only tools; code has 8 resources/7 read-only tools"
severity: Low
labels: docs, mcp
confidence: 5
cross_validated_by: [mcp-audit-agent, direct-code-read]
---

## Summary

README.md:85 says:

> 6 MCP resources + 5 read-only tools (3 more behind `--allow-write-tools`) + 3 prompts

README.md:779 reinforces the lower count by enumerating:

> Claude Code can read `coral://manifest`, `coral://lock`,
> `coral://wiki/<repo>/<slug>`, `coral://wiki/_index`, `coral://stats`,
> and call the read-only tools (`query`, `search`, `find_backlinks`,
> `affected_repos`, `verify`).

The code in `crates/coral-mcp/src/resources.rs:101-152`
(`static_catalog()`) returns **8** entries:

1. `coral://manifest`
2. `coral://lock`
3. `coral://graph`        ← not in README
4. `coral://wiki/_index`
5. `coral://stats`
6. `coral://test-report/latest`  ← not in README
7. `coral://contracts`    ← not in README
8. `coral://coverage`     ← not in README

`crates/coral-mcp/src/tools.rs::ToolCatalog::read_only()` returns **7**
tools (verified by counting; per the MCP audit agent: `query`, `search`,
`find_backlinks`, `affected_repos`, `verify`, `list_interfaces`,
`contract_status`). README lists 5.

## Why it matters

New users following README quickstart configure their MCP client
expecting 5 read-only tools and 6 resources; they then encounter 2-3
additional surfaces they have to figure out from the catalog. Worse,
an automation that compares the list returned by `resources/list` to
"the documented 6" treats the extras as anomalies.

## Suggested fix

Update README.md:85 to say "8 MCP resources + 7 read-only tools (N
behind `--allow-write-tools`) + 3 prompts" and update the enumeration
at README.md:779. Consider generating the README MCP section from the
catalog at release time so it can't drift again (a single `cargo run
--bin coral -- mcp catalog --format markdown` could emit the
documentation table).

## Cross-validation

MCP agent flagged this; I verified `static_catalog()` at
`resources.rs:101-152` directly.
