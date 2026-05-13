//! `coral doctor` end-to-end coverage (BACKLOG #5 step 3/4, v0.40.0).
//!
//! Companion to the in-file `#[cfg(test)] mod tests` block in
//! `coral-cli::commands::doctor` — those tests exercise the wizard
//! branches via the `Prompter` abstraction introduced in v0.40.0;
//! these spawn the real `coral` binary so the top-level dispatcher
//! (argument parsing, exit codes, stdout shape) is also covered. The
//! split is intentional: the unit tests verify branch logic; these
//! verify the user-facing CLI contract.
//!
//! Why a dedicated file: `cli_smoke.rs` is reserved for the smallest
//! "the binary builds and prints --version" surface. The doctor tests
//! are O(seconds) because they spawn the binary repeatedly; isolating
//! them in their own file makes the test name in `cargo nextest` legible
//! when one regresses.

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;
use std::path::Path;
use tempfile::TempDir;

/// `coral doctor --wizard` requires a TTY; under `cargo test` stdin is
/// always a pipe, so the wizard refuses with a clear error. Asserts
/// the binary surfaces the same message the in-file unit test verifies
/// at the function level (different layer of the stack — the unit test
/// reaches `run_wizard` directly, this one goes through main + clap).
#[test]
fn doctor_wizard_refuses_non_tty() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["doctor", "--wizard"])
        .assert()
        .failure()
        .stderr(contains("TTY").or(contains("tty")));
}

/// `coral doctor --non-interactive` emits valid JSON that contains the
/// frozen self-check schema fields. Skill consumers (the slash command
/// `/coral:coral-doctor`) JSON-path into this output, so a schema break
/// here breaks the skill — same contract the in-file unit test verifies
/// at the `run_probes()` layer.
#[test]
fn doctor_non_interactive_emits_json_envelope() {
    let tmp = TempDir::new().unwrap();
    let assert = Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["doctor", "--non-interactive"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    // Parse the JSON envelope: must be a single object with the
    // schema_version + coral_status fields present.
    let val: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("doctor --non-interactive must emit valid JSON");
    assert!(
        val.get("schema_version").is_some(),
        "missing schema_version field in: {stdout}"
    );
    assert!(
        val.get("coral_status").is_some(),
        "missing coral_status field in: {stdout}"
    );
    for field in ["providers_configured", "warnings", "suggestions"] {
        assert!(
            val.get(field).is_some(),
            "missing required field `{field}` in: {stdout}"
        );
    }
}

/// `coral doctor` (no flags) prints a human-readable diagnostic header.
/// Exercises the default branch of the dispatcher. We don't assert the
/// full layout — that's an implementation detail of `print_human_report`
/// — only the header line which is part of the user-facing contract
/// (skill description in PRD §7.3 references "Coral doctor — diagnostics").
#[test]
fn doctor_default_prints_diagnostic_header() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("doctor")
        .assert()
        .success()
        .stdout(contains("Coral doctor"))
        .stdout(contains("diagnostics").or(contains("status")));
}

/// Doctor runs successfully against a tempdir that's a git repo. The
/// probe pipeline doesn't require any Coral state — `coral init` is
/// not a prerequisite — so a brand-new git repo is enough to exercise
/// the "git_repo" branch of `run_probes` end-to-end through the binary.
#[test]
fn doctor_default_succeeds_in_fresh_git_repo() {
    let tmp = TempDir::new().unwrap();
    git_init_with_commit(tmp.path());
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("doctor")
        .assert()
        .success()
        .stdout(contains("Coral doctor"));
}

/// The `--non-interactive` schema is stable across two consecutive
/// invocations (no fields are added/removed between calls). This guards
/// against accidentally introducing non-determinism (e.g. a timestamp
/// in the envelope) in a refactor.
#[test]
fn doctor_non_interactive_schema_is_deterministic() {
    let tmp = TempDir::new().unwrap();
    let first = Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["doctor", "--non-interactive"])
        .assert()
        .success();
    let second = Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["doctor", "--non-interactive"])
        .assert()
        .success();
    let a: serde_json::Value =
        serde_json::from_slice(&first.get_output().stdout).expect("a parses");
    let b: serde_json::Value =
        serde_json::from_slice(&second.get_output().stdout).expect("b parses");
    // Same set of top-level keys.
    let a_keys: std::collections::BTreeSet<&str> = a
        .as_object()
        .map(|m| m.keys().map(String::as_str).collect())
        .unwrap_or_default();
    let b_keys: std::collections::BTreeSet<&str> = b
        .as_object()
        .map(|m| m.keys().map(String::as_str).collect())
        .unwrap_or_default();
    assert_eq!(
        a_keys, b_keys,
        "two consecutive `doctor --non-interactive` calls disagree on key set"
    );
}

/// Helper — git init + empty commit so `coral doctor` sees a HEAD.
/// Mirrors the shape `cli_smoke.rs` uses; duplicated here so the file
/// stands alone (the existing helper is `mod`-private to cli_smoke).
fn git_init_with_commit(repo: &Path) {
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.email", "doctor-test@coral.local"][..],
        &["config", "user.name", "Coral Doctor Test"][..],
        &["commit", "-q", "--allow-empty", "-m", "doctor fixture"][..],
    ] {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(repo)
            .status()
            .expect("git invocation failed");
        assert!(
            status.success(),
            "git {args:?} failed in {}",
            repo.display()
        );
    }
}
