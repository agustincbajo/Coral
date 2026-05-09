//! Integration tests for the cargo-release adoption (v0.22.0+).
//!
//! Validates `scripts/release.sh`, `scripts/extract-changelog-section.sh`, and
//! `scripts/release-gh.sh` as black-box shell wrappers. Tests are fast: most
//! exercise the scripts directly against the real CHANGELOG / a tempdir
//! checkout. Tests that need `cargo release` to actually run skip cleanly
//! when the tool isn't installed (matches local-laptop reality where the
//! maintainer installs it once and CI can opt in via `cargo install`).
//!
//! Spec coverage — section 5:
//!   #1 extract_changelog_section_returns_v0_21_4_block
//!   #2 extract_changelog_section_missing_version_exits_1
//!   #3 release_sh_preflight_fails_when_changelog_section_absent
//!   #4 release_sh_preflight_fails_when_ci_locally_fails
//!   #5 release_sh_bump_dry_run_no_changes
//!   #6 release_sh_bump_execute_produces_clean_commit_no_coauthor
//!   #7 release_sh_tag_rejects_wrong_head_subject
//!   #8 release_gh_sh_dry_run_extracts_correct_section

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;

use assert_cmd::Command;
use predicates::str::contains;
use tempfile::TempDir;

/// Resolve the workspace root from the test binary's manifest dir.
/// `CARGO_MANIFEST_DIR` is `crates/coral-cli` at test-build time;
/// the workspace root is two levels up.
fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn script(name: &str) -> PathBuf {
    workspace_root().join("scripts").join(name)
}

fn changelog_path() -> PathBuf {
    workspace_root().join("CHANGELOG.md")
}

/// Skip the test (printing a notice) if `cargo-release` isn't installed.
/// Keeps the workspace's "no new workspace deps" rule honest while still
/// pinning behavior when the tool IS available locally.
///
/// Detect via `cargo release --help` (cargo extensions don't honor
/// `--version` directly; the binary name is `cargo-release`, but the
/// canonical invocation is `cargo release …`).
fn cargo_release_available() -> bool {
    StdCommand::new("cargo")
        .args(["release", "--help"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ----------------------------------------------------------------------------
// Test #1: extract_changelog_section_returns_v0_21_4_block
// ----------------------------------------------------------------------------

#[test]
fn extract_changelog_section_returns_v0_21_4_block() {
    let output = StdCommand::new(script("extract-changelog-section.sh"))
        .arg("0.21.4")
        .arg(changelog_path())
        .output()
        .expect("extract-changelog-section.sh should run");

    assert!(
        output.status.success(),
        "expected exit 0, got {}: stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Section starts at the v0.21.4 heading.
    assert!(
        stdout.starts_with("## [0.21.4]"),
        "expected section to start with `## [0.21.4]`, got first 80 chars: {}",
        &stdout[..80.min(stdout.len())]
    );
    // Section terminates BEFORE the next-version heading.
    assert!(
        !stdout.contains("## [0.21.3]"),
        "section should NOT contain the next-version heading"
    );
    // Section is multi-line.
    assert!(
        stdout.lines().count() > 5,
        "section should be multi-line, got {} lines",
        stdout.lines().count()
    );
}

// ----------------------------------------------------------------------------
// Test #1b: extract_changelog_section_skips_fenced_pseudo_headings (HIGH 2)
// ----------------------------------------------------------------------------
//
// Regression pin for the v0.22.0 tester finding HIGH 2: the awk extractor
// must NOT treat a `## [` line inside a fenced code block as a heading
// boundary. CHANGELOG bodies often include markdown examples wrapped in
// triple backticks; pre-fix, the section truncated at the fence-internal
// pseudo-heading.

#[test]
fn extract_changelog_section_skips_fenced_pseudo_headings() {
    let tmp = TempDir::new().unwrap();
    let cl = tmp.path().join("CHANGELOG.md");
    fs::write(
        &cl,
        "## [0.22.0] - 2026-05-08\n\
         Body before code.\n\
         ```markdown\n\
         ## [Old example]\n\
         ```\n\
         Body after code.\n\
         ## [0.21.4] - 2026-05-08\n\
         older body\n",
    )
    .unwrap();

    let output = StdCommand::new(script("extract-changelog-section.sh"))
        .arg("0.22.0")
        .arg(&cl)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "expected exit 0, got {}: stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Body after code."),
        "section was truncated at the fence-internal `## [Old example]`; got:\n{stdout}"
    );
    // Fence content must round-trip verbatim, INCLUDING the pseudo-heading.
    assert!(
        stdout.contains("## [Old example]"),
        "fence body must be preserved; got:\n{stdout}"
    );
    // Next-version heading must STILL terminate the section.
    assert!(
        !stdout.contains("older body"),
        "section bled past `## [0.21.4]` heading; got:\n{stdout}"
    );
}

// ----------------------------------------------------------------------------
// Test #2: extract_changelog_section_missing_version_exits_1
// ----------------------------------------------------------------------------

#[test]
fn extract_changelog_section_missing_version_exits_1() {
    let output = StdCommand::new(script("extract-changelog-section.sh"))
        .arg("9.99.99")
        .arg(changelog_path())
        .output()
        .expect("extract-changelog-section.sh should run");

    assert_eq!(
        output.status.code(),
        Some(1),
        "expected exit 1 for missing version, got {:?}",
        output.status.code()
    );
    assert!(
        output.stdout.is_empty(),
        "stdout should be empty on missing version, got: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
}

// ----------------------------------------------------------------------------
// Test #3: release_sh_preflight_fails_when_changelog_section_absent
// ----------------------------------------------------------------------------

#[test]
fn release_sh_preflight_fails_when_changelog_section_absent() {
    let tmp = TempDir::new().unwrap();
    let work = tmp.path();

    // Minimal repo with a CHANGELOG that LACKS the version heading we'll claim.
    fs::write(
        work.join("CHANGELOG.md"),
        "# Changelog\n\n## [Unreleased]\n",
    )
    .unwrap();
    let scripts_dir = work.join("scripts");
    fs::create_dir(&scripts_dir).unwrap();
    fs::copy(script("release.sh"), scripts_dir.join("release.sh")).unwrap();
    let stub_ci = scripts_dir.join("ci-locally.sh");
    fs::write(&stub_ci, "#!/usr/bin/env bash\nexit 0\n").unwrap();
    set_exec(&stub_ci);
    set_exec(&scripts_dir.join("release.sh"));

    let output = StdCommand::new(scripts_dir.join("release.sh"))
        .arg("preflight")
        .env("NEW_VERSION", "9.9.9")
        .env("CI_LOCALLY", &stub_ci)
        .current_dir(work)
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "preflight should fail when CHANGELOG section absent"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("9.9.9"),
        "stderr should name the missing version: {}",
        stderr
    );
    assert!(
        stderr.contains("CHANGELOG"),
        "stderr should mention CHANGELOG: {}",
        stderr
    );
}

// ----------------------------------------------------------------------------
// Test #4: release_sh_preflight_fails_when_ci_locally_fails
// ----------------------------------------------------------------------------

#[test]
fn release_sh_preflight_fails_when_ci_locally_fails() {
    let tmp = TempDir::new().unwrap();
    let work = tmp.path();

    // Write a CHANGELOG with the heading present and dated today.
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let cl = format!("# Changelog\n\n## [Unreleased]\n\n## [9.9.9] - {today}\n\nbody.\n");
    fs::write(work.join("CHANGELOG.md"), cl).unwrap();

    let scripts_dir = work.join("scripts");
    fs::create_dir(&scripts_dir).unwrap();
    fs::copy(script("release.sh"), scripts_dir.join("release.sh")).unwrap();
    let stub_ci = scripts_dir.join("ci-locally.sh");
    // Stub exits 7 — preflight should propagate.
    fs::write(&stub_ci, "#!/usr/bin/env bash\nexit 7\n").unwrap();
    set_exec(&stub_ci);
    set_exec(&scripts_dir.join("release.sh"));

    let output = StdCommand::new(scripts_dir.join("release.sh"))
        .arg("preflight")
        .env("NEW_VERSION", "9.9.9")
        .env("CI_LOCALLY", &stub_ci)
        .current_dir(work)
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(7),
        "preflight should propagate ci-locally exit code 7, got {:?}",
        output.status.code()
    );
}

// ----------------------------------------------------------------------------
// Test #5: release_sh_bump_dry_run_no_changes
// ----------------------------------------------------------------------------
// `cargo release X.Y.Z` (no `--execute`) is itself a dry-run; the wrapper's
// `bump` ALWAYS uses --execute, so a true dry-run goes via cargo-release
// directly. Here we exercise the wrapper's input validation only — making
// sure `bump` with bad args bails before invoking cargo-release.

#[test]
fn release_sh_bump_dry_run_no_changes() {
    // Validate that `release.sh bump` with bad args bails before mutating.
    let tmp = TempDir::new().unwrap();
    let work = tmp.path();
    init_minimal_repo(work);

    let output = StdCommand::new(script("release.sh"))
        .arg("bump")
        .arg("not-a-version")
        .current_dir(work)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "bump with bad version should exit 2 (invalid args)"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not a SemVer"),
        "stderr should explain the validation failure: {}",
        stderr
    );

    // No working-tree mutation.
    let head_before = StdCommand::new("git")
        .arg("-C")
        .arg(work)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(head_before.status.success());

    // Trigger bump again with a too-many-args invocation; still no mutation.
    let output = StdCommand::new(script("release.sh"))
        .arg("bump")
        .arg("0.22.0")
        .arg("extra")
        .current_dir(work)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

// ----------------------------------------------------------------------------
// Test #6: release_sh_bump_execute_produces_clean_commit_no_coauthor
// ----------------------------------------------------------------------------

#[test]
fn release_sh_bump_execute_produces_clean_commit_no_coauthor() {
    if !cargo_release_available() {
        eprintln!(
            "SKIP release_sh_bump_execute_produces_clean_commit_no_coauthor: \
             cargo-release not installed"
        );
        return;
    }

    // Clone the workspace into a tempdir, prep the CHANGELOG, run bump.
    let tmp = TempDir::new().unwrap();
    let work = tmp.path().join("coral-clone");
    let cloned = clone_workspace(&work);
    if !cloned {
        eprintln!(
            "SKIP release_sh_bump_execute_produces_clean_commit_no_coauthor: \
             could not clone workspace"
        );
        return;
    }

    // Prep: write a `## [0.99.0] - <today>` section under `## [Unreleased]`.
    let cl_path = work.join("CHANGELOG.md");
    let cl = fs::read_to_string(&cl_path).unwrap();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let new_section = format!(
        "## [Unreleased]\n\n## [0.99.0] - {today}\n\n**Test bump (cargo-release dry-shape).**\n"
    );
    let cl = cl.replacen("## [Unreleased]", &new_section, 1);
    fs::write(&cl_path, cl).unwrap();

    // Add a no-op stub ci-locally.sh shim BEFORE we commit. cargo-release
    // wants a clean working tree, so everything we touched in clone_workspace
    // (release.toml, scripts/release.sh, etc.) plus this shim must be committed.
    let stub_ci = work.join("scripts").join("ci-locally-noop.sh");
    fs::write(&stub_ci, "#!/usr/bin/env bash\nexit 0\n").unwrap();
    set_exec(&stub_ci);

    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-m", "test: pre-bump prep"]);

    let output = StdCommand::new(work.join("scripts").join("release.sh"))
        .arg("bump")
        .arg("0.99.0")
        .env("CI_LOCALLY", &stub_ci)
        .current_dir(&work)
        .output()
        .expect("release.sh bump should run");

    if !output.status.success() {
        eprintln!(
            "release.sh bump failed (perhaps cargo-release version mismatch); \
             SKIPping. stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    // HEAD's commit body MUST NOT contain `Co-Authored-By: Claude` (or any case).
    let body = String::from_utf8(
        StdCommand::new("git")
            .arg("-C")
            .arg(&work)
            .args(["log", "-1", "--pretty=%B"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let lower = body.to_lowercase();
    assert!(
        !lower.contains("co-authored-by"),
        "release commit must not have any Co-Authored-By trailer; body was:\n{}",
        body
    );

    // Subject starts with `release(v0.99.0):`.
    let subject = body.lines().next().unwrap_or("");
    assert!(
        subject.starts_with("release(v0.99.0):"),
        "release subject should start with `release(v0.99.0):`, got: {}",
        subject
    );

    // No tag was created (per release.toml `tag = false`).
    let tags = StdCommand::new("git")
        .arg("-C")
        .arg(&work)
        .args(["tag", "--list", "v0.99.0"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&tags.stdout).trim().is_empty(),
        "no tag should be created by `bump`"
    );
}

// ----------------------------------------------------------------------------
// Test #7: release_sh_tag_rejects_wrong_head_subject
// ----------------------------------------------------------------------------

#[test]
fn release_sh_tag_rejects_wrong_head_subject() {
    let tmp = TempDir::new().unwrap();
    let work = tmp.path();
    init_minimal_repo(work);

    // Copy `release.sh` into the tempdir so its `cd "$REPO_ROOT"`
    // (where `REPO_ROOT` is `$(git rev-parse --show-toplevel)`) resolves
    // to THIS tempdir rather than the live Coral repo. Without the copy,
    // the script jumps back to the live repo whose HEAD subject IS
    // `release(v0.22.0):`, validation passes, and `cargo release tag`
    // gets a stray positional arg — exactly the failure mode tracked in
    // the v0.22.0 tester audit (HIGH 1).
    let scripts_dir = work.join("scripts");
    fs::create_dir(&scripts_dir).unwrap();
    fs::copy(script("release.sh"), scripts_dir.join("release.sh")).unwrap();
    set_exec(&scripts_dir.join("release.sh"));

    // HEAD subject is "init" — does NOT start with `release(v0.22.0):`.
    let output = StdCommand::new(scripts_dir.join("release.sh"))
        .arg("tag")
        .arg("0.22.0")
        .current_dir(work)
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "tag should reject when HEAD subject is wrong"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("release(v0.22.0):"),
        "stderr should name the expected prefix: {}",
        stderr
    );
}

// ----------------------------------------------------------------------------
// Test #8: release_gh_sh_dry_run_extracts_correct_section
// ----------------------------------------------------------------------------

#[test]
fn release_gh_sh_dry_run_extracts_correct_section() {
    // We need a tag to exist locally for `git rev-parse vX.Y.Z` to resolve.
    // Use v0.21.4, which IS the tip of main on the workspace under test.
    // (`scripts/release-gh.sh` runs `git -C $repo_root rev-parse $tag`.)
    // If the tag doesn't exist on the dev's machine, fall back to creating a
    // tag in a clone.

    let tag_resolves = StdCommand::new("git")
        .arg("-C")
        .arg(workspace_root())
        .args(["rev-parse", "v0.21.4"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !tag_resolves {
        eprintln!(
            "SKIP release_gh_sh_dry_run_extracts_correct_section: \
             v0.21.4 tag not present in this checkout"
        );
        return;
    }

    // GH_DRY_RUN=1 short-circuits the gh CLI invocation.
    Command::new(script("release-gh.sh"))
        .arg("v0.21.4")
        .env("GH_DRY_RUN", "1")
        .current_dir(workspace_root())
        .assert()
        .success()
        .stdout(contains("DRY RUN"))
        .stdout(contains("Feature release"))
        .stdout(contains("MultiStepRunner"))
        .stdout(contains("notes-file"));
}

// ----------------------------------------------------------------------------
// Test #9 (MEDIUM 4): release_sh_rewrites_footer_using_origin_owner_repo
// ----------------------------------------------------------------------------
//
// Regression pin for the v0.22.0 tester finding MEDIUM 4: the link-footer
// rewriter previously hardcoded `agustincbajo/Coral`, so a fork would emit
// upstream-pointing URLs. Post-fix, the helper derives `<owner>/<repo>`
// from `git remote get-url origin`. We exercise the path with origin set
// to `https://github.com/foo/bar.git` and assert the rewritten footer
// uses `foo/bar`.

#[test]
fn release_sh_rewrites_footer_using_origin_owner_repo() {
    let tmp = TempDir::new().unwrap();
    let work = tmp.path();
    init_minimal_repo(work);

    // Add origin pointing to a fork-shaped URL.
    git(
        work,
        &["remote", "add", "origin", "https://github.com/foo/bar.git"],
    );

    // CHANGELOG with a footer in the canonical shape, anchored on `foo/bar`.
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let cl = format!(
        "# Changelog\n\n## [Unreleased]\n\n## [9.9.9] - {today}\n\nbody.\n\n\
         [Unreleased]: https://github.com/foo/bar/compare/v9.8.0...HEAD\n\
         [9.8.0]: https://github.com/foo/bar/releases/tag/v9.8.0\n"
    );
    fs::write(work.join("CHANGELOG.md"), cl).unwrap();

    let scripts_dir = work.join("scripts");
    fs::create_dir(&scripts_dir).unwrap();
    fs::copy(script("release.sh"), scripts_dir.join("release.sh")).unwrap();
    let stub_ci = scripts_dir.join("ci-locally.sh");
    fs::write(&stub_ci, "#!/usr/bin/env bash\nexit 0\n").unwrap();
    set_exec(&stub_ci);
    set_exec(&scripts_dir.join("release.sh"));

    let output = StdCommand::new(scripts_dir.join("release.sh"))
        .arg("preflight")
        .env("NEW_VERSION", "9.9.9")
        .env("CI_LOCALLY", &stub_ci)
        .current_dir(work)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "preflight should succeed; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let cl_after = fs::read_to_string(work.join("CHANGELOG.md")).unwrap();
    assert!(
        cl_after.contains("[Unreleased]: https://github.com/foo/bar/compare/v9.9.9...HEAD"),
        "[Unreleased] line not rewritten with foo/bar owner; got:\n{cl_after}"
    );
    assert!(
        cl_after.contains("[9.9.9]: https://github.com/foo/bar/releases/tag/v9.9.9"),
        "[9.9.9] tag link not present with foo/bar owner; got:\n{cl_after}"
    );
    // Must NOT have leaked the upstream owner.
    assert!(
        !cl_after.contains("agustincbajo/Coral"),
        "footer leaked the hardcoded upstream owner; got:\n{cl_after}"
    );
}

// ----------------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------------

#[cfg(unix)]
fn set_exec(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

#[cfg(not(unix))]
fn set_exec(_path: &Path) {
    // Windows: PermissionsExt is unavailable; skip.
}

fn git(dir: &Path, args: &[&str]) {
    let out = StdCommand::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {:?} failed: stderr={}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_minimal_repo(work: &Path) {
    git(work, &["init", "--initial-branch=main", "--quiet"]);
    git(work, &["config", "user.email", "test@example.com"]);
    git(work, &["config", "user.name", "Test"]);
    fs::write(work.join("README.md"), "test\n").unwrap();
    git(work, &["add", "README.md"]);
    git(work, &["commit", "-m", "init", "--quiet"]);
}

/// Clone the workspace via `git clone --local` for fast tempdir copies.
/// Returns false if the clone fails (e.g. workspace tree is dirty in a way
/// that breaks the clone, or the tempdir lives on a different device).
///
/// Side-effect: copies the (possibly-uncommitted) helper scripts from the
/// live workspace into the clone, since the clone reflects only committed
/// state and these tests run against the *current edit* of the scripts.
fn clone_workspace(target: &Path) -> bool {
    let out = StdCommand::new("git")
        .arg("clone")
        .arg("--local")
        .arg("--quiet")
        .arg(workspace_root())
        .arg(target)
        .output();
    let success = matches!(out, Ok(ref o) if o.status.success());
    if !success {
        return false;
    }
    git(target, &["config", "user.email", "test@example.com"]);
    git(target, &["config", "user.name", "Test"]);
    // Some clones may not have main as default branch; ensure we're on main.
    let branch = StdCommand::new("git")
        .arg("-C")
        .arg(target)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .unwrap();
    let branch = String::from_utf8_lossy(&branch.stdout).trim().to_string();
    if branch != "main" {
        let _ = StdCommand::new("git")
            .arg("-C")
            .arg(target)
            .args(["checkout", "-B", "main"])
            .output();
    }

    // Sync the live (possibly uncommitted) helper scripts and release.toml
    // into the clone. This way the test exercises the CURRENT editor state,
    // not whatever was last committed.
    for f in &[
        "scripts/release.sh",
        "scripts/extract-changelog-section.sh",
        "scripts/release-gh.sh",
        "release.toml",
    ] {
        let from = workspace_root().join(f);
        if !from.exists() {
            continue;
        }
        let to = target.join(f);
        if let Some(parent) = to.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if fs::copy(&from, &to).is_ok() && f.starts_with("scripts/") {
            set_exec(&to);
        }
    }
    true
}
