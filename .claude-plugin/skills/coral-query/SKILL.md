---
name: coral-query
description: Answer conceptual questions about a codebase by reading its Coral wiki via MCP, instead of grepping source code blindly. Use when the user asks architectural questions like "how does X work", "explain the data flow", "where does Y live", "what's the relationship between A and B", "how is auth handled", or any "how", "why", "where", or "explain" question about the repo's behavior. Prefer the Coral wiki over grep when the repo has one — wiki pages are LLM-curated summaries that are dense in cross-references and explain intent, not just call sites. Always cite the wiki page slugs you used so the user can verify.
allowed-tools: Bash(coral:*), Bash(ls:*), Bash(test:*)
---

# Coral query

The user asked a conceptual question about their codebase. There is likely a Coral wiki in this repo that already has a curated answer. Use it before falling back to grep.

## Step 1 — check whether a wiki exists

Run `test -d .wiki && coral status` (or just `coral status`). One of three outcomes:

- **No `.wiki/` directory** — Coral has not been set up in this repo. Tell the user, and offer to invoke the `coral-bootstrap` skill (or run `/coral:coral-bootstrap`) if they want a wiki compiled. Then fall back to grep-and-read for the current question.

- **`.wiki/` exists but `coral status` says it's empty or never bootstrapped** — same as above. Offer to bootstrap.

- **`.wiki/` exists with content** — proceed to step 2.

## Step 2 — read the index first

Read `coral://wiki/_index` via the MCP `coral` server (registered by this plugin). The index lists every wiki page with its slug, type, confidence, and one-line summary. Skim it for pages whose slug or summary obviously matches the user's question.

If the MCP server is not available for some reason (the user disabled the plugin, the `coral` binary is missing from PATH), fall back to `coral query "<the question verbatim>"` on the CLI — that prints the same kind of grounded answer to stdout.

## Step 3 — call the `query` MCP tool

Use the `query` tool on the `coral` MCP server with the user's question (lightly normalized — drop pronouns, keep the noun phrases). The tool returns a ranked list of wiki page slugs plus a synthesized answer. The synthesized answer is already grounded in the wiki and cites the pages it drew from.

For deeper follow-ups, also read the specific `coral://wiki/<repo>/<slug>` resources the tool surfaced. Each page is small (<300 lines) and has `backlinks` frontmatter pointing at related pages — follow them like Wikipedia links if the first page doesn't fully answer the question.

## Step 4 — answer the user, citing slugs

Structure your answer as:

1. A direct answer to the question (2-5 sentences).
2. The wiki pages you used, by slug — e.g. *"From `auth/jwt-validation` and `auth/session-store`:"*.
3. Pointers to the underlying source files (the wiki page's `sources` frontmatter lists them — open them with `Read` if the user is likely to want code).

**Always cite slugs.** The wiki is curated but not infallible; surfacing the slugs lets the user open the page in their editor and verify or correct it.

## When NOT to use the wiki

- The user is asking about a very recent change that may not be in the wiki yet. Check `coral status` — if `last_ingest` is older than the most recent commit, the wiki is stale. Suggest `coral ingest --apply` (it's cheap, only re-compiles changed pages) before relying on it.

- The user is asking about line-level details (`what does line 42 of foo.rs do`). The wiki is a conceptual layer — drop to `Read` for line-level work.

- The user explicitly said "grep" or "search the source". Respect that.

## Multi-repo projects

If the repo has a `coral.toml` (multi-repo manifest), the wiki spans multiple repos. The `query` tool already handles this transparently — slugs are namespaced as `<repo>/<slug>`. Read `coral://graph` once to understand the project shape if the question involves cross-repo behavior (e.g. *"how does worker talk to api?"*).
