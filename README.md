# Coral

> Karpathy-style LLM Wiki maintainer for Git repos.

Coral compiles your codebase into an interconnected Markdown wiki that an LLM (Claude) maintains as you push code. Each merge updates the wiki incrementally; nightly lint catches contradictions; weekly consolidation prunes redundant pages.

**Status:** alpha (v0.1.0 in development).

## Install

```bash
cargo install --path crates/coral-cli
```

## Quickstart

See [docs/USAGE.md](docs/USAGE.md) (coming in v0.1.0).

## License

MIT © 2026 Agustín Bajo
