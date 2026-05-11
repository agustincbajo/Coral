---
title: "cli: `coral stats --symbols --format json` emits HashMap-ordered keys — non-deterministic output breaks golden tests and CI diffs"
severity: Medium
labels: bug, cli, determinism
confidence: 5
cross_validated_by: [cli-ux-audit-agent, direct-code-read]
---

## Summary

`crates/coral-cli/src/commands/stats.rs:48` and `:54` accumulate
breakdowns in `HashMap`:

```rust
let mut by_kind: HashMap<SymbolKind, usize> = HashMap::new();
…
let mut by_lang: HashMap<String, usize> = HashMap::new();
```

The JSON serializer at lines 85-95 emits both maps via `serde_json::json!`
without sorting. `serde_json` preserves the source iteration order;
`HashMap` is randomized per-process (RandomState seed). Two runs of
`coral stats --symbols --format json` against the same workspace will
produce JSON whose `by_kind` and `by_language` keys appear in different
orders. Byte-level diffing or golden-snapshot testing will flake.

The Markdown branch (lines 103-114) sorts by `count` desc but uses
unstable `sort_by_key`, so ties between languages with equal counts
are also non-deterministic.

## Why it matters

- `coral context-build` and downstream consumers that compare stats
  output between commits can't get byte-identical output.
- The README mentions reproducibility as a non-functional requirement
  (NFR-15 in `docs/PRD-v0.24-evolution.md:322`: "Reproducibilidad
  cross-platform: outputs byte-idénticos Mac/Linux/Windows-WSL para
  mismo input+lockfile"). Even on the same machine, this command
  violates it.

## Repro

```bash
coral stats --symbols --format json > a.json
coral stats --symbols --format json > b.json
diff a.json b.json
# Different key order, or different tie-break ordering.
```

## Suggested fix

Change both `HashMap` accumulators to `BTreeMap`, OR convert at
serialization time via `into_iter().collect::<BTreeMap<_,_>>()`.
For the Markdown branch, add a secondary sort key (slug/lang name
ascending) after the primary `count` desc sort:

```rust
kinds.sort_by(|(ak, av), (bk, bv)| bv.cmp(av).then_with(|| ak.to_string().cmp(&bk.to_string())));
```

Add a regression test that runs the JSON branch twice in the same
process and asserts byte-equal output. (Same-process is enough — the
HashMap seed differs across runs but not within one process; the
flake is across CI runs.)

## Cross-validation

CLI-UX agent flagged this; I verified `HashMap` declarations at lines
48/54 and the unsorted JSON emit at lines 85-95.
