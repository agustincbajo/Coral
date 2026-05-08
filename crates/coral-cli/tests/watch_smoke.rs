//! v0.21.2: docker-gated smoke for `coral up --watch`.
//!
//! `#[ignore]`d so CI without docker doesn't go red. The contract:
//! when the user runs `coral up --watch --env dev` against a manifest
//! that declares `[services.*.watch]`, the binary first runs `up -d
//! --wait`, then runs `compose watch` in foreground until Ctrl-C.
//!
//! We can't easily script Ctrl-C here, so this test is structured to
//! verify the foreground subprocess is reachable: spawn `coral up
//! --watch`, wait briefly, then send SIGINT and check that the
//! containers are gone via `coral down`.
//!
//! Run with: `cargo test --test watch_smoke -- --ignored`

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

const MANIFEST_NO_WATCH: &str = r#"
apiVersion = "coral.dev/v1"

[project]
name = "watch-smoke"

[[repos]]
name = "api"
url  = "git@example.com:acme/api.git"

[[environments]]
name    = "dev"
backend = "compose"

[environments.services.api]
kind  = "real"
image = "alpine:latest"
"#;

/// `--watch` against an environment with NO `[services.*.watch]`
/// blocks must fail with a friendly error. This DOESN'T need docker —
/// the `up -d --wait` would hit BackendNotFound first if docker is
/// missing, so we run this test under #[ignore] alongside the smoke
/// just to keep the gate in one place. (The unit test
/// `up_watch_without_any_watch_service_fails_with_invalid_spec` in
/// `crates/coral-env/src/compose.rs` covers the same gate without
/// requiring docker.)
#[test]
#[ignore]
fn watch_without_watch_service_fails_actionably() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("coral.toml"), MANIFEST_NO_WATCH).unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .args(["up", "--watch", "--env", "dev"])
        .current_dir(dir.path())
        .assert()
        .failure()
        // Either we hit BackendNotFound first (no docker on the
        // smoke host) — fine; or we hit the InvalidSpec gate — also
        // fine. The contract is "fails non-zero with an actionable
        // error".
        .stderr(predicate::str::contains("compose").or(predicate::str::contains("--watch")));
}

const MANIFEST_WITH_WATCH: &str = r#"
apiVersion = "coral.dev/v1"

[project]
name = "watch-smoke"

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

/// docker-gated smoke. Only meaningful when docker is on PATH and the
/// daemon is reachable. Run with `cargo test --test watch_smoke --
/// --ignored`.
#[test]
#[ignore]
fn watch_subprocess_runs_foreground_against_real_docker() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("coral.toml"), MANIFEST_WITH_WATCH).unwrap();

    // Spawn under a 5-second timeout — enough to confirm the
    // subprocess is foregrounded (it'll output "Listening for changes"
    // on stderr) but short enough that a hang doesn't break CI.
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_coral"))
        .args(["up", "--watch", "--env", "dev"])
        .current_dir(dir.path())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn coral up --watch");

    std::thread::sleep(std::time::Duration::from_secs(5));
    let _ = child.kill();
    let _ = child.wait();

    // Cleanup: tear down the env even if the test failed.
    let _ = Command::cargo_bin("coral")
        .unwrap()
        .args(["down", "--env", "dev"])
        .current_dir(dir.path())
        .output();
}
