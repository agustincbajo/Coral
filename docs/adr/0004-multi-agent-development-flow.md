# ADR 0004 — Multi-agent development flow

**Date:** 2026-04-30  
**Status:** accepted (used to build v0.1.0)

## Context

The user requested that Coral itself be built using a multi-agent flow:

> *"un agente orquestador que no va a escribir código, un agente que va a escribir código pero no va a aprobar, y otro agente superespecializado para la prueba. Si falla algo, va a ir el agente que escribe código, y así se va a ir iterando."*

This is more than a curiosity — it's a real test of whether the Karpathy LLM Wiki pattern (Coral itself) can be produced by multi-agent orchestration without a human writing code.

## Decision

Three roles, strictly enforced:

1. **Orchestrator** (Claude in the foreground) — defines per-phase specs, manages the coder↔tester loop, handles commits and pushes. Writes ZERO production code.
2. **Coder agent** (`general-purpose` subagent invoked via the Agent tool) — receives a spec, implements code, runs `cargo build` to confirm it compiles. **Does not approve.** Reports diff + summary.
3. **Tester agent** (`general-purpose` subagent with restricted instruction) — runs `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`. Reports pass/fail + log of failures. **Does not edit.**

### Loop per phase

```
Orchestrator: define spec → Coder: implement → Tester: verify
                                                    │
                                ┌── pass ──► Orchestrator commits + advances
                                │
                                └── fail ──► Orchestrator forwards log to Coder → loop
```

Cutoff: 3 iterations per phase before escalating to the user.

## Consequences

**Positive:**
- **Each phase landed as one atomic, green commit.** Easy to revert if needed.
- **The Coder never sees the orchestrator's full state**, only the focused spec. Limits context bloat.
- **The Tester is incentivized to be honest**: it has no edit privileges, so reporting "PASS" when something fails would be self-defeating.
- **The pattern scales**: 9 phases (A–I), 150+ tests, 8 commits, all green from the first attempt for 7/9 phases. Only 1 phase (E1) needed a single human-side fix (one fmt + one clippy line).
- **Documents the development process**: every phase has its commit message + test counts, traceable in `git log`.

**Negative:**
- **Token cost** — multi-agent dispatching is more expensive than one big monolithic agent. For Coral v0.1.0, this was ~5x normal cost. Justified by quality bar.
- **Long iteration time** — each round-trip (orchestrator → coder → tester → orchestrator) takes 1–3 minutes for non-trivial tasks. Total build time: ~3 hours for 9 phases.
- **The Coder occasionally over-implements** (e.g., adds unsolicited tests). Spec discipline matters.

## Alternatives considered

- **Single-agent (one Coder, no Tester)**: rejected by the user. Multi-agent gives independent verification.
- **Orchestrator also writes code occasionally**: tried for fmt fixes (mechanical, 1-line). Acceptable for trivial mechanical changes; the orchestrator must NOT write business logic.
- **Different agent specializations** (e.g., one Coder per crate): could work, but for v0.1 a single general-purpose Coder per phase was sufficient.

## What worked

- **Detailed specs.** The orchestrator wrote ~200-line specs per phase with concrete file paths, function signatures, test cases, edge cases.
- **Explicit rules in spec**: "no commits", "no `unwrap` in prod code", "run `cargo fmt` before reporting".
- **Phase atomicity**: every commit is a green workspace. Rollback is trivial.

## What to improve

- **Tester scope**: in some phases the Tester only ran the affected crate (`cargo test -p coral-cli`), missing cross-crate breakage. Solution adopted: always run `cargo test --workspace` at the end.
- **Clippy fixups loop**: when the Tester reports a clippy failure, it's tempting to fix it directly. The discipline of going back to the Coder takes longer but preserves the contract.
