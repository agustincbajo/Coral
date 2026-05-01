# Architecture

## Workspace layout

5 crates plus a template bundle and composite GitHub Actions:

```
coral/
├── crates/
│   ├── coral-cli/      ← bin: `coral`. Clap dispatcher. Each LLM subcommand
│   │                     exposes both `run` (constructs ClaudeRunner) and
│   │                     `run_with_runner` (testable with MockRunner).
│   │
│   ├── coral-core/     ← types + parsing. Pure Rust, zero LLM coupling.
│   │   ├── error.rs    ← thiserror enum + Result alias
│   │   ├── frontmatter.rs ← Frontmatter, PageType, Status, Confidence,
│   │   │                    parse(), serialize()
│   │   ├── wikilinks.rs ← extract() with code-fence + escape skipping
│   │   ├── page.rs     ← Page = Frontmatter + body + path; from_file/write
│   │   ├── index.rs    ← WikiIndex (catalog + last_commit anchor)
│   │   ├── log.rs      ← WikiLog (append-only)
│   │   ├── gitdiff.rs  ← parse_name_status() + run() + head_sha()
│   │   ├── walk.rs     ← rayon-parallel page reader, skips _archive/
│   │   ├── cache.rs    ← WalkCache: mtime-keyed Frontmatter cache (.coral-cache.json)
│   │   ├── embeddings.rs ← EmbeddingsIndex: cosine-similarity vector store
│   │   │                    (.coral-embeddings.json), schema-versioned
│   │   └── search.rs   ← TF-IDF tokenizer + scorer (offline default)
│   │
│   ├── coral-lint/     ← LintReport + LintIssue + 5 structural checks
│   │   ├── report.rs    ← issue model + markdown rendering
│   │   ├── structural.rs ← broken_wikilinks, orphan_pages, low_confidence,
│   │   │                    high_confidence_without_sources, stale_status
│   │   └── semantic.rs  ← invokes Runner; parses severity:slug:message
│   │
│   ├── coral-runner/   ← Runner trait + ClaudeRunner + MockRunner +
│   │   │                  Prompt + PromptBuilder
│   │   ├── runner.rs    ← trait + ClaudeRunner (sync std::process)
│   │   ├── mock.rs      ← FIFO scripted responses + call capture
│   │   └── prompt.rs    ← `{{var}}` regex substitution
│   │
│   └── coral-stats/    ← StatsReport (totals, by_type/status, confidence
│                          stats, orphan candidates)
│
├── template/           ← embedded via include_dir!; surfaced by `coral sync`
│   ├── agents/         ← 4 Claude Code subagents
│   ├── commands/       ← 4 slash commands
│   ├── prompts/        ← 4 versioned prompt templates ({{var}} placeholders)
│   ├── schema/SCHEMA.base.md ← contract with the bibliotecario subagent
│   └── workflows/wiki-maintenance.yml ← 3 jobs (ingest, lint nightly, consolidate weekly)
│
└── .github/
    ├── actions/        ← composite actions consumable by external repos
    │   ├── ingest/action.yml
    │   ├── lint/action.yml
    │   └── consolidate/action.yml
    └── workflows/
        ├── ci.yml       ← fmt + clippy + test on push/PR
        └── release.yml  ← cargo-release + GH binaries on tag
```

## Data flow

```
        ┌─────────────────────────────────────────────────┐
        │                  Your Git repo                   │
        │                                                  │
        │   src/  docs/  Cargo.toml  …                    │
        │                                                  │
        │   .wiki/  ←─── SCHEMA.md, index.md, log.md       │
        │           ←─── modules/, concepts/, entities/,   │
        │                flows/, decisions/, synthesis/,   │
        │                operations/, sources/, gaps/       │
        └────────────────────┬─────────────────────────────┘
                             │
                             │ coral CLI commands
                             ▼
        ┌─────────────────────────────────────────────────┐
        │                  coral-cli                       │
        │                                                  │
        │  init / sync / lint --structural / stats        │  ← no LLM
        │                                                  │
        │  bootstrap / ingest / query / consolidate /     │  ← LLM via Runner
        │  onboard / lint --semantic                      │
        │                       │                          │
        │                       ▼                          │
        │              ┌──────────────┐                    │
        │              │  Runner      │                    │
        │              │  trait       │                    │
        │              └──────┬───────┘                    │
        │                     │                            │
        │            ┌────────┴───────┐                    │
        │            ▼                ▼                    │
        │     ClaudeRunner     MockRunner                  │
        │     (prod / CI)      (tests)                     │
        │            │                                     │
        │            ▼                                     │
        │     `claude --print`                             │
        └─────────────────────────────────────────────────┘
```

## Performance choices

- **Lazy regex compilation**: `static LINK_RE: OnceLock<Regex>` for wikilinks; same pattern for log entry parser. Compiled once, shared across pages.
- **Parallel walk**: `walkdir::WalkDir` + `rayon::par_iter`. A 500-page wiki scans + parses in <50ms on Apple Silicon.
- **No libgit2**: `gitdiff::run` shells out to `git diff --name-status`. Saves ~5MB of binary size and avoids C dependencies.
- **Embedded template**: `include_dir!` macro bakes the entire `template/` tree into the binary. `coral sync` extracts at runtime — zero filesystem dependencies post-install.
- **Sync I/O for runner**: `claude --print` is a one-shot subprocess. No tokio runtime needed for the v0.1 happy path. Simpler binary, less indirection.
- **Stable sort by severity → page → message** in lint reports for deterministic output (CI diffs are clean).
- **Release profile**: `lto = "thin"`, `codegen-units = 1`, `strip = true`. Target binary <8MB stripped.

## Testing strategy

- **Pure functions over `&[Page]`**: every lint check, every stats computation, every prompt builder is testable without filesystem or network.
- **`MockRunner` for LLM-coupled code**: unit tests of `query::run_with_runner` push scripted responses to a `MockRunner`, then assert the prompt the runner received.
- **`tempfile::TempDir` + private `Mutex<()>`** for tests that touch `current_dir`. No global `serial_test` dependency.
- **`#[ignore]` for tests requiring real `claude` CLI or git binary**. Run via `cargo test -- --ignored`.
- **Composite GH actions and workflow YAML are validated by tests**: `template_validation.rs` parses every embedded YAML to catch refactor regressions.

## Multi-agent development flow (how Coral was built)

This codebase was built using a 3-agent loop, replicating the pattern proposed in the plan:

1. **Orchestrator** (Claude in the foreground) — defines per-phase specs.
2. **Coder agent** (`general-purpose` subagent) — implements code, runs `cargo build/fmt/clippy`. Does NOT approve.
3. **Tester agent** — runs `cargo test/clippy/fmt --check`. Reports pass/fail. Does NOT edit.

If the Tester reports a failure, the orchestrator hands the failure log + original spec back to the Coder for another iteration (cutoff at 3 retries before escalating to the user). This is documented in [docs/adr/0004-multi-agent-development-flow.md](adr/0004-multi-agent-development-flow.md).

The result: 9 sequential phases (A–I), each landing as one atomic commit, every commit with green CI.

## Lifecycle

```
                                                       merge to main
                                                            │
   ┌──────────┐    ┌──────────┐    ┌──────────┐             ▼
   │  init    │───▶│ bootstrap│───▶│  page    │      ┌──────────────┐
   │  (once)  │    │  (once)  │    │  edit    │      │ GH Action     │
   └──────────┘    └──────────┘    └────┬─────┘      │ ingest job    │
                                        │            └──────┬───────┘
                                        ▼                   │
                                   ┌──────────┐             ▼
                                   │  lint    │      ┌──────────────┐
                                   │ struct.  │      │ wiki/auto-    │
                                   │  (CI)    │      │ ingest PR     │
                                   └──────────┘      └──────────────┘
                                                            │
                                                            ▼
                                                     ┌──────────────┐
                                                     │ nightly lint │
                                                     │   semantic   │
                                                     └──────────────┘
                                                            │
                                                            ▼
                                                     ┌──────────────┐
                                                     │ weekly       │
                                                     │ consolidate  │
                                                     └──────────────┘
```

## Key invariants

- **HEAD wins over the wiki.** If a page contradicts code, mark `status: stale`.
- **`log.md` is append-only.** Never edit, never reorder.
- **Every page declares `last_updated_commit`.** The lint flags pages whose commit is no longer reachable from HEAD.
- **Confidence ≥ 0.7 requires `sources`.** The lint enforces this.
- **A new page links to ≥2 existing pages and is linked by ≥1.** Otherwise it's an orphan and lint warns.
- **Decisions never duplicate ADR content.** `decisions/index.md` is link-only to `docs/adr/`.

## Versioning + sync model

Coral itself is versioned with `cargo-release`. Each tag (`v0.1.0`, `v0.2.0`, …) ships:

- The `coral` binary (Linux + macOS, x86_64 + aarch64) as GitHub release assets.
- The exact `template/` contents embedded in that binary.

A consumer repo pins a Coral version with `.coral-template-version`. Running `coral sync --version v0.2.0` extracts the template files for that version. The consumer's `.wiki/SCHEMA.md` is **only** copied on first sync; subsequent syncs leave it alone (it co-evolves locally).

See [docs/adr/0005-versioning-and-sync.md](adr/0005-versioning-and-sync.md).
