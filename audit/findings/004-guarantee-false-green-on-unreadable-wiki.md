---
title: "cli: `coral guarantee` returns Verdict::Green when the wiki cannot be read — CI deploy-gate silently passes on a broken wiki"
severity: High
labels: bug, cli, ci, guarantee
confidence: 5
cross_validated_by: [cli-ux-audit-agent, direct-code-read]
---

## Summary

`coral guarantee` is intended as a CI deploy-gate that aggregates lint,
contract, and other checks. If `coral_core::walk::read_pages()` returns
`Err` (corrupt frontmatter on any page, permission denied, broken
symlinks under `.wiki/`), the lint check at
`crates/coral-cli/src/commands/guarantee.rs:179-190` swallows the error
and returns `CheckResult { passed: 0, failures: 0, warnings: 0, detail:
"failed to read wiki pages" }`.

```rust
let pages = match coral_core::walk::read_pages(wiki_path) {
    Ok(p) => p,
    Err(_) => {
        return CheckResult {
            name: "lint",
            passed: 0,
            warnings: 0,
            failures: 0,
            detail: "failed to read wiki pages".into(),
        };
    }
};
```

A `CheckResult` with zero failures and zero warnings contributes
**nothing** to the verdict aggregation. Combined with similar swallow
patterns in the sister checks (likely `run_contract_check`,
`run_structural_check`), a fully broken wiki — every page corrupted —
yields `Verdict::Green`, exit code 0. Whatever CI gate is wired to
`coral guarantee` ships the corruption to production.

## Impact

The README positions `coral guarantee` as the deploy-gate (the
"Sessions" / "AI ecosystem" layers depend on it indirectly). False-green
on a corrupt wiki defeats the entire premise. A single malformed
frontmatter block (yaml-ng parser error) in any page is enough to flip
the gate green.

## Repro

1. `coral init` in a fresh project.
2. Append `\n---\n\n` (an unterminated frontmatter block) to any
   `.wiki/**/*.md` page. `read_pages()` returns Err for that page; the
   walker may abort on the first failure.
3. `coral guarantee`.
4. Observe: verdict is Green and the exit code is 0, despite the
   "failed to read wiki pages" detail line.

## Suggested fix

1. In `guarantee.rs:179-190`, surface the error as a failure (`failures:
   1`) **or** `anyhow::bail!` so the whole verdict short-circuits.
2. Audit the parallel patterns in the same file: `run_contract_check`,
   any `_check` that calls `walk::read_pages` or otherwise can hit a
   "no signal" Err path.
3. Add a regression test: write a known-broken page into a temp wiki,
   call `guarantee::run`, assert `Verdict::Red` (or
   `ExitCode::FAILURE`).

## Cross-validation

CLI-UX agent flagged this; I verified the swallow pattern directly
at `guarantee.rs:179-190`. The shape (`failures: 0, warnings: 0`
on read error) is unambiguous.
