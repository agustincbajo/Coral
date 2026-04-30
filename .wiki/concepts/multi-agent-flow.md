---
slug: multi-agent-flow
type: concept
last_updated_commit: 213ac997cf61ad89610b3cfbe40af05e6b7fa8a8
confidence: 0.85
sources:
  - docs/adr/0004-multi-agent-development-flow.md
backlinks:
  - cli
  - runner-trait
status: reviewed
---

# Multi-agent development flow

The 3-role loop used to build Coral v0.1.0:

1. **Orchestrator** — defines specs, no code.
2. **Coder agent** (`general-purpose` subagent) — implements, runs `cargo build`. Doesn't approve.
3. **Tester agent** — runs `cargo test/clippy/fmt --check`. Doesn't edit. Reports pass/fail.

Loop: spec → Coder → Tester → on fail back to Coder (cutoff 3 iterations).

## Why it worked

- Each phase landed as one atomic green commit.
- Specs were detailed (~200 lines per phase), reducing ambiguity.
- The Tester has no edit privileges — incentive to report honestly.
- The [[runner-trait]] made every CLI command testable without `claude` in the sandbox.

Documented in detail in `docs/adr/0004-multi-agent-development-flow.md`.

## What broke once

Phase E1 had a clippy complaint about a redundant boolean expression and a `cargo fmt` issue. The orchestrator chose to fix those mechanically (single-line changes) rather than round-trip back to the Coder. Discipline trade-off: orchestrator may apply fmt/clippy mechanical fixes; never business logic.
