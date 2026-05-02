//! Cross-runner contract suite — verifies that every concrete `Runner`
//! implementation honors the same observable behaviors at the trait
//! boundary, so that callers can swap one runner for another without
//! surprises.
//!
//! The trait contract being asserted (per-runner specializations noted):
//!
//! 1. **Empty user prompt** — calling `run` with `Prompt::default()`
//!    (which has `user = ""`) must NOT panic. Either it returns `Ok(_)`
//!    from a substitute binary (echo prints just the flags) or it
//!    surfaces an `Err`. Both are acceptable; the invariant is "no
//!    panic".
//!
//! 2. **NotFound on bogus binary path** — runners that spawn a
//!    subprocess (Claude, Gemini, Local) map `ErrorKind::NotFound` from
//!    `Command::spawn` to `RunnerError::NotFound`. HttpRunner has no
//!    binary (it shells `curl` and reports endpoint failures as
//!    `NonZeroExit` / `AuthFailed`). MockRunner doesn't spawn anything,
//!    so this leg is exercised by `push_err(RunnerError::NotFound)`.
//!
//! 3. **`Prompt::default()` is a valid input shape** — every runner
//!    accepts a fully-default `Prompt` without panicking. This is a
//!    second guard on top of (1) that uses the literal `Default` impl
//!    rather than a hand-built struct, in case `Prompt::default` ever
//!    diverges from `Prompt { ..Default::default() }`.
//!
//! 4. **`run_streaming` emits at least one chunk on the OK path** —
//!    the trait's default implementation calls `on_chunk(&out.stdout)`
//!    once after `run` returns Ok. Custom overrides (Claude / Gemini /
//!    Local) read stdout line-by-line and emit per-line chunks. Either
//!    way, an OK invocation that produces ANY stdout must surface at
//!    least one chunk.
//!
//!    Per-runner specialization:
//!    - Claude / Gemini / Local: spawn `/bin/echo`, which prints a
//!      newline-terminated line of args, so streaming sees ≥ 1 chunk.
//!    - Mock: pre-load a chunked OK response.
//!    - Http: cannot reach an OK path without booting an HTTP server.
//!      We exercise the streaming surface by pointing at an unreachable
//!      endpoint and asserting it returns `Err` without panicking
//!      (which is the equivalent shape — the production contract is
//!      "no-panic; surface error" rather than "always emit a chunk on
//!      every input"). The OK-path streaming is already covered by the
//!      wiremock-based suite at `tests/wiremock_http.rs`.
//!
//! 5. **`timeout: Some(0)` returns `Timeout` quickly** — for runners
//!    that honor `prompt.timeout` via the shared `run_streaming_command`
//!    helper or their own poll loop (Claude / Gemini / Local), a
//!    zero-duration timeout MUST surface as `RunnerError::Timeout`
//!    (not panic, not hang). Verified against `/usr/bin/yes`, which
//!    writes "y\n" forever and ignores its argv — exactly the
//!    long-running fixture this leg needs.
//!
//!    HttpRunner does not honor `prompt.timeout` in v0.5 (the
//!    `curl` subprocess runs to completion); MockRunner has no notion
//!    of wall-clock timeout. Both runners SKIP this leg with a
//!    documented justification rather than asserting non-applicable
//!    behavior. If either gains timeout support later, add the assert
//!    back.

use coral_runner::{
    ClaudeRunner, GeminiRunner, HttpRunner, LocalRunner, MockRunner, Prompt, Runner, RunnerError,
};
use std::time::Duration;

mod common;
use common::forever_yes_script;

/// Shared assertion 1: empty / default `Prompt` does not panic.
/// The invariant is "no panic", not "returns Ok" — substitute binaries
/// will likely surface Err for an empty argv. Either is acceptable.
fn assert_empty_prompt_no_panic<R: Runner>(runner: &R, _name: &str) {
    let prompt = Prompt::default();
    // Whatever the runner returns, the call must complete (no panic).
    let _ = runner.run(&prompt);
}

/// Shared assertion 3: literal `Prompt::default()` is a valid input shape.
/// Same observable as (1) but with `Prompt::default()` directly to guard
/// against drift between `Default::default()` and `Prompt { .. }`.
fn assert_prompt_default_works<R: Runner>(runner: &R, _name: &str) {
    let prompt: Prompt = Prompt::default();
    let _ = runner.run(&prompt);
}

/// Shared assertion 4: `run_streaming` of a successful invocation emits
/// at least one chunk. Skipped when `run` itself errors (we'd never
/// reach the chunk-emit path, which is fine — we only assert the
/// invariant for the OK case).
fn assert_streaming_emits_chunk_on_ok<R: Runner>(runner: &R, prompt: &Prompt) {
    let mut chunks: Vec<String> = Vec::new();
    let res = runner.run_streaming(prompt, &mut |c| chunks.push(c.to_string()));
    if res.is_ok() {
        assert!(
            !chunks.is_empty(),
            "OK run_streaming must emit at least one chunk; got 0"
        );
    }
}

// ---------------------------------------------------------------------------
// ClaudeRunner — uses /bin/echo as the substitute binary.
// ---------------------------------------------------------------------------

/// Verifies ClaudeRunner honors all five contract clauses, using
/// `/bin/echo` as a stand-in for the real `claude` CLI and `/usr/bin/yes`
/// for the timeout leg. Both binaries are present on macOS + Linux.
#[test]
fn claude_runner_honors_contract() {
    let r = ClaudeRunner::with_binary("/bin/echo");

    // (1) empty prompt — must not panic.
    assert_empty_prompt_no_panic(&r, "claude");

    // (2) NotFound on bogus binary.
    let bogus = ClaudeRunner::with_binary("/nonexistent/coral-cross-claude-xyz");
    let err = bogus
        .run(&Prompt {
            user: "x".into(),
            ..Default::default()
        })
        .unwrap_err();
    assert!(
        matches!(err, RunnerError::NotFound),
        "ClaudeRunner with bogus binary must return NotFound, got: {err:?}"
    );

    // (3) Prompt::default() works as an input shape.
    assert_prompt_default_works(&r, "claude");

    // (4) Streaming OK path emits ≥ 1 chunk. /bin/echo prints
    // "--print hello\n" → reader sees one line → one chunk.
    let ok_prompt = Prompt {
        user: "hello".into(),
        ..Default::default()
    };
    assert_streaming_emits_chunk_on_ok(&r, &ok_prompt);

    // (5) timeout: Some(0) — must surface as Timeout quickly against
    // a long-running command. Using a tempdir shell script that ignores
    // every CLI arg and writes "y\n" forever; see `common::forever_yes_script`
    // for why /usr/bin/yes itself doesn't work on Linux.
    let (_dir, script) = forever_yes_script();
    let timeout_runner = ClaudeRunner::with_binary(&script);
    let timeout_prompt = Prompt {
        user: "ignored".into(),
        timeout: Some(Duration::from_millis(0)),
        ..Default::default()
    };
    let start = std::time::Instant::now();
    let err = timeout_runner.run(&timeout_prompt).unwrap_err();
    let elapsed = start.elapsed();
    assert!(
        matches!(err, RunnerError::Timeout(_)),
        "ClaudeRunner with timeout=0 must return Timeout, got: {err:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "ClaudeRunner Timeout(0) must fire quickly, took {elapsed:?}"
    );
}

// ---------------------------------------------------------------------------
// GeminiRunner — uses /bin/echo as the substitute binary.
// ---------------------------------------------------------------------------

/// Verifies GeminiRunner honors all five contract clauses with `/bin/echo`
/// substitution. Same pattern as the Claude leg.
#[test]
fn gemini_runner_honors_contract() {
    let r = GeminiRunner::with_binary("/bin/echo");

    assert_empty_prompt_no_panic(&r, "gemini");

    let bogus = GeminiRunner::with_binary("/nonexistent/coral-cross-gemini-xyz");
    let err = bogus
        .run(&Prompt {
            user: "x".into(),
            ..Default::default()
        })
        .unwrap_err();
    assert!(
        matches!(err, RunnerError::NotFound),
        "GeminiRunner with bogus binary must return NotFound, got: {err:?}"
    );

    assert_prompt_default_works(&r, "gemini");

    let ok_prompt = Prompt {
        user: "hello".into(),
        ..Default::default()
    };
    assert_streaming_emits_chunk_on_ok(&r, &ok_prompt);

    let (_dir, script) = forever_yes_script();
    let timeout_runner = GeminiRunner::with_binary(&script);
    let timeout_prompt = Prompt {
        user: "ignored".into(),
        timeout: Some(Duration::from_millis(0)),
        ..Default::default()
    };
    let start = std::time::Instant::now();
    let err = timeout_runner.run(&timeout_prompt).unwrap_err();
    let elapsed = start.elapsed();
    assert!(
        matches!(err, RunnerError::Timeout(_)),
        "GeminiRunner with timeout=0 must return Timeout, got: {err:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "GeminiRunner Timeout(0) must fire quickly, took {elapsed:?}"
    );
}

// ---------------------------------------------------------------------------
// LocalRunner — uses /bin/echo as the substitute binary.
// ---------------------------------------------------------------------------

/// Verifies LocalRunner honors all five contract clauses with `/bin/echo`
/// substitution. Same pattern as the Claude/Gemini legs.
#[test]
fn local_runner_honors_contract() {
    let r = LocalRunner::with_binary("/bin/echo");

    assert_empty_prompt_no_panic(&r, "local");

    let bogus = LocalRunner::with_binary("/nonexistent/coral-cross-local-xyz");
    let err = bogus
        .run(&Prompt {
            user: "x".into(),
            ..Default::default()
        })
        .unwrap_err();
    assert!(
        matches!(err, RunnerError::NotFound),
        "LocalRunner with bogus binary must return NotFound, got: {err:?}"
    );

    assert_prompt_default_works(&r, "local");

    let ok_prompt = Prompt {
        user: "hello".into(),
        ..Default::default()
    };
    assert_streaming_emits_chunk_on_ok(&r, &ok_prompt);

    let (_dir, script) = forever_yes_script();
    let timeout_runner = LocalRunner::with_binary(&script);
    let timeout_prompt = Prompt {
        user: "ignored".into(),
        timeout: Some(Duration::from_millis(0)),
        ..Default::default()
    };
    let start = std::time::Instant::now();
    let err = timeout_runner.run(&timeout_prompt).unwrap_err();
    let elapsed = start.elapsed();
    assert!(
        matches!(err, RunnerError::Timeout(_)),
        "LocalRunner with timeout=0 must return Timeout, got: {err:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "LocalRunner Timeout(0) must fire quickly, took {elapsed:?}"
    );
}

// ---------------------------------------------------------------------------
// HttpRunner — uses an unreachable URL (port 1) instead of a binary.
// ---------------------------------------------------------------------------

/// Verifies HttpRunner honors the contract clauses that apply to it.
/// Skips clauses that don't apply with documented justification:
/// - clause (2) "NotFound on bogus binary" doesn't map — HttpRunner has
///   no binary, only an endpoint; bad endpoints surface as
///   `NonZeroExit` (curl exits non-zero on connection refused) or
///   `AuthFailed` (if the endpoint somehow returns 401-shaped data).
///   We DO assert the analogous "endpoint-error returns Err, not panic".
/// - clause (5) "timeout: Some(0)" doesn't map — HttpRunner does not
///   honor `prompt.timeout` in v0.5 (the curl subprocess runs to
///   completion). If timeout support is added, expand this leg.
#[test]
fn http_runner_honors_contract() {
    // Port 1 is a privileged port that's virtually never bound; curl
    // will fail to connect quickly. This exercises the curl-spawn +
    // error-mapping path without needing a live HTTP server.
    let r = HttpRunner::new("http://127.0.0.1:1/v1/chat/completions");

    // (1) empty prompt — must not panic. Will return Err (curl can't
    // connect), but that's fine.
    assert_empty_prompt_no_panic(&r, "http");

    // (2) Bogus-endpoint analog: `run` must return Err (any flavor)
    // without panicking. Connection refused → `NonZeroExit`, in-line
    // 401 from a coincidentally-bound port → `AuthFailed`.
    let res = r.run(&Prompt {
        user: "x".into(),
        ..Default::default()
    });
    assert!(
        res.is_err(),
        "HttpRunner with unreachable endpoint must return Err, got: {res:?}"
    );

    // (3) Prompt::default() works as an input shape.
    assert_prompt_default_works(&r, "http");

    // (4) Streaming surface: contract is "no panic". An unreachable
    // endpoint will surface Err from the underlying `run`, so we don't
    // get to the chunk-emit path. The wiremock-based test suite asserts
    // the OK-path emits a chunk; here we just confirm the streaming
    // call doesn't panic on the error path.
    let stream_prompt = Prompt {
        user: "hello".into(),
        ..Default::default()
    };
    let mut chunks: Vec<String> = Vec::new();
    let stream_res = r.run_streaming(&stream_prompt, &mut |c| chunks.push(c.to_string()));
    // Either Err (expected for unreachable URL) or Ok (impossible here
    // unless port 1 is somehow bound). Either way, no panic.
    assert!(
        stream_res.is_err(),
        "HttpRunner streaming an unreachable URL must return Err, got: {stream_res:?}"
    );

    // (5) Skipped — HttpRunner does not honor prompt.timeout in v0.5.
    // See the doc comment above.
}

// ---------------------------------------------------------------------------
// MockRunner — programmable in-process runner; no binary, no endpoint.
// ---------------------------------------------------------------------------

/// Verifies MockRunner honors the contract clauses. Per-clause notes:
/// - (1) empty prompt — `run` returns the default empty `RunOutput` if
///   the queue is empty; either way, no panic.
/// - (2) NotFound — exercised by pushing a `NotFound` error onto the
///   queue and asserting the runner faithfully relays it.
/// - (3) `Prompt::default()` — same shape as (1).
/// - (4) Streaming — `push_ok_chunked` ensures streaming emits the
///   queued chunks; the default `push_ok` emits a single chunk equal
///   to the full stdout (covered too).
/// - (5) timeout — MockRunner has no notion of wall-clock timeout
///   (it doesn't sleep or block). Skipped with documented rationale.
#[test]
fn mock_runner_honors_contract() {
    let r = MockRunner::new();

    // (1) empty prompt — empty queue, returns the default empty
    // RunOutput without panicking.
    assert_empty_prompt_no_panic(&r, "mock");

    // (2) NotFound — pushed error must propagate.
    r.push_err(RunnerError::NotFound);
    let err = r
        .run(&Prompt {
            user: "x".into(),
            ..Default::default()
        })
        .unwrap_err();
    assert!(
        matches!(err, RunnerError::NotFound),
        "MockRunner must propagate pushed NotFound, got: {err:?}"
    );

    // (3) Prompt::default() — empty queue, default response.
    let r2 = MockRunner::new();
    assert_prompt_default_works(&r2, "mock");

    // (4a) Streaming with `push_ok_chunked` emits each pushed chunk.
    let r3 = MockRunner::new();
    r3.push_ok_chunked(vec!["chunk-a", "chunk-b"]);
    let stream_prompt = Prompt {
        user: "x".into(),
        ..Default::default()
    };
    let mut chunks: Vec<String> = Vec::new();
    let out = r3
        .run_streaming(&stream_prompt, &mut |c| chunks.push(c.to_string()))
        .unwrap();
    assert!(
        !chunks.is_empty(),
        "MockRunner OK run_streaming must emit at least one chunk; got 0"
    );
    assert_eq!(out.stdout, "chunk-achunk-b");

    // (4b) Streaming with `push_ok` (single response, no chunks) emits
    // exactly one chunk equal to the full stdout — the default trait
    // behavior surfaced via the mock.
    let r4 = MockRunner::new();
    r4.push_ok("single");
    let mut chunks2: Vec<String> = Vec::new();
    let _ = r4
        .run_streaming(&stream_prompt, &mut |c| chunks2.push(c.to_string()))
        .unwrap();
    assert_eq!(
        chunks2,
        vec!["single".to_string()],
        "MockRunner push_ok streaming must emit single chunk = full stdout"
    );

    // (5) Skipped — MockRunner has no wall-clock timeout semantics.
    // It returns whatever was queued; nothing to time out.
}
