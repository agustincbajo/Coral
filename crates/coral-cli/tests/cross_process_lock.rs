//! Cross-process integration test for the v0.15 `with_exclusive_lock`.
//!
//! The unit test in `crates/coral-core/src/atomic.rs::tests::
//! with_exclusive_lock_serializes_concurrent_load_modify_save` spawns
//! N std::thread workers within ONE process. POSIX `flock(2)` works the
//! same across threads-in-process and across processes (it attaches to
//! kernel inodes), but a real cross-process test eliminates the
//! "but it's still one process holding the kernel inode handle" doubt.
//!
//! This test spawns N `coral _test_lock_incr <counter-file>` subprocesses
//! in parallel via `std::thread::scope` + `Command::cargo_bin`. Each
//! process:
//!   1. Acquires `with_exclusive_lock(<counter-file>)`.
//!   2. Reads the counter file as a u64.
//!   3. Increments by 1.
//!   4. Writes back via `atomic_write_string`.
//!   5. Releases the lock and exits.
//!
//! If the lock works at the OS-process boundary, all N increments land
//! and the final counter == N. If it doesn't, we observe the classic
//! lost-update race: final counter < N.
//!
//! `_test_lock_incr` is a hidden subcommand registered in
//! `crates/coral-cli/src/main.rs` with `#[command(hide = true)]` —
//! deliberately not part of the public CLI surface.

use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

/// Number of cooperating processes. Picked to be larger than typical
/// CPU core counts so we actually contend on the lock instead of each
/// process getting its own time slice without contention.
const N_PROCESSES: usize = 16;

#[test]
fn cross_process_lock_serializes_n_subprocess_increments() {
    let dir = TempDir::new().expect("tempdir");
    let counter_path = dir.path().join("counter.txt");
    fs::write(&counter_path, "0").expect("seed counter");

    // Spawn N subprocesses in parallel via std::thread::scope. Each
    // thread spawns ONE `coral _test_lock_incr` subprocess and waits
    // for it to exit.
    std::thread::scope(|s| {
        for _ in 0..N_PROCESSES {
            let counter_path = counter_path.clone();
            s.spawn(move || {
                Command::cargo_bin("coral")
                    .expect("locate coral binary")
                    .arg("_test_lock_incr")
                    .arg(&counter_path)
                    .assert()
                    .success();
            });
        }
    });

    let final_value: u64 = fs::read_to_string(&counter_path)
        .expect("read final counter")
        .trim()
        .parse()
        .expect("parse final counter");

    assert_eq!(
        final_value,
        N_PROCESSES as u64,
        "v0.15 with_exclusive_lock must serialize all {N_PROCESSES} subprocess writers; \
         observed final counter = {final_value} (lost {} updates). \
         Lock failure means cross-process file locking is broken — every concurrent \
         `coral ingest` invocation against the same wiki could lose data.",
        N_PROCESSES as u64 - final_value
    );
}

/// Sanity check: the `_test_lock_incr` helper itself increments the
/// file when invoked once. Confirms the test fixture works before we
/// pile concurrency on top.
#[test]
fn test_lock_incr_helper_increments_counter_once() {
    let dir = TempDir::new().expect("tempdir");
    let counter_path = dir.path().join("counter.txt");
    fs::write(&counter_path, "42").expect("seed");

    Command::cargo_bin("coral")
        .expect("locate coral binary")
        .arg("_test_lock_incr")
        .arg(&counter_path)
        .assert()
        .success();

    let value: u64 = fs::read_to_string(&counter_path)
        .expect("read")
        .trim()
        .parse()
        .expect("parse");
    assert_eq!(value, 43, "helper must increment 42 → 43");
}

/// Negative case: invoking `_test_lock_incr` against a path containing
/// non-numeric content surfaces as a non-zero exit. Pins that the
/// helper validates input rather than silently corrupting state.
#[test]
fn test_lock_incr_helper_rejects_non_numeric_counter() {
    let dir = TempDir::new().expect("tempdir");
    let counter_path = dir.path().join("counter.txt");
    fs::write(&counter_path, "not a number").expect("seed");

    Command::cargo_bin("coral")
        .expect("locate coral binary")
        .arg("_test_lock_incr")
        .arg(&counter_path)
        .assert()
        .failure();
}
