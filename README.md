# Coral

> Karpathy-style LLM Wiki maintainer for Git repos.

[![CI](https://github.com/agustincbajo/Coral/actions/workflows/ci.yml/badge.svg)](https://github.com/agustincbajo/Coral/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/agustincbajo/Coral?display_name=tag)](https://github.com/agustincbajo/Coral/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Tests](https://img.shields.io/badge/tests-342%20passing-brightgreen)](#testing--ci)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](rust-toolchain.toml)

Coral compiles your codebase into an interconnected Markdown wiki that an LLM (Claude) maintains as you push code. Each merge updates the wiki incrementally; nightly lint catches contradictions; weekly consolidation prunes redundant pages.

> *"The IDE is Claude Code. The programmer is you + the LLM. The wiki is the living memory of your codebase."*

---

## Table of contents

- [Why Coral](#why-coral)
- [Install](#install)
- [Quickstart (5 minutes)](#quickstart-5-minutes)
- [Subcommands at a glance](#subcommands-at-a-glance)
- [The wiki schema](#the-wiki-schema)
- [CI integration](#ci-integration)
- [Multi-provider LLM support](#multi-provider-llm-support)
- [Auth setup](#auth-setup)
- [Configuration](#configuration)
- [Architecture](#architecture)
- [Performance](#performance)
- [Testing & CI](#testing--ci)
- [How Coral itself was built](#how-coral-itself-was-built)
- [Roadmap](#roadmap)
- [Contributing](#contributing)
- [References & related work](#references--related-work)
- [License](#license)

---

## Why Coral

The naive approach to giving an LLM context about your repo is a giant `AGENTS.md` file. It grows out of control, eats your context window, drifts out of sync with the code, and provides zero auditability.

Coral implements [Andrej Karpathy's LLM Wiki pattern](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f) instead: a constellation of small (<300 line) Markdown pages, each tagged with frontmatter (`slug`, `type`, `confidence`, `sources`, `backlinks`), curated by an LLM bibliotecario subagent under a strict SCHEMA.

| Aspect | Naive `AGENTS.md` | Coral wiki |
|---|---|---|
| Storage | Single growing file | Constellation of small Markdown pages |
| State | Implicit, drifts | Explicit, with `last_updated_commit` per page |
| Lock-in | None | None — plain Markdown in Git |
| Auditability | Opaque | Each page cites verifiable `sources` |
| Maintenance | Manual | Incremental ingest on every push |
| Search | grep | TF-IDF (default) + opt-in Voyage embeddings (`--engine embeddings`) |

**What you get out of the box:**

- A `coral` CLI binary (~2.8 MB, statically linked) with 12 subcommands.
- 4 Claude Code subagents pre-configured (bibliotecario, linter, consolidator, onboarder).
- 4 versioned prompt templates with `{{var}}` substitution.
- 5 deterministic structural lint checks + 1 LLM-driven semantic check.
- 3 composite GitHub Actions for CI: `ingest`, `lint`, `consolidate`.
- Optional Hermes quality gate: a second LLM independently validates wiki PRs.
- Multi-provider runner (Claude default, Gemini optional, MockRunner for tests).
- TF-IDF search by default, plus opt-in semantic search via Voyage AI (`coral search --engine embeddings`).
- 4 export formats (Markdown bundle, JSON, Notion API bodies, JSONL for fine-tunes).

---

## Install

### Prerequisites

- **Rust** 1.85+ (stable). Install via [rustup](https://rustup.rs/).
- **Git** 2.30+.
- **Claude Code CLI** (`claude` in `PATH`). Required only for LLM-backed subcommands. See [claude.com/code](https://claude.com/code).

### From a tagged release (recommended)

```bash
cargo install --locked --git https://github.com/agustincbajo/Coral --tag v0.1.0 coral-cli
```

### From `main` (latest unreleased)

```bash
cargo install --locked --git https://github.com/agustincbajo/Coral coral-cli
```

### From source

```bash
git clone https://github.com/agustincbajo/Coral && cd Coral
cargo install --locked --path crates/coral-cli
coral --version    # → coral 0.1.0
```

See [docs/INSTALL.md](docs/INSTALL.md) for the full setup including CI tokens and Hermes wiring.

---

## Quickstart (5 minutes)

```bash
# 0. Inside any Git repo:
cd ~/your-project

# 1. Initialize the wiki (creates .wiki/{SCHEMA, index, log, type subdirs}).
coral init

# 2. Ask the wiki a question — uses your local Claude Code session.
coral query "How is an order created?"
# → Streams the answer, citing pages as [[wikilinks]].

# 3. Lint structural issues (broken links, orphans, low confidence, ...).
coral lint --structural
# → Markdown report, exit 1 if any critical issue.

# 4. Look at wiki health stats.
coral stats
# → Total pages, by type, by status, confidence avg/min/max, orphan candidates.

# 5. Search the wiki — TF-IDF by default, or semantic embeddings (opt-in).
coral search "outbox dispatcher"
# Semantic via Voyage (requires VOYAGE_API_KEY):
# coral search "how does retry work" --engine embeddings
# → Top-N pages with scores + snippets.

# 6. Export the wiki — Markdown bundle, raw JSON, Notion API bodies, or JSONL.
coral export --format markdown-bundle --out wiki.md
coral export --format notion-json --out notion-bodies.json
# JSONL with LLM-generated Q/A pairs (3-5 per page) for fine-tuning:
coral export --format jsonl --qa --out wiki-qa.jsonl

# 7. Pull subagent / prompt updates from a tagged Coral release.
coral sync --version v0.1.0
# Or remote (any tag):
coral sync --remote --version v0.2.0
```

The full reference is in [docs/USAGE.md](docs/USAGE.md).

---

## Subcommands at a glance

| Command | What it does | Needs LLM? |
|---|---|---|
| `coral init` | Scaffold `.wiki/` with SCHEMA, index, log, and 9 type subdirs. Idempotent. | No |
| `coral bootstrap [--apply]` | First-time wiki compilation from `HEAD`. `--dry-run` (default) prints plan; `--apply` writes pages. | Yes |
| `coral ingest [--from SHA] [--apply]` | Incremental update from `last_commit`. Same dry-run / apply semantics. | Yes |
| `coral query <q>` | Streamed answer using the wiki as context. Cites slugs as `[[wikilinks]]`. | Yes |
| `coral lint [--structural\|--semantic\|--all]` | Structural (deterministic) + semantic (LLM) lint. Exit 1 on critical. | Optional |
| `coral consolidate` | Suggest merges, retirements, splits. Output YAML — caller decides. | Yes |
| `coral stats [--format markdown\|json]` | Health dashboard. JSON validates against `docs/schemas/stats.schema.json`. | No |
| `coral search <q> [--engine tfidf\|embeddings] [--limit N]` | TF-IDF (default) or Voyage embeddings (`--engine embeddings`, opt-in). Top-N pages with score + snippet. | No (TF-IDF) / Voyage key (embeddings) |
| `coral sync [--version V] [--remote] [--pin K=V] [--unpin K]` | Lay subagents/prompts/workflow into `<cwd>/template/`. Per-file pinning via `.coral-pins.toml`. | No |
| `coral export --format <fmt> [--out FILE] [--qa]` | Export to `markdown-bundle`, `json`, `notion-json`, or `jsonl`. With `--qa`, jsonl emits LLM-generated Q/A pairs. | Optional |
| `coral notion-push [--type T]` | Push pages to a Notion database via curl (reads `NOTION_TOKEN` + `CORAL_NOTION_DB`). | No |
| `coral onboard --profile <P>` | Tailored 5–10 page reading path for a reader profile. | Yes |
| `coral prompts list` | Show which prompts are local-overridden, embedded, or fallback. | No |

---

## The wiki schema

Every page in `.wiki/` has YAML frontmatter:

```yaml
---
slug: order-creation
type: module                 # module | concept | entity | flow | decision | synthesis | operation | source | gap | reference
last_updated_commit: abc123  # 40-char git sha
confidence: 0.85             # 0.0..1.0, honest self-assessment vs HEAD
sources:                     # list of paths or URLs that back the claims
  - src/features/create_order/
  - docs/adr/0007-saga-orchestration.md
backlinks:                   # explicit inbound references (the lint also walks bodies)
  - idempotency
  - outbox-pattern
status: draft                # draft | reviewed | verified | stale | archived | reference
---

# Order creation

The body is plain Markdown with [[wikilinks]] to other pages…
```

### Page types and when to create them

| Type | Create when |
|---|---|
| `modules/` | New vertical slice in `src/features/` (Rust) or equivalent. |
| `concepts/` | A reusable abstraction appears in ≥ 2 modules. |
| `entities/` | New domain type with non-trivial invariants. |
| `flows/` | Multi-step request flow that crosses modules. |
| `decisions/` | New ADR — link-only entry in `decisions/index.md`. |
| `synthesis/` | Decision with explicit tradeoffs worth narrating. |
| `operations/` | Runbook for on-call (deploy, restore, incident triage). |
| `sources/` | RFC, paper, or external doc referenced by code or ADRs. |
| `gaps/` | Detected by lint — pages that *should* exist but don't. |

### Rules of gold

1. **HEAD wins.** If the wiki contradicts the code, mark the page `status: stale`.
2. **A new page links to ≥ 2 existing pages and is linked by ≥ 1.** Otherwise it's an orphan; the lint warns.
3. **Never delete pages**; archive by moving to `.wiki/_archive/`.
4. **Decisions are link-only.** `decisions/index.md` references `docs/adr/*` paths; never duplicates content.
5. **Confidence ≥ 0.7 requires sources.** Lint enforces this.
6. **`log.md` is append-only.** Never edit; never reorder.

The full SCHEMA is at [`template/schema/SCHEMA.base.md`](template/schema/SCHEMA.base.md). Consumer repos extend it locally — `coral sync` copies it once and never overwrites it.

---

## CI integration

Coral ships 3 reusable composite GitHub Actions consumable by any repo with this single line:

```yaml
- uses: agustincbajo/Coral/.github/actions/ingest@v0.1.0
  with:
    claude_code_oauth_token: ${{ secrets.CLAUDE_CODE_OAUTH_TOKEN }}
```

### The three actions

| Action | Trigger | What it does |
|---|---|---|
| `ingest` | `push` to `main` | Runs `/wiki-ingest` from `last_commit` to `HEAD`. Opens PR `wiki/auto-ingest`. |
| `lint` | nightly schedule | `coral lint --all` (structural + semantic). Posts findings as a PR comment / issue. |
| `consolidate` | weekly schedule | Suggests merges/retirements/splits. Opens PR `wiki/consolidate`. |

### Embeddings cache (opt-in, for `--engine embeddings` workflows)

If your CI runs `coral search --engine embeddings`, drop the cache action **before** the search step so each run only re-embeds pages whose content changed:

```yaml
- uses: actions/checkout@v4
- uses: agustincbajo/Coral/.github/actions/embeddings-cache@v0.4.0
  with:
    wiki_root: .wiki    # default
- run: coral search --engine embeddings "outbox dispatcher" --limit 5
  env:
    VOYAGE_API_KEY: ${{ secrets.VOYAGE_API_KEY }}
```

Cache key strategy: `<prefix>-<ref>-<hash of .wiki/**/*.md>`. Falls back to the most recent run on the same branch when the exact hash misses, so a single page edit reuses ~all vectors. Cross-branch reuse is intentionally NOT done — branches often diverge and a stale vector silently ranks wrong.

### Hermes quality gate (opt-in)

```yaml
- uses: agustincbajo/Coral/.github/actions/validate@v0.1.0
  with:
    claude_code_oauth_token: ${{ secrets.CLAUDE_CODE_OAUTH_TOKEN }}
    pr_number: ${{ github.event.pull_request.number }}
```

A separate LLM (default `opus`) independently validates that every claim in changed `.wiki/**/*.md` files is backed by the cited `sources:`. Posts `REQUEST CHANGES` if any rejection. Skip threshold via `min_pages_to_validate` (default 5).

### OAuth token setup (once)

See the dedicated [Auth setup](#auth-setup) section below — it covers local shells, CI, and the gotcha when running `coral` from inside Claude Code.

---

## Multi-provider LLM support

```bash
coral query "..." --provider claude                              # default
coral query "..." --provider gemini                              # uses GeminiRunner
coral query "..." --provider local --model /m/llama-3-8b.gguf    # uses LocalRunner (llama.cpp)
CORAL_PROVIDER=gemini coral lint --semantic
```

`coral-runner` exposes a `Runner` trait with four implementations:

- **`ClaudeRunner`** — shells out to `claude --print` (production default).
- **`GeminiRunner`** — invokes `gemini -p <prompt> -m <model>` (system prompt prepended). Useful for cheap nightly lint.
- **`LocalRunner`** — invokes `llama-cli -p <prompt> -m <model.gguf> --no-display-prompt`. Truly offline; pair with `--auto-fix` for cheap iterative lint cleanup.
- **`MockRunner`** — FIFO scripted responses for tests; captures prompts for assertions.

Future runners (OpenAI Responses, vLLM-served local model, etc.) are one new file in `crates/coral-runner/src/`.

The same shape applies to embeddings: `coral-runner` exposes an `EmbeddingsProvider` trait with `VoyageProvider` (production) and `MockEmbeddingsProvider` (tests). Other providers (OpenAI text-embedding-3, Anthropic when shipped) land as one new struct.

---

## Auth setup

Coral's LLM-driven subcommands (`bootstrap`, `ingest`, `query`, `lint --semantic`, `consolidate`, `onboard`, `export --qa`) shell out to the `claude` CLI in `--print` mode. The `claude` subprocess needs its own auth — Coral does **not** pass anything through, and the parent shell's `ANTHROPIC_API_KEY` may not be valid in the subprocess (see "Running from inside Claude Code" below).

### Local shell (recommended)

```bash
claude setup-token   # one-time; generates an OAuth token tied to your Anthropic subscription
```

`claude` stores the token in its own keychain entry. Once set, every `coral` invocation in any shell uses it. To verify:

```bash
echo "ping" | claude --print
# → should print a short reply, exit 0
```

If you see `Failed to authenticate. API Error: 401 …` here, `coral`'s LLM commands will fail with `RunnerError::AuthFailed` (since v0.3.2). Fix it at this layer first.

### CI (GitHub Actions)

```bash
claude setup-token   # locally
# Paste the token at:
# GitHub → Org → Settings → Secrets → CLAUDE_CODE_OAUTH_TOKEN
```

All consumer repos in the org inherit the secret via the composite actions. No `ANTHROPIC_API_KEY` required.

### Running `coral` from inside Claude Code (gotcha)

If you're invoking `coral` from a Claude Code session, the parent process exports `ANTHROPIC_API_KEY` and `ANTHROPIC_BASE_URL` pointing at the host-managed proxy. **The `claude --print` subprocess cannot use those credentials** — it gets 401. Two workarounds:

- **Run `claude setup-token` once** in a normal shell; the resulting OAuth token is independent of Claude Code's env vars and works from any subprocess.
- **Or export a real `ANTHROPIC_API_KEY`** (your own, not the proxy's) in the shell that runs `coral`.

Since v0.3.2, `coral` detects this case and prints an actionable hint (`Run \`claude setup-token\` or export ANTHROPIC_API_KEY in this shell.`) instead of a silent `exit 1`.

### Embeddings provider auth

`coral search --engine embeddings` needs `VOYAGE_API_KEY` set in the shell. Without it, the command exits with a clear error pointing at the env var. The default `--engine tfidf` path needs no API key and works offline.

---

## Configuration

| File / env var | Purpose |
|---|---|
| `.wiki/SCHEMA.md` | Local SCHEMA — extends the base shipped with Coral. Never overwritten by `coral sync`. |
| `.wiki/index.md` | Catalog + `last_commit` anchor. Maintained automatically. |
| `.wiki/log.md` | Append-only operation log. |
| `.coral-pins.toml` | Per-file template version pinning. |
| `.coral-template-version` | Legacy single-line marker (still written for bcompat). |
| `prompts/<name>.md` | Local override of an embedded prompt template. |
| `CORAL_PROVIDER` | LLM provider override (`claude` \| `gemini`). |
| `CLAUDE_CODE_OAUTH_TOKEN` | OAuth token for Claude Code (required in CI). |
| `RUST_LOG=coral=debug` | Verbose logging. |

### Per-file pinning example

```toml
# .coral-pins.toml
default = "v0.1.0"

[pins]
"agents/wiki-bibliotecario" = "v0.2.0"
"prompts/ingest" = "v0.2.0"
```

`coral sync` reads this file and resolves the version per file. Update via:

```bash
coral sync --pin "agents/wiki-bibliotecario=v0.2.0"
coral sync --unpin "agents/wiki-bibliotecario"
```

### Prompt override priority

```
<cwd>/prompts/<name>.md   ← highest (local override, survives upgrades)
template/prompts/<name>.md ← embedded in the binary
hardcoded fallback const   ← in code; only if both above missing
```

`coral prompts list` shows which one is in effect for each known prompt name.

---

## Architecture

### Workspace layout

```
coral/
├── crates/
│   ├── coral-cli/      ← bin: `coral`. Clap dispatcher.
│   ├── coral-core/     ← types + parsing (frontmatter, wikilinks, page,
│   │                     index, log, gitdiff, walk, search). Pure Rust,
│   │                     zero LLM coupling.
│   ├── coral-lint/     ← LintReport + 5 structural checks + semantic via runner.
│   ├── coral-runner/   ← Runner trait + ClaudeRunner + GeminiRunner +
│   │                     MockRunner + PromptBuilder.
│   └── coral-stats/    ← StatsReport + JsonSchema + Markdown / JSON renderers.
│
├── template/           ← embedded via include_dir!; surfaced by `coral sync`.
│   ├── agents/         ← 4 Claude Code subagents.
│   ├── commands/       ← 4 slash commands.
│   ├── prompts/        ← 4 versioned prompt templates with {{var}} placeholders.
│   ├── schema/SCHEMA.base.md ← base contract for the bibliotecario.
│   └── workflows/wiki-maintenance.yml ← 3-job CI template.
│
├── .github/
│   ├── actions/{ingest,lint,consolidate,validate}/action.yml
│   └── workflows/{ci.yml,release.yml}
│
├── docs/
│   ├── INSTALL.md, USAGE.md, ARCHITECTURE.md, PERF.md
│   ├── adr/0001..0007*.md          ← architecture decisions
│   └── schemas/stats.schema.json   ← JSON schema for `coral stats --format json`
│
└── .wiki/                          ← Coral uses Coral; self-hosted dogfooding
```

### Data flow

```
        ┌─────────────────────────────────────────────────┐
        │                  Your Git repo                   │
        │   src/  docs/  Cargo.toml  …                     │
        │   .wiki/  ←─── SCHEMA.md, index.md, log.md       │
        │           ←─── modules/, concepts/, entities/,   │
        │                flows/, decisions/, synthesis/,   │
        │                operations/, sources/, gaps/      │
        └────────────────────┬─────────────────────────────┘
                             │  coral CLI
                             ▼
        ┌─────────────────────────────────────────────────┐
        │  coral-cli                                       │
        │                                                  │
        │  init/sync/lint --structural/stats/search/export │ ← no LLM
        │                                                  │
        │  bootstrap/ingest/query/consolidate/onboard +    │ ← LLM via Runner
        │  lint --semantic                                 │
        │                       │                          │
        │                       ▼                          │
        │              ┌──────────────┐                    │
        │              │  Runner      │◄──── MockRunner    │
        │              │  trait       │      (tests)       │
        │              └──────┬───────┘                    │
        │                     │                            │
        │            ┌────────┴───────┐                    │
        │            ▼                ▼                    │
        │     ClaudeRunner     GeminiRunner                │
        │     (prod default)   (--provider gemini)         │
        └─────────────────────────────────────────────────┘
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the deep dive and the 7 ADRs in [docs/adr/](docs/adr/) for design rationale.

---

## Performance

Coral aims for sub-100 ms cold-start on the structural commands.

| Operation | Wiki size | Time (debug) | Time (release) |
|---|---|---|---|
| `coral init` | empty | ~30 ms | ~10 ms |
| `coral lint --structural` | 14 pages | ~80 ms | ~25 ms |
| `coral stats` | 14 pages | ~70 ms | ~20 ms |
| `coral sync` | embedded template | ~40 ms | ~15 ms |
| `coral search` | 14 pages | <10 ms | <5 ms |

Release profile: `lto = "thin"`, `codegen-units = 1`, `strip = true`, `panic = "abort"`. Binary 2.8 MB stripped.

Methodology, hot paths, and profiling tips in [docs/PERF.md](docs/PERF.md).

---

## Testing & CI

```bash
cargo test --workspace                        # 342 tests passing
cargo test --workspace -- --ignored           # 8 ignored (real-claude / real-gemini /
                                              # real-llama / real-voyage / real-openai
                                              # / real-git smokes)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo bench --workspace -- --test             # benchmarks compile + run once
```

### Test breakdown (v0.6.0)

| Crate / target | Tests |
|---|---|
| `coral-core` (lib + benches) | 94 + 2 ignored (real-git smoke) |
| `coral-lint` (lib + benches) | 47 |
| `coral-runner` | 47 + 5 ignored (real-claude / real-gemini / real-llama / real-voyage / real-openai smokes) |
| `coral-stats` | 14 |
| `coral-cli` (unit) | 96 + 2 ignored |
| `coral-cli` (integration: cli_smoke) | 31 + 1 ignored |
| `coral-cli` (e2e: full_lifecycle, multi_repo, query_cycle) | 9 |
| `coral-cli` (template_validation) | 14 |
| **Total** | **342 + 8 ignored** |

### CI pipeline

- **`ci.yml`** runs on every push to `main` and PR: `fmt`, `clippy`, `test`, `audit` (cargo-audit, soft-fail), `deny` (cargo-deny, hard gate on licenses + duplicate-versions per [`deny.toml`](deny.toml)).
- **`release.yml`** runs on tag push (`v*.*.*`): builds Linux x86_64 + macOS x86_64+aarch64, strips binaries, uploads `.tar.gz` + `.sha256` to a GitHub Release.

---

## How Coral itself was built

Coral was built using a **3-role multi-agent loop** — and it's documented in [ADR 0004](docs/adr/0004-multi-agent-development-flow.md).

```
Orchestrator: define spec → Coder: implement → Tester: verify
                                                    │
                                ┌── pass ──► Orchestrator commits + advances
                                │
                                └── fail ──► Orchestrator forwards log to Coder → loop
```

- **Orchestrator** (Claude in the foreground) defines per-phase specs, manages the coder ↔ tester loop, handles commits and pushes. **Writes zero production code.**
- **Coder agent** (`general-purpose` subagent) receives a spec, implements code, runs `cargo build` to confirm it compiles. **Does not approve.**
- **Tester agent** runs `cargo test/clippy/fmt --check`. **Does not edit.** Reports pass/fail + log of failures.

Coral v0.1.0 shipped through 9 sequential phases (A–I), each landing as one atomic, green commit. Coral v0.2.0 closed 14 of 15 issues across 6 batches the same way: every commit was green from the first attempt for 5/6 batches, with one batch needing a single mechanical fmt + clippy fix.

---

## Roadmap

### v0.1.0 — initial release (April 2026) ✅

- Cargo workspace with 5 crates.
- 10 subcommands declared (5 LLM-using, 5 deterministic).
- Embedded skill bundle: subagents, prompts, SCHEMA, workflow.
- 3 composite GH actions.
- 150 tests + 3 ignored.

### v0.2.0 — current (closed 14/15 issues) ✅

| # | Title | Status |
|---|---|---|
| #1 | bootstrap/ingest write pages | ✅ — `--apply` flag |
| #2 | walk skips top-level system files | ✅ |
| #3 | CHANGELOG + cargo-release | ✅ |
| #4 | Streaming `coral query` | ✅ |
| #5 | `coral search` (TF-IDF) | ✅ |
| #6 | Hermes quality gate | ✅ |
| #7 | Local prompt overrides | ✅ |
| #8 | GeminiRunner (multi-provider) | ✅ |
| #9 | Notion sync (via `coral export --format notion-json`) | ✅ |
| #10 | `coral sync --remote` | ✅ |
| #11 | Per-file version pinning (`.coral-pins.toml`) | ✅ |
| #12 | `orchestra-ingest` consumer repo | 🚫 deferred (separate-repo follow-up) |
| #13 | Fine-tune dataset (`coral export --format jsonl`) | ✅ |
| #14 | Perf docs + release-profile tweaks | ✅ |
| #15 | Stats coverage + JSON schema | ✅ |

### v0.3.x — patches ✅

- v0.3.0: mtime-cached frontmatter parsing + LLM-driven Q/A pairs.
- v0.3.1: embeddings-backed search via Voyage AI.
- v0.3.2: 3 dogfooding fixes (UTF-8 search panic, runner auth UX, CWD_LOCK race).

### v0.4.0 — multi-provider runners ✅

- `EmbeddingsProvider` trait + Voyage / OpenAI / Mock impls.
- Real `GeminiRunner` (no longer wraps Claude).
- `LocalRunner` (llama.cpp / `llama-cli`).
- `coral search --embeddings-provider <voyage|openai>`.
- README "Auth setup" section.
- `coral query` telemetry + `notion-push` dry-run-default.

### v0.5.0 — apply-flow + streaming + docs ✅

- `coral validate-pin`.
- `coral lint --staged` + `--auto-fix [--apply]`.
- `embeddings-cache` composite GH action.
- `coral diff <slugA> <slugB>` (structural).
- `coral export --format html` (single-file static site).
- `coral consolidate --apply` (retire path).
- `coral onboard --apply` (persists path as wiki page).
- Streaming runner unification (Gemini + Local now token-by-token).

### v0.6.0 — quality + apply-flow extension + CI hardening ✅

- 4 new structural lint checks (`CommitNotInGit`, `SourceNotFound`, `ArchivedPageLinked`, `UnknownExtraField`).
- `coral diff --semantic` (LLM-driven contradictions + overlap).
- `coral consolidate --apply` extended to handle merges + splits.
- `criterion` benchmarks for 5 hot paths.
- `cargo-audit` + `cargo-deny` CI jobs.
- ADR 0008 (multi-provider runner+embeddings) + ADR 0009 (auto-fix scope).
- Parallelized embeddings batching across rayon thread pool.

### Tracked but blocked

- **Self-hosted dogfooding** of `.wiki/` — needs `claude setup-token` from the maintainer (the parent's `ANTHROPIC_API_KEY` doesn't reach the `claude --print` subprocess when Coral runs from inside Claude Code).
- **`AnthropicEmbeddingsProvider`** — gated on Anthropic publishing the embeddings API.
- **`sqlite-vec` migration** — explicitly deferred in [ADR 0006](docs/adr/0006-local-semantic-search-storage.md) until a wiki crosses ~5k pages.
- **`orchestra-ingest` reference consumer repo** — separate-repo follow-up (issue #12).

---

## Contributing

### Development workflow

```bash
git clone https://github.com/agustincbajo/Coral && cd Coral
cargo build --workspace
cargo test --workspace
```

Before pushing:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

### Conventions

- **Edition 2024**, `rust-version = 1.85`. Pinned in `rust-toolchain.toml`.
- **Workspace deps** in the root `Cargo.toml` `[workspace.dependencies]`. Crates use `workspace = true`.
- **No `unwrap()` / `panic!` in production code.** OK in tests.
- **No `unsafe`.** If you think you need it, file an issue first.
- **Errors via `thiserror` (libraries) or `anyhow` (binary).**
- **Tests inline** with `#[cfg(test)] mod tests`. Integration tests in `tests/` directories.
- **Commit messages** follow [Conventional Commits](https://www.conventionalcommits.org/). Footer: `Closes #N` to auto-close issues.

### Releasing

See [`.wiki/operations/release-checklist.md`](.wiki/operations/release-checklist.md). Short version:

```bash
cargo release X.Y.Z   # uses release.toml; rotates CHANGELOG, bumps versions, tags, pushes
```

GitHub Actions handle the binary builds + Release creation.

### Reporting bugs

Open an issue with:

- Coral version (`coral --version`).
- Rust version (`rustc --version`).
- OS + arch.
- Minimal reproduction (a tempdir + a sequence of commands).

---

## References & related work

- **Karpathy's LLM Wiki gist** (3 Apr 2026) — [karpathy/442a6bf555914893e9891c11519de94f](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f). The canonical reference.
- **Yysun, *Bringing the LLM Wiki Idea to a Codebase*** (DEV.to, 12 Apr 2026) — translation to a software repo, with a `git-wiki` skill.
- **Rohit Gangupantulu, *LLM Wiki v2*** ([gist](https://gist.github.com/rohitg00/2067ab416f7bbe447c1977edaaa681e2)) — extension with hooks, lifecycle, retention decay.
- **`cablate/llm-atomic-wiki`** — atom layer + two-layer lint + topic branches.
- **`NicholasSpisak/second-brain`** — wizard + 4 skills + 3 slash commands (the base Pau Berenguer's video uses).
- **`Astro-Han/karpathy-llm-wiki`** — packaged Agent Skill compatible with Claude Code, Codex, Cursor.
- **`Pratiyush/llm-wiki`** — full implementation with 16 lint rules + 5-state lifecycle + Auto-Dream consolidation.
- **DAIR.AI Academy** — pedagogical analysis with the 4-phase interactive diagram.
- **VentureBeat (Apr 2026)** — *Karpathy shares 'LLM Knowledge Base' architecture that bypasses RAG with an evolving markdown library*.
- **Pau Berenguer (10 Apr 2026)** — *Claude Code Will Never Forget Anything Again* — the consumer-side Obsidian video that spawned the broader pattern.

---

## License

MIT © 2026 Agustín Bajo. See [LICENSE](LICENSE).
