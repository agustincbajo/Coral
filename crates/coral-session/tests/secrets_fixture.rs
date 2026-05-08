//! Integration test: real-shaped Claude Code JSONL fixture round-trips
//! through `parse_transcript` + `scrub` + `capture_from_path` and lands
//! in `.coral/sessions/` with EVERY secret pattern redacted.
//!
//! The fixture under `tests/fixtures/claude_code_with_secrets.jsonl` is
//! a hand-redacted miniature transcript that contains the v0.20 PRD
//! "must-redact" list:
//!
//! - `sk-ant-…` (Anthropic key)
//! - `ghp_…` (GitHub PAT)
//! - `AKIA…` (AWS access key)
//! - `Bearer <jwt>` (3-segment JWT)
//! - inline `eyJ…` JWT in tool_use input
//! - `Authorization:` header form
//! - `ANTHROPIC_API_KEY=…` env-export inline
//!
//! The fixture intentionally embeds these in BOTH `user` plain-string
//! content AND inside an `assistant` `tool_use.input` block so the
//! scrubber proves it works on the JSON-serialized body too — we run
//! the scrubber over the whole captured JSONL string, not per-message.

use coral_session::capture::{CaptureOptions, CaptureSource, capture_from_path};
use std::path::PathBuf;
use tempfile::TempDir;

/// Returns the fixture path, anchored to the crate root via
/// `CARGO_MANIFEST_DIR` so the test works regardless of `cargo test`
/// working directory.
fn fixture_path() -> PathBuf {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo");
    PathBuf::from(manifest)
        .join("tests")
        .join("fixtures")
        .join("claude_code_with_secrets.jsonl")
}

/// Captures the secrets fixture and asserts every must-redact token
/// is replaced with the corresponding `[REDACTED:<kind>]` marker.
#[test]
fn fixture_capture_redacts_every_must_redact_token() {
    let proj = TempDir::new().unwrap();
    let opts = CaptureOptions {
        source_path: fixture_path(),
        source: CaptureSource::ClaudeCode,
        project_root: proj.path().to_path_buf(),
        scrub_secrets: true,
    };
    let outcome = capture_from_path(&opts).expect("capture should succeed");
    assert!(
        outcome.redaction_count >= 5,
        "expected ≥5 redactions, got {}",
        outcome.redaction_count
    );

    let captured = std::fs::read_to_string(&outcome.captured_path).unwrap();

    // None of the original token bodies should be present.
    let banned = [
        "sk-ant-api03-FIXTUREabcDEFghiJKLmnoPQRstuVWX",
        "ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHHIIII",
        "AKIAIOSFODNN7EXAMPLE",
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0In0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c",
    ];
    for tok in banned {
        assert!(
            !captured.contains(tok),
            "secret {tok:?} survived scrubbing in captured file:\n{captured}"
        );
    }

    // At least one redaction marker for each must-redact category
    // should appear. The Anthropic key is embedded in an env-export
    // assignment (`ANTHROPIC_API_KEY=…`) so it can be caught by EITHER
    // the `anthropic_key` regex OR the wider `env_assignment` regex —
    // both are valid outcomes (the env_assignment pattern is intentionally
    // greedier so the var-name plus value land together).
    assert!(
        captured.contains("[REDACTED:anthropic_key]")
            || captured.contains("[REDACTED:env_assignment]"),
        "expected anthropic-or-env_assignment marker in captured output"
    );
    let expected_markers = ["[REDACTED:github_token]", "[REDACTED:aws_access_key]"];
    for marker in expected_markers {
        assert!(
            captured.contains(marker),
            "expected marker {marker:?} not found in captured output"
        );
    }
    // The JWT in the fixture lives inside an `Authorization: Bearer …`
    // header; the longer header pattern consumes it. So either the
    // standalone JWT marker OR the authorization marker is acceptable.
    assert!(
        captured.contains("[REDACTED:jwt]") || captured.contains("[REDACTED:authorization]"),
        "expected jwt or authorization marker in captured output"
    );
}

/// `--no-scrub` writes the fixture verbatim — proves the opt-out
/// path is a real path, not a no-op. The CLI requires
/// `--yes-i-really-mean-it` to enable this; the library variant just
/// honours the flag.
#[test]
fn fixture_capture_no_scrub_preserves_secrets() {
    let proj = TempDir::new().unwrap();
    let opts = CaptureOptions {
        source_path: fixture_path(),
        source: CaptureSource::ClaudeCode,
        project_root: proj.path().to_path_buf(),
        scrub_secrets: false,
    };
    let outcome = capture_from_path(&opts).expect("capture should succeed");
    assert_eq!(outcome.redaction_count, 0, "no-scrub must skip redactor");

    let captured = std::fs::read_to_string(&outcome.captured_path).unwrap();
    assert!(captured.contains("sk-ant-api03-FIXTUREabcDEFghiJKLmnoPQRstuVWX"));
    assert!(captured.contains("AKIAIOSFODNN7EXAMPLE"));
}
