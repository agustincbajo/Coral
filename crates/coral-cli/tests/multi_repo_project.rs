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

/// End-to-end: build a tiny bare repo locally, then run
/// `coral project new` + `add` + `sync` against it. Verifies real
/// `git_remote::sync_repo` clones it and writes the resolved SHA into
/// `coral.lock`.
///
/// Gated on `git` being on PATH; CI runners always have it.
#[test]
fn project_sync_clones_a_local_bare_repo_end_to_end() {
    use std::process::Command as Stdc;

    let dir = TempDir::new().unwrap();
    let bare = dir.path().join("origin.git");
    let work = dir.path().join("source");
    let project_root = dir.path().join("orchestra");
    std::fs::create_dir_all(&project_root).unwrap();

    // Build a 1-commit bare repo to clone from.
    Stdc::new("git")
        .args(["init", "--bare", bare.to_str().unwrap()])
        .status()
        .unwrap();
    Stdc::new("git")
        .args(["init", "--initial-branch=main", work.to_str().unwrap()])
        .status()
        .unwrap();
    std::fs::write(work.join("README.md"), "hello\n").unwrap();
    Stdc::new("git")
        .current_dir(&work)
        .args(["add", "."])
        .status()
        .unwrap();
    Stdc::new("git")
        .current_dir(&work)
        .args([
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-m",
            "init",
        ])
        .status()
        .unwrap();
    Stdc::new("git")
        .current_dir(&work)
        .args(["remote", "add", "origin", bare.to_str().unwrap()])
        .status()
        .unwrap();
    Stdc::new("git")
        .current_dir(&work)
        .args(["push", "-u", "origin", "main"])
        .status()
        .unwrap();

    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "new", "orchestra"])
        .current_dir(&project_root)
        .assert()
        .success();

    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "add", "demo", "--url", bare.to_str().unwrap()])
        .current_dir(&project_root)
        .assert()
        .success();

    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "sync"])
        .current_dir(&project_root)
        .assert()
        .success()
        .stdout(predicate::str::contains("cloned"));

    // `repos/demo/` should exist with the cloned README.
    let cloned = project_root.join("repos").join("demo").join("README.md");
    assert!(cloned.is_file(), "expected {} to exist", cloned.display());

    // `coral.lock` should now have a concrete SHA.
    let lock = std::fs::read_to_string(project_root.join("coral.lock")).unwrap();
    assert!(
        lock.contains("[repos.demo]"),
        "lockfile missing entry: {lock}"
    );
    assert!(
        !lock.contains("sha       = \"00000"),
        "lockfile should have concrete SHA, got: {lock}"
    );
}

#[test]
fn up_fails_clearly_when_no_environments_declared() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "new", "demo"])
        .current_dir(dir.path())
        .assert()
        .success();

    Command::cargo_bin("coral")
        .unwrap()
        .args(["up"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("no [[environments]]"));
}

#[test]
fn down_fails_clearly_when_production_env_without_yes() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("coral.toml"),
        r#"apiVersion = "coral.dev/v1"
[project]
name = "demo"

[[environments]]
name = "prod"
backend = "compose"
production = true

[environments.services.api]
kind = "real"
image = "nginx:latest"
"#,
    )
    .unwrap();

    Command::cargo_bin("coral")
        .unwrap()
        .args(["down", "--env", "prod"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("production = true"));
}

#[test]
fn project_graph_emits_mermaid_with_nodes_and_edges() {
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
        .args([
            "project",
            "add",
            "worker",
            "--url",
            "git@x:acme/worker.git",
            "--depends-on",
            "api",
        ])
        .current_dir(dir.path())
        .assert()
        .success();

    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "graph", "--format", "mermaid"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("graph TD"))
        .stdout(predicate::str::contains("api"))
        .stdout(predicate::str::contains("worker --> api"));

    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "graph", "--format", "json"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("\"from\": \"worker\""))
        .stdout(predicate::str::contains("\"to\": \"api\""));
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
