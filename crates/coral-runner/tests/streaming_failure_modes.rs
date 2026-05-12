//! Streaming failure-mode test suite (#31 audit-gap conversion).
//!
//! v0.19.7 cycle-3 audit explicitly listed `coral-runner::run_streaming`
//! as "did NOT examine" for adversarial mid-stream conditions. The
//! auditor traced the streaming code path and confirmed v0.19.5
//! `scrub_secrets` is applied, but did not exercise:
//!
//!   1. Mid-stream truncation: provider closes the stream after a
//!      partial chunk. Specifically — a subprocess that writes some
//!      lines, then exits without writing a final newline.
//!   2. Hang past `prompt.timeout`: subprocess writes nothing and
//!      holds open. The runner must kill it within the deadline.
//!   3. Partial event: subprocess writes bytes WITHOUT a terminating
//!      newline before exit.
//!
//! Coral's `run_streaming_command` reads child stdout line-by-line via
//! `BufReader::read_line`, so the contract pinned here is:
//!
//!   - Lines emitted before EOF reach `on_chunk` in order.
//!   - A trailing partial line (bytes without `\n`) is still surfaced
//!     as a final chunk; `read_line` returns it on EOF.
//!   - When `prompt.timeout` elapses, the runner sends SIGKILL and
//!     returns `RunnerError::Timeout(_)` within ~1s of the deadline.
//!   - `prompt.timeout = None` is honored: the runner waits as long
//!     as the subprocess takes (no spurious early termination).
//!
//! These tests use the `ClaudeRunner` with a tempdir shell script as
//! the binary — the same fixture pattern as
//! `claude_runner_streaming_timeout_kills_child` in
//! `crates/coral-runner/src/runner.rs`. The script contents are
//! adversarial; each test pins one mid-stream failure mode.
//!
//! Why no `wiremock` here: `HttpRunner::run` itself sets `stream: false`
//! at the wire level — Coral does not chunked-decode HTTP SSE today;
//! the streaming layer is purely subprocess-stdout. A future HTTP-SSE
//! runner would need its own adversarial fixtures (#31 follow-up).

#![cfg(unix)]

use coral_runner::{ClaudeRunner, Prompt, Runner, RunnerError, test_script_lock};
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// Build a tempdir shell script with the given body. Sets `0755` and
/// returns the (TempDir, path) pair — caller must keep `TempDir` alive
/// for the test's duration so Drop doesn't unlink the script before
/// it's spawned.
///
/// Linux CI fix: combine `sync_all()` + drop with an outer module
/// mutex (`SCRIPT_LOCK`) held by every test that calls this helper.
/// Belt-and-braces against the ETXTBSY race; the mutex is the
/// authoritative serialiser, the sync_all/drop are defensive in
/// case some future test path bypasses the lock.
fn script(body: &str) -> (TempDir, PathBuf) {
    use std::io::Write as _;

    let dir = tempfile::Builder::new()
        .prefix("coral-stream-")
        .tempdir()
        .expect("tempdir");
    let path = dir.path().join("runner.sh");
    {
        let mut f = std::fs::File::create(&path).expect("create script");
        f.write_all(format!("#!/bin/sh\n{body}\n").as_bytes())
            .expect("write script");
        f.sync_all().expect("sync script");
        // `f` dropped here — write fd released before exec.
    }
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).expect("chmod 755");
    (dir, path)
}

/// #31 (1) — Truncation case A: subprocess writes 2 complete lines
/// then exits cleanly (with the final `\n`). Both lines must reach
/// `on_chunk` and the runner must succeed (no truncation error
/// because there's nothing truncated).
#[test]
fn streaming_two_complete_lines_then_clean_exit() {
    let _lock = test_script_lock();
    let (_dir, script_path) = script("printf 'first\\n'; printf 'second\\n'; exit 0");
    let r = ClaudeRunner::with_binary(&script_path);
    let mut chunks: Vec<String> = Vec::new();
    let prompt = Prompt {
        user: "ignored".into(),
        timeout: Some(Duration::from_secs(5)),
        ..Default::default()
    };
    let out = r
        .run_streaming(&prompt, &mut |c| chunks.push(c.to_string()))
        .expect("clean two-line stream must succeed");
    // Two chunks (each with trailing newline).
    assert_eq!(
        chunks.len(),
        2,
        "expected 2 chunks, got {}: {chunks:?}",
        chunks.len()
    );
    assert_eq!(chunks[0], "first\n");
    assert_eq!(chunks[1], "second\n");
    assert!(out.stdout.contains("first\nsecond\n"));
}

/// #31 (1) — Truncation case B: subprocess writes 2 complete lines
/// THEN bytes without a trailing newline ("third"). The subprocess
/// then exits cleanly (status 0). The runner's line-reader surfaces
/// the trailing partial bytes as a final chunk on EOF.
///
/// This is the contract for "provider closes stream mid-third event":
/// downstream consumers see a chunk that's incomplete BUT they get to
/// inspect it. The crate doesn't synthesize a `StreamTruncated` error
/// today; pinning the current behavior so any future change is
/// intentional.
#[test]
fn streaming_partial_final_line_is_surfaced_on_eof() {
    let _lock = test_script_lock();
    let (_dir, script_path) =
        script("printf 'first\\n'; printf 'second\\n'; printf 'partial-no-newline'; exit 0");
    let r = ClaudeRunner::with_binary(&script_path);
    let mut chunks: Vec<String> = Vec::new();
    let prompt = Prompt {
        user: "ignored".into(),
        timeout: Some(Duration::from_secs(5)),
        ..Default::default()
    };
    let out = r
        .run_streaming(&prompt, &mut |c| chunks.push(c.to_string()))
        .expect("partial trailing bytes must not turn the run into Err");
    // Three chunks: two complete, one partial.
    assert_eq!(
        chunks.len(),
        3,
        "expected 3 chunks (two newline-terminated + 1 partial), got {chunks:?}"
    );
    assert_eq!(chunks[0], "first\n");
    assert_eq!(chunks[1], "second\n");
    assert_eq!(chunks[2], "partial-no-newline");
    assert!(out.stdout.contains("partial-no-newline"));
}

/// #31 (1) — Truncation case C: subprocess crashes mid-line
/// (non-zero exit) AFTER writing a partial line. The runner must
/// surface a non-zero-exit error, NOT silently absorb the partial
/// chunk into a successful result.
///
/// Pre-#31 contract: any non-zero exit code (including the case
/// where the subprocess died with a partial line in its buffer) ->
/// `RunnerError::NonZeroExit` or `RunnerError::AuthFailed`.
#[test]
fn streaming_partial_then_nonzero_exit_returns_err() {
    let _lock = test_script_lock();
    let (_dir, script_path) = script("printf 'first\\n'; printf 'partial-no-newline'; exit 1");
    let r = ClaudeRunner::with_binary(&script_path);
    let mut chunks: Vec<String> = Vec::new();
    let prompt = Prompt {
        user: "ignored".into(),
        timeout: Some(Duration::from_secs(5)),
        ..Default::default()
    };
    let err = r
        .run_streaming(&prompt, &mut |c| chunks.push(c.to_string()))
        .expect_err("non-zero exit must produce Err");
    assert!(
        matches!(
            err,
            RunnerError::NonZeroExit { .. } | RunnerError::AuthFailed(_)
        ),
        "expected NonZeroExit or AuthFailed, got: {err:?}"
    );
    // Pre-error chunks were still delivered to on_chunk in order.
    assert!(!chunks.is_empty(), "first line should still reach on_chunk");
    assert_eq!(chunks[0], "first\n");
}

/// #31 (2) — Hang past timeout: subprocess writes nothing and holds
/// open (sleep 30). With `prompt.timeout = 200ms`, the runner must
/// kill the child within ~1s and return `RunnerError::Timeout(_)`.
///
/// Pinning the deadline guard so a future refactor of the streaming
/// loop can't accidentally let it run unbounded.
#[test]
fn streaming_silent_hang_is_killed_at_timeout() {
    let _lock = test_script_lock();
    // `sleep 30` writes nothing to stdout — the runner's recv_timeout
    // path fires first.
    let (_dir, script_path) = script("sleep 30");
    let r = ClaudeRunner::with_binary(&script_path);
    let mut chunks: Vec<String> = Vec::new();
    let prompt = Prompt {
        user: "ignored".into(),
        timeout: Some(Duration::from_millis(200)),
        ..Default::default()
    };
    let start = Instant::now();
    let err = r
        .run_streaming(&prompt, &mut |c| chunks.push(c.to_string()))
        .expect_err("must time out");
    let elapsed = start.elapsed();
    assert!(
        matches!(err, RunnerError::Timeout(_)),
        "expected Timeout, got: {err:?}"
    );
    assert!(
        // CI runners (especially under shared load and with our crate-
        // wide test_script_lock serialiser holding off concurrent tests)
        // can take noticeably longer than the timeout itself to
        // propagate the kill. 5s gives ~25× the 200ms timeout — more
        // than enough headroom for slow CI without losing the
        // assertion's purpose (verifying the runner doesn't hang
        // open indefinitely past the deadline).
        elapsed < Duration::from_secs(5),
        "should kill well within 5s, took {elapsed:?}"
    );
    assert!(chunks.is_empty(), "silent stream should yield no chunks");
}

/// #31 (2) — Mixed: subprocess emits one line, then hangs. The
/// timeout must fire on the subsequent silence. This pins the
/// total-wall-clock timeout contract: a run that exceeds
/// `prompt.timeout` (whether idle or active) gets killed.
///
/// Note: the timeout is total-wall-clock, NOT idle-since-last-byte
/// (the streaming loop computes `remaining = timeout - start.elapsed()`).
/// On a slow CI runner the shell startup itself can eat the entire
/// 1s budget — so the line MAY or MAY NOT have arrived before the
/// timeout fires. We assert both: timeout fires deterministically,
/// and the pre-hang chunk (if delivered) was the expected `partial\n`.
///
/// v0.19.8 validator follow-up: the script body uses `exec sleep 30`
/// rather than `sleep 30` so `child.kill()` SIGKILLs the actual
/// blocking process. With a plain `sleep 30`, the shell wrapper gets
/// killed but the orphaned `sleep` keeps the stdout pipe alive,
/// which makes `reader_thread.join()` block until sleep exits
/// naturally (~30s). `exec` replaces the shell with sleep so the
/// kill SIGKILLs sleep directly. Real LLM subprocesses don't double-
/// fork — this matches production shape rather than masking the
/// orphan-shell case as flake.
#[test]
fn streaming_one_line_then_hang_is_killed_at_timeout() {
    let _lock = test_script_lock();
    let (_dir, script_path) = script("printf 'partial\\n'; exec sleep 30");
    let r = ClaudeRunner::with_binary(&script_path);
    let mut chunks: Vec<String> = Vec::new();
    let prompt = Prompt {
        user: "ignored".into(),
        timeout: Some(Duration::from_secs(1)),
        ..Default::default()
    };
    let start = Instant::now();
    let err = r
        .run_streaming(&prompt, &mut |c| chunks.push(c.to_string()))
        .expect_err("must time out even after a line");
    let elapsed = start.elapsed();
    assert!(
        matches!(err, RunnerError::Timeout(_)),
        "expected Timeout, got: {err:?}"
    );
    assert!(
        // Headroom for slow CI runners — see the sibling silent_hang
        // test for the rationale.
        elapsed < Duration::from_secs(5),
        "should kill well within 5s, took {elapsed:?}"
    );
    // The pre-hang line either arrived or didn't (depending on shell
    // startup latency vs. timeout budget). When it arrived, it must
    // have been the verbatim "partial\n" — pinning that delivery
    // didn't drop / corrupt the chunk on the way to on_chunk.
    if !chunks.is_empty() {
        assert_eq!(
            chunks,
            vec!["partial\n".to_string()],
            "if a pre-hang chunk was delivered it must be `partial\\n`: {chunks:?}"
        );
    }
}

/// #31 (3) — Many small chunks emitted rapidly: pin that the
/// streaming loop scales reasonably (no accumulator pathology).
/// The script writes 200 short lines as fast as the shell can,
/// then exits. All 200 lines must arrive in order.
#[test]
fn streaming_many_chunks_arrive_in_order() {
    let _lock = test_script_lock();
    let (_dir, script_path) =
        script("i=0; while [ $i -lt 200 ]; do printf 'line-%03d\\n' $i; i=$((i+1)); done; exit 0");
    let r = ClaudeRunner::with_binary(&script_path);
    let mut chunks: Vec<String> = Vec::new();
    let prompt = Prompt {
        user: "ignored".into(),
        timeout: Some(Duration::from_secs(10)),
        ..Default::default()
    };
    r.run_streaming(&prompt, &mut |c| chunks.push(c.to_string()))
        .expect("rapid emission must succeed");
    assert_eq!(
        chunks.len(),
        200,
        "expected 200 chunks, got {}",
        chunks.len()
    );
    for (i, chunk) in chunks.iter().enumerate() {
        let expected = format!("line-{i:03}\n");
        assert_eq!(
            chunk, &expected,
            "chunk {i} mismatch: expected {expected:?}, got {chunk:?}"
        );
    }
}

/// #31 (3) — Empty stream: subprocess writes nothing and exits 0.
/// The runner must succeed with an empty `RunOutput.stdout` and
/// `on_chunk` must be invoked zero times.
#[test]
fn streaming_empty_stdout_clean_exit_succeeds() {
    let _lock = test_script_lock();
    let (_dir, script_path) = script("exit 0");
    let r = ClaudeRunner::with_binary(&script_path);
    let mut chunks: Vec<String> = Vec::new();
    let prompt = Prompt {
        user: "ignored".into(),
        timeout: Some(Duration::from_secs(5)),
        ..Default::default()
    };
    let out = r
        .run_streaming(&prompt, &mut |c| chunks.push(c.to_string()))
        .expect("empty clean stream must succeed");
    assert!(chunks.is_empty(), "no stdout → no chunks: {chunks:?}");
    assert!(out.stdout.is_empty());
}

/// #31 (3) — Stderr-only: subprocess writes only to stderr (which
/// the runner does NOT stream chunk-by-chunk — it captures it after
/// the fact for error messages). The streaming-success path with
/// exit 0 must still succeed and `on_chunk` is not invoked. Stderr
/// is captured into `RunOutput.stderr`.
#[test]
fn streaming_stderr_only_clean_exit_succeeds_without_chunks() {
    let _lock = test_script_lock();
    let (_dir, script_path) = script("printf 'this is stderr\\n' >&2; exit 0");
    let r = ClaudeRunner::with_binary(&script_path);
    let mut chunks: Vec<String> = Vec::new();
    let prompt = Prompt {
        user: "ignored".into(),
        timeout: Some(Duration::from_secs(5)),
        ..Default::default()
    };
    let out = r
        .run_streaming(&prompt, &mut |c| chunks.push(c.to_string()))
        .expect("stderr-only clean exit must succeed");
    assert!(
        chunks.is_empty(),
        "stderr does not feed on_chunk: {chunks:?}"
    );
    assert!(out.stdout.is_empty());
    assert!(
        out.stderr.contains("this is stderr"),
        "stderr should be captured: {:?}",
        out.stderr
    );
}
