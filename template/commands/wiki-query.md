---
description: Query the wiki. Reads pages under .wiki/ and returns a cited answer.
argument-hint: <question>
allowed-tools: Read, Glob, Grep
---

Use the @wiki-bibliotecario subagent in read-only mode to answer the user's question:

$ARGUMENTS

The agent must:
- Read only `.wiki/` (never `src/`, `docs/`, etc.)
- Cite pages by slug in `[[wikilink]]` form
- Be terse; aim for under 200 words unless the question demands depth
