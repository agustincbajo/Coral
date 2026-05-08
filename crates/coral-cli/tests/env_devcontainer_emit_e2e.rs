//! v0.21 `coral env devcontainer emit` E2E.
//!
//! Drives the CLI end-to-end against a tempdir `coral.toml` with an
//! API + Postgres environment. Pins:
//!
//! - Stdout output parses as JSON and contains the selected service.
//! - `--write` lands a well-formed file at the conventional location.
//! - Unknown `--env` exits non-zero with an actionable stderr.
//! - `--service` override survives through to the rendered JSON.

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
image = "node:20"
ports = [3000]

[environments.services.db]
kind = "real"
image = "postgres:16"
ports = [5432]
"#;

fn write_fixture(dir: &TempDir) {
    std::fs::write(dir.path().join("coral.toml"), FIXTURE_TOML).unwrap();
}

#[test]
fn emit_prints_devcontainer_json_to_stdout() {
    let dir = TempDir::new().unwrap();
    write_fixture(&dir);

    let output = Command::cargo_bin("coral")
        .unwrap()
        .args(["env", "devcontainer", "emit", "--env", "dev"])
        .current_dir(dir.path())
        .assert()
        .success()
        .get_output()
        .clone();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout must be valid JSON: {e}; got:\n{stdout}"));

    // Service-selection algorithm: no `repo` set, fall back to alphabetic
    // first real service. `api` < `db`, so `api` wins.
    assert_eq!(
        v.get("service").and_then(|s| s.as_str()),
        Some("api"),
        "service must be the alphabetically-first real service"
    );

    // dockerComposeFile is always a single-element array in v0.21.
    let arr = v
        .get("dockerComposeFile")
        .and_then(|x| x.as_array())
        .unwrap();
    assert_eq!(arr.len(), 1);
    let path = arr[0].as_str().unwrap();
    assert!(
        path.starts_with("../.coral/env/compose/") && path.ends_with(".yml"),
        "expected ../.coral/env/compose/<hash>.yml, got {path}"
    );

    // forwardPorts is the union, deduped + sorted.
    let ports: Vec<u64> = v
        .get("forwardPorts")
        .and_then(|x| x.as_array())
        .unwrap()
        .iter()
        .map(|p| p.as_u64().unwrap())
        .collect();
    assert_eq!(ports, vec![3000, 5432]);
}

#[test]
fn emit_with_write_lands_file_at_conventional_path() {
    let dir = TempDir::new().unwrap();
    write_fixture(&dir);

    Command::cargo_bin("coral")
        .unwrap()
        .args(["env", "devcontainer", "emit", "--env", "dev", "--write"])
        .current_dir(dir.path())
        .assert()
        .success();

    let target = dir.path().join(".devcontainer/devcontainer.json");
    assert!(target.is_file(), "expected file at {}", target.display());

    let content = std::fs::read_to_string(&target).unwrap();
    let v: serde_json::Value = serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("written file must be valid JSON: {e}; got:\n{content}"));

    assert_eq!(v.get("name").and_then(|s| s.as_str()), Some("coral-dev"));
    assert_eq!(v.get("service").and_then(|s| s.as_str()), Some("api"));
}

#[test]
fn emit_with_unknown_env_exits_nonzero_and_lists_available() {
    let dir = TempDir::new().unwrap();
    write_fixture(&dir);

    Command::cargo_bin("coral")
        .unwrap()
        .args(["env", "devcontainer", "emit", "--env", "nonexistent"])
        .current_dir(dir.path())
        .assert()
        .failure()
        // The shared `resolve_env` helper enumerates available envs
        // when the wanted one isn't found.
        .stderr(predicate::str::contains("nonexistent"))
        .stderr(predicate::str::contains("dev"));
}

#[test]
fn emit_service_override_survives_through_write() {
    let dir = TempDir::new().unwrap();
    write_fixture(&dir);

    Command::cargo_bin("coral")
        .unwrap()
        .args([
            "env",
            "devcontainer",
            "emit",
            "--env",
            "dev",
            "--service",
            "db",
            "--write",
        ])
        .current_dir(dir.path())
        .assert()
        .success();

    let content =
        std::fs::read_to_string(dir.path().join(".devcontainer/devcontainer.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(
        v.get("service").and_then(|s| s.as_str()),
        Some("db"),
        "explicit --service override must take precedence over the auto-selection algorithm"
    );
}

#[test]
fn emit_with_unknown_service_override_errors() {
    let dir = TempDir::new().unwrap();
    write_fixture(&dir);

    Command::cargo_bin("coral")
        .unwrap()
        .args([
            "env",
            "devcontainer",
            "emit",
            "--env",
            "dev",
            "--service",
            "ghost",
        ])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("ghost"));
}
