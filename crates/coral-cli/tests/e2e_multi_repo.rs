//! E2E multi-repo sync test.
//!
//! Verifies that `coral sync` lays the embedded template into a fresh
//! consumer repo, and that the same template applied to a second repo
//! yields identical contents (modulo timestamps / version markers).

use coral_cli::commands::sync::{self, SyncArgs};
use std::fs;
use std::sync::Mutex;
use tempfile::TempDir;

/// `sync` calls `current_dir()`, so any test that exercises it must hold this lock.
static CWD_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn sync_reproducible_across_repos() {
    let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let cur = std::env::current_dir().unwrap();

    let a = TempDir::new().unwrap();
    let b = TempDir::new().unwrap();

    // Repo A
    std::env::set_current_dir(a.path()).unwrap();
    sync::run(SyncArgs::default(), None).unwrap();
    let schema_a = fs::read_to_string(a.path().join("template/schema/SCHEMA.base.md")).unwrap();
    let agent_a =
        fs::read_to_string(a.path().join("template/agents/wiki-bibliotecario.md")).unwrap();
    let workflow_a =
        fs::read_to_string(a.path().join("template/workflows/wiki-maintenance.yml")).unwrap();

    // Repo B
    std::env::set_current_dir(b.path()).unwrap();
    sync::run(SyncArgs::default(), None).unwrap();
    let schema_b = fs::read_to_string(b.path().join("template/schema/SCHEMA.base.md")).unwrap();
    let agent_b =
        fs::read_to_string(b.path().join("template/agents/wiki-bibliotecario.md")).unwrap();
    let workflow_b =
        fs::read_to_string(b.path().join("template/workflows/wiki-maintenance.yml")).unwrap();

    // Restore cwd before any assertion that might panic.
    std::env::set_current_dir(&cur).unwrap();

    assert_eq!(schema_a, schema_b, "SCHEMA must be identical across repos");
    assert_eq!(agent_a, agent_b, "Subagent must be identical across repos");
    assert_eq!(
        workflow_a, workflow_b,
        "Workflow must be identical across repos"
    );

    // Both have the version marker.
    assert!(a.path().join(".coral-template-version").exists());
    assert!(b.path().join(".coral-template-version").exists());
}

#[test]
fn sync_pinned_version_matches_binary() {
    let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let cur = std::env::current_dir().unwrap();
    let tmp = TempDir::new().unwrap();
    std::env::set_current_dir(tmp.path()).unwrap();

    sync::run(
        SyncArgs {
            version: Some(format!("v{}", env!("CARGO_PKG_VERSION"))),
            force: false,
        },
        None,
    )
    .unwrap();

    let marker = std::fs::read_to_string(tmp.path().join(".coral-template-version")).unwrap();
    assert_eq!(marker.trim(), format!("v{}", env!("CARGO_PKG_VERSION")));

    std::env::set_current_dir(&cur).unwrap();
}

#[test]
fn sync_pinned_version_mismatch_fails() {
    let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let cur = std::env::current_dir().unwrap();
    let tmp = TempDir::new().unwrap();
    std::env::set_current_dir(tmp.path()).unwrap();

    let res = sync::run(
        SyncArgs {
            version: Some("v999.999.999".into()),
            force: false,
        },
        None,
    );

    std::env::set_current_dir(&cur).unwrap();
    assert!(
        res.is_err(),
        "syncing a pinned version that mismatches the binary must error"
    );
}
