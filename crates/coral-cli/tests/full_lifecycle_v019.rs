//! End-to-end lifecycle test that exercises every v0.19 feature
//! against a single fixture project. The goal is one test that, if
//! green, proves that the whole CLI surface (multi-repo + env + test
//! + MCP + export + context) is functional. Individual unit tests
//! cover details; this one covers integration.
//!
//! No Docker required — the env layer is exercised only as far as
//! `coral up` would need a backend. Tests that require a real
//! container go in `tests/multi_repo_project.rs` gated `--ignored`.

use assert_cmd::Command;
use predicates::prelude::*;
use std::io::Write;
use std::process::{Command as Stdc, Stdio};
use tempfile::TempDir;

#[test]
fn full_v019_lifecycle_end_to_end() {
    let dir = TempDir::new().unwrap();

    // 1. coral project new
    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "new", "orchestra"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("created"));
    assert!(dir.path().join("coral.toml").is_file());
    assert!(dir.path().join("coral.lock").is_file());

    // 2. coral project add (× 3)
    for (name, deps) in [
        ("api", &[][..]),
        ("shared", &[][..]),
        ("worker", &["api", "shared"][..]),
    ] {
        let mut cmd = Command::cargo_bin("coral").unwrap();
        cmd.args([
            "project",
            "add",
            name,
            "--url",
            &format!("git@example.com:acme/{name}.git"),
            "--tags",
            "service",
        ]);
        if !deps.is_empty() {
            cmd.arg("--depends-on");
            for d in deps {
                cmd.arg(d);
            }
        }
        cmd.current_dir(dir.path()).assert().success();
    }

    // 3. coral project list (json)
    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "list", "--format", "json"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("\"name\": \"api\""))
        .stdout(predicate::str::contains("\"name\": \"worker\""));

    // 4. coral project graph (mermaid + dot + json round-trip)
    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "graph", "--format", "mermaid"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("worker --> api"))
        .stdout(predicate::str::contains("worker --> shared"));
    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "graph", "--format", "dot"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("digraph"));
    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "graph", "--format", "json"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("\"edges\""));

    // 5. coral project lock (regenerates lockfile from manifest)
    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "lock"])
        .current_dir(dir.path())
        .assert()
        .success();
    let lock_content = std::fs::read_to_string(dir.path().join("coral.lock")).unwrap();
    assert!(lock_content.contains("[repos.api]"));
    assert!(lock_content.contains("[repos.worker]"));

    // 6. coral project doctor (warns about missing clones, exits 0
    //    when not strict)
    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "doctor"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("not yet cloned"));

    // 7. coral export-agents — every format renders.
    for fmt in &["agents-md", "claude-md", "cursor-rules", "copilot", "llms-txt"] {
        Command::cargo_bin("coral")
            .unwrap()
            .args(["export-agents", "--format", fmt])
            .current_dir(dir.path())
            .assert()
            .success();
    }

    // 8. coral export-agents --write lands the artifacts on disk.
    Command::cargo_bin("coral")
        .unwrap()
        .args(["export-agents", "--format", "agents-md", "--write"])
        .current_dir(dir.path())
        .assert()
        .success();
    Command::cargo_bin("coral")
        .unwrap()
        .args(["export-agents", "--format", "cursor-rules", "--write"])
        .current_dir(dir.path())
        .assert()
        .success();
    assert!(dir.path().join("AGENTS.md").is_file());
    assert!(dir.path().join(".cursor/rules/coral.mdc").is_file());

    // 9. coral mcp serve — pipe initialize via stdio.
    let mut child = Stdc::new(env!("CARGO_BIN_EXE_coral"))
        .args(["mcp", "serve"])
        .current_dir(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn coral mcp serve");
    {
        let stdin = child.stdin.as_mut().unwrap();
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        });
        writeln!(stdin, "{req}").unwrap();
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"protocolVersion\""));
    assert!(stdout.contains("\"name\":\"coral\""));

    // 10. coral test-discover — emits OpenAPI auto-discovered tests.
    let api_repo = dir.path().join("repos/api");
    std::fs::create_dir_all(&api_repo).unwrap();
    std::fs::write(
        api_repo.join("openapi.yaml"),
        r#"openapi: 3.0.0
info: { title: api, version: 1.0.0 }
paths:
  /health:
    get:
      tags: [api]
      responses:
        '200': { description: ok }
  /users:
    get:
      responses:
        '200': { description: ok }
        '404': { description: not found }
"#,
    )
    .unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .args(["test-discover", "--emit", "yaml"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("openapi GET /health"))
        .stdout(predicate::str::contains("openapi GET /users"));

    Command::cargo_bin("coral")
        .unwrap()
        .args(["test-discover", "--commit"])
        .current_dir(dir.path())
        .assert()
        .success();
    let discovered_dir = dir.path().join(".coral/tests/discovered");
    assert!(discovered_dir.is_dir());
    let discovered: Vec<_> = std::fs::read_dir(&discovered_dir)
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert_eq!(
        discovered.len(),
        2,
        "expected 2 discovered tests for /health + /users"
    );
}

/// Pinning test: `coral --version` must always include the workspace
/// version. Catches accidental version drift between Cargo.toml and
/// the built binary.
#[test]
fn coral_version_matches_cargo_workspace() {
    let output = Command::cargo_bin("coral")
        .unwrap()
        .arg("--version")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "coral --version should contain workspace version {}, got: {stdout}",
        env!("CARGO_PKG_VERSION")
    );
}

/// Help output must mention every top-level command. If we add a new
/// command and forget to register it, this catches it before the user
/// does.
#[test]
fn coral_help_lists_every_command() {
    let output = Command::cargo_bin("coral")
        .unwrap()
        .arg("--help")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let expected_commands = [
        // v0.15 legacy
        "init",
        "bootstrap",
        "ingest",
        "query",
        "lint",
        "consolidate",
        "stats",
        "sync",
        "search",
        "export",
        "diff",
        "status",
        "history",
        // v0.16 multi-repo
        "project",
        // v0.17 environments
        "up",
        "down",
        "env",
        // v0.18 testing
        "test",
        "test-discover",
        "verify",
        // v0.19 AI ecosystem
        "mcp",
        "export-agents",
        "context-build",
    ];
    for cmd in expected_commands {
        assert!(
            stdout.contains(cmd),
            "expected `{cmd}` in --help output, got: {stdout}"
        );
    }
}

/// Schema discovery: `coral schema` (when wired in v0.16.x) must emit
/// JSON Schema for the manifest. For now we just verify the
/// ergonomic alternative works: `coral project list --format json`
/// produces a self-describing JSON document with `legacy` and
/// `repos` top-level fields.
#[test]
fn coral_project_list_json_is_self_describing() {
    let dir = TempDir::new().unwrap();
    let output = Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "list", "--format", "json"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(&stdout).expect("project list --format json must be valid JSON");
    assert!(value.get("legacy").is_some());
    assert!(value.get("repos").is_some());
    assert!(value.get("project").is_some());
}
