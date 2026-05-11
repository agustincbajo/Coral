---
title: "concurrency: `coral project lock` reads + mutates + writes `coral.lock` outside any flock — concurrent invocation with `project sync` causes lost-update"
severity: High
labels: bug, concurrency, project
confidence: 5
cross_validated_by: [concurrency-audit-agent, direct-code-read]
---

## Summary

`crates/coral-cli/src/commands/project/sync.rs:112` correctly wraps the
load + modify + save of `coral.lock` in
`coral_core::atomic::with_exclusive_lock(&lockfile_path, …)`. The sibling
command `crates/coral-cli/src/commands/project/lock.rs:25-95` does the
same logical load-modify-save with **no surrounding lock**:

```rust
// crates/coral-cli/src/commands/project/lock.rs:34
let mut lock = Lockfile::load_or_default(&lock_path)
    .with_context(|| format!("loading {}", lock_path.display()))?;

// ... mutate lock (lines 38-87) ...

// crates/coral-cli/src/commands/project/lock.rs:93
lock.write_atomic(&lock_path)
    .with_context(|| format!("writing {}", lock_path.display()))?;
```

`write_atomic` is correctly tmp+rename (so the file on disk is never
torn), but the read-then-write window has no flock. Two concurrent
`coral project lock` invocations — or one `coral project lock` racing
`coral project sync` — will:

1. Both call `load_or_default` and get a snapshot.
2. Both mutate their own copy (one might purge a stale entry that the
   other expected to keep; both might upsert the same repo with
   different SHAs).
3. Both call `write_atomic`. Whichever rename lands second wins. The
   loser's writes are silently lost.

`crates/coral-cli/src/commands/project/add.rs:78` correctly uses
`with_exclusive_lock(&manifest_path, …)` — the lock guard pattern is
present in the codebase. `lock.rs` is the outlier.

The README claims:

> End-to-end concurrency safety: atomic writes (`tmp + rename`),
> cross-process `flock(2)` locking, race-free parallel `coral ingest`
> AND `coral project sync`.

`coral project lock` is omitted from the claim, but a reasonable user
will assume the same guarantee — and the asymmetric defence makes
`sync`'s flock useless against a concurrent `lock`.

## Repro

In a multi-repo project with a populated `coral.toml`:

```bash
# shell 1
coral project sync &
# shell 2
coral project lock &
wait
git diff coral.lock
```

The lockfile loses one process's mutation. The damage is amplified by
the stale-entry purge at `lock.rs:79-87`: a stale entry that `sync`
added moments before `lock` reads its snapshot will be silently dropped
when `lock` writes back.

## Suggested fix

Wrap the load-mutate-save body of `project::lock::run` in
`coral_core::atomic::with_exclusive_lock(&lock_path, || { … })`. Then
re-read inside the closure (the same pattern used by
`project::sync::run`):

```rust
coral_core::atomic::with_exclusive_lock(&lock_path, || -> Result<()> {
    let mut lock = Lockfile::load_or_default(&lock_path)?;
    // ... mutations as today ...
    lock.write_atomic(&lock_path)?;
    Ok(())
})?;
```

Add a regression test mirroring
`crates/coral-cli/tests/cross_process_lock.rs`: spawn N concurrent
`coral project lock` subprocesses and assert that all upserts and all
stale-entry purges from all processes are present in the final file.

## Cross-validation

Concurrency agent flagged this; I verified
`project/sync.rs:99-112` (correctly locked),
`project/add.rs:78` (correctly locked), and `project/lock.rs:25-95`
(no lock) — three sites in the same module, two locked, one not.
