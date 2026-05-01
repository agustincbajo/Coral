---
slug: decisions
type: decision
last_updated_commit: 213ac997cf61ad89610b3cfbe40af05e6b7fa8a8
confidence: 1.0
sources:
  - docs/adr/
backlinks:
  - cli
  - core
  - runner
  - karpathy-wiki
  - multi-agent-flow
status: verified
---

# Architecture decisions (ADR index)

This is a **link-only** page. Every architectural decision lives in `docs/adr/` as the legal source of truth. The wiki references them but never duplicates content.

## Active ADRs

- [ADR 0001 — Rust CLI architecture](../../docs/adr/0001-rust-cli-architecture.md) — workspace structure with 5 focused crates.
- [ADR 0002 — Claude CLI subprocess vs Anthropic API](../../docs/adr/0002-claude-cli-vs-api.md) — why we shell out to `claude --print`.
- [ADR 0003 — Template distribution via `include_dir!`](../../docs/adr/0003-template-distribution-via-include-dir.md) — embed-in-binary strategy.
- [ADR 0004 — Multi-agent development flow](../../docs/adr/0004-multi-agent-development-flow.md) — orchestrator/coder/tester loop.
- [ADR 0005 — Versioning and sync semantics](../../docs/adr/0005-versioning-and-sync.md) — single SemVer line, SCHEMA stays local.
- [ADR 0006 — Local semantic search storage](../../docs/adr/0006-local-semantic-search-storage.md) — TF-IDF stub in v0.2, embeddings in v0.3.

## Why link-only

ADR content evolves slowly and is the canonical reference for a decision. Synthesizing it in the wiki risks drift. Lint enforces this: any page in `decisions/` must NOT have body content beyond links + brief tagging.
