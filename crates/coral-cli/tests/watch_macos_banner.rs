//! v0.21.2: macOS-only banner regression for `coral up --watch`.
//!
//! Acceptance criterion #5: "On macOS, a single `WARNING:` line goes
//! to stderr before the watch subprocess starts, mentioning
//! docker/for-mac#7832 by URL."
//!
//! The whole file is gated `#[cfg(target_os = "macos")]` so this test
//! only runs on Mac runners (Linux CI passes it as a no-op). The
//! banner is emitted before `backend.up` returns, so we can assert
//! the warning shows up even when the rest of the path fails (no
//! docker, no real watch subprocess, etc.).

#![cfg(target_os = "macos")]

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

const MANIFEST_WITH_WATCH: &str = r#"
apiVersion = "coral.dev/v1"

[project]
name = "macos-banner"

[[repos]]
name = "api"
url  = "git@example.com:acme/api.git"

[[environments]]
name    = "dev"
backend = "compose"

[environments.services.api]
kind  = "real"
image = "alpine:latest"

[environments.services.api.watch]
rebuild = ["./Dockerfile"]
"#;

#[test]
fn macos_emits_warning_banner_before_watch_subprocess() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("coral.toml"), MANIFEST_WITH_WATCH).unwrap();
    // No `coral.lock` is needed here — the env layer doesn't consult
    // it for `--watch` validation.

    Command::cargo_bin("coral")
        .unwrap()
        .args(["up", "--watch", "--env", "dev"])
        .current_dir(dir.path())
        .assert()
        .stderr(predicate::str::contains("WARNING:"))
        .stderr(predicate::str::contains(
            "https://github.com/docker/for-mac/issues/7832",
        ));
    // The above invocation will exit non-zero (no docker / no real
    // env) — that's fine. The banner is the only assertion we need.
}
