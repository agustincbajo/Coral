# Coral

> **The project manifest for AI-era development.** Multi-repo wiki + dev environments + functional testing + Model Context Protocol server, in a single Rust binary.

[![CI](https://github.com/agustincbajo/Coral/actions/workflows/ci.yml/badge.svg)](https://github.com/agustincbajo/Coral/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/agustincbajo/Coral?display_name=tag)](https://github.com/agustincbajo/Coral/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](rust-toolchain.toml)
[![MCP](https://img.shields.io/badge/MCP-2025--11--25-blue?logo=anthropic)](https://modelcontextprotocol.io/)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/agustincbajo/Coral/badge)](https://scorecard.dev/viewer/?uri=github.com/agustincbajo/Coral)

Coral is a Karpathy-style LLM wiki for your code, scaled to microservice-shaped projects: declare your repos in a `coral.toml`, bring up a multi-service environment, run functional tests, and expose the whole thing to coding agents (Claude Code, Cursor, Continue, Cline, Goose, Codex, Copilot) via Model Context Protocol — one binary, open source, all local. Hardened across **five multi-agent audit cycles**.

> *"The IDE is Claude Code. The programmer is you + the LLM. The wiki is the living memory of your codebase. Coral is the manifest that makes both intelligible across N repos."*

### Try it in 60 seconds

```bash
# Install (Linux/macOS — Windows prereqs below)
cargo install --locked --git https://github.com/agustincbajo/Coral --tag v0.30.0 coral-cli

# Scaffold a wiki in any git repo
cd /path/to/your/repo
coral init
coral bootstrap --apply   # one-time LLM compile of .wiki/ — needs `claude` on $PATH
coral query "how does authentication work?"
```

That's the single-repo flow. Multi-repo, environments, tests, MCP, and session-distill all build on the same `coral` binary — see the [Quickstart](#quickstart) section below.

---

## Table of contents

**Getting started**
- [What you get](#what-you-get) · [Why Coral](#why-coral) · [Install](#install)
- [Quickstart](#quickstart) (single-repo, multi-repo, environments+tests, MCP server, session-distill)

**Use it**
- [Cookbook — 7 common workflows](#cookbook--common-workflows)
- [MCP client integration](#mcp-client-integration) (Claude Code · Cursor · Continue · Cline · Goose · raw JSON-RPC · HTTP/SSE)
- [Output examples](#output-examples) — what each command actually prints

**Reference**
- [Subcommand reference](#subcommand-reference) · [Wiki schema](#the-wiki-schema) · [`coral.toml`](#the-coraltoml-manifest) · [`coral.lock`](#the-corallock-lockfile) · [Test schema](#test-schema-coraltestsyamlhurl)
- [Multi-provider LLM support](#multi-provider-llm-support) · [Auth setup](#auth-setup) · [Configuration](#configuration)

**Operations**
- [Backward compatibility](#backward-compatibility) · [Security model](#security-model) · [CI integration](#ci-integration) · [Performance](#performance) · [Testing & CI](#testing--ci)
- [Troubleshooting](#troubleshooting) · [FAQ](#faq) · [Glossary](#glossary)

**Project**
- [Architecture](#architecture) · [Comparison vs adjacent tools](#comparison-vs-adjacent-tools) · [Roadmap](#roadmap)
- [How Coral itself was built](#how-coral-itself-was-built) · [Releasing](#releasing) · [Contributing](#contributing) · [References](#references--related-work) · [License](#license)

---

## What you get

A single `coral` binary (~6.3 MB stripped, statically linked, MSRV 1.85, ad-hoc-codesigned on macOS) with **42 leaf subcommands** (29 top-level commands, six of which group sub-subcommands) across six layers:

| Layer | Commands | Since |
|---|---|---|
| **Wiki** | `init` `bootstrap` `ingest` `query` `lint` `consolidate` `stats` `sync` `onboard` `prompts` `search` `export` `notion-push` `validate-pin` `diff` `status` `history` | v0.1+ |
| **Multi-repo** | `project new/list/add/sync/doctor/lock/graph` | v0.16 |
| **Environments** | `up` `down` `env status/logs/exec/import/devcontainer emit` | v0.17, v0.19.7 (`import`), v0.21.0 (`devcontainer emit`) |
| **Functional testing** | `test` `test-discover` `verify` `contract check` | v0.18, v0.19 (`contract`) |
| **AI ecosystem** | `mcp serve` `export-agents` `context-build` | v0.19 |
| **Sessions** | `session capture/list/show/forget/distill` | v0.20 |

Plus:

- **9 Rust crates** in a workspace: `coral-cli`, `coral-core`, `coral-env`, `coral-test`, `coral-mcp`, `coral-runner`, `coral-lint`, `coral-stats`, `coral-session`.
- **5 LLM runner implementations** (`Claude`, `Gemini`, `Local` llama.cpp, `Http` OpenAI-compat, `Mock` for tests). API keys never appear in process argv (piped via stdin); request bodies never appear in argv either (per-call tempfile mode 0600 via RAII guard).
- **3 embeddings providers** (`Voyage`, `OpenAI`, `Anthropic`).
- **2 storage backends** (JSON default, SQLite via `CORAL_EMBEDDINGS_BACKEND=sqlite`).
- **11 structural lint checks** (incl. `unreviewed-distilled` v0.20 + `injection-suspected` v0.19.5 default-on since v0.20.2) + 1 LLM-driven semantic check + auto-fix routing.
- **5 export formats** for the wiki (`markdown-bundle`, `json`, `notion-json`, `jsonl`, `html`).
- **5 export formats** for AI agent instructions (`agents-md`, `claude-md`, `cursor-rules`, `copilot`, `llms-txt`) — manifest-driven, NOT LLM-driven.
- **3 user-reachable test kinds today** (`Healthcheck`, `UserDefined`, `MockTestRunner` for tests) **+ 6 reserved variants** (`LlmGenerated`, `Contract`, `PropertyBased`, `Recorded`, `Event`, `Trace`, `E2eBrowser`) on the `TestKind` enum for forward-compat. Only the first three are wired to a `TestRunner` impl; the reserved variants exist on the data type so the wire format stays stable when their runners ship.
- **8 MCP resources + 7 read-only tools (3 more behind `--allow-write-tools`) + 3 prompts** exposed via JSON-RPC 2.0 stdio. MCP `mimeType` matches actual payload per resource (catalog-driven). `.coral/audit.log` rotates at 16 MiB. Notification methods (no `id`) silently no-op per JSON-RPC 2.0 §4.1.
- **End-to-end concurrency safety**: atomic writes (`tmp + rename`), cross-process `flock(2)` locking, race-free parallel `coral ingest` AND `coral project sync`. `WikiLog::append_atomic` is race-free under contending writers (header+entry sequence cannot be reordered).
- **Hardened against adversarial inputs**: slug allowlist (`is_safe_filename_slug` + `is_safe_repo_name`) at every interpolation site; `--` separator before user-controlled positionals in every `git` invocation (CVE-2017-1000117 / CVE-2024-32004 family); 32 MiB cap on every `read_to_string` of user-supplied content; secret scrubbing in every `RunnerError` Display.
- **Backward-compat guarantee**: every v0.15 single-repo workflow keeps working — pinned by a dedicated `bc-regression` test job (6 fixtures) that runs on every PR.

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
cargo install --locked --git https://github.com/agustincbajo/Coral --tag v0.30.0 coral-cli
```

(Replace `v0.30.0` with the latest tag from the [Releases page](https://github.com/agustincbajo/Coral/releases).)

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

#### Windows — extra prereqs before `cargo build`

The default `rustup` host on Windows is `stable-x86_64-pc-windows-gnu`, which depends on `dlltool.exe` from MinGW-w64 binutils — and `dlltool.exe` is **not** shipped with the rustup toolchain. A fresh `cargo build` will fail with `error: error calling dlltool 'dlltool.exe': program not found`. Pick one of:

- **MSVC (recommended):** `rustup default stable-x86_64-pc-windows-msvc`, then install ["Build Tools for Visual Studio"](https://visualstudio.microsoft.com/downloads/#build-tools-for-visual-studio-2022) and tick the **Desktop development with C++** workload.
- **GNU:** install MinGW-w64 (e.g. `winget install MartinStorsjo.LLVM-MinGW`) so `dlltool.exe` lands on `PATH`.

Common gotcha: Git Bash's `C:\Program Files\Git\usr\bin\link.exe` (a coreutils tool) shadows MSVC's `link.exe` on `PATH` and breaks the MSVC linker with `link: extra operand …rcgu.o`. Reorder `PATH` so the MSVC `link.exe` wins, or run the build from a "x64 Native Tools Command Prompt for VS" shell.

### Pre-built binaries

Each tagged release ships pre-built binaries for x86_64 Linux, x86_64 macOS, and aarch64 macOS (Apple Silicon) on the [Releases page](https://github.com/agustincbajo/Coral/releases). Download `coral-vX.Y.Z-<target>.tar.gz`, verify the SHA-256, extract the `coral` binary, place it on your `$PATH`.

```bash
# Replace VERSION and TARGET with the values for the release you want; e.g.
#   VERSION=v0.30.0
#   TARGET=aarch64-apple-darwin   # x86_64-apple-darwin or x86_64-unknown-linux-gnu
VERSION=v0.30.0
TARGET=aarch64-apple-darwin
curl -L -o coral.tar.gz "https://github.com/agustincbajo/Coral/releases/download/${VERSION}/coral-${VERSION}-${TARGET}.tar.gz"
shasum -a 256 -c coral.tar.gz.sha256  # if you also downloaded the .sha256 sidecar
tar -xzf coral.tar.gz
sudo mv "coral-${VERSION}-${TARGET}/coral" /usr/local/bin/
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

### Live reload (`coral up --watch`, v0.21.2+)

Declare what to sync, rebuild, or restart on file changes:

```toml
[environments.services.api.watch]
rebuild      = ["./Dockerfile", "./go.sum"]
restart      = ["./config.yaml"]
initial_sync = true                       # compose ≥ 2.27 — fires once on attach

[[environments.services.api.watch.sync]]
path   = "./src"
target = "/app/src"

[[environments.services.api.watch.sync]]
path   = "./templates"
target = "/app/templates"
```

Then:

```bash
coral up --watch --env dev               # up -d --wait, then `compose watch` foreground until Ctrl-C
coral env watch --env dev                # alias for `coral up --watch`
```

`compose watch` streams sync events (`syncing X files to Y`, `rebuilding service Z`) to your terminal — same UX as `tilt up` or `skaffold dev`. Ctrl-C tears the watch subprocess down cleanly without killing the running containers (`coral down` does that). At least one service must declare `[services.<name>.watch]`; running `--watch` against a manifest with no watch blocks fails fast with an actionable error.

> **macOS caveat.** `compose watch` on macOS hits an upstream Docker fsevents flakiness — sometimes sync events stop firing after long sessions, or files on case-sensitive volumes are ignored. Tracked at [docker/for-mac#7832](https://github.com/docker/for-mac/issues/7832). Coral emits a one-line `WARNING:` to stderr on macOS so the issue is never silent. Workaround when sync stalls: restart Docker Desktop.

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

## Quickstart — capture and distill agent sessions

**Shipped in v0.20.0** ([#16](https://github.com/agustincbajo/Coral/issues/16)). Coral can now fold the conversations that produced your wiki *back into* the wiki — agent transcripts (Claude Code today; Cursor and ChatGPT tracked) become curated synthesis pages. The flow is opt-in at every step and gated by the same trust-by-curation contract that governs `coral test generate` output.

```bash
# 1. Capture the most-recent Claude Code session whose `cwd` matches this repo.
#    Privacy scrubber is on by default — API keys, JWTs, AWS creds, etc.
#    are replaced with [REDACTED:<kind>] markers before bytes hit disk.
coral session capture --from claude-code
# captured 5c359daf-… (412 messages, 7 redactions)
#   → .coral/sessions/2026-05-08_claude-code_a1b2c3d4.jsonl

# 2. Inspect captures.
coral session list
coral session show 5c359daf

# 3. Distill into wiki-shaped synthesis pages (one LLM call).
#    Pages always land as `reviewed: false` — `coral lint` blocks the commit
#    until a human flips the flag.
coral session distill 5c359daf --apply
# → .coral/sessions/distilled/<slug>.md      (always)
# → .wiki/synthesis/<slug>.md                (with --apply, also reviewed: false)

# 4. Review the page in your editor, flip `reviewed: true`, commit.
$EDITOR .wiki/synthesis/<slug>.md

# 5. (Optional) drop the raw transcript once curated.
coral session forget 5c359daf --yes
```

Storage layout: raw `.jsonl` and `index.json` are gitignored (added to `.gitignore` automatically by `coral init`); curated `.wiki/synthesis/*.md` ships in git. The `.coral/sessions/distilled/` mirror is also gitignored — it's a holding cell, not the canonical wiki.

Privacy posture and the full design-question rationale live in [docs/SESSIONS.md](docs/SESSIONS.md). TL;DR:

- Scrubber is on by default. Opt-out requires both `--no-scrub` AND `--yes-i-really-mean-it`.
- Distilled pages always carry `reviewed: false`. `coral lint --rule unreviewed-distilled` raises Critical and the bundled pre-commit hook blocks the commit.
- Cross-format support is staged: Claude Code first; `--from cursor` and `--from chatgpt` exist as CLI flags but currently emit a clear "not yet implemented; track #16" error.

### Patch mode (`--as-patch`, v0.21.3+)

Default `coral session distill <id>` is **option (a) / page-emit**: 1–3 NEW synthesis pages land under `.coral/sessions/distilled/<slug>.md` (and at `.wiki/synthesis/<slug>.md` with `--apply`). When the session's insight is a small **edit to an EXISTING page** rather than a whole new page, that's the wrong shape.

v0.21.3 adds an opt-in `--as-patch` flag — **option (b) / patch-emit**. Instead of synthesis pages, the LLM proposes 1–N **unified-diff patches** against existing `.wiki/<slug>.md` pages.

```bash
# 1. Capture as before.
coral session capture --from claude-code

# 2. Patch-emit. Top-K=10 BM25-ranked candidate pages from .wiki/ are
#    surfaced in the prompt by default; tune with --candidates N (or 0
#    to skip candidate collection entirely).
coral session distill 5c359daf --as-patch --candidates 10
# distilled 5c359daf… → 2 patch(es):
#   0. modules/authentication: The session revealed JWT refresh uses sliding window
#   1. modules/rate-limit: Per-tenant counters, not global
# written:
#   - .coral/sessions/patches/5c359daf-0.patch
#   - .coral/sessions/patches/5c359daf-0.json
#   - .coral/sessions/patches/5c359daf-1.patch
#   - .coral/sessions/patches/5c359daf-1.json

# 3. Review each .patch by eye, OR pre-validate with --apply.
coral session distill 5c359daf --as-patch --apply
# applied:
#   - .wiki/modules/authentication.md (reviewed: false)
#   - .wiki/modules/rate-limit.md (reviewed: false)
```

**Validation pipeline** — every patch passes through this gauntlet BEFORE any file lands:

1. **Slug allow-list.** Each `/`-separated component of `target_slug` must pass `is_safe_filename_slug` (kebab/snake-case ASCII, no `..`, no leading `.`, no shell metacharacters).
2. **Wiki existence.** The resolved page MUST exist in `list_page_paths(.wiki)`. Patches against non-existent pages reject at parse time.
3. **Diff header agreement.** The `--- a/<X>.md` and `+++ b/<X>.md` headers must agree with `target_slug`. Mismatches reject at parse time.
4. **`git apply --check`.** Every patch is dry-run-validated against `project_root` via `git apply --check --unsafe-paths --directory=.wiki <patch>`. (`--unsafe-paths` permits paths outside the index — NOT untrusted paths. By the time we shell out, the slug is already allow-list-validated.)

**Pre-apply atomicity** — if ANY patch in the set fails its check, NO files are written and the command exits non-zero with the patch index + git stderr verbatim. This is the same all-or-nothing contract option (a) has always provided.

**Sidecar `.json` shape:**

```json
{
  "target_slug": "modules/authentication",
  "rationale": "The session revealed JWT refresh uses a sliding window…",
  "prompt_version": 2,
  "runner_name": "claude",
  "session_id": "5c359daf-…",
  "captured_at": "2026-05-08T10:00:00+00:00",
  "reviewed": false
}
```

**`--apply` semantics** — Coral OWNS the `reviewed: false` flip. After each `git apply` succeeds, Coral re-reads the touched page, sets `frontmatter.extra["reviewed"] = false`, and re-writes. The LLM's job is body content; the trust gate is Coral's job. `coral lint --rule unreviewed-distilled` then blocks the commit until a human flips it.

**Default vs. patch mode in one line:** if the LLM has something *new* to say (a clarifying paragraph, a counter-intuitive finding, an architectural note that didn't exist before) → page mode. If the LLM has a small surgical fix (a corrected line, an added caveat, a clarified sentence) → patch mode.

**`forget` cleanup** — `coral session forget <id>` sweeps both `distilled_outputs` (page-mode artifacts) AND `patch_outputs` (patch-mode artifacts) from `.coral/sessions/`. **`.wiki/` mutations from `--apply --as-patch` are NOT undone** — distill-as-patch's apply is one-way (the user owns the wiki post-apply).

---

## Cookbook — common workflows

Real-world recipes that show how the layers compose. Each one is copy-paste-ready — every command has been exercised against the test suite or in dogfooding.

### Recipe 1 — Stand up a new microservices project from zero

You have nothing. You want a multi-repo project with wiki, dev environment, and smoke tests.

```bash
mkdir orchestra && cd orchestra
git init -q && git commit --allow-empty -qm "init"

# 1. Multi-repo manifest
coral project new orchestra
coral project add api    --remote github --tags service,team:platform
coral project add worker --remote github --tags service,team:data
coral project add shared --remote github --tags library

# 2. Resolve every repo's URL via the [remotes.github] template,
#    parallel-clone, write coral.lock with resolved SHAs.
coral project sync

# 3. Aggregated wiki — Coral compiles a Markdown page per concept
#    cross-repo. Slugs become `<repo>/<slug>` automatically.
coral bootstrap --apply

# 4. Verify everything's coherent
coral lint --severity critical          # exits 0 → ready to ship
coral status --format markdown          # daily-use dashboard
```

After this you have `coral.toml` + `coral.lock` + `repos/{api,worker,shared}/` + `.wiki/` + `.coral/`. Commit them all (`.gitignore` for `repos/` if you don't want to vendor — `coral project sync` re-clones on demand).

### Recipe 2 — Migrate from raw `docker-compose.yml`

You already have a `docker-compose.yml` and don't want to author the `[[environments]]` block from scratch.

```bash
coral env import docker-compose.yml > /tmp/imported.toml
# Review the output. Things Coral couldn't translate cleanly land as
# `# TODO:` comments — addresses long-form depends_on, list-form
# environment, port ranges, extends/profiles/volumes/networks.

# Paste the contents into your coral.toml as a top-level [[environments]]
# block. Then bring it up:
coral up --env dev
coral verify                            # runs the imported healthchecks
```

The importer is **conservative + advisory**. Only fields that round-trip cleanly through `EnvironmentSpec` are emitted. Heuristics infer `kind = "http"` from `CMD ["curl", "-f", "http://.../health"]` patterns and `kind = "exec"` from arbitrary `CMD-SHELL` lines via `sh -c`. Compose duration strings (`5s`, `1m30s`, `2h`) parse to seconds.

### Recipe 3 — Onboard a new contributor in 30 seconds

The wiki is the persistent memory. Use it.

```bash
# 1. Generate a personalized reading path. The runner picks 5–10 pages
#    in dependency order based on the profile.
coral onboard --profile backend --apply

# 2. Or, agent-friendly: dump a curated context-budgeted bundle.
coral context-build --query "how does the auth flow work" --budget 80000 > context.md
# Paste context.md into Claude Code, Cursor, ChatGPT, anything with a
# context window — the bundle sorts by (confidence desc, length asc)
# under the budget cap. No LLM was invoked to assemble it.

# 3. For a structured first day:
coral export-agents --format claude-md --write    # CLAUDE.md
coral export-agents --format cursor-rules --write # .cursor/rules/coral.mdc
coral export-agents --format llms-txt --write     # llms.txt
```

The agent-instruction files are **manifest-driven, not LLM-driven** — they render deterministically from `[project]`, `[[repos]]`, `[hooks]` (when present) so re-running produces byte-identical output. Empirical context-engineering work (incl. [Anthropic's published guidance](https://www.anthropic.com/engineering/context-engineering)) has consistently found LLM-synthesised AGENTS.md files degrade agent task success vs. deterministic templates.

### Recipe 4 — Cross-repo contract testing

Catch interface drift between a provider's `openapi.yaml` and a consumer's `.coral/tests/` BEFORE the test environment even comes up.

```bash
# Setup: provider repo declares its API; consumer repo declares its
# expectations as test fixtures.
echo '
openapi: 3.0.0
info: { title: api, version: 1.0 }
paths:
  /users:
    get:
      responses: { "200": { description: ok } }
' > repos/api/openapi.yaml

mkdir -p repos/worker/.coral/tests
echo '
name: worker-against-api
service: worker
steps:
  - http: GET /users
    expect: { status: 200 }
' > repos/worker/.coral/tests/api.yaml

# Drift detection — deterministic, no LLM. Use --strict to gate CI.
coral contract check --strict --format json > contract-report.json
# exit 0 → consumer + provider agree
# exit non-zero → drift report with structured findings
```

`coral contract check` walks every `[[repos]] depends_on` edge, parses the upstream's OpenAPI spec, and diffs against every `.coral/tests/**` reference (yaml + hurl). Findings include `UnknownEndpoint`, `UnknownMethod`, `StatusDrift`, `MissingProviderSpec`, `MalformedProviderSpec`. Generates the same JSON shape as `coral test --format junit` so existing CI reporters can consume it.

### Recipe 5 — Continuous wiki maintenance with `coral ingest`

Wire `coral ingest` into your post-commit / post-merge workflow so the wiki stays current automatically.

```bash
# In CI (.github/workflows/ingest.yml):
- name: Update Coral wiki
  run: |
    coral ingest --apply --severity warning  # idempotent; uses last_commit
    if [ -n "$(git status --porcelain .wiki/)" ]; then
      git config user.name  "coral-bot"
      git config user.email "coral-bot@example.com"
      git add .wiki/
      git commit -m "chore(wiki): coral ingest"
      git push
    fi
```

Or with `--affected` for sub-repo selectivity in multi-repo projects:

```bash
coral ingest --affected --since main~10 --apply
# Only repos whose tip changed since main~10 are re-ingested,
# DFS-walking depends_on so downstream consumers also refresh.
```

### Recipe 6 — Hardened production posture

If your wiki is committed to a public repo and accepts external PRs, lock it down.

```bash
# 1. Reject prompt-injection patterns at lint time. The scan is
#    **on by default since v0.20.2** — keep it that way (or pass
#    `--no-check-injection` only if you have a parallel mitigation).
coral lint --severity warning
# Detects `<|system|>`, `</system>`, base64 runs >100 chars, unicode
# bidi (U+202E) and tag chars (U+E0000–U+E007F), confidence-drop
# instruction patterns.

# 2. Tag every repo with a trust_level (manifest field, planned for
#    v0.20+). Until then, gate `coral query` with --strict so cross-repo
#    citations are required.

# 3. Run `coral project doctor` on every PR.
coral project doctor --format json
# Checks: clones present, ref drift, uncommitted changes, lockfile
# staleness, auth setup per remote.

# 4. The CI workflow already runs cargo audit + cargo deny.
#    Add a step that re-asserts no dependencies bring in unsafe
#    licenses or known CVEs.
```

### Recipe 7 — Connect Coral to a coding agent

See the next section ([MCP client integration](#mcp-client-integration)) for vendor-specific configuration. Quick taste:

```bash
# Boot Coral as an MCP server (stdio transport, read-only by default).
coral mcp serve --transport stdio &

# Need wider tool access? `--read-only` is on by default; the 5 read-only
# tools (query, search, find_backlinks, affected_repos, verify) are
# advertised in `tools/list` and dispatched in `tools/call` regardless.
# To unlock the 3 write tools (run_test, up, down) — both in the catalog
# AND in the dispatcher — pass `--allow-write-tools`. Pre-v0.20.2 the
# behaviour drifted: `--read-only false` alone listed write tools but
# the dispatcher then rejected calls; v0.20.2 #38 collapses both
# surfaces onto `--allow-write-tools`.
coral mcp serve --transport stdio --allow-write-tools &
```

> **Transport status (v0.21.1+).** Both `--transport stdio` (the default — every shipped MCP client speaks it) and `--transport http --port <p>` (Streamable HTTP per MCP 2025-11-25) ship. HTTP defaults to binding `127.0.0.1` and validates `Origin` against `null` / `http://localhost*` / `http://127.0.0.1*` only — a DNS-rebinding mitigation. `--bind 0.0.0.0` is opt-in and emits a stderr warning banner. See [Security model for the HTTP transport](#security-model-for-the-http-transport) below for the full threat model.

Test the boot manually:

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"resources/list","params":{}}' | coral mcp serve --transport stdio
# → {"jsonrpc":"2.0","id":1,"result":{"resources":[...6 catalog URIs...]}}
```

---

## MCP client integration

Coral speaks Model Context Protocol 2025-11-25. Below are copy-paste configs for the common clients.

### Claude Code (`claude` CLI)

Edit `~/.claude/settings.json` (or `.claude/settings.json` in the project root):

```json
{
  "mcpServers": {
    "coral": {
      "command": "coral",
      "args": ["mcp", "serve", "--transport", "stdio"],
      "env": {
        "RUST_LOG": "coral_mcp=info"
      }
    }
  }
}
```

After restart, Claude Code can read all 8 resources — `coral://manifest`, `coral://lock`, `coral://graph`, `coral://wiki/<repo>/<slug>`, `coral://wiki/_index`, `coral://stats`, `coral://test-report/latest`, `coral://contracts`, `coral://coverage` — and call the 7 read-only tools (`query`, `search`, `find_backlinks`, `affected_repos`, `verify`, `list_interfaces`, `contract_status`).

To enable write tools (`run_test` plus 2 more, gated):

```json
"args": ["mcp", "serve", "--transport", "stdio", "--allow-write-tools"]
```

Every write-tool invocation is logged to `.coral/audit.log` (rotates at 16 MiB).

### Cursor

In Cursor's MCP settings (Cmd+, → MCP Servers):

```json
{
  "name": "coral",
  "command": "coral mcp serve --transport stdio",
  "cwd": "/absolute/path/to/your/project"
}
```

Same resource + tool catalog as Claude Code.

### Continue

`~/.continue/config.yaml`:

```yaml
mcpServers:
  - name: coral
    command: coral
    args:
      - mcp
      - serve
      - --transport
      - stdio
```

### Cline

Cline reads `.cline/mcp.json`:

```json
{
  "mcpServers": {
    "coral": {
      "command": "coral",
      "args": ["mcp", "serve", "--transport", "stdio"]
    }
  }
}
```

### Goose

`~/.config/goose/config.yaml`:

```yaml
extensions:
  coral:
    type: stdio
    cmd: coral mcp serve --transport stdio
```

### Generic JSON-RPC over stdio

For any client that speaks raw MCP JSON-RPC, `coral mcp serve --transport stdio` is the entry point. The server announces `protocolVersion: "2025-11-25"` in the `initialize` handshake and responds to `resources/list`, `resources/read`, `tools/list`, `tools/call`, `prompts/list`, `prompts/get`. Notifications (no `id` field) silently no-op per spec §4.1.

### HTTP/SSE transport (v0.21.1+)

Streamable HTTP per the MCP 2025-11-25 spec. Three endpoints under `/mcp`:

| Method | Body / required headers | Server response |
|---|---|---|
| `POST /mcp` | JSON-RPC envelope + `Content-Type: application/json` + `Accept: application/json, text/event-stream` | `200 application/json` for single-answer; `204` for notification (no `id`) |
| `GET /mcp` | `Accept: text/event-stream` | `200 text/event-stream` empty stream + `: keep-alive\n\n` heartbeat every 15s |
| `DELETE /mcp` | `Mcp-Session-Id: <id>` | `204` if session existed; `404` otherwise |
| `OPTIONS /mcp` | (CORS preflight) | `200` with `Access-Control-Allow-Methods: POST, GET, DELETE, OPTIONS` |

Worked example (initialize):

```bash
coral mcp serve --transport http --port 3737 &
curl -sS -X POST -H "Content-Type: application/json" \
  -H "Accept: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
  http://127.0.0.1:3737/mcp
# → {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{...},"serverInfo":{"name":"coral","version":"0.21.1"}}}
# Response also carries: Mcp-Session-Id: <uuid-shaped opaque cookie>
```

Echo the `Mcp-Session-Id` cookie on subsequent POSTs so the server can correlate the conversation:

```bash
curl -sS -X POST -H "Content-Type: application/json" \
  -H "Accept: application/json" \
  -H "Mcp-Session-Id: <uuid from initialize>" \
  -d '{"jsonrpc":"2.0","id":2,"method":"resources/list","params":{}}' \
  http://127.0.0.1:3737/mcp
```

Tear down explicitly with `DELETE`:

```bash
curl -sS -X DELETE \
  -H "Mcp-Session-Id: <uuid>" \
  http://127.0.0.1:3737/mcp
# → 204 No Content (session removed); subsequent DELETE on the same id returns 404.
```

Default port is `3737`. `--port 0` asks the OS to pick a free port; the resolved port is logged to stderr (`coral mcp serve — listening on http://127.0.0.1:NNNNN/mcp`).

### Security model for the HTTP transport

The HTTP transport is intended for **localhost-only** use. The defense in depth, in priority order:

1. **Default bind is `127.0.0.1`.** Other hosts on the local network can't reach the server. `--bind 0.0.0.0` is opt-in and emits a `WARNING:` stderr banner so a server bound to every interface is never silent.
2. **Origin allowlist.** Browser clients send `Origin`; we accept only `null` (file://), `http://localhost[:port]`, `http://127.0.0.1[:port]`, and `http://[::1][:port]`. Anything else returns 403. This is the spec's DNS-rebinding mitigation: an attacker who tricks a browser into pointing a malicious DNS record at `127.0.0.1` can't bypass the Origin check because the page itself was loaded from the attacker's host.
3. **Body cap (4 MiB) → 413.** Cap is the hard maximum; legit MCP envelopes fit in well under 100 KiB.
4. **Concurrency cap (32 in-flight requests) → 503.** Keeps the per-process FD budget bounded.
5. **Batched JSON-RPC arrays → 400.** v0.21.1 doesn't support batching yet; we reject up front so a client that mistakenly sends a batch gets a clear error.
6. **Session ID is opaque, not authentication.** `Mcp-Session-Id` is a correlation token — it doesn't grant access. Anyone who can connect to the port can mint a session via `initialize`. Authentication over the HTTP transport is a future feature; for now the localhost default is the protection.

What this **does NOT defend against**:

- A native client (any non-browser) can spoof `Origin` trivially. The 127.0.0.1 default is the load-bearing protection — exposing the server to a network you don't control turns it into an exfiltration vector.
- A multi-tenant Linux box where another local user can connect to your `127.0.0.1` listener. Linux's loopback is shared across UIDs.
- A compromised `coral` binary. Verify SHA-256 against the GitHub release.

If you need network-reachable MCP access, terminate TLS at a reverse proxy (nginx, Caddy, Cloudflare Tunnel) and require client authentication there — Coral's HTTP transport is not a public-internet-facing service.

### Resources catalog

| URI | mimeType | Content |
|---|---|---|
| `coral://manifest` | `application/json` | `coral.toml` parsed to JSON |
| `coral://lock` | `application/json` | `coral.lock` with resolved SHAs |
| `coral://stats` | `application/json` | `StatsReport` (page count, avg confidence, orphans) |
| `coral://wiki/_index` | `application/json` | aggregated slug list cross-repo |
| `coral://wiki/<repo>/_index` | `application/json` | per-repo slug list (`_default` = aggregate, see below) |
| `coral://wiki/<repo>/<slug>` | `application/json` | `{slug, type, status, confidence, last_updated_commit, sources, backlinks, body}` |

Each per-page resource is JSON not raw markdown — clients render the body field as markdown if they want to.

> **`_default` is a reserved repo-name sentinel.** `coral://wiki/_default/_index` returns the aggregated cross-repo slug list (it is the same payload as `coral://wiki/_index`, kept for backwards compatibility with single-repo clients). Because of this, `_default` cannot be used as a real repo name in `coral.toml` — `Project::validate()` rejects `[[repos]] name = "_default"` with a message naming the reservation. The legacy single-repo case (no `[[repos]]` blocks) is rendered under `_default` automatically.

### Tools catalog

Default (`--read-only`, the default):
- `query` — LLM-backed Q&A over the wiki (returns `Skip` if no `ANTHROPIC_API_KEY` / `GEMINI_API_KEY` is set; the agent should fall back to direct resource reads).
- `search` — TF-IDF + optional Voyage embeddings, no LLM.
- `find_backlinks` — list every page that wikilinks to the given slug.
- `affected_repos` — DFS over `depends_on` since a given SHA.
- `verify` — run `coral verify` (liveness healthchecks, <30s).

Enabled with `--allow-write-tools`:
- `run_test` — invoke `coral test` on a specific case ID. Logged to `.coral/audit.log`.
- (2 more reserved for v0.20+).

### Prompts catalog

- `prompts/onboard?profile=<name>` — "you are a new dev with this profile; here are the pages to read in this order".
- `prompts/cross-repo-trace?flow=<name>` — "explain this flow walking pages cross-repo".
- `prompts/code-review?repo=<name>&pr=<n>` — "review this PR against the wiki".

---

## Output examples

What each command actually prints. Pasted from real runs against a tmpdir fixture.

### `coral status`

```
# Wiki status

- Wiki: `.wiki`
- Last commit: `8823181`
- Pages: 24
- Lint: Critical: 0 | Warning: 2 | Info: 1
- Stats: 24 pages, avg confidence 0.83, 0 orphan candidate(s)

## Recent log

- 2026-05-04T17:02:11+00:00 ingest: 3 pages updated
- 2026-05-04T16:45:33+00:00 lint: 0 critical, 2 warning
- 2026-05-04T15:12:08+00:00 consolidate: 1 page retired, 1 merged
- 2026-05-04T13:48:55+00:00 bootstrap: 24 pages compiled
- 2026-05-04T13:42:10+00:00 init: wiki created
```

JSON:

```json
{
  "wiki": ".wiki",
  "last_commit": "8823181",
  "pages": 24,
  "lint": { "critical": 0, "warning": 2, "info": 1 },
  "stats": { "total_pages": 24, "confidence_avg": 0.83, "orphan_candidates": 0 },
  "recent_log": [
    { "timestamp": "2026-05-04T17:02:11+00:00", "op": "ingest", "summary": "3 pages updated" }
  ]
}
```

### `coral lint`

```
# Lint report

24 pages, 3 issues (0 critical, 2 warning, 1 info)

| severity | rule | page | message |
|----------|------|------|---------|
| ⚠ | high-confidence-without-sources | api/auth-flow | confidence=0.95 but sources is empty |
| ⚠ | broken-wikilink | worker/order | links to `[[non-existent]]` |
| ℹ | low-confidence | shared/migration-2026-04 | confidence=0.4; consider --consolidate |
```

Exit codes: 0 (no issues at the requested severity), 1 (issues found ≥ severity).

### `coral verify`

```
✔ db        Healthy   (interval=5s, retries=6, start_period=20s)
✔ api       Healthy   (interval=2s, retries=5, start_period=30s)
✔ worker    Healthy   (interval=5s, retries=3, start_period=15s)

3/3 services healthy in 4.2s
```

Exit 0 if every healthcheck passes, non-zero otherwise. JSON variant gives a structured report consumable by GitHub Actions reporters.

### `coral test`

```
# Test report

| status | service | case | duration |
|--------|---------|------|----------|
| ✔ | api | smoke-users-list | 0.12s |
| ✔ | api | smoke-users-create | 0.34s |
| ✔ | worker | integration-order-flow | 1.8s |
| ✘ | worker | integration-cancel | 0.5s — expected 200, got 404 |

3/4 passing in 2.76s
```

JUnit XML variant:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<testsuites tests="4" failures="1" time="2.76">
  <testsuite name="api" tests="2" failures="0" time="0.46">
    <testcase classname="api" name="smoke-users-list" time="0.12"/>
    <testcase classname="api" name="smoke-users-create" time="0.34"/>
  </testsuite>
  <testsuite name="worker" tests="2" failures="1" time="2.30">
    <testcase classname="worker" name="integration-order-flow" time="1.8"/>
    <testcase classname="worker" name="integration-cancel" time="0.5">
      <failure>expected 200, got 404</failure>
    </testcase>
  </testsuite>
</testsuites>
```

### `coral query`

```
$ coral query "how does worker authenticate against api?"

# how does worker authenticate against api?

The worker authenticates via JWT bearer token. Specifically:

1. Worker reads `WORKER_API_TOKEN` from env at startup (api/auth-flow §2).
2. Each outbound request to `api` includes `Authorization: Bearer <token>`
   in the header (worker/http-client §1).
3. The api validates against `JWT_SECRET` (api/auth-flow §3).
4. Token rotation is handled by `shared/secrets-rotator` on a 24h cycle.

## Sources

- [api/auth-flow](.wiki/concepts/api/auth-flow.md) §2 §3
- [worker/http-client](.wiki/modules/worker/http-client.md) §1
- [shared/secrets-rotator](.wiki/modules/shared/secrets-rotator.md)

(confidence: 0.91, runner: claude-sonnet-4-5)
```

`coral query --strict` requires every claim to cite a slug; without it the runner is allowed (and instructed) to admit "I don't know" rather than hallucinate.

### `coral env status`

```
| service | state    | health  | restarts | published ports |
|---------|----------|---------|----------|-----------------|
| api     | Running  | Pass    | 0        | 3000->3000      |
| worker  | Running  | Pass    | 0        | —               |
| db      | Running  | Pass    | 0        | 5432->5432      |
```

### `coral project graph --format mermaid`

```
graph TD
  api
  worker --> api
  shared
  api --> shared
  worker --> shared
```

GitHub renders this directly in the README. `--format dot` for Graphviz, `--format json` for tooling.

### `coral contract check`

```
# Contract drift report

3 finding(s) (1 error, 2 warning):

| severity | consumer | provider | message |
|----------|----------|----------|---------|
| ✘ | worker | api | unknown endpoint: GET /users/{id}/permissions (provider's openapi.yaml does not declare this path) |
| ⚠ | worker | api | status drift: GET /users expects 200; provider documents [201, 400] |
| ⚠ | analytics | shared | provider 'shared' has openapi spec but it failed to parse: yaml line 12 unexpected key |
```

Exit 0 by default; `--strict` raises warnings to errors and exits non-zero.

---

## Subcommand reference

### Wiki layer (v0.15+)

| Command | Purpose | Needs LLM? |
|---|---|---|
| `coral init [--force]` | Scaffold `.wiki/` with SCHEMA, index, log, and 9 type subdirs. Idempotent. | No |
| `coral bootstrap [--apply]` | First-time wiki compilation from `HEAD`. `--dry-run` (default) prints plan; `--apply` writes pages. | Yes |
| `coral ingest [--from SHA] [--apply]` | Incremental update from `last_commit`. Same dry-run / apply semantics. | Yes |
| `coral query <q>` | Streamed answer using the wiki as context. Cites slugs. | Yes |
| `coral lint [--structural\|--semantic\|--all] [--fix] [--rule R] [--no-check-injection]` | 11 structural + 1 LLM semantic check, optional auto-fix. The `injection-suspected` scan is on by default since v0.20.2; pass `--no-check-injection` to suppress. Exit 1 on critical. | Optional |
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
| `coral up [--env NAME] [--service NAME]... [--detach] [--build] [--watch]` | `EnvBackend::up`. Default `--detach=true`. Compose backend renders `.coral/env/compose/<hash>.yml`. `--watch` (v0.21.2+) runs `compose watch` foreground after `up -d --wait` succeeds — see "Live reload" above. |
| `coral down [--env] [--volumes] [--yes]` | Tear down. `--yes` required when `production = true`. |
| `coral env status [--env] [--format markdown\|json]` | Live service state from `EnvBackend::status()`. |
| `coral env logs <service> [--env] [--tail N]` | Read recent logs (compose `logs --no-color --no-log-prefix --timestamps`). |
| `coral env exec <service> [--env] -- <cmd>...` | One-shot exec inside a container. Exit code propagates. |
| `coral env import <compose.yml> [--env NAME] [--write] [--out PATH]` | (v0.19.7+) Convert an existing `docker-compose.yml` into a starter `[[environments]]` block for `coral.toml`. Conservative + advisory: emits only fields that round-trip through `EnvironmentSpec`; everything else surfaces as a `# TODO:` comment. Heuristic infers `kind = "http"` from `CMD curl URL` patterns. |
| `coral env devcontainer emit [--env NAME] [--service NAME] [--write] [--out PATH]` | (v0.21.0+) Render a `.devcontainer/devcontainer.json` from the active `[[environments]]` block so VS Code / Cursor / GitHub Codespaces can attach to the same Compose project Coral runs. Pure offline emit: `forwardPorts` from `RealService.ports`, `dockerComposeFile` points at `../.coral/env/compose/<hash>.yml`. `--service` overrides the auto-selection (first real service with `repo = "..."`, fallback alphabetic). `--write` lands the file atomically at `<project_root>/.devcontainer/devcontainer.json`. |
| `coral env watch [--env NAME] [--service NAME]... [--build]` | (v0.21.2+) Alias for `coral up --watch`. Runs `compose watch` foreground until Ctrl-C. |

### Functional testing layer (v0.18+)

| Command | Purpose |
|---|---|
| `coral verify [--env NAME]` | Run all healthchecks. Liveness only, <30s budget. Exit non-zero on any fail. |
| `coral test [--service N]... [--kind smoke\|healthcheck\|user-defined]... [--tag T]... [--format markdown\|json\|junit] [--update-snapshots] [--include-discovered] [--env]` | Run union of healthcheck + user-defined YAML + Hurl + optional OpenAPI-discovered cases. JUnit XML for CI. |
| `coral test-discover [--emit markdown\|yaml] [--commit]` | Auto-generate TestCases from `openapi.{yaml,yml,json}` in repos. **No LLM**, deterministic mapping. |
| `coral contract check [--format markdown\|json] [--strict]` | **Cross-repo interface drift detection.** Walks each repo's OpenAPI spec (provider) and `.coral/tests/*.{yaml,yml,hurl}` (consumer); for every `[[repos]] depends_on` edge reports unknown endpoints, unknown methods, and status-code drift. Fails fast in CI **before** the test environment is even brought up. |

### AI ecosystem layer (v0.19+)

| Command | Purpose | Needs LLM? |
|---|---|---|
| `coral mcp serve [--transport stdio\|http] [--port N] [--bind ADDR] [--read-only true\|false] [--allow-write-tools]` | MCP server (JSON-RPC 2.0, MCP 2025-11-25). Exposes 6 resources, 3 prompts, and 5 read-only tools (`query`, `search`, `find_backlinks`, `affected_repos`, `verify`); the 3 write tools (`run_test`, `up`, `down`) require `--allow-write-tools`. Read-only by default — pass `--read-only false` to disable. **v0.21.1+ ships both `--transport stdio` (default) and `--transport http`** (Streamable HTTP per the spec; default port 3737, default bind `127.0.0.1`, see [Security model](#security-model-for-the-http-transport)). | No |
| `coral export-agents --format <agents-md\|claude-md\|cursor-rules\|copilot\|llms-txt> [--write] [--out PATH]` | Manifest-driven instruction file emission. **NOT LLM-driven** — see [Anthropic's context-engineering guidance](https://www.anthropic.com/engineering/context-engineering) for why deterministic templates beat synthesized ones. | No |
| `coral context-build --query <q> --budget <tokens> [--format markdown\|json] [--seeds N]` | Smart context loader. TF-IDF rank + backlink BFS + greedy fill under token budget. | No |

### Sessions layer (v0.20+)

`coral session` is the umbrella for capturing and curating agent transcripts. The privacy scrubber runs on capture by default (opt-out is a hard `--no-scrub --yes-i-really-mean-it` two-flag combo); distilled output lands as `reviewed: false` so the `unreviewed-distilled` lint check + pre-commit hook gate it out of the canonical wiki until a human flips the flag. See [`docs/SESSIONS.md`](./docs/SESSIONS.md) for the full PRD-derived design notes.

| Command | Purpose | Needs LLM? |
|---|---|---|
| `coral session capture <path> [--source claude-code] [--no-scrub --yes-i-really-mean-it]` | Copy an agent transcript into `.coral/sessions/<date>_<source>_<sha8>.jsonl` and update `.coral/sessions/index.json`. Runs the privacy scrubber by default. | No |
| `coral session list [--format markdown\|json]` | Tabular view of captured sessions with redaction counts and a `distilled: yes/no` column. | No |
| `coral session show <id> [--messages] [--limit N]` | Inspect a captured session — metadata + first/last N messages. | No |
| `coral session distill <id> [--apply] [--model MODEL]` | Single-pass `Runner::run` that emits 1–3 wiki findings per session under `.coral/sessions/distilled/<slug>.md`. With `--apply`, also writes to `.wiki/synthesis/<slug>.md` with `reviewed: false` frontmatter. The `unreviewed-distilled` lint blocks commits until a human flips the flag (qualified — see v0.20.2 H2: only fires for pages that carry both `reviewed: false` AND a populated `source.runner` field). | Yes |
| `coral session distill <id> --as-patch [--apply] [--candidates N] [--model MODEL]` | **v0.21.3.** Option (b) / patch-emit. Instead of new pages, propose 1–N **unified-diff patches** against existing `.wiki/<slug>.md` pages. Patches save to `.coral/sessions/patches/<id>-<idx>.patch` plus a sidecar `.json`. With `--apply`, each patch is `git apply`-ed and the touched page's frontmatter is rewritten so `reviewed: false`. Pre-apply atomicity: if any patch fails `git apply --check`, no files are written. `--candidates N` (default 10) controls how many BM25-ranked candidate pages are surfaced in the prompt; `--candidates 0` skips candidate collection. | Yes |
| `coral session forget <id>` | Delete the raw transcript, every distilled output, the optional `--apply` mirror under `.wiki/synthesis/`, every `.patch`/`.json` recorded in `patch_outputs` under `.coral/sessions/patches/`, and the matching index entry. v0.21.3+: also sweeps the patch-emit artifacts. **`.wiki/` mutations from `--apply --as-patch` are NOT undone** — that flow is one-way. | No |

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

## Comparison vs adjacent tools

Coral occupies a specific niche: **the project manifest for multi-repo AI-augmented dev**. The closest neighbors solve overlapping problems differently. The honest take:

| Tool | Coral position |
|---|---|
| **Devin Wiki** ([Cognition](https://www.cognition.ai/)) | Closest functional match. Devin's wiki is proprietary, hosted, and tightly bound to Devin the agent. **Coral is open source, locally runnable, and consumable from any MCP-speaking agent.** Same "structured wiki on top of a codebase" thesis, two opposite distribution models. |
| **deepwiki-open** ([AsyncFuncAI](https://github.com/AsyncFuncAI/deepwiki-open)) | Python + Docker + ChromaDB-backed RAG service. Single-repo focus. **Coral is single-binary Rust, manifest-driven, multi-repo-first, no vector DB by default** (TF-IDF baseline; Voyage embeddings opt-in). |
| **OpenDeepWiki** ([AIDotNet](https://github.com/AIDotNet/OpenDeepWiki)) | .NET + TypeScript wiki UI on top of an LLM-generated knowledge base. Per-repo. **Coral has no UI; the wiki is plain Markdown on disk, diff-able in git, edited in your editor of choice.** No frontend dependency. |
| **Cursor multi-root workspaces** ([3.2+](https://www.cursor.com/changelog)) | Per-IDE-session multi-root view. Not persisted across sessions. **Coral's manifest is committed to git; survives IDE restarts; consumable cross-tool.** Cursor multi-root and Coral are complementary — Cursor is the editor view, Coral is the on-disk truth. |
| **Aider repo-map** ([paul-gauthier/aider](https://github.com/paul-gauthier/aider)) | In-session syntactic map (regenerated each run). Per-language, no semantic curation. **Coral is cross-session, semantic, curated by an LLM librarian under a strict schema.** Different abstraction layer. |
| **`AGENTS.md` / `CLAUDE.md` files** | Hand-written single-file conventions. Drift constantly with the code. **Coral renders these deterministically from `coral.toml` via `coral export-agents`** so you never hand-author them. Per [Anthropic's guidance](https://www.anthropic.com/engineering/context-engineering) + empirical work, LLM-synthesised AGENTS.md files degrade agent task success vs deterministic templates from structured config. |
| **Backstage / Cortex / OpsLevel** | Service catalogs. UI-heavy, focused on ownership/SLA dashboards, not on agent-readable wiki + manifest. **Coral is CLI-first, manifest-only, agent-readable.** Solves a different problem (developer-experience layer vs ops-portal layer). |
| **Bazel / Pants / Nx** | Build systems with monorepo manifests. Coral doesn't replace these — **Coral runs alongside.** Bazel governs how you build; Coral governs how the project is documented + reasoned about by agents. |
| **Garden / Skaffold / Tilt / DevSpace** | Dev-environment orchestrators. Coral's `coral up` is a thin Compose wrapper today (v0.17+); it's not trying to compete. **Coral imports from `docker-compose.yml`** (`coral env import`) so users with existing Garden/Skaffold setups can layer Coral on top without rewriting their stack. |
| **Pact / Spring Cloud Contract** | Consumer-driven contract testing. **Coral's `coral contract check` is the lighter sibling** — file-based contracts (no broker), deterministic (no LLM), runs as a CI gate. Pact is more featureful for organizations with mature contract workflows; Coral ships zero-setup for repos that just want drift detection.|

Where Coral genuinely differs:

1. **Multi-repo as a first-class manifest, not a session-scoped IDE feature.** `coral.toml` is committed to git; readable by any tool; persists across sessions; works without an IDE.
2. **Wiki is plain Markdown on disk.** Every page is `git diff`-able. No DB, no UI, no vendor lock-in. `coral export --format html` gives you a static site if you want one.
3. **Wiki is *machine-readable via MCP*.** Coding agents (Claude Code, Cursor, Continue, Cline, Goose, Codex, Copilot) read `coral://wiki/...` URIs; humans read the same files in their editor.
4. **Single-binary, single-process.** No services to operate, no DB to back up, no auth flow to set up.

Where Coral is **explicitly not trying to compete**:

- **Build systems.** Use Bazel/Pants/Nx/Cargo/pnpm — Coral runs orthogonally.
- **k8s-native dev environments.** v0.20+ may add `KindBackend` / `TiltBackend`; today Compose only.
- **Full E2E browser testing.** Use Playwright. Coral occupies the [microservice honeycomb middle layer](https://martinfowler.com/articles/2021-test-shapes.html) — integration + smoke + contract.
- **Unit testing.** Use `cargo test` / `pytest` / `jest`. Coral is functional-test layer.
- **Service catalog / ownership UI.** Use Backstage / Cortex / OpsLevel.

---

## Security model

Coral has been hardened across four audit cycles (v0.19.3 → v0.20.2) with explicit threat-model boundaries. **The threat surface is "a malicious commit / PR to a Coral-managed repo": adversary can supply arbitrary `coral.toml`, `openapi.yaml`, wiki page bodies, and test YAML.** Hardening below mitigates the high-impact vectors of that threat model.

### Process-level secret hygiene

- **API keys never appear in `argv`.** `Authorization: Bearer <token>` headers are piped to curl via stdin (`-H @-`), not arg-listed. Visible to anyone with `ps auxe` / `/proc/<pid>/cmdline`/process accounting otherwise.
- **Request bodies never appear in `argv`.** Same fix applied to `--data-binary` — bodies stream via stdin (when API key isn't already claiming it) or via per-call tempfile with `mode 0600` on Unix (`OpenOptions::new().create_new(true).mode(0o600)`).
- **Tempfile lifecycle is RAII-managed.** Cleanup happens on success, error, panic-unwind. Verified by `temp_file_guard_removes_path_on_drop` regression test.
- **`AuthFailed` Display scrubs `Bearer …` / `x-api-key:` substrings** before emission. Defense against LLM proxies that echo request headers in their error responses.
- **Audit log for MCP write-tools.** `.coral/audit.log` (JSON-line per call: `{ts, tool, args, result_summary}`). Rotates at 16 MiB to `.coral/audit.log.1`.

### Path-traversal hardening

Every place a slug or repo name lands in a filesystem path interpolation routes through one of two allowlists:

- `coral_core::slug::is_safe_filename_slug` — for wiki page slugs from frontmatter / LLM YAML / consolidate plans / export targets.
- `coral_core::slug::is_safe_repo_name` — for repo names in `coral.toml` and MCP URI segments.

Both reject: empty, leading `.`, leading `-`, contains `/` `\\` `..` whitespace NUL, anything outside `[a-zA-Z0-9_-]`, length > 200. **A malicious `coral.toml` with `name = "../escape"` is rejected at validate time, not at clone time.**

### Subprocess-level injection hardening

Coral shells out to `git` (clone, fetch, checkout, merge, diff, rev-parse, ls-files) and `docker compose` / `podman compose` extensively. Every git invocation that takes a user-controlled positional argument:

- Inserts `--` between flags and positionals, defending against the [CVE-2017-1000117](https://nvd.nist.gov/vuln/detail/CVE-2017-1000117) / [CVE-2024-32004](https://nvd.nist.gov/vuln/detail/CVE-2024-32004) family of `git clone` option-injection bugs.
- Refuses positional inputs starting with `-` where `--` isn't applicable (e.g. `git checkout <ref>` doesn't accept `--`-terminated input the way `git clone` does — Coral instead rejects refs that look like flags).

### Concurrency-safety

- **Atomic writes** via `coral_core::atomic::atomic_write_string` (tmp + rename). Used for every user-visible file Coral writes: `.wiki/pages/**`, `.wiki/index.md`, `.wiki/log.md`, `coral.lock`, `.coral/tests/**`, compose YAML artifacts, `.coral/exports/*`.
- **Cross-process serialization** via `coral_core::atomic::with_exclusive_lock` (`flock(2)` advisory lock). Used for every read-modify-write sequence on shared state: `.wiki/index.md` during `coral ingest`, `coral.lock` during `coral project sync`, `.wiki/log.md` during any append.
- **Race-free under contending writers.** `WikiLog::append_atomic` first-create header race fixed in v0.19.6 (was reproducible 1/50 with 4 threads pre-fix).
- **Verified by stress tests.** 50-thread atomic-write test, 16-subprocess flock test, 4-thread WikiLog header race test, 2-process `coral.lock` upsert test.

### Adversarial-input file-size caps

`coral_core::walk::read_pages` caps any single page at 32 MiB. `coral-test::contract_check` caps OpenAPI spec reads at 32 MiB. Defense against memory-exhaustion attacks via a multi-GB attacker-controlled file.

### Prompt-injection detection

`coral lint` scans page bodies for the obvious markers:
- Fake delimiters: `<|system|>`, `</system>`, `</user>`, `</human>`, `<|im_start|>`, `<|im_end|>`.
- Header-shaped substrings like `Authorization:`, `Bearer`, `x-api-key:`.
- Base64 runs > 100 chars (often hidden encoded payloads).
- Unicode bidi controls (`U+202E`) and tag chars (`U+E0000`–`U+E007F`).
- Confidence-drop instruction patterns ("ignore previous instructions", "you are now…").

Surfaces hits as `LintCode::InjectionSuspected` (warning severity). **Default-on since v0.20.2** (audit cycle 4 H4) — pass `--no-check-injection` to suppress when you need a fast lint loop. The bundled pre-commit hook (`template/hooks/pre-commit.sh`) also invokes it alongside the `unreviewed-distilled` gate so distilled pages with injection-shaped bodies are surfaced before they land in the repo.

In addition to the lint, every command that interpolates a wiki body into an LLM prompt (`coral query`, `coral diff --semantic`, `coral lint --auto-fix`, `coral lint --suggest-sources`) now wraps each body in a `<wiki-page>` CDATA fence and appends an explicit "untrusted content boundaries" notice to the system prompt. The CDATA terminator (`]]>`) is defanged on the way in so a malicious body cannot escape its envelope (added in v0.20.2, audit cycle 4 H3).

### What Coral does NOT defend against

- **A compromised LLM provider.** If your `ANTHROPIC_API_KEY` is exfiltrated, Coral can't help.
- **A compromised local user (Linux multi-tenant).** macOS isolates tempfiles per-user under `$TMPDIR`; Linux's `/tmp` is shared across UIDs but Coral mitigates via mode-0600 tempfiles. A user with the same UID as you can still read your tempfiles — that's a kernel-level isolation question, not a Coral one.
- **A compromised `coral` binary.** Verify SHA-256 against the GitHub release; ad-hoc codesigning is included for first-launch macOS Gatekeeper but doesn't establish vendor identity.
- **Network-level MITM on git clone.** Use `https://` with cert pinning, or `ssh://` (Coral always honors your git config; no special handling).
- **CSRF / DNS-rebinding on the MCP HTTP/SSE transport.** v0.21.1 ships HTTP/SSE behind `coral mcp serve --transport http`. The default bind is `127.0.0.1` and the Origin allowlist defends browser clients against DNS-rebinding; native clients can spoof Origin, so the localhost default is the load-bearing protection. Don't bind `0.0.0.0` unless you know what you're doing — see [Security model for the HTTP transport](#security-model-for-the-http-transport).

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

9 crates in a Cargo workspace. Each crate owns one concern; the trait families (`Runner`, `EnvBackend`, `TestRunner`, `ResourceProvider`/`ToolDispatcher`) keep concrete implementations swappable.

```
crates/
├── coral-cli/        # 42 CLI subcommands; clap dispatcher; thin adapters over the libraries
├── coral-core/       # types: Page, Frontmatter, WikiIndex, WikiLog, Project, Lockfile;
│                     # atomic file writes + flock; gitdiff + git_remote subprocess wrappers;
│                     # wiki walk (rayon, content-hashed cache); TF-IDF search; embeddings JSON+SQLite backends.
├── coral-env/        # EnvBackend trait + ComposeBackend (compose YAML render + subprocess);
│                     # Healthcheck model (Http/Tcp/Exec/Grpc + timing); EnvPlan + status;
│                     # MockBackend for upstream tests; runtime detection (docker/podman).
├── coral-test/       # TestRunner trait + 9 TestKind variants (3 wired today: Healthcheck, UserDefined,
│                     # MockTestRunner; 6 reserved for future runners); HurlRunner + OpenAPI Discovery;
│                     # probe (TCP/HTTP/exec/gRPC); JUnit emit.
├── coral-mcp/        # JSON-RPC 2.0 stdio MCP server; ResourceProvider trait; static catalogs
│                     # (resources, tools, prompts); read-only enforcement; protocol 2025-11-25.
├── coral-runner/     # Runner trait (Send+Sync); 5 impls: Claude, Gemini, Local, Http, Mock;
│                     # PromptBuilder with {{var}} substitution; embeddings providers
│                     # (Voyage, OpenAI, Anthropic, Mock).
├── coral-lint/       # 11 structural checks + 1 LLM semantic check; auto-fix routing;
│                     # `unreviewed-distilled` (qualified-on, v0.20+) + `injection-suspected`
│                     # (default-on since v0.20.2).
├── coral-session/    # `coral session` family — capture/list/show/forget/distill;
│                     # privacy scrubber (default-on); distilled output tracking with
│                     # per-finding cleanup (v0.20.2+).
└── coral-stats/      # StatsReport (totals, by_type/status, confidence stats).
```

Dependency graph (top-down):

```
coral-cli ─┬─→ coral-core ─────→ rusqlite, fs4, walkdir, serde, toml, chrono, rayon
           ├─→ coral-env ──────→ coral-core
           ├─→ coral-test ─────→ coral-env, coral-core
           ├─→ coral-mcp ──────→ coral-core
           ├─→ coral-runner ───→ (no internal deps)
           ├─→ coral-lint ─────→ coral-core, coral-runner
           ├─→ coral-session ──→ coral-core, coral-runner
           └─→ coral-stats ────→ coral-core
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

### `coral up --watch` fails on macOS Sonoma+ with "compose watch" file-descriptor errors

Known [Docker Desktop 4.57+ regression](https://github.com/docker/for-mac/issues/7832). Coral emits a one-line `WARNING:` banner to stderr before starting `compose watch` on macOS, so this issue is never silent. Workarounds:

- Add a `.dockerignore` at each repo root excluding `vendor/`, `node_modules/`, `target/`.
- Run `coral up --env dev` without `--watch` — `[services.*.watch]` blocks in `coral.toml` are only consulted by `--watch`, so omitting the flag keeps the env up without the live-reload subprocess.
- Restart Docker Desktop when sync stalls — the underlying fsevents handle gets corrupt over long sessions.
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

## FAQ

### Does Coral replace [Cursor / Claude Code / Continue / …]?

No. Coral is the **manifest layer**: the project's structured truth on disk that those tools consume via MCP. Cursor and Claude Code are editors/agents; Coral describes the project. They're complementary — most users run both.

### Can I use Coral without LLMs?

Yes. Most commands work without any LLM provider:

- `coral init`, `coral status`, `coral lint`, `coral stats`, `coral search`, `coral diff`, `coral history`, `coral export`, `coral export-agents`, `coral context-build`, all `coral project *`, all `coral env *`, `coral verify`, `coral test` (excluding LLM-generated cases), `coral test-discover`, `coral contract check`, `coral mcp serve` (read-only mode).

LLM-required commands: `coral bootstrap`, `coral ingest --apply`, `coral query`, `coral consolidate`, `coral onboard`, `coral lint --auto-fix`, `coral lint --suggest-sources`, `coral test generate` (v0.19.0+).

### What LLM providers are supported?

5 runner implementations:
- **Claude** (Anthropic, default) — via `claude` CLI or `ANTHROPIC_API_KEY`.
- **Gemini** — via `gemini-cli` or `GEMINI_API_KEY`.
- **Local** — via llama.cpp / Ollama / any local OpenAI-compatible server.
- **Http** — any OpenAI-compatible HTTP endpoint via `CORAL_HTTP_BASE_URL`.
- **Mock** — for tests.

### Does Coral send my code to the LLM?

Only when you explicitly run an LLM-backed subcommand. The wiki layer reads source via `git diff` / `git ls-files` to construct prompts, and the prompt content is whatever you'd see in `coral ingest --dry-run` or `coral query --print-prompt` first. **Coral never sends data to its own backend** — it has no backend; the LLM call goes directly from your machine to the provider you configured.

### How does Coral handle my secrets?

`Authorization: Bearer <token>` headers are piped to curl via stdin (`-H @-`), never visible in `argv` / `ps`. Request bodies (which can include user prompts) are also kept out of `argv` — either via stdin or via per-call tempfile with mode 0600 on Unix. RAII guards ensure tempfile cleanup on every return path. `AuthFailed` Display scrubs Bearer / x-api-key substrings before emission. See [Security model](#security-model) for the full posture.

### Does Coral work offline?

The wiki, lint, search, stats, diff, history, export, MCP server, env (after `coral up`), and verify layers all work offline. LLM-backed commands need network unless you configure a local runner. `coral project sync` needs network for the initial clone.

### What's the relationship between `coral.toml` and `package.json` / `Cargo.toml` / `BUILD.bazel`?

Orthogonal. `coral.toml` describes the **project shape** (which repos, dev environment, tests). It doesn't replace `package.json` (which describes a JS/TS package's build), `Cargo.toml` (which describes a Rust crate's build), or `BUILD.bazel` (which describes Bazel build targets). Coral runs alongside any of those.

### Can I commit `.wiki/` to git?

Yes — that's the recommended workflow. `.wiki/pages/*.md` are plain Markdown with YAML frontmatter; they `git diff` cleanly. `.wiki/index.md` and `.wiki/log.md` are also human-readable. Commit them; the `coral ingest` runs in CI (or pre-commit hook) keep them current.

### What happens when I run two `coral ingest --apply` simultaneously?

Both serialize via `flock(2)` on `.wiki/index.md`. Both writes land. No data loss. Stress-tested with N=10 parallel processes; verified in `crates/coral-core/tests/concurrency.rs`. Same guarantee for `coral project sync`'s `coral.lock` write (added in v0.19.6).

### What happens if my `coral.toml` has a syntax error?

`coral` exits with a clear parse error pointing at the offending line. Pre-v0.19.6 it silently fell back to legacy single-repo mode; v0.19.6 fixed that.

### Can I use Coral on Windows?

Compilation works (Rust targets Windows). Day-to-day commands work in WSL. Native Windows `cmd.exe` / PowerShell support is best-effort; some path-handling edge cases may surface (CRLF in cache fast path was fixed in v0.19.6, but expect more rough edges than macOS / Linux). File a bug if you hit one.

### Is the wiki-as-context approach better than embedding the whole codebase?

Per [Anthropic's context-engineering guidance](https://www.anthropic.com/engineering/context-engineering) and broader empirical work, **structured note-taking persisted across sessions outperforms monolithic context dumps**. Coral leans into this: the wiki is small (typically ~300 lines/page, a few dozen pages), curated by an LLM librarian under a strict schema, and budget-loadable via `coral context-build --budget <tokens>`. Empirically: comparable token budget → meaningfully better task success vs raw codebase dumps.

### What's the SLA / support model?

Hobby project. No SLA. Issues filed at [github.com/agustincbajo/Coral/issues](https://github.com/agustincbajo/Coral/issues) get triaged in best-effort timeframes. The codebase has 1124 tests + 4-cycle multi-agent audit history; quality bar is high but support cadence isn't.

### Can I use Coral's wiki schema with a different tool?

Yes. The wiki is plain Markdown with YAML frontmatter following a documented schema (see [The wiki schema](#the-wiki-schema)). Any tool that reads Markdown + YAML can consume it. You only need Coral if you want the ingest/lint/query/MCP/agent-export workflows.

---

## Glossary

Coral-specific terminology used throughout this README and in the source.

- **Page** — a single Markdown file under `.wiki/pages/<type>/<slug>.md`. Has YAML frontmatter + body.
- **Slug** — the unique identifier for a page (`auth-flow`, `order-saga`). In multi-repo mode slugs are namespaced as `<repo>/<slug>`.
- **Frontmatter** — the YAML block at the top of every page: `slug`, `type`, `status`, `confidence`, `last_updated_commit`, `sources`, `backlinks`. See [The wiki schema](#the-wiki-schema).
- **Page type** — one of `module`, `operation`, `concept`, `reference`, `index`, `decision`, `log`, `synthesis`. Determines the page's role and which directory it lives in.
- **Status** — `draft`, `reviewed`, `stale`. Drives lint behavior and ingest decisions.
- **Confidence** — float 0.0–1.0. The runner's self-reported confidence in the page's accuracy. Pages with high confidence but no `sources` get flagged.
- **Sources** — list of file paths or PR/commit refs the page is grounded in. Format: `src/auth.rs:12-87` or `#142` (PR ref).
- **Backlinks** — list of slugs that wikilink TO this page. Maintained by `coral lint`.
- **Wikilink** — `[[other-slug]]` or `[[other-slug|alias]]` reference inside a page body. Coral's lint validates all wikilinks point at real pages.
- **Wiki layout** — `aggregated` (single `.wiki/` at project root, slugs namespaced) is the only supported value as of v0.19.x.
- **Project** — the unit of multi-repo organization. Declared in `coral.toml`. Has a name, defaults, remotes, repos, environments.
- **Repo** — one entry under `[[repos]]` in `coral.toml`. Has a name, URL (or remote+template), ref, optional path override, tags, depends_on.
- **Tag** — string label on a repo (`service`, `library`, `team:platform`). Filterable via `--tag` on most commands.
- **Manifest** — `coral.toml`. Diff-able in git. Single source of truth for project shape.
- **Lockfile** — `coral.lock`. Resolved SHAs for each repo at last `coral project sync`. Same role as `Cargo.lock` / `package-lock.json`.
- **Local overrides** — `coral.local.toml`. Gitignored, per-developer overrides (point a repo at a local clone, override a ref).
- **Environment** — one entry under `[[environments]]` in `coral.toml`. Declares a backend (`compose`), services, optional healthchecks.
- **Backend** — the env-orchestration target: `compose` (v0.17, only one shipped), `kind`/`tilt` (v0.20+ behind feature flags).
- **Service** — a single container in an environment. Declared at `[environments.services.<name>]`.
- **Healthcheck** — declarative liveness probe on a service. `kind = "http" | "tcp" | "exec" | "grpc"` + timing.
- **Mode** — `managed` (Coral generates compose YAML) or `adopt` (user brings their own; v0.20+ only).
- **TestKind** — discriminator for `TestCase`. The enum carries 9 variants for forward-compat (`Healthcheck`, `UserDefined`, `LlmGenerated`, `Contract`, `PropertyBased`, `Recorded`, `Event`, `Trace`, `E2eBrowser`); only **3 are user-reachable today** (`Healthcheck`, `UserDefined`, plus `MockTestRunner` for tests). The remaining 6 are reserved markers — their runners ship in later cycles.
- **Runner** — the `Runner` trait in `coral-runner` plus its 5 implementations (`Claude`, `Gemini`, `Local`, `Http`, `Mock`). Provider-agnostic LLM call abstraction.
- **EnvBackend** — the `EnvBackend` trait in `coral-env`. Wraps the env-orchestration tool; `ComposeBackend` is the only impl today.
- **TestRunner** — the `TestRunner` trait in `coral-test`. Multiple impls: `HealthcheckRunner`, `UserDefinedRunner`, `HurlRunner`, `DiscoveryRunner`.
- **MCP resource** — a read-only URI exposed by `coral mcp serve` that returns content. Listed under [MCP client integration](#mcp-client-integration).
- **MCP tool** — a callable function exposed by `coral mcp serve`. Read-only by default; write tools gated by `--allow-write-tools`.
- **MCP prompt** — a parameterized prompt template `coral mcp serve` exposes for client use.
- **Ingest** — incremental wiki update from `last_commit` to HEAD. Run `coral ingest --apply`.
- **Bootstrap** — full wiki compilation from HEAD (one-shot, expensive, runs once per project lifetime). Run `coral bootstrap --apply`.
- **Consolidate** — LLM-suggested merges/splits/retirements of redundant pages. Run `coral consolidate --apply`.
- **Onboard** — generate a personalized reading-order page list. Run `coral onboard --profile <p> --apply`.
- **Lint** — structural + semantic checks over the wiki. 11 structural rules (incl. v0.20.0 `unreviewed-distilled` + `injection-suspected`) + 1 LLM rule.
- **Session** — one captured agent transcript (Claude Code today; Cursor/ChatGPT tracked). Lives at `<project_root>/.coral/sessions/<date>_<source>_<sha8>.jsonl`. Gitignored by default.
- **Captured session** — the raw `.jsonl` after `coral session capture`. May be scrubbed (default) or verbatim (`--no-scrub --yes-i-really-mean-it`).
- **Distilled session** — the `.md` synthesis page produced by `coral session distill`. Lands as `reviewed: false` so the trust-by-curation gate (`coral lint`) blocks any commit until a human reviews.
- **`coral.toml` apiVersion** — schema versioning field. Currently `coral.dev/v1`. Forward-compatible bump path via `coral migrate` (v0.21+).

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

✅ **Shipped (v0.19.1 → v0.20.x — audit-driven hardening sprint):**

A 3-cycle multi-agent audit found and resolved ~50 bugs across reliability, security, doc-vs-reality, concurrency, and adversarial-input handling. Highlights:

- **MCP server fully wired** (was a stub at v0.19.0 — `resources/read` returned `-32601` for every URI; now serves all 6 catalog URIs end-to-end with spec-compliant `mimeType` per resource).
- **Critical security hardening**: `git clone` option-injection (CVE-2017-1000117 family) closed; slug allowlists at every interpolation site; API keys + request bodies migrated off `argv` to stdin / mode-0600 tempfile via RAII guard; `AuthFailed` Display scrubs Bearer/x-api-key.
- **Concurrency**: `coral ingest --apply` lost-update race on `.wiki/index.md` + `coral project sync` lost-update race on `coral.lock` + `WikiLog::append_atomic` first-create header race all fixed via `with_exclusive_lock` (cross-process flock).
- **`coral env import <compose.yml>` shipped (v0.19.7)** — convert existing docker-compose.yml to coral.toml starter.
- **Test count**: 700+ → 928, BC contract holds.

Full per-release detail in [CHANGELOG](CHANGELOG.md).

🚧 **v0.19.x patches still on the table** (filed as GitHub issues):

- [#26](https://github.com/agustincbajo/Coral/issues/26) — MCP `resources/list` cursor pagination (currently unbounded).
- [#27](https://github.com/agustincbajo/Coral/issues/27) — wikilink escaped pipe `[[a\|b]]` regex edge case.
- [#28](https://github.com/agustincbajo/Coral/issues/28) — document the `_default` magic repo prefix in MCP URIs.
- [#29](https://github.com/agustincbajo/Coral/issues/29) — OpenAPI adversarial-input audit ($ref cycles, giant inline examples, escaped paths).
- [#30](https://github.com/agustincbajo/Coral/issues/30) — `coral export --format html` XSS surface audit.
- [#31](https://github.com/agustincbajo/Coral/issues/31) — streaming HTTP runners stress test (mid-stream truncation, cancellation, recv_timeout).
- [#32](https://github.com/agustincbajo/Coral/issues/32) — `*.lock.lock` zero-byte sentinel cleanup (TOCTOU deferred).
- [#33](https://github.com/agustincbajo/Coral/issues/33) — `WikiLog` regex `op` shape (log v2 format eventually).

✅ **Shipped (v0.20.0 — `coral session`):**

- **[#16](https://github.com/agustincbajo/Coral/issues/16) — `coral session capture/distill`.** Capture agent transcripts (Claude Code JSONL today; Cursor / ChatGPT tracked) into `.coral/sessions/`, scrub secrets by default, distill into `reviewed: false` synthesis pages enforced by `coral lint`. New `coral-session` crate; five subcommands (`capture`, `list`, `forget`, `distill`, `show`); 25-pattern privacy scrubber + secrets fixture; full e2e against the Claude Code JSONL schema. See [docs/SESSIONS.md](docs/SESSIONS.md). 1068+ tests pass (was 977).

✅ **Shipped (v0.21.0 — `coral env devcontainer emit`):**

- **`coral env devcontainer emit` (offline).** Render a `.devcontainer/devcontainer.json` from the active `[[environments]]` block so VS Code / Cursor / GitHub Codespaces can attach to the same Compose project Coral runs. Pure renderer in the env layer (`coral_env::render_devcontainer`), no I/O; `--write` lands the file atomically at `.devcontainer/devcontainer.json` (atomic-write, sibling tempfile + rename, matching `coral env import --write`). Service auto-selection prefers the first real service with a `repo = "..."` reference and falls back alphabetically; `--service` overrides explicitly. `forwardPorts` is the union of every `RealService.ports` from the spec, deduped and sorted. **1124 tests pass (was 1108; +16).**

✅ **Shipped (v0.21.2 — `coral up --watch`):**

- **`coral up --watch` (compose 2.22+ `develop.watch`).** Wire the wave-1 `WatchSpec` / `SyncRule` types through the YAML renderer and `ComposeBackend::up`. After `up -d --wait` succeeds, run `compose watch` foreground until Ctrl-C; SIGINT (130) tears the watch subprocess down cleanly without killing containers. `coral env watch` is an alias. macOS users get a one-line `WARNING:` banner pointing at [docker/for-mac#7832](https://github.com/docker/for-mac/issues/7832). `EnvCapabilities::watch` flips `true`. BC sacred: services without `[services.*.watch]` emit byte-identical YAML to v0.21.1. **1174 tests pass (was 1155; +19).**

✅ **Shipped (v0.21.4 — `MultiStepRunner` opt-in):**

- **`coral consolidate --tiered` (planner + executor + reviewer).** New `MultiStepRunner` trait with a concrete `TieredRunner` impl that decomposes a single LLM run into three sequential calls — planner emits 1-5 sub-tasks as YAML, one executor call per sub-task, reviewer synthesizes results into the consolidate plan parser input. Per-tier `provider` + `model` configurable via `[runner.tiered.{planner,executor,reviewer}]` in `coral.toml`. Pure-Rust `len/4` token budget (default 200K) gates pre-flight at three points and surfaces the new additive `RunnerError::BudgetExceeded { actual, budget }` on overrun. CLI flag `--tiered` wins over manifest opt-in (`[runner.tiered.consolidate] enabled = true`). BC sacred: `coral consolidate` (no flag, no manifest opt-in) is byte-identical to v0.21.3 — pinned by snapshot test PLUS drift detector. Manifests without `[runner]` round-trip byte-identically. Zero new workspace deps. See [`docs/runner-tiered.md`](docs/runner-tiered.md). **1217 tests pass (was 1197; +20).**

🔮 **v0.22+ feature roadmap:**

- `coral session capture --from cursor` and `--from chatgpt` (the v0.20 flags currently emit a clear "not yet implemented" error pointing at #16).
- `KindBackend`, `TiltBackend`, `K3dBackend` (k8s local, behind feature flags).
- `PropertyBasedRunner` (proptest from OpenAPI), `RecordedRunner` (Keploy traffic capture, Linux-only feature).
- `EventRunner` (AsyncAPI, Testcontainers Kafka/Rabbit), `TraceRunner` (OTLP queries).
- `ContractRunner` (consumer-driven, `coral.contracts.lock` with `--can-i-deploy`).
- `coral test generate --auto-validate` (LLM-augmented, with iterative retry against the live env).
- `coral chaos inject` (Toxiproxy / Pumba sidecar).
- `coral monitor up` (synthetic monitoring, tests-as-monitors).
- `coral skill build / publish` (Anthropic Skills marketplace bundle).
- gRPC test steps (via `grpcurl` subprocess or `tonic` reflection).
- Cross-repo glob (`[[repos]] glob = "services/*"`) and sub-manifests `<include>`.
- `coral env attach <service>`, `coral env reset`, `coral env port-forward`, `coral env open`, `coral env prune`.
- SWE-ContextBench benchmark publication.

Detailed PRD covering every iteration (multi-repo, environments, testing, MCP, AGENTS.md research) is tracked in the maintainer's plans directory; per-release decisions and trade-offs are summarised in the [CHANGELOG](CHANGELOG.md).

---

## How Coral itself was built

### Dogfooding

Coral's own `.wiki/` is maintained by Coral. Each merge to `main` runs `coral ingest --apply` via the `.github/actions/ingest` composite action, with a Claude bibliotecario subagent doing the page curation under the SCHEMA in `template/schema/SCHEMA.base.md`.

### PRD-first

The PRD was written first (5 PRD iterations, validated against industry: Bazel, Nx, Turborepo, Cargo workspaces, Garden, Compose Watch, Tilt, Skaffold, Pact, Schemathesis, Hurl, Stepci, MCP, AGENTS.md research, Devin Wiki competitive analysis). Each `coral project` / `coral env` / `coral test` / `coral mcp` subcommand has a wave-1 (scaffold) → wave-2 (real impl) → wave-3 (advanced features) progression in the CHANGELOG, plus dedicated unit tests at every wave.

### Pluggable trait pattern

The pluggable trait pattern (`Runner` → `EnvBackend` → `TestRunner` → `ResourceProvider`/`ToolDispatcher`) was a deliberate copy of itself: one trait, one error type, one `Mock*` impl in the same crate, one factory function. Once you've debugged one of them, debugging another is muscle memory.

### Multi-agent audit pipeline

The v0.19.x sprint shipped 10 patch releases (v0.19.0 → v0.20.2) closing ~70 bugs surfaced by **four audit cycles**, each cycle running multiple parallel agents with non-overlapping mandates:

| Cycle | Agents | Focus | Findings |
|---|---|---|---|
| 1 (post-v0.19.0) | 1 broad agent | bugs, doc-vs-reality, cross-platform | 11 |
| 2 (post-v0.19.4) | 3 parallel | reliability + security + doc-vs-reality | ~30 |
| 3 (post-v0.19.5) | 1 deep dive | concurrency + new MCP code + adversarial inputs | 12 |

Each cycle followed the same loop:
1. **Audit agent(s)** — scoped mandate, instructed to verify before reporting (no hand-waving), output a punch list with severity tiers.
2. **Fix agent** — given the full punch list + working agreements; lands all fixes in one commit; runs `scripts/ci-locally.sh` until green; bumps version + CHANGELOG; **does not push or tag**.
3. **Validator agent** — re-verifies each fix is real (stash-validates the highest-impact ones), confirms regression tests catch the bug class, looks for fix-agent design decisions that need scrutiny; recommends ship / NEEDS WORK / FAIL.
4. **Maintainer applies any small inline fixes** (when the validator catches a partial fix, like the v0.19.5 README L568+ regression), then pushes + tags + creates the GitHub release.

This pipeline is reproducible — the agent prompts live in the `.claude/` directory and can be re-run on any future major release. v0.19.5 (30+ findings, fix agent + validator + 1 inline fix) and v0.19.6 (12 findings, fix agent + validator) are the canonical examples.

### `scripts/ci-locally.sh`

CI on GitHub Actions has been blocked by a billing issue throughout the v0.19.x sprint, so `scripts/ci-locally.sh` mirrors the four blocking jobs (fmt, clippy `-D warnings`, test --workspace, bc-regression) for local verification — runs in ~100s including warm cache. Used as the gate before every release tag.

---

## Releasing

Coral uses [`cargo-release`](https://github.com/crate-ci/cargo-release) for version bumps, wrapped by `scripts/release.sh` so the maintainer flow stays a one-liner per phase. See [`release.toml`](release.toml) for the wire-level config; the wrapper enforces working agreements (`push = false` and `tag = false` by default, no `Co-Authored-By` trailer, single author).

The flow has three local-only phases plus one GitHub-side step. Run them in order:

1. **Hand-write the changelog entry.** Open `CHANGELOG.md`, add a `## [X.Y.Z] - <YYYY-MM-DD>` heading directly under `## [Unreleased]`, and write the body. Preflight will refuse to proceed without this.
2. **Bump.**
   ```bash
   cargo install --locked cargo-release      # one-time, if not already installed
   scripts/release.sh bump X.Y.Z
   ```
   This invokes `release.sh preflight` (asserts the changelog heading + runs `scripts/ci-locally.sh`), then `cargo release X.Y.Z --no-tag --no-push --no-confirm --execute`. The result is a single local commit `release(vX.Y.Z): bump version` that bumps `[workspace.package].version` and every `coral-* = "X.Y.Z"` line. **No tag, no push.** Inspect with `git log -1 --stat`. If the message needs more than `: bump version` you can `git commit --amend` to swap in a feature subject.
3. **Tester sign-off.** Hand the bump commit to the tester agent (or run your own validation). Past testers in this repo have caught real partial-fixes pre-tag — don't skip.
4. **Tag and push.**
   ```bash
   scripts/release.sh tag X.Y.Z
   ```
   Validates the HEAD subject starts with `release(vX.Y.Z):`, then `cargo release tag X.Y.Z --execute && cargo release push --execute` — the tag push triggers `.github/workflows/release.yml`, which builds release binaries for Linux x86_64, macOS Intel, and macOS Apple Silicon.
5. **Wait for binaries**, then **finalize the GitHub release.** Run `gh run list --workflow release.yml` until the build is green, then:
   ```bash
   scripts/release-gh.sh vX.Y.Z
   ```
   This extracts the `## [X.Y.Z]` changelog section verbatim into `/tmp/coral-release-vX.Y.Z.md`, parses the `**Feature release: …**` bold prefix as the release title, and either updates the auto-created GH release (the workflow runs `softprops/action-gh-release@v2` with auto-notes — we replace those with the curated changelog) or creates a new one if absent.

The two helper scripts are standalone-useful:

- `scripts/extract-changelog-section.sh X.Y.Z [PATH]` — prints a changelog section verbatim. Exits 1 if the version isn't present. Useful for ad-hoc grepping or driving other release-note tooling.
- `scripts/release-gh.sh vX.Y.Z` — supports `GH_DRY_RUN=1` to preview the title and notes file without invoking `gh`.

If something goes wrong:

- **Preflight fails on `ci-locally.sh`.** Fix the failing check, then re-run `release.sh bump X.Y.Z`. Nothing has been committed yet.
- **Tag step rejects wrong subject.** The HEAD commit must start with `release(vX.Y.Z):`. If you forgot to bump first, run `scripts/release.sh bump X.Y.Z`. If you amended the subject to something the validator doesn't accept, amend it again to start with that prefix.
- **`release-gh.sh` runs before binaries are built.** Harmless — the script only updates the release's title and notes; the workflow uploads the binaries asynchronously and they appear in the release once complete.

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
