---
slug: cli
type: module
last_updated_commit: 721050563f1ed29954b279fe334bf6bc8c8e2c34
confidence: 0.9
sources:
- crates/coral-cli/src/main.rs
- crates/coral-cli/src/lib.rs
- crates/coral-cli/src/commands/
backlinks:
- core
- lint
- runner
- stats
status: verified
---

# `coral-cli` — the CLI binary

Clap-derived dispatcher for the `coral` binary. Lives at `crates/coral-cli`.

Each subcommand is a module under `commands/`:

| Command | Module | Needs LLM? |
|---|---|---|
| `init` | `commands/init.rs` | No |
| `bootstrap` | `commands/bootstrap.rs` | Yes |
| `ingest` | `commands/ingest.rs` | Yes |
| `query` | `commands/query.rs` | Yes |
| `lint` | `commands/lint.rs` | Optional (`--semantic`, `--auto-fix`, `--suggest-sources`) |
| `consolidate` | `commands/consolidate.rs` | Yes |
| `stats` | `commands/stats.rs` | No |
| `sync` | `commands/sync.rs` | No |
| `onboard` | `commands/onboard.rs` | Yes |
| `prompts` | `commands/prompts.rs` | No |
| [[search]] | `commands/search.rs` | Optional (`--engine embeddings`) |
| `export` | `commands/export.rs` | Optional (`--qa`) |
| `notion-push` | `commands/notion_push.rs` | No |
| `validate-pin` | `commands/validate_pin.rs` | No |
| `diff` | `commands/diff.rs` | Optional (`--semantic`) |
| `status` | `commands/status.rs` | No |
| `history` | `commands/history.rs` | No |

LLM-using commands expose two entry points:

- `run(args, root)` — constructs a `ClaudeRunner` (see [[runner]]) and dispatches.
- `run_with_runner(args, root, &dyn Runner)` — testable with [[mock-runner]] from `coral-runner`.

The `lib` target in `Cargo.toml` exposes the commands module so integration tests can call them directly with `MockRunner`. The `bin` target produces the `coral` binary.

See the [[karpathy-wiki]] concept for the underlying pattern Coral implements. Architectural decisions are tracked in [[decisions]]; the rationale for the Rust workspace lives in [[why-rust]]; release procedure in [[release-checklist]].
