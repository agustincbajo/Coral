# GTM drafts — community/social channels

Drafts for the v0.32.x → v0.39.0 release train. Tone is the
maintainer's decision (see BACKLOG #2). Each channel below carries
**two tone variants** — pick whichever fits, mix freely. All
drafts are **publish-ready text only**: no commitment to schedule
or channel mix.

The marketplace listing copy lives in
[`docs/SUBMISSION-DRAFT.md`](SUBMISSION-DRAFT.md) — this file covers
everything else (X/Twitter, HN, Reddit, Discord, LinkedIn).

**Last refreshed:** 2026-05-13, post-v0.39.0.

---

## What's actually new (factual scaffolding for any tone)

Use these as the "facts" any draft can borrow from. Don't invent
numbers — every figure here is measured or auditable.

- **Single Rust binary**, ~6.3 MB stripped on Linux x86_64, ~14 MiB on
  Windows MSVC release. Statically linked, ad-hoc-codesigned on
  macOS. MSRV 1.89.
- **42 leaf subcommands** across 6 layers (per README §What you get).
- **Five auto-invoked Claude Code skills** (bootstrap, query, onboard,
  ui, doctor) + 2 slash commands (`/coral:coral-bootstrap`,
  `/coral:coral-doctor`) + SessionStart hook.
- **Sigstore SLSA provenance** on every release artifact since v0.33.0
  (`gh attestation verify <asset> --repo agustincbajo/Coral`).
- **Bi-temporal wiki history** — time-travel slider in the WebUI,
  Karpathy-style "wiki maintainer" pattern.
- **Cost-confirmed bootstrap** — `coral bootstrap --estimate` shows
  predicted token spend before any LLM call; `--max-cost` enforces a
  hard ceiling.
- **4-path provider mini-wizard** — Anthropic API key / Gemini /
  Ollama / claude CLI install. Lands users on whichever provider is
  fastest for them.
- **`coral feedback submit`** — opt-in, zero-phone-home calibration
  sharing (AF-1 compliant: operator copies sanitized JSON manually
  into a discussion comment). New in v0.38.0.
- **mimalloc allocator** with per-platform measured baselines:
  +29.7–42.7% throughput vs system allocator across three
  representative workloads (Windows MSVC; macOS + Linux baselines
  landed in CI run 25804983205). ADR-0012.
- **Clippy panic-risk hard gate** at 0 warnings since v0.37.0 —
  every `unwrap`/`expect`/`panic` in production code is justified or
  refactored to `Result`.
- **`cargo deny check advisories`** passes with **zero suppressed
  advisories** since v0.39.0 (bincode→postcard migration cleared
  RUSTSEC-2025-0141).

## What's unique vs neighbors

These are the "why Coral, not X" deltas — useful when the channel
asks "isn't this just LightRAG / Cursor MDC / GraphRAG / ...?":

- **vs LightRAG / GraphRAG**: bi-temporal axis (wiki diff between
  any two commits + any two doc states). Time-travel queries.
- **vs Cursor `.cursor/rules` / Continue.dev custom commands**:
  multi-repo manifest (`coral.toml`) means one source-of-truth across
  N repos, not per-IDE config drift.
- **vs Claude Code plugins listed in code.claude.com**: Coral is the
  first plugin that builds a queryable wiki rather than just
  surfacing LSP/tool integration.
- **vs SaaS RAG products**: single binary, all-local, no telemetry,
  Sigstore-verified provenance, MIT.

---

## Channel: HackerNews — "Show HN" draft

### Title

```
Show HN: Coral – Karpathy-style LLM wiki + MCP server for any git repo
```

(63 chars — under HN's 80-char title limit. Includes "Show HN:"
prefix per HN convention.)

### Body

**Tone A — technical-restrained:**

```
Coral builds a queryable LLM wiki (.wiki/) over your git repo and
exposes it to Claude Code (and any MCP client) so architecture
questions get answered without re-reading the codebase.

It's a single Rust binary (~6.3 MB stripped on Linux), MIT,
statically linked, Sigstore-attested on every release.

What it does:
- `coral bootstrap` builds the wiki from your repo with a cost
  estimate up front (`--estimate`) and a hard ceiling (`--max-cost`).
- `coral query <question>` answers questions over the wiki using
  your configured provider (Anthropic / Gemini / Ollama / claude CLI).
- `coral ui serve` opens a force-directed graph WebUI with a bi-
  temporal slider — diff wiki state between any two commits.
- `coral mcp serve` exposes the wiki + manifest + lockfile + test
  results to Claude Code, Cursor, Continue, etc.
- Multi-repo: declare your N repos in coral.toml and Coral keeps a
  cross-repo lockfile so the wiki references stay consistent.

What it's not:
- Not a SaaS. All-local, zero telemetry, opt-in `coral feedback
  submit` is sanitized-JSON-to-discussion-comment by design.
- Not a wiki editor. Coral is the maintainer; the wiki is git-
  native markdown you can edit in any editor.
- Not multi-modal. Text + PDF only (and PDFs gated behind a flag).

The plug-and-play path is two commands:
  /plugin marketplace add agustincbajo/Coral
  /plugin install coral@coral

Or `curl install.sh | bash` if you prefer the binary directly.

Six audit cycles, five skills auto-invoked from natural language,
clippy panic-risk hard-gate at 0 warnings, cargo deny clean with
no suppressed advisories.

Feedback welcome. The PRD is in the repo (docs/PRD-v0.34-onboarding.md)
if you want to see what's planned next.

https://github.com/agustincbajo/Coral
```

**Tone B — builder-narrative:**

```
I've been wanting an LLM wiki for my own multi-repo setup that
doesn't lock me into a SaaS, doesn't phone home, runs offline if I
want it to, and ships as one statically-linked Rust binary. None
of the existing options checked every box, so I built one.

Coral is the result. It builds a Karpathy-style wiki (LLM
maintains it, you read it, it lives in your git repo) and exposes
it via Model Context Protocol so Claude Code (or any MCP client)
can answer architecture questions without re-reading the codebase.

Two non-obvious bets:
1. Bi-temporal axis — wiki state has its own history, separate
   from the code's history. The WebUI lets you scrub between any
   two commits and any two doc states. I haven't seen another
   wiki tool do this.
2. Single binary, six audit cycles. mimalloc + panic-risk hard
   gate + Sigstore attestation + cargo deny no-suppressed-
   advisories. Boring infra so the interesting parts can move fast.

Five auto-invoked Claude Code skills mean you don't have to learn
slash commands — just say "set up coral" or "where do I start?"
and the right skill fires from CLAUDE.md routing.

Install:
  /plugin marketplace add agustincbajo/Coral
  /plugin install coral@coral

Or `curl install.sh | bash`. MIT, source on GH, the PRD is in the
repo if you want to argue with the decisions.

https://github.com/agustincbajo/Coral
```

**Posting tip:** HN front-page hits don't correlate with quality —
they correlate with showing up at low-traffic hours (US West Coast
6-8am Pacific weekdays). The first 30 minutes of comment activity
decide everything. Be ready to respond.

---

## Channel: X / Twitter / Mastodon (thread)

### Tone A — visual-led, tweet-storm style

**Tweet 1 (hook + image):**

```
The wiki for your codebase, in one Rust binary.

Karpathy-style: LLM writes it, you read it, lives in your git repo.

Bi-temporal slider scrubs between any two commits AND any two doc
states. Single command to bootstrap, single command to query.

🧵👇

[attach docs/assets/ui-graph-en.png — the force-directed graph view]
```

**Tweet 2 (the install line):**

```
Install:

curl -fsSL https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.sh | bash

Or in Claude Code:
/plugin marketplace add agustincbajo/Coral
/plugin install coral@coral

That's it. Five auto-invoked skills handle the rest.
```

**Tweet 3 (the architecture pitch):**

```
What Coral isn't:
❌ SaaS
❌ Phones home
❌ Multi-modal bloat
❌ "Cloud-first"

What it is:
✅ ~6.3 MB stripped Linux binary
✅ MIT, all-local
✅ MCP server (Claude Code / Cursor / Continue / ...)
✅ Sigstore SLSA provenance on every release
✅ cargo deny clean, zero suppressed advisories
```

**Tweet 4 (the bi-temporal demo):**

```
The visual trick:

Wiki has TWO timelines.
- Code commits (git log)
- Doc revisions (separate)

Slider lets you scrub both independently — diff what the wiki
*said* in April vs what the code *was* in May. Catches drift
nobody else surfaces.

[attach docs/assets/ui-drift-en.png]
```

**Tweet 5 (community CTA):**

```
v0.39.0 just shipped:
- bincode → postcard (RUSTSEC-2025-0141 cleared)
- Coverage CI floor 55→60% (workspace measured at 83.81%)
- 1921 tests, 8/8 release jobs, 3/3 OS smoke green

Roadmap, PRD, and the BACKLOG are all in the repo.

https://github.com/agustincbajo/Coral
```

### Tone B — single-thread, narrative

**Tweet 1:**

```
Built a Karpathy-style LLM wiki maintainer for git repos.

Single Rust binary. MCP server. Five Claude Code skills auto-
invoke from natural language. No SaaS, no telemetry, statically
linked, Sigstore-attested every release.

v0.39.0 is live. Show-not-tell thread 👇
```

**Tweet 2-5:** rebuild from Tone A subset; the key claims that
should always be in the thread are:
- bi-temporal slider (the unique-vs-LightRAG move)
- ~6.3 MB binary (the anti-bloat signal)
- MCP server (the LLM-tooling signal)
- /plugin install one-liner (the friction-zero signal)

---

## Channel: Reddit — r/rust

**Title:**

```
Coral v0.39.0 — Rust-built LLM wiki maintainer (MCP server) for any git repo, MSRV 1.89, RUSTSEC-2025-0141 cleared
```

**Body:**

```
Coral is a Karpathy-style LLM wiki maintainer for git repos, built
in Rust as a single statically-linked binary (~6.3 MB stripped on
Linux). Exposes the wiki to Claude Code (and any MCP client) via
Model Context Protocol so LLMs can answer architecture questions
without re-reading the entire codebase.

Recent Rust-relevant work:

- v0.39.0: migrated coral-core::search_index from bincode 2.x to
  postcard 1.x; cleared RUSTSEC-2025-0141 (the bincode-upstream-
  unmaintained advisory, applies to all bincode versions). Drop
  was transparent — the existing decode-failure rebuild path
  absorbed the on-disk format change.

- v0.37.0: clippy panic-risk job promoted to hard CI gate at 0
  warnings. Every unwrap/expect/panic in production code is
  refactored or has a documented "statically unreachable" justification.

- v0.36.0: mimalloc allocator baseline benchmarks landed across
  Linux + macOS + Windows. Measured +29.7-42.7% throughput vs
  system allocator across TF-IDF / page-parse / JSON-Value
  workloads. ADR-0012 promoted from "claimed" to "measured".

- v0.36-prep: 17/33 (51%) modules demoted from pub to pub(crate)
  via curated re-exports at crate root. Phase C ARCH audit closure.

- v0.35.0: MSRV bumped 1.85→1.89, ADR-0011 documents why (let-
  else and async fn in traits both stabilized at 1.85 but the
  edition=2024 work needed 1.89).

The whole workspace is 10 crates (coral-{cli,core,env,lint,runner,
session,stats,test,mcp,ui}). cargo deny config in repo.

https://github.com/agustincbajo/Coral
```

## Channel: Reddit — r/programming or r/MachineLearning

**Title:**

```
Coral — bi-temporal LLM wiki maintainer for any git repo (Rust, MCP, MIT)
```

**Body:** use a condensed version of the HN Tone A draft. The
audience is broader so emphasize the user-facing wins (plug-and-
play, ~60s onboarding, multi-repo manifest) over the Rust-internal
work.

---

## Channel: Discord — Claude Code / Anthropic builders

**Channel:** #show-and-tell or equivalent.

```
hey 👋 shipped Coral v0.39.0 — Karpathy-style LLM wiki + MCP server
for any git repo

plug-and-play in Claude Code:
  /plugin marketplace add agustincbajo/Coral
  /plugin install coral@coral
  /reload-plugins

then type anything ("set up coral", "where do I start") — five auto-
invoked skills + SessionStart hook handle routing. No slash commands
to memorize.

a few non-obvious bits:
- bi-temporal wiki history (scrub between any two commits AND any
  two doc states in the WebUI)
- cost-confirmed bootstrap (`coral bootstrap --estimate` before
  any LLM call, `--max-cost` enforces a ceiling)
- 4-path provider wizard (Anthropic / Gemini / Ollama / claude CLI)
  for users who don't have claude CLI yet

source: github.com/agustincbajo/Coral
PRD: docs/PRD-v0.34-onboarding.md in the repo

feedback super welcome — opt-in `coral feedback submit` emits
sanitized JSON you can paste into a GH discussion. AF-1 compliant
(zero phone home).
```

---

## Channel: LinkedIn (optional, only if maintainer uses it)

**Body:**

```
Shipped Coral v0.39.0 — a multi-repo developer's manifest for the
AI era.

The bet: large-language-model-assisted development needs a shared
substrate. Not a SaaS, not a "cloud platform" — a single Rust
binary that builds a queryable wiki from your git history, keeps
it bi-temporally synchronized with the code, and exposes it to
coding agents via Model Context Protocol.

Six multi-agent audit cycles. Five auto-invoked skills. Zero
telemetry. Sigstore SLSA provenance on every release.

What this changes for teams:
- New hires bootstrap a wiki of any repo in ~60 seconds.
- Architecture questions get answered without re-reading the code.
- Multi-repo manifest (`coral.toml`) means N services have one
  source-of-truth, not per-IDE drift.

MIT, source on GitHub. Roadmap, PRD, and 12 ADRs in the repo.

github.com/agustincbajo/Coral
```

---

## Image / GIF / screenshot inventory

Use these assets (when available) per channel. Last verified
2026-05-13.

| Asset | Status | Use cases |
|---|---|---|
| `docs/assets/ui-pages-en.png` | ✅ shipped | hero (M1) |
| `docs/assets/ui-graph-en.png` | ✅ shipped | force-directed graph hero |
| `docs/assets/ui-query-en.png` | ✅ shipped | LLM query playground |
| `docs/assets/ui-manifest-en.png` | ✅ shipped | multi-repo manifest |
| `docs/assets/ui-interfaces-en.png` | 🔴 missing (BACKLOG #1) | Interfaces view |
| `docs/assets/ui-drift-en.png` | 🔴 missing (BACKLOG #1) | bi-temporal drift |
| `docs/assets/ui-affected-en.png` | 🔴 missing (BACKLOG #1) | affected-by-change |
| `docs/assets/ui-tools-en.png` | 🔴 missing (BACKLOG #1) | tools view |
| `docs/assets/ui-guarantee-en.png` | 🔴 missing (BACKLOG #1) | guarantee view |
| `docs/getting-started.gif.placeholder` | 🟡 placeholder | 60-second flow |

The 5 missing M2/M3 PNGs and the GIF are the only blockers for the
visual channels (X/Twitter, LinkedIn, marketplace listing).

---

## Release-train summary (use as factual scaffolding)

For any draft that mentions specific releases, these are the
short-form changelog hooks:

- **v0.34.0** — M1 onboarding stack (5 skills, SessionStart hook,
  4-path provider wizard, `coral self-upgrade`, cost-confirmed
  bootstrap, `--with-claude-config`).
- **v0.34.1** — same-day patch: GITHUB_TOKEN auth on self-upgrade,
  Windows hook latency rewrite (mean 644ms → 367ms), Ollama config
  bridge, post-release-smoke `workflow_run` trigger.
- **v0.35.0** — MSRV 1.85→1.89 (ADR-0011), Phase C public-mod
  surface reduction (17/33 modules to pub(crate), 51% reduction).
- **v0.36.0** — mimalloc cross-platform baseline (+29.7-42.7%
  measured throughput), SPA sibling-gen hardening
  (`Vary: Accept-Encoding` always-on, raw bundles dropped for
  files ≥100 KiB).
- **v0.37.0** — clippy panic-risk job promoted to hard gate at 0
  warnings (from 45 in v0.36, 104 baseline).
- **v0.38.0** — breaking: `coral wiki serve` removed (3-version
  deprecation window honoured). CHANGELOG.md backfilled (Keep-a-
  Changelog), opt-in `coral feedback submit`, MSRV badge sync.
- **v0.39.0** — bincode→postcard migration clears RUSTSEC-2025-
  0141; `cargo deny check advisories` zero-suppressed. Coverage
  CI floor 55→60% (measured workspace at 83.81%).

---

## Anti-patterns — what NOT to put in a GTM draft

- ❌ Performance numbers without a baseline (e.g. "10x faster" —
  faster than what? specify the workload + the baseline).
- ❌ "Production-ready" claims (Coral is pre-1.0; the BACKLOG
  documents real open items honestly).
- ❌ Comparisons to specific commercial products by name unless
  the comparison is rigorously fact-checked. Stick to category
  comparisons ("vs SaaS RAG", "vs IDE-specific config").
- ❌ Claims about "AI-era" / "next-gen" / "revolutionary" without
  a concrete demo. Show, don't adjective.
- ❌ Numbers from the PRD without checking they still hold (the
  PRD has aspirational numbers; the README and BACKLOG have the
  current measured numbers).
- ❌ Hyperbolic security claims. We have Sigstore + cargo deny +
  panic-risk hard gate — say those, don't say "audited" or
  "enterprise-ready".

---

## Decision: which channel first?

**If the maintainer wants signal:** HN Show-HN first. The
Anthropic curated marketplace submission second (gated on
`docs/SUBMISSION-DRAFT.md`). Then ride whichever the HN audience
asks about — Rust internals, MCP integration, bi-temporal wiki
mechanics.

**If the maintainer wants community:** Discord (#show-and-tell)
first to friendly audience that already runs Claude Code; gather
direct feedback for 1-2 weeks; refine; THEN HN.

**If the maintainer wants nothing-public-yet:** keep this file as
a private prep doc. None of these drafts is committed to a
specific date or channel — the file's purpose is to remove "I
don't have draft text" as a blocker on a slow-news day.
