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
use std::path::Path;
use tempfile::TempDir;

/// v0.34.0 cleanup B2: materialise a real git HEAD before invoking
/// `coral init`. Only the MCP-serve test below needs this — the rest
/// of the file drives `coral project *` which is git-agnostic.
fn git_init_with_commit(repo: &Path) {
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.email", "multi-repo-test@coral.local"][..],
        &["config", "user.name", "Coral Multi-Repo Test"][..],
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
fn test_discover_generates_yaml_from_openapi_spec() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "new", "demo"])
        .current_dir(dir.path())
        .assert()
        .success();
    // Drop a tiny OpenAPI fixture under repos/api/.
    let api = dir.path().join("repos/api");
    std::fs::create_dir_all(&api).unwrap();
    std::fs::write(
        api.join("openapi.yaml"),
        r#"openapi: 3.0.0
info: { title: api, version: 1.0.0 }
paths:
  /health:
    get:
      tags: [api]
      responses:
        '200': { description: ok }
"#,
    )
    .unwrap();

    Command::cargo_bin("coral")
        .unwrap()
        .args(["test-discover", "--emit", "yaml"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("openapi GET /health"));

    Command::cargo_bin("coral")
        .unwrap()
        .args(["test-discover", "--commit"])
        .current_dir(dir.path())
        .assert()
        .success();

    let dir_listing = std::fs::read_dir(dir.path().join(".coral/tests/discovered")).unwrap();
    assert!(dir_listing.count() >= 1);
}

#[test]
fn export_agents_md_includes_project_metadata() {
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
        .args(["export-agents", "--format", "agents-md"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("# AGENTS.md"))
        .stdout(predicate::str::contains("`demo`"))
        .stdout(predicate::str::contains("`api`"));

    // --write should land at AGENTS.md
    Command::cargo_bin("coral")
        .unwrap()
        .args(["export-agents", "--format", "agents-md", "--write"])
        .current_dir(dir.path())
        .assert()
        .success();
    assert!(dir.path().join("AGENTS.md").is_file());

    // Cursor rules should land at .cursor/rules/coral.mdc
    Command::cargo_bin("coral")
        .unwrap()
        .args(["export-agents", "--format", "cursor-rules", "--write"])
        .current_dir(dir.path())
        .assert()
        .success();
    assert!(dir.path().join(".cursor/rules/coral.mdc").is_file());
}

#[test]
fn mcp_serve_responds_to_initialize_via_stdio() {
    use std::io::Write;
    use std::process::{Command as Stdc, Stdio};

    let dir = TempDir::new().unwrap();
    git_init_with_commit(dir.path());
    Command::cargo_bin("coral")
        .unwrap()
        .args(["init"])
        .current_dir(dir.path())
        .assert()
        .success();

    // Spawn `coral mcp serve` and pipe a single initialize request.
    let mut child = Stdc::new(env!("CARGO_BIN_EXE_coral"))
        .args(["mcp", "serve"])
        .current_dir(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn coral mcp serve");

    {
        let stdin = child.stdin.as_mut().expect("stdin");
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        });
        writeln!(stdin, "{}", req).unwrap();
    }
    // Drop stdin so the server's stdin loop exits cleanly.
    drop(child.stdin.take());

    let output = child.wait_with_output().expect("wait");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"protocolVersion\""),
        "expected initialize response, got: {stdout}"
    );
    assert!(
        stdout.contains("\"name\":\"coral\""),
        "expected serverInfo, got: {stdout}"
    );
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
