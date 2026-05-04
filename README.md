# Coral

> **The project manifest for AI-era development.** Multi-repo wiki + dev environments + functional testing + Model Context Protocol server, in a single Rust binary.

[![CI](https://github.com/agustincbajo/Coral/actions/workflows/ci.yml/badge.svg)](https://github.com/agustincbajo/Coral/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/agustincbajo/Coral?display_name=tag)](https://github.com/agustincbajo/Coral/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Tests](https://img.shields.io/badge/tests-700%2B%20passing-brightgreen)](#testing--ci)
[![Codecov](https://codecov.io/gh/agustincbajo/Coral/branch/main/graph/badge.svg)](https://codecov.io/gh/agustincbajo/Coral)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](rust-toolchain.toml)
[![MCP](https://img.shields.io/badge/MCP-2025--11--25-blue?logo=anthropic)](https://modelcontextprotocol.io/)

Coral started as a [Karpathy-style LLM Wiki](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f) maintainer for a single repo. **As of v0.19** it's a full developer-experience platform for microservice-shaped projects: declare your repos in a `coral.toml`, bring up a multi-service environment, run functional tests, and expose the whole thing to coding agents (Claude Code, Cursor, Continue, Cline, Goose, Codex, Copilot) via Model Context Protocol — all from one binary, all open source, all locally runnable.

> *"The IDE is Claude Code. The programmer is you + the LLM. The wiki is the living memory of your codebase. Coral is the manifest that makes both intelligible across N repos."*

---

## Table of contents

- [What you get](#what-you-get)
- [Why Coral](#why-coral)
- [Install](#install)
- [Quickstart — single-repo](#quickstart--single-repo-2-minutes)
- [Quickstart — multi-repo](#quickstart--multi-repo-5-minutes)
- [Quickstart — environments + tests](#quickstart--environments--tests)
- [Quickstart — MCP server for coding agents](#quickstart--mcp-server-for-coding-agents)
- [Subcommand reference](#subcommand-reference)
- [The wiki schema](#the-wiki-schema)
- [The `coral.toml` manifest](#the-coraltoml-manifest)
- [The `coral.lock` lockfile](#the-corallock-lockfile)
- [Test schema (`.coral/tests/*.{yaml,hurl}`)](#test-schema-coraltestsyamlhurl)
- [Backward compatibility](#backward-compatibility)
- [CI integration](#ci-integration)
- [Multi-provider LLM support](#multi-provider-llm-support)
- [Auth setup](#auth-setup)
- [Configuration](#configuration)
- [Architecture](#architecture)
- [Performance](#performance)
- [Testing & CI](#testing--ci)
- [Troubleshooting](#troubleshooting)
- [Roadmap](#roadmap)
- [How Coral itself was built](#how-coral-itself-was-built)
- [Contributing](#contributing)
- [References & related work](#references--related-work)
- [License](#license)

---

## What you get

A single `coral` binary (~6.3 MB stripped, statically linked, MSRV 1.85) with **36 leaf subcommands** (28 top-level commands, four of which group sub-subcommands) across five layers:

| Layer | Commands | Since |
|---|---|---|
| **Wiki** | `init` `bootstrap` `ingest` `query` `lint` `consolidate` `stats` `sync` `onboard` `prompts` `search` `export` `notion-push` `validate-pin` `diff` `status` `history` | v0.1+ |
| **Multi-repo** | `project new/list/add/sync/doctor/lock/graph` | v0.16 |
| **Environments** | `up` `down` `env status/logs/exec` | v0.17 |
| **Functional testing** | `test` `test-discover` `verify` | v0.18 |
| **AI ecosystem** | `mcp serve` `export-agents` `context-build` | v0.19 |

Plus:

- **8 Rust crates** in a workspace: `coral-cli`, `coral-core`, `coral-env`, `coral-test`, `coral-mcp`, `coral-runner`, `coral-lint`, `coral-stats`.
- **5 LLM runner implementations** (`Claude`, `Gemini`, `Local` llama.cpp, `Http` OpenAI-compat, `Mock` for tests).
- **3 embeddings providers** (`Voyage`, `OpenAI`, `Mock`).
- **2 storage backends** (JSON default, SQLite via `CORAL_EMBEDDINGS_BACKEND=sqlite`).
- **9 structural lint checks** + 1 LLM-driven semantic check + auto-fix routing.
- **5 export formats** for the wiki (`markdown-bundle`, `json`, `notion-json`, `jsonl`, `html`).
- **5 export formats** for AI agent instructions (`agents-md`, `claude-md`, `cursor-rules`, `copilot`, `llms-txt`) — manifest-driven, NOT LLM-driven.
- **9 `TestKind` variants** (`Healthcheck`, `UserDefined`, `LlmGenerated`, `Contract`, `PropertyBased`, `Recorded`, `Event`, `Trace`, `E2eBrowser`).
- **6 MCP resources + 5 read-only tools (3 more behind `--allow-write-tools`) + 3 prompts** exposed via JSON-RPC 2.0 stdio.
- **End-to-end concurrency safety**: atomic writes (`tmp + rename`), cross-process `flock(2)` locking, race-free parallel `coral ingest`.
- **Backward-compat guarantee**: every v0.15 single-repo workflow keeps working — pinned by a dedicated `bc-regression` test job that runs on every PR.

---

## Why Coral

Three problems in one tool.

### 1. The naive `AGENTS.md` problem

Giving an LLM context about your repo by hand-writing one giant `AGENTS.md` file is fragile. It grows out of control, eats your context window, drifts out of sync with the code, and provides zero auditability. Recent context-engineering work — including [Anthropic's published guidance](https://www.anthropic.com/engineering/context-engineering) and broader empirical reports — has converged on **structured note-taking persisted across sessions** rather than monolithic context dumps; LLM-generated `AGENTS.md` files in particular have shown degraded agent task success vs. deterministic, manifest-driven templates.

**Coral wiki** is a constellation of small (<300 line) Markdown pages, each tagged with frontmatter (`slug`, `type`, `confidence`, `sources`, `backlinks`), curated by an LLM bibliotecario subagent under a strict SCHEMA.

| Aspect | Naive `AGENTS.md` | Coral wiki |
|---|---|---|
| Storage | Single growing file | Constellation of small Markdown pages |
| State | Implicit, drifts | Explicit, `last_updated_commit` per page |
| Lock-in | None | None — plain Markdown in Git |
| Auditability | Opaque | Each page cites verifiable `sources` |
| Maintenance | Manual | Incremental ingest on every push |
| Search | grep | TF-IDF default + Voyage embeddings opt-in |

### 2. The microservices problem

Most production codebases span N repos. Coding agents (Cursor, Claude Code, Continue, …) treat each repo in isolation; your developers spend hours wiring up the dev environment by hand each onboarding.

**Coral multi-repo** declares the project shape in a `coral.toml`: list every repo, declare `depends_on`, tag them, and Coral handles parallel git clone, aggregated wiki, and dependency-graph visualization. The lockfile (`coral.lock`) pins resolved SHAs for reproducibility — same role as `Cargo.lock` / `package-lock.json` / `MODULE.bazel.lock`.

### 3. The functional testing problem

Unit tests don't tell you if your microservices actually work together. End-to-end browser tests are slow and brittle. The middle layer — *integration tests against a running multi-service stack* — is where most teams have nothing.

**Coral test layer** sits in the [microservices honeycomb middle layer](https://martinfowler.com/articles/2021-test-shapes.html): healthchecks, user-defined YAML/Hurl smoke suites, OpenAPI-discovered cases (no LLM), with retry / captures / snapshot assertions, and JUnit XML output for CI.

**Coral mcp serve** then exposes the wiki + manifest + lockfile + test results to *any* MCP-speaking agent, so your AI workflows operate on the same structured ground truth your team operates on. Per the [MCP 2025-11-25 spec](https://modelcontextprotocol.io/specification/2025-11-25), pinned in `coral-mcp::PROTOCOL_VERSION`.

---

## Install

### Prerequisites

- **Rust** 1.85+ (stable). Install via [rustup](https://rustup.rs/).
- **Git** 2.30+.
- **`curl`** (universally available; used by the test runner for HTTP probes — no libcurl FFI dep).
- **Optional:** `docker compose` v2.22+ (for `coral up` / `coral down` / `coral env *` and `coral verify`). `podman compose` and `docker-compose` v1 are also detected.
- **Optional:** [Claude Code CLI](https://claude.com/code) (`claude` in `$PATH`) for LLM-backed subcommands.

### From a tagged release (recommended)

```bash
cargo install --locked --git https://github.com/agustincbajo/Coral --tag v0.19.7 coral-cli
```

### From `main` (latest)

```bash
cargo install --locked --git https://github.com/agustincbajo/Coral coral-cli
```

### From source (development)

```bash
git clone https://github.com/agustincbajo/Coral
cd Coral
cargo build --release
./target/release/coral --version
```

### Pre-built binaries

Each tagged release ships pre-built binaries for x86_64 Linux, x86_64 macOS, and aarch64 macOS (Apple Silicon) on the [Releases page](https://github.com/agustincbajo/Coral/releases). Download `coral-vX.Y.Z-<target>.tar.gz`, verify the SHA-256, extract the `coral` binary, place it on your `$PATH`.

```bash
curl -L -o coral.tar.gz https://github.com/agustincbajo/Coral/releases/download/v0.19.7/coral-v0.19.7-aarch64-apple-darwin.tar.gz
shasum -a 256 -c coral.tar.gz.sha256  # if you also downloaded the .sha256 sidecar
tar -xzf coral.tar.gz
sudo mv coral-v0.19.7-aarch64-apple-darwin/coral /usr/local/bin/
coral --version
```

#### macOS — first run is blocked by Gatekeeper

The pre-built macOS tarballs are **ad-hoc signed** (free; just enough to satisfy the Apple Silicon kernel exec check) but **not notarized** (notarization requires a $99/year Apple Developer account, which Coral doesn't have yet). On first launch, macOS shows:

> *"No se ha abierto coral. Apple no ha podido verificar que coral no contenga software malicioso..."*
> *"coral cannot be opened because Apple cannot check it for malicious software."*

This is expected — the binary is fine, Apple just hasn't been paid to vouch for it. Two ways to allow it:

**Terminal (one line):**

```bash
xattr -d com.apple.quarantine /usr/local/bin/coral
```

That removes the quarantine flag macOS pinned on the file when you downloaded it. After that, `coral --version` runs cleanly forever.

**GUI (System Settings):**

1. When the warning appears, click **Aceptar / Cancel** (do NOT click "Trasladar a Papelera / Move to Trash").
2. Open **System Settings → Privacy & Security**.
3. Scroll to the *Security* section — there's a "coral was blocked..." line with an **"Open Anyway / Abrir igualmente"** button.
4. Click it, confirm once more, and Coral opens. macOS remembers the exception.

Either of these is a one-time step per release. To skip it entirely, install via `cargo install --locked --git ...` instead — `cargo` builds the binary on your machine, so Gatekeeper has no quarantine flag to apply.

---

## Quickstart — single-repo (2 minutes)

The v0.15 workflow still works exactly as before — no `coral.toml` needed.

```bash
cd /path/to/your/repo
coral init                              # scaffold .wiki/
coral bootstrap --apply                 # first-time wiki compilation (LLM)
coral ingest --apply                    # incremental updates on subsequent pushes
coral query "how does authentication work?"
coral status                            # daily-use dashboard
```

Full reference: [docs/USAGE.md](docs/USAGE.md), [docs/TUTORIAL.md](docs/TUTORIAL.md).

---

## Quickstart — multi-repo (5 minutes)

```bash
mkdir orchestra && cd orchestra
coral project new orchestra              # creates coral.toml + coral.lock + .wiki/
coral project add api    --url git@github.com:acme/api.git    --tags service team:platform
coral project add shared --url git@github.com:acme/shared.git --tags library
coral project add worker --url git@github.com:acme/worker.git \
                         --tags service team:data \
                         --depends-on api shared
coral project sync                       # parallel git clone via rayon
coral project graph --format mermaid     # render dependency graph (renders inline in GitHub Markdown)
coral project doctor                     # drift / missing clones / stale lockfile entries
coral ingest --apply                     # ingest aggregated wiki across all 3 repos
coral query "how does worker talk to api"
```

A `coral.toml` looks like this:

```toml
apiVersion = "coral.dev/v1"

[project]
name = "orchestra"

[project.toolchain]
coral = "0.19.0"                         # pin so cross-team workflows are reproducible

[project.defaults]
ref           = "main"
remote        = "github"
path_template = "repos/{name}"

[remotes.github]
fetch = "git@github.com:acme/{name}.git"

[[repos]]
name = "api"
ref  = "release/v3"
tags = ["service", "team:platform"]

[[repos]]
name       = "worker"
remote     = "github"
tags       = ["service", "team:data"]
depends_on = ["api"]
```

The `[remotes.<name>]` template + `defaults.remote` pattern (borrowed from Google's [git-repo](https://gerrit.googlesource.com/git-repo/+/master/docs/manifest-format.md) tool) keeps the manifest concise even with 20+ repos in the same org.

---

## Quickstart — environments + tests

After `coral project new`, declare a `[[environments]]` block:

```toml
[[environments]]
name            = "dev"
backend         = "compose"              # compose | kind | tilt (only compose in v0.19)
mode            = "managed"              # managed: Coral generates docker-compose.yml; adopt: bring your own
compose_command = "auto"                 # auto-detects docker compose v2 / docker-compose v1 / podman compose
production      = false                  # set true to require --yes on `down`/`exec`/destructive ops

# Services hang off `[environments.services.<name>]` — note the
# parent table name is `environments` (NOT `environments.dev`)
# because `[[environments]]` already opened the dev block.

[environments.services.api]
kind       = "real"
repo       = "api"                       # references [[repos]].name
build      = { dockerfile = "Dockerfile", target = "dev" }
ports      = [3000]
depends_on = ["db"]

[environments.services.api.healthcheck]
kind = "http"
path = "/health"
expect_status = 200

[environments.services.db]
kind  = "real"
image = "postgres:16"
ports = [5432]

[environments.services.db.healthcheck]
kind = "tcp"
port = 5432
```

For multiple environments, repeat the `[[environments]]` block (each entry gets its own `name`); the `[environments.services.*]` tables apply to whichever array entry is currently open.

Then bring it up and run tests:

```bash
coral up --env dev                       # docker compose up -d --wait, with healthcheck loop
coral env status --format markdown       # | service | state | health | restarts | ports |
coral env logs api --tail 100
coral verify                             # liveness only, <30s — exits non-zero if any healthcheck fails
coral test --tag smoke                   # functional smoke tests, <2min
coral test --format junit > junit.xml    # consumed by GitHub Actions reporter / CircleCI / Jenkins
coral down                               # tear down
```

Author tests as YAML in `.coral/tests/*.yaml`:

```yaml
name: api smoke
service: api
tags: [smoke]
retry: { max: 3, backoff: exponential, on: ["5xx"] }
steps:
  - http: GET /users
    expect:
      status: 200
      body_contains: "users"
  - http: POST /users
    body: { name: "test" }
    capture: { user_id: "$.id" }
    expect:
      status: 201
  - http: GET /users/${user_id}           # ${var} substitution from previous capture
    expect:
      status: 200
      snapshot: "fixtures/user.json"      # snapshot assertion; --update-snapshots accepts new outputs
  - exec: ["psql", "-U", "postgres", "-c", "select count(*) from users"]
    expect:
      exit_code: 0
      stdout_contains: "1"
```

Or in `.hurl` syntax (one block per request, no extra metadata required):

```hurl
# coral: name=api-smoke service=api tags=smoke,api
GET /health
HTTP 200

GET /users
Authorization: Bearer test-token
HTTP 200
[Asserts]
jsonpath "$.users" exists
```

Or auto-generate them from your OpenAPI spec — **no LLM, deterministic**:

```bash
coral test-discover                              # print summary
coral test-discover --emit yaml                  # emit YAML to stdout
coral test-discover --commit                     # write under .coral/tests/discovered/
coral test --include-discovered                  # include discovered cases in the run
```

### Multi-repo interface change detection

The single most expensive bug in microservice testing: service A changes its OpenAPI, breaks service B's expectations, and you only find out 20 minutes into a CI run when the runtime test fails with a generic 404. **`coral contract check`** prevents this by diffing each consumer's `.coral/tests/` against each provider's `openapi.yaml` *before* the test environment is even brought up:

```bash
coral contract check                  # markdown summary; exit 0 if only warnings
coral contract check --strict         # fail on any finding (CI gate)
coral contract check --format json    # CI-friendly machine-readable output
```

What it detects (deterministic, no LLM):

| Drift | Severity | Example |
|---|---|---|
| **Unknown endpoint** | Error | worker tests `GET /users/{id}` but api removed it |
| **Unknown method** | Error | worker tests `POST /users` but api only declares `GET /users` |
| **Status drift** | Warning (Error in `--strict`) | worker expects `200` but api now documents only `201` |
| **Missing provider spec** | Warning | worker `depends_on api` but no `openapi.yaml` at `repos/api/` |

Coverage tested in [`crates/coral-cli/tests/multi_repo_interface_change.rs`](crates/coral-cli/tests/multi_repo_interface_change.rs) — 8 end-to-end scenarios. Both YAML and Hurl test files are scanned. Path matching honors OpenAPI `{param}` placeholders against consumer-side concrete paths and `${var}` runtime substitutions.

For Pact-style consumer-driven contracts with a `coral.contracts.lock` and `--can-i-deploy`, see the v0.20+ roadmap.

---

## Quickstart — MCP server for coding agents

Coral exposes the wiki + manifest + lockfile + test results as a [Model Context Protocol](https://modelcontextprotocol.io/) server — any MCP-speaking agent (Claude Code, Cursor, Continue, Cline, Goose, Codex, Copilot, …) can read it cross-session.

```bash
coral mcp serve                                  # default: stdio transport, --read-only
```

Wire it into Claude Code with a `.claude/mcp.json` snippet (see [docs/CLAUDE_CODE.md](docs/CLAUDE_CODE.md) for the full setup):

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

Or generate the agent instruction files directly (deterministic, no LLM):

```bash
coral export-agents --format agents-md       --write    # writes AGENTS.md
coral export-agents --format claude-md       --write    # writes CLAUDE.md
coral export-agents --format cursor-rules    --write    # writes .cursor/rules/coral.mdc
coral export-agents --format copilot         --write    # writes .github/copilot-instructions.md
coral export-agents --format llms-txt        --write    # writes llms.txt
```

**Why deterministic templates instead of LLM-generated?** Empirical work on context files (and [Anthropic's context-engineering guidance](https://www.anthropic.com/engineering/context-engineering)) has consistently found that LLM-synthesized `AGENTS.md` files degrade agent task success vs. human-curated or template-rendered ones. Coral's templates pull structured data from `coral.toml` (project name, repos, dependencies) — not synthesized prose. Richer manifest blocks (`[project.agents_md]`, `[hooks]`) are on the v0.20+ roadmap; today the renderer reads only the fields that ship parsed.

For prompt-paste workflows where you don't have an MCP-speaking client:

```bash
coral context-build --query "how does authentication work" --budget 50000 > context.md
# Pastes a curated, budget-bounded markdown blob ready for any prompt.
```

The loader uses TF-IDF ranking + backlink BFS + greedy fill under your token budget, sorted by `(confidence desc, body length asc)` so the most-trusted concise sources lead.

---

## Subcommand reference

### Wiki layer (v0.15+)

| Command | Purpose | Needs LLM? |
|---|---|---|
| `coral init [--force]` | Scaffold `.wiki/` with SCHEMA, index, log, and 9 type subdirs. Idempotent. | No |
| `coral bootstrap [--apply]` | First-time wiki compilation from `HEAD`. `--dry-run` (default) prints plan; `--apply` writes pages. | Yes |
| `coral ingest [--from SHA] [--apply]` | Incremental update from `last_commit`. Same dry-run / apply semantics. | Yes |
| `coral query <q>` | Streamed answer using the wiki as context. Cites slugs. | Yes |
| `coral lint [--structural\|--semantic\|--all] [--fix] [--rule R]` | 9 structural + 1 LLM semantic check, optional auto-fix. Exit 1 on critical. | Optional |
| `coral consolidate [--apply]` | Suggest merges, retirements, splits. Output YAML — caller decides. | Yes |
| `coral stats [--format markdown\|json]` | Health dashboard. JSON validates against `docs/schemas/stats.schema.json`. | No |
| `coral search <q> [--engine tfidf\|embeddings] [--algorithm tfidf\|bm25] [--limit N]` | TF-IDF default; Voyage embeddings opt-in. `--algorithm bm25` switches the offline ranker (better precision on 100+ page wikis). Top-N pages with score + snippet. | No (TF-IDF/BM25) / Voyage key (embeddings) |
| `coral sync [--version V] [--remote]` | Lay subagents/prompts/workflow into `<cwd>/template/`. Per-file pinning via `.coral-pins.toml`. | No |
| `coral export --format <markdown-bundle\|json\|notion-json\|jsonl\|html> [--out FILE] [--qa]` | Export the wiki. With `--qa`, jsonl emits LLM-generated Q/A pairs. | Optional |
| `coral notion-push [--type T]` | Push pages to a Notion database via curl. Reads `NOTION_TOKEN` + `CORAL_NOTION_DB`. | No |
| `coral onboard --profile <P>` | Tailored 5–10 page reading path for a reader profile. | Yes |
| `coral prompts list` | Show which prompts are local-overridden, embedded, or fallback. | No |
| `coral validate-pin` | Verify every version in `.coral-pins.toml` exists as a tag in the remote repo. | No |
| `coral diff <slugA> <slugB>` | Structural diff (frontmatter, sources, wikilinks, body stats). | No |
| `coral status [--format markdown\|json]` | Daily-use dashboard. | No |
| `coral history <slug>` | Log entries that mention a slug, reverse chronological. | No |

### Multi-repo layer (v0.16+)

| Command | Purpose |
|---|---|
| `coral project new [<name>] [--remote R] [--force] [--pin-toolchain]` | Create `coral.toml` + empty `coral.lock`. |
| `coral project list [--format markdown\|json] [--tag T]` | Tabular view of declared repos with resolved URLs. |
| `coral project add <name> [--url\|--remote] [--ref] [--path] [--tags ...] [--depends-on ...]` | Append a repo entry. Validates manifest invariants on save. |
| `coral project sync [--repo N]... [--tag T]... [--exclude N]... [--sequential] [--strict]` | Clone or fast-forward selected repos (parallel via rayon by default). Auth failures and dirty trees are skipped-with-warning. |
| `coral project lock [--dry-run]` | Refresh `coral.lock` from the manifest without pulling. |
| `coral project graph [--format mermaid\|dot\|json] [--title T]` | Visualize repo dependency graph. Mermaid renders inline in GitHub-flavored Markdown. |
| `coral project doctor [--strict]` | Drift / health check: unknown apiVersion, missing clones, stale lockfile entries, duplicate paths. |

### Environments layer (v0.17+)

| Command | Purpose |
|---|---|
| `coral up [--env NAME] [--service NAME]... [--detach] [--build]` | `EnvBackend::up`. Default `--detach=true`. Compose backend renders `.coral/env/compose/<hash>.yml`. |
| `coral down [--env] [--volumes] [--yes]` | Tear down. `--yes` required when `production = true`. |
| `coral env status [--env] [--format markdown\|json]` | Live service state from `EnvBackend::status()`. |
| `coral env logs <service> [--env] [--tail N]` | Read recent logs (compose `logs --no-color --no-log-prefix --timestamps`). |
| `coral env exec <service> [--env] -- <cmd>...` | One-shot exec inside a container. Exit code propagates. |

### Functional testing layer (v0.18+)

| Command | Purpose |
|---|---|
| `coral verify [--env NAME]` | Run all healthchecks. Liveness only, <30s budget. Exit non-zero on any fail. |
| `coral test [--service N]... [--kind smoke\|healthcheck\|user-defined]... [--tag T]... [--format markdown\|json\|junit] [--update-snapshots] [--include-discovered] [--env]` | Run union of healthcheck + user-defined YAML + Hurl + optional OpenAPI-discovered cases. JUnit XML for CI. |
| `coral test-discover [--emit markdown\|yaml] [--commit]` | Auto-generate TestCases from `openapi.{yaml,yml,json}` in repos. **No LLM**, deterministic mapping. |
| `coral contract check [--format markdown\|json] [--strict]` | **Cross-repo interface drift detection.** Walks each repo's OpenAPI spec (provider) and `.coral/tests/*.{yaml,yml,hurl}` (consumer); for every `[[repos]] depends_on` edge reports unknown endpoints, unknown methods, and status-code drift. Fails fast in CI **before** the test environment is even brought up. |

### AI ecosystem layer (v0.19+)

| Command | Purpose |
|---|---|
| `coral mcp serve [--transport stdio] [--read-only true|false] [--allow-write-tools]` | MCP server (JSON-RPC 2.0 stdio, MCP 2025-11-25). Exposes 6 resources, 3 prompts, and 5 read-only tools (`query`, `search`, `find_backlinks`, `affected_repos`, `verify`); the 3 write tools (`run_test`, `up`, `down`) require `--allow-write-tools`. Read-only by default — pass `--read-only false` to disable. |
| `coral export-agents --format <agents-md\|claude-md\|cursor-rules\|copilot\|llms-txt> [--write] [--out PATH]` | Manifest-driven instruction file emission. **NOT LLM-driven** — see [Anthropic's context-engineering guidance](https://www.anthropic.com/engineering/context-engineering) for why deterministic templates beat synthesized ones. |
| `coral context-build --query <q> --budget <tokens> [--format markdown\|json] [--seeds N]` | Smart context loader. TF-IDF rank + backlink BFS + greedy fill under token budget. |

---

## The wiki schema

Every page in `.wiki/` has YAML frontmatter:

```yaml
---
slug: auth-flow
type: flow                          # module | concept | entity | flow | decision | synthesis | operation | source | gap | index | log | schema | readme | reference
last_updated_commit: a1b2c3d
confidence: 0.85                    # 0.0..1.0; pages with confidence >= 0.7 must cite >=1 source
status: reviewed                    # draft | reviewed | verified | stale | archived | reference
sources:                            # plain strings (path:line-range or PR ref)
  - "src/auth.rs:12-87"
  - "PR #142"
backlinks:
  - login-handler
  - jwt-verification
---

# How auth works

This page explains the auth flow…
```

Wikilinks use `[[slug]]` syntax. In multi-repo projects with name collisions, use `[[<repo>/<slug>]]`. The lint detects ambiguity and exits non-zero on bad references.

The full SCHEMA — Page types, Confidence semantics, Source format, lint rules — lives in `template/schema/SCHEMA.base.md` (embedded in the binary via `include_dir!`). `coral sync` lays a copy at `<repo>/template/`.

---

## The `coral.toml` manifest

Lives at the meta-repo root. TOML for consistency with `Cargo.toml` / `.coral-pins.toml`.

```toml
apiVersion = "coral.dev/v1"               # closed list. Future versions will hard-fail with a migrate hint.

[project]
name = "orchestra"
wiki_layout = "aggregated"                # only "aggregated" in v0.19; per-repo layout deferred.

[project.toolchain]
coral = "0.19.0"                          # pinned binary version, like .coral-pins.toml at the project level.

[project.defaults]
ref           = "main"                    # default branch / tag / sha
remote        = "github"                  # default remote name (refs into [remotes.<name>])
path_template = "repos/{name}"            # default checkout path; {name} substituted

[remotes.github]
fetch = "git@github.com:acme/{name}.git"

[remotes.gitlab-internal]
fetch = "git@gitlab.acme.internal:platform/{name}.git"

[[repos]]
name = "api"                              # url resolves to git@github.com:acme/api.git
ref  = "release/v3"                       # override of defaults.ref
tags = ["service", "team:platform"]

[[repos]]
name       = "worker"
remote     = "gitlab-internal"            # override of defaults.remote
tags       = ["service", "team:data"]
depends_on = ["api"]                      # implicit cross-repo dep (used by `--affected` filters)

[[repos]]
name = "shared"
url  = "git@github.com:acme/shared.git"   # explicit URL, overrides remote+template
tags = ["library"]

[[environments]]
name            = "dev"
backend         = "compose"               # compose | kind | tilt (only compose in v0.19)
mode            = "managed"               # managed | adopt
compose_command = "auto"                  # auto | docker | podman
production      = false
env_file        = "env/dev.env"           # optional: load env vars from this file

[environments.services.api]
kind       = "real"
repo       = "api"
build      = { context = ".", dockerfile = "Dockerfile", target = "dev" }
ports      = [3000]
env        = { DATABASE_URL = "postgres://db:5432/app" }
depends_on = ["db"]

[environments.services.api.healthcheck]
kind          = "http"
path          = "/health"
expect_status = 200
headers       = { "X-Internal-Auth" = "${HEALTHCHECK_TOKEN}" }

[environments.services.api.healthcheck.timing]
interval_s           = 2
timeout_s            = 5
retries              = 5
start_period_s       = 30
start_interval_s     = 1
consecutive_failures = 3

[environments.services.db]
kind  = "real"
image = "postgres:16"
ports = [5432]

[environments.services.db.healthcheck]
kind = "tcp"
port = 5432

[environments.services.db.healthcheck.timing]
interval_s           = 5
timeout_s            = 3
retries              = 6
start_period_s       = 20
consecutive_failures = 3
```

Validation rules (enforced on every load):

- `apiVersion` ∈ `{"coral.dev/v1"}`. Hard-fail with an actionable message on anything else.
- Every `service.repo` references a real `[[repos]].name`.
- `repo.depends_on` and `service.depends_on` cycles are detected (DFS three-color marking) and rejected.
- `production = true` requires `--yes` on `coral down`, `coral env exec`, `coral env reset`.

---

## The `coral.lock` lockfile

Sibling of `coral.toml`. Separates **intent** (`ref = "main"` in the manifest) from **resolved** (the SHA actually clone-ed). Same role as `Cargo.lock` / `package-lock.json` / `MODULE.bazel.lock`. Written atomically (`tmp + rename` while holding `flock(2)` on the file) by `coral project sync`.

```toml
# Generated by `coral project sync` — do NOT edit by hand
apiVersion = "coral.dev/v1"
resolved_at = "2026-05-03T14:22:11Z"

[repos.api]
url        = "git@github.com:acme/api.git"
ref        = "release/v3"
sha        = "8f3a9b2c1d4e5f6789abcdef0123456789abcdef"
synced_at  = "2026-05-03T14:22:08Z"

[repos.worker]
url        = "git@gitlab.acme.internal:platform/worker.git"
ref        = "main"
sha        = "1a2b3c4d5e6f7890abcdef1234567890abcdef12"
synced_at  = "2026-05-03T14:22:10Z"
```

Auto-creates on first read (Cargo.lock semantics). `coral project doctor` reports drift.

---

## Test schema (`.coral/tests/*.{yaml,hurl}`)

Two formats are supported side-by-side; detection is by file extension. Both are parsed into the same in-memory `YamlSuite` model so the same executor runs them.

### YAML format

```yaml
name: api smoke
service: api
tags: [smoke, regression]
retry:
  max: 3
  backoff: exponential                    # none | linear | exponential (capped at 5s)
  on: ["5xx", "timeout"]                  # 5xx | 4xx | timeout | any
steps:
  - http: GET /users
    headers: { Accept: "application/json" }
    expect:
      status: 200
      body_contains: '"users":'

  - http: POST /users
    body: { name: "test", email: "test@example.com" }
    capture: { user_id: "$.id" }          # extract from response, available as ${user_id} below
    expect: { status: 201 }
    retry: { max: 5, backoff: linear, on: ["5xx"] }   # per-step retry overrides suite default

  - http: GET /users/${user_id}
    expect:
      status: 200
      snapshot: "fixtures/user.json"      # writes on first run / --update-snapshots, compares otherwise

  - exec: ["psql", "-U", "postgres", "-c", "select count(*) from users"]
    expect:
      exit_code: 0
      stdout_contains: "1"
```

Supported assertions on HTTP steps: `status`, `body_contains`, `snapshot`. (gRPC steps, GraphQL helper, JSONPath asserts beyond capture, parallel execution: deferred to v0.20+.)

### Hurl format

```hurl
# coral: name=api-smoke service=api tags=smoke,api

GET /health
HTTP 200

GET /users
Authorization: Bearer test-token
HTTP 200
[Asserts]
jsonpath "$.users" exists

POST /users
HTTP 201
```

The minimal Hurl subset Coral parses: request line (`<METHOD> <URL>`), headers, `HTTP <status>` response line, `[Asserts] jsonpath "$.path" exists`, and a `# coral: name=… service=… tags=…` directive. Captures, options, and request bodies are deferred — write those in YAML for now.

### OpenAPI auto-discovery (no LLM)

Drop an `openapi.yaml` / `openapi.json` / `swagger.yaml` / `swagger.json` anywhere in your project (Coral walks recursively, skipping `.git/`, `.coral/`, `node_modules/`, `target/`, `vendor/`, `dist/`, `build/`). One TestCase per `(path, method)` is emitted, with `expect.status` set from the spec's lowest 2xx response. Endpoints that declare `requestBody.required = true` are skipped — Coral never fabricates request bodies.

```bash
coral test-discover                       # print summary table
coral test-discover --emit yaml           # print YAML test suites to stdout
coral test-discover --commit              # write .coral/tests/discovered/<id>.yaml
coral test --include-discovered           # include them in the run
```

---

## Backward compatibility

**Hard guarantee:** every v0.15 single-repo workflow keeps working byte-for-byte on v0.19+. No `coral.toml` → every command synthesizes a 1-repo project from the cwd via `Project::synthesize_legacy`. Pinned by the [`bc-regression` test suite](crates/coral-cli/tests/bc_regression.rs) that runs on every PR.

| Surface | v0.15 behavior | v0.19+ behavior |
|---|---|---|
| `coral init` (no `coral.toml`) | scaffolds `<cwd>/.wiki/` | identical |
| `coral status`, `coral lint`, `coral query`, … | operate on `<cwd>/.wiki/` | identical when no `coral.toml` is found |
| `--wiki-root <path>` flag | overrides default | identical |
| `coral init` after migrating to `coral.toml` | n/a | scaffolds `<root>/.wiki/` (the project's aggregated wiki); single-repo entries still work |

What's *not* preserved: when you opt in to multi-repo by creating a `coral.toml`, the wiki layout becomes aggregated and slugs may need to be namespaced. The lint detects this and exits non-zero on ambiguous wikilinks. There's no in-place migration tool — the recommended path is "stop the world, run `coral consolidate` once after migrating."

---

## CI integration

### GitHub Actions

```yaml
- name: Coral lint (wiki structural integrity)
  uses: agustincbajo/Coral/.github/actions/lint@v0.19.0

- name: Coral ingest (incremental wiki update)
  uses: agustincbajo/Coral/.github/actions/ingest@v0.19.0
  env:
    ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}

- name: Coral verify (env healthchecks)
  run: |
    coral up --env ci --detach
    coral verify --env ci
    coral down

- name: Coral functional tests (JUnit XML)
  run: |
    coral up --env ci --detach
    coral test --env ci --tag smoke --format junit > junit.xml
    coral down
- uses: dorny/test-reporter@v1
  with:
    name: 'Coral functional tests'
    path: junit.xml
    reporter: java-junit
```

### GitLab CI

```yaml
coral_lint:
  image: rust:1.85
  script:
    - cargo install --locked --git https://github.com/agustincbajo/Coral --tag v0.19.0 coral-cli
    - coral lint --all
    - coral verify --env ci

coral_test:
  image: docker:24
  services: [docker:dind]
  script:
    - coral up --env ci --detach
    - coral test --env ci --format junit > junit.xml
    - coral down
  artifacts:
    reports:
      junit: junit.xml
```

### Composite GitHub Actions

Five composite actions ship under `.github/actions/`:

- `ingest/` — incremental wiki ingest (calls `coral ingest --apply`)
- `lint/` — structural + semantic lint with PR-comment summary
- `consolidate/` — weekly consolidation suggestion (PR with proposed merges)
- `embeddings-cache/` — Voyage embeddings cache restore/save
- `validate/` — Hermes-style PR validator (LLM-validated wiki claims)

A `verify` action for env healthchecks lands in v0.20+.

---

## Multi-provider LLM support

```bash
# Default: Claude Code CLI (claude binary in $PATH)
coral query "..."

# Gemini
CORAL_PROVIDER=gemini coral query "..."

# Local llama.cpp
CORAL_PROVIDER=local CORAL_LOCAL_BINARY=$HOME/llama.cpp/build/bin/llama-cli coral query "..."

# Any OpenAI-compatible endpoint (vLLM, LM Studio, Ollama OpenAI mode, …)
CORAL_PROVIDER=http CORAL_HTTP_BASE_URL=http://localhost:11434/v1 \
  CORAL_HTTP_MODEL=llama3.1:70b coral query "..."

# Tests / CI: deterministic
CORAL_PROVIDER=mock coral query "..."
```

The 5 runners share a single `Runner: Send + Sync` trait (`crates/coral-runner/src/runner.rs`); errors are provider-agnostic since v0.15.1.

---

## Auth setup

| Service | Env var | How to get a token |
|---|---|---|
| Anthropic (Claude) | `ANTHROPIC_API_KEY` | console.anthropic.com → API keys |
| Google (Gemini) | `GEMINI_API_KEY` | aistudio.google.com → Get API key |
| Voyage embeddings | `VOYAGE_API_KEY` | voyageai.com → API |
| OpenAI embeddings | `OPENAI_API_KEY` | platform.openai.com → API keys |
| Notion push | `NOTION_TOKEN` + `CORAL_NOTION_DB` | notion.so/my-integrations → New integration; copy DB ID from URL |

Coral never prompts for or stores credentials. Git auth (for `coral project sync`) delegates 100% to your SSH agent / git credential helper / `~/.gitconfig`. PRD risk #10: when one repo's auth fails, sync skips it with a warning instead of aborting the whole project.

---

## Configuration

| File | Purpose | Format |
|---|---|---|
| `coral.toml` | Project manifest | TOML |
| `coral.lock` | Resolved SHAs | TOML, generated, do not edit |
| `coral.local.toml` | Per-developer overrides (gitignored) | TOML; merged in memory |
| `.coral-pins.toml` | Pin Coral version + per-file template overrides | TOML |
| `.coral/tests/*.yaml` | User-defined YAML test suites | YAML |
| `.coral/tests/*.hurl` | User-defined Hurl test suites | Hurl |
| `.coral/tests/discovered/*.yaml` | OpenAPI-discovered tests | YAML, generated |
| `.coral/snapshots/*.json` | Snapshot fixtures | JSON, written by tests |
| `.coral/env/compose/<hash>.yml` | Generated compose YAML | YAML, generated |
| `.coral/audit.log` | MCP write-tool audit | text, append-only |
| `.coral-cache.json` | Embeddings + ingest cache | JSON, gitignored |
| `.wiki/index.md` | Wiki index | Markdown + frontmatter |
| `.wiki/log.md` | Append-only operation log | Markdown |
| `.wiki/SCHEMA.md` | Schema contract for the bibliotecario subagent | Markdown |

Environment variables:

| Var | Purpose | Default |
|---|---|---|
| `CORAL_PROVIDER` | LLM provider for `query`/`bootstrap`/etc. | `claude` |
| `CORAL_LOCAL_BINARY` | Path to `llama-cli` for local provider | (none) |
| `CORAL_HTTP_BASE_URL` | OpenAI-compat base URL | (none) |
| `CORAL_HTTP_MODEL` | Model name for HTTP provider | `gpt-4o-mini` |
| `CORAL_HTTP_API_KEY` | API key for HTTP provider | (read from request env if unset) |
| `CORAL_EMBEDDINGS_BACKEND` | `json` or `sqlite` | `json` |
| `CORAL_EMBEDDINGS_PROVIDER` | `voyage` / `openai` / `mock` | `voyage` |
| `RUST_LOG` | Logging filter (e.g. `coral=debug,info`) | `info` |
| `RUST_BACKTRACE` | Stack traces on panic | (unset) |
| `ANTHROPIC_API_KEY` | Auth for Claude runner | (none) |
| `GEMINI_API_KEY` | Auth for Gemini runner | (none) |
| `VOYAGE_API_KEY` | Auth for Voyage embeddings | (none) |
| `OPENAI_API_KEY` | Auth for OpenAI embeddings / HTTP runner | (none) |
| `NOTION_TOKEN` + `CORAL_NOTION_DB` | Auth + DB id for `coral notion-push` | (none) |

---

## Architecture

8 crates in a Cargo workspace. Each crate owns one concern; the trait families (`Runner`, `EnvBackend`, `TestRunner`, `ResourceProvider`/`ToolDispatcher`) keep concrete implementations swappable.

```
crates/
├── coral-cli/        # 34 CLI subcommands; clap dispatcher; thin adapters over the libraries
├── coral-core/       # types: Page, Frontmatter, WikiIndex, WikiLog, Project, Lockfile;
│                     # atomic file writes + flock; gitdiff + git_remote subprocess wrappers;
│                     # wiki walk (rayon); TF-IDF search; embeddings JSON+SQLite backends.
├── coral-env/        # EnvBackend trait + ComposeBackend (compose YAML render + subprocess);
│                     # Healthcheck model (Http/Tcp/Exec/Grpc + timing); EnvPlan + status;
│                     # MockBackend for upstream tests; runtime detection (docker/podman).
├── coral-test/       # TestRunner trait + 9 TestKind variants; HealthcheckRunner + UserDefinedRunner +
│                     # HurlRunner + OpenAPI Discovery; probe (TCP/HTTP/exec/gRPC); JUnit emit;
│                     # MockTestRunner.
├── coral-mcp/        # JSON-RPC 2.0 stdio MCP server; ResourceProvider trait; static catalogs
│                     # (resources, tools, prompts); read-only enforcement; protocol 2025-11-25.
├── coral-runner/     # Runner trait (Send+Sync); 5 impls: Claude, Gemini, Local, Http, Mock;
│                     # PromptBuilder with {{var}} substitution.
├── coral-lint/       # 9 structural checks + 1 LLM semantic check; auto-fix routing.
└── coral-stats/      # StatsReport (totals, by_type/status, confidence stats).
```

Dependency graph (top-down):

```
coral-cli ─┬─→ coral-core ──→ rusqlite, fs4, walkdir, serde, toml, chrono, rayon
           ├─→ coral-env ───→ coral-core
           ├─→ coral-test ──→ coral-env, coral-core
           ├─→ coral-mcp ───→ coral-core
           ├─→ coral-runner → (no internal deps)
           ├─→ coral-lint ──→ coral-core, coral-runner
           └─→ coral-stats ─→ coral-core
```

Trait pluggability:

```rust
// coral-runner — LLM backends
pub trait Runner: Send + Sync {
    fn run(&self, prompt: Prompt) -> RunnerResult<RunOutput>;
    fn run_streaming(&self, prompt: Prompt, sink: &mut dyn Write) -> RunnerResult<RunOutput>;
}

// coral-env — environment backends (compose today; kind/tilt deferred)
pub trait EnvBackend: Send + Sync {
    fn up(&self, plan: &EnvPlan, opts: &UpOptions) -> EnvResult<EnvHandle>;
    fn down(&self, plan: &EnvPlan, opts: &DownOptions) -> EnvResult<()>;
    fn status(&self, plan: &EnvPlan) -> EnvResult<EnvStatus>;
    fn logs(&self, plan: &EnvPlan, service: &str, opts: &LogsOptions) -> EnvResult<Vec<LogLine>>;
    fn exec(&self, plan: &EnvPlan, service: &str, cmd: &[String], opts: &ExecOptions) -> EnvResult<ExecOutput>;
}

// coral-test — test runners
pub trait TestRunner: Send + Sync {
    fn supports(&self, kind: TestKind) -> bool;
    fn run(&self, case: &TestCase, env: &EnvHandle) -> TestResult<TestReport>;
    fn discover(&self, project_root: &Path) -> TestResult<Vec<TestCase>>;
}

// coral-mcp — MCP resource + tool providers
pub trait ResourceProvider: Send + Sync {
    fn list(&self) -> Vec<Resource>;
    fn read(&self, uri: &str) -> Option<String>;
}
pub trait ToolDispatcher: Send + Sync {
    fn call(&self, name: &str, args: &serde_json::Value) -> ToolCallResult;
}
```

For deeper architecture notes see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

---

## Performance

Measured on an Apple M1 Pro (10c CPU, 32GB RAM) against Coral's own dogfooded `.wiki/` (95 pages, ~1.2MB).

| Operation | Cold start | Warm | Notes |
|---|---|---|---|
| `coral status` | 78ms | 42ms | walks `.wiki/` once |
| `coral lint --structural` | 110ms | 60ms | 9 checks, all rayon-parallel where it matters |
| `coral search <q>` | 95ms | 50ms | TF-IDF default; embeddings opt-in |
| `coral stats --format json` | 88ms | 45ms | |
| `coral project graph --format mermaid` | 12ms | 8ms | pure data transformation |
| `coral mcp serve` (one round-trip) | n/a | <5ms | JSON-RPC dispatch |
| `coral context-build --budget 50000` | 130ms | 70ms | TF-IDF + BFS |

Binary size: **~6.3 MB stripped** (release build, `panic = "abort"`, LTO thin, codegen-units = 1, all features). The `coral-mcp` server, `rusqlite` (bundled), and the LLM-runner adapters are the largest contributors. Disable defaults via Cargo features for a smaller binary.

The `coral-env` and `coral-test` layers are I/O-bound (subprocess `docker compose`, `git`, `curl`); CPU is never the bottleneck there.

---

## Testing & CI

Coral's own test suite is large because it's the reference user — every refactor that breaks the trait contract is caught.

| Crate | Unit tests | Notes |
|---|---|---|
| `coral-cli` | 222 | clap parser, every command's happy path, error paths |
| `coral-core` | 157 | manifest parser, Lockfile round-trip, wikilinks, frontmatter, atomic writes, flock concurrency, walk, search, embeddings (JSON + SQLite), git_remote |
| `coral-env` | 21 | compose YAML render (per-field), runtime detection (docker/podman), healthcheck loop with consecutive_failures policy, MockBackend recorder |
| `coral-test` | 48 | probe (TCP open/closed, exec true/false), Hurl parser, OpenAPI discovery, captures + retry + snapshot, JUnit XML, MockTestRunner |
| `coral-mcp` | 18 | JSON-RPC dispatch matrix, read-only enforcement, prompts substitution |
| `coral-runner` | 47 | per-runner contract tests (Claude, Gemini, Local, Http, Mock); cross-runner contract conformance |
| `coral-lint` | 64 | per-rule unit tests + 5 ignored realistic-fixture tests |
| `coral-stats` | 9 | |
| **Integration (E2E)** | **30+** | `bc_regression` (6), `multi_repo_project` (12), `full_lifecycle_v019` (4), `cli_smoke`, `cross_process_lock`, `e2e_full_lifecycle`, `e2e_query_cycle`, `snapshot_cli`, `stress_large_wiki`, `template_validation` |

Run them all:

```bash
cargo test --workspace --all-features
cargo test --test bc_regression -p coral-cli           # backward-compat gate
cargo test --test full_lifecycle_v019 -p coral-cli     # end-to-end CLI
cargo test --test multi_repo_project -p coral-cli      # multi-repo + env + MCP
```

CI (GitHub Actions, `.github/workflows/ci.yml`) gates on:

- **Rustfmt** — `cargo fmt --all -- --check`
- **Clippy** — `cargo clippy --workspace --all-targets -- -D warnings`
- **Test (stable)** — `cargo test --workspace --all-features`
- **Test (MSRV 1.85)** — `cargo build --workspace --locked`
- **Backward-compat** — `cargo test --test bc_regression -p coral-cli`
- **Cross-platform smoke** (ubuntu-latest, macos-latest) — `cargo build --release && coral init` round-trip
- **Licenses + duplicate versions** — `cargo deny --all-features check`
- **Security audit** — `cargo audit --deny warnings`
- **Coverage** — `cargo llvm-cov` → Codecov

Concurrency: each PR cancels the previous in-progress run on the same ref.

Nightly (`.github/workflows/nightly.yml`) runs the `--ignored` smoke tests against real LLM and embeddings APIs — Anthropic, Gemini, Voyage, OpenAI, plus a 200-page wiki stress test.

---

## Troubleshooting

### CI is showing "recent account payments have failed"

This is a **GitHub Actions billing issue**, not a Coral bug. Update your billing settings or spending limit at github.com/settings/billing/payment_information. The CI workflow itself is correct — re-running after billing is resolved should work without code changes.

### `coral up` fails on macOS Sonoma+ with "compose watch" file-descriptor errors

Known [Docker Desktop 4.57+ regression](https://github.com/docker/for-mac/issues/7832). Workarounds:

- Add a `.dockerignore` at each repo root excluding `vendor/`, `node_modules/`, `target/`.
- Pass `--no-watch` to `coral up` (not yet shipped — coming in v0.19.x; for now, edit the manifest's `[services.*.watch]` block and re-run `up`).
- Switch to Linux for development, or use `colima` / `podman` (`compose_command = "podman"`).

### `coral project sync` fails on one repo, succeeds on others

By design (PRD risk #10). Sync prints a `⚠ skipped (auth)` per failed repo and continues. Common fixes:

- Auth — `ssh -T git@github.com` to verify your key is loaded; `eval $(ssh-agent) && ssh-add` if not.
- 2FA / SAML — visit the repo's URL in your browser to complete SSO before the next sync.
- Skip the failing repo via `coral project sync --exclude badrepo` until it's resolved.

### `coral mcp serve` says "tool 'X' is not wired in this build"

v0.19.5+ ships a real dispatcher: `search`, `find_backlinks`, and `affected_repos` return live data. `query` is intentionally deferred — it requires an LLM provider key and streaming, which doesn't fit the JSON-RPC tools/call envelope; use the CLI `coral query` for those. `verify`, `run_test`, `up`, `down` still return a `Skip` (the env-touching tools need wiring through `coral-env`); they're the next batch.

### `coral test --include-discovered` finds my OpenAPI but generates 0 cases

Check the spec: every operation has either no `requestBody` or `requestBody.required = false`. Coral never fabricates request bodies. For `POST` / `PUT` / `PATCH` endpoints with required bodies, write the test in YAML/Hurl by hand, or run `coral test generate` (LLM-augmented, post-MVP).

### `cargo install --locked` fails with "unable to find a matching version"

Coral's MSRV is 1.85. Check `rustc --version`; if older, `rustup update stable`.

### Wiki query returns "I don't know" on something I'm sure is in the wiki

- `coral search <q>` first to confirm the page exists and isn't hidden behind unusual frontmatter.
- `coral query --strict <q>` (planned for v0.20) requires citations and avoids hallucination.
- `coral lint --semantic` to detect contradictions; the LLM may be giving up on conflicting pages.

### How do I migrate from `.wiki/` (v0.15) to `coral.toml` multi-repo (v0.19+)?

Single-repo workflows keep working as-is — no migration required. To move to multi-repo:

1. From the directory that should be your project root: `coral project new <name>`.
2. `coral project add <each-repo>`.
3. `coral project sync` to clone everything.
4. The aggregated `.wiki/` lives at the project root; the per-repo wiki at `<old-repo>/.wiki/` is preserved but no longer used by `coral query`. Either move pages into the new aggregated wiki manually (one-time chore) or run `coral consolidate` to suggest merges.

---

## Roadmap

✅ **Shipped (v0.19.0):**

- Multi-repo manifest (`coral.toml`), lockfile (`coral.lock`), 7 `coral project` subcommands.
- Real `ComposeBackend` + `coral up`/`down`/`env *` (compose v2 / v1 / podman).
- `HealthcheckRunner` + `UserDefinedRunner` (YAML + Hurl) with retry / captures / snapshots.
- OpenAPI auto-discovery (`coral test-discover`, no LLM).
- MCP server (`coral mcp serve`) — JSON-RPC 2.0 stdio, MCP 2025-11-25.
- `coral export-agents` (manifest-driven instruction files for AGENTS.md / CLAUDE.md / cursor-rules / copilot / llms-txt).
- `coral context-build` (smart context loader under explicit token budget).
- 700+ unit tests + 30+ E2E across 8 crates.

🚧 **v0.19.x (patches):**

- `coral mcp serve --transport http` (Streamable HTTP / SSE).
- `coral mcp` tool dispatcher wires `query` / `search` / `verify` / `affected_repos` to real CLI commands.
- `coral env devcontainer emit` — generate `.devcontainer/devcontainer.json` from the manifest.
- `coral env import <compose.yml>` — generate a starter `coral.toml` from an existing compose file.
- `coral up --watch` (compose 2.22 `develop.watch`) — Linux first; macOS waits on the Docker bug.
- `coral env attach <service>`, `coral env reset`, `coral env port-forward`, `coral env open`, `coral env prune`.
- `coral lint --check-injection` for prompt-injection patterns in wiki pages.

🔮 **v0.20+:**

- `KindBackend`, `TiltBackend`, `K3dBackend` (k8s local).
- `PropertyBasedRunner` (proptest from OpenAPI), `RecordedRunner` (Keploy traffic capture, Linux-only feature).
- `EventRunner` (AsyncAPI, Testcontainers Kafka/Rabbit), `TraceRunner` (OTLP queries).
- `ContractRunner` (consumer-driven, `coral.contracts.lock` with `--can-i-deploy`).
- `coral test generate --auto-validate` (LLM-augmented, with iterative retry against the live env).
- `coral chaos inject` (Toxiproxy / Pumba sidecar).
- `coral monitor up` (synthetic monitoring, tests-as-monitors).
- `coral skill build / publish` (Anthropic Skills marketplace bundle).
- `MultiStepRunner` (planner + executor + reviewer with per-step model tiering).
- gRPC test steps (via `grpcurl` subprocess or `tonic` reflection).
- Cross-repo glob (`[[repos]] glob = "services/*"`) and sub-manifests `<include>`.
- SWE-ContextBench benchmark publication.

Detailed PRD covering every PRD iteration (multi-repo, environments, testing, MCP, AGENTS.md research) is tracked privately in the maintainer's plans directory; the relevant decisions and trade-offs are summarised in the [CHANGELOG](CHANGELOG.md) per release.

---

## How Coral itself was built

Dogfood: Coral's own `.wiki/` is maintained by Coral. Each merge to `main` runs `coral ingest --apply` via the `.github/actions/ingest` composite action, with a Claude bibliotecario subagent doing the page curation under the SCHEMA in `template/schema/SCHEMA.base.md`.

The PRD ([this](https://github.com/agustincbajo/Coral/blob/main/.claude/plans/quiero-que-eval-es-todo-glittery-eclipse.md)) was written first (5 PRD iterations, validated against industry: Bazel, Nx, Turborepo, Cargo workspaces, Garden, Compose Watch, Tilt, Skaffold, Pact, Schemathesis, Hurl, Stepci, MCP, AGENTS.md research, Devin Wiki competitive analysis). Each `coral project` / `coral env` / `coral test` / `coral mcp` subcommand has a wave-1 (scaffold) → wave-2 (real impl) → wave-3 (advanced features) progression in the CHANGELOG, plus dedicated unit tests at every wave.

The pluggable trait pattern (`Runner` → `EnvBackend` → `TestRunner` → `ResourceProvider`/`ToolDispatcher`) was a deliberate copy of itself: one trait, one error type, one Mock impl, one factory function. Once you've debugged one of them, debugging another is muscle memory.

---

## Contributing

PRs welcome. Three guardrails:

1. **`cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --all-features`** must pass locally before push.
2. **Backward compat** — every PR is gated on `cargo test --test bc_regression`. v0.15 single-repo behavior is sacred.
3. **Wiki drift** — if your PR touches a slug, run `coral lint --check-spec-vs-server` (when wired) and update the relevant page so the wiki doesn't drift.

For larger contributions (new `EnvBackend`, new `TestRunner`, new MCP transport), open a discussion first; the PRD doc is the source of truth for design decisions.

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full guide.

---

## References & related work

### Core influences

- **[Karpathy LLM Wiki gist](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f)** — the page-as-frontmatter-document idea that started Coral.
- **[Anthropic Context Engineering](https://www.anthropic.com/engineering/context-engineering)** — structured note-taking, sub-agents, retrieval. Coral wiki is a concrete implementation.
- **[Model Context Protocol (2025-11-25)](https://modelcontextprotocol.io/specification/2025-11-25)** — the MCP spec Coral pins.

### Multi-repo manifest precedents

- **[Google git-repo manifest](https://gerrit.googlesource.com/git-repo/+/master/docs/manifest-format.md)** — `<remote>` / `<default>` / `<project>` pattern Coral copies.
- **[Bazel `MODULE.bazel`](https://bazel.build/external/module)** — module + lockfile separation; Coral's `coral.toml` + `coral.lock` mirror this.
- **[Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html)** — the simplest "1 manifest, N members" model.

### Functional testing

- **[Hurl](https://hurl.dev)** — HTTP-test-as-text format; Coral parses a minimal subset.
- **[Schemathesis](https://schemathesis.io/)** — property-based API testing from OpenAPI; Coral's `test-discover` is a deterministic subset (full property-based testing in v0.20+).
- **[Pact](https://pact.io)** — consumer-driven contracts; informs the `coral.contracts.lock` semantics planned for v0.20+.
- **[Microservices honeycomb test shape](https://martinfowler.com/articles/2021-test-shapes.html)** — the test pyramid for microservices Coral targets.

### Coding agent ecosystem

- **[AGENTS.md spec](https://agents.md/)** — the cross-tool agent instruction format Coral emits.
- **[Anthropic — Context engineering for agents](https://www.anthropic.com/engineering/context-engineering)** — design rationale for structured note-taking + deterministic instruction files (vs. LLM-synthesized ones); Coral's manifest-driven exporter follows this guidance.
- **[rmcp Rust SDK](https://github.com/modelcontextprotocol/rust-sdk)** — official MCP Rust SDK; Coral's hand-rolled JSON-RPC server in `coral-mcp` will swap to this in v0.20+ if the spec stabilizes further.

### Comparable / adjacent tools

- **[deepwiki-open](https://github.com/AsyncFuncAI/deepwiki-open)** — Python+Docker, single-repo wiki generator. Complementary; Coral's niche is multi-repo + manifest.
- **[OpenDeepWiki](https://github.com/AIDotNet/OpenDeepWiki)** — C#/TS, repo-as-MCP-server. Same MCP angle; different stack.
- **[Devin Wiki](https://cognition.ai/blog/devin-2)** — proprietary closed-source wiki; Coral is the open-source counterpart.
- **[Sourcegraph Cody Enterprise](https://sourcegraph.com/docs/cody/enterprise)** — multi-repo context for agents; complementary, not competitive (Coral is local-first, Cody is hosted).

---

## License

MIT — see [LICENSE](LICENSE).

Coral and its dependencies are independently licensed. `cargo-deny` enforces the allowlist on every PR; the current allowlist is in [deny.toml](deny.toml).
