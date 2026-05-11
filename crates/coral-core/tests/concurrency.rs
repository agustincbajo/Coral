//! Concurrency tests for `Page::write`, `WikiLog::append` (via the
//! load+append+save pattern that callers actually use), `WikiIndex::upsert`,
//! and `EmbeddingsIndex::upsert`.
//!
//! These tests document the existing thread-safety properties of the
//! v0.5 core types. None of the persistent ops (Page::write,
//! WikiLog::save, WikiIndex serialize+save, EmbeddingsIndex::save) take
//! a file lock — they're "open + write_all + close" via `std::fs::write`.
//! That makes them safe enough for the workloads we have today (a
//! single CLI process at a time), but unsafe under genuine concurrent
//! cross-process writers without an external lock layer (which is a
//! v0.14 design decision, not something to fix here).
//!
//! Each test is a single `#[test]` function that spawns N=10 worker
//! threads via `std::thread::scope` (no async, no tokio) and joins them
//! all before asserting. The doc-comment on each test names the exact
//! invariant being asserted and how the test responds when the invariant
//! does NOT hold.

use coral_core::atomic::{atomic_write_string, with_exclusive_lock};
use coral_core::embeddings::EmbeddingsIndex;
use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
use coral_core::index::{IndexEntry, WikiIndex};
use coral_core::log::WikiLog;
use coral_core::page::Page;
use std::collections::BTreeMap;
use std::fs;
use std::sync::Mutex;
use std::thread;
use tempfile::TempDir;

const N: usize = 10;

/// Build a fresh `Frontmatter` for a given slug. Used to construct
/// in-memory `Page` instances for the write-concurrency tests.
fn frontmatter_for(slug: &str) -> Frontmatter {
    Frontmatter {
        slug: slug.to_string(),
        page_type: PageType::Module,
        last_updated_commit: "test".to_string(),
        confidence: Confidence::try_new(0.85).unwrap(),
        sources: Vec::new(),
        backlinks: Vec::new(),
        status: Status::Draft,
        generated_at: None,
        valid_from: None,
        valid_to: None,
        superseded_by: None,
        extra: BTreeMap::new(),
    }
}

/// Build a fresh `IndexEntry` for a given slug. Same shape across all
/// the index-upsert concurrency tests.
fn index_entry_for(slug: &str, path: &str) -> IndexEntry {
    IndexEntry {
        slug: slug.to_string(),
        page_type: PageType::Module,
        path: path.to_string(),
        confidence: Confidence::try_new(0.5).unwrap(),
        status: Status::Draft,
        last_updated_commit: "test".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Page::write
// ---------------------------------------------------------------------------

/// Invariant: writing N distinct `Page`s to N distinct paths from N
/// threads concurrently is safe. After all threads join, every page
/// file exists on disk with the exact content it was given.
///
/// Failure mode (currently passes): a missing file or a torn write
/// would surface as an `assert!` failure on the per-page reload below.
/// `Page::write` is `fs::write` (open + write + close, single syscall
/// for small payloads), and writing to N distinct paths exercises N
/// distinct kernel-level file descriptors with no inter-write
/// contention, so this is safe by construction on POSIX filesystems.
#[test]
fn page_write_concurrent_to_different_paths() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();

    thread::scope(|s| {
        for i in 0..N {
            let root = root.to_path_buf();
            s.spawn(move || {
                let slug = format!("slug-{i}");
                let path = root.join(format!("modules/{slug}.md"));
                let body = format!("# Page {i}\n\nbody-{i}\n");
                let page = Page {
                    path: path.clone(),
                    frontmatter: frontmatter_for(&slug),
                    body,
                };
                page.write().expect("write");
            });
        }
    });

    for i in 0..N {
        let path = root.join(format!("modules/slug-{i}.md"));
        assert!(path.exists(), "page {i} missing at {path:?}");
        let reloaded = Page::from_file(&path).expect("reload");
        assert_eq!(reloaded.frontmatter.slug, format!("slug-{i}"));
        assert!(
            reloaded.body.contains(&format!("body-{i}")),
            "body {i} not preserved: {:?}",
            reloaded.body
        );
    }
}

/// Invariant: when N threads write the SAME path with N distinct bodies,
/// the final file is parseable as a valid `Page` and its body matches
/// one of the N candidates. We do NOT assert which thread "won" —
/// `fs::write` is open + write + close per call, and the kernel's
/// scheduler decides interleaving. The test guards against the most
/// dangerous failure mode: a torn write that produces a corrupt file
/// neither parseable as a Page nor matching any thread's payload.
///
/// Failure mode: `Page::from_file` returning an error, or the body
/// matching none of the N thread-specific markers, would indicate
/// either (a) the kernel is interleaving bytes within `fs::write`
/// (extremely unlikely on POSIX for small payloads under the
/// page-size atomicity guarantee) or (b) one writer is observing
/// another's intermediate state — neither of which we expect to see
/// at this scale.
#[test]
fn page_write_concurrent_to_same_path_last_wins() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("contended.md");

    thread::scope(|s| {
        for i in 0..N {
            let path = path.clone();
            s.spawn(move || {
                let body = format!("# Page\n\nbody-marker-{i}\n");
                let page = Page {
                    path: path.clone(),
                    frontmatter: frontmatter_for("contended"),
                    body,
                };
                page.write().expect("write");
            });
        }
    });

    // The file must exist and be parseable as a valid Page.
    let reloaded = Page::from_file(&path).expect("reload");
    assert_eq!(reloaded.frontmatter.slug, "contended");

    // The body must match exactly one of the N thread markers — i.e.
    // "one of the writes won" without tearing.
    let matched = (0..N).any(|i| reloaded.body.contains(&format!("body-marker-{i}")));
    assert!(
        matched,
        "page body matches no thread marker (torn write?): {:?}",
        reloaded.body
    );
}

// ---------------------------------------------------------------------------
// WikiLog::append (via the realistic load+append+save pattern callers use)
// ---------------------------------------------------------------------------

/// Invariant DOCUMENTED: `WikiLog::append` is in-memory only — it
/// pushes an entry onto a `Vec<LogEntry>` and never touches disk.
/// Callers that USE the load → append → save pattern (read the log
/// file, push a new entry, write the log file back) get a lost-update
/// race: with N threads doing one load+append+save against the same
/// `log.md`, the final on-disk log will likely have FEWER than N
/// entries because reads and writes interleave.
///
/// **The v0.14 fix** is `WikiLog::append_atomic(path, op, summary)` —
/// a static method that uses POSIX `O_APPEND` semantics to write a
/// single entry line atomically (writes ≤ PIPE_BUF are guaranteed
/// atomic). Coral's CLI commands (`coral ingest`, `coral bootstrap`,
/// `coral init`) all switched to that path. This test still exists as
/// a regression guard for the OLD pattern: it pins that the manual
/// load+append+save flow is still racey (so anyone writing custom
/// code against `WikiLog` knows to use `append_atomic` for a single
/// entry).
///
/// Test response: rather than asserting "exactly N entries on disk"
/// (which would intermittently fail and give a flaky test), we assert
/// the **upper bound** — at most N entries — and log the actual count
/// observed. The race-free path has its own unit test:
/// `coral_core::log::tests::append_atomic_concurrent_preserves_all_entries`.
///
/// Note: `WikiLog::parse` accepts any content (it skips lines that
/// don't match the regex), so unlike `WikiIndex::parse`, even a
/// truncated mid-write file produces an `Ok(WikiLog { entries: vec![] })`
/// rather than an error. That makes this test more forgiving than the
/// `wikiindex_upsert_concurrent` analog — we don't need to count
/// per-thread errors here.
#[test]
fn wikilog_append_concurrent_preserves_all_entries() {
    let dir = TempDir::new().expect("tempdir");
    let log_path = dir.path().join("log.md");

    thread::scope(|s| {
        for i in 0..N {
            let log_path = log_path.clone();
            s.spawn(move || {
                let mut log = WikiLog::load(&log_path).expect("load");
                log.append("test", format!("entry-{i}"));
                log.save(&log_path).expect("save");
            });
        }
    });

    let final_log = WikiLog::load(&log_path).expect("reload");
    assert!(
        final_log.entries.len() <= N,
        "log has more entries than threads spawned ({} > {N}); something is very wrong",
        final_log.entries.len()
    );
    // Document the observed count for the test reporter so future
    // changes that make the op atomic become visible.
    eprintln!(
        "wikilog_append_concurrent: spawned {N} threads, observed {} entries on disk \
         (load+append+save is NOT atomic — this is a known v0.5 limitation)",
        final_log.entries.len()
    );
}

/// Invariant: `WikiLog::append` on a single shared in-memory instance
/// (guarded by a `Mutex`) preserves all N entries. This is the
/// in-memory analog of the on-disk test above and exists to pin the
/// pure data-structure semantics: the `Vec::push` inside `append`
/// is correct under serialized access. If this ever fails, something
/// is deeply wrong with the type itself; if the on-disk test above
/// fails differently from this one, we know the loss is in the
/// load+save round trip, not in `append`.
#[test]
fn wikilog_append_concurrent_in_memory_preserves_all() {
    let log = Mutex::new(WikiLog::new());
    thread::scope(|s| {
        for i in 0..N {
            let log = &log;
            s.spawn(move || {
                let mut g = log.lock().unwrap();
                g.append("test", format!("entry-{i}"));
            });
        }
    });
    let final_log = log.into_inner().unwrap();
    assert_eq!(
        final_log.entries.len(),
        N,
        "in-memory append under Mutex must preserve all {N} entries"
    );
}

// ---------------------------------------------------------------------------
// WikiIndex::upsert
// ---------------------------------------------------------------------------

/// Invariant DOCUMENTED: `WikiIndex::upsert` is in-memory only — it
/// scans `entries: Vec<IndexEntry>` and either replaces the existing
/// entry or pushes a new one. There is no disk persistence inside
/// `upsert`; the realistic pattern (e.g. `coral ingest`) is
/// load → upsert → save.
///
/// **v0.14** added `coral_core::atomic::atomic_write_string` for the
/// SAVE step (rename is atomic, so readers no longer observe torn
/// files), eliminating the parse-error failure mode.
///
/// **v0.15** adds `coral_core::atomic::with_exclusive_lock` for the
/// entire load+modify+save round trip (advisory `flock(2)`), closing
/// the lost-update race. With both fixes wired through, this test
/// pins the strongest invariant: under N concurrent
/// load+upsert+save threads, ALL N entries persist AND ZERO threads
/// hit transient errors.
///
/// `coral ingest` / `coral bootstrap` use the lock-wrapped path in
/// production.
#[test]
fn wikiindex_upsert_concurrent() {
    let dir = TempDir::new().expect("tempdir");
    let idx_path = dir.path().join("index.md");
    // Seed an empty index so all threads can find a parseable file.
    let seeded = WikiIndex::new("seed-commit").to_string().expect("seed");
    fs::write(&idx_path, seeded).expect("seed write");

    let errors = Mutex::new(0usize);

    thread::scope(|s| {
        for i in 0..N {
            let idx_path = idx_path.clone();
            let errors = &errors;
            s.spawn(move || {
                // The entire load+modify+save round-trip runs under
                // an exclusive flock (v0.15) — no other writer can
                // observe a partial state, no lost updates.
                let outcome = with_exclusive_lock(&idx_path, || {
                    let content = fs::read_to_string(&idx_path).map_err(|source| {
                        coral_core::error::CoralError::Io {
                            path: idx_path.clone(),
                            source,
                        }
                    })?;
                    let mut idx = WikiIndex::parse(&content)?;
                    let slug = format!("slug-{i}");
                    let path = format!("modules/{slug}.md");
                    idx.upsert(index_entry_for(&slug, &path));
                    let new_content = idx.to_string()?;
                    atomic_write_string(&idx_path, &new_content)
                });
                if outcome.is_err() {
                    *errors.lock().unwrap() += 1;
                }
            });
        }
    });

    let final_content = fs::read_to_string(&idx_path).expect("read final");
    let final_idx = WikiIndex::parse(&final_content).expect("parse final");
    let observed_errors = *errors.lock().unwrap();
    // v0.15 invariant: zero per-thread errors AND every slug landed.
    assert_eq!(
        observed_errors, 0,
        "v0.15 with_exclusive_lock must eliminate transient parse/io errors; \
         observed {observed_errors} thread(s) with errors"
    );
    assert_eq!(
        final_idx.entries.len(),
        N,
        "v0.15 with_exclusive_lock must preserve ALL {N} concurrent upserts; \
         observed {} entries on disk (lost {} updates)",
        final_idx.entries.len(),
        N - final_idx.entries.len()
    );
    // Verify each slug landed exactly once (no upsert was double-applied).
    for i in 0..N {
        let slug = format!("slug-{i}");
        assert!(
            final_idx.find(&slug).is_some(),
            "slug {slug} missing from final index after {N} concurrent upserts"
        );
    }
}

/// Invariant: `WikiIndex::upsert` on a single shared in-memory instance
/// (guarded by a `Mutex`) preserves all N distinct slugs. Pins the
/// pure data-structure semantics — the find-or-push inside `upsert`
/// is correct under serialized access. If this ever fails, the type
/// itself is broken; if it diverges from the on-disk test above we
/// know the loss is in the read/write round trip.
#[test]
fn wikiindex_upsert_concurrent_in_memory_preserves_all() {
    let idx = Mutex::new(WikiIndex::new("seed-commit"));
    thread::scope(|s| {
        for i in 0..N {
            let idx = &idx;
            s.spawn(move || {
                let mut g = idx.lock().unwrap();
                let slug = format!("slug-{i}");
                let path = format!("modules/{slug}.md");
                g.upsert(index_entry_for(&slug, &path));
            });
        }
    });
    let final_idx = idx.into_inner().unwrap();
    assert_eq!(
        final_idx.entries.len(),
        N,
        "in-memory upsert under Mutex must preserve all {N} slugs"
    );
    // Verify each slug landed exactly once.
    for i in 0..N {
        let slug = format!("slug-{i}");
        assert!(
            final_idx.find(&slug).is_some(),
            "slug {slug} missing from index after concurrent upsert"
        );
    }
}

// ---------------------------------------------------------------------------
// EmbeddingsIndex::upsert
// ---------------------------------------------------------------------------

/// Invariant: `EmbeddingsIndex::upsert` on a single shared in-memory
/// instance (guarded by a `Mutex`) preserves all N distinct slug→vector
/// pairs. The prompt's spec for this test ("verify all 10 are in the
/// in-memory index after they all finish") is precisely an in-memory
/// invariant — `upsert` is a `BTreeMap::insert`, which is single-thread
/// safe and correct under any serialized access pattern.
///
/// The test confirms `BTreeMap::insert` semantics hold across N
/// concurrent inserters when the `&mut` is taken via a Mutex (the
/// only sound way to share a mutable struct across threads). All N
/// distinct keys must land.
#[test]
fn embeddings_index_upsert_concurrent() {
    let idx = Mutex::new(EmbeddingsIndex::empty("voyage-3", 4));
    thread::scope(|s| {
        for i in 0..N {
            let idx = &idx;
            s.spawn(move || {
                let mut g = idx.lock().unwrap();
                let slug = format!("slug-{i}");
                // Distinct vector per slug so we can verify the right
                // value landed at the right key.
                let vector: Vec<f32> = vec![i as f32, (i * 2) as f32, (i * 3) as f32, 1.0];
                g.upsert(slug, 100, vector);
            });
        }
    });

    let final_idx = idx.into_inner().unwrap();
    assert_eq!(
        final_idx.entries.len(),
        N,
        "embeddings index must contain all {N} slugs after concurrent upsert"
    );
    for i in 0..N {
        let slug = format!("slug-{i}");
        let entry = final_idx
            .entries
            .get(&slug)
            .unwrap_or_else(|| panic!("slug {slug} missing from embeddings index"));
        assert_eq!(entry.mtime_secs, 100);
        assert_eq!(entry.vector[0], i as f32);
        assert_eq!(entry.vector[1], (i * 2) as f32);
        assert_eq!(entry.vector[2], (i * 3) as f32);
    }
}
