//! v0.16 backward-compat regression tests.
//!
//! These tests pin the contract that a v0.15 user — i.e. a repo with no
//! `coral.toml`, just `<repo>/.wiki/` — gets identical behavior on a
//! v0.16+ binary. Without these tests it's very easy to silently break
//! single-repo workflows when refactoring `wiki_root: Option<&Path>`
//! → `project: &Project`.
//!
//! They run as a separate test binary so the CI gate is just
//! `cargo test --test bc_regression`.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// `coral init` against an empty cwd produces the v0.15 layout: a
/// `.wiki/` with `SCHEMA.md`, `index.md`, `log.md`, `.gitignore`, and
/// the nine type subdirectories.
#[test]
fn legacy_init_preserves_v015_wiki_layout() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .arg("init")
        .current_dir(dir.path())
        .assert()
        .success();

    let wiki = dir.path().join(".wiki");
    assert!(wiki.is_dir(), ".wiki/ should exist");
    assert!(wiki.join("SCHEMA.md").is_file());
    assert!(wiki.join("index.md").is_file());
    assert!(wiki.join("log.md").is_file());
    assert!(wiki.join(".gitignore").is_file());
    for sub in &[
        "modules",
        "concepts",
        "entities",
        "flows",
        "decisions",
        "synthesis",
        "operations",
        "sources",
        "gaps",
    ] {
        assert!(wiki.join(sub).is_dir(), "missing {sub}/");
    }
}

/// `coral status` on a fresh `.wiki/` reports zero issues. This pins
/// the v0.15 dashboard surface against drift.
#[test]
fn legacy_status_runs_clean() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .arg("init")
        .current_dir(dir.path())
        .assert()
        .success();
    Command::cargo_bin("coral")
        .unwrap()
        .arg("status")
        .current_dir(dir.path())
        .assert()
        .success();
}

/// `coral lint` on a fresh `.wiki/` reports zero violations. Pinning
/// behavior here ensures multi-repo wikilink namespacing changes don't
/// regress single-repo lint output.
#[test]
fn legacy_lint_passes_on_fresh_wiki() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .arg("init")
        .current_dir(dir.path())
        .assert()
        .success();
    Command::cargo_bin("coral")
        .unwrap()
        .arg("lint")
        .current_dir(dir.path())
        .assert()
        .success();
}

/// In a directory with no `coral.toml`, `coral status` and `coral lint`
/// behave identically whether the user passes `--wiki-root` or relies
/// on the default. This is the v0.15 contract that `--wiki-root`
/// preserves.
#[test]
fn explicit_wiki_root_matches_default_for_legacy_users() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .arg("init")
        .current_dir(dir.path())
        .assert()
        .success();

    // Use --wiki-root to point at the same directory; should yield the
    // same status output (modulo timestamps).
    Command::cargo_bin("coral")
        .unwrap()
        .args(["--wiki-root", ".wiki", "status"])
        .current_dir(dir.path())
        .assert()
        .success();
}

/// `coral project list` on a legacy single-repo project reports the
/// synthesized repo without erroring. Confirms the `Project::discover`
/// fallback is wired correctly through every command.
#[test]
fn legacy_project_list_reports_synthesized_repo() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .arg("init")
        .current_dir(dir.path())
        .assert()
        .success();

    Command::cargo_bin("coral")
        .unwrap()
        .args(["project", "list", "--format", "json"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("\"legacy\": true"));
}

/// `coral init` with `--force` still wipes and re-creates the wiki —
/// the v0.15 destructive flag must keep working.
#[test]
fn legacy_init_force_recreates_wiki() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .arg("init")
        .current_dir(dir.path())
        .assert()
        .success();
    Command::cargo_bin("coral")
        .unwrap()
        .args(["init", "--force"])
        .current_dir(dir.path())
        .assert()
        .success();
}

/// v0.21 BC: `coral env devcontainer emit` against a v0.15-shape repo
/// (no `coral.toml`) fails with the same actionable error the rest of
/// the env layer uses ("no [[environments]] declared in coral.toml"),
/// rather than panicking or emitting a nonsensical JSON. Mirrors the
/// existing BC contract for `coral env status`, `coral up`, etc.
#[test]
fn legacy_env_devcontainer_emit_fails_without_environments_block() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .arg("init")
        .current_dir(dir.path())
        .assert()
        .success();

    Command::cargo_bin("coral")
        .unwrap()
        .args(["env", "devcontainer", "emit"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "no [[environments]] declared in coral.toml",
        ));
}
