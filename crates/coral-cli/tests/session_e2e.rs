//! End-to-end test for `coral session` subcommands.
//!
//! Drives the CLI binary against a tmpdir + a hand-rolled Claude
//! Code JSONL fixture so the full flow is covered without network or
//! a real LLM call:
//!
//!   coral init
//!     → project-root .gitignore now lists `.coral/sessions/*.jsonl`
//!   coral session capture --from claude-code <fixture-path>
//!     → .coral/sessions/<date>_claude-code_<sha>.jsonl exists
//!     → secrets in the fixture are redacted in the captured file
//!     → index.json carries one entry
//!   coral session list
//!     → renders a Markdown table with the captured short-id
//!   coral session list --format json
//!     → renders parseable JSON
//!   coral session show <short-id>
//!     → prints session metadata
//!   coral session forget <short-id> --yes
//!     → removes both the .jsonl and the index entry
//!
//! Distillation is exercised at the library level
//! (`coral-session::distill::tests`) because driving `Runner` through
//! the binary requires a real provider; the integration test stops
//! at the LLM-free flow.

use assert_cmd::Command;
use predicates::str::contains;
use std::path::PathBuf;
use tempfile::TempDir;

/// Tiny fixture transcript with two messages and a fake Anthropic
/// key embedded so we can assert the scrubber fired.
const FIXTURE_TRANSCRIPT: &str = r#"{"type":"user","sessionId":"e2e-test-session-12345","timestamp":"2026-05-08T10:00:00Z","cwd":"/x","message":{"role":"user","content":"Use sk-ant-api03-E2EE2EE2EE2EE2EE2EE2EE2EE2EE for the API call"}}
{"type":"assistant","sessionId":"e2e-test-session-12345","timestamp":"2026-05-08T10:00:01Z","message":{"role":"assistant","content":[{"type":"text","text":"OK"}]}}
"#;

fn write_fixture(dir: &std::path::Path) -> PathBuf {
    let p = dir.join("source-transcript.jsonl");
    std::fs::write(&p, FIXTURE_TRANSCRIPT).unwrap();
    p
}

#[test]
fn session_capture_list_show_forget_full_flow() {
    let proj = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    let fixture = write_fixture(src.path());

    // 1) init — sets up the project-root .gitignore with session patterns.
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(proj.path())
        .arg("init")
        .assert()
        .success();
    let gitignore = std::fs::read_to_string(proj.path().join(".gitignore")).unwrap();
    assert!(
        gitignore.contains(".coral/sessions/*.jsonl"),
        "init must add session patterns to .gitignore, got:\n{gitignore}"
    );
    assert!(
        gitignore.contains("!.coral/sessions/distilled/"),
        "init must include distilled negation, got:\n{gitignore}"
    );

    // 2) capture — writes a redacted JSONL + index.
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(proj.path())
        .args(["session", "capture", "--from", "claude-code"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(contains("captured e2e-test-session-12345"))
        .stdout(contains("redactions"));

    let sessions_dir = proj.path().join(".coral/sessions");
    let captured: Vec<_> = std::fs::read_dir(&sessions_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            n.ends_with(".jsonl")
        })
        .collect();
    assert_eq!(captured.len(), 1, "expected exactly one captured jsonl");
    let captured_path = captured[0].path();
    let captured_body = std::fs::read_to_string(&captured_path).unwrap();
    assert!(
        !captured_body.contains("sk-ant-api03-E2EE2EE2"),
        "secret survived scrubbing: {captured_body}"
    );
    assert!(
        captured_body.contains("[REDACTED:") || captured_body.contains("anthropic_key"),
        "no redaction marker present: {captured_body}"
    );
    let idx_path = sessions_dir.join("index.json");
    assert!(idx_path.exists(), "index.json missing");
    let idx_body = std::fs::read_to_string(&idx_path).unwrap();
    assert!(idx_body.contains("e2e-test-session-12345"));

    // 3) list (markdown) — table contains the short-id.
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(proj.path())
        .args(["session", "list"])
        .assert()
        .success()
        .stdout(contains("e2e-test"))
        .stdout(contains("claude-code"));

    // 4) list (json) — parseable + has the right session.
    let json_out = Command::cargo_bin("coral")
        .unwrap()
        .current_dir(proj.path())
        .args(["session", "list", "--format", "json"])
        .output()
        .unwrap();
    assert!(json_out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_slice(&json_out.stdout).expect("list --format json must be valid JSON");
    let arr = parsed.as_array().expect("list json must be an array");
    assert_eq!(arr.len(), 1);
    assert_eq!(
        arr[0].get("session_id").and_then(|v| v.as_str()),
        Some("e2e-test-session-12345")
    );

    // 5) show — by short id.
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(proj.path())
        .args(["session", "show", "e2e-test"])
        .assert()
        .success()
        .stdout(contains("e2e-test-session-12345"))
        .stdout(contains("first 2 message"));

    // 6) forget --yes — clean teardown.
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(proj.path())
        .args(["session", "forget", "e2e-test", "--yes"])
        .assert()
        .success()
        .stdout(contains("deleted session"));
    assert!(!captured_path.exists(), "raw jsonl should be gone");
    let post_index = std::fs::read_to_string(&idx_path).unwrap();
    assert!(
        !post_index.contains("e2e-test-session-12345"),
        "index entry should be gone: {post_index}"
    );
}

/// `coral session capture --no-scrub` without the confirmation flag
/// must fail fast — that's the privacy gate.
#[test]
fn session_capture_no_scrub_without_confirmation_fails() {
    let proj = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    let fixture = write_fixture(src.path());
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(proj.path())
        .arg("init")
        .assert()
        .success();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(proj.path())
        .args(["session", "capture", "--from", "claude-code", "--no-scrub"])
        .arg(&fixture)
        .assert()
        .failure()
        .stderr(contains("yes-i-really-mean-it"));
}

/// `coral session capture --no-scrub --yes-i-really-mean-it` writes
/// the source bytes verbatim — pinning the documented escape hatch.
#[test]
fn session_capture_no_scrub_with_confirmation_writes_raw_bytes() {
    let proj = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    let fixture = write_fixture(src.path());
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(proj.path())
        .arg("init")
        .assert()
        .success();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(proj.path())
        .args([
            "session",
            "capture",
            "--from",
            "claude-code",
            "--no-scrub",
            "--yes-i-really-mean-it",
        ])
        .arg(&fixture)
        .assert()
        .success();
    let sessions_dir = proj.path().join(".coral/sessions");
    let entry = std::fs::read_dir(&sessions_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| {
            let n = e.file_name();
            n.to_string_lossy().ends_with(".jsonl")
        })
        .expect("captured jsonl");
    let body = std::fs::read_to_string(entry.path()).unwrap();
    assert!(
        body.contains("sk-ant-api03-E2EE2EE2"),
        "no-scrub must preserve source bytes verbatim: {body}"
    );
}

/// Cursor / chatgpt sources surface a clear "not yet implemented"
/// error pointing at issue #16, not a clap-level rejection.
#[test]
fn session_capture_cursor_returns_not_yet_implemented() {
    let proj = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    let fixture = write_fixture(src.path());
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(proj.path())
        .arg("init")
        .assert()
        .success();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(proj.path())
        .args(["session", "capture", "--from", "cursor"])
        .arg(&fixture)
        .assert()
        .failure()
        .stderr(contains("not yet implemented"))
        .stderr(contains("#16"));
}

/// `coral lint` flags `reviewed: false` synthesis pages as Critical
/// — the trust-by-curation gate.
#[test]
fn lint_rejects_unreviewed_distilled_page_as_critical() {
    let proj = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(proj.path())
        .arg("init")
        .assert()
        .success();
    // Drop a synthesis page with reviewed: false into the wiki.
    // v0.20.1 cycle-4 audit H2: the `unreviewed-distilled` lint is
    // qualified — only fires when `reviewed: false` AND `source.runner`
    // names an LLM provider. So this fixture mirrors what `coral
    // session distill` actually emits, including the `source` block.
    let synth_dir = proj.path().join(".wiki/synthesis");
    std::fs::create_dir_all(&synth_dir).unwrap();
    std::fs::write(
        synth_dir.join("test.md"),
        r#"---
slug: test
type: synthesis
last_updated_commit: abc
confidence: 0.4
status: draft
sources: []
backlinks: []
reviewed: false
source:
  runner: "claude-sonnet-4-5"
  prompt_version: 1
  session_id: "abc123def456"
  captured_at: "2026-05-08T10:00:00Z"
---

# test

A distilled page that hasn't been reviewed yet.
"#,
    )
    .unwrap();

    // `coral lint` exits non-zero when a Critical issue exists.
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(proj.path())
        .args(["lint", "--format", "json"])
        .assert()
        .failure()
        .stdout(contains("unreviewed_distilled"));
}
