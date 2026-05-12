//! End-to-end test for the multi-repo interface change scenario.
//!
//! Simulates a realistic situation: `worker` depends on `api`. Tests
//! in `worker/.coral/tests/` reference `api`'s endpoints. The `api`
//! team changes their OpenAPI spec — removes an endpoint, changes a
//! method, drifts a status code.
//!
//! These tests prove that **Coral detects the drift before the test
//! environment is even brought up**, so CI fails fast with a precise
//! "consumer X expects Y from provider Z but provider only declares W"
//! message instead of a generic 404.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;
use tempfile::TempDir;

/// Helper: write a string to a path, creating parent dirs.
fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

/// v0.34.0 cleanup B2: materialise a real git HEAD before invoking
/// `coral init`. Used by the legacy single-repo regression below.
fn git_init_with_commit(repo: &Path) {
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.email", "interface-test@coral.local"][..],
        &["config", "user.name", "Coral Interface Test"][..],
        &["commit", "-q", "--allow-empty", "-m", "fixture"][..],
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

/// Set up a fixture project with `api` (provider) + `worker`
/// (consumer that depends on api). The provider has 3 endpoints; the
/// consumer's tests reference 2 of them.
fn setup_fixture(dir: &TempDir, api_openapi: &str, worker_tests: &str) {
    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "new", "demo"])
        .current_dir(dir.path())
        .assert()
        .success();
    Command::cargo_bin("coral")
        .unwrap()
        .args([
            "project",
            "add",
            "api",
            "--url",
            "git@example.com:acme/api.git",
        ])
        .current_dir(dir.path())
        .assert()
        .success();
    Command::cargo_bin("coral")
        .unwrap()
        .args([
            "project",
            "add",
            "worker",
            "--url",
            "git@example.com:acme/worker.git",
            "--depends-on",
            "api",
        ])
        .current_dir(dir.path())
        .assert()
        .success();

    write(&dir.path().join("repos/api/openapi.yaml"), api_openapi);
    write(
        &dir.path()
            .join("repos/worker/.coral/tests/api-integration.yaml"),
        worker_tests,
    );
}

const API_OPENAPI_INITIAL: &str = r#"openapi: 3.0.0
info: { title: api, version: 1.0.0 }
paths:
  /users:
    get:
      tags: [api]
      responses:
        '200': { description: ok }
  /users/{id}:
    get:
      tags: [api]
      responses:
        '200': { description: ok }
        '404': { description: not found }
  /orders:
    get:
      tags: [api]
      responses:
        '200': { description: ok }
"#;

const WORKER_TESTS: &str = r#"name: worker integration
service: worker
tags: [smoke]
steps:
  - http: GET /users
    expect:
      status: 200
  - http: GET /users/42
    expect:
      status: 200
"#;

/// Baseline: when api's spec matches what worker tests reference,
/// `coral contract check` exits 0 and reports no findings.
#[test]
fn happy_path_no_drift_when_interfaces_match() {
    let dir = TempDir::new().unwrap();
    setup_fixture(&dir, API_OPENAPI_INITIAL, WORKER_TESTS);

    Command::cargo_bin("coral")
        .unwrap()
        .args(["contract", "check"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("no contract drift"));
}

/// Scenario A: api's team **removes** an endpoint that worker
/// depends on. `coral contract check` must surface the drift with an
/// `Error` severity, including the consumer file path.
#[test]
fn scenario_a_endpoint_removed_is_detected() {
    let dir = TempDir::new().unwrap();
    setup_fixture(&dir, API_OPENAPI_INITIAL, WORKER_TESTS);

    // Change: remove /users/{id} from api's spec.
    let updated = r#"openapi: 3.0.0
info: { title: api, version: 1.0.0 }
paths:
  /users:
    get:
      responses:
        '200': { description: ok }
"#;
    write(&dir.path().join("repos/api/openapi.yaml"), updated);

    let assert = Command::cargo_bin("coral")
        .unwrap()
        .args(["contract", "check"])
        .current_dir(dir.path())
        .assert()
        .failure(); // exit non-zero on hard error
    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("/users/42"),
        "expected the consumer-side path in the report, got: {stdout}"
    );
    assert!(
        stdout.contains("api"),
        "expected the provider repo name in the report, got: {stdout}"
    );
}

/// Scenario B: api's team changes a `GET` to `POST` (idiomatic
/// breaking change). `coral contract check` must surface
/// `UnknownMethod`.
#[test]
fn scenario_b_method_changed_is_detected() {
    let dir = TempDir::new().unwrap();
    setup_fixture(&dir, API_OPENAPI_INITIAL, WORKER_TESTS);

    let updated = r#"openapi: 3.0.0
info: { title: api, version: 1.0.0 }
paths:
  /users:
    post:                                      # was GET, now POST
      responses:
        '201': { description: created }
  /users/{id}:
    get:
      responses:
        '200': { description: ok }
"#;
    write(&dir.path().join("repos/api/openapi.yaml"), updated);

    let assert = Command::cargo_bin("coral")
        .unwrap()
        .args(["contract", "check"])
        .current_dir(dir.path())
        .assert()
        .failure();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        stdout.contains("GET") && stdout.contains("/users") && stdout.contains("POST"),
        "expected method-drift mention; got: {stdout}"
    );
}

/// Scenario C: api's team drifts a status code. The consumer expects
/// 200, the provider now only documents 201. This is a **warning** —
/// not an error — because the runtime test will fail anyway and we
/// don't want to over-report. `--strict` promotes warnings to errors
/// for CI gates.
#[test]
fn scenario_c_status_drift_warns_by_default_errors_in_strict() {
    let dir = TempDir::new().unwrap();
    setup_fixture(&dir, API_OPENAPI_INITIAL, WORKER_TESTS);

    let updated = r#"openapi: 3.0.0
info: { title: api, version: 1.0.0 }
paths:
  /users:
    get:
      responses:
        '201': { description: was-200-now-201 }
        '400': { description: bad request }
  /users/{id}:
    get:
      responses:
        '200': { description: ok }
"#;
    write(&dir.path().join("repos/api/openapi.yaml"), updated);

    // Default mode: warning, exit 0.
    Command::cargo_bin("coral")
        .unwrap()
        .args(["contract", "check"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("⚠"))
        .stdout(predicate::str::contains("200"));

    // Strict mode: warning becomes a failure.
    Command::cargo_bin("coral")
        .unwrap()
        .args(["contract", "check", "--strict"])
        .current_dir(dir.path())
        .assert()
        .failure();
}

/// Scenario D: worker `depends_on api` but api hasn't been synced.
/// `coral contract check` warns about the missing provider rather
/// than silently passing.
#[test]
fn scenario_d_unsynced_provider_is_flagged() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "new", "demo"])
        .current_dir(dir.path())
        .assert()
        .success();
    Command::cargo_bin("coral")
        .unwrap()
        .args([
            "project",
            "add",
            "api",
            "--url",
            "git@example.com:acme/api.git",
        ])
        .current_dir(dir.path())
        .assert()
        .success();
    Command::cargo_bin("coral")
        .unwrap()
        .args([
            "project",
            "add",
            "worker",
            "--url",
            "git@example.com:acme/worker.git",
            "--depends-on",
            "api",
        ])
        .current_dir(dir.path())
        .assert()
        .success();
    write(
        &dir.path()
            .join("repos/worker/.coral/tests/api-integration.yaml"),
        WORKER_TESTS,
    );
    // Note: no repos/api/openapi.yaml on disk.

    let assert = Command::cargo_bin("coral")
        .unwrap()
        .args(["contract", "check"])
        .current_dir(dir.path())
        .assert(); // could be success or failure — depends on whether we promote to error
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        stdout.contains("openapi") || stdout.contains("api"),
        "expected mention of missing provider spec; got: {stdout}"
    );
}

/// Scenario E: JSON output is valid for CI tooling.
#[test]
fn json_output_is_parseable() {
    let dir = TempDir::new().unwrap();
    setup_fixture(&dir, API_OPENAPI_INITIAL, WORKER_TESTS);
    let output = Command::cargo_bin("coral")
        .unwrap()
        .args(["contract", "check", "--format", "json"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(&stdout).expect("contract check --format json must be valid JSON");
    assert!(value.get("findings").is_some());
    assert!(value.get("has_errors").is_some());
}

/// Scenario F: Hurl test files are also scanned. If a `.hurl` file
/// references an endpoint api has removed, contract check catches it.
#[test]
fn hurl_files_are_scanned_too() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "new", "demo"])
        .current_dir(dir.path())
        .assert()
        .success();
    for (name, deps) in [("api", &[][..]), ("worker", &["api"][..])] {
        let mut cmd = Command::cargo_bin("coral").unwrap();
        cmd.args([
            "project",
            "add",
            name,
            "--url",
            &format!("git@example.com:acme/{name}.git"),
        ]);
        if !deps.is_empty() {
            cmd.arg("--depends-on");
            for d in deps {
                cmd.arg(d);
            }
        }
        cmd.current_dir(dir.path()).assert().success();
    }
    // Provider with /health only.
    write(
        &dir.path().join("repos/api/openapi.yaml"),
        r#"openapi: 3.0.0
info: { title: api, version: 1.0.0 }
paths:
  /health:
    get:
      responses: { '200': { description: ok } }
"#,
    );
    // Hurl test references /missing — should be flagged.
    write(
        &dir.path()
            .join("repos/worker/.coral/tests/integration.hurl"),
        r#"GET /missing
HTTP 200
"#,
    );

    let assert = Command::cargo_bin("coral")
        .unwrap()
        .args(["contract", "check"])
        .current_dir(dir.path())
        .assert()
        .failure();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        stdout.contains("/missing"),
        "expected /missing in report; got: {stdout}"
    );
}

/// Scenario G: legacy single-repo project rejects `coral contract
/// check` with a clear error.
#[test]
fn legacy_single_repo_project_rejects_contract_check() {
    let dir = TempDir::new().unwrap();
    git_init_with_commit(dir.path());
    Command::cargo_bin("coral")
        .unwrap()
        .arg("init")
        .current_dir(dir.path())
        .assert()
        .success();
    let assert = Command::cargo_bin("coral")
        .unwrap()
        .args(["contract", "check"])
        .current_dir(dir.path())
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("coral.toml") || stderr.contains("legacy"),
        "expected error mentioning coral.toml/legacy; got: {stderr}"
    );
}
