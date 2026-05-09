//! End-to-end CLI tests for `coral monitor` (v0.23.1).
//!
//! These exercise the binary surface directly: spin up a tempdir,
//! write a `coral.toml` with `[[environments.<env>.monitors]]`,
//! invoke `coral monitor list` / `coral monitor history` / `coral
//! monitor stop` against it. The `up` path needs a live env, so its
//! coverage lives in `crates/coral-cli/src/commands/monitor/up.rs`
//! tests via a synthetic tick fn (see `monitor_loop_*`).

use assert_cmd::Command;
use predicates::str::contains;
use tempfile::TempDir;

/// Write a `coral.toml` with one env containing two monitors. Used
/// across the list / history tests. The `[project]` and `[[repos]]`
/// sections are required by `Project::discover` to recognize the
/// manifest as the project root.
fn fixture_with_monitors(tmp: &TempDir) {
    let toml = r#"apiVersion = "coral.dev/v1"
[project]
name = "demo"

[[repos]]
name = "api"
path = "."

[[environments]]
name = "staging"
backend = "compose"

[environments.services.api]
kind = "real"
image = "api:latest"

[[environments.monitors]]
name = "smoke-loop"
tag = "smoke"
interval_seconds = 60
on_failure = "log"

[[environments.monitors]]
name = "canary"
interval_seconds = 30
on_failure = "fail-fast"
"#;
    std::fs::write(tmp.path().join("coral.toml"), toml).unwrap();
}

/// **T8 — `coral monitor list` shows every declared monitor.** The
/// status column is `stopped` for every row because no JSONL ledger
/// exists yet (no `monitor up` has run).
#[test]
fn monitor_list_shows_declared_monitors() {
    let tmp = TempDir::new().unwrap();
    fixture_with_monitors(&tmp);
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["monitor", "list"])
        .assert()
        .success()
        .stdout(contains("smoke-loop"))
        .stdout(contains("canary"))
        .stdout(contains("stopped"));
}

/// `coral monitor list --env <unknown>` lists no monitors but still
/// exits 0 (the "no monitors declared" message is informational).
#[test]
fn monitor_list_unknown_env_prints_friendly_message() {
    let tmp = TempDir::new().unwrap();
    fixture_with_monitors(&tmp);
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["monitor", "list", "--env", "ghost"])
        .assert()
        .success()
        .stdout(contains("no monitors declared"));
}

/// `coral monitor history` against a non-existent JSONL file prints
/// a friendly error and exits 2.
#[test]
fn monitor_history_missing_file_exits_2() {
    let tmp = TempDir::new().unwrap();
    fixture_with_monitors(&tmp);
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args([
            "monitor",
            "history",
            "--env",
            "staging",
            "--monitor",
            "smoke-loop",
        ])
        .assert()
        .code(2)
        .stderr(contains("no JSONL file"));
}

/// `coral monitor history --tail N` against a fixture JSONL file with
/// 10 lines returns lines 6-10 in original order.
#[test]
fn monitor_history_tail_returns_last_n_lines() {
    let tmp = TempDir::new().unwrap();
    fixture_with_monitors(&tmp);
    let jsonl_dir = tmp.path().join(".coral/monitors");
    std::fs::create_dir_all(&jsonl_dir).unwrap();
    let jsonl_path = jsonl_dir.join("staging-smoke-loop.jsonl");
    let mut text = String::new();
    for i in 1..=10 {
        text.push_str(&format!(
            r#"{{"timestamp":"2026-05-09T12:00:0{}+00:00","env":"staging","monitor_name":"smoke-loop","total":1,"passed":1,"failed":0,"duration_ms":{}}}{}"#,
            i % 10,
            i,
            "\n"
        ));
    }
    std::fs::write(&jsonl_path, &text).unwrap();
    let output = Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args([
            "monitor",
            "history",
            "--env",
            "staging",
            "--monitor",
            "smoke-loop",
            "--tail",
            "5",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let printed: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    // Should be lines 6..=10 by `duration_ms`.
    assert_eq!(printed.len(), 5, "expected 5 lines, got: {printed:?}");
    for (idx, line) in printed.iter().enumerate() {
        let expected_dur = idx + 6; // 6..=10
        assert!(
            line.contains(&format!("\"duration_ms\":{expected_dur}}}")),
            "line {idx} missing duration_ms={expected_dur}: {line}"
        );
    }
}

/// `coral monitor stop` is a v0.23.1 stub — exits 0 with the
/// deferred-message text.
#[test]
fn monitor_stop_is_deferred_stub() {
    let tmp = TempDir::new().unwrap();
    fixture_with_monitors(&tmp);
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args([
            "monitor",
            "stop",
            "--env",
            "staging",
            "--monitor",
            "smoke-loop",
        ])
        .assert()
        .success()
        .stdout(contains("deferred to v0.24+"));
}

/// `coral monitor up --detach` is rejected with the deferred-message.
#[test]
fn monitor_up_detach_is_rejected() {
    let tmp = TempDir::new().unwrap();
    fixture_with_monitors(&tmp);
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["monitor", "up", "--env", "staging", "--detach"])
        .assert()
        .failure()
        .stderr(contains("--detach is deferred to v0.24+"));
}
