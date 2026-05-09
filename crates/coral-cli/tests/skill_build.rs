//! End-to-end tests for `coral skill build` and `coral skill publish`
//! (v0.22.6).
//!
//! Each test spawns the `coral` binary against a stubbed workspace
//! (`Cargo.toml` with `members = ["crates/*"]` + a populated
//! `template/` tree) so the binary's `locate_template_dir` walk
//! resolves to the temp dir and never reaches into the actual
//! Coral checkout. This keeps tests hermetic and lets them run in
//! parallel with the rest of the suite.
//!
//! Pin map (acceptance criteria from the v0.22.6 orchestrator spec):
//! - Test 1 (`skill_build_produces_valid_zip`)             → AC 1, 2.
//! - Test 2 (`skill_build_includes_agents_prompts_hooks`)  → AC 5.
//! - Test 3 (`skill_build_excludes_schema_workflows_*`)    → AC 6.
//! - Test 4 (`skill_build_skill_md_frontmatter_*`)         → AC 3, 4.
//! - Test 5 (`skill_build_deterministic_two_runs`)         → AC 8.
//! - Test 6 (`skill_publish_stub_emits_deferred_message`)  → AC 9.
//! - AC 7 (`--output` override) is covered transitively by every
//!   test (they all use `--output <tempdir>/foo.zip`).
//! - AC 10 (template/ unchanged) holds by construction — the binary
//!   only reads from `template/`, never writes.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Locate the `coral` binary cargo built for this test crate.
fn coral_bin() -> PathBuf {
    let p = std::env::var_os("CARGO_BIN_EXE_coral")
        .map(PathBuf::from)
        .expect("CARGO_BIN_EXE_coral not set; cargo test should always set it");
    assert!(p.exists(), "coral bin missing at {}", p.display());
    p
}

/// Stub a minimal "Coral workspace" inside `dir`:
/// - a workspace `Cargo.toml` (so `locate_template_dir` accepts it),
/// - `template/agents/wiki-linter.md`,
/// - `template/prompts/consolidate.md`,
/// - `template/hooks/pre-commit.sh`,
/// - `template/schema/SCHEMA.base.md` (excluded — must NOT ship),
/// - `template/workflows/wiki-maintenance.yml` (excluded),
/// - `template/commands/wiki-ingest.md` (excluded),
/// - `crates/.gitkeep` (so `members = ["crates/*"]` is satisfied
///   when cargo expands the glob; technically not required for our
///   ad-hoc walker but mirrors the real workspace layout).
fn stub_workspace(dir: &Path) {
    std::fs::write(
        dir.join("Cargo.toml"),
        r#"[workspace]
members = ["crates/*"]
resolver = "2"

[workspace.package]
version = "0.0.0"
edition = "2024"
"#,
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("crates")).unwrap();
    let _ = std::fs::write(dir.join("crates/.gitkeep"), b"");

    let template = dir.join("template");
    for sub in [
        "agents",
        "prompts",
        "hooks",
        "schema",
        "workflows",
        "commands",
    ] {
        std::fs::create_dir_all(template.join(sub)).unwrap();
    }
    std::fs::write(
        template.join("agents/wiki-linter.md"),
        "---\nname: wiki-linter\ndescription: Stub linter for tests.\n---\n\nbody\n",
    )
    .unwrap();
    std::fs::write(
        template.join("agents/wiki-validator.md"),
        // No frontmatter at all — exercises the "no description"
        // fallback path so tests cover both shapes.
        "# wiki-validator\n\nstub body\n",
    )
    .unwrap();
    std::fs::write(
        template.join("prompts/consolidate.md"),
        "# Consolidate prompt\n\nstub\n",
    )
    .unwrap();
    std::fs::write(
        template.join("prompts/lint-auto-fix.md"),
        "# Lint auto-fix prompt\n\nstub\n",
    )
    .unwrap();
    std::fs::write(
        template.join("hooks/pre-commit.sh"),
        "#!/bin/sh\necho stub\nexit 0\n",
    )
    .unwrap();
    // Excluded subdirs — populate so we can prove they're skipped.
    std::fs::write(
        template.join("schema/SCHEMA.base.md"),
        "# Schema (SHOULD NOT SHIP)\n",
    )
    .unwrap();
    std::fs::write(
        template.join("workflows/wiki-maintenance.yml"),
        "# Workflow (SHOULD NOT SHIP)\n",
    )
    .unwrap();
    std::fs::write(
        template.join("commands/wiki-ingest.md"),
        "# Slash command (SHOULD NOT SHIP)\n",
    )
    .unwrap();
}

/// Read the named file out of a zip archive on disk. Panics on
/// missing entry to keep assertions noisy in CI.
fn read_zip_entry(zip_path: &Path, name: &str) -> Vec<u8> {
    let f = std::fs::File::open(zip_path).expect("open zip");
    let mut archive = zip::ZipArchive::new(f).expect("zip parses");
    let mut e = archive.by_name(name).expect("entry exists");
    let mut buf = Vec::new();
    e.read_to_end(&mut buf).unwrap();
    buf
}

/// Names of every zip entry, sorted, for set-membership assertions.
fn zip_entry_names(zip_path: &Path) -> Vec<String> {
    let f = std::fs::File::open(zip_path).expect("open zip");
    let mut archive = zip::ZipArchive::new(f).expect("zip parses");
    let mut names: Vec<String> = (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_string())
        .collect();
    names.sort();
    names
}

/// Spawn `coral skill build --output <out>` from `wsdir`.
fn run_build(wsdir: &Path, out: &Path) -> std::process::Output {
    Command::new(coral_bin())
        .current_dir(wsdir)
        .args([
            "skill",
            "build",
            "--output",
            out.to_str().expect("output is UTF-8"),
        ])
        .output()
        .expect("spawn coral skill build")
}

#[test]
fn skill_build_produces_valid_zip() {
    let tmp = tempfile::tempdir().unwrap();
    stub_workspace(tmp.path());
    let out = tmp.path().join("bundle.zip");

    let output = run_build(tmp.path(), &out);
    assert!(
        output.status.success(),
        "exit non-zero; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out.exists(), "zip not written at {}", out.display());

    // Re-open via zip::ZipArchive — guards against torn archives.
    let f = std::fs::File::open(&out).unwrap();
    let mut archive = zip::ZipArchive::new(f).expect("zip is well-formed");
    assert!(!archive.is_empty(), "zip must have at least one entry");
    // SKILL.md must be present at root.
    assert!(
        archive.by_name("SKILL.md").is_ok(),
        "SKILL.md missing from zip root"
    );

    // stdout describes the artifact.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("wrote") && stdout.contains("bundle.zip"),
        "expected `wrote ...bundle.zip...` line, got: {stdout}"
    );
}

#[test]
fn skill_build_includes_agents_prompts_hooks() {
    let tmp = tempfile::tempdir().unwrap();
    stub_workspace(tmp.path());
    let out = tmp.path().join("bundle.zip");

    let status = run_build(tmp.path(), &out);
    assert!(status.status.success());

    let names = zip_entry_names(&out);
    // Exact files we stubbed must be present at the rewritten zip
    // paths (with the `template/` prefix stripped).
    for expected in [
        "agents/wiki-linter.md",
        "agents/wiki-validator.md",
        "prompts/consolidate.md",
        "prompts/lint-auto-fix.md",
        "hooks/pre-commit.sh",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "missing expected entry {expected}; have: {names:?}"
        );
    }
    // And the body of one of them is byte-identical to the source.
    let body = read_zip_entry(&out, "hooks/pre-commit.sh");
    assert_eq!(
        std::str::from_utf8(&body).unwrap(),
        "#!/bin/sh\necho stub\nexit 0\n",
        "hook body must round-trip byte-exactly"
    );
}

#[test]
fn skill_build_excludes_schema_workflows_commands() {
    let tmp = tempfile::tempdir().unwrap();
    stub_workspace(tmp.path());
    let out = tmp.path().join("bundle.zip");

    let status = run_build(tmp.path(), &out);
    assert!(status.status.success());

    let names = zip_entry_names(&out);
    for excluded in [
        "schema/SCHEMA.base.md",
        "workflows/wiki-maintenance.yml",
        "commands/wiki-ingest.md",
        // Defense-in-depth: also pin the bare directory prefixes
        // so a future refactor that smuggles them in via a
        // different path still trips this assertion.
        "schema/",
        "workflows/",
        "commands/",
    ] {
        assert!(
            !names.iter().any(|n| n.starts_with(excluded)),
            "entry {excluded} must NOT be in the bundle; have: {names:?}"
        );
    }
}

#[test]
fn skill_build_skill_md_frontmatter_has_name_description_version() {
    let tmp = tempfile::tempdir().unwrap();
    stub_workspace(tmp.path());
    let out = tmp.path().join("bundle.zip");

    let status = run_build(tmp.path(), &out);
    assert!(status.status.success());

    let body = String::from_utf8(read_zip_entry(&out, "SKILL.md")).unwrap();
    assert!(
        body.starts_with("---\n"),
        "SKILL.md must open with YAML frontmatter; got:\n{body}"
    );
    assert!(
        body.contains("\nname: coral\n"),
        "frontmatter `name: coral` missing"
    );
    assert!(
        body.contains("description: "),
        "frontmatter `description:` missing"
    );
    // AC #4: version must equal the running binary's CARGO_PKG_VERSION.
    let expected = format!("\nversion: {}\n", env!("CARGO_PKG_VERSION"));
    assert!(
        body.contains(&expected),
        "SKILL.md frontmatter must carry version {} (CARGO_PKG_VERSION); body was:\n{body}",
        env!("CARGO_PKG_VERSION")
    );
    // The closing `---\n` for the frontmatter block is also required.
    let after_open = body.strip_prefix("---\n").unwrap();
    assert!(
        after_open.contains("\n---\n"),
        "frontmatter must close with `---`"
    );
}

#[test]
fn skill_build_deterministic_two_runs() {
    let tmp = tempfile::tempdir().unwrap();
    stub_workspace(tmp.path());
    let out1 = tmp.path().join("run1.zip");
    let out2 = tmp.path().join("run2.zip");

    assert!(run_build(tmp.path(), &out1).status.success());
    // Sleep a tick so any naive `now()`-based timestamping would
    // produce a diff. The 1980-epoch pin is what keeps the bytes
    // stable — without it, this assertion is the canary.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    assert!(run_build(tmp.path(), &out2).status.success());

    let b1 = std::fs::read(&out1).unwrap();
    let b2 = std::fs::read(&out2).unwrap();
    assert_eq!(
        b1.len(),
        b2.len(),
        "two runs produced different zip sizes ({} vs {})",
        b1.len(),
        b2.len()
    );
    assert_eq!(
        b1, b2,
        "two consecutive `coral skill build` runs MUST produce byte-identical zips"
    );
}

#[test]
fn skill_publish_stub_emits_deferred_message() {
    // No workspace stub needed — `publish` doesn't touch `template/`.
    let output = Command::new(coral_bin())
        .args(["skill", "publish"])
        .output()
        .expect("spawn coral skill publish");

    assert!(
        output.status.success(),
        "exit non-zero; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    // The whole sentence is asserted byte-for-byte (modulo the
    // trailing newline `println!` adds) so a future copy edit is
    // a deliberate, test-touching change.
    let expected = "publish is deferred to v0.23+; for now, run `coral skill build` \
                    and submit the zip manually to https://github.com/anthropics/skills\n";
    assert_eq!(
        stdout, expected,
        "deferred-message text must match spec §D5"
    );
}
