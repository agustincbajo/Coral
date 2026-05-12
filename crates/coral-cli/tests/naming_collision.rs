//! `coral doctor` vs `coral project doctor` naming-collision regression
//! (v0.34.0 cross-FR consistency item #X4, PRD §0 item 24).
//!
//! Two diagnostic surfaces exist on purpose:
//!
//! * `coral doctor` (new in v0.34.0) — top-level wrapper around
//!   `coral self-check`'s probe pipeline; serves the `coral-doctor`
//!   skill and the `/coral:coral-doctor` slash command.
//! * `coral project doctor` (pre-existing since v0.16) — multi-repo
//!   manifest health check (apiVersion, clones, lockfile, unique paths).
//!
//! The two report formats are NOT interchangeable: the new flow emits
//! the `SelfCheck` JSON envelope (FR-ONB-9 hook contract), the old flow
//! emits a `[severity] message` text report keyed on `coral.toml`
//! entries. PRD item #X4 requires a test asserting that
//! `coral doctor --non-interactive` returns the **new** envelope so
//! consumers of the JSON contract (skill, hook, future automation)
//! don't accidentally bind to the old text output.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn doctor_non_interactive_emits_self_check_json_envelope() {
    let tmp = TempDir::new().unwrap();
    let out = Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["doctor", "--non-interactive"])
        .output()
        .expect("spawn coral doctor");
    assert!(
        out.status.success(),
        "coral doctor --non-interactive exited non-zero. stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8(out.stdout).expect("stdout not UTF-8");
    let json: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("coral doctor --non-interactive emitted non-JSON output (expected SelfCheck envelope, got `coral project doctor`-shaped text?): {e}\nstdout was:\n{stdout}")
    });

    // The SelfCheck envelope (FR-ONB-9 / PRD Appendix F) is keyed by
    // `schema_version` + `coral_status`. The old `coral project doctor`
    // emits free-form text with NO `schema_version`, so this assertion
    // is the definitive boundary between the two flows.
    assert!(
        json.get("schema_version").is_some(),
        "doctor --non-interactive output is missing `schema_version` — looks like the old project-doctor format leaked through. stdout: {stdout}"
    );
    assert!(
        json.get("coral_status").is_some(),
        "doctor --non-interactive output is missing `coral_status` — see schema in coral-cli::commands::self_check. stdout: {stdout}"
    );
    // Spot-check a handful of FR-ONB-6 fields so a future regression
    // that strips them (and re-emits something JSON-shaped but empty)
    // still fails.
    for key in [
        "coral_version",
        "binary_path",
        "platform",
        "providers_available",
    ] {
        assert!(
            json.get(key).is_some(),
            "doctor --non-interactive output is missing `{key}` from the SelfCheck envelope. stdout: {stdout}"
        );
    }
}

#[test]
fn doctor_default_prints_human_report_not_project_doctor_format() {
    // `coral doctor` (no flags) prints a human-readable header. The old
    // `coral project doctor` prints `coral project doctor — manifest
    // health` (verified in crates/coral-cli/src/commands/project/
    // doctor.rs::print_report). If a future refactor accidentally
    // routes the top-level `doctor` to the project-doctor handler, the
    // header would differ — this test pins the boundary.
    let tmp = TempDir::new().unwrap();
    let out = Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("doctor")
        .output()
        .expect("spawn coral doctor");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("Coral doctor"),
        "coral doctor stdout missing 'Coral doctor' header. stdout: {stdout}"
    );
    // Anti-assertion: the legacy `project doctor` header MUST NOT
    // appear. (`coral project doctor` prints e.g. `# Coral project
    // doctor` or similar — the key phrase "project doctor" never
    // appears in the new flow.)
    assert!(
        !stdout.to_lowercase().contains("project doctor"),
        "coral doctor stdout leaks the legacy `project doctor` format. stdout: {stdout}"
    );
}

#[test]
fn project_doctor_still_works_independently() {
    // Belt-and-suspenders: confirm `coral project doctor` is still
    // routable and is NOT shadowed by the new top-level `doctor`. This
    // is the BC promise the PRD makes — v0.16 callers (CI scripts that
    // run `coral project doctor --strict`) must keep working.
    let tmp = TempDir::new().unwrap();
    // `coral project doctor` expects either a legacy single-repo
    // layout (no coral.toml -> emits "legacy single-repo project"
    // info) or a real coral.toml. In a pristine tempdir with no
    // .wiki/ it falls through to the legacy path and exits 0.
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["init"])
        .assert()
        .success();

    let out = Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["project", "doctor"])
        .output()
        .expect("spawn coral project doctor");
    assert!(
        out.status.success(),
        "coral project doctor regressed. stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The legacy flow prints free-form text (NOT JSON). If a refactor
    // ever swaps it for the SelfCheck envelope, this assertion fails
    // -> human decision required.
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        serde_json::from_str::<Value>(stdout.trim()).is_err(),
        "coral project doctor unexpectedly emits JSON now — that breaks the v0.16 BC contract. stdout: {stdout}"
    );
}
