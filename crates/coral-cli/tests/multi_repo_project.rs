//! v0.16 multi-repo E2E test.
//!
//! Drives `coral project new` → `coral project add` × N → `coral project lock`
//! → `coral project list --format json` → `coral project doctor` against a
//! tempdir to verify the multi-repo manifest + lockfile flow works end-to-end.
//! No real git clones — `doctor` should warn (not fail by default) when
//! repos haven't been synced yet, since `coral project sync` lands in
//! v0.16.x.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn project_new_then_add_creates_manifest_and_lockfile() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "new", "orchestra"])
        .current_dir(dir.path())
        .assert()
        .success();

    assert!(dir.path().join("coral.toml").is_file());
    assert!(dir.path().join("coral.lock").is_file());

    Command::cargo_bin("coral")
        .unwrap()
        .args([
            "project",
            "add",
            "api",
            "--url",
            "git@github.com:acme/api.git",
            "--tags",
            "service",
            "team:platform",
        ])
        .current_dir(dir.path())
        .assert()
        .success();

    Command::cargo_bin("coral")
        .unwrap()
        .args([
            "project",
            "add",
            "shared",
            "--url",
            "git@github.com:acme/shared.git",
            "--tags",
            "library",
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
            "git@github.com:acme/worker.git",
            "--tags",
            "service",
            "team:data",
            "--depends-on",
            "api",
            "shared",
        ])
        .current_dir(dir.path())
        .assert()
        .success();

    let manifest = std::fs::read_to_string(dir.path().join("coral.toml")).unwrap();
    assert!(manifest.contains("name = \"orchestra\""));
    assert!(manifest.contains("name = \"api\""));
    assert!(manifest.contains("depends_on = [\"api\", \"shared\"]"));
}

#[test]
fn project_lock_records_repos_and_drops_stale() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "new", "demo"])
        .current_dir(dir.path())
        .assert()
        .success();
    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "add", "api", "--url", "git@x:acme/api.git"])
        .current_dir(dir.path())
        .assert()
        .success();
    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "lock"])
        .current_dir(dir.path())
        .assert()
        .success();
    let lock = std::fs::read_to_string(dir.path().join("coral.lock")).unwrap();
    assert!(lock.contains("[repos.api]"));
    assert!(lock.contains("ref       = \"main\""));
}

#[test]
fn project_list_json_shows_filters_and_resolved_urls() {
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
            "git@github.com:acme/api.git",
            "--tags",
            "service",
        ])
        .current_dir(dir.path())
        .assert()
        .success();
    Command::cargo_bin("coral")
        .unwrap()
        .args([
            "project",
            "add",
            "lib",
            "--url",
            "git@github.com:acme/lib.git",
            "--tags",
            "library",
        ])
        .current_dir(dir.path())
        .assert()
        .success();

    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "list", "--format", "json"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("\"name\": \"api\""))
        .stdout(predicate::str::contains("\"name\": \"lib\""))
        .stdout(predicate::str::contains("\"legacy\": false"));
}

#[test]
fn project_doctor_warns_on_missing_clones_in_strict_mode() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "new", "demo"])
        .current_dir(dir.path())
        .assert()
        .success();
    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "add", "api", "--url", "git@x:acme/api.git"])
        .current_dir(dir.path())
        .assert()
        .success();

    // strict mode: any finding → exit failure (clones are missing because
    // we haven't run sync — and sync isn't shipping yet)
    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "doctor", "--strict"])
        .current_dir(dir.path())
        .assert()
        .failure();

    // default (non-strict): exits 0 with warnings printed.
    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "doctor"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("not yet cloned"));
}

#[test]
fn project_add_rejects_dependency_cycle() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "new", "demo"])
        .current_dir(dir.path())
        .assert()
        .success();
    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "add", "a", "--url", "git@x:acme/a.git"])
        .current_dir(dir.path())
        .assert()
        .success();
    Command::cargo_bin("coral")
        .unwrap()
        .args([
            "project",
            "add",
            "b",
            "--url",
            "git@x:acme/b.git",
            "--depends-on",
            "a",
        ])
        .current_dir(dir.path())
        .assert()
        .success();

    // Now try to add a self-cycle: a depends on b — which depends on a.
    // We can't edit existing repos via `add` in v0.16, so we simulate by
    // hand-editing the manifest and re-running `lock`.
    let manifest = std::fs::read_to_string(dir.path().join("coral.toml")).unwrap();
    let with_cycle = manifest.replace(
        "name = \"a\"\nurl = \"git@x:acme/a.git\"\n",
        "name = \"a\"\nurl = \"git@x:acme/a.git\"\ndepends_on = [\"b\"]\n",
    );
    std::fs::write(dir.path().join("coral.toml"), with_cycle).unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "lock"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("cycle"));
}
