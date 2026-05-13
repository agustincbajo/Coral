# Anthropic plugin marketplace submission — Coral

Copy-paste-ready text for `claude.ai/settings/plugins/submit` (Claude.ai
users) or `platform.claude.com/plugins/submit` (Console users). The
form is browser-only and not formally documented; the fields below
are educated guesses based on what Anthropic typically asks for
plugin listings. Adjust on the fly if the form asks for something
different.

---

## Form fields

### Plugin name

```
coral
```

Already kebab-case, single word, matches the binary + slash command
namespace (`/coral:coral-doctor`).

### Tagline (one-liner, ≤80 chars)

```
Karpathy-style bi-temporal LLM wiki for any git repo, with cost-confirmed bootstrap
```

(78 chars — fits a typical tweet-length tagline limit.)

### Short description (≤200 words)

```
Coral turns any git repo into a queryable Karpathy-style LLM wiki
(.wiki/), preserves a bi-temporal history of the codebase + the
docs about the codebase, and exposes it to Claude Code via an MCP
server so you can ask architecture questions without re-reading
the whole codebase.

Five auto-invoked skills (bootstrap, query, onboard, ui, doctor)
plus two deterministic slash commands cover the entire lifecycle:
generate the wiki with cost-confirmed bootstrap, query it
conversationally, onboard new contributors, browse a force-
directed graph WebUI, or diagnose environment issues. A
SessionStart hook reports repo state to Claude on every session
open so the right next action is suggested without the user
needing to know skill names. A 4-path provider mini-wizard
(Anthropic API key / Gemini / Ollama / claude CLI) onboards users
who don't have claude CLI yet.

Single-binary distribution (~14 MiB stripped) with Sigstore
provenance, cross-platform install (Linux / macOS / Windows),
SLSA L3-equivalent attestation on every release, and a 7-week
internal audit cycle behind it (docs/audits/ in the repo).
```

### Long description (form section "What does this plugin do?")

```
Coral is a documentation-as-code tool that maintains an LLM-readable
wiki for any git repo, gives Claude Code direct read access to it
via an MCP server, and keeps the wiki bi-temporally synchronized
with the underlying codebase.

# The wiki

A `.wiki/` directory at the repo root contains one markdown page
per significant concept, module, flow, or interface. Each page has
typed frontmatter (slug, sources, last-updated-commit, confidence)
plus a human-readable body. Pages cross-reference each other via
wikilinks (`[[other-slug]]`), and Coral maintains a force-directed
graph view of the relationships via its WebUI (`coral ui serve`).

The bi-temporal aspect: every page records the commit SHA it was
generated against, AND the wiki is timestamped on every change.
Users can ask "show me the architecture as it was 3 weeks ago" or
"what changed in the `outbox` page between v1.0 and v1.1" — the
slider in the WebUI exposes both axes.

# The Claude Code integration

The Coral plugin registers an MCP server (`coral mcp serve`) that
exposes the wiki as MCP resources. Claude reads pages, follows
wikilinks, and answers user questions without re-scanning the
repository on every prompt. The MCP server runs over stdio by
default and HTTP/SSE optionally (for shared multi-agent setups —
the HTTP transport is bearer-auth-protected and ships with CSPRNG
session IDs).

Five skills auto-invoke based on user intent:
- coral-bootstrap — first-time wiki generation with cost
  confirmation (shows estimate + upper-bound before spending real
  $$ on LLM calls; `--max-cost` aborts mid-flight if exceeded)
- coral-query — conversational lookup against the wiki
- coral-onboard — recommended reading order for new contributors
- coral-ui — background-spawn the WebUI for graphical browsing
- coral-doctor — diagnose environment + provider mini-wizard

Two slash commands give explicit determinism when needed:
- /coral:coral-bootstrap
- /coral:coral-doctor

A SessionStart hook runs `coral self-check --quick` whenever
Claude Code opens a Coral-enabled repo, reporting environment
state as silent context so Claude can suggest the right next
action without the user typing anything specific.

# What's NOT in Coral

- No SaaS, no phone-home telemetry, no auto-submission of code.
  Everything runs local. (Anti-feature AF-1 documented in the PRD.)
- No automatic LLM calls — every cost-incurring command shows the
  estimate and asks for confirmation, with `--max-cost=USD` hard
  abort.
- No auto-installation of secrets — API keys land in
  `.coral/config.toml` (chmod 600 on Unix) only after the user
  pastes them in the provider mini-wizard.

# Distribution

- Single binary (~14 MiB stripped on Linux x86_64).
- Cross-platform: Linux glibc, macOS x86_64 + aarch64, Windows MSVC.
- Sigstore provenance + SLSA L3-equivalent attestation on every
  GitHub release (verifiable via `gh attestation verify`).
- Install via `curl install.sh | bash` (Linux/macOS) or
  `iwr install.ps1 | iex` (Windows).
- Plugin discovered via `/plugin marketplace add agustincbajo/Coral`
  (today) or via this curated Anthropic marketplace listing once
  approved.
```

### License

```
MIT
```

### Category

```
developer-tools
```

(Matches the existing `marketplace.json` field.)

### GitHub repository

```
agustincbajo/Coral
```

### Homepage / project URL

```
https://github.com/agustincbajo/Coral
```

### Maintainer contact

Use whatever you put in the form by default — the GitHub repo's
issues page + commit email are the canonical channels for
maintenance.

### Tags / keywords

```
mcp, wiki, multi-repo, context-engineering, rust, bi-temporal, bootstrap, sigstore, slsa
```

(Coral's existing 5 + 4 audit-related for marketplace SEO.)

---

## Use cases (the meat of the marketplace appeal)

Pick whichever 3-4 resonate with the platform's audience. The form
may ask for "primary use case" — go with the first one, it's the
broadest.

### 1. Onboarding a new hire to a 50k-LOC backend

**Before Coral.** New dev clones the repo, opens `README.md`, then
spends a week pinging Slack for "where does the order flow start?"
and "how does the outbox dispatch retry?". Senior devs re-answer
the same questions every quarter.

**With Coral.** Maintainer ran `coral bootstrap --apply` once
(estimated cost: $0.42 on claude-sonnet-4.5 for the 47 pages over
this repo). The new dev opens Claude Code in the repo and types
"explain the order flow". Claude routes through the SessionStart
hook context + the `coral-query` skill + the MCP server, reads
the relevant `flows/order.md` page, follows wikilinks to
`modules/order.md` + `concepts/outbox.md`, and answers in
context — citing line numbers and commits. New dev's first day is
productive instead of waiting for Slack.

### 2. Documenting a refactor before/after

**Before Coral.** Senior dev refactors the auth module across 8
files. PR description has bullet points. Reviewers ask "what did
this change vs the old design?" and there's no easy diff of the
docs.

**With Coral.** Pre-refactor, the wiki has `concepts/auth.md` at
commit `abc123`. Post-refactor, the dev runs `coral ingest --apply`
(updates the wiki for the touched files; cost ~$0.03). The wiki
slider lets reviewers compare `auth.md` at `abc123` vs `def456`
side-by-side. The PR description is now "see the wiki diff at the
attached URL" — zero hand-written bullet points lost in
translation.

### 3. Multi-repo monorepo with 12+ services

**Before Coral.** Each service has its own README + ADRs. Cross-
service questions ("which services consume `user-events`?") require
grep across 12 repos. The architect maintains a private Notion
doc that's always 6 weeks out of date.

**With Coral.** `coral.toml` declares the 12 repos. `coral project
sync` clones them into one workspace. `coral bootstrap --apply`
generates per-repo wikis + a top-level manifest. The WebUI's
`/manifest` view shows declared interfaces; `/affected --since=v2.0
--by=user-events` shows which services would re-deploy if
`user-events` changes. The architect's Notion doc is replaced by
queryable, code-aware truth.

### 4. AI/LLM-cost-conscious team

**Before Coral.** Team wants to use LLMs for codebase documentation
but every "let's just run this on the codebase" turns into a $50
surprise bill at month-end. PR reviews avoid LLM tools because
"we're not sure what it'll cost."

**With Coral.** Every cost-incurring command shows `--estimate`
first with both a point estimate and an upper bound:

```
Estimated cost: $0.42 (up to $0.53 — margin ±25%)
Pages: 47 | Tokens: ~200k | Provider: claude-sonnet-4.5
```

The user runs `coral bootstrap --apply --max-cost=0.50` to enforce
a hard cap. If a transient provider issue inflates real cost mid-
flight, Coral aborts at the cap, marks the state `partial: true`,
and `coral bootstrap --resume` continues from the last completed
page without re-paying.

### 5. CI/CD gates on architectural contracts

**Before Coral.** Team has a soft "we agreed at the design review
that the user-service emits a `user.created` event." Six months
later someone changes the event shape and breaks 4 downstream
services. No one noticed because the agreement was tribal.

**With Coral.** `coral.toml` declares contract files; `coral test
guarantee --can-i-deploy` runs in GitHub Actions before every
merge. If the `user.created` event schema changes in a way that
breaks consumers (declared in their respective `coral.toml`s),
the CI gate goes red and the PR can't merge. Contract checking
without paying for a dedicated contract-test framework.

### 6. Audit trail for LLM-generated documentation

**Before Coral.** Team uses ChatGPT to generate docs, pastes
output into `docs/architecture.md`, commits. Six months later
nobody knows which lines a human wrote vs which the LLM
hallucinated.

**With Coral.** Every page Coral generates has frontmatter
declaring `reviewed: false` until a human flips it. `coral lint`
emits a `Critical: UnreviewedDistilled` finding for any
`reviewed: false` page; pre-commit hooks block the commit until
the human approves the content. The wiki's bi-temporal history
shows exactly when LLM-generated content was reviewed by which
git author.

### 7. Plug-and-play for users without claude CLI

**Before Coral.** User discovers a Rust tool via blog post,
installs the binary, types `tool init`, gets a wall of error
about "ANTHROPIC_API_KEY not set" or "claude CLI not found".
Bails.

**With Coral.** User runs `coral doctor --wizard`. A four-path
interactive prompt asks: do you want to use (1) an Anthropic API
key directly, (2) Gemini, (3) Ollama for fully local LLM, or (4)
install the official claude CLI? Each path is verified with a
1-token ping (no wasted spend) and persisted to
`.coral/config.toml` only after the provider responds 200. User
moves on without Slack search.

### 8. Bi-temporal "wait, what did the docs say at v1.2?"

**Before Coral.** Senior engineer remembers the architecture
diagram had an event-bus in 2024 but the current diagram doesn't.
"Was that on purpose? When did it change?" No one knows. Git blame
on the docs file shows a 600-line refactor commit.

**With Coral.** The wiki bi-temporal slider in `coral ui serve`
lets the engineer drag back to the v1.2 release date. The
`architecture.md` page at that point shows the event-bus. The
"valid-from"/"valid-to" metadata on each fact reveals the
2025-Q1 commit that removed it, with the PR justification linked
in the frontmatter. Decision archaeology that takes 5 minutes,
not 5 hours.

### 9. Security-conscious enterprise pilot

**Before Coral.** Procurement asks "what's your supply chain?".
The tool vendor sends a 12-page PDF. Procurement asks "can we
verify the binary was built from the open-source code?". Vendor
shrugs.

**With Coral.** Every release ships with Sigstore in-toto
attestation. End user runs:

```bash
gh attestation verify coral-v0.37.0-x86_64-unknown-linux-gnu.tar.gz \
  --repo agustincbajo/Coral
```

Verification succeeds against the GitHub OIDC + Sigstore public-
good instance, binding the artifact's SHA-256 to the exact
`release.yml` workflow + commit SHA + builder identity. SLSA L3-
equivalent provenance, zero long-lived signing keys. Procurement
gets `docs/SLSA-VERIFICATION.md` as the canonical reference.

---

## Quick-start install snippet (for the form's "How do users
install this?" field if present)

```bash
# Linux / macOS — one-line install + plugin marketplace registration
curl -fsSL https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.sh \
  | bash -s -- --with-claude-config

# Windows PowerShell — equivalent (uses install.ps1)
& ([scriptblock]::Create((iwr -useb https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.ps1).Content)) -WithClaudeConfig

# Then open Claude Code in your repo and type anything ("hola", "what
# does this repo do?", "/coral:coral-bootstrap"...). The plugin's
# CLAUDE.md routing + SessionStart hook handle the rest.
```

If they're already in Claude Code without the binary installed,
the manual paste route works too:

```
/plugin marketplace add agustincbajo/Coral
/plugin install coral@coral
/reload-plugins
```

## Screenshot suggestions (if the form asks for visuals)

1. **WebUI graph view** — `coral ui serve` running, force-directed
   graph of 30+ pages with the bi-temporal slider at the bottom.
2. **MCP query in Claude Code** — Claude responding to "explain
   the auth flow" with citations from the wiki, MCP tool calls
   visible in the inspector.
3. **Provider wizard** — terminal output of `coral doctor --wizard`
   asking "which provider?".
4. **Cost estimate** — `coral bootstrap --estimate` output showing
   "Estimated cost: $0.42 (up to $0.53)".

Capture per the recipe in BACKLOG item #1 (Chrome headless with
`--enable-webgl --use-gl=swiftshader --window-size=1400,900`).

---

## Security model summary (if the form asks for one)

Coral runs entirely local. There is no phone-home, no anonymous
usage stats, no SaaS multi-tenant. Trust boundaries:

- **Binary**: Sigstore in-toto attestation on every release. SHA-256
  pinned in `.sha256` sidecars. Verifiable with `gh attestation
  verify` or `cosign verify-blob-attestation`. Documented in
  `docs/SLSA-VERIFICATION.md`.
- **API keys**: stored in `.coral/config.toml` (`chmod 600` on
  Unix; ACL TODO for Windows tracked in BACKLOG). Never logged.
  Only written after the provider returns 200 to a 1-token ping.
- **MCP HTTP transport**: bearer-auth gate ordered FIRST in the
  request handler (blocks pre-auth DoS). CSPRNG session IDs via
  `rand::random::<[u8;16]>()` → `OsRng` → kernel `getrandom`.
- **WebUI**: 128-bit entropy floor on `--token` (NIST SP 800-131A
  symmetric-secret floor). Auto-mint via CSPRNG when missing.
  Loopback bypass on `127.0.0.1` only.
- **Bootstrap LLM prompt injection**: `coral_lint::structural::
  check_injection` runs on every page body before disk write.
  Catches fake system prompts, embedded auth headers, base64
  payloads, unicode bidi-override.

Full audit trail: `docs/audits/AUDIT-SECURITY-2026-05-12.md` +
`docs/audits/SYNTHESIS-2026-05-12.md` (6-axis audit + validator-
paired pattern, ~14 Critical + ~52 High findings closed across
v0.35.0 — v0.37.0).

ADR-0010 documents the deliberate decision to stay with
synchronous `tiny_http` (no tokio) to keep the supply chain small
and the binary single-purpose.

---

## What's next if Anthropic asks for clarifications

The PRD itself (`docs/PRD-v0.34-onboarding.md`, ~1500 lines,
4 review iterations) is the canonical answer to "design
decisions" questions. ADRs at `docs/adr/0010..0012` cover the
specific architectural calls (blocking I/O substrate, MSRV policy,
mimalloc allocator). The audit synthesis at `docs/audits/
SYNTHESIS-2026-05-12.md` is the operational view of "what's
been measured + decided + shipped".
