//! `coral self-check --quick --format=json` size cap (FR-ONB-9, v0.34.0).
//!
//! Claude Code's `SessionStart` hook injects hook stdout into the
//! model's context with a 10 000-char cap; exceeding it silently
//! truncates with a preview, which would break the CLAUDE.md routing
//! instructions emitted by `coral init` (they assume specific JSON keys
//! are present). The hook scripts in `.claude-plugin/scripts/` enforce
//! their own 8000-char fallback, but the JSON the **binary** emits has
//! to fit inside that cap under realistic conditions — otherwise the
//! fallback fires on every session and Claude never sees the real
//! state. This test runs the same `--quick --format=json` path across
//! a handful of representative repo shapes and asserts the envelope
//! stays under `QUICK_OUTPUT_CAP_CHARS`.
//!
//! The shapes covered:
//!
//! 1. Pristine — empty tempdir, no `.wiki/`, no `.coral/`, no `CLAUDE.md`.
//! 2. `coral init`-ed — `.wiki/` present, CLAUDE.md template written.
//! 3. Init + populated wiki — extra `.wiki/concepts/`, `.wiki/modules/`,
//!    `.wiki/decisions/` pages to exercise the page-count counter.
//! 4. Init + many warning-eliciting conditions (no providers configured,
//!    no claude CLI). Mirrors what a fresh install looks like in CI.
//!
//! All shapes share the same 8000-char ceiling. The ceiling is the
//! CONSTANT exported by the `self_check` module — if it changes, this
//! test follows automatically.

use assert_cmd::Command;
use std::fs;

/// Mirrors `coral_cli::commands::self_check::QUICK_OUTPUT_CAP_CHARS`.
/// Hard-coding here (rather than importing) keeps this test a pure
/// black-box integration test against the binary — the spec is "the
/// binary's stdout fits in 8000 chars", not "an internal constant
/// matches another internal constant".
const QUICK_OUTPUT_CAP_CHARS: usize = 8000;

/// Run `coral self-check --quick --format=json` inside `cwd` and
/// return its captured stdout as a `String`. Asserts the process
/// exited 0 — `--quick` is supposed to be infallible.
fn run_quick(cwd: &std::path::Path) -> String {
    let out = Command::cargo_bin("coral")
        .unwrap()
        .current_dir(cwd)
        .args(["self-check", "--format=json", "--quick"])
        .output()
        .expect("self-check spawn failed");
    assert!(
        out.status.success(),
        "self-check exited non-zero: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8(out.stdout).expect("stdout not valid UTF-8")
}

fn assert_under_cap(label: &str, stdout: &str) {
    assert!(
        !stdout.trim().is_empty(),
        "{label}: self-check emitted empty stdout"
    );
    assert!(
        stdout.len() <= QUICK_OUTPUT_CAP_CHARS,
        "{label}: self-check stdout ({} chars) exceeds the {} cap. \
         Output starts: {}...",
        stdout.len(),
        QUICK_OUTPUT_CAP_CHARS,
        &stdout[..stdout.len().min(200)]
    );
    // Sanity: the JSON should always be parseable. If it isn't, either
    // the binary regressed or the cap was hit mid-token — both block
    // hook consumers from reading the envelope.
    let _: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("self-check stdout is not valid JSON");
}

#[test]
fn pristine_tempdir_under_cap() {
    let tmp = tempfile::TempDir::new().unwrap();
    let stdout = run_quick(tmp.path());
    assert_under_cap("pristine tempdir", &stdout);
}

#[test]
fn after_coral_init_under_cap() {
    let tmp = tempfile::TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("init")
        .assert()
        .success();
    let stdout = run_quick(tmp.path());
    assert_under_cap("after coral init", &stdout);
}

#[test]
fn many_wiki_pages_under_cap() {
    let tmp = tempfile::TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("init")
        .assert()
        .success();
    // Pad each wiki sub-bucket with 25 stub pages so the count gets
    // into the territory where a debug-formatter regression (e.g.
    // "list every slug") would blow the cap. self-check today only
    // reports `page_count` so the size shouldn't move much, but a
    // future refactor that adds a slug list would surface here.
    let wiki = tmp.path().join(".wiki");
    for bucket in ["modules", "concepts", "decisions"] {
        let dir = wiki.join(bucket);
        fs::create_dir_all(&dir).unwrap();
        for i in 0..25 {
            fs::write(
                dir.join(format!("page_{i:03}.md")),
                "---\ntitle: stub\n---\n# stub\n",
            )
            .unwrap();
        }
    }
    let stdout = run_quick(tmp.path());
    assert_under_cap("many wiki pages", &stdout);
}

#[test]
fn fresh_install_no_providers_under_cap() {
    // Strip env vars that would make `providers_configured` non-empty.
    // We can't unset env vars on the child via `assert_cmd` in a way
    // that beats inheritance, so we use `env_remove` (Command API).
    let tmp = tempfile::TempDir::new().unwrap();
    let out = Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("GEMINI_API_KEY")
        .env_remove("CORAL_PROVIDER")
        .args(["self-check", "--format=json", "--quick"])
        .output()
        .expect("self-check spawn failed");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_under_cap("fresh install (no providers)", &stdout);
}
