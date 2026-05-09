//! End-to-end CLI tests for `coral test --emit k6` (v0.22.2).
//!
//! Four tests, mirroring the spec's §5 acceptance items 7-10:
//! 7. `cli_emit_k6_writes_to_stdout_by_default`
//! 8. `cli_emit_k6_with_emit_output_writes_atomically`
//! 9. `cli_emit_k6_zero_matches_exits_2_with_diagnostic`
//! 10. `cli_emit_k6_rejects_format_junit_combo`

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

const FIXTURE_TOML: &str = r#"apiVersion = "coral.dev/v1"
[project]
name = "demo"

[[environments]]
name = "dev"
backend = "compose"

[environments.services.api]
kind = "real"
image = "nginx:latest"
ports = [3000]

[environments.services.api.healthcheck]
kind = "http"
path = "/healthz"
"#;

fn write_fixture(dir: &TempDir) {
    std::fs::write(dir.path().join("coral.toml"), FIXTURE_TOML).unwrap();
}

#[test]
fn cli_emit_k6_writes_to_stdout_by_default() {
    let dir = TempDir::new().unwrap();
    write_fixture(&dir);

    let assert = Command::cargo_bin("coral")
        .unwrap()
        .args(["test", "--emit", "k6"])
        .current_dir(dir.path())
        .assert()
        .success();
    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Acceptance #2: valid k6 JS with the expected scaffolding.
    assert!(stdout.contains("import http from 'k6/http';"));
    assert!(stdout.contains("import { check, sleep } from 'k6';"));
    assert!(stdout.contains("export const options"));
    // The fixture has an HTTP healthcheck on `api` → exactly one body
    // block.
    assert!(stdout.contains("http.get(`${SVC_API_BASE}/healthz`"));
    // Stderr surfaces the emit summary so the user sees skips.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("k6 emit summary: included=1 skipped=0"),
        "stderr: {stderr}"
    );
}

#[test]
fn cli_emit_k6_with_emit_output_writes_atomically() {
    let dir = TempDir::new().unwrap();
    write_fixture(&dir);

    let target = dir.path().join("dist").join("load.js");
    Command::cargo_bin("coral")
        .unwrap()
        .args([
            "test",
            "--emit",
            "k6",
            "--emit-output",
            target.to_str().unwrap(),
        ])
        .current_dir(dir.path())
        .assert()
        .success()
        // Stdout is empty when --emit-output is set.
        .stdout(predicate::str::is_empty());

    assert!(target.is_file(), "{} should exist", target.display());
    let contents = std::fs::read_to_string(&target).unwrap();
    assert!(contents.contains("import http from 'k6/http';"));
    assert!(contents.contains("export const options"));
    // No leftover atomic-write tmp files.
    let parent_entries: Vec<_> = std::fs::read_dir(target.parent().unwrap())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name())
        .collect();
    assert_eq!(
        parent_entries.len(),
        1,
        "expected only target file, got {parent_entries:?}"
    );
}

#[test]
fn cli_emit_k6_zero_matches_exits_2_with_diagnostic() {
    // Service-less project: no healthcheck-bearing service, no
    // user-defined tests under `.coral/tests`. `coral test --emit k6`
    // should exit 2 with a stderr diagnostic that names the active
    // filters (acceptance #10).
    let dir = TempDir::new().unwrap();
    let toml = r#"apiVersion = "coral.dev/v1"
[project]
name = "empty"

[[environments]]
name = "dev"
backend = "compose"

[environments.services.api]
kind = "real"
image = "nginx:latest"
ports = [3000]
"#;
    std::fs::write(dir.path().join("coral.toml"), toml).unwrap();

    Command::cargo_bin("coral")
        .unwrap()
        .args(["test", "--emit", "k6"])
        .current_dir(dir.path())
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no test cases match"))
        .stderr(predicate::str::contains("services="))
        .stderr(predicate::str::contains("tags="));
}

#[test]
fn cli_emit_k6_rejects_format_junit_combo() {
    // Acceptance #9: --format applies to test execution; --emit
    // selects an emitter. Combining --emit k6 with --format junit must
    // exit 2 with a one-line diagnostic that names both flags.
    let dir = TempDir::new().unwrap();
    write_fixture(&dir);

    Command::cargo_bin("coral")
        .unwrap()
        .args(["test", "--emit", "k6", "--format", "junit"])
        .current_dir(dir.path())
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--format"))
        .stderr(predicate::str::contains("--emit"));
}

#[test]
fn cli_emit_k6_rejects_format_json_combo() {
    // v0.22.2 in-cycle fix: ANY non-default `--format` must be
    // rejected when `--emit` is set. Pre-fix only `junit` was caught;
    // `--format json --emit k6` silently produced k6 JS on stdout
    // while the user expected JSON. Mirror the junit case.
    let dir = TempDir::new().unwrap();
    write_fixture(&dir);

    Command::cargo_bin("coral")
        .unwrap()
        .args(["test", "--emit", "k6", "--format", "json"])
        .current_dir(dir.path())
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--format"))
        .stderr(predicate::str::contains("--emit"))
        // Message names the offending format value so the user can
        // see which side of the conflict they typed.
        .stderr(predicate::str::contains("json"));
}

#[test]
fn cli_emit_k6_accepts_default_format_markdown() {
    // Sanity check the gate doesn't over-trigger: `--format markdown`
    // (the clap default) with `--emit k6` is allowed because the user
    // didn't actually pick a competing format. Without this pin the
    // earlier fix risks rejecting every `--emit` invocation since clap
    // populates the default into the field unconditionally.
    let dir = TempDir::new().unwrap();
    write_fixture(&dir);

    Command::cargo_bin("coral")
        .unwrap()
        .args(["test", "--emit", "k6", "--format", "markdown"])
        .current_dir(dir.path())
        .assert()
        .success();
}

// ---------------------------------------------------------------
// `node --check` coverage (v0.22.2 in-cycle fix, MEDIUM 2).
//
// Pre-fix: AC #2 ("emitted JS passes node --check") was only verified
// in dev rehearsal — no automated test invoked node, so the dash-in-
// service-name HIGH bug shipped because nothing was actually feeding
// the emitter output to a JS engine. These tests pin the contract
// against drift. We use `.mjs` (ESM) extension so node --check
// surfaces ESM-only syntax errors (the dash bug only failed under ESM,
// not CJS).
//
// `node` is a hard build dep for the k6 emitter contract but optional
// for non-k6 work — gate-skip cleanly when absent so a developer
// without node can still pass `cargo test`.
// ---------------------------------------------------------------

/// Skip the test (printing a notice) if `node` isn't on PATH. Same
/// pattern as `cargo_release_available()` in `release_flow.rs`.
fn node_available() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn cli_emit_k6_output_passes_node_check_happy_path() {
    if !node_available() {
        eprintln!("SKIP cli_emit_k6_output_passes_node_check_happy_path: node not on PATH");
        return;
    }
    let dir = TempDir::new().unwrap();
    write_fixture(&dir);

    let target = dir.path().join("load.mjs");
    Command::cargo_bin("coral")
        .unwrap()
        .args([
            "test",
            "--emit",
            "k6",
            "--emit-output",
            target.to_str().unwrap(),
        ])
        .current_dir(dir.path())
        .assert()
        .success();

    let node_out = std::process::Command::new("node")
        .args(["--check", target.to_str().unwrap()])
        .output()
        .expect("node --check should run");
    assert!(
        node_out.status.success(),
        "emitted JS failed `node --check`:\n--- stderr ---\n{}\n--- script ---\n{}",
        String::from_utf8_lossy(&node_out.stderr),
        std::fs::read_to_string(&target).unwrap_or_default()
    );
}

#[test]
fn cli_emit_k6_output_passes_node_check_with_dashed_service_name() {
    // HIGH regression test for the v0.22.2 in-cycle fix. Pre-fix this
    // produced `const SVC_MY-API_BASE = ...` which crashed `node
    // --check` with `SyntaxError: Missing initializer in const
    // declaration`. Post-fix: `SVC_MY_API_BASE`, parses cleanly.
    if !node_available() {
        eprintln!(
            "SKIP cli_emit_k6_output_passes_node_check_with_dashed_service_name: node not on PATH"
        );
        return;
    }
    let dir = TempDir::new().unwrap();
    let toml = r#"apiVersion = "coral.dev/v1"
[project]
name = "demo"

[[environments]]
name = "dev"
backend = "compose"

[environments.services."my-api"]
kind = "real"
image = "nginx:latest"
ports = [3000]

[environments.services."my-api".healthcheck]
kind = "http"
path = "/h"
"#;
    std::fs::write(dir.path().join("coral.toml"), toml).unwrap();

    let target = dir.path().join("load.mjs");
    Command::cargo_bin("coral")
        .unwrap()
        .args([
            "test",
            "--emit",
            "k6",
            "--emit-output",
            target.to_str().unwrap(),
        ])
        .current_dir(dir.path())
        .assert()
        .success();

    // Pin the sanitized form: the `_` (not `-`) is the load-bearing
    // change. If the emitter regresses, this assertion catches it
    // before `node --check` even runs.
    let script = std::fs::read_to_string(&target).expect("emitted file");
    assert!(
        script.contains("const SVC_MY_API_BASE = __ENV.CORAL_MY_API_BASE"),
        "dash service name must produce JS-safe ident; got:\n{script}"
    );
    assert!(
        !script.contains("SVC_MY-API_BASE"),
        "must not emit pre-fix unsanitized form; got:\n{script}"
    );

    let node_out = std::process::Command::new("node")
        .args(["--check", target.to_str().unwrap()])
        .output()
        .expect("node --check should run");
    assert!(
        node_out.status.success(),
        "dash-named service emitted JS failed `node --check`:\n--- stderr ---\n{}\n--- script ---\n{}",
        String::from_utf8_lossy(&node_out.stderr),
        script
    );
}
