---
title: "concurrency+docs: BM25 persisted search index uses non-atomic, unlocked write; `compute_content_hash` docs say SHA-256 but code uses 64-bit SipHash"
severity: Medium
labels: bug, concurrency, search, docs
confidence: 5
cross_validated_by: [concurrency-audit-agent, direct-code-read]
---

## Summary

Two related bugs in `crates/coral-core/src/search_index.rs`.

### A. Non-atomic, unlocked persistence

`SearchIndex::save_index` (lines 178-189) writes the bincoded index
without tmp+rename and without flock:

```rust
pub fn save_index(&self, path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let encoded = bincode::serialize(self).map_err(...)?;
    let mut file = fs::File::create(path)?;  // truncates target on POSIX
    file.write_all(&encoded)?;
    file.sync_all()?;
    Ok(())
}
```

Failure modes:

- A SIGKILL or panic between `create(path)` (which truncates) and
  `write_all` leaves a zero-length or short file. The next
  `load_index` returns `bincode::Error::InvalidData`, which
  `search_with_index` (lines 218-243) silently swallows and falls back
  to a full rebuild — so this is "only" a perf regression, not data
  loss. But on Windows, `File::create` may fail with sharing-violation
  if another process holds the file open, aborting the second writer
  entirely.

- Two concurrent processes can both fail content-hash validation, both
  call `save_index`, and on POSIX a reader that opens between the two
  writes can read interleaved bytes that bincode-decode to garbage.
  Same fallback-to-rebuild masks correctness; the user just pays
  extra latency without knowing why.

The README's blanket "atomic writes (`tmp + rename`)" claim does not
hold for `.coral/search-index.bin`.

### B. `compute_content_hash` doc/implementation mismatch

The doc comment at `crates/coral-core/src/search_index.rs:247` says:

```
SHA-256 of all page bodies concatenated in slug-sorted order.
```

The implementation (lines 250-266) uses `std::hash::DefaultHasher`
(SipHash 1-3, 64-bit output, **NOT** cryptographic), and formats with
`{:016x}` (16 hex chars = 64 bits):

```rust
use std::hash::{DefaultHasher, Hash, Hasher};
…
let mut hasher = DefaultHasher::new();
for (slug, body) in &sorted {
    slug.hash(&mut hasher);
    body.hash(&mut hasher);
}
format!("{:016x}", hasher.finish())
```

64-bit hash means a non-trivial birthday-collision rate at ~4 billion
documents. More immediately: `is_valid_for` (line 207-210) is used as
the cache-validity gate. A collision between two different page sets
will make `load_index` serve a stale index for a wiki it doesn't
match — silent staleness, not a fallback to rebuild.

Probability is small for typical wiki sizes (collisions ~10⁻¹² for
1k pages), but the docs are wrong and the choice of hash should
match the documented contract.

## Suggested fix

For **A**:
```rust
pub fn save_index(&self, path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let encoded = bincode::serialize(self).map_err(...)?;
    coral_core::atomic::atomic_write_bytes(path, &encoded)?;
    Ok(())
}
```
Plus wrap the load+rebuild+save sequence in `search_with_index` in
`with_exclusive_lock(&index_path, …)` so two ingest processes don't
race the rebuild.

For **B**: either switch to a real SHA-256 (e.g. `sha2` crate, single
new dep) and update output to 64 hex chars; or update the doc comment
to "64-bit SipHash digest" and acknowledge the (very small) collision
risk. SHA-256 is the safer change since it matches the existing
contract.

## Cross-validation

Concurrency agent flagged both; I read the file directly. The
`compute_content_hash` doc line is at `search_index.rs:247`, the
implementation at lines 250-266.
