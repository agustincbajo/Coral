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
