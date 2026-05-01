# Coral

> Karpathy-style LLM Wiki maintainer for Git repos.

Coral compiles your codebase into an interconnected Markdown wiki that an LLM (Claude) maintains as you push code. Each merge updates the wiki incrementally; nightly lint catches contradictions; weekly consolidation prunes redundant pages.

> *"The IDE is Claude Code. The programmer is you + the LLM. The wiki is the living memory of your codebase."*

**Status:** v0.1.0 (alpha).

---

## Why Coral?

The naive approach to giving an LLM context about your repo is a giant `AGENTS.md` file. It grows out of control, eats your context window, drifts out of sync with the code, and provides no auditability.

Coral implements [Andrej Karpathy's LLM Wiki pattern](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f) instead: a constellation of small (<300 line) Markdown pages, each tagged with frontmatter (`slug`, `type`, `confidence`, `sources`, `backlinks`), curated by an LLM bibliotecario subagent under a strict SCHEMA.

| Aspect | Naive `AGENTS.md` | Coral wiki |
|---|---|---|
| Storage | Single growing file | Constellation of small Markdown pages |
| State | Implicit, drifts | Explicit, with `last_updated_commit` per page |
| Lock-in | None | None — plain Markdown in Git |
| Auditability | Opaque | Each page cites verifiable `sources` |
| Maintenance | Manual | Incremental ingest on every push |

---

## Install

```bash
cargo install --locked --git https://github.com/agustincbajo/Coral coral-cli
```

Or build from source:

```bash
git clone https://github.com/agustincbajo/Coral && cd Coral
cargo install --locked --path crates/coral-cli
coral --version    # → coral 0.1.0
```

See [docs/INSTALL.md](docs/INSTALL.md) for prerequisites.

---

## Quickstart

```bash
# 1. In a Git repo:
coral init
# Creates .wiki/{SCHEMA.md, index.md, log.md, modules/, concepts/, ...}

# 2. Ask the wiki a question (uses your local Claude Code session):
coral query "How is an order created?"

# 3. Lint the wiki (structural + optional semantic):
coral lint --all

# 4. See wiki health:
coral stats

# 5. Pull subagent + workflow updates from a tagged Coral release:
coral sync --version v0.1.0
```

For the full subcommand reference see [docs/USAGE.md](docs/USAGE.md).

---

## Architecture

5 Rust crates in a Cargo workspace:

- **`coral-cli`** — the `coral` binary; clap-derived CLI dispatcher.
- **`coral-core`** — frontmatter, wikilinks, page model, index, log, gitdiff, parallel walk.
- **`coral-lint`** — structural checks (broken links, orphans, low confidence) + semantic lint via runner.
- **`coral-runner`** — `Runner` trait + `ClaudeRunner` (shells `claude --print`) + `MockRunner`.
- **`coral-stats`** — wiki health dashboard (markdown + JSON).

Plus:
- **`template/`** — embedded skill bundle (4 subagents, 4 slash commands, 4 prompts, base SCHEMA, GitHub workflow). Distributed via `include_dir!` and laid out by `coral sync`.
- **`.github/actions/`** — composite actions (`ingest`, `lint`, `consolidate`) consumable by external repos via `uses: agustincbajo/Coral/.github/actions/X@v0.1.0`.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and the ADRs in [docs/adr/](docs/adr/) for design decisions.

---

## Tests

```bash
cargo test --workspace                  # 150 unit + integration tests
cargo test --workspace -- --ignored     # 3 ignored tests requiring real `claude` CLI / git
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

CI runs all of the above on every push to `main` and every PR.

---

## Performance

Coral aims for sub-100 ms cold-start on the structural commands (`init`, `lint --structural`, `stats`, `sync`). Methodology, baseline numbers, profiling tips, and the release-profile config live in [docs/PERF.md](docs/PERF.md).

---

## License

MIT © 2026 Agustín Bajo
